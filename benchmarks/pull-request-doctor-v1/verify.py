#!/usr/bin/env python3
"""Verify the controlled Pull Request Doctor v1 semantic corpus."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
from typing import Callable


BUNDLE = Path(__file__).resolve().parent
DEFAULT_CASES = BUNDLE / "cases.json"
DEFAULT_MANIFEST = BUNDLE / "manifest.json"
DEFAULT_ORACLE = BUNDLE / "oracle.json"
CHECKSUMS = BUNDLE / "SHA256SUMS"

CASES_SCHEMA = "stratadiff-pull-request-doctor-cases-v1"
MANIFEST_SCHEMA = "stratadiff-pull-request-doctor-manifest-v1"
MATERIALIZED_SCHEMA = "stratadiff-pull-request-doctor-materialized-v1"
ORACLE_SCHEMA = "stratadiff-pull-request-doctor-oracle-v1"
SNAPSHOT_SCHEMA = "stratadiff-pull-request-doctor-snapshot-v1"
DATASET_VERSION = "1.0.0"

CASE_ID_PATTERN = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
COVERAGE_PATTERN = re.compile(r"^[a-z][a-z0-9_]*$")
OID_PATTERN = re.compile(r"^[0-9a-f]{40}$")
REPOSITORY_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
TIMESTAMP_PATTERN = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")

COLLECTION_STATUSES = {"complete", "partial"}
COLLECTION_SURFACES = {"target", "requirements", "check_runs", "commit_statuses"}
CHECK_STATUSES = {"queued", "in_progress", "completed"}
CHECK_CONCLUSIONS = {
    "action_required",
    "cancelled",
    "failure",
    "neutral",
    "skipped",
    "stale",
    "startup_failure",
    "success",
    "timed_out",
}
SUCCESSFUL_CHECK_CONCLUSIONS = {"neutral", "skipped", "success"}
LEGACY_STATES = {"error", "failure", "pending", "success"}
POLICY_KINDS = {"branch_protection", "ruleset"}
POLICY_KIND_ORDER = {"ruleset": 0, "branch_protection": 1}
REQUIREMENT_STATUSES = {
    "failed",
    "missing",
    "pending",
    "satisfied",
    "source_mismatch",
    "source_unknown",
}
VERDICTS = {"checks_blocked", "checks_clear", "inconclusive"}
FORBIDDEN_SNAPSHOT_KEYS = {
    "authorization",
    "body",
    "commit_message",
    "diff",
    "files",
    "output",
    "patch",
    "text",
    "token",
}

EXPECTED_SCENARIOS = {
    "clean-pinned": "clean_pinned",
    "clean-unpinned": "clean_unpinned",
    "duplicate-policy-dedupe": "duplicate_policy_deduplication",
    "failed-check": "failed",
    "head-drift-incomplete": "exact_head_drift_fail_closed",
    "legacy-success": "legacy_status_success",
    "missing-complete": "missing_complete",
    "missing-partial-inconclusive": "missing_partial_fail_closed",
    "partial-known-failure": "partial_known_blocker",
    "pending-check": "pending",
    "pinned-check-legacy-failure": "pinned_legacy_failure",
    "right-and-wrong-app": "matching_source_precedence",
    "same-name-check-status-split-brain": "check_status_split_brain",
    "source-ambiguous-unpinned": "source_ambiguity",
    "wrong-app": "source_mismatch",
}

CLAIM_BOUNDARY = {
    "controlled_required_check_semantics_supported": True,
    "end_to_end_cli_conformance_supported": False,
    "failure_prevalence_supported": False,
    "live_collector_conformance_supported": False,
    "market_demand_supported": False,
    "merge_safety_supported": False,
    "production_accuracy_supported": False,
}

EXPECTED_CHECKSUM_FILES = {
    "README.md",
    "cases.json",
    "manifest.json",
    "oracle.json",
    "verify.py",
}


class BenchmarkError(RuntimeError):
    """A benchmark asset violates the frozen bundle contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise BenchmarkError(message)


def require_exact_keys(value: dict[str, object], keys: set[str], label: str) -> None:
    require(set(value) == keys, f"{label} fields differ: {sorted(set(value) ^ keys)}")


def require_object(value: object, label: str) -> dict[str, object]:
    require(type(value) is dict, f"{label} must be an object")
    assert isinstance(value, dict)
    return value


def require_array(value: object, label: str) -> list[object]:
    require(type(value) is list, f"{label} must be an array")
    assert isinstance(value, list)
    return value


def require_string(value: object, label: str) -> str:
    require(type(value) is str and bool(value), f"{label} must be a non-empty string")
    assert isinstance(value, str)
    return value


def require_int(value: object, label: str, minimum: int = 0) -> int:
    require(type(value) is int and value >= minimum, f"{label} must be an integer >= {minimum}")
    assert isinstance(value, int)
    return value


def require_nullable_int(value: object, label: str) -> int | None:
    if value is None:
        return None
    return require_int(value, label, 1)


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


def read_canonical_json(path: Path) -> tuple[bytes, dict[str, object]]:
    payload = path.read_bytes()
    value = json.loads(payload, object_pairs_hook=unique_json_object)
    require(type(value) is dict, f"{path} must contain one JSON object")
    assert isinstance(value, dict)
    require(payload == canonical_json(value), f"{path} is not canonical JSON")
    return payload, value


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def require_url(value: object, label: str, prefix: str) -> str:
    url = require_string(value, label)
    require(url.startswith(prefix), f"{label} must remain inside {prefix}")
    return url


def reject_forbidden_snapshot_keys(value: object, path: str) -> None:
    if isinstance(value, dict):
        for key, item in value.items():
            require(
                key.casefold() not in FORBIDDEN_SNAPSHOT_KEYS,
                f"forbidden payload field at {path}.{key}",
            )
            reject_forbidden_snapshot_keys(item, f"{path}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            reject_forbidden_snapshot_keys(item, f"{path}[{index}]")


def validate_snapshot_defaults(value: object) -> dict[str, object]:
    defaults = require_object(value, "snapshot_defaults")
    require_exact_keys(
        defaults,
        {"captured_at", "provider_url", "repository", "schema", "target"},
        "snapshot_defaults",
    )
    require(defaults["schema"] == SNAPSHOT_SCHEMA, "snapshot schema changed")
    captured_at = require_string(defaults["captured_at"], "snapshot_defaults.captured_at")
    require(bool(TIMESTAMP_PATTERN.fullmatch(captured_at)), "captured_at is not UTC seconds")
    require(defaults["provider_url"] == "https://github.com", "provider must be github.com")
    repository = require_string(defaults["repository"], "snapshot_defaults.repository")
    require(bool(REPOSITORY_PATTERN.fullmatch(repository)), "repository must be owner/name")

    target = require_object(defaults["target"], "snapshot_defaults.target")
    require_exact_keys(target, {"base_ref", "base_sha", "head_sha", "number", "url"}, "target")
    require_string(target["base_ref"], "target.base_ref")
    base_sha = require_string(target["base_sha"], "target.base_sha")
    head_sha = require_string(target["head_sha"], "target.head_sha")
    require(bool(OID_PATTERN.fullmatch(base_sha)), "target.base_sha is invalid")
    require(bool(OID_PATTERN.fullmatch(head_sha)), "target.head_sha is invalid")
    require(base_sha != head_sha, "base and head identity must differ")
    number = require_int(target["number"], "target.number", 1)
    require_url(
        target["url"],
        "target.url",
        f"https://github.com/{repository}/pull/{number}",
    )
    return defaults


def validate_collection(
    value: object, label: str, include_metrics: bool
) -> dict[str, object]:
    collection = require_object(value, label)
    keys = {"api_calls", "gaps", "response_bytes", "status"} if include_metrics else {"gaps", "status"}
    require_exact_keys(collection, keys, label)
    if include_metrics:
        require_int(collection["api_calls"], f"{label}.api_calls")
        require_int(collection["response_bytes"], f"{label}.response_bytes")
    status = require_string(collection["status"], f"{label}.status")
    require(status in COLLECTION_STATUSES, f"{label}.status is invalid")
    gaps = require_array(collection["gaps"], f"{label}.gaps")
    seen: set[tuple[str, str]] = set()
    for index, item in enumerate(gaps):
        gap = require_object(item, f"{label}.gaps[{index}]")
        require_exact_keys(gap, {"reason", "surface"}, f"{label}.gaps[{index}]")
        surface = require_string(gap["surface"], f"{label}.gaps[{index}].surface")
        reason = require_string(gap["reason"], f"{label}.gaps[{index}].reason")
        require(surface in COLLECTION_SURFACES, f"{label}.gaps[{index}] surface is invalid")
        require((surface, reason) not in seen, f"{label} repeats an identical gap")
        seen.add((surface, reason))
    require((status == "complete") == (len(gaps) == 0), f"{label} completeness disagrees with gaps")
    return collection


def validate_requirements(value: object, label: str, repository_url: str) -> list[dict[str, object]]:
    requirements = require_array(value, label)
    require(bool(requirements), f"{label} must not be empty")
    validated: list[dict[str, object]] = []
    for index, item in enumerate(requirements):
        requirement = require_object(item, f"{label}[{index}]")
        require_exact_keys(
            requirement,
            {"context", "expected_app_id", "policies"},
            f"{label}[{index}]",
        )
        require_string(requirement["context"], f"{label}[{index}].context")
        require_nullable_int(requirement["expected_app_id"], f"{label}[{index}].expected_app_id")
        policies = require_array(requirement["policies"], f"{label}[{index}].policies")
        require(bool(policies), f"{label}[{index}].policies must not be empty")
        policy_ids: set[tuple[str, str]] = set()
        for policy_index, policy_item in enumerate(policies):
            policy = require_object(policy_item, f"{label}[{index}].policies[{policy_index}]")
            require_exact_keys(
                policy,
                {"id", "kind", "name", "url"},
                f"{label}[{index}].policies[{policy_index}]",
            )
            kind = require_string(policy["kind"], f"{label}[{index}].policies[{policy_index}].kind")
            policy_id = require_string(policy["id"], f"{label}[{index}].policies[{policy_index}].id")
            require(kind in POLICY_KINDS, f"{label}[{index}] policy kind is invalid")
            require_string(policy["name"], f"{label}[{index}].policies[{policy_index}].name")
            require_url(
                policy["url"],
                f"{label}[{index}].policies[{policy_index}].url",
                repository_url,
            )
            require((kind, policy_id) not in policy_ids, f"{label}[{index}] repeats a policy")
            policy_ids.add((kind, policy_id))
        validated.append(requirement)
    return validated


def validate_check_runs(value: object, label: str, repository_url: str) -> list[dict[str, object]]:
    check_runs = require_array(value, label)
    validated: list[dict[str, object]] = []
    ids: set[int] = set()
    for index, item in enumerate(check_runs):
        check = require_object(item, f"{label}[{index}]")
        require_exact_keys(
            check,
            {"app_id", "app_slug", "conclusion", "id", "name", "status", "url"},
            f"{label}[{index}]",
        )
        check_id = require_int(check["id"], f"{label}[{index}].id", 1)
        require(check_id not in ids, f"{label} repeats check-run ID {check_id}")
        ids.add(check_id)
        require_string(check["name"], f"{label}[{index}].name")
        require_nullable_int(check["app_id"], f"{label}[{index}].app_id")
        if check["app_slug"] is not None:
            require_string(check["app_slug"], f"{label}[{index}].app_slug")
        status = require_string(check["status"], f"{label}[{index}].status")
        require(status in CHECK_STATUSES, f"{label}[{index}].status is invalid")
        if status == "completed":
            conclusion = require_string(check["conclusion"], f"{label}[{index}].conclusion")
            require(conclusion in CHECK_CONCLUSIONS, f"{label}[{index}].conclusion is invalid")
        else:
            require(check["conclusion"] is None, f"{label}[{index}] unfinished check has conclusion")
        require_url(check["url"], f"{label}[{index}].url", repository_url)
        validated.append(check)
    return validated


def validate_statuses(value: object, label: str, repository_url: str) -> list[dict[str, object]]:
    statuses = require_array(value, label)
    validated: list[dict[str, object]] = []
    ids: set[int] = set()
    for index, item in enumerate(statuses):
        status = require_object(item, f"{label}[{index}]")
        require_exact_keys(
            status,
            {"context", "creator_id", "creator_login", "id", "state", "url"},
            f"{label}[{index}]",
        )
        status_id = require_int(status["id"], f"{label}[{index}].id", 1)
        require(status_id not in ids, f"{label} repeats commit-status ID {status_id}")
        ids.add(status_id)
        require_string(status["context"], f"{label}[{index}].context")
        require_nullable_int(status["creator_id"], f"{label}[{index}].creator_id")
        if status["creator_login"] is not None:
            require_string(status["creator_login"], f"{label}[{index}].creator_login")
        state = require_string(status["state"], f"{label}[{index}].state")
        require(state in LEGACY_STATES, f"{label}[{index}].state is invalid")
        require_url(status["url"], f"{label}[{index}].url", repository_url)
        validated.append(status)
    return validated


def validate_snapshot(value: dict[str, object], label: str) -> None:
    require_exact_keys(
        value,
        {
            "captured_at",
            "check_runs",
            "collection",
            "provider_url",
            "repository",
            "requirements",
            "schema",
            "statuses",
            "target",
        },
        label,
    )
    reject_forbidden_snapshot_keys(value, label)
    require(value["schema"] == SNAPSHOT_SCHEMA, f"{label} schema changed")
    repository = require_string(value["repository"], f"{label}.repository")
    repository_url = f"https://github.com/{repository}"
    validate_collection(value["collection"], f"{label}.collection", True)
    validate_requirements(value["requirements"], f"{label}.requirements", repository_url)
    validate_check_runs(value["check_runs"], f"{label}.check_runs", repository_url)
    validate_statuses(value["statuses"], f"{label}.statuses", repository_url)


def validate_cases_asset(value: dict[str, object]) -> list[dict[str, object]]:
    require_exact_keys(
        value,
        {"cases", "dataset_version", "schema", "snapshot_defaults"},
        "cases asset",
    )
    require(value["schema"] == CASES_SCHEMA, "unsupported cases schema")
    require(value["dataset_version"] == DATASET_VERSION, "cases version changed")
    defaults = validate_snapshot_defaults(value["snapshot_defaults"])
    cases = require_array(value["cases"], "cases")
    require(len(cases) >= 15, "doctor corpus must contain at least 15 cases")

    materialized: list[dict[str, object]] = []
    case_ids: set[str] = set()
    for index, item in enumerate(cases):
        case = require_object(item, f"cases[{index}]")
        require_exact_keys(case, {"covers", "description", "id", "snapshot"}, f"cases[{index}]")
        case_id = require_string(case["id"], f"cases[{index}].id")
        require(bool(CASE_ID_PATTERN.fullmatch(case_id)), f"invalid case ID {case_id!r}")
        require(case_id not in case_ids, f"duplicate case ID {case_id}")
        case_ids.add(case_id)
        require_string(case["description"], f"cases[{index}].description")

        covers = require_array(case["covers"], f"cases[{index}].covers")
        require(bool(covers), f"cases[{index}].covers must not be empty")
        cover_values = [require_string(tag, f"cases[{index}].covers") for tag in covers]
        require(len(cover_values) == len(set(cover_values)), f"cases[{index}] repeats coverage")
        require(
            all(COVERAGE_PATTERN.fullmatch(tag) for tag in cover_values),
            f"cases[{index}] has invalid coverage",
        )

        fragment = require_object(case["snapshot"], f"cases[{index}].snapshot")
        require_exact_keys(
            fragment,
            {"check_runs", "collection", "requirements", "statuses"},
            f"cases[{index}].snapshot",
        )
        snapshot = copy.deepcopy(defaults)
        for key in ("check_runs", "collection", "requirements", "statuses"):
            snapshot[key] = copy.deepcopy(fragment[key])
        validate_snapshot(snapshot, f"cases[{index}].materialized_snapshot")
        materialized.append(
            {
                "covers": copy.deepcopy(covers),
                "description": case["description"],
                "id": case_id,
                "snapshot": snapshot,
            }
        )

    require(case_ids == set(EXPECTED_SCENARIOS), "required scenario IDs differ")
    for case in materialized:
        case_id = case["id"]
        require(
            case["covers"] == [EXPECTED_SCENARIOS[case_id]],
            f"{case_id} coverage label changed",
        )
    return materialized


def policy_token(policy: dict[str, object]) -> str:
    return f"{policy['kind']}:{policy['id']}"


def policy_token_sort_key(token: str) -> tuple[int, str]:
    kind, policy_id = token.split(":", 1)
    require(kind in POLICY_KIND_ORDER, f"unknown policy token kind {kind!r}")
    return POLICY_KIND_ORDER[kind], policy_id


def check_run_state(check: dict[str, object]) -> str:
    if check["status"] != "completed":
        return "pending"
    if check["conclusion"] in SUCCESSFUL_CHECK_CONCLUSIONS:
        return "satisfied"
    return "failed"


def legacy_state(status: dict[str, object]) -> str:
    if status["state"] == "success":
        return "satisfied"
    if status["state"] == "pending":
        return "pending"
    return "failed"


def reduce_signals(states: list[str]) -> str:
    require(bool(states), "cannot reduce empty signals")
    if "failed" in states:
        return "failed"
    if "pending" in states:
        return "pending"
    return "satisfied"


def derive_case(case: dict[str, object]) -> dict[str, object]:
    snapshot = require_object(case["snapshot"], f"{case['id']}.snapshot")
    collection = require_object(snapshot["collection"], f"{case['id']}.collection")
    check_runs = require_array(snapshot["check_runs"], f"{case['id']}.check_runs")
    statuses = require_array(snapshot["statuses"], f"{case['id']}.statuses")
    requirements = require_array(snapshot["requirements"], f"{case['id']}.requirements")
    signal_collection_incomplete = any(
        gap["surface"] in {"check_runs", "commit_statuses"}
        for gap in collection["gaps"]
    )

    grouped: dict[tuple[str, int | None], dict[str, dict[str, object]]] = {}
    for item in requirements:
        requirement = require_object(item, f"{case['id']}.requirement")
        key = (requirement["context"], requirement["expected_app_id"])
        require(type(key[0]) is str, f"{case['id']} context type changed")
        require(key not in grouped or isinstance(grouped[key], dict), "invalid requirement group")
        if key not in grouped:
            grouped[key] = {}
        for item_policy in require_array(requirement["policies"], f"{case['id']}.policies"):
            policy = require_object(item_policy, f"{case['id']}.policy")
            token = policy_token(policy)
            if token in grouped[key]:
                require(grouped[key][token] == policy, f"{case['id']} policy identity is ambiguous")
            grouped[key][token] = policy

    derived_requirements: list[dict[str, object]] = []
    for context, expected_app_id in sorted(
        grouped,
        key=lambda key: (key[0], -1 if key[1] is None else key[1]),
    ):
        named_checks = [check for check in check_runs if check["name"] == context]
        named_statuses = [status for status in statuses if status["context"] == context]

        if expected_app_id is not None:
            expected_checks = [
                check for check in named_checks if check["app_id"] == expected_app_id
            ]
            if expected_checks:
                states = [check_run_state(check) for check in expected_checks]
                states.extend(legacy_state(status) for status in named_statuses)
                requirement_status = reduce_signals(states)
                if requirement_status == "satisfied" and signal_collection_incomplete:
                    requirement_status = "source_unknown"
            elif signal_collection_incomplete:
                requirement_status = "source_unknown"
            elif any(check["app_id"] is None for check in named_checks) or named_statuses:
                requirement_status = "source_unknown"
            elif named_checks:
                requirement_status = "source_mismatch"
            else:
                requirement_status = "missing"
        else:
            if not named_checks and not named_statuses:
                requirement_status = (
                    "source_unknown" if signal_collection_incomplete else "missing"
                )
            else:
                states = [check_run_state(check) for check in named_checks]
                states.extend(legacy_state(status) for status in named_statuses)
                requirement_status = reduce_signals(states)
                distinct_app_count = len(
                    {
                        check["app_id"]
                        for check in named_checks
                        if check["app_id"] is not None
                    }
                )
                if requirement_status == "satisfied" and (
                    distinct_app_count > 1 or signal_collection_incomplete
                ):
                    requirement_status = "source_unknown"

        evidence = [f"check_run:{check['id']}" for check in named_checks]
        evidence.extend(f"commit_status:{status['id']}" for status in named_statuses)
        derived_requirements.append(
            {
                "context": context,
                "evidence": sorted(evidence),
                "expected_app_id": expected_app_id,
                "policies": sorted(
                    grouped[(context, expected_app_id)], key=policy_token_sort_key
                ),
                "status": requirement_status,
            }
        )

    blockers = {"failed", "missing", "pending", "source_mismatch"}
    if any(item["status"] in blockers for item in derived_requirements):
        verdict = "checks_blocked"
    elif collection["status"] == "partial" or any(
        item["status"] == "source_unknown" for item in derived_requirements
    ):
        verdict = "inconclusive"
    else:
        verdict = "checks_clear"
    return {
        "collection_status": collection["status"],
        "gaps": copy.deepcopy(collection["gaps"]),
        "requirements": derived_requirements,
        "verdict": verdict,
    }


def derive_oracle(materialized: list[dict[str, object]]) -> dict[str, object]:
    cases = {case["id"]: derive_case(case) for case in materialized}
    return {
        "cases": dict(sorted(cases.items())),
        "dataset_version": DATASET_VERSION,
        "schema": ORACLE_SCHEMA,
    }


def validate_named_scenarios(
    materialized: list[dict[str, object]], derived: dict[str, object]
) -> None:
    inputs = {case["id"]: case for case in materialized}
    outputs = require_object(derived["cases"], "derived.cases")

    partial_missing = require_object(
        outputs["missing-partial-inconclusive"], "missing-partial-inconclusive"
    )
    partial_requirement = require_object(
        require_array(partial_missing["requirements"], "partial requirements")[0],
        "partial requirement",
    )
    require(partial_requirement["status"] == "source_unknown", "partial absence became missing")
    require(partial_missing["verdict"] == "inconclusive", "partial absence became conclusive")
    require(
        all(
            output["verdict"] != "checks_clear"
            for output in outputs.values()
            if output["collection_status"] == "partial"
        ),
        "a partial snapshot became checks_clear",
    )
    partial_blocker = require_object(outputs["partial-known-failure"], "partial-known-failure")
    partial_blocker_requirement = require_object(
        require_array(partial_blocker["requirements"], "partial blocker requirements")[0],
        "partial blocker requirement",
    )
    require(partial_blocker_requirement["status"] == "failed", "known partial blocker was erased")
    require(partial_blocker["verdict"] == "checks_blocked", "known partial blocker became unknown")

    right_wrong_input = require_object(
        inputs["right-and-wrong-app"]["snapshot"], "right-and-wrong-app snapshot"
    )
    right_wrong_checks = require_array(right_wrong_input["check_runs"], "right-and-wrong-app checks")
    require(len({check["app_id"] for check in right_wrong_checks}) == 2, "source precedence control lost")
    right_wrong_output = require_object(outputs["right-and-wrong-app"], "right-and-wrong-app")
    right_wrong_requirement = require_object(
        require_array(right_wrong_output["requirements"], "right-and-wrong-app requirements")[0],
        "right-and-wrong-app requirement",
    )
    require(right_wrong_requirement["status"] == "satisfied", "matching App lost precedence")
    require(len(right_wrong_requirement["evidence"]) == 2, "same-name App evidence was hidden")

    ambiguous_input = require_object(
        inputs["source-ambiguous-unpinned"]["snapshot"], "source-ambiguous input"
    )
    ambiguous_checks = require_array(ambiguous_input["check_runs"], "source-ambiguous checks")
    require(
        len({check["app_id"] for check in ambiguous_checks}) == 2,
        "source-ambiguity control lost one App",
    )
    ambiguous_output = require_object(
        outputs["source-ambiguous-unpinned"], "source-ambiguous output"
    )
    ambiguous_requirement = require_object(
        require_array(ambiguous_output["requirements"], "source-ambiguous requirements")[0],
        "source-ambiguous requirement",
    )
    require(
        ambiguous_requirement["status"] == "source_unknown",
        "two unpinned Apps were treated as one trusted producer",
    )
    require(ambiguous_output["verdict"] == "inconclusive", "source ambiguity became clear")

    split_output = require_object(
        outputs["same-name-check-status-split-brain"], "split-brain output"
    )
    split_requirement = require_object(
        require_array(split_output["requirements"], "split-brain requirements")[0],
        "split-brain requirement",
    )
    require(split_requirement["status"] == "failed", "split brain did not fail closed")
    require(
        {token.split(":", 1)[0] for token in split_requirement["evidence"]}
        == {"check_run", "commit_status"},
        "split-brain evidence lost one provider surface",
    )

    pinned_legacy_output = require_object(
        outputs["pinned-check-legacy-failure"], "pinned-legacy output"
    )
    pinned_legacy_requirement = require_object(
        require_array(pinned_legacy_output["requirements"], "pinned-legacy requirements")[0],
        "pinned-legacy requirement",
    )
    require(
        pinned_legacy_requirement["status"] == "failed",
        "legacy failure was hidden by a pinned-App success",
    )
    require(
        {token.split(":", 1)[0] for token in pinned_legacy_requirement["evidence"]}
        == {"check_run", "commit_status"},
        "pinned-legacy evidence lost one provider surface",
    )

    duplicate_input = require_object(
        inputs["duplicate-policy-dedupe"]["snapshot"], "duplicate-policy input"
    )
    duplicate_output = require_object(outputs["duplicate-policy-dedupe"], "duplicate-policy output")
    require(len(duplicate_input["requirements"]) == 2, "duplicate-policy control lost")
    duplicate_requirements = require_array(duplicate_output["requirements"], "deduped requirements")
    require(len(duplicate_requirements) == 1, "duplicate requirement was not deduplicated")
    require(len(duplicate_requirements[0]["policies"]) == 2, "policy provenance was discarded")

    head_output = require_object(outputs["head-drift-incomplete"], "head-drift output")
    head_requirement = require_object(
        require_array(head_output["requirements"], "head-drift requirements")[0],
        "head-drift requirement",
    )
    require(head_requirement["status"] == "satisfied", "head-drift control lacks positive signal")
    require(head_output["verdict"] == "inconclusive", "head drift produced false clear")
    require(
        any(gap["surface"] == "target" for gap in head_output["gaps"]),
        "head-drift target gap disappeared",
    )


def validate_oracle(value: dict[str, object], case_ids: set[str]) -> None:
    require_exact_keys(value, {"cases", "dataset_version", "schema"}, "oracle")
    require(value["schema"] == ORACLE_SCHEMA, "unsupported oracle schema")
    require(value["dataset_version"] == DATASET_VERSION, "oracle version changed")
    cases = require_object(value["cases"], "oracle.cases")
    require(set(cases) == case_ids, "oracle case IDs differ from inputs")
    for case_id, item in cases.items():
        outcome = require_object(item, f"oracle.cases.{case_id}")
        require_exact_keys(
            outcome,
            {"collection_status", "gaps", "requirements", "verdict"},
            f"oracle.cases.{case_id}",
        )
        require(outcome["collection_status"] in COLLECTION_STATUSES, f"{case_id} status invalid")
        require(outcome["verdict"] in VERDICTS, f"{case_id} verdict invalid")
        validate_collection(
            {"gaps": outcome["gaps"], "status": outcome["collection_status"]},
            f"oracle.cases.{case_id}.collection",
            False,
        )
        requirements = require_array(outcome["requirements"], f"oracle.cases.{case_id}.requirements")
        require(bool(requirements), f"oracle.cases.{case_id}.requirements is empty")
        seen_keys: set[tuple[str, int | None]] = set()
        for index, requirement_item in enumerate(requirements):
            requirement = require_object(
                requirement_item, f"oracle.cases.{case_id}.requirements[{index}]"
            )
            require_exact_keys(
                requirement,
                {"context", "evidence", "expected_app_id", "policies", "status"},
                f"oracle.cases.{case_id}.requirements[{index}]",
            )
            context = require_string(requirement["context"], f"{case_id}.context")
            app_id = require_nullable_int(requirement["expected_app_id"], f"{case_id}.expected_app_id")
            require((context, app_id) not in seen_keys, f"{case_id} repeats a requirement result")
            seen_keys.add((context, app_id))
            require(requirement["status"] in REQUIREMENT_STATUSES, f"{case_id} result status invalid")
            evidence = require_array(requirement["evidence"], f"{case_id}.evidence")
            policies = require_array(requirement["policies"], f"{case_id}.policies")
            require(
                evidence == sorted(set(evidence)),
                f"{case_id} evidence must be sorted and unique",
            )
            require(
                policies == sorted(set(policies), key=policy_token_sort_key) and bool(policies),
                f"{case_id} policies must be sorted, unique, and non-empty",
            )


def summarize(
    materialized: list[dict[str, object]], derived: dict[str, object]
) -> dict[str, object]:
    requirement_statuses = {status: 0 for status in sorted(REQUIREMENT_STATUSES)}
    verdicts = {verdict: 0 for verdict in sorted(VERDICTS)}
    requirements = 0
    policies = 0
    evidence_refs = 0
    collection_gaps = 0
    cases = require_object(derived["cases"], "derived.cases")
    for outcome_item in cases.values():
        outcome = require_object(outcome_item, "derived outcome")
        verdicts[outcome["verdict"]] += 1
        collection_gaps += len(outcome["gaps"])
        for requirement_item in outcome["requirements"]:
            requirement = require_object(requirement_item, "derived requirement")
            requirements += 1
            requirement_statuses[requirement["status"]] += 1
            policies += len(requirement["policies"])
            evidence_refs += len(requirement["evidence"])
    coverage = {
        tag
        for case in materialized
        for tag in require_array(case["covers"], f"{case['id']}.covers")
    }
    return {
        "cases": len(materialized),
        "collection_gaps": collection_gaps,
        "coverage_tags": len(coverage),
        "evidence_refs": evidence_refs,
        "policies": policies,
        "requirement_statuses": requirement_statuses,
        "requirements": requirements,
        "verdicts": verdicts,
    }


def validate_manifest(
    value: dict[str, object],
    cases_bytes: bytes,
    oracle_bytes: bytes,
    materialized: list[dict[str, object]],
    summary: dict[str, object],
) -> None:
    require_exact_keys(
        value,
        {
            "acceptance_gates",
            "assets",
            "claim_boundary",
            "dataset_license",
            "dataset_version",
            "description",
            "designation",
            "expected_summary",
            "name",
            "required_coverage",
            "schema",
        },
        "manifest",
    )
    require(value["schema"] == MANIFEST_SCHEMA, "unsupported manifest schema")
    require(value["dataset_version"] == DATASET_VERSION, "manifest version changed")
    require(value["dataset_license"] == "MIT", "dataset license changed")
    require(value["designation"] == "controlled_semantic_regression", "designation changed")
    require_string(value["name"], "manifest.name")
    require_string(value["description"], "manifest.description")

    assets = require_object(value["assets"], "manifest.assets")
    require_exact_keys(assets, {"cases", "oracle"}, "manifest.assets")
    for name, payload in (("cases", cases_bytes), ("oracle", oracle_bytes)):
        asset = require_object(assets[name], f"manifest.assets.{name}")
        require_exact_keys(asset, {"path", "sha256"}, f"manifest.assets.{name}")
        require(asset["path"] == f"{name}.json", f"manifest {name} path changed")
        digest = require_string(asset["sha256"], f"manifest.assets.{name}.sha256")
        require(bool(SHA256_PATTERN.fullmatch(digest)), f"manifest {name} hash is invalid")
        require(digest == sha256_bytes(payload), f"manifest {name} hash mismatch")

    boundary = require_object(value["claim_boundary"], "manifest.claim_boundary")
    require(boundary == CLAIM_BOUNDARY, "claim boundary was weakened or changed")
    require(value["expected_summary"] == summary, "manifest expected summary changed")

    required_coverage = require_array(value["required_coverage"], "manifest.required_coverage")
    require(
        required_coverage == sorted(EXPECTED_SCENARIOS.values()),
        "manifest required coverage changed",
    )
    observed_coverage = {
        tag
        for case in materialized
        for tag in require_array(case["covers"], f"{case['id']}.covers")
    }
    require(set(required_coverage) <= observed_coverage, "required coverage is not observed")

    gates = require_object(value["acceptance_gates"], "manifest.acceptance_gates")
    require_exact_keys(
        gates,
        {
            "minimum_cases",
            "minimum_checks_blocked_cases",
            "minimum_checks_clear_controls",
            "minimum_inconclusive_controls",
            "minimum_legacy_status_cases",
            "minimum_partial_cases",
            "minimum_pinned_requirements",
        },
        "manifest.acceptance_gates",
    )
    for key, item in gates.items():
        require_int(item, f"manifest.acceptance_gates.{key}", 1)

    derived_cases = require_object(summary["verdicts"], "summary.verdicts")
    partial_cases = sum(
        require_object(case["snapshot"], f"{case['id']}.snapshot")["collection"]["status"]
        == "partial"
        for case in materialized
    )
    pinned_requirements = sum(
        len(
            {
                (requirement["context"], requirement["expected_app_id"])
                for requirement in case["snapshot"]["requirements"]
                if requirement["expected_app_id"] is not None
            }
        )
        for case in materialized
    )
    legacy_cases = sum(bool(case["snapshot"]["statuses"]) for case in materialized)
    require(summary["cases"] >= gates["minimum_cases"], "minimum case gate failed")
    require(
        derived_cases["checks_blocked"] >= gates["minimum_checks_blocked_cases"],
        "blocked-case gate failed",
    )
    require(
        derived_cases["checks_clear"] >= gates["minimum_checks_clear_controls"],
        "clear-control gate failed",
    )
    require(
        derived_cases["inconclusive"] >= gates["minimum_inconclusive_controls"],
        "inconclusive-control gate failed",
    )
    require(legacy_cases >= gates["minimum_legacy_status_cases"], "legacy-status gate failed")
    require(partial_cases >= gates["minimum_partial_cases"], "partial-case gate failed")
    require(
        pinned_requirements >= gates["minimum_pinned_requirements"],
        "pinned-requirement gate failed",
    )


def validate_bundle_data(
    cases_bytes: bytes,
    cases_asset: dict[str, object],
    oracle_bytes: bytes,
    oracle: dict[str, object],
    manifest: dict[str, object],
) -> tuple[list[dict[str, object]], dict[str, object], dict[str, object]]:
    materialized = validate_cases_asset(cases_asset)
    case_ids = {case["id"] for case in materialized}
    validate_oracle(oracle, case_ids)
    derived = derive_oracle(materialized)
    require(oracle == derived, "frozen oracle differs from independent derivation")
    validate_named_scenarios(materialized, derived)
    summary = summarize(materialized, derived)
    validate_manifest(manifest, cases_bytes, oracle_bytes, materialized, summary)
    return materialized, derived, summary


def load_bundle(
    cases_path: Path, oracle_path: Path, manifest_path: Path
) -> tuple[bytes, dict[str, object], bytes, dict[str, object], dict[str, object]]:
    cases_bytes, cases_asset = read_canonical_json(cases_path)
    oracle_bytes, oracle = read_canonical_json(oracle_path)
    _, manifest = read_canonical_json(manifest_path)
    return cases_bytes, cases_asset, oracle_bytes, oracle, manifest


def parse_checksum_text(value: str) -> dict[str, str]:
    entries: dict[str, str] = {}
    for line_number, line in enumerate(value.splitlines(), 1):
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9_.-]+)", line)
        require(match is not None, f"SHA256SUMS line {line_number} is malformed")
        assert match is not None
        digest, name = match.groups()
        require(name not in entries, f"SHA256SUMS repeats {name}")
        entries[name] = digest
    require(set(entries) == EXPECTED_CHECKSUM_FILES, "SHA256SUMS file set differs")
    return entries


def validate_checksums(path: Path = CHECKSUMS) -> None:
    entries = parse_checksum_text(path.read_text(encoding="utf-8"))
    for name, digest in entries.items():
        require(sha256_file(BUNDLE / name) == digest, f"SHA256SUMS mismatch for {name}")


def rebound_manifest(
    manifest: dict[str, object], cases_asset: dict[str, object], oracle: dict[str, object]
) -> dict[str, object]:
    rebound = copy.deepcopy(manifest)
    rebound["assets"]["cases"]["sha256"] = sha256_bytes(canonical_json(cases_asset))
    rebound["assets"]["oracle"]["sha256"] = sha256_bytes(canonical_json(oracle))
    return rebound


def expect_failure(action: Callable[[], object], label: str) -> None:
    failed = False
    try:
        action()
    except BenchmarkError:
        failed = True
    require(failed, f"self-test mutation unexpectedly passed: {label}")


def validate_mutation(
    cases_asset: dict[str, object], oracle: dict[str, object], manifest: dict[str, object]
) -> None:
    rebound = rebound_manifest(manifest, cases_asset, oracle)
    validate_bundle_data(
        canonical_json(cases_asset),
        cases_asset,
        canonical_json(oracle),
        oracle,
        rebound,
    )


def command_verify(arguments: argparse.Namespace) -> None:
    cases_bytes, cases_asset, oracle_bytes, oracle, manifest = load_bundle(
        arguments.cases, arguments.oracle, arguments.manifest
    )
    _, _, summary = validate_bundle_data(
        cases_bytes, cases_asset, oracle_bytes, oracle, manifest
    )
    validate_checksums()
    verdicts = summary["verdicts"]
    print(
        "verified Pull Request Doctor v1: "
        f"{summary['cases']} controlled cases, "
        f"{verdicts['checks_clear']} clear controls, "
        f"{verdicts['checks_blocked']} blocked, "
        f"{verdicts['inconclusive']} inconclusive"
    )


def command_summary(arguments: argparse.Namespace) -> None:
    _, cases_asset = read_canonical_json(arguments.cases)
    materialized = validate_cases_asset(cases_asset)
    derived = derive_oracle(materialized)
    print(canonical_json(summarize(materialized, derived)).decode("utf-8"), end="")


def command_derive_oracle(arguments: argparse.Namespace) -> None:
    _, cases_asset = read_canonical_json(arguments.cases)
    materialized = validate_cases_asset(cases_asset)
    print(canonical_json(derive_oracle(materialized)).decode("utf-8"), end="")


def command_materialize(arguments: argparse.Namespace) -> None:
    _, cases_asset = read_canonical_json(arguments.cases)
    materialized = validate_cases_asset(cases_asset)
    value = {
        "cases": [
            {"id": case["id"], "snapshot": case["snapshot"]}
            for case in materialized
        ],
        "dataset_version": DATASET_VERSION,
        "schema": MATERIALIZED_SCHEMA,
    }
    print(canonical_json(value).decode("utf-8"), end="")


def command_self_test(arguments: argparse.Namespace) -> None:
    cases_bytes, cases_asset, oracle_bytes, oracle, manifest = load_bundle(
        arguments.cases, arguments.oracle, arguments.manifest
    )
    validate_bundle_data(cases_bytes, cases_asset, oracle_bytes, oracle, manifest)
    validate_checksums()

    forged_oracle = copy.deepcopy(oracle)
    forged_oracle["cases"]["missing-complete"]["requirements"][0]["status"] = "satisfied"
    expect_failure(
        lambda: validate_mutation(cases_asset, forged_oracle, manifest),
        "forged oracle status",
    )

    omitted_cases = copy.deepcopy(cases_asset)
    omitted_oracle = copy.deepcopy(oracle)
    omitted_cases["cases"].pop()
    del omitted_oracle["cases"]["head-drift-incomplete"]
    expect_failure(
        lambda: validate_mutation(omitted_cases, omitted_oracle, manifest),
        "omitted required scenario",
    )

    false_complete = copy.deepcopy(cases_asset)
    for case in false_complete["cases"]:
        if case["id"] == "missing-partial-inconclusive":
            case["snapshot"]["collection"]["gaps"] = []
            case["snapshot"]["collection"]["status"] = "complete"
    expect_failure(
        lambda: validate_mutation(false_complete, oracle, manifest),
        "partial absence promoted to complete missing",
    )

    no_dedupe_control = copy.deepcopy(cases_asset)
    for case in no_dedupe_control["cases"]:
        if case["id"] == "duplicate-policy-dedupe":
            case["snapshot"]["requirements"].pop()
    expect_failure(
        lambda: validate_mutation(no_dedupe_control, oracle, manifest),
        "duplicate-policy control removed",
    )

    no_head_gap = copy.deepcopy(cases_asset)
    for case in no_head_gap["cases"]:
        if case["id"] == "head-drift-incomplete":
            case["snapshot"]["collection"]["gaps"] = []
            case["snapshot"]["collection"]["status"] = "complete"
    expect_failure(
        lambda: validate_mutation(no_head_gap, oracle, manifest),
        "head drift promoted to clear",
    )

    weak_boundary = copy.deepcopy(manifest)
    weak_boundary["claim_boundary"]["production_accuracy_supported"] = True
    expect_failure(
        lambda: validate_bundle_data(
            cases_bytes, cases_asset, oracle_bytes, oracle, weak_boundary
        ),
        "weakened claim boundary",
    )

    stale_hash = copy.deepcopy(manifest)
    stale_hash["assets"]["cases"]["sha256"] = "0" * 64
    expect_failure(
        lambda: validate_bundle_data(cases_bytes, cases_asset, oracle_bytes, oracle, stale_hash),
        "stale asset hash",
    )

    leaked_payload = copy.deepcopy(cases_asset)
    leaked_payload["cases"][0]["snapshot"]["check_runs"][0]["output"] = "not allowed"
    expect_failure(
        lambda: validate_mutation(leaked_payload, oracle, manifest),
        "forbidden check output",
    )

    collapsed_apps = copy.deepcopy(cases_asset)
    for case in collapsed_apps["cases"]:
        if case["id"] == "source-ambiguous-unpinned":
            case["snapshot"]["check_runs"][1]["app_id"] = 15368
            case["snapshot"]["check_runs"][1]["app_slug"] = "github-actions"
    expect_failure(
        lambda: validate_mutation(collapsed_apps, oracle, manifest),
        "source ambiguity collapsed into one App",
    )

    hidden_legacy_failure = copy.deepcopy(cases_asset)
    for case in hidden_legacy_failure["cases"]:
        if case["id"] == "pinned-check-legacy-failure":
            case["snapshot"]["statuses"] = []
    expect_failure(
        lambda: validate_mutation(hidden_legacy_failure, oracle, manifest),
        "pinned legacy failure removed",
    )

    erased_partial_blocker = copy.deepcopy(cases_asset)
    for case in erased_partial_blocker["cases"]:
        if case["id"] == "partial-known-failure":
            case["snapshot"]["check_runs"][0]["conclusion"] = "success"
    expect_failure(
        lambda: validate_mutation(erased_partial_blocker, oracle, manifest),
        "known blocker erased under partial collection",
    )

    expect_failure(
        lambda: json.loads(
            b'{"schema":"one","schema":"two"}\n',
            object_pairs_hook=unique_json_object,
        ),
        "duplicate JSON key",
    )

    checksum_text = CHECKSUMS.read_text(encoding="utf-8").replace(
        "README.md", "README-copy.md", 1
    )
    expect_failure(lambda: parse_checksum_text(checksum_text), "checksum file substitution")

    print("self-test passed: 13 tamper and fail-closed mutations rejected")


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser(description=__doc__)
    value.add_argument("--cases", type=Path, default=DEFAULT_CASES)
    value.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    value.add_argument("--oracle", type=Path, default=DEFAULT_ORACLE)
    subcommands = value.add_subparsers(dest="command", required=True)
    subcommands.add_parser("verify").set_defaults(handler=command_verify)
    subcommands.add_parser("self-test").set_defaults(handler=command_self_test)
    subcommands.add_parser("derive-oracle").set_defaults(handler=command_derive_oracle)
    subcommands.add_parser("summary").set_defaults(handler=command_summary)
    subcommands.add_parser("materialize").set_defaults(handler=command_materialize)
    return value


def main() -> None:
    arguments = parser().parse_args()
    arguments.handler(arguments)


if __name__ == "__main__":
    main()
