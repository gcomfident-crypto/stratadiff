#!/usr/bin/env python3
"""Materialize and evaluate the frozen Review Continuity v1 histories."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
BUNDLE = ROOT / "benchmarks/review-continuity-v1"
MANIFEST_PATH = BUNDLE / "manifest.json"
ORACLE_PATH = BUNDLE / "oracle.json"
MANIFEST_SCHEMA = "stratadiff-review-continuity-manifest-v1"
ORACLE_SCHEMA = "stratadiff-review-continuity-oracle-v1"
EVALUATION_SCHEMA = "stratadiff-review-continuity-evaluation-v1"
DELTA_SCHEMA = (
    "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/"
    "schema/review-delta-v1.schema.json"
)
METHODS = ("stratadiff", "git_patch_id", "checkpoint_to_head", "git_range_diff")
SNAPSHOTS = ("A", "B", "C", "D")
PARENTS = {"B": "A", "C": "A", "D": "C"}
REQUIRED_COVERAGE = {
    "pure_rebase",
    "author_followup",
    "dropped_reviewed_edit",
    "patch_id_whitespace_collision",
    "hazardous_parent_change",
    "stack_squash",
    "rename_and_edit",
}
CASE_ID_PATTERN = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
OID_PATTERN = re.compile(r"^[0-9a-f]{40}$")
RANGE_SUMMARY_PATTERN = re.compile(
    r"^(?:[0-9]+|-):\s+[0-9a-f-]+\s+([=<>!])\s+(?:[0-9]+|-):\s+[0-9a-f-]+"
)


class ContinuityError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContinuityError(message)


def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        require(key not in value, f"duplicate JSON key: {key}")
        value[key] = item
    return value


def load_json(path: Path) -> dict[str, object]:
    value = json.loads(path.read_bytes(), object_pairs_hook=unique_object)
    require(isinstance(value, dict), f"{path} must contain one JSON object")
    return value


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode(
        "utf-8"
    )


def atomic_write(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(canonical_json(value))
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def validate_relative_path(value: str, label: str) -> None:
    path = PurePosixPath(value)
    require(value != "", f"{label} is empty")
    require(not path.is_absolute(), f"{label} is absolute")
    require(".." not in path.parts, f"{label} escapes the repository")
    require("." not in path.parts, f"{label} is not normalized")
    require("\\" not in value, f"{label} must use Git separators")
    require(str(path) == value, f"{label} is not normalized")


def validate_method_result(value: dict[str, object], label: str) -> None:
    require(
        set(value) == {"classification", "review_paths", "attention_line_changes"},
        f"{label} has unexpected fields",
    )
    require(isinstance(value["classification"], str), f"{label} classification is invalid")
    paths = value["review_paths"]
    require(isinstance(paths, list), f"{label} review_paths must be a list")
    require(paths == sorted(set(paths)), f"{label} review_paths must be sorted and unique")
    for path in paths:
        validate_relative_path(path, f"{label} review path")
    line_changes = value["attention_line_changes"]
    require(
        isinstance(line_changes, int) and line_changes >= 0,
        f"{label} attention_line_changes is invalid",
    )


def validate_bundle(
    manifest: dict[str, object], oracle: dict[str, object]
) -> tuple[dict[str, dict[str, object]], dict[str, dict[str, object]]]:
    require(manifest["schema"] == MANIFEST_SCHEMA, "unsupported manifest schema")
    require(oracle["schema"] == ORACLE_SCHEMA, "unsupported oracle schema")
    require(manifest["dataset_version"] == "1.0.0", "unsupported dataset version")
    require(
        oracle["dataset_version"] == manifest["dataset_version"],
        "oracle dataset version differs",
    )
    require(manifest["dataset_license"] == "MIT", "unexpected dataset license")
    require(
        manifest["designation"] == "controlled_synthetic_comparative_regression",
        "unexpected benchmark designation",
    )
    require(
        set(manifest["required_coverage"]) == REQUIRED_COVERAGE,
        "required coverage differs from the v1 contract",
    )
    require(
        manifest["oracle"]["path"] == "benchmarks/review-continuity-v1/oracle.json",
        "manifest oracle path differs",
    )
    require(
        manifest["oracle"]["sha256"] == sha256_file(ORACLE_PATH),
        "oracle digest differs from the frozen manifest",
    )
    require(
        set(manifest["baseline_contracts"]) == set(METHODS),
        "baseline contracts do not match the evaluated methods",
    )
    require(len(oracle["claim_boundary"]) >= 5, "oracle claim boundary is incomplete")

    cases: dict[str, dict[str, object]] = {}
    coverage: set[str] = set()
    for case in manifest["cases"]:
        case_id = case["id"]
        require(CASE_ID_PATTERN.fullmatch(case_id) is not None, f"invalid case ID: {case_id}")
        require(case_id not in cases, f"duplicate manifest case: {case_id}")
        require(case["description"], f"{case_id} has no description")
        require(case["covers"], f"{case_id} has no coverage label")
        coverage.update(case["covers"])
        require(set(case["history"]) == set(SNAPSHOTS), f"{case_id} needs A/B/C/D")
        for snapshot in SNAPSHOTS:
            files = case["history"][snapshot]["files"]
            require(isinstance(files, dict) and files, f"{case_id}.{snapshot} files invalid")
            for path, content in files.items():
                validate_relative_path(path, f"{case_id}.{snapshot} path")
                require(isinstance(content, str), f"{case_id}.{snapshot}:{path} is not text")
        cases[case_id] = case
    require(coverage == REQUIRED_COVERAGE, "case coverage is incomplete or unexpected")

    oracles: dict[str, dict[str, object]] = {}
    for case_oracle in oracle["cases"]:
        case_id = case_oracle["id"]
        require(case_id not in oracles, f"duplicate oracle case: {case_id}")
        required = case_oracle["required_attention_paths"]
        carryable = case_oracle["carryable_paths"]
        require(required == sorted(set(required)), f"{case_id} required paths not canonical")
        require(carryable == sorted(set(carryable)), f"{case_id} carryable paths not canonical")
        require(not set(required) & set(carryable), f"{case_id} path has conflicting labels")
        for path in required + carryable:
            validate_relative_path(path, f"{case_id} oracle path")
        exact_lines = case_oracle["exact_residue_line_changes"]
        require(
            isinstance(exact_lines, int) and exact_lines >= 0,
            f"{case_id} exact residue lines invalid",
        )
        expected_methods = case_oracle["expected_methods"]
        require(set(expected_methods) == set(METHODS), f"{case_id} method set differs")
        for method in METHODS:
            validate_method_result(expected_methods[method], f"{case_id}.{method}")
        oracles[case_id] = case_oracle
    require(set(cases) == set(oracles), "manifest and oracle case sets differ")
    return cases, oracles


def isolated_environment(extra: dict[str, str] | None = None) -> dict[str, str]:
    environment = {
        name: value for name, value in os.environ.items() if not name.upper().startswith("GIT_")
    }
    environment["LC_ALL"] = "C"
    environment["GIT_CONFIG_GLOBAL"] = os.devnull
    environment["GIT_CONFIG_NOSYSTEM"] = "1"
    environment["GIT_NO_REPLACE_OBJECTS"] = "1"
    environment["GIT_NO_LAZY_FETCH"] = "1"
    environment["GIT_TERMINAL_PROMPT"] = "0"
    if extra is not None:
        environment.update(extra)
    return environment


def run_git(
    repository: Path,
    arguments: list[str],
    *,
    input_bytes: bytes | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", "--no-replace-objects", "-C", str(repository), *arguments],
        env=isolated_environment(),
        input=input_bytes,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=check,
    )


def git_text(repository: Path, arguments: list[str]) -> str:
    return run_git(repository, arguments).stdout.decode("utf-8").strip()


def clear_worktree(repository: Path) -> None:
    for child in repository.iterdir():
        if child.name == ".git":
            continue
        if child.is_dir() and not child.is_symlink():
            shutil.rmtree(child)
        else:
            child.unlink()


def write_snapshot(repository: Path, snapshot: dict[str, object]) -> None:
    clear_worktree(repository)
    for relative, content in sorted(snapshot["files"].items()):
        destination = repository.joinpath(*PurePosixPath(relative).parts)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(content, encoding="utf-8")


def commit_snapshot(
    repository: Path, case_id: str, case_index: int, snapshot_index: int, label: str
) -> str:
    run_git(repository, ["add", "--all"])
    timestamp = f"2002-01-{case_index + 1:02d}T{snapshot_index:02d}:00:00Z"
    identity = {
        "GIT_AUTHOR_NAME": "Review Continuity Benchmark",
        "GIT_AUTHOR_EMAIL": "continuity@stratadiff.invalid",
        "GIT_AUTHOR_DATE": timestamp,
        "GIT_COMMITTER_NAME": "Review Continuity Benchmark",
        "GIT_COMMITTER_EMAIL": "continuity@stratadiff.invalid",
        "GIT_COMMITTER_DATE": timestamp,
    }
    subject = "reviewed change" if label in ("B", "D") else f"snapshot {label}"
    subprocess.run(
        [
            "git",
            "--no-replace-objects",
            "-C",
            str(repository),
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            f"{case_id} {subject}",
        ],
        env=isolated_environment(identity),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    commit = git_text(repository, ["rev-parse", "HEAD"])
    require(OID_PATTERN.fullmatch(commit) is not None, f"{case_id} {label} OID invalid")
    run_git(repository, ["tag", f"snapshot-{label}", commit])
    return commit


def materialize_case(case: dict[str, object], case_index: int, root: Path) -> dict[str, object]:
    case_root = root / case["id"]
    repository = case_root / "repository"
    repository.mkdir(parents=True)
    run_git(repository, ["init", "-q"])
    snapshots: dict[str, str] = {}

    write_snapshot(repository, case["history"]["A"])
    snapshots["A"] = commit_snapshot(repository, case["id"], case_index, 0, "A")
    write_snapshot(repository, case["history"]["B"])
    snapshots["B"] = commit_snapshot(repository, case["id"], case_index, 1, "B")
    run_git(repository, ["checkout", "-q", "--detach", snapshots["A"]])
    write_snapshot(repository, case["history"]["C"])
    snapshots["C"] = commit_snapshot(repository, case["id"], case_index, 2, "C")
    write_snapshot(repository, case["history"]["D"])
    snapshots["D"] = commit_snapshot(repository, case["id"], case_index, 3, "D")

    verify_materialized(case, repository, snapshots)
    return {"id": case["id"], "repository": repository, "snapshots": snapshots}


def materialize(cases: list[dict[str, object]], root: Path) -> list[dict[str, object]]:
    require(not root.exists(), f"materialization path already exists: {root}")
    root.mkdir(parents=True)
    return [materialize_case(case, index, root) for index, case in enumerate(cases)]


def tree_files(repository: Path, commit: str) -> dict[str, str]:
    output = run_git(repository, ["ls-tree", "-r", "-z", commit]).stdout
    observed: dict[str, str] = {}
    for record in output.split(b"\0"):
        if not record:
            continue
        metadata, path_bytes = record.split(b"\t", 1)
        mode, kind, object_id = metadata.decode("ascii").split()
        require(mode == "100644" and kind == "blob", "benchmark tree is not regular text")
        content = run_git(repository, ["cat-file", "blob", object_id]).stdout.decode("utf-8")
        observed[path_bytes.decode("utf-8")] = content
    return observed


def verify_materialized(
    case: dict[str, object], repository: Path, snapshots: dict[str, str]
) -> None:
    for label in SNAPSHOTS:
        require(
            tree_files(repository, snapshots[label]) == case["history"][label]["files"],
            f"{case['id']} {label} tree differs from manifest",
        )
    require(
        git_text(repository, ["rev-list", "--parents", "-n", "1", snapshots["A"]]).split()
        == [snapshots["A"]],
        f"{case['id']} A is not a root commit",
    )
    for child, parent in PARENTS.items():
        require(
            git_text(
                repository, ["rev-list", "--parents", "-n", "1", snapshots[child]]
            ).split()
            == [snapshots[child], snapshots[parent]],
            f"{case['id']} {child} parent is not {parent}",
        )
    require(
        git_text(repository, ["merge-base", snapshots["B"], snapshots["D"]])
        == snapshots["A"],
        f"{case['id']} old merge base is not A",
    )
    require(
        git_text(repository, ["merge-base", snapshots["C"], snapshots["D"]])
        == snapshots["C"],
        f"{case['id']} current merge base is not C",
    )


def diff_scope(repository: Path, before: str, after: str) -> dict[str, object]:
    output = run_git(
        repository,
        ["diff", "--no-renames", "--numstat", "--format=", before, after, "--"],
    ).stdout.decode("utf-8")
    paths: list[str] = []
    line_changes = 0
    for line in output.splitlines():
        additions, deletions, path = line.split("\t", 2)
        require(additions != "-" and deletions != "-", "binary input is outside v1")
        validate_relative_path(path, "Git diff path")
        paths.append(path)
        line_changes += int(additions) + int(deletions)
    return {
        "paths": sorted(set(paths)),
        "line_changes": line_changes,
        "diff_sha256": sha256_bytes(
            run_git(
                repository,
                ["diff", "--no-renames", "--binary", "--full-index", before, after, "--"],
            ).stdout
        ),
    }


def patch_id(repository: Path, before: str, after: str) -> str | None:
    patch = run_git(
        repository,
        ["diff", "--no-renames", "--binary", "--full-index", before, after, "--"],
    ).stdout
    output = subprocess.run(
        ["git", "patch-id", "--stable"],
        env=isolated_environment(),
        input=patch,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    ).stdout.decode("ascii").strip()
    if output == "":
        return None
    fields = output.split()
    require(len(fields) == 2, "git patch-id returned an unexpected record")
    require(OID_PATTERN.fullmatch(fields[0]) is not None, "git patch-id returned an invalid ID")
    return fields[0]


def result(classification: str, scope: dict[str, object]) -> dict[str, object]:
    return {
        "classification": classification,
        "review_paths": scope["paths"],
        "attention_line_changes": scope["line_changes"],
    }


def empty_result(classification: str) -> dict[str, object]:
    return {
        "classification": classification,
        "review_paths": [],
        "attention_line_changes": 0,
    }


def evaluate_patch_id(
    repository: Path, snapshots: dict[str, str]
) -> tuple[dict[str, object], dict[str, object]]:
    reviewed = patch_id(repository, snapshots["A"], snapshots["B"])
    current = patch_id(repository, snapshots["C"], snapshots["D"])
    current_scope = diff_scope(repository, snapshots["C"], snapshots["D"])
    if reviewed == current:
        method = empty_result("equivalent_patch")
    else:
        method = result("different_patch", current_scope)
    return method, {
        "reviewed_patch_id": reviewed,
        "current_patch_id": current,
        "current_scope": current_scope,
    }


def evaluate_checkpoint_to_head(
    repository: Path, snapshots: dict[str, str]
) -> tuple[dict[str, object], dict[str, object]]:
    scope = diff_scope(repository, snapshots["B"], snapshots["D"])
    classification = "identical_trees" if not scope["paths"] else "different_trees"
    return result(classification, scope), {"scope": scope}


def evaluate_range_diff(
    repository: Path, snapshots: dict[str, str]
) -> tuple[dict[str, object], dict[str, object]]:
    output = run_git(
        repository,
        [
            "range-diff",
            "--no-color",
            "--no-dual-color",
            f"{snapshots['A']}..{snapshots['B']}",
            f"{snapshots['C']}..{snapshots['D']}",
        ],
    ).stdout
    markers = []
    for line in output.decode("utf-8").splitlines():
        match = RANGE_SUMMARY_PATTERN.match(line)
        if match is not None:
            markers.append(match.group(1))
    require(markers, "git range-diff returned no summary rows")
    old_scope = diff_scope(repository, snapshots["A"], snapshots["B"])
    current_scope = diff_scope(repository, snapshots["C"], snapshots["D"])
    if all(marker == "=" for marker in markers):
        method = empty_result("equivalent_series")
    else:
        method = {
            "classification": "changed_series",
            "review_paths": sorted(set(old_scope["paths"]) | set(current_scope["paths"])),
            "attention_line_changes": (
                old_scope["line_changes"] + current_scope["line_changes"]
            ),
        }
    return method, {
        "markers": markers,
        "output_sha256": sha256_bytes(output),
        "old_scope": old_scope,
        "current_scope": current_scope,
    }


def optional_field(mapping: dict[str, object], key: str) -> object | None:
    return mapping[key] if key in mapping else None


def evaluate_stratadiff(
    binary: Path, case_root: Path, repository: Path, snapshots: dict[str, str]
) -> tuple[dict[str, object], dict[str, object]]:
    report_path = case_root / "review.json"
    delta_path = case_root / "review-delta.json"
    command = [
        str(binary),
        "review",
        "--repo",
        str(repository),
        "--checkpoint",
        snapshots["B"],
        "--format=json",
        f"--output={report_path}",
        f"--review-delta-output={delta_path}",
        "--",
        snapshots["C"],
        snapshots["D"],
    ]
    completed = subprocess.run(
        command,
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    require(completed.stdout == b"", "StrataDiff unexpectedly wrote report bytes to stdout")
    delta_bytes = delta_path.read_bytes()
    delta = json.loads(delta_bytes, object_pairs_hook=unique_object)
    require(delta["schema"] == DELTA_SCHEMA, "StrataDiff delta schema differs")
    require(delta["old_base_commit"] == snapshots["A"], "StrataDiff old base is not A")
    require(delta["checkpoint_commit"] == snapshots["B"], "StrataDiff checkpoint is not B")
    require(delta["current_base_commit"] == snapshots["C"], "StrataDiff current base is not C")
    require(delta["head_commit"] == snapshots["D"], "StrataDiff head is not D")

    paths: set[str] = set()
    line_changes = 0
    baseline_bases: list[str] = []
    fallback_reasons: list[str] = []
    normalized_entries: list[dict[str, object]] = []
    for entry in delta["entries"]:
        file = entry["file"]
        if "before_path" in file:
            paths.add(file["before_path"])
        if "after_path" in file:
            paths.add(file["after_path"])
        envelope = file["line_change_envelope"]
        line_changes += envelope["additions"] + envelope["deletions"]
        baseline_bases.append(entry["baseline_basis"])
        fallback = optional_field(entry, "fallback_reason")
        if fallback is not None:
            fallback_reasons.append(fallback)
        normalized_entries.append(
            {
                "before_path": optional_field(file, "before_path"),
                "after_path": optional_field(file, "after_path"),
                "additions": envelope["additions"],
                "deletions": envelope["deletions"],
                "baseline_basis": entry["baseline_basis"],
                "fallback_reason": fallback,
            }
        )
    unresolved_paths = []
    for unresolved in delta["unresolved_retired_changes"]:
        paths.add(unresolved["path"])
        unresolved_paths.append(unresolved["path"])
    gate_passed = delta["summary"]["gate_passed"]
    classification = "carry_all" if gate_passed else "review_residue"
    method = {
        "classification": classification,
        "review_paths": sorted(paths),
        "attention_line_changes": line_changes,
    }

    gate_path = case_root / "gate.json"
    gate = subprocess.run(
        [
            str(binary),
            "review",
            "--repo",
            str(repository),
            "--checkpoint",
            snapshots["B"],
            "--format=json",
            f"--output={gate_path}",
            "--fail-on-review-residue",
            "--",
            snapshots["C"],
            snapshots["D"],
        ],
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    require(
        (gate.returncode == 0) == gate_passed,
        "StrataDiff gate exit contradicts its delta summary",
    )
    if not gate_passed:
        require(b"review delta gate is open" in gate.stderr, "gate failure omitted its reason")
    evidence = {
        "comparison": delta["comparison"],
        "delta_sha256": sha256_bytes(delta_bytes),
        "gate_exit_code": gate.returncode,
        "baseline_bases": sorted(baseline_bases),
        "fallback_reasons": sorted(fallback_reasons),
        "displayable_files": delta["summary"]["displayable_files"],
        "unresolved_retired_changes": delta["summary"]["unresolved_retired_changes"],
        "entries": normalized_entries,
        "unresolved_paths": sorted(unresolved_paths),
    }
    return method, evidence


def score_method(
    method: dict[str, object], case_oracle: dict[str, object]
) -> dict[str, object]:
    expected_paths = set(case_oracle["required_attention_paths"])
    observed_paths = set(method["review_paths"])
    missing = sorted(expected_paths - observed_paths)
    extra = sorted(observed_paths - expected_paths)
    exact_lines = case_oracle["exact_residue_line_changes"]
    observed_lines = method["attention_line_changes"]
    under_attention = max(0, exact_lines - observed_lines)
    false_carry = bool(missing or under_attention)
    avoidable = None
    if not false_carry:
        avoidable = max(0, observed_lines - exact_lines)
    return {
        "missing_required_paths": missing,
        "extra_attention_paths": extra,
        "under_attention_line_changes": under_attention,
        "false_carry": false_carry,
        "avoidable_line_changes": avoidable,
        "exact_path_and_line_match": (
            not missing and not extra and observed_lines == exact_lines
        ),
    }


def evaluate_case(
    binary: Path,
    case: dict[str, object],
    case_oracle: dict[str, object],
    materialized: dict[str, object],
) -> dict[str, object]:
    repository = materialized["repository"]
    snapshots = materialized["snapshots"]
    methods: dict[str, dict[str, object]] = {}
    evidence: dict[str, dict[str, object]] = {}
    methods["git_patch_id"], evidence["git_patch_id"] = evaluate_patch_id(
        repository, snapshots
    )
    methods["checkpoint_to_head"], evidence["checkpoint_to_head"] = (
        evaluate_checkpoint_to_head(repository, snapshots)
    )
    methods["git_range_diff"], evidence["git_range_diff"] = evaluate_range_diff(
        repository, snapshots
    )
    methods["stratadiff"], evidence["stratadiff"] = evaluate_stratadiff(
        binary, repository.parent, repository, snapshots
    )

    for method in METHODS:
        expected = case_oracle["expected_methods"][method]
        require(
            methods[method] == expected,
            f"{case['id']} {method} differs from frozen oracle: "
            f"observed={methods[method]} expected={expected}",
        )
    scores = {
        method: score_method(method_result, case_oracle)
        for method, method_result in methods.items()
    }
    return {
        "id": case["id"],
        "snapshots": snapshots,
        "oracle": {
            "required_attention_paths": case_oracle["required_attention_paths"],
            "carryable_paths": case_oracle["carryable_paths"],
            "exact_residue_line_changes": case_oracle["exact_residue_line_changes"],
        },
        "methods": methods,
        "scores": scores,
        "evidence": evidence,
    }


def aggregate(results: list[dict[str, object]]) -> dict[str, object]:
    methods: dict[str, dict[str, object]] = {}
    for method in METHODS:
        scores = [case["scores"][method] for case in results]
        method_results = [case["methods"][method] for case in results]
        avoidable_values = [
            score["avoidable_line_changes"]
            for score in scores
            if score["avoidable_line_changes"] is not None
        ]
        methods[method] = {
            "cases": len(results),
            "false_carry_cases": sum(1 for score in scores if score["false_carry"]),
            "missing_required_paths": sum(
                len(score["missing_required_paths"]) for score in scores
            ),
            "extra_attention_paths": sum(
                len(score["extra_attention_paths"]) for score in scores
            ),
            "exact_path_and_line_cases": sum(
                1 for score in scores if score["exact_path_and_line_match"]
            ),
            "attention_line_changes": sum(
                result_value["attention_line_changes"] for result_value in method_results
            ),
            "avoidable_line_changes_on_non_false_carry_cases": sum(avoidable_values),
        }
    return {"cases": len(results), "methods": methods}


def run_binary(binary: Path, arguments: list[str]) -> bytes:
    return subprocess.run(
        [str(binary), *arguments],
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    ).stdout


def command_self_test(
    manifest: dict[str, object], oracles: dict[str, dict[str, object]]
) -> None:
    with tempfile.TemporaryDirectory(prefix="stratadiff-review-continuity-self-test-") as temp:
        first_root = Path(temp) / "first"
        second_root = Path(temp) / "second"
        first = materialize(manifest["cases"], first_root)
        second = materialize(manifest["cases"], second_root)
        first_ids = {item["id"]: item["snapshots"] for item in first}
        second_ids = {item["id"]: item["snapshots"] for item in second}
        require(first_ids == second_ids, "materialized commit IDs are not deterministic")
        for item in first:
            case_id = item["id"]
            repository = item["repository"]
            snapshots = item["snapshots"]
            observed = {
                "git_patch_id": evaluate_patch_id(repository, snapshots)[0],
                "checkpoint_to_head": evaluate_checkpoint_to_head(repository, snapshots)[0],
                "git_range_diff": evaluate_range_diff(repository, snapshots)[0],
            }
            for method in observed:
                require(
                    observed[method] == oracles[case_id]["expected_methods"][method],
                    f"{case_id} {method} self-test differs from oracle: "
                    f"observed={observed[method]} "
                    f"expected={oracles[case_id]['expected_methods'][method]}",
                )
        whitespace = next(item for item in first if item["id"] == "whitespace-patch-id-collision")
        whitespace_evidence = evaluate_patch_id(
            whitespace["repository"], whitespace["snapshots"]
        )[1]
        require(
            whitespace_evidence["reviewed_patch_id"]
            == whitespace_evidence["current_patch_id"],
            "whitespace case does not collide under stable patch-id",
        )
        stack = next(item for item in first if item["id"] == "stack-squash-parent-hazard")
        require(
            git_text(stack["repository"], ["rev-parse", f"{stack['snapshots']['B']}^{{tree}}"])
            == git_text(
                stack["repository"], ["rev-parse", f"{stack['snapshots']['D']}^{{tree}}"]
            ),
            "stack hazard does not preserve the old reviewed head tree",
        )
        require(
            git_text(stack["repository"], ["rev-parse", f"{stack['snapshots']['C']}^{{tree}}"])
            != git_text(
                stack["repository"], ["rev-parse", f"{stack['snapshots']['D']}^{{tree}}"]
            ),
            "stack hazard has no parent-relative residue",
        )
    print(f"review-continuity-v1 self-test passed: {len(manifest['cases'])} histories")


def command_run(
    manifest: dict[str, object],
    oracles: dict[str, dict[str, object]],
    binary: Path,
    output: Path,
    workdir: Path | None,
    require_clean: bool,
) -> None:
    binary = binary.resolve()
    require(binary.is_file(), f"StrataDiff binary does not exist: {binary}")
    build_info = json.loads(run_binary(binary, ["build-info"]), object_pairs_hook=unique_object)
    require(build_info["schema"] == "stratadiff-build-info-v1", "invalid build-info schema")
    if require_clean:
        require(build_info["git_dirty"] is False, "--require-clean rejected a dirty build")
        require(build_info["build_profile"] == "release", "--require-clean needs release")

    if workdir is None:
        temporary = tempfile.TemporaryDirectory(prefix="stratadiff-review-continuity-run-")
        materialization_root = Path(temporary.name) / "materialized"
    else:
        temporary = None
        materialization_root = workdir.resolve()
    try:
        materialized = materialize(manifest["cases"], materialization_root)
        materialized_by_id = {item["id"]: item for item in materialized}
        results = [
            evaluate_case(
                binary,
                case,
                oracles[case["id"]],
                materialized_by_id[case["id"]],
            )
            for case in manifest["cases"]
        ]
        evaluation = {
            "schema": EVALUATION_SCHEMA,
            "dataset_version": manifest["dataset_version"],
            "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
            "manifest_sha256": sha256_file(MANIFEST_PATH),
            "oracle_sha256": sha256_file(ORACLE_PATH),
            "runner_sha256": sha256_file(Path(__file__).resolve()),
            "provenance": {
                "binary_path": str(binary),
                "binary_sha256": sha256_file(binary),
                "build_info": build_info,
                "git_version": subprocess.run(
                    ["git", "--version"],
                    env=isolated_environment(),
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=True,
                ).stdout.decode("utf-8").strip(),
                "require_clean": require_clean,
            },
            "cases": results,
            "aggregate": aggregate(results),
        }
        atomic_write(output.resolve(), evaluation)
    finally:
        if temporary is not None:
            temporary.cleanup()
    print(f"wrote {len(results)}-case evaluation to {output.resolve()}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("validate", help="validate the frozen manifest and oracle")
    subparsers.add_parser("self-test", help="prove deterministic histories and baselines")
    run_parser = subparsers.add_parser("run", help="evaluate a StrataDiff binary")
    run_parser.add_argument("--stratadiff", type=Path, required=True)
    run_parser.add_argument("--output", type=Path, required=True)
    run_parser.add_argument("--workdir", type=Path)
    run_parser.add_argument("--require-clean", action="store_true")
    arguments = parser.parse_args()

    manifest = load_json(MANIFEST_PATH)
    oracle = load_json(ORACLE_PATH)
    _, oracles = validate_bundle(manifest, oracle)
    if arguments.command == "validate":
        print(f"review-continuity-v1 bundle valid: {len(manifest['cases'])} cases")
    elif arguments.command == "self-test":
        command_self_test(manifest, oracles)
    else:
        command_run(
            manifest,
            oracles,
            arguments.stratadiff,
            arguments.output,
            arguments.workdir,
            arguments.require_clean,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
