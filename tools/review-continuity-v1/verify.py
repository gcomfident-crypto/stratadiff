#!/usr/bin/env python3
"""Offline verifier and tamper tests for Review Continuity v1 evaluations."""

from __future__ import annotations

import argparse
import copy
import difflib
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import tempfile


ROOT = Path(__file__).resolve().parents[2]
BUNDLE = ROOT / "benchmarks/review-continuity-v1"
MANIFEST_PATH = BUNDLE / "manifest.json"
ORACLE_PATH = BUNDLE / "oracle.json"
CHECKSUM_PATH = BUNDLE / "SHA256SUMS"
RUNNER_PATH = Path(__file__).resolve().parent / "run.py"
VERIFIER_PATH = Path(__file__).resolve()
MANIFEST_SCHEMA = "stratadiff-review-continuity-manifest-v1"
ORACLE_SCHEMA = "stratadiff-review-continuity-oracle-v1"
EVALUATION_SCHEMA = "stratadiff-review-continuity-evaluation-v1"
METHODS = ("stratadiff", "git_patch_id", "checkpoint_to_head", "git_range_diff")
SNAPSHOTS = ("A", "B", "C", "D")
OID_PATTERN = re.compile(r"^[0-9a-f]{40}$")
DIGEST_PATTERN = re.compile(r"^[0-9a-f]{64}$")
EXPECTED_CHECKSUM_PATHS = {
    "README.md": BUNDLE / "README.md",
    "manifest.json": MANIFEST_PATH,
    "oracle.json": ORACLE_PATH,
    "../../tools/review-continuity-v1/run.py": RUNNER_PATH,
    "../../tools/review-continuity-v1/verify.py": VERIFIER_PATH,
}


class VerificationError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise VerificationError(message)


def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        require(key not in value, f"duplicate JSON key: {key}")
        value[key] = item
    return value


def decode_json(payload: bytes, label: str) -> dict[str, object]:
    value = json.loads(payload, object_pairs_hook=unique_object)
    require(isinstance(value, dict), f"{label} must contain a JSON object")
    return value


def load_json(path: Path) -> dict[str, object]:
    return decode_json(path.read_bytes(), str(path))


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode(
        "utf-8"
    )


def validate_relative_path(value: str, label: str) -> None:
    path = PurePosixPath(value)
    require(value != "", f"{label} is empty")
    require(not path.is_absolute(), f"{label} is absolute")
    require(".." not in path.parts, f"{label} escapes its root")
    require("." not in path.parts, f"{label} is not normalized")
    require("\\" not in value, f"{label} contains a backslash")
    require(str(path) == value, f"{label} is not normalized")


def validate_bundle_bytes(
    manifest_bytes: bytes, oracle_bytes: bytes
) -> tuple[dict[str, object], dict[str, object], dict[str, dict[str, object]]]:
    manifest = decode_json(manifest_bytes, "manifest")
    oracle = decode_json(oracle_bytes, "oracle")
    require(manifest["schema"] == MANIFEST_SCHEMA, "manifest schema differs")
    require(oracle["schema"] == ORACLE_SCHEMA, "oracle schema differs")
    require(manifest["dataset_version"] == "1.0.0", "manifest version differs")
    require(
        oracle["dataset_version"] == manifest["dataset_version"],
        "oracle version differs",
    )
    require(
        manifest["designation"] == "controlled_synthetic_comparative_regression",
        "manifest designation differs",
    )
    require(
        manifest["oracle"]["path"] == "benchmarks/review-continuity-v1/oracle.json",
        "manifest oracle path differs",
    )
    require(
        manifest["oracle"]["sha256"] == sha256_bytes(oracle_bytes),
        "oracle digest differs from manifest",
    )
    require(
        set(manifest["baseline_contracts"]) == set(METHODS),
        "manifest baseline method set differs",
    )
    manifest_ids = []
    for case in manifest["cases"]:
        case_id = case["id"]
        require(case_id not in manifest_ids, f"duplicate manifest case: {case_id}")
        manifest_ids.append(case_id)
    oracle_by_id: dict[str, dict[str, object]] = {}
    for case in oracle["cases"]:
        case_id = case["id"]
        require(case_id not in oracle_by_id, f"duplicate oracle case: {case_id}")
        required_paths = case["required_attention_paths"]
        carryable_paths = case["carryable_paths"]
        require(
            required_paths == sorted(set(required_paths)),
            f"{case_id} required paths are not sorted and unique",
        )
        require(
            carryable_paths == sorted(set(carryable_paths)),
            f"{case_id} carryable paths are not sorted and unique",
        )
        require(
            not set(required_paths) & set(carryable_paths),
            f"{case_id} has conflicting oracle paths",
        )
        for path in required_paths + carryable_paths:
            validate_relative_path(path, f"{case_id} oracle path")
        exact_lines = case["exact_residue_line_changes"]
        require(
            isinstance(exact_lines, int) and exact_lines >= 0,
            f"{case_id} exact residue lines invalid",
        )
        require(set(case["expected_methods"]) == set(METHODS), f"{case_id} methods differ")
        for method in METHODS:
            validate_method_result(
                case["expected_methods"][method], f"{case_id}.{method} oracle"
            )
        oracle_by_id[case_id] = case
    require(manifest_ids == list(oracle_by_id), "manifest/oracle case order differs")
    return manifest, oracle, oracle_by_id


def validate_method_result(value: dict[str, object], label: str) -> None:
    require(
        set(value) == {"classification", "review_paths", "attention_line_changes"},
        f"{label} fields differ",
    )
    require(isinstance(value["classification"], str), f"{label} classification invalid")
    paths = value["review_paths"]
    require(paths == sorted(set(paths)), f"{label} paths are not sorted and unique")
    for path in paths:
        validate_relative_path(path, f"{label} path")
    require(
        isinstance(value["attention_line_changes"], int)
        and value["attention_line_changes"] >= 0,
        f"{label} line count invalid",
    )


def expected_score(
    method: dict[str, object], case_oracle: dict[str, object]
) -> dict[str, object]:
    required = set(case_oracle["required_attention_paths"])
    observed = set(method["review_paths"])
    missing = sorted(required - observed)
    extra = sorted(observed - required)
    exact_lines = case_oracle["exact_residue_line_changes"]
    observed_lines = method["attention_line_changes"]
    under_attention = max(0, exact_lines - observed_lines)
    false_carry = bool(missing or under_attention)
    avoidable = None if false_carry else max(0, observed_lines - exact_lines)
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


def expected_aggregate(cases: list[dict[str, object]]) -> dict[str, object]:
    methods: dict[str, dict[str, object]] = {}
    for method in METHODS:
        scores = [case["scores"][method] for case in cases]
        results = [case["methods"][method] for case in cases]
        avoidable = [
            score["avoidable_line_changes"]
            for score in scores
            if score["avoidable_line_changes"] is not None
        ]
        methods[method] = {
            "cases": len(cases),
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
                result["attention_line_changes"] for result in results
            ),
            "avoidable_line_changes_on_non_false_carry_cases": sum(avoidable),
        }
    return {"cases": len(cases), "methods": methods}


def validate_hex_digest(value: str, label: str) -> None:
    require(DIGEST_PATTERN.fullmatch(value) is not None, f"{label} is not SHA-256")


def independent_line_changes(before: str, after: str) -> int:
    matcher = difflib.SequenceMatcher(
        None,
        before.splitlines(keepends=True),
        after.splitlines(keepends=True),
        autojunk=False,
    )
    total = 0
    for tag, before_start, before_end, after_start, after_end in matcher.get_opcodes():
        if tag != "equal":
            total += before_end - before_start
            total += after_end - after_start
    return total


def independent_scope(
    manifest_case: dict[str, object], before_label: str, after_label: str
) -> dict[str, object]:
    before_files = manifest_case["history"][before_label]["files"]
    after_files = manifest_case["history"][after_label]["files"]
    paths = []
    line_changes = 0
    for path in sorted(set(before_files) | set(after_files)):
        before = before_files[path] if path in before_files else ""
        after = after_files[path] if path in after_files else ""
        if before == after:
            continue
        paths.append(path)
        line_changes += independent_line_changes(before, after)
    return {"paths": paths, "line_changes": line_changes}


def validate_scope(
    value: dict[str, object], expected: dict[str, object], label: str
) -> None:
    require(set(value) == {"paths", "line_changes", "diff_sha256"}, f"{label} fields differ")
    require(value["paths"] == expected["paths"], f"{label} paths differ from manifest bytes")
    require(
        value["line_changes"] == expected["line_changes"],
        f"{label} line count differs from manifest bytes",
    )
    validate_hex_digest(value["diff_sha256"], f"{label} diff")


def validate_evidence(
    manifest_case: dict[str, object], methods: dict[str, object], evidence: dict[str, object]
) -> None:
    case_id = manifest_case["id"]
    require(set(evidence) == set(METHODS), f"{case_id} evidence methods differ")
    patch = evidence["git_patch_id"]
    require(
        set(patch) == {"reviewed_patch_id", "current_patch_id", "current_scope"},
        f"{case_id} patch-id evidence fields differ",
    )
    for key in ("reviewed_patch_id", "current_patch_id"):
        value = patch[key]
        require(
            value is None or OID_PATTERN.fullmatch(value) is not None,
            f"{case_id} {key} invalid",
        )
    patch_equal = patch["reviewed_patch_id"] == patch["current_patch_id"]
    current_scope = independent_scope(manifest_case, "C", "D")
    validate_scope(patch["current_scope"], current_scope, f"{case_id} patch-id scope")
    expected_patch_method = (
        {
            "classification": "equivalent_patch",
            "review_paths": [],
            "attention_line_changes": 0,
        }
        if patch_equal
        else {
            "classification": "different_patch",
            "review_paths": current_scope["paths"],
            "attention_line_changes": current_scope["line_changes"],
        }
    )
    require(
        methods["git_patch_id"] == expected_patch_method,
        f"{case_id} patch-id method contradicts evidence",
    )

    checkpoint = evidence["checkpoint_to_head"]
    require(set(checkpoint) == {"scope"}, f"{case_id} checkpoint evidence fields differ")
    checkpoint_scope = independent_scope(manifest_case, "B", "D")
    validate_scope(
        checkpoint["scope"], checkpoint_scope, f"{case_id} checkpoint-to-head scope"
    )
    checkpoint_empty = checkpoint_scope["paths"] == []
    expected_checkpoint_method = {
        "classification": "identical_trees" if checkpoint_empty else "different_trees",
        "review_paths": checkpoint_scope["paths"],
        "attention_line_changes": checkpoint_scope["line_changes"],
    }
    require(
        methods["checkpoint_to_head"] == expected_checkpoint_method,
        f"{case_id} checkpoint method contradicts evidence",
    )

    range_evidence = evidence["git_range_diff"]
    require(
        set(range_evidence) == {"markers", "output_sha256", "old_scope", "current_scope"},
        f"{case_id} range-diff evidence fields differ",
    )
    require(range_evidence["markers"], f"{case_id} range-diff markers empty")
    require(
        all(marker in ("=", "<", ">", "!") for marker in range_evidence["markers"]),
        f"{case_id} range-diff marker invalid",
    )
    validate_hex_digest(range_evidence["output_sha256"], f"{case_id} range-diff output")
    old_scope = independent_scope(manifest_case, "A", "B")
    validate_scope(range_evidence["old_scope"], old_scope, f"{case_id} old range scope")
    validate_scope(
        range_evidence["current_scope"], current_scope, f"{case_id} current range scope"
    )
    range_equal = all(marker == "=" for marker in range_evidence["markers"])
    expected_range_method = (
        {
            "classification": "equivalent_series",
            "review_paths": [],
            "attention_line_changes": 0,
        }
        if range_equal
        else {
            "classification": "changed_series",
            "review_paths": sorted(set(old_scope["paths"]) | set(current_scope["paths"])),
            "attention_line_changes": (
                old_scope["line_changes"] + current_scope["line_changes"]
            ),
        }
    )
    require(
        methods["git_range_diff"] == expected_range_method,
        f"{case_id} range-diff method contradicts evidence",
    )

    stratadiff = evidence["stratadiff"]
    require(
        set(stratadiff)
        == {
            "comparison",
            "delta_sha256",
            "gate_exit_code",
            "baseline_bases",
            "fallback_reasons",
            "displayable_files",
            "unresolved_retired_changes",
            "entries",
            "unresolved_paths",
        },
        f"{case_id} StrataDiff evidence fields differ",
    )
    require(
        stratadiff["comparison"]
        in ("checkpoint_to_head", "per_file_review_baseline_to_head"),
        f"{case_id} StrataDiff comparison invalid",
    )
    validate_hex_digest(stratadiff["delta_sha256"], f"{case_id} StrataDiff delta")
    gate_passed = stratadiff["gate_exit_code"] == 0
    require(
        gate_passed == (methods["stratadiff"]["classification"] == "carry_all"),
        f"{case_id} StrataDiff gate contradicts classification",
    )
    require(
        isinstance(stratadiff["displayable_files"], int)
        and stratadiff["displayable_files"] >= 0,
        f"{case_id} displayable count invalid",
    )
    require(
        isinstance(stratadiff["unresolved_retired_changes"], int)
        and stratadiff["unresolved_retired_changes"] >= 0,
        f"{case_id} unresolved count invalid",
    )
    require(
        stratadiff["displayable_files"] == len(stratadiff["entries"]),
        f"{case_id} displayable count contradicts entries",
    )
    require(
        stratadiff["unresolved_retired_changes"] == len(stratadiff["unresolved_paths"]),
        f"{case_id} unresolved count contradicts paths",
    )
    require(
        stratadiff["unresolved_paths"] == sorted(set(stratadiff["unresolved_paths"])),
        f"{case_id} unresolved paths are not sorted and unique",
    )
    observed_paths = set(stratadiff["unresolved_paths"])
    observed_lines = 0
    observed_bases = []
    observed_fallbacks = []
    for index, entry in enumerate(stratadiff["entries"]):
        require(
            set(entry)
            == {
                "before_path",
                "after_path",
                "additions",
                "deletions",
                "baseline_basis",
                "fallback_reason",
            },
            f"{case_id} StrataDiff entry {index} fields differ",
        )
        require(
            entry["before_path"] is not None or entry["after_path"] is not None,
            f"{case_id} StrataDiff entry {index} has no path",
        )
        for key in ("before_path", "after_path"):
            if entry[key] is not None:
                validate_relative_path(entry[key], f"{case_id} StrataDiff {key}")
                observed_paths.add(entry[key])
        for key in ("additions", "deletions"):
            require(
                isinstance(entry[key], int) and entry[key] >= 0,
                f"{case_id} StrataDiff entry {index} {key} invalid",
            )
            observed_lines += entry[key]
        require(isinstance(entry["baseline_basis"], str), "baseline basis invalid")
        observed_bases.append(entry["baseline_basis"])
        if entry["fallback_reason"] is not None:
            require(isinstance(entry["fallback_reason"], str), "fallback reason invalid")
            observed_fallbacks.append(entry["fallback_reason"])
    require(
        stratadiff["baseline_bases"] == sorted(observed_bases),
        f"{case_id} StrataDiff baseline basis summary differs",
    )
    require(
        stratadiff["fallback_reasons"] == sorted(observed_fallbacks),
        f"{case_id} StrataDiff fallback summary differs",
    )
    expected_stratadiff_method = {
        "classification": "carry_all" if gate_passed else "review_residue",
        "review_paths": sorted(observed_paths),
        "attention_line_changes": observed_lines,
    }
    require(
        methods["stratadiff"] == expected_stratadiff_method,
        f"{case_id} StrataDiff method contradicts normalized delta entries",
    )


def verify_evaluation_bytes(
    evaluation_bytes: bytes,
    manifest_bytes: bytes,
    oracle_bytes: bytes,
) -> dict[str, object]:
    manifest, _, oracle_by_id = validate_bundle_bytes(manifest_bytes, oracle_bytes)
    evaluation = decode_json(evaluation_bytes, "evaluation")
    require(
        set(evaluation)
        == {
            "schema",
            "dataset_version",
            "generated_at",
            "manifest_sha256",
            "oracle_sha256",
            "runner_sha256",
            "provenance",
            "cases",
            "aggregate",
        },
        "evaluation top-level fields differ",
    )
    require(evaluation["schema"] == EVALUATION_SCHEMA, "evaluation schema differs")
    require(
        evaluation["dataset_version"] == manifest["dataset_version"],
        "evaluation version differs",
    )
    require(
        evaluation["manifest_sha256"] == sha256_bytes(manifest_bytes),
        "evaluation manifest digest differs",
    )
    require(
        evaluation["oracle_sha256"] == sha256_bytes(oracle_bytes),
        "evaluation oracle digest differs",
    )
    require(
        evaluation["runner_sha256"] == sha256_file(RUNNER_PATH),
        "evaluation runner digest differs",
    )

    provenance = evaluation["provenance"]
    require(
        set(provenance)
        == {"binary_path", "binary_sha256", "build_info", "git_version", "require_clean"},
        "provenance fields differ",
    )
    validate_hex_digest(provenance["binary_sha256"], "binary digest")
    require(provenance["git_version"].startswith("git version "), "Git version invalid")
    build_info = provenance["build_info"]
    require(build_info["schema"] == "stratadiff-build-info-v1", "build-info schema differs")
    if provenance["require_clean"]:
        require(build_info["git_dirty"] is False, "clean evaluation reports a dirty build")
        require(build_info["build_profile"] == "release", "clean evaluation is not release")

    expected_ids = [case["id"] for case in manifest["cases"]]
    manifest_by_id = {case["id"]: case for case in manifest["cases"]}
    observed_ids = [case["id"] for case in evaluation["cases"]]
    require(observed_ids == expected_ids, "evaluation case order or membership differs")
    for case in evaluation["cases"]:
        case_id = case["id"]
        require(
            set(case) == {"id", "snapshots", "oracle", "methods", "scores", "evidence"},
            f"{case_id} result fields differ",
        )
        require(set(case["snapshots"]) == set(SNAPSHOTS), f"{case_id} snapshots differ")
        require(
            len(set(case["snapshots"].values())) == len(SNAPSHOTS),
            f"{case_id} snapshot IDs are not distinct",
        )
        for label in SNAPSHOTS:
            require(
                OID_PATTERN.fullmatch(case["snapshots"][label]) is not None,
                f"{case_id} {label} OID invalid",
            )
        case_oracle = oracle_by_id[case_id]
        expected_oracle_echo = {
            "required_attention_paths": case_oracle["required_attention_paths"],
            "carryable_paths": case_oracle["carryable_paths"],
            "exact_residue_line_changes": case_oracle["exact_residue_line_changes"],
        }
        require(case["oracle"] == expected_oracle_echo, f"{case_id} oracle echo differs")
        require(set(case["methods"]) == set(METHODS), f"{case_id} method set differs")
        require(set(case["scores"]) == set(METHODS), f"{case_id} score set differs")
        for method in METHODS:
            validate_method_result(case["methods"][method], f"{case_id}.{method}")
            require(
                case["methods"][method] == case_oracle["expected_methods"][method],
                f"{case_id} {method} differs from frozen oracle",
            )
            require(
                case["scores"][method]
                == expected_score(case["methods"][method], case_oracle),
                f"{case_id} {method} score differs",
            )
        validate_evidence(manifest_by_id[case_id], case["methods"], case["evidence"])
    require(
        evaluation["aggregate"] == expected_aggregate(evaluation["cases"]),
        "evaluation aggregate differs from independently recomputed values",
    )
    return evaluation


def parse_checksums(payload: bytes) -> dict[str, str]:
    records: dict[str, str] = {}
    for line in payload.decode("utf-8").splitlines():
        digest, relative = line.split("  ", 1)
        require(DIGEST_PATTERN.fullmatch(digest) is not None, "checksum digest invalid")
        require(relative not in records, f"duplicate checksum path: {relative}")
        records[relative] = digest
    require(set(records) == set(EXPECTED_CHECKSUM_PATHS), "checksum path set differs")
    return records


def verify_checksums(payload: bytes) -> None:
    records = parse_checksums(payload)
    for relative, path in EXPECTED_CHECKSUM_PATHS.items():
        require(records[relative] == sha256_file(path), f"checksum mismatch: {relative}")


def fake_scope(scope: dict[str, object], digest_character: str) -> dict[str, object]:
    return {
        "paths": scope["paths"],
        "line_changes": scope["line_changes"],
        "diff_sha256": digest_character * 64,
    }


def fake_evidence(
    manifest_case: dict[str, object], methods: dict[str, object]
) -> dict[str, object]:
    patch_equal = methods["git_patch_id"]["classification"] == "equivalent_patch"
    range_equal = methods["git_range_diff"]["classification"] == "equivalent_series"
    carry = methods["stratadiff"]["classification"] == "carry_all"
    current_scope = independent_scope(manifest_case, "C", "D")
    checkpoint_scope = independent_scope(manifest_case, "B", "D")
    old_scope = independent_scope(manifest_case, "A", "B")
    entries = []
    remaining_lines = methods["stratadiff"]["attention_line_changes"]
    for index, path in enumerate(methods["stratadiff"]["review_paths"]):
        line_changes = remaining_lines if index == 0 else 0
        remaining_lines -= line_changes
        entries.append(
            {
                "before_path": path,
                "after_path": path,
                "additions": line_changes,
                "deletions": 0,
                "baseline_basis": "synthetic_self_test",
                "fallback_reason": None,
            }
        )
    return {
        "git_patch_id": {
            "reviewed_patch_id": "a" * 40,
            "current_patch_id": "a" * 40 if patch_equal else "b" * 40,
            "current_scope": fake_scope(current_scope, "1"),
        },
        "checkpoint_to_head": {"scope": fake_scope(checkpoint_scope, "2")},
        "git_range_diff": {
            "markers": ["="] if range_equal else ["!"],
            "output_sha256": "d" * 64,
            "old_scope": fake_scope(old_scope, "3"),
            "current_scope": fake_scope(current_scope, "4"),
        },
        "stratadiff": {
            "comparison": "per_file_review_baseline_to_head",
            "delta_sha256": "e" * 64,
            "gate_exit_code": 0 if carry else 1,
            "baseline_bases": sorted(
                entry["baseline_basis"] for entry in entries
            ),
            "fallback_reasons": [],
            "displayable_files": len(entries),
            "unresolved_retired_changes": 0,
            "entries": entries,
            "unresolved_paths": [],
        },
    }


def synthetic_evaluation(
    manifest: dict[str, object], oracle_by_id: dict[str, dict[str, object]]
) -> dict[str, object]:
    cases = []
    for case_index, case in enumerate(manifest["cases"]):
        case_id = case["id"]
        case_oracle = oracle_by_id[case_id]
        methods = copy.deepcopy(case_oracle["expected_methods"])
        scores = {
            method: expected_score(methods[method], case_oracle) for method in METHODS
        }
        snapshots = {
            label: f"{case_index * 4 + label_index + 1:040x}"
            for label_index, label in enumerate(SNAPSHOTS)
        }
        cases.append(
            {
                "id": case_id,
                "snapshots": snapshots,
                "oracle": {
                    "required_attention_paths": case_oracle["required_attention_paths"],
                    "carryable_paths": case_oracle["carryable_paths"],
                    "exact_residue_line_changes": case_oracle["exact_residue_line_changes"],
                },
                "methods": methods,
                "scores": scores,
                "evidence": fake_evidence(case, methods),
            }
        )
    return {
        "schema": EVALUATION_SCHEMA,
        "dataset_version": manifest["dataset_version"],
        "generated_at": "2000-01-01T00:00:00Z",
        "manifest_sha256": sha256_file(MANIFEST_PATH),
        "oracle_sha256": sha256_file(ORACLE_PATH),
        "runner_sha256": sha256_file(RUNNER_PATH),
        "provenance": {
            "binary_path": "/synthetic/stratadiff",
            "binary_sha256": "f" * 64,
            "build_info": {
                "schema": "stratadiff-build-info-v1",
                "engine_version": "self-test",
                "git_revision": "0" * 40,
                "git_dirty": True,
                "cargo_lock_sha256": "0" * 64,
                "build_profile": "self-test",
                "rustc_version": "self-test",
            },
            "git_version": "git version self-test",
            "require_clean": False,
        },
        "cases": cases,
        "aggregate": expected_aggregate(cases),
    }


def expect_rejected(action, expected_fragment: str) -> None:
    try:
        action()
    except (VerificationError, json.JSONDecodeError) as error:
        require(
            expected_fragment in str(error),
            f"tamper failed for the wrong reason: {error}",
        )
        return
    raise VerificationError(f"tamper was accepted; expected: {expected_fragment}")


def command_self_test() -> None:
    manifest_bytes = MANIFEST_PATH.read_bytes()
    oracle_bytes = ORACLE_PATH.read_bytes()
    manifest, _, oracle_by_id = validate_bundle_bytes(manifest_bytes, oracle_bytes)
    evaluation = synthetic_evaluation(manifest, oracle_by_id)
    verify_evaluation_bytes(canonical_json(evaluation), manifest_bytes, oracle_bytes)

    mutated_oracle = oracle_bytes.replace(b"six controlled", b"SIX controlled", 1)
    expect_rejected(
        lambda: validate_bundle_bytes(manifest_bytes, mutated_oracle),
        "oracle digest differs",
    )

    duplicate_key = canonical_json(evaluation).replace(
        b'{\n  "aggregate"', b'{\n  "schema": "duplicate",\n  "aggregate"', 1
    )
    expect_rejected(
        lambda: verify_evaluation_bytes(duplicate_key, manifest_bytes, oracle_bytes),
        "duplicate JSON key",
    )

    forged = copy.deepcopy(evaluation)
    forged_case = forged["cases"][2]
    forged_case["methods"]["git_patch_id"]["review_paths"] = ["feature.py"]
    forged_case["methods"]["git_patch_id"]["attention_line_changes"] = 2
    forged_case["scores"]["git_patch_id"] = expected_score(
        forged_case["methods"]["git_patch_id"], oracle_by_id[forged_case["id"]]
    )
    forged["aggregate"] = expected_aggregate(forged["cases"])
    expect_rejected(
        lambda: verify_evaluation_bytes(
            canonical_json(forged), manifest_bytes, oracle_bytes
        ),
        "differs from frozen oracle",
    )

    omitted = copy.deepcopy(evaluation)
    omitted["cases"].pop()
    omitted["aggregate"] = expected_aggregate(omitted["cases"])
    expect_rejected(
        lambda: verify_evaluation_bytes(
            canonical_json(omitted), manifest_bytes, oracle_bytes
        ),
        "case order or membership differs",
    )

    contradictory = copy.deepcopy(evaluation)
    contradictory["cases"][0]["evidence"]["git_range_diff"]["markers"] = ["!"]
    expect_rejected(
        lambda: verify_evaluation_bytes(
            canonical_json(contradictory), manifest_bytes, oracle_bytes
        ),
        "range-diff method contradicts evidence",
    )

    extra_field = copy.deepcopy(evaluation)
    extra_field["unfrozen"] = True
    expect_rejected(
        lambda: verify_evaluation_bytes(
            canonical_json(extra_field), manifest_bytes, oracle_bytes
        ),
        "top-level fields differ",
    )

    checksum_records = {
        relative: sha256_file(path) for relative, path in EXPECTED_CHECKSUM_PATHS.items()
    }
    checksum_payload = "".join(
        f"{checksum_records[relative]}  {relative}\n"
        for relative in sorted(checksum_records)
    ).encode("utf-8")
    verify_checksums(checksum_payload)
    tampered_checksums = checksum_payload.replace(
        checksum_records["oracle.json"].encode("ascii"), b"0" * 64, 1
    )
    expect_rejected(lambda: verify_checksums(tampered_checksums), "checksum mismatch")
    print("review-continuity-v1 verifier self-test passed: 7 tamper classes rejected")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    verify_parser = subparsers.add_parser("verify", help="verify one saved evaluation")
    verify_parser.add_argument("--evaluation", type=Path, required=True)
    subparsers.add_parser("verify-bundle", help="verify the frozen bundle checksums")
    subparsers.add_parser("self-test", help="run semantic and checksum tamper tests")
    arguments = parser.parse_args()

    if arguments.command == "verify-bundle":
        verify_checksums(CHECKSUM_PATH.read_bytes())
        manifest, _, _ = validate_bundle_bytes(
            MANIFEST_PATH.read_bytes(), ORACLE_PATH.read_bytes()
        )
        print(f"review-continuity-v1 bundle verified: {len(manifest['cases'])} cases")
    elif arguments.command == "self-test":
        command_self_test()
    else:
        evaluation = verify_evaluation_bytes(
            arguments.evaluation.read_bytes(),
            MANIFEST_PATH.read_bytes(),
            ORACLE_PATH.read_bytes(),
        )
        print(
            "review-continuity-v1 evaluation verified: "
            f"{len(evaluation['cases'])} cases"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
