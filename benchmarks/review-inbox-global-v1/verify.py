#!/usr/bin/env python3
"""Verify the deterministic Review Inbox Global v1 semantic corpus."""

from __future__ import annotations

import argparse
import copy
from datetime import datetime
import hashlib
import json
from pathlib import Path
import re
from typing import Callable


BUNDLE = Path(__file__).resolve().parent
DEFAULT_CASES = BUNDLE / "cases.json"
DEFAULT_MANIFEST = BUNDLE / "manifest.json"
DEFAULT_ORACLE = BUNDLE / "oracle.json"

CASES_SCHEMA = "stratadiff-review-inbox-global-cases-v1"
MANIFEST_SCHEMA = "stratadiff-review-inbox-global-manifest-v1"
ORACLE_SCHEMA = "stratadiff-review-inbox-global-oracle-v1"
DATASET_VERSION = "1.0.0"
MAX_CANDIDATES = 100
MAX_REVIEWS = 10_000

FORMAL_STATES = {"APPROVED", "CHANGES_REQUESTED"}
REVIEW_STATES = FORMAL_STATES | {"COMMENTED", "DISMISSED", "PENDING"}
SEARCH_OUTCOMES = {"ok", "forbidden", "repository_not_found"}
REVALIDATION_OUTCOMES = {"matched", "forbidden", "not_found"}
ID_PATTERN = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
TAG_PATTERN = re.compile(r"^[a-z][a-z0-9_]*$")
HOST_PATTERN = re.compile(
    r"^[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?$"
)
LOGIN_PATTERN = re.compile(
    r"^[A-Za-z0-9](?:[A-Za-z0-9_.-]{0,253}[A-Za-z0-9])?$"
)
NODE_PATTERN = re.compile(r"^[A-Za-z0-9_:+/=-]{1,256}$")
REPOSITORY_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
OID_PATTERN = re.compile(r"^[0-9a-f]{40}$")
TIMESTAMP_PATTERN = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$"
)
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
FORBIDDEN_KEYS = {
    "authorization",
    "body",
    "commit_message",
    "content",
    "diff",
    "email",
    "files",
    "message",
    "patch",
    "source",
    "text",
    "title",
    "token",
}


class BenchmarkError(RuntimeError):
    """A benchmark asset violates the frozen bundle contract."""


class ScenarioError(RuntimeError):
    """A materialized observation must fail closed with a stable reason code."""

    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


def require(condition: bool, message: str) -> None:
    if not condition:
        raise BenchmarkError(message)


def reject(code: str) -> None:
    raise ScenarioError(code)


def scenario_require(condition: bool, code: str) -> None:
    if not condition:
        reject(code)


def require_exact_keys(value: dict[str, object], keys: set[str], label: str) -> None:
    require(set(value) == keys, f"{label} fields differ: {sorted(set(value) ^ keys)}")


def scenario_exact_keys(value: object, keys: set[str], code: str) -> dict[str, object]:
    scenario_require(isinstance(value, dict), code)
    assert isinstance(value, dict)
    scenario_require(set(value) == keys, code)
    return value


def require_object(value: object, label: str) -> dict[str, object]:
    require(isinstance(value, dict), f"{label} must be an object")
    return value


def require_array(value: object, label: str) -> list[object]:
    require(isinstance(value, list), f"{label} must be an array")
    return value


def require_string(value: object, label: str) -> str:
    require(isinstance(value, str), f"{label} must be a string")
    return value


def require_int(value: object, label: str, minimum: int = 0) -> int:
    require(type(value) is int and value >= minimum, f"{label} must be an integer >= {minimum}")
    return value


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
    require(isinstance(value, dict), f"{path} must contain a JSON object")
    require(payload == canonical_json(value), f"{path} is not canonical JSON")
    return payload, value


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def reject_forbidden_keys(value: object, path: str) -> None:
    if isinstance(value, dict):
        for key, item in value.items():
            require(key.casefold() not in FORBIDDEN_KEYS, f"forbidden field at {path}.{key}")
            reject_forbidden_keys(item, f"{path}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            reject_forbidden_keys(item, f"{path}[{index}]")


def decode_pointer(pointer: object, label: str) -> list[str]:
    text = require_string(pointer, label)
    require(text.startswith("/") and text != "/", f"{label} must be a non-root JSON pointer")
    parts = text[1:].split("/")
    require(
        all(part and "~" not in part for part in parts),
        f"{label} uses unsupported JSON Pointer escaping",
    )
    return parts


def pointer_value(root: object, pointer: object, label: str) -> object:
    value = root
    for part in decode_pointer(pointer, label):
        if isinstance(value, dict):
            require(part in value, f"{label} does not resolve: {pointer}")
            value = value[part]
        elif isinstance(value, list):
            require(part.isascii() and part.isdecimal(), f"{label} list index is invalid")
            index = int(part)
            require(index < len(value), f"{label} list index is out of range")
            value = value[index]
        else:
            raise BenchmarkError(f"{label} traverses a scalar: {pointer}")
    return value


def pointer_parent(root: object, pointer: object, label: str) -> tuple[object, str]:
    parts = decode_pointer(pointer, label)
    parent: object = root
    for part in parts[:-1]:
        if isinstance(parent, dict):
            require(part in parent, f"{label} does not resolve: {pointer}")
            parent = parent[part]
        elif isinstance(parent, list):
            require(part.isascii() and part.isdecimal(), f"{label} list index is invalid")
            index = int(part)
            require(index < len(parent), f"{label} list index is out of range")
            parent = parent[index]
        else:
            raise BenchmarkError(f"{label} traverses a scalar: {pointer}")
    return parent, parts[-1]


def apply_patch(base: dict[str, object], operations: list[object], label: str) -> dict[str, object]:
    observation = copy.deepcopy(base)
    for index, item in enumerate(operations):
        operation = require_object(item, f"{label}[{index}]")
        op = require_string(operation["op"], f"{label}[{index}].op")
        if op in {"replace", "append"}:
            require_exact_keys(operation, {"op", "path", "value"}, f"{label}[{index}]")
        elif op == "append_copy":
            require_exact_keys(operation, {"from", "op", "path"}, f"{label}[{index}]")
        else:
            raise BenchmarkError(f"{label}[{index}] has unsupported operation {op!r}")

        if op == "replace":
            parent, key = pointer_parent(observation, operation["path"], f"{label}[{index}].path")
            if isinstance(parent, dict):
                require(key in parent, f"{label}[{index}] replaces an absent field")
                parent[key] = copy.deepcopy(operation["value"])
            elif isinstance(parent, list):
                require(key.isascii() and key.isdecimal(), f"{label}[{index}] index is invalid")
                position = int(key)
                require(position < len(parent), f"{label}[{index}] index is out of range")
                parent[position] = copy.deepcopy(operation["value"])
            else:
                raise BenchmarkError(f"{label}[{index}] replace parent is a scalar")
        elif op == "append":
            target = pointer_value(observation, operation["path"], f"{label}[{index}].path")
            require(isinstance(target, list), f"{label}[{index}] append target must be an array")
            target.append(copy.deepcopy(operation["value"]))
        else:
            target = pointer_value(observation, operation["path"], f"{label}[{index}].path")
            require(isinstance(target, list), f"{label}[{index}] append target must be an array")
            copied = pointer_value(observation, operation["from"], f"{label}[{index}].from")
            target.append(copy.deepcopy(copied))
    return observation


def validate_cases_asset(value: dict[str, object]) -> list[dict[str, object]]:
    require_exact_keys(
        value,
        {"base_observation", "cases", "dataset_version", "schema"},
        "cases asset",
    )
    require(value["schema"] == CASES_SCHEMA, "unsupported cases schema")
    require(value["dataset_version"] == DATASET_VERSION, "cases dataset version differs")
    base = require_object(value["base_observation"], "base_observation")
    cases = require_array(value["cases"], "cases")
    require(len(cases) >= 30, "global corpus must contain at least 30 cases")
    reject_forbidden_keys(base, "base_observation")

    materialized: list[dict[str, object]] = []
    case_ids: set[str] = set()
    for index, item in enumerate(cases):
        case = require_object(item, f"cases[{index}]")
        require_exact_keys(
            case,
            {"covers", "description", "id", "patch", "transition_relation"},
            f"cases[{index}]",
        )
        case_id = require_string(case["id"], f"cases[{index}].id")
        require(ID_PATTERN.fullmatch(case_id) is not None, f"invalid case ID {case_id!r}")
        require(case_id not in case_ids, f"duplicate case ID {case_id}")
        case_ids.add(case_id)
        description = require_string(case["description"], f"{case_id}.description")
        require(20 <= len(description) <= 240, f"{case_id} description length is invalid")
        covers = require_array(case["covers"], f"{case_id}.covers")
        require(covers, f"{case_id} must declare coverage")
        require(
            all(isinstance(tag, str) and TAG_PATTERN.fullmatch(tag) for tag in covers),
            f"{case_id} has an invalid coverage tag",
        )
        require(len(set(covers)) == len(covers), f"{case_id} has duplicate coverage tags")
        patch = require_array(case["patch"], f"{case_id}.patch")
        for patch_index, operation_value in enumerate(patch):
            operation = require_object(
                operation_value, f"{case_id}.patch[{patch_index}]"
            )
            if "value" in operation:
                reject_forbidden_keys(
                    operation["value"], f"{case_id}.patch[{patch_index}].value"
                )
        observation = apply_patch(base, patch, f"{case_id}.patch")
        relation = case["transition_relation"]
        if relation is not None:
            relation_object = require_object(relation, f"{case_id}.transition_relation")
            require_exact_keys(
                relation_object,
                {"kind", "reference"},
                f"{case_id}.transition_relation",
            )
            require(
                relation_object["kind"] in {"same", "different"},
                f"{case_id} relation kind is invalid",
            )
            reference = require_string(
                relation_object["reference"], f"{case_id}.transition_relation.reference"
            )
            require(reference != case_id, f"{case_id} cannot reference itself")
        materialized.append(
            {
                "covers": copy.deepcopy(covers),
                "description": description,
                "id": case_id,
                "observation": observation,
                "transition_relation": copy.deepcopy(relation),
            }
        )

    for case in materialized:
        relation = case["transition_relation"]
        if relation is not None:
            require(relation["reference"] in case_ids, f"{case['id']} relation target is absent")
    return materialized


def parse_timestamp(value: object, code: str) -> datetime:
    scenario_require(isinstance(value, str), code)
    assert isinstance(value, str)
    scenario_require(TIMESTAMP_PATTERN.fullmatch(value) is not None, code)
    return datetime.fromisoformat(value[:-1] + "+00:00")


def validate_login(value: object, code: str) -> str:
    scenario_require(isinstance(value, str), code)
    assert isinstance(value, str)
    scenario_require(LOGIN_PATTERN.fullmatch(value) is not None, code)
    return value


def validate_node(value: object, code: str) -> str:
    scenario_require(isinstance(value, str), code)
    assert isinstance(value, str)
    scenario_require(NODE_PATTERN.fullmatch(value) is not None, code)
    return value


def validate_oid(value: object, code: str, *, nullable: bool) -> str | None:
    if value is None:
        scenario_require(nullable, code)
        return None
    scenario_require(isinstance(value, str), code)
    assert isinstance(value, str)
    scenario_require(OID_PATTERN.fullmatch(value) is not None, code)
    return value


def validate_actor(value: object, code: str) -> dict[str, object]:
    actor = scenario_exact_keys(value, {"login", "node_id"}, code)
    validate_login(actor["login"], code)
    validate_node(actor["node_id"], code)
    return actor


def validate_reviewer(value: object, code: str) -> dict[str, object]:
    reviewer = scenario_exact_keys(value, {"actor_type", "login", "node_id"}, code)
    scenario_require(reviewer["actor_type"] == "User", code)
    validate_login(reviewer["login"], code)
    validate_node(reviewer["node_id"], code)
    return reviewer


def same_actor(observed: dict[str, object], expected: dict[str, object]) -> bool:
    return (
        observed["node_id"] == expected["node_id"]
        and isinstance(observed["login"], str)
        and isinstance(expected["login"], str)
        and observed["login"].casefold() == expected["login"].casefold()
    )


def validate_repository(
    value: object, host: str, code: str
) -> dict[str, object]:
    repository = scenario_exact_keys(
        value, {"name_with_owner", "node_id", "url"}, code
    )
    name = repository["name_with_owner"]
    scenario_require(isinstance(name, str), code)
    assert isinstance(name, str)
    scenario_require(REPOSITORY_PATTERN.fullmatch(name) is not None, code)
    validate_node(repository["node_id"], code)
    scenario_require(repository["url"] == f"https://{host}/{name}", code)
    return repository


def transition_key(
    host: str,
    repository: dict[str, object],
    pull_request: dict[str, object],
    reviewer: dict[str, object],
    checkpoint: dict[str, object],
    review_request_active: bool,
) -> str:
    fields = [
        "stratadiff-review-inbox-global-transition-v1",
        host,
        repository["node_id"],
        pull_request["node_id"],
        reviewer["node_id"],
        checkpoint["node_id"],
        checkpoint["commit_oid"],
        pull_request["head_oid"],
        checkpoint["checkpoint_base_oid"] or "<unavailable>",
        pull_request["current_base_oid"] or "<unavailable>",
        "review-requested" if review_request_active else "not-requested",
    ]
    digest = hashlib.sha256()
    for field in fields:
        scenario_require(isinstance(field, str), "transition_identity_invalid")
        digest.update(field.encode("utf-8"))
        digest.update(b"\0")
    return digest.hexdigest()


def derive_candidate(
    candidate_value: object,
    host: str,
    authenticated_actor: dict[str, object],
    reviewer: dict[str, object],
    requested_repository: dict[str, object] | None,
) -> dict[str, object]:
    candidate = scenario_exact_keys(
        candidate_value,
        {
            "pull_request",
            "repository",
            "review_history",
            "review_request_active",
            "revalidation",
        },
        "candidate_shape_invalid",
    )
    repository = validate_repository(candidate["repository"], host, "provider_url_mismatch")
    if requested_repository is not None:
        scenario_require(repository == requested_repository, "repository_identity_changed")

    pull_request = scenario_exact_keys(
        candidate["pull_request"],
        {
            "current_base_oid",
            "head_oid",
            "node_id",
            "number",
            "state",
            "total_review_count",
            "updated_at",
            "url",
        },
        "pull_request_shape_invalid",
    )
    validate_node(pull_request["node_id"], "pull_request_identity_invalid")
    scenario_require(type(pull_request["number"]) is int and pull_request["number"] > 0, "pull_request_identity_invalid")
    scenario_require(pull_request["state"] == "OPEN", "pull_request_not_open")
    scenario_require(
        pull_request["url"]
        == f"{repository['url']}/pull/{pull_request['number']}",
        "provider_url_mismatch",
    )
    parse_timestamp(pull_request["updated_at"], "pull_request_timestamp_invalid")
    validate_oid(pull_request["head_oid"], "head_oid_invalid", nullable=True)
    validate_oid(
        pull_request["current_base_oid"], "current_base_oid_invalid", nullable=True
    )
    scenario_require(
        type(pull_request["total_review_count"]) is int
        and pull_request["total_review_count"] >= 0,
        "review_count_invalid",
    )
    scenario_require(type(candidate["review_request_active"]) is bool, "review_request_invalid")

    revalidation = scenario_exact_keys(
        candidate["revalidation"],
        {
            "outcome",
            "pull_request_node_id",
            "repository",
            "snapshot_matches",
            "viewer",
        },
        "revalidation_shape_invalid",
    )
    scenario_require(revalidation["outcome"] in REVALIDATION_OUTCOMES, "revalidation_outcome_invalid")
    if revalidation["outcome"] == "forbidden":
        reject("candidate_forbidden")
    if revalidation["outcome"] == "not_found":
        reject("candidate_not_found")
    revalidation_viewer = validate_actor(
        revalidation["viewer"], "authenticated_actor_invalid"
    )
    scenario_require(
        same_actor(revalidation_viewer, authenticated_actor),
        "authenticated_actor_changed",
    )
    revalidated_repository = validate_repository(
        revalidation["repository"], host, "provider_url_mismatch"
    )
    scenario_require(
        revalidated_repository == repository, "repository_identity_changed"
    )
    scenario_require(
        revalidation["pull_request_node_id"] == pull_request["node_id"],
        "pull_request_identity_changed",
    )
    scenario_require(
        revalidation["snapshot_matches"] is True, "candidate_changed_during_revalidation"
    )

    history = scenario_exact_keys(
        candidate["review_history"],
        {
            "cursor_advanced",
            "nodes",
            "pages_observed",
            "reported_count",
            "terminal_page_observed",
        },
        "review_history_shape_invalid",
    )
    nodes = history["nodes"]
    scenario_require(isinstance(nodes, list), "review_history_shape_invalid")
    assert isinstance(nodes, list)
    scenario_require(
        type(history["reported_count"]) is int and history["reported_count"] >= 0,
        "review_count_invalid",
    )
    scenario_require(
        type(history["pages_observed"]) is int and history["pages_observed"] >= 1,
        "review_pagination_invalid",
    )
    scenario_require(type(history["cursor_advanced"]) is bool, "review_pagination_invalid")
    scenario_require(
        type(history["terminal_page_observed"]) is bool,
        "review_pagination_invalid",
    )
    scenario_require(
        history["terminal_page_observed"], "incomplete_review_pagination"
    )
    if history["pages_observed"] > 1:
        scenario_require(history["cursor_advanced"], "review_pagination_cursor_stalled")
    scenario_require(
        history["reported_count"] == len(nodes), "review_history_count_mismatch"
    )
    scenario_require(
        history["reported_count"] <= MAX_REVIEWS, "reviewer_history_limit_exceeded"
    )
    scenario_require(
        pull_request["total_review_count"] >= history["reported_count"],
        "review_count_invalid",
    )

    review_node_ids: set[str] = set()
    database_ids: set[int] = set()
    eligible: list[tuple[datetime, int, dict[str, object]]] = []
    for review_value in nodes:
        review = scenario_exact_keys(
            review_value,
            {
                "author",
                "checkpoint_base_oid",
                "commit_oid",
                "database_id",
                "node_id",
                "state",
                "submitted_at",
                "url",
            },
            "review_shape_invalid",
        )
        node_id = validate_node(review["node_id"], "review_node_invalid")
        scenario_require(node_id not in review_node_ids, "duplicate_review_node")
        review_node_ids.add(node_id)
        author = validate_reviewer(review["author"], "reviewer_identity_mismatch")
        scenario_require(same_actor(author, reviewer), "reviewer_identity_mismatch")
        scenario_require(review["state"] in REVIEW_STATES, "review_state_invalid")
        database_id = review["database_id"]
        if database_id is not None:
            scenario_require(type(database_id) is int and database_id > 0, "review_database_id_invalid")
            assert isinstance(database_id, int)
            scenario_require(
                database_id not in database_ids, "duplicate_review_database_id"
            )
            database_ids.add(database_id)
            scenario_require(
                review["url"]
                == f"{pull_request['url']}#pullrequestreview-{database_id}",
                "review_url_invalid",
            )
        else:
            scenario_require(isinstance(review["url"], str), "review_url_invalid")
        validate_oid(review["commit_oid"], "review_commit_invalid", nullable=True)
        validate_oid(
            review["checkpoint_base_oid"], "checkpoint_base_oid_invalid", nullable=True
        )
        if review["submitted_at"] is not None:
            submitted_at = parse_timestamp(
                review["submitted_at"], "review_timestamp_invalid"
            )
        else:
            submitted_at = None

        if review["state"] in FORMAL_STATES:
            scenario_require(database_id is not None, "completed_review_database_id_missing")
            scenario_require(submitted_at is not None, "completed_review_timestamp_missing")
            scenario_require(review["commit_oid"] is not None, "review_commit_invalid")
            assert database_id is not None and submitted_at is not None
            eligible.append((submitted_at, database_id, review))

    if not eligible:
        return {
            "category": "no_eligible_reviews",
            "checkpoint_review_node_id": None,
            "reason": None,
            "transition_key": None,
            "trigger": None,
        }

    _, _, checkpoint = max(eligible, key=lambda item: (item[0], item[1]))
    checkpoint_node_id = checkpoint["node_id"]
    if pull_request["head_oid"] is None:
        return {
            "category": "unobservable",
            "checkpoint_review_node_id": checkpoint_node_id,
            "reason": "head_oid_unavailable",
            "transition_key": None,
            "trigger": None,
        }
    if pull_request["total_review_count"] > MAX_REVIEWS:
        return {
            "category": "unobservable",
            "checkpoint_review_node_id": checkpoint_node_id,
            "reason": "resume_review_limit_exceeded",
            "transition_key": None,
            "trigger": None,
        }

    head_changed = checkpoint["commit_oid"] != pull_request["head_oid"]
    if not head_changed and checkpoint["checkpoint_base_oid"] is None:
        return {
            "category": "unobservable",
            "checkpoint_review_node_id": checkpoint_node_id,
            "reason": "checkpoint_base_oid_unavailable",
            "transition_key": None,
            "trigger": None,
        }
    if not head_changed and pull_request["current_base_oid"] is None:
        return {
            "category": "unobservable",
            "checkpoint_review_node_id": checkpoint_node_id,
            "reason": "current_base_oid_unavailable",
            "transition_key": None,
            "trigger": None,
        }
    base_changed = (
        checkpoint["checkpoint_base_oid"] is not None
        and pull_request["current_base_oid"] is not None
        and checkpoint["checkpoint_base_oid"] != pull_request["current_base_oid"]
    )
    triggers: list[str] = []
    if head_changed:
        triggers.append("head_changed")
    if base_changed:
        triggers.append("base_drift")
    if candidate["review_request_active"]:
        triggers.append("review_re_requested")
    if not triggers:
        return {
            "category": "up_to_date",
            "checkpoint_review_node_id": checkpoint_node_id,
            "reason": None,
            "transition_key": None,
            "trigger": None,
        }
    return {
        "category": "actionable",
        "checkpoint_review_node_id": checkpoint_node_id,
        "reason": None,
        "transition_key": transition_key(
            host,
            repository,
            pull_request,
            reviewer,
            checkpoint,
            candidate["review_request_active"],
        ),
        "trigger": "+".join(triggers),
    }


def error_result(code: str) -> dict[str, object]:
    return {
        "checkpoint_review_node_id": None,
        "counts": {
            "actionable": 0,
            "no_eligible_reviews": 0,
            "unobservable": 0,
            "up_to_date": 0,
        },
        "error": code,
        "reason": None,
        "result": "error",
        "status": None,
        "transition_key": None,
        "trigger": None,
    }


def derive_observation(value: object) -> dict[str, object]:
    try:
        observation = scenario_exact_keys(
            value, {"provider", "scope", "search"}, "observation_shape_invalid"
        )
        provider = scenario_exact_keys(
            observation["provider"], {"host"}, "provider_identity_invalid"
        )
        host = provider["host"]
        scenario_require(isinstance(host, str), "provider_identity_invalid")
        assert isinstance(host, str)
        scenario_require(
            HOST_PATTERN.fullmatch(host) is not None and host == host.lower(),
            "provider_identity_invalid",
        )
        scope = scenario_exact_keys(
            observation["scope"],
            {"authenticated_actor", "requested_repository", "reviewer"},
            "scope_shape_invalid",
        )
        authenticated_actor = validate_actor(
            scope["authenticated_actor"], "authenticated_actor_invalid"
        )
        reviewer = validate_reviewer(scope["reviewer"], "reviewer_identity_invalid")
        requested_repository_value = scope["requested_repository"]
        if requested_repository_value is None:
            requested_repository = None
        else:
            requested_repository = validate_repository(
                requested_repository_value, host, "provider_url_mismatch"
            )

        search = scenario_exact_keys(
            observation["search"],
            {"candidates", "has_next_page", "issue_count", "limit", "outcome", "viewer"},
            "search_shape_invalid",
        )
        scenario_require(search["outcome"] in SEARCH_OUTCOMES, "search_outcome_invalid")
        if search["outcome"] == "forbidden":
            reject("search_forbidden")
        if search["outcome"] == "repository_not_found":
            reject("repository_not_found")
        viewer = validate_actor(search["viewer"], "authenticated_actor_invalid")
        scenario_require(
            same_actor(viewer, authenticated_actor), "authenticated_actor_changed"
        )
        candidates = search["candidates"]
        scenario_require(isinstance(candidates, list), "search_shape_invalid")
        assert isinstance(candidates, list)
        scenario_require(
            type(search["limit"]) is int and 1 <= search["limit"] <= MAX_CANDIDATES,
            "search_limit_invalid",
        )
        scenario_require(len(candidates) <= search["limit"], "search_limit_exceeded")
        scenario_require(
            type(search["issue_count"]) is int
            and search["issue_count"] >= len(candidates),
            "search_count_invalid",
        )
        scenario_require(type(search["has_next_page"]) is bool, "search_pagination_invalid")
        scenario_require(
            search["has_next_page"] == (search["issue_count"] > len(candidates)),
            "search_pagination_inconsistent",
        )

        candidate_results: list[dict[str, object]] = []
        pull_request_ids: set[str] = set()
        for candidate_value in candidates:
            candidate_result = derive_candidate(
                candidate_value,
                host,
                authenticated_actor,
                reviewer,
                requested_repository,
            )
            candidate = candidate_value
            assert isinstance(candidate, dict)
            pull_request = candidate["pull_request"]
            assert isinstance(pull_request, dict)
            pull_request_id = pull_request["node_id"]
            assert isinstance(pull_request_id, str)
            scenario_require(
                pull_request_id not in pull_request_ids, "duplicate_pull_request_node"
            )
            pull_request_ids.add(pull_request_id)
            candidate_results.append(candidate_result)

        counts = {
            "actionable": sum(
                result["category"] == "actionable" for result in candidate_results
            ),
            "no_eligible_reviews": sum(
                result["category"] == "no_eligible_reviews"
                for result in candidate_results
            ),
            "unobservable": sum(
                result["category"] == "unobservable" for result in candidate_results
            ),
            "up_to_date": sum(
                result["category"] == "up_to_date" for result in candidate_results
            ),
        }
        if search["has_next_page"]:
            status = "partial"
        elif counts["actionable"]:
            status = "actionable"
        elif counts["unobservable"]:
            status = "insufficient_evidence"
        elif counts["up_to_date"]:
            status = "up_to_date"
        else:
            status = "no_eligible_reviews"
        selected = [
            result["checkpoint_review_node_id"]
            for result in candidate_results
            if result["checkpoint_review_node_id"] is not None
        ]
        triggers = [
            result["trigger"] for result in candidate_results if result["trigger"] is not None
        ]
        reasons = [
            result["reason"] for result in candidate_results if result["reason"] is not None
        ]
        keys = [
            result["transition_key"]
            for result in candidate_results
            if result["transition_key"] is not None
        ]
        return {
            "checkpoint_review_node_id": selected[0] if len(selected) == 1 else None,
            "counts": counts,
            "error": None,
            "reason": reasons[0] if len(reasons) == 1 else None,
            "result": "success",
            "status": status,
            "transition_key": keys[0] if len(keys) == 1 else None,
            "trigger": triggers[0] if len(triggers) == 1 else None,
        }
    except ScenarioError as error:
        return error_result(error.code)


def public_result(result: dict[str, object]) -> dict[str, object]:
    value = copy.deepcopy(result)
    del value["transition_key"]
    return value


def derive_cases(
    materialized: list[dict[str, object]],
) -> tuple[dict[str, dict[str, object]], dict[str, str]]:
    results: dict[str, dict[str, object]] = {}
    transition_keys: dict[str, str] = {}
    for case in materialized:
        case_id = case["id"]
        assert isinstance(case_id, str)
        result = derive_observation(case["observation"])
        results[case_id] = public_result(result)
        if result["transition_key"] is not None:
            assert isinstance(result["transition_key"], str)
            transition_keys[case_id] = result["transition_key"]

    for case in materialized:
        relation = case["transition_relation"]
        if relation is None:
            continue
        case_id = case["id"]
        reference = relation["reference"]
        require(case_id in transition_keys, f"{case_id} relation has no actionable transition")
        require(reference in transition_keys, f"{case_id} relation target has no actionable transition")
        equal = transition_keys[case_id] == transition_keys[reference]
        require(
            equal == (relation["kind"] == "same"),
            f"{case_id} transition relation {relation['kind']} failed against {reference}",
        )
    return results, transition_keys


def validate_oracle(
    value: dict[str, object], derived: dict[str, dict[str, object]]
) -> None:
    require_exact_keys(value, {"cases", "dataset_version", "schema"}, "oracle")
    require(value["schema"] == ORACLE_SCHEMA, "unsupported oracle schema")
    require(value["dataset_version"] == DATASET_VERSION, "oracle dataset version differs")
    cases = require_object(value["cases"], "oracle.cases")
    require(set(cases) == set(derived), "oracle case membership differs")
    for case_id, expected in cases.items():
        expected_object = require_object(expected, f"oracle.cases.{case_id}")
        require_exact_keys(
            expected_object,
            {
                "checkpoint_review_node_id",
                "counts",
                "error",
                "reason",
                "result",
                "status",
                "trigger",
            },
            f"oracle.cases.{case_id}",
        )
        counts = require_object(expected_object["counts"], f"oracle.cases.{case_id}.counts")
        require_exact_keys(
            counts,
            {"actionable", "no_eligible_reviews", "unobservable", "up_to_date"},
            f"oracle.cases.{case_id}.counts",
        )
        require(expected_object == derived[case_id], f"oracle differs for {case_id}")


def summarize(
    materialized: list[dict[str, object]], derived: dict[str, dict[str, object]]
) -> dict[str, object]:
    coverage: set[str] = set()
    relations = 0
    for case in materialized:
        coverage.update(case["covers"])
        relations += case["transition_relation"] is not None
    return {
        "candidate_decisions": {
            category: sum(
                result["counts"][category] for result in derived.values()
            )
            for category in [
                "actionable",
                "no_eligible_reviews",
                "unobservable",
                "up_to_date",
            ]
        },
        "cases": len(derived),
        "coverage_tags": len(coverage),
        "error_cases": sum(result["result"] == "error" for result in derived.values()),
        "relation_assertions": relations,
        "status": {
            status: sum(result["status"] == status for result in derived.values())
            for status in [
                "actionable",
                "insufficient_evidence",
                "no_eligible_reviews",
                "partial",
                "up_to_date",
            ]
        },
        "success_cases": sum(
            result["result"] == "success" for result in derived.values()
        ),
    }


def coverage_tags(materialized: list[dict[str, object]]) -> set[str]:
    tags: set[str] = set()
    for case in materialized:
        tags.update(case["covers"])
    return tags


def validate_manifest(
    value: dict[str, object],
    cases_bytes: bytes,
    oracle_bytes: bytes,
    materialized: list[dict[str, object]],
    derived: dict[str, dict[str, object]],
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
    require(value["dataset_version"] == DATASET_VERSION, "manifest dataset version differs")
    require(value["designation"] == "controlled_semantic_regression", "manifest designation differs")
    require(value["dataset_license"] == "MIT", "manifest dataset license differs")
    require_string(value["name"], "manifest.name")
    require_string(value["description"], "manifest.description")

    assets = require_object(value["assets"], "manifest.assets")
    require_exact_keys(assets, {"cases", "oracle"}, "manifest.assets")
    expected_bindings = {
        "cases": {"path": "cases.json", "sha256": sha256_bytes(cases_bytes)},
        "oracle": {"path": "oracle.json", "sha256": sha256_bytes(oracle_bytes)},
    }
    require(assets == expected_bindings, "manifest asset bindings differ")

    required_coverage = require_array(value["required_coverage"], "manifest.required_coverage")
    require(
        all(isinstance(tag, str) and TAG_PATTERN.fullmatch(tag) for tag in required_coverage),
        "manifest required coverage contains an invalid tag",
    )
    require(
        len(set(required_coverage)) == len(required_coverage),
        "manifest required coverage contains duplicates",
    )
    observed_tags = coverage_tags(materialized)
    require(
        set(required_coverage) <= observed_tags,
        f"missing required coverage: {sorted(set(required_coverage) - observed_tags)}",
    )

    gates = require_object(value["acceptance_gates"], "manifest.acceptance_gates")
    require_exact_keys(
        gates,
        {
            "minimum_actionable_cases",
            "minimum_cases",
            "minimum_error_cases",
            "minimum_partial_cases",
            "minimum_relation_assertions",
            "minimum_unobservable_cases",
        },
        "manifest.acceptance_gates",
    )
    for name, gate in gates.items():
        require_int(gate, f"manifest.acceptance_gates.{name}")
    summary = summarize(materialized, derived)
    require(summary == value["expected_summary"], "manifest expected summary differs")
    require(summary["cases"] >= gates["minimum_cases"], "minimum case gate failed")
    require(
        summary["error_cases"] >= gates["minimum_error_cases"],
        "minimum error-case gate failed",
    )
    require(
        summary["relation_assertions"] >= gates["minimum_relation_assertions"],
        "minimum relation gate failed",
    )
    require(
        summary["status"]["actionable"] >= gates["minimum_actionable_cases"],
        "minimum actionable gate failed",
    )
    require(
        summary["status"]["partial"] >= gates["minimum_partial_cases"],
        "minimum partial gate failed",
    )
    require(
        summary["status"]["insufficient_evidence"]
        >= gates["minimum_unobservable_cases"],
        "minimum unobservable gate failed",
    )
    claim_boundary = require_array(value["claim_boundary"], "manifest.claim_boundary")
    require(len(claim_boundary) >= 4, "manifest claim boundary is incomplete")
    require(
        all(isinstance(item, str) and len(item) >= 20 for item in claim_boundary),
        "manifest claim boundary entries are invalid",
    )
    reject_forbidden_keys(value, "manifest")


def validate_bundle_data(
    cases_bytes: bytes,
    cases_asset: dict[str, object],
    oracle_bytes: bytes,
    oracle: dict[str, object],
    manifest: dict[str, object],
) -> tuple[list[dict[str, object]], dict[str, dict[str, object]], dict[str, object]]:
    materialized = validate_cases_asset(cases_asset)
    derived, _ = derive_cases(materialized)
    validate_oracle(oracle, derived)
    validate_manifest(
        manifest, cases_bytes, oracle_bytes, materialized, derived
    )
    return materialized, derived, summarize(materialized, derived)


def load_bundle(
    cases_path: Path, oracle_path: Path, manifest_path: Path
) -> tuple[
    bytes,
    dict[str, object],
    bytes,
    dict[str, object],
    dict[str, object],
]:
    cases_bytes, cases_asset = read_canonical_json(cases_path)
    oracle_bytes, oracle = read_canonical_json(oracle_path)
    _, manifest = read_canonical_json(manifest_path)
    return cases_bytes, cases_asset, oracle_bytes, oracle, manifest


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


def command_verify(arguments: argparse.Namespace) -> None:
    cases_bytes, cases_asset, oracle_bytes, oracle, manifest = load_bundle(
        arguments.cases, arguments.oracle, arguments.manifest
    )
    _, _, summary = validate_bundle_data(
        cases_bytes, cases_asset, oracle_bytes, oracle, manifest
    )
    print(
        "verified Review Inbox Global v1: "
        f"{summary['cases']} cases, "
        f"{summary['status']['actionable']} actionable, "
        f"{summary['status']['partial']} partial, "
        f"{summary['error_cases']} fail-closed errors"
    )


def command_derive_oracle(arguments: argparse.Namespace) -> None:
    _, cases_asset = read_canonical_json(arguments.cases)
    materialized = validate_cases_asset(cases_asset)
    derived, _ = derive_cases(materialized)
    value = {
        "cases": derived,
        "dataset_version": DATASET_VERSION,
        "schema": ORACLE_SCHEMA,
    }
    print(canonical_json(value).decode("utf-8"), end="")


def command_summary(arguments: argparse.Namespace) -> None:
    _, cases_asset = read_canonical_json(arguments.cases)
    materialized = validate_cases_asset(cases_asset)
    derived, _ = derive_cases(materialized)
    print(canonical_json(summarize(materialized, derived)).decode("utf-8"), end="")


def command_materialize(arguments: argparse.Namespace) -> None:
    _, cases_asset = read_canonical_json(arguments.cases)
    materialized = validate_cases_asset(cases_asset)
    value = {
        "cases": materialized,
        "dataset_version": DATASET_VERSION,
        "schema": "stratadiff-review-inbox-global-materialized-v1",
    }
    print(canonical_json(value).decode("utf-8"), end="")


def command_self_test(arguments: argparse.Namespace) -> None:
    cases_bytes, cases_asset, oracle_bytes, oracle, manifest = load_bundle(
        arguments.cases, arguments.oracle, arguments.manifest
    )
    validate_bundle_data(cases_bytes, cases_asset, oracle_bytes, oracle, manifest)

    wrong_oracle = copy.deepcopy(oracle)
    wrong_oracle["cases"]["approved-stale"]["status"] = "up_to_date"
    wrong_oracle_manifest = rebound_manifest(manifest, cases_asset, wrong_oracle)
    expect_failure(
        lambda: validate_bundle_data(
            cases_bytes,
            cases_asset,
            canonical_json(wrong_oracle),
            wrong_oracle,
            wrong_oracle_manifest,
        ),
        "forged oracle classification",
    )

    changed_case = copy.deepcopy(cases_asset)
    changed_case["cases"][0]["patch"].append(
        {
            "op": "replace",
            "path": "/search/candidates/0/pull_request/head_oid",
            "value": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        }
    )
    changed_manifest = rebound_manifest(manifest, changed_case, oracle)
    expect_failure(
        lambda: validate_bundle_data(
            canonical_json(changed_case),
            changed_case,
            oracle_bytes,
            oracle,
            changed_manifest,
        ),
        "observation changed under a rebound checksum",
    )

    omitted_case = copy.deepcopy(cases_asset)
    omitted_case["cases"].pop()
    omitted_manifest = rebound_manifest(manifest, omitted_case, oracle)
    expect_failure(
        lambda: validate_bundle_data(
            canonical_json(omitted_case),
            omitted_case,
            oracle_bytes,
            oracle,
            omitted_manifest,
        ),
        "omitted case",
    )

    leaked_payload = copy.deepcopy(cases_asset)
    leaked_payload["cases"][0]["patch"].append(
        {
            "op": "append",
            "path": "/search/candidates",
            "value": {"body": "must never be frozen"},
        }
    )
    leaked_manifest = rebound_manifest(manifest, leaked_payload, oracle)
    expect_failure(
        lambda: validate_bundle_data(
            canonical_json(leaked_payload),
            leaked_payload,
            oracle_bytes,
            oracle,
            leaked_manifest,
        ),
        "forbidden payload field",
    )

    stale_manifest = copy.deepcopy(manifest)
    stale_manifest["assets"]["cases"]["sha256"] = "0" * 64
    expect_failure(
        lambda: validate_bundle_data(
            cases_bytes, cases_asset, oracle_bytes, oracle, stale_manifest
        ),
        "stale asset checksum",
    )

    bad_relation = copy.deepcopy(cases_asset)
    for case in bad_relation["cases"]:
        if case["id"] == "identical-replay":
            case["transition_relation"]["kind"] = "different"
    bad_relation_manifest = rebound_manifest(manifest, bad_relation, oracle)
    expect_failure(
        lambda: validate_bundle_data(
            canonical_json(bad_relation),
            bad_relation,
            oracle_bytes,
            oracle,
            bad_relation_manifest,
        ),
        "forged transition relation",
    )

    unknown_path = copy.deepcopy(cases_asset)
    unknown_path["cases"][0]["patch"].append(
        {"op": "replace", "path": "/search/unknown", "value": 1}
    )
    unknown_path_manifest = rebound_manifest(manifest, unknown_path, oracle)
    expect_failure(
        lambda: validate_bundle_data(
            canonical_json(unknown_path),
            unknown_path,
            oracle_bytes,
            oracle,
            unknown_path_manifest,
        ),
        "unknown mutation path",
    )

    duplicate_key = b'{"schema":"one","schema":"two"}\n'
    expect_failure(
        lambda: json.loads(duplicate_key, object_pairs_hook=unique_json_object),
        "duplicate JSON key",
    )

    weak_manifest = copy.deepcopy(manifest)
    weak_manifest["required_coverage"].append("missing_required_semantic")
    expect_failure(
        lambda: validate_bundle_data(
            cases_bytes, cases_asset, oracle_bytes, oracle, weak_manifest
        ),
        "unmet required coverage",
    )

    print("self-test passed: 9 independent tamper and contract mutations rejected")


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
