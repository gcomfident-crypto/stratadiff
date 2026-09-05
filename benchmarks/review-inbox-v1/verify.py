#!/usr/bin/env python3
"""Verify the Review Inbox REST/GraphQL dual-observation seed."""

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
DEFAULT_PROTOCOL = BUNDLE / "protocol.json"
DEFAULT_ORACLE = BUNDLE / "oracle-v1.json"
DEFAULT_REST_OBSERVATION = BUNDLE / "rest-observation-v1.json"
DEFAULT_GRAPHQL_OBSERVATION = BUNDLE / "graphql-observation-v1.json"
PROTOCOL_SCHEMA = "stratadiff-review-inbox-protocol-v1"
ORACLE_SCHEMA = "stratadiff-review-inbox-dual-oracle-v1"
REST_SCHEMA = "stratadiff-review-inbox-rest-observation-v1"
GRAPHQL_SCHEMA = "stratadiff-review-inbox-graphql-observation-v1"
DATASET_VERSION = "1.1.0"
FORMAL_STATES = {"APPROVED", "CHANGES_REQUESTED"}
REVIEW_STATES = FORMAL_STATES | {"COMMENTED", "DISMISSED", "PENDING"}
OID_PATTERN = re.compile(r"^[0-9a-f]{40}$")
LOGIN_PATTERN = re.compile(r"^[A-Za-z0-9](?:[A-Za-z0-9_.-]{0,253}[A-Za-z0-9])?$")
NODE_ID_PATTERN = re.compile(r"^[A-Za-z0-9_:+/=-]{1,256}$")
REPOSITORY_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
TIMESTAMP_PATTERN = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$"
)
FORBIDDEN_KEYS = {
    "authorization",
    "body",
    "comments",
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


class InboxOracleError(RuntimeError):
    """A frozen Inbox evidence asset violates its declared contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise InboxOracleError(message)


def require_exact_keys(value: dict[str, object], keys: set[str], label: str) -> None:
    require(set(value) == keys, f"{label} fields differ: {sorted(set(value) ^ keys)}")


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


def parse_timestamp(value: object, label: str) -> datetime:
    text = require_string(value, label)
    require(TIMESTAMP_PATTERN.fullmatch(text) is not None, f"{label} must be RFC3339 UTC")
    return datetime.fromisoformat(text[:-1] + "+00:00")


def validate_oid(value: object, label: str, *, nullable: bool = False) -> str | None:
    if value is None:
        require(nullable, f"{label} must not be null")
        return None
    oid = require_string(value, label)
    require(OID_PATTERN.fullmatch(oid) is not None, f"{label} must be a lowercase SHA-1 OID")
    return oid


def validate_login(value: object, label: str) -> str:
    login = require_string(value, label)
    require(LOGIN_PATTERN.fullmatch(login) is not None, f"{label} is not a canonical login")
    return login


def validate_node_id(value: object, label: str) -> str:
    node_id = require_string(value, label)
    require(NODE_ID_PATTERN.fullmatch(node_id) is not None, f"{label} is not a node ID")
    return node_id


def validate_database_id(value: object, label: str) -> int:
    if type(value) is int:
        return require_int(value, label, 1)
    text = require_string(value, label)
    require(
        text.isascii() and text.isdecimal() and not text.startswith("0"),
        f"{label} must be positive canonical decimal",
    )
    return int(text)


def reject_forbidden_keys(value: object, path: str) -> None:
    if isinstance(value, dict):
        for key, item in value.items():
            require(key.casefold() not in FORBIDDEN_KEYS, f"forbidden field at {path}.{key}")
            reject_forbidden_keys(item, f"{path}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            reject_forbidden_keys(item, f"{path}[{index}]")


def validate_privacy(value: object, label: str) -> None:
    privacy = require_object(value, label)
    require_exact_keys(
        privacy,
        {
            "commit_messages_collected",
            "diffs_or_patches_collected",
            "pr_text_collected",
            "review_text_collected",
            "source_code_collected",
            "tokens_collected",
        },
        label,
    )
    require(all(item is False for item in privacy.values()), f"{label} flags must all be false")


def validate_capture_window(
    started_value: object, completed_value: object, label: str
) -> tuple[datetime, datetime]:
    started = parse_timestamp(started_value, f"{label}.started")
    completed = parse_timestamp(completed_value, f"{label}.completed")
    require(started <= completed, f"{label} completes before it starts")
    return started, completed


def validate_protocol(value: dict[str, object]) -> None:
    require_exact_keys(
        value,
        {
            "acceptance_gates",
            "capture",
            "dataset_version",
            "limitations",
            "normalization",
            "schema",
            "selection",
        },
        "protocol",
    )
    require(value["schema"] == PROTOCOL_SCHEMA, "unsupported protocol schema")
    require(value["dataset_version"] == DATASET_VERSION, "unsupported dataset version")

    gates = require_object(value["acceptance_gates"], "protocol.acceptance_gates")
    require_exact_keys(
        gates,
        {
            "actionable_cases",
            "checkpoint_disagreements",
            "classification_disagreements",
            "forbidden_payload_fields",
            "head_disagreements",
            "review_history_disagreements",
            "reviewer_identity_disagreements",
            "stable_controls",
        },
        "protocol.acceptance_gates",
    )
    for name, gate in gates.items():
        require_int(gate, f"protocol.acceptance_gates.{name}")

    capture = require_object(value["capture"], "protocol.capture")
    require_exact_keys(
        capture,
        {"maximum_cross_api_span_seconds", "observations", "source_kind"},
        "protocol.capture",
    )
    require_int(
        capture["maximum_cross_api_span_seconds"],
        "protocol.capture.maximum_cross_api_span_seconds",
        1,
    )
    require(
        capture["source_kind"] == "public_pull_request_review_metadata",
        "protocol source kind differs",
    )
    observations = require_array(capture["observations"], "protocol.capture.observations")
    expected_observations = [
        {
            "api": "github_rest_v3",
            "asset": "rest-observation-v1.json",
            "role": "independent_metadata_oracle",
        },
        {
            "api": "github_graphql_v4",
            "asset": "graphql-observation-v1.json",
            "role": "product_path_representation",
        },
    ]
    require(observations == expected_observations, "protocol observation bindings differ")

    normalization = require_object(value["normalization"], "protocol.normalization")
    require_exact_keys(
        normalization,
        {"classification", "completed_states", "reviewer_identity", "selection_order"},
        "protocol.normalization",
    )
    require(
        normalization["completed_states"] == ["APPROVED", "CHANGES_REQUESTED"],
        "protocol completed states differ",
    )
    require(
        normalization["selection_order"] == ["submitted_at", "database_id"],
        "protocol selection order differs",
    )
    classification = require_object(
        normalization["classification"], "protocol.normalization.classification"
    )
    require(
        classification
        == {
            "different_checkpoint_and_head_oid": "actionable",
            "equal_checkpoint_and_head_oid": "up_to_date",
            "missing_checkpoint_oid": "unobservable",
        },
        "protocol classification differs",
    )
    identity = require_object(
        normalization["reviewer_identity"], "protocol.normalization.reviewer_identity"
    )
    require(
        identity
        == {
            "actor_type": "User",
            "immutable_database_id_required": True,
            "login_match": "case_insensitive",
        },
        "protocol reviewer identity differs",
    )

    selection = require_object(value["selection"], "protocol.selection")
    require_exact_keys(
        selection,
        {"case_ids", "method", "next_prospective_expansion"},
        "protocol.selection",
    )
    case_ids = require_array(selection["case_ids"], "protocol.selection.case_ids")
    require(
        case_ids and all(isinstance(item, str) and item for item in case_ids),
        "case IDs differ",
    )
    require(len(case_ids) == len(set(case_ids)), "protocol case IDs must be unique")
    expansion = require_object(
        selection["next_prospective_expansion"],
        "protocol.selection.next_prospective_expansion",
    )
    require_exact_keys(
        expansion,
        {"minimum_cases", "required_buckets"},
        "protocol.selection.next_prospective_expansion",
    )
    require_int(expansion["minimum_cases"], "prospective minimum cases", 30)
    buckets = require_array(expansion["required_buckets"], "prospective required buckets")
    require(len(buckets) == len(set(buckets)), "prospective buckets must be unique")

    limitations = require_array(value["limitations"], "protocol.limitations")
    require(
        len(limitations) >= 5 and all(isinstance(item, str) and item for item in limitations),
        "protocol must disclose at least five limitations",
    )


def validate_reviewer_identity(value: object, label: str) -> dict[str, object]:
    identity = require_object(value, label)
    require_exact_keys(identity, {"database_id", "login", "type"}, label)
    database_id = validate_database_id(identity["database_id"], f"{label}.database_id")
    login = validate_login(identity["login"], f"{label}.login")
    require(identity["type"] == "User", f"{label}.type must be User")
    return {"database_id": database_id, "login": login, "type": "User"}


def normalize_review(
    *,
    database_id: object,
    node_id: object,
    state: object,
    submitted_at: object,
    commit_oid: object,
    label: str,
) -> dict[str, object]:
    normalized_database_id = validate_database_id(database_id, f"{label}.database_id")
    normalized_node_id = validate_node_id(node_id, f"{label}.node_id")
    normalized_state = require_string(state, f"{label}.state")
    require(normalized_state in REVIEW_STATES, f"{label}.state is unsupported")
    if submitted_at is None:
        require(
            normalized_state not in FORMAL_STATES,
            f"{label}.submitted_at is required for a completed review",
        )
        normalized_submitted_at = None
    else:
        parse_timestamp(submitted_at, f"{label}.submitted_at")
        normalized_submitted_at = submitted_at
    normalized_oid = validate_oid(commit_oid, f"{label}.commit_oid", nullable=True)
    return {
        "commit_oid": normalized_oid,
        "database_id": normalized_database_id,
        "node_id": normalized_node_id,
        "state": normalized_state,
        "submitted_at": normalized_submitted_at,
    }


def validate_normalized_reviews(
    reviews: list[dict[str, object]], label: str
) -> list[dict[str, object]]:
    database_ids = [review["database_id"] for review in reviews]
    node_ids = [review["node_id"] for review in reviews]
    require(len(database_ids) == len(set(database_ids)), f"{label} has duplicate database IDs")
    require(len(node_ids) == len(set(node_ids)), f"{label} has duplicate node IDs")
    return sorted(
        reviews,
        key=lambda review: (
            "" if review["submitted_at"] is None else review["submitted_at"],
            review["database_id"],
        ),
    )


def validate_rest_observation(
    value: dict[str, object], protocol: dict[str, object]
) -> tuple[dict[str, dict[str, object]], tuple[datetime, datetime]]:
    require_exact_keys(
        value,
        {
            "api",
            "capture_completed_at",
            "capture_started_at",
            "cases",
            "dataset_version",
            "privacy",
            "repository",
            "schema",
        },
        "REST observation",
    )
    require(value["schema"] == REST_SCHEMA, "unsupported REST observation schema")
    require(value["dataset_version"] == DATASET_VERSION, "REST dataset version differs")
    repository = require_string(value["repository"], "REST repository")
    require(REPOSITORY_PATTERN.fullmatch(repository) is not None, "REST repository is invalid")
    api = require_object(value["api"], "REST api")
    require(
        api
        == {
            "api_family": "GitHub REST",
            "api_version": "2022-11-28",
            "base_url": "https://api.github.com",
            "pull_request_request": "GET /repos/{owner}/{repo}/pulls/{number}",
            "review_request": (
                "GET /repos/{owner}/{repo}/pulls/{number}/reviews?per_page=100 "
                "with Link pagination"
            ),
            "reviewer_reduction": (
                "Fetch every review page, then retain case-insensitive login matches "
                "and require one immutable User database ID"
            ),
        },
        "REST API provenance differs",
    )
    capture_window = validate_capture_window(
        value["capture_started_at"], value["capture_completed_at"], "REST capture"
    )
    validate_privacy(value["privacy"], "REST privacy")
    reject_forbidden_keys(value, "REST observation")

    raw_cases = require_array(value["cases"], "REST cases")
    expected_ids = protocol["selection"]["case_ids"]
    require(
        [require_object(case, "REST case")["id"] for case in raw_cases] == expected_ids,
        "REST cases differ from protocol order",
    )
    normalized_cases: dict[str, dict[str, object]] = {}
    for index, raw_case in enumerate(raw_cases):
        case = require_object(raw_case, f"REST cases[{index}]")
        require_exact_keys(
            case,
            {"id", "pull_request", "requested_reviewer", "review_observation"},
            f"REST cases[{index}]",
        )
        case_id = require_string(case["id"], f"REST cases[{index}].id")
        reviewer = validate_reviewer_identity(
            case["requested_reviewer"], f"{case_id}.requested_reviewer"
        )
        pull_request = require_object(case["pull_request"], f"{case_id}.pull_request")
        require_exact_keys(
            pull_request,
            {"draft", "head_sha", "html_url", "number", "state", "updated_at"},
            f"{case_id}.pull_request",
        )
        number = require_int(pull_request["number"], f"{case_id}.number", 1)
        require(pull_request["state"] == "open", f"{case_id} is not open in REST")
        require(type(pull_request["draft"]) is bool, f"{case_id}.draft must be boolean")
        parse_timestamp(pull_request["updated_at"], f"{case_id}.updated_at")
        head_oid = validate_oid(pull_request["head_sha"], f"{case_id}.head_sha")
        url = require_string(pull_request["html_url"], f"{case_id}.html_url")
        require(
            url == f"https://github.com/{repository}/pull/{number}",
            f"{case_id} REST URL is not canonical",
        )

        review_observation = require_object(
            case["review_observation"], f"{case_id}.review_observation"
        )
        require_exact_keys(
            review_observation,
            {
                "endpoint_review_count",
                "matching_review_count",
                "pages_fetched",
                "reviews",
                "terminal_page_observed",
            },
            f"{case_id}.review_observation",
        )
        endpoint_count = require_int(
            review_observation["endpoint_review_count"],
            f"{case_id}.endpoint_review_count",
        )
        matching_count = require_int(
            review_observation["matching_review_count"],
            f"{case_id}.matching_review_count",
            1,
        )
        pages_fetched = require_int(
            review_observation["pages_fetched"], f"{case_id}.pages_fetched", 1
        )
        require(
            pages_fetched == max(1, (endpoint_count + 99) // 100),
            f"{case_id} REST page count differs from endpoint count",
        )
        require(
            review_observation["terminal_page_observed"] is True,
            f"{case_id} REST pagination is incomplete",
        )
        raw_reviews = require_array(review_observation["reviews"], f"{case_id}.reviews")
        require(len(raw_reviews) == matching_count, f"{case_id} REST matching count differs")
        require(endpoint_count >= matching_count, f"{case_id} REST endpoint count is too small")

        reviews: list[dict[str, object]] = []
        for review_index, raw_review in enumerate(raw_reviews):
            review = require_object(raw_review, f"{case_id}.reviews[{review_index}]")
            require_exact_keys(
                review,
                {"commit_id", "id", "node_id", "state", "submitted_at", "user"},
                f"{case_id}.reviews[{review_index}]",
            )
            author = validate_reviewer_identity(
                review["user"], f"{case_id}.reviews[{review_index}].user"
            )
            require(
                author["login"].casefold() == reviewer["login"].casefold()
                and author["database_id"] == reviewer["database_id"],
                f"{case_id} REST review author differs from requested reviewer",
            )
            reviews.append(
                normalize_review(
                    database_id=review["id"],
                    node_id=review["node_id"],
                    state=review["state"],
                    submitted_at=review["submitted_at"],
                    commit_oid=review["commit_id"],
                    label=f"{case_id}.reviews[{review_index}]",
                )
            )
        normalized_cases[case_id] = {
            "head_oid": head_oid,
            "is_draft": pull_request["draft"],
            "number": number,
            "repository": repository,
            "reviewer": reviewer,
            "reviews": validate_normalized_reviews(reviews, f"{case_id} REST reviews"),
            "state": "OPEN",
            "updated_at": pull_request["updated_at"],
            "url": url,
        }
    return normalized_cases, capture_window


def validate_graphql_observation(
    value: dict[str, object], protocol: dict[str, object]
) -> tuple[dict[str, dict[str, object]], tuple[datetime, datetime]]:
    require_exact_keys(
        value,
        {
            "api",
            "captureCompletedAt",
            "captureStartedAt",
            "cases",
            "datasetVersion",
            "privacy",
            "repository",
            "schema",
        },
        "GraphQL observation",
    )
    require(value["schema"] == GRAPHQL_SCHEMA, "unsupported GraphQL observation schema")
    require(value["datasetVersion"] == DATASET_VERSION, "GraphQL dataset version differs")
    repository = require_string(value["repository"], "GraphQL repository")
    require(REPOSITORY_PATTERN.fullmatch(repository) is not None, "GraphQL repository is invalid")
    api = require_object(value["api"], "GraphQL api")
    require(
        api
        == {
            "api_family": "GitHub GraphQL",
            "api_version": "v4",
            "endpoint": "https://api.github.com/graphql",
            "query_name": "ReviewInboxEvidenceCapture",
            "review_request": (
                "reviews(first: 100, after: $cursor, author: $reviewer) "
                "with connection pagination"
            ),
        },
        "GraphQL API provenance differs",
    )
    capture_window = validate_capture_window(
        value["captureStartedAt"], value["captureCompletedAt"], "GraphQL capture"
    )
    validate_privacy(value["privacy"], "GraphQL privacy")
    reject_forbidden_keys(value, "GraphQL observation")

    raw_cases = require_array(value["cases"], "GraphQL cases")
    expected_ids = protocol["selection"]["case_ids"]
    require(
        [require_object(case, "GraphQL case")["id"] for case in raw_cases] == expected_ids,
        "GraphQL cases differ from protocol order",
    )
    normalized_cases: dict[str, dict[str, object]] = {}
    for index, raw_case in enumerate(raw_cases):
        case = require_object(raw_case, f"GraphQL cases[{index}]")
        require_exact_keys(
            case,
            {"id", "pullRequest", "reviewerArgument"},
            f"GraphQL cases[{index}]",
        )
        case_id = require_string(case["id"], f"GraphQL cases[{index}].id")
        reviewer_argument = validate_login(
            case["reviewerArgument"], f"{case_id}.reviewerArgument"
        )
        pull_request = require_object(case["pullRequest"], f"{case_id}.pullRequest")
        require_exact_keys(
            pull_request,
            {
                "headRefOid",
                "id",
                "isDraft",
                "number",
                "reviews",
                "state",
                "updatedAt",
                "url",
            },
            f"{case_id}.pullRequest",
        )
        validate_node_id(pull_request["id"], f"{case_id}.pullRequest.id")
        number = require_int(pull_request["number"], f"{case_id}.number", 1)
        require(pull_request["state"] == "OPEN", f"{case_id} is not open in GraphQL")
        require(type(pull_request["isDraft"]) is bool, f"{case_id}.isDraft must be boolean")
        parse_timestamp(pull_request["updatedAt"], f"{case_id}.updatedAt")
        head_oid = validate_oid(pull_request["headRefOid"], f"{case_id}.headRefOid")
        url = require_string(pull_request["url"], f"{case_id}.url")
        require(
            url == f"https://github.com/{repository}/pull/{number}",
            f"{case_id} GraphQL URL is not canonical",
        )

        connection = require_object(pull_request["reviews"], f"{case_id}.reviews")
        require_exact_keys(
            connection,
            {"nodes", "pagesFetched", "terminalPageObserved", "totalCount"},
            f"{case_id}.reviews",
        )
        total_count = require_int(connection["totalCount"], f"{case_id}.totalCount", 1)
        pages_fetched = require_int(connection["pagesFetched"], f"{case_id}.pagesFetched", 1)
        require(
            pages_fetched == max(1, (total_count + 99) // 100),
            f"{case_id} GraphQL page count differs from totalCount",
        )
        require(
            connection["terminalPageObserved"] is True,
            f"{case_id} GraphQL pagination is incomplete",
        )
        raw_reviews = require_array(connection["nodes"], f"{case_id}.reviews.nodes")
        require(len(raw_reviews) == total_count, f"{case_id} GraphQL totalCount differs")

        reviewer: dict[str, object] | None = None
        reviews: list[dict[str, object]] = []
        for review_index, raw_review in enumerate(raw_reviews):
            review = require_object(raw_review, f"{case_id}.reviews[{review_index}]")
            require_exact_keys(
                review,
                {"author", "commit", "fullDatabaseId", "id", "state", "submittedAt"},
                f"{case_id}.reviews[{review_index}]",
            )
            author_value = require_object(
                review["author"], f"{case_id}.reviews[{review_index}].author"
            )
            require_exact_keys(
                author_value,
                {"__typename", "databaseId", "login"},
                f"{case_id}.reviews[{review_index}].author",
            )
            author = validate_reviewer_identity(
                {
                    "database_id": author_value["databaseId"],
                    "login": author_value["login"],
                    "type": author_value["__typename"],
                },
                f"{case_id}.reviews[{review_index}].author",
            )
            require(
                author["login"].casefold() == reviewer_argument.casefold(),
                f"{case_id} GraphQL author differs from reviewer argument",
            )
            if reviewer is None:
                reviewer = author
            else:
                require(author == reviewer, f"{case_id} GraphQL reviewer identity changed")

            commit_value = review["commit"]
            if commit_value is None:
                commit_oid = None
            else:
                commit = require_object(
                    commit_value, f"{case_id}.reviews[{review_index}].commit"
                )
                require_exact_keys(
                    commit, {"oid"}, f"{case_id}.reviews[{review_index}].commit"
                )
                commit_oid = commit["oid"]
            reviews.append(
                normalize_review(
                    database_id=review["fullDatabaseId"],
                    node_id=review["id"],
                    state=review["state"],
                    submitted_at=review["submittedAt"],
                    commit_oid=commit_oid,
                    label=f"{case_id}.reviews[{review_index}]",
                )
            )
        require(reviewer is not None, f"{case_id} has no GraphQL reviewer identity")
        normalized_cases[case_id] = {
            "head_oid": head_oid,
            "is_draft": pull_request["isDraft"],
            "number": number,
            "repository": repository,
            "reviewer": reviewer,
            "reviews": validate_normalized_reviews(reviews, f"{case_id} GraphQL reviews"),
            "state": pull_request["state"],
            "updated_at": pull_request["updatedAt"],
            "url": url,
        }
    return normalized_cases, capture_window


def review_sort_key(review: dict[str, object]) -> tuple[datetime, int]:
    require(review["submitted_at"] is not None, "completed review lacks submitted_at")
    return (
        parse_timestamp(review["submitted_at"], "review.submitted_at"),
        require_int(review["database_id"], "review.database_id", 1),
    )


def derive_case(case: dict[str, object]) -> dict[str, object]:
    reviews = case["reviews"]
    require(isinstance(reviews, list), "normalized reviews must be an array")
    completed = [review for review in reviews if review["state"] in FORMAL_STATES]
    require(completed, "seed case has no completed reviewer checkpoint")
    checkpoint = max(completed, key=review_sort_key)
    checkpoint_key = review_sort_key(checkpoint)
    later_non_completed = sum(
        review["state"] not in FORMAL_STATES
        and review["submitted_at"] is not None
        and review_sort_key(review) > checkpoint_key
        for review in reviews
    )
    checkpoint_oid = checkpoint["commit_oid"]
    if checkpoint_oid is None:
        classification = "unobservable"
    elif checkpoint_oid == case["head_oid"]:
        classification = "up_to_date"
    else:
        classification = "actionable"
    return {
        "classification": classification,
        "head_oid": case["head_oid"],
        "later_non_completed_review_count": later_non_completed,
        "reviewer": {
            "database_id": case["reviewer"]["database_id"],
            "login": case["reviewer"]["login"],
        },
        "reviewer_review_count": len(reviews),
        "selected_checkpoint": {
            "database_id": str(checkpoint["database_id"]),
            "node_id": checkpoint["node_id"],
            "oid": checkpoint_oid,
            "state": checkpoint["state"],
            "submitted_at": checkpoint["submitted_at"],
        },
    }


def comparison_cases_and_summary(
    rest_cases: dict[str, dict[str, object]],
    graphql_cases: dict[str, dict[str, object]],
    protocol: dict[str, object],
) -> tuple[list[dict[str, object]], dict[str, int]]:
    results: list[dict[str, object]] = []
    for case_id in protocol["selection"]["case_ids"]:
        rest_case = rest_cases[case_id]
        graphql_case = graphql_cases[case_id]
        for field in ("is_draft", "number", "repository", "state", "url"):
            require(
                rest_case[field] == graphql_case[field],
                f"{case_id} cross-API {field} differs",
            )
        rest_result = derive_case(rest_case)
        graphql_result = derive_case(graphql_case)
        agreement = {
            "checkpoint": rest_result["selected_checkpoint"]
            == graphql_result["selected_checkpoint"],
            "classification": rest_result["classification"]
            == graphql_result["classification"],
            "head": rest_result["head_oid"] == graphql_result["head_oid"],
            "review_history": rest_case["reviews"] == graphql_case["reviews"],
            "reviewer_identity": rest_result["reviewer"] == graphql_result["reviewer"],
        }
        results.append(
            {
                "agreement": agreement,
                "expected": graphql_result["classification"],
                "graphql": graphql_result,
                "head_oid": graphql_result["head_oid"],
                "id": case_id,
                "is_draft": graphql_case["is_draft"],
                "later_non_completed_review_count": graphql_result[
                    "later_non_completed_review_count"
                ],
                "number": graphql_case["number"],
                "repository": graphql_case["repository"],
                "rest": rest_result,
                "reviewer": graphql_result["reviewer"]["login"],
                "reviewer_review_count": graphql_result["reviewer_review_count"],
                "selected_checkpoint": graphql_result["selected_checkpoint"],
                "updated_at": graphql_case["updated_at"],
                "url": graphql_case["url"],
            }
        )

    summary = {
        "actionable": sum(
            result["graphql"]["classification"] == "actionable" for result in results
        ),
        "cases": len(results),
        "checkpoint_disagreements": sum(
            not result["agreement"]["checkpoint"] for result in results
        ),
        "classification_disagreements": sum(
            not result["agreement"]["classification"] for result in results
        ),
        "commented_after_completed": sum(
            result["graphql"]["later_non_completed_review_count"] > 0
            for result in results
        ),
        "head_disagreements": sum(not result["agreement"]["head"] for result in results),
        "review_history_disagreements": sum(
            not result["agreement"]["review_history"] for result in results
        ),
        "reviewer_identity_disagreements": sum(
            not result["agreement"]["reviewer_identity"] for result in results
        ),
        "unobservable": sum(
            result["graphql"]["classification"] == "unobservable" for result in results
        ),
        "up_to_date": sum(
            result["graphql"]["classification"] == "up_to_date" for result in results
        ),
    }
    return results, summary


def validate_oracle(
    value: dict[str, object],
    protocol_bytes: bytes,
    rest_bytes: bytes,
    graphql_bytes: bytes,
    expected_cases: list[dict[str, object]],
    expected_summary: dict[str, int],
    expected_captured_at: str,
    protocol: dict[str, object],
) -> None:
    require_exact_keys(
        value,
        {
            "captured_at",
            "cases",
            "dataset_version",
            "observations",
            "protocol_sha256",
            "schema",
            "summary",
        },
        "oracle",
    )
    require(value["schema"] == ORACLE_SCHEMA, "unsupported oracle schema")
    require(value["dataset_version"] == DATASET_VERSION, "oracle dataset version differs")
    parse_timestamp(value["captured_at"], "oracle.captured_at")
    require(
        value["captured_at"] == expected_captured_at,
        "oracle capture timestamp differs from the latest observation",
    )
    require(value["protocol_sha256"] == sha256_bytes(protocol_bytes), "protocol hash differs")
    observations = require_object(value["observations"], "oracle.observations")
    require_exact_keys(
        observations, {"github_graphql_v4", "github_rest_v3"}, "oracle.observations"
    )
    expected_bindings = {
        "github_graphql_v4": {
            "path": "graphql-observation-v1.json",
            "sha256": sha256_bytes(graphql_bytes),
        },
        "github_rest_v3": {
            "path": "rest-observation-v1.json",
            "sha256": sha256_bytes(rest_bytes),
        },
    }
    require(observations == expected_bindings, "oracle observation hashes differ")
    reject_forbidden_keys(value, "oracle")
    require(value["cases"] == expected_cases, "oracle cases differ from dual-API derivation")
    require(value["summary"] == expected_summary, "oracle summary differs from recomputation")

    gates = protocol["acceptance_gates"]
    require(
        gates["actionable_cases"] == expected_summary["actionable"],
        "actionable gate failed",
    )
    require(
        gates["stable_controls"] == expected_summary["up_to_date"],
        "stable-control gate failed",
    )
    for field in (
        "checkpoint_disagreements",
        "classification_disagreements",
        "head_disagreements",
        "review_history_disagreements",
        "reviewer_identity_disagreements",
    ):
        require(gates[field] == expected_summary[field], f"{field} gate failed")
    require(gates["forbidden_payload_fields"] == 0, "privacy gate must be zero")
    require(
        expected_summary["commented_after_completed"] >= 1,
        "missing commented-after-completed case",
    )


def validate_bundle(
    protocol_bytes: bytes,
    protocol: dict[str, object],
    rest_bytes: bytes,
    rest: dict[str, object],
    graphql_bytes: bytes,
    graphql: dict[str, object],
    oracle: dict[str, object],
) -> tuple[list[dict[str, object]], dict[str, int]]:
    validate_protocol(protocol)
    rest_cases, rest_window = validate_rest_observation(rest, protocol)
    graphql_cases, graphql_window = validate_graphql_observation(graphql, protocol)
    earliest = min(rest_window[0], graphql_window[0])
    latest = max(rest_window[1], graphql_window[1])
    if rest_window[1] >= graphql_window[1]:
        latest_capture_text = rest["capture_completed_at"]
    else:
        latest_capture_text = graphql["captureCompletedAt"]
    maximum_span = protocol["capture"]["maximum_cross_api_span_seconds"]
    require(
        (latest - earliest).total_seconds() <= maximum_span,
        "cross-API capture span exceeds the protocol limit",
    )
    expected_cases, expected_summary = comparison_cases_and_summary(
        rest_cases, graphql_cases, protocol
    )
    validate_oracle(
        oracle,
        protocol_bytes,
        rest_bytes,
        graphql_bytes,
        expected_cases,
        expected_summary,
        latest_capture_text,
        protocol,
    )
    return expected_cases, expected_summary


def rebound_oracle(
    oracle: dict[str, object], rest: dict[str, object], graphql: dict[str, object]
) -> dict[str, object]:
    rebound = copy.deepcopy(oracle)
    rebound["observations"]["github_rest_v3"]["sha256"] = sha256_bytes(
        canonical_json(rest)
    )
    rebound["observations"]["github_graphql_v4"]["sha256"] = sha256_bytes(
        canonical_json(graphql)
    )
    return rebound


def expect_failure(action: Callable[[], object], label: str) -> None:
    failed = False
    try:
        action()
    except InboxOracleError:
        failed = True
    require(failed, f"self-test mutation unexpectedly passed: {label}")


def command_verify(arguments: argparse.Namespace) -> None:
    protocol_bytes, protocol = read_canonical_json(arguments.protocol)
    rest_bytes, rest = read_canonical_json(arguments.rest_observation)
    graphql_bytes, graphql = read_canonical_json(arguments.graphql_observation)
    _, oracle = read_canonical_json(arguments.oracle)
    _, summary = validate_bundle(
        protocol_bytes,
        protocol,
        rest_bytes,
        rest,
        graphql_bytes,
        graphql,
        oracle,
    )
    print(
        "verified Review Inbox v1 dual observation: "
        f"{summary['cases']} cases, "
        f"{summary['actionable']} actionable, "
        f"{summary['classification_disagreements']} classification disagreements"
    )


def command_self_test(arguments: argparse.Namespace) -> None:
    protocol_bytes, protocol = read_canonical_json(arguments.protocol)
    rest_bytes, rest = read_canonical_json(arguments.rest_observation)
    graphql_bytes, graphql = read_canonical_json(arguments.graphql_observation)
    _, oracle = read_canonical_json(arguments.oracle)
    validate_bundle(
        protocol_bytes,
        protocol,
        rest_bytes,
        rest,
        graphql_bytes,
        graphql,
        oracle,
    )

    changed_graphql_head = copy.deepcopy(graphql)
    changed_graphql_head["cases"][0]["pullRequest"]["headRefOid"] = "0" * 40
    changed_head_oracle = rebound_oracle(oracle, rest, changed_graphql_head)
    expect_failure(
        lambda: validate_bundle(
            protocol_bytes,
            protocol,
            rest_bytes,
            rest,
            canonical_json(changed_graphql_head),
            changed_graphql_head,
            changed_head_oracle,
        ),
        "cross-API head disagreement",
    )

    promoted_comment = copy.deepcopy(graphql)
    promoted_comment["cases"][0]["pullRequest"]["reviews"]["nodes"][-1]["state"] = "APPROVED"
    promoted_comment_oracle = rebound_oracle(oracle, rest, promoted_comment)
    expect_failure(
        lambda: validate_bundle(
            protocol_bytes,
            protocol,
            rest_bytes,
            rest,
            canonical_json(promoted_comment),
            promoted_comment,
            promoted_comment_oracle,
        ),
        "cross-API checkpoint disagreement",
    )

    leaked_payload = copy.deepcopy(rest)
    leaked_payload["cases"][0]["body"] = "must never be captured"
    leaked_oracle = rebound_oracle(oracle, leaked_payload, graphql)
    expect_failure(
        lambda: validate_bundle(
            protocol_bytes,
            protocol,
            canonical_json(leaked_payload),
            leaked_payload,
            graphql_bytes,
            graphql,
            leaked_oracle,
        ),
        "forbidden payload",
    )

    incomplete_graphql = copy.deepcopy(graphql)
    incomplete_graphql["cases"][0]["pullRequest"]["reviews"][
        "terminalPageObserved"
    ] = False
    incomplete_oracle = rebound_oracle(oracle, rest, incomplete_graphql)
    expect_failure(
        lambda: validate_bundle(
            protocol_bytes,
            protocol,
            rest_bytes,
            rest,
            canonical_json(incomplete_graphql),
            incomplete_graphql,
            incomplete_oracle,
        ),
        "incomplete GraphQL pagination",
    )

    wrong_reviewer = copy.deepcopy(rest)
    wrong_reviewer["cases"][0]["review_observation"]["reviews"][0]["user"][
        "database_id"
    ] += 1
    wrong_reviewer_oracle = rebound_oracle(oracle, wrong_reviewer, graphql)
    expect_failure(
        lambda: validate_bundle(
            protocol_bytes,
            protocol,
            canonical_json(wrong_reviewer),
            wrong_reviewer,
            graphql_bytes,
            graphql,
            wrong_reviewer_oracle,
        ),
        "mutable-login identity collision",
    )

    stale_protocol = copy.deepcopy(oracle)
    stale_protocol["protocol_sha256"] = "0" * 64
    expect_failure(
        lambda: validate_bundle(
            protocol_bytes,
            protocol,
            rest_bytes,
            rest,
            graphql_bytes,
            graphql,
            stale_protocol,
        ),
        "stale protocol hash",
    )

    stale_observation = copy.deepcopy(oracle)
    stale_observation["observations"]["github_rest_v3"]["sha256"] = "0" * 64
    expect_failure(
        lambda: validate_bundle(
            protocol_bytes,
            protocol,
            rest_bytes,
            rest,
            graphql_bytes,
            graphql,
            stale_observation,
        ),
        "stale observation hash",
    )

    changed_oracle = copy.deepcopy(oracle)
    changed_oracle["cases"][0]["expected"] = "up_to_date"
    expect_failure(
        lambda: validate_bundle(
            protocol_bytes,
            protocol,
            rest_bytes,
            rest,
            graphql_bytes,
            graphql,
            changed_oracle,
        ),
        "frozen expected classification",
    )

    tie_reviews = [
        {
            "commit_oid": "1" * 40,
            "database_id": 41,
            "node_id": "first",
            "state": "APPROVED",
            "submitted_at": "2026-09-05T00:00:00Z",
        },
        {
            "commit_oid": "2" * 40,
            "database_id": 42,
            "node_id": "second",
            "state": "CHANGES_REQUESTED",
            "submitted_at": "2026-09-05T00:00:00Z",
        },
    ]
    require(
        max(tie_reviews, key=review_sort_key)["database_id"] == 42,
        "database ID tie-break self-test failed",
    )
    print("Review Inbox v1 dual-observation self-test passed")


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--protocol", type=Path, default=DEFAULT_PROTOCOL)
    parser.add_argument("--oracle", type=Path, default=DEFAULT_ORACLE)
    parser.add_argument(
        "--rest-observation", type=Path, default=DEFAULT_REST_OBSERVATION
    )
    parser.add_argument(
        "--graphql-observation", type=Path, default=DEFAULT_GRAPHQL_OBSERVATION
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    verify = subparsers.add_parser("verify")
    verify.set_defaults(function=command_verify)
    self_test = subparsers.add_parser("self-test")
    self_test.set_defaults(function=command_self_test)
    return parser.parse_args()


def main() -> None:
    arguments = parse_arguments()
    arguments.function(arguments)


if __name__ == "__main__":
    main()
