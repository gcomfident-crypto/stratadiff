#!/usr/bin/env python3
"""Freeze and verify the post-outcome Review Memory Audit regression bundle."""

from __future__ import annotations

import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import tempfile


ROOT = Path(__file__).resolve().parents[2]
BUNDLE = Path(__file__).resolve().parent
PROTOCOL_PATH = BUNDLE / "protocol.json"
GOLDEN_PATH = BUNDLE / "golden-v1.json"
CENSUS_TOOL_PATH = ROOT / "tools/review-churn-census/review_churn_census.py"

PROTOCOL_SCHEMA = "stratadiff-review-memory-audit-regression-protocol-v1"
GOLDEN_SCHEMA = "stratadiff-review-memory-audit-regression-golden-v1"
DATASET_VERSION = "1.0.0"
OPAQUE_ACTOR_PATTERN = re.compile(r"^actor-[0-9a-f]{24}$")
FORBIDDEN_KEYS = {
    "authorization",
    "avatar",
    "avatarurl",
    "body",
    "diff",
    "email",
    "login",
    "patch",
    "source",
    "token",
}


class AuditRegressionError(RuntimeError):
    """A frozen regression artifact or invariant is invalid."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AuditRegressionError(message)


def unique_json_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        require(key not in value, f"duplicate JSON key: {key}")
        value[key] = item
    return value


def canonical_json(value: object) -> bytes:
    return (
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def read_canonical_json(path: Path) -> tuple[bytes, dict[str, object]]:
    payload = path.read_bytes()
    value = json.loads(payload, object_pairs_hook=unique_json_object)
    require(isinstance(value, dict), f"{path} must contain a JSON object")
    require(payload == canonical_json(value), f"{path} is not canonical JSON")
    return payload, value


def atomic_write(path: Path, payload: bytes) -> None:
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(payload)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        directory_descriptor = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory_descriptor)
        finally:
            os.close(directory_descriptor)
    finally:
        if temporary.exists():
            temporary.unlink()


def load_census_tool():
    specification = importlib.util.spec_from_file_location(
        "review_churn_census_for_audit_regression", CENSUS_TOOL_PATH
    )
    require(specification is not None and specification.loader is not None, "cannot load Census tool")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


def completed_pairs(case: dict[str, object]) -> list[dict[str, object]]:
    return [
        pair
        for pair in case["reviewer_pairs"]
        if pair["latest_completed_checkpoint"] is not None
    ]


def case_flags(case: dict[str, object]) -> dict[str, bool]:
    pairs = completed_pairs(case)
    return {
        "missing_oid": any(
            pair["latest_completed_checkpoint"]["differs_from_final_head"] is None
            for pair in pairs
        ),
        "commented_newer": any(
            pair["commented_newer_commit_candidate"] for pair in pairs
        ),
        "dismissed": bool(case["classification"]["completed_review_dismissal"]),
        "post_force": any(
            pair["latest_completed_checkpoint"]["post_completed_review_force_push"]
            for pair in pairs
        ),
        "rereview": any(
            pair["latest_completed_checkpoint"]["force_push_rereview"]
            for pair in pairs
        ),
        "drift_without_force": any(
            pair["latest_completed_checkpoint"]["differs_from_final_head"] is True
            and not pair["latest_completed_checkpoint"]["post_latest_checkpoint_force_push"]
            for pair in pairs
        ),
        "bot_only": not case["classification"]["formal_peer_reviewed"]
        and case["counts"]["bot_review_sessions"] > 0,
        "zero_review": case["counts"]["observed_reviews"] == 0,
        "commented_only": not pairs and case["counts"]["commented_only_pairs"] > 0,
        "stable": bool(pairs)
        and all(
            pair["latest_completed_checkpoint"]["differs_from_final_head"] is False
            for pair in pairs
        ),
    }


def rank(seed: str, case: dict[str, object]) -> str:
    material = (seed + "\0" + case["id"]).encode("utf-8")
    return sha256_bytes(material)


def select_cases(
    protocol: dict[str, object], cases: list[dict[str, object]]
) -> list[tuple[str, dict[str, object]]]:
    selection = protocol["selection"]
    seed = selection["seed"]
    quotas = selection["bucket_quotas"]
    expected_order = [
        "missing_oid",
        "commented_newer",
        "dismissed",
        "rewrite_heavy",
        "drift_without_force",
        "bot_only",
        "zero_review",
        "commented_only",
        "stable",
    ]
    require(selection["bucket_order"] == expected_order, "bucket order changed")
    require(
        set(quotas) == set(expected_order),
        "bucket quotas do not match the frozen bucket order",
    )
    flags = {case["id"]: case_flags(case) for case in cases}
    selected: list[tuple[str, dict[str, object]]] = []
    used: set[str] = set()

    def take(bucket: str, predicate, count: int) -> None:
        eligible = sorted(
            [
                case
                for case in cases
                if case["id"] not in used and predicate(case)
            ],
            key=lambda case: rank(seed, case),
        )
        require(len(eligible) >= count, f"insufficient cases for {bucket}: {len(eligible)} < {count}")
        for case in eligible[:count]:
            selected.append((bucket, case))
            used.add(case["id"])

    take("missing_oid", lambda case: flags[case["id"]]["missing_oid"], quotas["missing_oid"])
    take(
        "commented_newer",
        lambda case: flags[case["id"]]["commented_newer"],
        quotas["commented_newer"],
    )
    take("dismissed", lambda case: flags[case["id"]]["dismissed"], quotas["dismissed"])

    rewrite_repositories = selection["rewrite_heavy_repositories"]
    require(
        len(rewrite_repositories) == quotas["rewrite_heavy"],
        "rewrite-heavy quota must equal its repository count",
    )
    for repository in rewrite_repositories:
        eligible = sorted(
            [
                case
                for case in cases
                if case["id"] not in used
                and case["repository"] == repository
                and flags[case["id"]]["post_force"]
            ],
            key=lambda case: (
                not flags[case["id"]]["rereview"],
                rank(seed, case),
            ),
        )
        require(eligible, f"no rewrite-heavy case for {repository}")
        selected.append(("rewrite_heavy", eligible[0]))
        used.add(eligible[0]["id"])

    take(
        "drift_without_force",
        lambda case: flags[case["id"]]["drift_without_force"],
        quotas["drift_without_force"],
    )
    take("bot_only", lambda case: flags[case["id"]]["bot_only"], quotas["bot_only"])
    take("zero_review", lambda case: flags[case["id"]]["zero_review"], quotas["zero_review"])
    take(
        "commented_only",
        lambda case: flags[case["id"]]["commented_only"],
        quotas["commented_only"],
    )

    stable_quota = quotas["stable"]
    required_repositories = set(selection["required_repository_coverage"])
    represented = {case["repository"] for _, case in selected}
    missing_repositories = sorted(required_repositories - represented, key=str.casefold)
    require(
        len(missing_repositories) <= stable_quota,
        "stable quota cannot satisfy required repository coverage",
    )
    stable_selected = 0
    for repository in missing_repositories:
        eligible = sorted(
            [
                case
                for case in cases
                if case["id"] not in used
                and case["repository"] == repository
                and flags[case["id"]]["stable"]
            ],
            key=lambda case: rank(seed, case),
        )
        require(eligible, f"no stable coverage case for {repository}")
        selected.append(("stable", eligible[0]))
        used.add(eligible[0]["id"])
        stable_selected += 1
    take(
        "stable",
        lambda case: flags[case["id"]]["stable"],
        stable_quota - stable_selected,
    )

    require(len(selected) == sum(quotas.values()), "selected case count differs from quota sum")
    require(len(used) == len(selected), "selected cases are not unique")
    require(
        {case["repository"] for _, case in selected} == required_repositories,
        "selected cases do not cover the required repositories exactly",
    )
    require(
        sum(
            flags[case["id"]]["rereview"]
            for bucket, case in selected
            if bucket == "rewrite_heavy"
        )
        >= 2,
        "rewrite-heavy selection lost same-reviewer re-review coverage",
    )
    return selected


def reviewer_pair_oracle(pair: dict[str, object]) -> dict[str, object]:
    checkpoint = pair["latest_completed_checkpoint"]
    checkpoint_oracle = None
    if checkpoint is not None:
        checkpoint_oracle = {
            "commit_oid": checkpoint["commit_oid"],
            "completed_state": checkpoint["completed_state"],
            "current_state": checkpoint["current_state"],
            "dismissed": checkpoint["dismissed"],
            "differs_from_final_head": checkpoint["differs_from_final_head"],
            "post_completed_review_force_push": checkpoint[
                "post_completed_review_force_push"
            ],
            "post_latest_checkpoint_force_push": checkpoint[
                "post_latest_checkpoint_force_push"
            ],
            "force_push_rereview": checkpoint["force_push_rereview"],
        }
    return {
        "reviewer_key": pair["reviewer_key"],
        "completed_review_sessions": pair["completed_review_sessions"],
        "latest_completed_checkpoint": checkpoint_oracle,
        "commented_only": pair["commented_only"],
        "commented_newer_commit_candidate": pair[
            "commented_newer_commit_candidate"
        ],
    }


def case_oracle(bucket: str, case: dict[str, object]) -> dict[str, object]:
    count_keys = [
        "observed_reviews",
        "peer_human_review_sessions",
        "bot_review_sessions",
        "self_review_sessions",
        "other_actor_review_sessions",
        "unknown_review_sessions",
        "completed_review_pairs",
        "observable_checkpoint_oid_pairs",
        "comparable_checkpoint_pairs",
        "drifted_checkpoint_pairs",
        "post_force_push_checkpoint_pairs",
        "force_push_rereview_pairs",
        "drift_without_observed_force_push_pairs",
        "commented_only_pairs",
        "commented_newer_commit_candidate_pairs",
    ]
    return {
        "case_id": case["id"],
        "repository": case["repository"],
        "pull_request_number": case["number"],
        "bucket": bucket,
        "formal_peer_reviewed": case["classification"]["formal_peer_reviewed"],
        "completed_reviewed": case["classification"]["completed_reviewed"],
        "stranded_reviewer": case["classification"]["stranded_reviewer"],
        "multi_round_completed_review": case["classification"][
            "multi_round_completed_review"
        ],
        "completed_review_dismissal": case["classification"][
            "completed_review_dismissal"
        ],
        "counts": {key: case["counts"][key] for key in count_keys},
        "reviewer_pairs": [
            reviewer_pair_oracle(pair) for pair in case["reviewer_pairs"]
        ],
    }


def expected_golden(
    protocol: dict[str, object], cases: list[dict[str, object]]
) -> dict[str, object]:
    selected = select_cases(protocol, cases)
    return {
        "schema": GOLDEN_SCHEMA,
        "dataset_version": DATASET_VERSION,
        "frozen_at": protocol["frozen_at"],
        "source_artifacts": dict(protocol["source_artifacts"]),
        "selection": {
            "algorithm": protocol["selection"]["algorithm"],
            "seed": protocol["selection"]["seed"],
            "case_count": len(selected),
            "repository_count": len({case["repository"] for _, case in selected}),
        },
        "cases": [case_oracle(bucket, case) for bucket, case in selected],
    }


def privacy_scan(value: object, label: str = "golden") -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            require(key.casefold() not in FORBIDDEN_KEYS, f"forbidden key {key!r} at {label}")
            if key == "reviewer_key":
                require(
                    isinstance(child, str) and OPAQUE_ACTOR_PATTERN.fullmatch(child) is not None,
                    f"invalid opaque reviewer key at {label}",
                )
            privacy_scan(child, f"{label}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            privacy_scan(child, f"{label}[{index}]")


def load_sources(
    protocol: dict[str, object]
) -> tuple[dict[str, object], dict[str, object]]:
    require(protocol["schema"] == PROTOCOL_SCHEMA, "unsupported regression protocol schema")
    require(protocol["dataset_version"] == DATASET_VERSION, "unsupported protocol version")
    source = protocol["source_artifacts"]
    capture_path = ROOT / source["capture_path"]
    manifest_path = ROOT / source["manifest_path"]
    capture_bytes, capture = read_canonical_json(capture_path)
    manifest_bytes, manifest = read_canonical_json(manifest_path)
    require(
        sha256_bytes(capture_bytes) == source["capture_sha256"],
        "source capture SHA-256 differs",
    )
    require(
        sha256_bytes(manifest_bytes) == source["manifest_sha256"],
        "source manifest SHA-256 differs",
    )
    require(
        manifest["capture_sha256"] == source["capture_sha256"],
        "manifest does not bind the frozen capture",
    )
    require(capture["capture_complete"] is True, "source capture is incomplete")
    require(capture["summary"]["capture_failures"] == 0, "source capture has failures")
    return capture, manifest


def verify_full_shadow(
    capture: dict[str, object], manifest: dict[str, object]
) -> list[dict[str, object]]:
    captured_cases = capture["cases"]
    manifest_cases = manifest["pull_requests"]
    require(len(captured_cases) == 500, "source capture no longer contains 500 cases")
    require(len(manifest_cases) == 500, "source manifest no longer contains 500 cases")
    expected_by_id = {case["id"]: case for case in manifest_cases}
    require(len(expected_by_id) == 500, "source manifest has duplicate case IDs")
    census = load_census_tool()
    for raw_case in captured_cases:
        observed = census.classify_case(raw_case)
        require(raw_case["id"] in expected_by_id, f"capture case is missing from manifest: {raw_case['id']}")
        require(
            observed == expected_by_id[raw_case["id"]],
            f"500-case shadow replay differs for {raw_case['id']}",
        )
    require(
        {case["id"] for case in captured_cases} == set(expected_by_id),
        "capture and manifest case sets differ",
    )
    return manifest_cases


def build_expected() -> dict[str, object]:
    _, protocol = read_canonical_json(PROTOCOL_PATH)
    capture, manifest = load_sources(protocol)
    cases = verify_full_shadow(capture, manifest)
    golden = expected_golden(protocol, cases)
    privacy_scan(golden)
    return golden


def command_freeze() -> None:
    golden = build_expected()
    atomic_write(GOLDEN_PATH, canonical_json(golden))
    print(f"froze {len(golden['cases'])} Review Memory Audit cases to {GOLDEN_PATH}")


def command_verify() -> None:
    expected = build_expected()
    payload, observed = read_canonical_json(GOLDEN_PATH)
    require(payload == canonical_json(observed), "golden JSON is not canonical")
    privacy_scan(observed)
    require(observed == expected, "golden regression artifact differs from deterministic rebuild")
    print(
        f"verified Review Memory Audit regression v1: {len(observed['cases'])} cases, "
        f"{observed['selection']['repository_count']} repositories"
    )


def command_self_test() -> None:
    expected = build_expected()
    _, observed = read_canonical_json(GOLDEN_PATH)
    require(observed == expected, "self-test baseline differs")
    tampered = copy.deepcopy(observed)
    tampered["cases"][0]["counts"]["observed_reviews"] += 1
    require(tampered != expected, "tamper self-test did not change the artifact")
    print("Review Memory Audit regression self-test passed")


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("freeze", "verify", "self-test"))
    return parser.parse_args()


def main() -> None:
    arguments = parse_arguments()
    if arguments.command == "freeze":
        command_freeze()
    elif arguments.command == "verify":
        command_verify()
    else:
        command_self_test()


if __name__ == "__main__":
    main()
