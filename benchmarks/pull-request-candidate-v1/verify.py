#!/usr/bin/env python3
"""Verify the offline Pull Request Candidate v1 benchmark."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
from typing import Callable


BUNDLE = Path(__file__).resolve().parent
CASES_PATH = BUNDLE / "cases.json"
MANIFEST_PATH = BUNDLE / "manifest.json"
ORACLE_PATH = BUNDLE / "oracle.json"
CHECKSUMS_PATH = BUNDLE / "SHA256SUMS"

CASES_SCHEMA = "stratadiff-pull-request-candidate-cases-v1"
MANIFEST_SCHEMA = "stratadiff-pull-request-candidate-manifest-v1"
ORACLE_SCHEMA = "stratadiff-pull-request-candidate-oracle-v1"
DATASET_VERSION = "1.0.0"

CASE_ID_PATTERN = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
COVERAGE_PATTERN = re.compile(r"^[a-z][a-z0-9_]*$")
DATE_PATTERN = re.compile(r"^\d{4}-\d{2}-\d{2}$")
OID_PATTERN = re.compile(r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$")
REPOSITORY_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")

CANDIDATE_KINDS = {"merge_group", "pr_head", "test_merge"}
COLLECTION_STATES = {"complete", "gap"}
OUTCOME_STATES = {"inconclusive", "retry", "selected"}
QUEUE_STATES = {"AWAITING_CHECKS", "LOCKED", "MERGEABLE", "QUEUED", "UNMERGEABLE"}
REASON_CODES = {
    "potential_merge_unavailable",
    "queue_entry_candidate",
    "queue_entry_unavailable",
    "signal_surface_gap",
    "target_drift",
    "test_merge_empty",
    "test_merge_signal",
}

EXPECTED_COVERAGE = {
    "api_gap_fail_closed",
    "live_capture_control",
    "multi_entry_queue_chain",
    "old_queue_green_new_missing",
    "queue_enabled_not_enqueued",
    "queue_target_drift",
    "queued_entry_missing",
    "queued_graphql_entry",
    "test_merge_check_signal",
    "test_merge_empty_fail_closed",
    "test_merge_legacy_status_signal",
    "test_merge_null",
}

EXPECTED_CASE_IDS = {
    "clickhouse-queued-entry",
    "github-docs-empty-test-merge",
    "old-queue-green-new-missing",
    "queue-enabled-not-enqueued",
    "queue-entry-drift",
    "queued-entry-missing",
    "test-merge-api-gap",
    "test-merge-check-signal",
    "test-merge-null",
    "test-merge-status-signal",
}

CLAIM_BOUNDARY = {
    "controlled_candidate_selection_supported": True,
    "end_to_end_collector_conformance_supported": False,
    "failure_prevalence_supported": False,
    "live_capture_currentness_supported": False,
    "merge_safety_supported": False,
    "production_accuracy_supported": False,
    "product_market_fit_supported": False,
}

EXPECTED_CHECKSUM_FILES = {
    "README.md",
    "cases.json",
    "manifest.json",
    "oracle.json",
    "verify.py",
}

DOCS_CAPTURE = {
    "base_sha": "831337b0fed60b90a72e2711a41dfcad72b5f288",
    "head_sha": "ca3e8c456e1e9cb81a9d194ec14afbcf78dee60c",
    "is_in_merge_queue": False,
    "is_merge_queue_enabled": True,
    "potential_merge_sha": "775b52f54d8348776d5ea0984823bad5da4dc92c",
    "pull_request": 45788,
    "test_merge_status_check_rollup": "absent",
}

CLICKHOUSE_ENTRIES = [
    {
        "base_sha": "16723d7e43a27fb1925d29f8f11c3a46e51b045a",
        "head_sha": "7b33e4ced120cbec2932b81bdbb2a75c6245a7c4",
        "id": "MQE_lQDOA5dJV88AAAABBVoJNs2aL84CwqYU",
        "position": 1,
        "pr_base_sha": "b922ae0ec55876ddebfd7cbda355a21b2d9ffe6b",
        "pr_head_sha": "5d5f1bb2928c76f103c6e4d60f3c28cd22daef3d",
        "pull_request": 116884,
        "state": "AWAITING_CHECKS",
    },
    {
        "base_sha": "7b33e4ced120cbec2932b81bdbb2a75c6245a7c4",
        "head_sha": "c220c3f9c222c84444575d17236079c0d3032103",
        "id": "MQE_lQDOA5dJV88AAAABAm2DHc2aL84CwqYy",
        "position": 2,
        "pr_base_sha": "73a16510728af191cc2b3be1f2234202d6d4d2f2",
        "pr_head_sha": "c0d7e0fe804ff2642e2687bdee4782a2abb31970",
        "pull_request": 115869,
        "state": "AWAITING_CHECKS",
    },
    {
        "base_sha": "c220c3f9c222c84444575d17236079c0d3032103",
        "head_sha": "716fe0d51ce91010ba437e7e0d97d6d5aabb4427",
        "id": "MQE_lQDOA5dJV88AAAABChnHBc2aL84Cwrqe",
        "position": 3,
        "pr_base_sha": "05f500731250ae3c47a6a718356ba03a769526fc",
        "pr_head_sha": "90866e140173ab9d119e9cfe2dff6a553dd2c21d",
        "pull_request": 118568,
        "state": "AWAITING_CHECKS",
    },
    {
        "base_sha": "716fe0d51ce91010ba437e7e0d97d6d5aabb4427",
        "head_sha": "aea04d668a1be653a213f99a6718309e87fe7093",
        "id": "MQE_lQDOA5dJV88AAAABAeYfbM2aL84CwsVc",
        "position": 4,
        "pr_base_sha": "26bd12dc68f43e7076765c9151b589897f25deba",
        "pr_head_sha": "e12888a649c1856179606fb6d0ef66d7b8d88502",
        "pull_request": 115675,
        "state": "AWAITING_CHECKS",
    },
    {
        "base_sha": "aea04d668a1be653a213f99a6718309e87fe7093",
        "head_sha": "3685123b11b88a967d5428dd61a9dfdf28ed06b8",
        "id": "MQE_lQDOA5dJV88AAAABCjXsfs2aL84Cws0o",
        "position": 5,
        "pr_base_sha": "5f096d5da845ea68c9639f6a327eea885a793b30",
        "pr_head_sha": "b6d382a5fdbd8ea43edb24d56b8f5cf37880aca1",
        "pull_request": 118616,
        "state": "AWAITING_CHECKS",
    },
]


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
    require(
        not any(ord(character) < 32 or ord(character) == 127 for character in value),
        f"{label} has control text",
    )
    return value


def require_int(value: object, label: str, minimum: int = 0) -> int:
    require(type(value) is int and value >= minimum, f"{label} must be an integer >= {minimum}")
    assert isinstance(value, int)
    return value


def require_bool(value: object, label: str) -> bool:
    require(type(value) is bool, f"{label} must be a boolean")
    assert isinstance(value, bool)
    return value


def require_oid(value: object, label: str) -> str:
    oid = require_string(value, label)
    require(bool(OID_PATTERN.fullmatch(oid)), f"{label} must be a lowercase full Git object ID")
    return oid


def require_nullable_oid(value: object, label: str) -> str | None:
    if value is None:
        return None
    return require_oid(value, label)


def unique_json_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        require(key not in value, f"duplicate JSON key: {key}")
        value[key] = item
    return value


def read_json(path: Path) -> tuple[bytes, dict[str, object]]:
    payload = path.read_bytes()
    value = json.loads(payload, object_pairs_hook=unique_json_object)
    return payload, require_object(value, str(path))


def sha256_bytes(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def validate_queue_entry(value: object, label: str) -> dict[str, object]:
    entry = require_object(value, label)
    require_exact_keys(entry, {"base_sha", "head_sha", "id", "position", "state"}, label)
    base_sha = require_oid(entry["base_sha"], f"{label}.base_sha")
    head_sha = require_oid(entry["head_sha"], f"{label}.head_sha")
    require(base_sha != head_sha, f"{label} base and head must differ")
    require_string(entry["id"], f"{label}.id")
    require_int(entry["position"], f"{label}.position", 1)
    state = require_string(entry["state"], f"{label}.state")
    require(state in QUEUE_STATES, f"{label}.state is invalid")
    return entry


def validate_target(value: object, label: str) -> dict[str, object]:
    target = require_object(value, label)
    require_exact_keys(
        target,
        {
            "base_sha",
            "head_sha",
            "is_in_merge_queue",
            "is_merge_queue_enabled",
            "potential_merge_sha",
            "queue_entry",
            "state",
        },
        label,
    )
    require(target["state"] == "open", f"{label}.state must be open")
    base_sha = require_oid(target["base_sha"], f"{label}.base_sha")
    head_sha = require_oid(target["head_sha"], f"{label}.head_sha")
    require(base_sha != head_sha, f"{label} PR base and head must differ")
    queue_enabled = require_bool(target["is_merge_queue_enabled"], f"{label}.is_merge_queue_enabled")
    in_queue = require_bool(target["is_in_merge_queue"], f"{label}.is_in_merge_queue")
    require_nullable_oid(target["potential_merge_sha"], f"{label}.potential_merge_sha")
    if target["queue_entry"] is None:
        pass
    else:
        entry = validate_queue_entry(target["queue_entry"], f"{label}.queue_entry")
        require(queue_enabled and in_queue, f"{label} has an entry without active queue membership")
        require(entry["head_sha"] != head_sha, f"{label} queue candidate must differ from PR head")
    require(not in_queue or queue_enabled, f"{label} cannot be queued when the queue is disabled")
    require(in_queue or target["queue_entry"] is None, f"{label} has a stale queue entry")
    return target


def validate_signal_surface(value: object, label: str) -> dict[str, object]:
    surface = require_object(value, label)
    require_exact_keys(surface, {"collection", "other", "success"}, label)
    collection = require_string(surface["collection"], f"{label}.collection")
    require(collection in COLLECTION_STATES, f"{label}.collection is invalid")
    success = require_int(surface["success"], f"{label}.success")
    other = require_int(surface["other"], f"{label}.other")
    require(
        collection == "complete" or success + other == 0,
        f"{label} cannot report signals from an unavailable surface",
    )
    return surface


def validate_signals(value: object, label: str) -> list[dict[str, object]]:
    raw_signals = require_array(value, label)
    signals: list[dict[str, object]] = []
    seen: set[str] = set()
    for index, item in enumerate(raw_signals):
        item_label = f"{label}[{index}]"
        signal = require_object(item, item_label)
        require_exact_keys(signal, {"check_runs", "commit_statuses", "sha"}, item_label)
        sha = require_oid(signal["sha"], f"{item_label}.sha")
        require(sha not in seen, f"{label} repeats {sha}")
        seen.add(sha)
        validate_signal_surface(signal["check_runs"], f"{item_label}.check_runs")
        validate_signal_surface(signal["commit_statuses"], f"{item_label}.commit_statuses")
        signals.append(signal)
    return signals


def validate_docs_provenance(provenance: dict[str, object], label: str) -> None:
    require(provenance["id"] == "github-docs-45788-2026-09-08", f"{label}.id changed")
    require(provenance["repository"] == "github/docs", f"{label}.repository changed")
    require(provenance["source_query"] == "repository.pullRequest(number:45788)", f"{label}.source_query changed")
    require(provenance["source_url"] == "https://github.com/github/docs/pull/45788", f"{label}.source_url changed")
    values = require_object(provenance["values"], f"{label}.values")
    require(values == DOCS_CAPTURE, f"{label}.values no longer match the frozen capture")


def validate_clickhouse_entry(value: object, label: str) -> dict[str, object]:
    entry = require_object(value, label)
    require_exact_keys(
        entry,
        {
            "base_sha",
            "head_sha",
            "id",
            "position",
            "pr_base_sha",
            "pr_head_sha",
            "pull_request",
            "state",
        },
        label,
    )
    base_sha = require_oid(entry["base_sha"], f"{label}.base_sha")
    head_sha = require_oid(entry["head_sha"], f"{label}.head_sha")
    pr_base_sha = require_oid(entry["pr_base_sha"], f"{label}.pr_base_sha")
    pr_head_sha = require_oid(entry["pr_head_sha"], f"{label}.pr_head_sha")
    require(len({base_sha, head_sha, pr_head_sha}) == 3, f"{label} does not preserve distinct queue identities")
    require(pr_base_sha != pr_head_sha, f"{label} PR base and head must differ")
    require_string(entry["id"], f"{label}.id")
    require_int(entry["position"], f"{label}.position", 1)
    require_int(entry["pull_request"], f"{label}.pull_request", 1)
    require(entry["state"] == "AWAITING_CHECKS", f"{label}.state changed")
    return entry


def validate_clickhouse_provenance(provenance: dict[str, object], label: str) -> None:
    require(provenance["id"] == "clickhouse-queue-first-five-2026-09-08", f"{label}.id changed")
    require(provenance["repository"] == "ClickHouse/ClickHouse", f"{label}.repository changed")
    require(provenance["source_query"] == "repository.mergeQueue.entries(first:5)", f"{label}.source_query changed")
    require(provenance["source_url"] == "https://github.com/ClickHouse/ClickHouse/queue/master", f"{label}.source_url changed")
    values = require_object(provenance["values"], f"{label}.values")
    require_exact_keys(values, {"entries"}, f"{label}.values")
    raw_entries = require_array(values["entries"], f"{label}.values.entries")
    entries = [validate_clickhouse_entry(item, f"{label}.values.entries[{index}]") for index, item in enumerate(raw_entries)]
    require(entries == CLICKHOUSE_ENTRIES, f"{label}.values no longer match the frozen capture")
    for index, entry in enumerate(entries):
        require(entry["position"] == index + 1, f"{label} queue positions are not contiguous")
        if index > 0:
            require(entry["base_sha"] == entries[index - 1]["head_sha"], f"{label} queue commit chain is broken")


def validate_provenance(value: object) -> dict[str, dict[str, object]]:
    raw_items = require_array(value, "provenance")
    require(len(raw_items) == 2, "provenance must contain exactly two captures")
    result: dict[str, dict[str, object]] = {}
    for index, item in enumerate(raw_items):
        label = f"provenance[{index}]"
        provenance = require_object(item, label)
        require_exact_keys(
            provenance,
            {"captured_on", "id", "kind", "repository", "source_query", "source_url", "values"},
            label,
        )
        identifier = require_string(provenance["id"], f"{label}.id")
        require(identifier not in result, f"duplicate provenance id {identifier}")
        captured_on = require_string(provenance["captured_on"], f"{label}.captured_on")
        require(bool(DATE_PATTERN.fullmatch(captured_on)), f"{label}.captured_on is invalid")
        require(captured_on == "2026-09-08", f"{label}.captured_on changed")
        repository = require_string(provenance["repository"], f"{label}.repository")
        require(bool(REPOSITORY_PATTERN.fullmatch(repository)), f"{label}.repository is invalid")
        require_string(provenance["source_query"], f"{label}.source_query")
        source_url = require_string(provenance["source_url"], f"{label}.source_url")
        require(source_url.startswith(f"https://github.com/{repository}"), f"{label}.source_url is outside the repository")
        kind = require_string(provenance["kind"], f"{label}.kind")
        if kind == "pull_request":
            validate_docs_provenance(provenance, label)
        elif kind == "merge_queue":
            validate_clickhouse_provenance(provenance, label)
        else:
            raise BenchmarkError(f"{label}.kind is invalid")
        result[identifier] = provenance
    return result


def validate_case_provenance(case: dict[str, object], provenance: dict[str, dict[str, object]], label: str) -> None:
    raw_ids = require_array(case["provenance_ids"], f"{label}.provenance_ids")
    provenance_ids = [require_string(item, f"{label}.provenance_ids") for item in raw_ids]
    require(provenance_ids == sorted(set(provenance_ids)), f"{label}.provenance_ids must be sorted and unique")
    for identifier in provenance_ids:
        require(identifier in provenance, f"{label} references unknown provenance {identifier}")

    before = require_object(require_object(case["observation"], f"{label}.observation")["before"], f"{label}.observation.before")
    if provenance_ids == ["github-docs-45788-2026-09-08"]:
        require(before["base_sha"] == DOCS_CAPTURE["base_sha"], f"{label} docs base does not match provenance")
        require(before["head_sha"] == DOCS_CAPTURE["head_sha"], f"{label} docs head does not match provenance")
        require(before["potential_merge_sha"] == DOCS_CAPTURE["potential_merge_sha"], f"{label} docs test merge does not match provenance")
        require(before["is_in_merge_queue"] == DOCS_CAPTURE["is_in_merge_queue"], f"{label} docs queue membership does not match provenance")
        require(before["is_merge_queue_enabled"] == DOCS_CAPTURE["is_merge_queue_enabled"], f"{label} docs queue availability does not match provenance")
    elif provenance_ids == ["clickhouse-queue-first-five-2026-09-08"]:
        first = CLICKHOUSE_ENTRIES[0]
        require(before["base_sha"] == first["pr_base_sha"], f"{label} ClickHouse PR base does not match provenance")
        require(before["head_sha"] == first["pr_head_sha"], f"{label} ClickHouse PR head does not match provenance")
        queue_entry = require_object(before["queue_entry"], f"{label}.observation.before.queue_entry")
        require(queue_entry["base_sha"] == first["base_sha"], f"{label} ClickHouse queue base does not match provenance")
        require(queue_entry["head_sha"] == first["head_sha"], f"{label} ClickHouse queue head does not match provenance")
        require(queue_entry["id"] == first["id"], f"{label} ClickHouse queue entry does not match provenance")
        require(queue_entry["position"] == first["position"], f"{label} ClickHouse position does not match provenance")
        require(queue_entry["state"] == first["state"], f"{label} ClickHouse state does not match provenance")


def validate_cases(value: dict[str, object]) -> list[dict[str, object]]:
    require_exact_keys(value, {"cases", "dataset_version", "provenance", "schema"}, "cases.json")
    require(value["schema"] == CASES_SCHEMA, "cases schema changed")
    require(value["dataset_version"] == DATASET_VERSION, "cases dataset version changed")
    provenance = validate_provenance(value["provenance"])
    raw_cases = require_array(value["cases"], "cases")
    cases: list[dict[str, object]] = []
    case_ids: set[str] = set()
    coverage_seen: set[str] = set()
    for index, item in enumerate(raw_cases):
        label = f"cases[{index}]"
        case = require_object(item, label)
        require_exact_keys(case, {"coverage", "description", "id", "observation", "provenance_ids"}, label)
        case_id = require_string(case["id"], f"{label}.id")
        require(bool(CASE_ID_PATTERN.fullmatch(case_id)), f"{label}.id is invalid")
        require(case_id not in case_ids, f"duplicate case id {case_id}")
        case_ids.add(case_id)
        require_string(case["description"], f"{label}.description")
        raw_coverage = require_array(case["coverage"], f"{label}.coverage")
        coverage = [require_string(tag, f"{label}.coverage") for tag in raw_coverage]
        require(bool(coverage), f"{label}.coverage must not be empty")
        require(coverage == sorted(set(coverage)), f"{label}.coverage must be sorted and unique")
        require(all(COVERAGE_PATTERN.fullmatch(tag) for tag in coverage), f"{label}.coverage has an invalid tag")
        coverage_seen.update(coverage)
        observation = require_object(case["observation"], f"{label}.observation")
        require_exact_keys(observation, {"after", "before", "signals"}, f"{label}.observation")
        validate_target(observation["before"], f"{label}.observation.before")
        validate_target(observation["after"], f"{label}.observation.after")
        validate_signals(observation["signals"], f"{label}.observation.signals")
        validate_case_provenance(case, provenance, label)
        cases.append(case)
    require(case_ids == EXPECTED_CASE_IDS, "case inventory changed")
    require(coverage_seen == EXPECTED_COVERAGE, "coverage inventory changed")
    return cases


def signal_count(signal: dict[str, object]) -> int:
    check_runs = require_object(signal["check_runs"], "signal.check_runs")
    statuses = require_object(signal["commit_statuses"], "signal.commit_statuses")
    return int(check_runs["success"]) + int(check_runs["other"]) + int(statuses["success"]) + int(statuses["other"])


def ignored_signal_shas(signals: list[dict[str, object]], selected_sha: str | None) -> list[str]:
    return sorted(
        str(signal["sha"])
        for signal in signals
        if signal["sha"] != selected_sha and signal_count(signal) > 0
    )


def outcome(
    case_id: str,
    status: str,
    candidate_kind: str | None,
    sha: str | None,
    base_sha: str | None,
    queue_entry_id: str | None,
    reason_code: str,
    ignored: list[str],
) -> dict[str, object]:
    return {
        "case_id": case_id,
        "status": status,
        "candidate_kind": candidate_kind,
        "sha": sha,
        "base_sha": base_sha,
        "queue_entry_id": queue_entry_id,
        "reason_code": reason_code,
        "ignored_signal_shas": ignored,
    }


def derive_case(case: dict[str, object]) -> dict[str, object]:
    case_id = str(case["id"])
    observation = require_object(case["observation"], f"{case_id}.observation")
    before = require_object(observation["before"], f"{case_id}.before")
    after = require_object(observation["after"], f"{case_id}.after")
    signals = [require_object(item, f"{case_id}.signals") for item in require_array(observation["signals"], f"{case_id}.signals")]

    if before != after:
        return outcome(case_id, "retry", None, None, None, None, "target_drift", ignored_signal_shas(signals, None))

    if bool(after["is_in_merge_queue"]):
        if after["queue_entry"] is None:
            return outcome(case_id, "inconclusive", None, None, None, None, "queue_entry_unavailable", ignored_signal_shas(signals, None))
        entry = require_object(after["queue_entry"], f"{case_id}.queue_entry")
        selected_sha = str(entry["head_sha"])
        return outcome(
            case_id,
            "selected",
            "merge_group",
            selected_sha,
            str(entry["base_sha"]),
            str(entry["id"]),
            "queue_entry_candidate",
            ignored_signal_shas(signals, selected_sha),
        )

    potential_merge_sha = after["potential_merge_sha"]
    if potential_merge_sha is None:
        return outcome(case_id, "inconclusive", None, None, None, None, "potential_merge_unavailable", ignored_signal_shas(signals, None))

    matching = [signal for signal in signals if signal["sha"] == potential_merge_sha]
    if len(matching) != 1:
        return outcome(case_id, "inconclusive", None, None, None, None, "signal_surface_gap", ignored_signal_shas(signals, None))
    signal = matching[0]
    check_runs = require_object(signal["check_runs"], f"{case_id}.check_runs")
    statuses = require_object(signal["commit_statuses"], f"{case_id}.commit_statuses")
    if check_runs["collection"] != "complete" or statuses["collection"] != "complete":
        return outcome(case_id, "inconclusive", None, None, None, None, "signal_surface_gap", ignored_signal_shas(signals, None))
    if signal_count(signal) > 0:
        selected_sha = str(potential_merge_sha)
        return outcome(
            case_id,
            "selected",
            "test_merge",
            selected_sha,
            str(after["base_sha"]),
            None,
            "test_merge_signal",
            ignored_signal_shas(signals, selected_sha),
        )
    return outcome(
        case_id,
        "inconclusive",
        None,
        None,
        None,
        None,
        "test_merge_empty",
        ignored_signal_shas(signals, None),
    )


def derive_oracle(cases: list[dict[str, object]]) -> dict[str, object]:
    return {
        "schema": ORACLE_SCHEMA,
        "dataset_version": DATASET_VERSION,
        "outcomes": [derive_case(case) for case in cases],
    }


def validate_outcome(value: object, label: str) -> dict[str, object]:
    result = require_object(value, label)
    require_exact_keys(
        result,
        {"base_sha", "candidate_kind", "case_id", "ignored_signal_shas", "queue_entry_id", "reason_code", "sha", "status"},
        label,
    )
    require_string(result["case_id"], f"{label}.case_id")
    status = require_string(result["status"], f"{label}.status")
    require(status in OUTCOME_STATES, f"{label}.status is invalid")
    reason_code = require_string(result["reason_code"], f"{label}.reason_code")
    require(reason_code in REASON_CODES, f"{label}.reason_code is invalid")
    ignored_raw = require_array(result["ignored_signal_shas"], f"{label}.ignored_signal_shas")
    ignored = [require_oid(item, f"{label}.ignored_signal_shas") for item in ignored_raw]
    require(ignored == sorted(set(ignored)), f"{label}.ignored_signal_shas must be sorted and unique")
    if status == "selected":
        kind = require_string(result["candidate_kind"], f"{label}.candidate_kind")
        require(kind in CANDIDATE_KINDS, f"{label}.candidate_kind is invalid")
        sha = require_oid(result["sha"], f"{label}.sha")
        require(sha not in ignored, f"{label} ignores its selected SHA")
        if kind == "pr_head":
            require(result["base_sha"] is None and result["queue_entry_id"] is None, f"{label} PR head metadata is invalid")
        elif kind == "test_merge":
            require_oid(result["base_sha"], f"{label}.base_sha")
            require(result["queue_entry_id"] is None, f"{label} test merge cannot have a queue entry")
        else:
            require_oid(result["base_sha"], f"{label}.base_sha")
            require_string(result["queue_entry_id"], f"{label}.queue_entry_id")
    else:
        require(result["candidate_kind"] is None, f"{label} unresolved outcome has a candidate kind")
        require(result["sha"] is None, f"{label} unresolved outcome has a SHA")
        require(result["base_sha"] is None, f"{label} unresolved outcome has a base SHA")
        require(result["queue_entry_id"] is None, f"{label} unresolved outcome has a queue entry")
    return result


def validate_oracle(value: dict[str, object], cases: list[dict[str, object]]) -> list[dict[str, object]]:
    require_exact_keys(value, {"dataset_version", "outcomes", "schema"}, "oracle.json")
    require(value["schema"] == ORACLE_SCHEMA, "oracle schema changed")
    require(value["dataset_version"] == DATASET_VERSION, "oracle dataset version changed")
    raw_outcomes = require_array(value["outcomes"], "oracle.outcomes")
    outcomes = [validate_outcome(item, f"oracle.outcomes[{index}]") for index, item in enumerate(raw_outcomes)]
    case_ids = [str(case["id"]) for case in cases]
    outcome_ids = [str(item["case_id"]) for item in outcomes]
    require(outcome_ids == case_ids, "oracle outcome order or inventory differs from cases")
    require(value == derive_oracle(cases), "oracle differs from independent candidate derivation")
    return outcomes


def summarize(cases_value: dict[str, object], outcomes: list[dict[str, object]]) -> dict[str, object]:
    cases = require_array(cases_value["cases"], "cases")
    provenance = require_array(cases_value["provenance"], "provenance")
    coverage = {
        str(tag)
        for case_value in cases
        for tag in require_array(require_object(case_value, "case")["coverage"], "case.coverage")
    }
    outcome_counts = {state: sum(item["status"] == state for item in outcomes) for state in sorted(OUTCOME_STATES)}
    kind_counts = {kind: sum(item["candidate_kind"] == kind for item in outcomes) for kind in sorted(CANDIDATE_KINDS)}
    queue_entries = sum(
        len(require_array(require_object(require_object(item, "provenance")["values"], "provenance.values")["entries"], "entries"))
        for item in provenance
        if require_object(item, "provenance")["kind"] == "merge_queue"
    )
    return {
        "candidate_kinds": kind_counts,
        "cases": len(cases),
        "coverage_tags": len(coverage),
        "live_provenance_captures": len(provenance),
        "outcomes": outcome_counts,
        "queue_entries_captured": queue_entries,
    }


def validate_manifest(
    value: dict[str, object],
    cases_payload: bytes,
    oracle_payload: bytes,
    cases_value: dict[str, object],
    outcomes: list[dict[str, object]],
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
        "manifest.json",
    )
    require(value["schema"] == MANIFEST_SCHEMA, "manifest schema changed")
    require(value["dataset_version"] == DATASET_VERSION, "manifest dataset version changed")
    require(value["name"] == "Pull Request Candidate v1", "manifest name changed")
    require(value["designation"] == "controlled_semantic_regression_with_live_provenance", "manifest designation changed")
    require_string(value["description"], "manifest.description")
    require(value["dataset_license"] == "MIT", "dataset license changed")
    require(value["claim_boundary"] == CLAIM_BOUNDARY, "claim boundary changed")

    required_coverage_raw = require_array(value["required_coverage"], "manifest.required_coverage")
    required_coverage = [require_string(tag, "manifest.required_coverage") for tag in required_coverage_raw]
    require(required_coverage == sorted(EXPECTED_COVERAGE), "manifest required coverage changed")

    assets = require_object(value["assets"], "manifest.assets")
    require_exact_keys(assets, {"cases", "oracle"}, "manifest.assets")
    for name, path, payload in (("cases", "cases.json", cases_payload), ("oracle", "oracle.json", oracle_payload)):
        asset = require_object(assets[name], f"manifest.assets.{name}")
        require_exact_keys(asset, {"path", "sha256"}, f"manifest.assets.{name}")
        require(asset["path"] == path, f"manifest.assets.{name}.path changed")
        digest = require_string(asset["sha256"], f"manifest.assets.{name}.sha256")
        require(bool(SHA256_PATTERN.fullmatch(digest)), f"manifest.assets.{name}.sha256 is invalid")
        require(digest == sha256_bytes(payload), f"manifest.assets.{name}.sha256 is stale")

    summary = summarize(cases_value, outcomes)
    require(value["expected_summary"] == summary, "manifest expected summary changed")
    gates = require_object(value["acceptance_gates"], "manifest.acceptance_gates")
    require_exact_keys(
        gates,
        {
            "minimum_cases",
            "minimum_inconclusive_cases",
            "minimum_live_provenance_captures",
            "minimum_merge_group_selections",
            "minimum_retry_cases",
            "minimum_selected_cases",
        },
        "manifest.acceptance_gates",
    )
    for key, item in gates.items():
        require_int(item, f"manifest.acceptance_gates.{key}")
    require(summary["cases"] >= gates["minimum_cases"], "minimum case gate failed")
    summary_outcomes = require_object(summary["outcomes"], "summary.outcomes")
    summary_kinds = require_object(summary["candidate_kinds"], "summary.candidate_kinds")
    require(summary_outcomes["inconclusive"] >= gates["minimum_inconclusive_cases"], "minimum inconclusive gate failed")
    require(summary["live_provenance_captures"] >= gates["minimum_live_provenance_captures"], "minimum live provenance gate failed")
    require(summary_kinds["merge_group"] >= gates["minimum_merge_group_selections"], "minimum merge-group gate failed")
    require(summary_outcomes["retry"] >= gates["minimum_retry_cases"], "minimum retry gate failed")
    require(summary_outcomes["selected"] >= gates["minimum_selected_cases"], "minimum selected gate failed")


def validate_checksum_text(payload: str) -> None:
    lines = payload.splitlines()
    entries: dict[str, str] = {}
    for index, line in enumerate(lines):
        parts = line.split("  ")
        require(len(parts) == 2, f"SHA256SUMS line {index + 1} is malformed")
        digest, name = parts
        require(bool(SHA256_PATTERN.fullmatch(digest)), f"SHA256SUMS digest for {name} is invalid")
        require(name not in entries, f"SHA256SUMS repeats {name}")
        require("/" not in name and name not in {".", ".."}, f"SHA256SUMS path {name} is unsafe")
        entries[name] = digest
    require(set(entries) == EXPECTED_CHECKSUM_FILES, "SHA256SUMS inventory changed")
    for name, digest in entries.items():
        require(sha256_file(BUNDLE / name) == digest, f"SHA256SUMS mismatch for {name}")


def validate_checksums() -> None:
    validate_checksum_text(CHECKSUMS_PATH.read_text(encoding="utf-8"))


def load_and_validate(check_checksums: bool = True) -> tuple[dict[str, object], dict[str, object], dict[str, object], list[dict[str, object]], list[dict[str, object]]]:
    cases_payload, cases_value = read_json(CASES_PATH)
    oracle_payload, oracle_value = read_json(ORACLE_PATH)
    _, manifest_value = read_json(MANIFEST_PATH)
    cases = validate_cases(cases_value)
    outcomes = validate_oracle(oracle_value, cases)
    validate_manifest(manifest_value, cases_payload, oracle_payload, cases_value, outcomes)
    if check_checksums:
        validate_checksums()
    return cases_value, oracle_value, manifest_value, cases, outcomes


def verify() -> None:
    cases_value, _, _, _, outcomes = load_and_validate()
    summary = summarize(cases_value, outcomes)
    print(
        "candidate benchmark verified: "
        f"{summary['cases']} cases, {summary['outcomes']['selected']} selected, "
        f"{summary['outcomes']['inconclusive']} inconclusive, {summary['outcomes']['retry']} retry"
    )


def expect_rejected(name: str, operation: Callable[[], None]) -> None:
    try:
        operation()
    except (BenchmarkError, json.JSONDecodeError):
        return
    raise BenchmarkError(f"self-test mutation was accepted: {name}")


def semantic_check(cases_value: dict[str, object], oracle_value: dict[str, object]) -> None:
    cases = validate_cases(cases_value)
    validate_oracle(oracle_value, cases)


def self_test() -> None:
    cases_value, oracle_value, manifest_value, _, _ = load_and_validate()
    rejected = 0

    def mutation(name: str, mutate: Callable[[dict[str, object], dict[str, object]], None]) -> None:
        nonlocal rejected
        cases_copy = copy.deepcopy(cases_value)
        oracle_copy = copy.deepcopy(oracle_value)
        mutate(cases_copy, oracle_copy)
        expect_rejected(name, lambda: semantic_check(cases_copy, oracle_copy))
        rejected += 1

    mutation("forged selected SHA", lambda _cases, oracle: oracle["outcomes"][8].__setitem__("sha", "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"))
    mutation("removed API-gap coverage", lambda cases, _oracle: cases["cases"][9].__setitem__("coverage", []))
    mutation("gap rewritten complete", lambda cases, _oracle: cases["cases"][9]["observation"]["signals"][0]["check_runs"].__setitem__("collection", "complete"))
    mutation("queue drift hidden", lambda cases, _oracle: cases["cases"][7]["observation"].__setitem__("after", copy.deepcopy(cases["cases"][7]["observation"]["before"])))
    mutation("ClickHouse entry removed", lambda cases, _oracle: cases["provenance"][1]["values"]["entries"].pop())
    mutation("docs capture changed", lambda cases, _oracle: cases["provenance"][0]["values"].__setitem__("head_sha", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"))
    mutation("queue entry fabricated", lambda cases, _oracle: cases["cases"][6]["observation"]["before"].__setitem__("is_in_merge_queue", False))
    mutation("invalid object ID", lambda cases, _oracle: cases["cases"][0]["observation"]["before"].__setitem__("head_sha", "short"))
    mutation("duplicate case ID", lambda cases, _oracle: cases["cases"][1].__setitem__("id", cases["cases"][0]["id"]))
    mutation("signal reported through gap", lambda cases, _oracle: cases["cases"][9]["observation"]["signals"][0]["check_runs"].__setitem__("success", 1))
    mutation("queue candidate replaced by PR head", lambda _cases, oracle: oracle["outcomes"][5].__setitem__("sha", "5d5f1bb2928c76f103c6e4d60f3c28cd22daef3d"))
    mutation("old green omitted", lambda cases, _oracle: cases["cases"][8]["observation"].__setitem__("signals", [cases["cases"][8]["observation"]["signals"][1]]))

    weakened_manifest = copy.deepcopy(manifest_value)
    weakened_manifest["claim_boundary"]["production_accuracy_supported"] = True
    expect_rejected(
        "weakened claim boundary",
        lambda: validate_manifest(
            weakened_manifest,
            CASES_PATH.read_bytes(),
            ORACLE_PATH.read_bytes(),
            cases_value,
            validate_oracle(oracle_value, validate_cases(cases_value)),
        ),
    )
    rejected += 1

    stale_manifest = copy.deepcopy(manifest_value)
    stale_manifest["assets"]["cases"]["sha256"] = "0" * 64
    expect_rejected(
        "stale asset hash",
        lambda: validate_manifest(
            stale_manifest,
            CASES_PATH.read_bytes(),
            ORACLE_PATH.read_bytes(),
            cases_value,
            validate_oracle(oracle_value, validate_cases(cases_value)),
        ),
    )
    rejected += 1

    expect_rejected(
        "duplicate JSON key",
        lambda: json.loads('{"schema":"a","schema":"b"}', object_pairs_hook=unique_json_object),
    )
    rejected += 1

    checksum_text = CHECKSUMS_PATH.read_text(encoding="utf-8")
    forged_checksum = ("0" if checksum_text[0] != "0" else "1") + checksum_text[1:]
    expect_rejected("checksum substitution", lambda: validate_checksum_text(forged_checksum))
    rejected += 1

    print(f"candidate benchmark self-test passed: rejected {rejected} mutations")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("derive-oracle", "self-test", "summary", "verify"))
    args = parser.parse_args()
    if args.command == "verify":
        verify()
    elif args.command == "self-test":
        self_test()
    else:
        cases_value, _, _, cases, outcomes = load_and_validate()
        if args.command == "derive-oracle":
            print(json.dumps(derive_oracle(cases), indent=2, ensure_ascii=False))
        else:
            print(json.dumps(summarize(cases_value, outcomes), indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
