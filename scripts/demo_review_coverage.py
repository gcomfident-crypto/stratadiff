#!/usr/bin/env python3

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
BENCHMARK = ROOT / "benchmarks" / "resumebench-real-v1"
VERIFIER = BENCHMARK / "verify.py"
ORACLE = BENCHMARK / "oracle.json"
DEFAULT_WORKSPACE = ROOT / "target" / "review-coverage-demo"
DEFAULT_BINARY = ROOT / "target" / "release" / "stratadiff"


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def run(arguments, *, cwd=ROOT, env=None):
    subprocess.run([str(argument) for argument in arguments], cwd=cwd, env=env, check=True)


def isolated_git_environment():
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.upper().startswith("GIT_")
    }
    environment["GIT_CONFIG_GLOBAL"] = os.devnull
    environment["GIT_CONFIG_NOSYSTEM"] = "1"
    environment["GIT_NO_LAZY_FETCH"] = "1"
    environment["GIT_NO_REPLACE_OBJECTS"] = "1"
    environment["GIT_TERMINAL_PROMPT"] = "0"
    return environment


def parse_arguments():
    parser = argparse.ArgumentParser(
        description="Reproduce StrataDiff's review-coverage gate on a real Gerrit rebase"
    )
    parser.add_argument(
        "--workspace",
        type=Path,
        default=DEFAULT_WORKSPACE,
        help="materialized fixture and reports directory",
    )
    parser.add_argument(
        "--stratadiff",
        type=Path,
        default=DEFAULT_BINARY,
        help="release StrataDiff binary",
    )
    parser.add_argument(
        "--offline",
        action="store_true",
        help="reuse an existing fixture and binary without building or fetching",
    )
    parser.add_argument(
        "--open",
        action="store_true",
        help="open the local Review Resume Workbench after verification",
    )
    return parser.parse_args()


def verify_release_binary(binary):
    result = subprocess.run(
        [str(binary), "build-info"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    require(not result.stderr, "stratadiff build-info produced diagnostics")
    info = json.loads(result.stdout)
    require(info["schema"] == "stratadiff-build-info-v1", "unsupported build-info schema")
    require(info["build_profile"] == "release", "demo requires a release binary")
    require(info["git_dirty"] is False, "demo requires a binary built from a clean Git checkout")


def validate_gate_report(report, oracle):
    expected = oracle["summary"]
    summary = report["summary"]["checkpoint"]
    files = report["files"]
    exact = sum(
        file["checkpoint_match_basis"] == "exact_git_change_identity"
        for file in files
        if "checkpoint_match_basis" in file
    )
    four_way = sum(
        file["checkpoint_match_basis"] == "exact_noninteracting_four_way_byte_replay"
        for file in files
        if "checkpoint_match_basis" in file
    )
    residue = sorted(
        file["after_path"]
        for file in files
        if file["checkpoint_state"] == "needs_review_now"
    )
    expected_residue = sorted(
        file["path_utf8"]
        for file in oracle["classification"]
        if file["checkpoint_state"] == "needs_review_now"
    )
    require(
        report["summary"]["changed_files"] == expected["current_pr_files"],
        "current file count differs",
    )
    require(
        summary["unchanged_since_checkpoint_files"] == expected["carried"],
        "carried count differs",
    )
    require(exact == expected["exactly_carried"], "exact-identity carry count differs")
    require(four_way == expected["replay_carried"], "four-way carry count differs")
    require(
        summary["needs_review_now_files"] == expected["needs_review_now"],
        "review residue count differs",
    )
    require(
        summary["retired_change_count"] == expected["retired_checkpoint_changes"],
        "retired count differs",
    )
    require(
        residue == expected_residue,
        "review residue paths differ from the provider-backed oracle",
    )
    return expected, residue


def main():
    arguments = parse_arguments()
    workspace = arguments.workspace.resolve()
    binary = arguments.stratadiff.resolve()
    repository = workspace / "repository.git"

    if arguments.offline:
        require(repository.is_dir(), f"offline fixture is missing: {repository}")
        require(binary.is_file(), f"offline release binary is missing: {binary}")
    else:
        require(
            binary == DEFAULT_BINARY.resolve(),
            "a custom --stratadiff binary is supported only with --offline",
        )
        run(["cargo", "build", "--locked", "--release", "--bin", "stratadiff"])
        if workspace.exists():
            require(
                repository.is_dir(),
                f"demo workspace exists without repository.git: {workspace}",
            )
        else:
            workspace.parent.mkdir(parents=True, exist_ok=True)
            run([sys.executable, VERIFIER, "materialize", "--output", workspace])

    verify_release_binary(binary)
    evaluation_path = workspace / "evaluation.json"
    report_path = workspace / "review-report.json"
    summary_path = workspace / "review-summary.md"
    run(
        [
            sys.executable,
            VERIFIER,
            "evaluate",
            "--repository",
            repository,
            "--stratadiff",
            binary,
            "--output",
            evaluation_path,
        ]
    )

    oracle = json.loads(ORACLE.read_text(encoding="utf-8"))
    snapshots = oracle["snapshots"]
    if report_path.exists():
        report_path.unlink()
    if summary_path.exists():
        summary_path.unlink()
    environment = isolated_git_environment()
    environment["GITHUB_STEP_SUMMARY"] = str(summary_path)
    gate = subprocess.run(
        [
            str(binary),
            "review",
            "--repo",
            str(repository),
            "--checkpoint",
            snapshots["B"],
            "--format",
            "json",
            "--output",
            str(report_path),
            "--github-summary",
            "--fail-on-review-residue",
            "--",
            snapshots["C"],
            snapshots["D"],
        ],
        cwd=ROOT,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    require(gate.returncode == 1, f"required check returned unexpected status {gate.returncode}")
    require(
        report_path.is_file() and report_path.stat().st_size > 0,
        "blocked gate did not preserve its JSON report",
    )
    require(
        summary_path.is_file() and summary_path.stat().st_size > 0,
        "blocked gate did not preserve its Markdown summary",
    )
    report = json.loads(report_path.read_text(encoding="utf-8"))
    expected, residue = validate_gate_report(report, oracle)
    expected_error = (
        f"review residue gate is open: {expected['needs_review_now']} "
        "current PR files need review"
    )
    require(expected_error in gate.stderr, "required check did not report the expected residue")

    print("ResumeBench-Real v1: PASS")
    print(f"Required check: BLOCKED as expected (exit {gate.returncode})")
    print(
        f"Coverage: {expected['current_pr_files']} current / {expected['carried']} carried "
        f"({expected['exactly_carried']} exact-identity + {expected['replay_carried']} four-way) / "
        f"{expected['needs_review_now']} need review / {expected['retired_checkpoint_changes']} retired"
    )
    for path in residue:
        print(f"Needs review: {path}")
    print(f"Evaluation: {evaluation_path}")
    print(f"JSON report: {report_path}")
    print(f"Markdown summary: {summary_path}")

    if arguments.open:
        run(
            [
                binary,
                "review",
                "--repo",
                repository,
                "--checkpoint",
                snapshots["B"],
                "--workbench",
                "--",
                snapshots["C"],
                snapshots["D"],
            ],
            env=isolated_git_environment(),
        )


if __name__ == "__main__":
    main()
