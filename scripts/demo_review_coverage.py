#!/usr/bin/env python3

import argparse
import copy
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
REVIEW_SCHEMA = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-v1.schema.json"
CHECKPOINT_MATCH_BASIS = (
    "exact_git_change_identity_or_noninteracting_four_way_byte_replay"
)


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
    environment["LC_ALL"] = "C"
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
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run pure validator mutation tests without Git, network, or a binary",
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
    snapshots = oracle["snapshots"]
    require(report["schema"] == REVIEW_SCHEMA, "review report schema differs")
    require(report["requested_base"] == snapshots["C"], "requested base differs")
    require(report["base_commit"] == snapshots["C"], "comparison base differs")
    require(report["requested_head"] == snapshots["D"], "requested head differs")
    require(report["head_commit"] == snapshots["D"], "comparison head differs")

    checkpoint = report["checkpoint"]
    require(
        checkpoint["requested_revision"] == snapshots["B"],
        "requested checkpoint differs",
    )
    require(checkpoint["commit"] == snapshots["B"], "checkpoint commit differs")
    require(
        checkpoint["base_commit"] == snapshots["A"],
        "checkpoint base differs",
    )
    require(
        checkpoint["match_basis"] == CHECKPOINT_MATCH_BASIS,
        "global checkpoint match basis differs",
    )

    summary = report["summary"]["checkpoint"]
    files = report["files"]
    expected_by_path = {}
    for classification in oracle["classification"]:
        path = classification["path_utf8"]
        require(path not in expected_by_path, f"oracle contains duplicate path: {path}")
        expected_by_path[path] = classification
    require(
        len(expected_by_path) == expected["current_pr_files"],
        "oracle classification count differs",
    )

    observed = {}
    for file in files:
        path = file["after_path"]
        require(path not in observed, f"review report contains duplicate path: {path}")
        require(path in expected_by_path, f"review report contains unexpected path: {path}")
        classification = expected_by_path[path]
        require(
            file["checkpoint_state"] == classification["checkpoint_state"],
            f"checkpoint state differs: {path}",
        )
        if "checkpoint_match_basis" in classification:
            require(
                "checkpoint_match_basis" in file,
                f"checkpoint match basis is missing: {path}",
            )
            require(
                file["checkpoint_match_basis"]
                == classification["checkpoint_match_basis"],
                f"checkpoint match basis differs: {path}",
            )
        else:
            require(
                "checkpoint_match_basis" not in file,
                f"needs-review file unexpectedly has a match basis: {path}",
            )
        observed[path] = file

    missing = sorted(set(expected_by_path) - set(observed))
    require(not missing, f"review report omitted expected paths: {', '.join(missing)}")
    residue = sorted(
        path
        for path, file in observed.items()
        if file["checkpoint_state"] == "needs_review_now"
    )
    exact = sum(
        file["checkpoint_match_basis"] == "exact_git_change_identity"
        for file in observed.values()
        if "checkpoint_match_basis" in file
    )
    four_way = sum(
        file["checkpoint_match_basis"]
        == "exact_noninteracting_four_way_byte_replay"
        for file in observed.values()
        if "checkpoint_match_basis" in file
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
    return expected, residue


def self_test_report(oracle):
    snapshots = oracle["snapshots"]
    files = []
    for classification in oracle["classification"]:
        file = {
            "after_path": classification["path_utf8"],
            "checkpoint_state": classification["checkpoint_state"],
        }
        if "checkpoint_match_basis" in classification:
            file["checkpoint_match_basis"] = classification["checkpoint_match_basis"]
        files.append(file)
    return {
        "schema": REVIEW_SCHEMA,
        "requested_base": snapshots["C"],
        "base_commit": snapshots["C"],
        "requested_head": snapshots["D"],
        "head_commit": snapshots["D"],
        "checkpoint": {
            "requested_revision": snapshots["B"],
            "commit": snapshots["B"],
            "base_commit": snapshots["A"],
            "match_basis": CHECKPOINT_MATCH_BASIS,
        },
        "summary": {
            "changed_files": oracle["summary"]["current_pr_files"],
            "checkpoint": {
                "unchanged_since_checkpoint_files": oracle["summary"]["carried"],
                "needs_review_now_files": oracle["summary"]["needs_review_now"],
                "retired_change_count": oracle["summary"]["retired_checkpoint_changes"],
            },
        },
        "files": files,
    }


def require_validation_failure(label, report, oracle):
    try:
        validate_gate_report(report, oracle)
    except RuntimeError:
        return
    raise RuntimeError(f"validator accepted mutation: {label}")


def run_self_test():
    oracle = json.loads(ORACLE.read_text(encoding="utf-8"))
    valid = self_test_report(oracle)
    validate_gate_report(valid, oracle)

    swapped_basis = copy.deepcopy(valid)
    swapped_files = {file["after_path"]: file for file in swapped_basis["files"]}
    replay_path = "Documentation/user-search.txt"
    exact_path = "java/com/google/gerrit/server/query/change/ChangePredicates.java"
    swapped_files[replay_path]["checkpoint_match_basis"] = "exact_git_change_identity"
    swapped_files[exact_path]["checkpoint_match_basis"] = (
        "exact_noninteracting_four_way_byte_replay"
    )
    require_validation_failure("per-file bases swapped", swapped_basis, oracle)

    mutations = [
        ("schema", ["schema"], "wrong-schema"),
        ("requested base", ["requested_base"], oracle["snapshots"]["A"]),
        ("comparison base", ["base_commit"], oracle["snapshots"]["A"]),
        ("requested head", ["requested_head"], oracle["snapshots"]["B"]),
        ("comparison head", ["head_commit"], oracle["snapshots"]["B"]),
        (
            "requested checkpoint",
            ["checkpoint", "requested_revision"],
            oracle["snapshots"]["A"],
        ),
        ("checkpoint commit", ["checkpoint", "commit"], oracle["snapshots"]["A"]),
        ("checkpoint base", ["checkpoint", "base_commit"], oracle["snapshots"]["C"]),
        ("global match basis", ["checkpoint", "match_basis"], "wrong-basis"),
    ]
    for label, keys, value in mutations:
        mutated = copy.deepcopy(valid)
        target = mutated
        for key in keys[:-1]:
            target = target[key]
        target[keys[-1]] = value
        require_validation_failure(label, mutated, oracle)

    wrong_state = copy.deepcopy(valid)
    wrong_state["files"][0]["checkpoint_state"] = "needs_review_now"
    require_validation_failure("per-file state", wrong_state, oracle)

    missing_basis = copy.deepcopy(valid)
    del missing_basis["files"][0]["checkpoint_match_basis"]
    require_validation_failure("missing per-file basis", missing_basis, oracle)

    unexpected_basis = copy.deepcopy(valid)
    needs_review = next(
        file
        for file in unexpected_basis["files"]
        if file["checkpoint_state"] == "needs_review_now"
    )
    needs_review["checkpoint_match_basis"] = "exact_git_change_identity"
    require_validation_failure("unexpected residue basis", unexpected_basis, oracle)

    missing_file = copy.deepcopy(valid)
    missing_file["files"].pop()
    require_validation_failure("missing file", missing_file, oracle)

    extra_file = copy.deepcopy(valid)
    extra_file["files"].append(
        {"after_path": "unexpected.txt", "checkpoint_state": "needs_review_now"}
    )
    require_validation_failure("extra file", extra_file, oracle)

    duplicate_file = copy.deepcopy(valid)
    duplicate_file["files"].append(copy.deepcopy(duplicate_file["files"][0]))
    require_validation_failure("duplicate file", duplicate_file, oracle)
    print("Demo review-coverage validator self-test: PASS")


def main():
    arguments = parse_arguments()
    if arguments.self_test:
        run_self_test()
        return

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

    verify_release_binary(binary)

    if not arguments.offline:
        if workspace.exists():
            require(
                repository.is_dir(),
                f"demo workspace exists without repository.git: {workspace}",
            )
        else:
            workspace.parent.mkdir(parents=True, exist_ok=True)
            run([sys.executable, VERIFIER, "materialize", "--output", workspace])
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
    require(
        gate.returncode == 1,
        f"required check returned unexpected status {gate.returncode}: {gate.stderr.strip()}",
    )
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
        f"review delta gate is open: {expected['needs_review_now']} files need review"
    )
    require(
        expected_error in gate.stderr,
        f"required check did not report the expected residue: {gate.stderr.strip()}",
    )

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
