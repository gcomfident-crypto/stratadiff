#!/usr/bin/env python3

import argparse
from collections import Counter
import copy
from datetime import datetime
import hashlib
import json
from pathlib import Path
import tempfile


ROOT = Path(__file__).resolve().parents[2]
CENSUS_ROOT = ROOT / "benchmarks" / "review-churn-census-v1"
DEFAULT_SAMPLING_PLAN = CENSUS_ROOT / "sampling-plan.json"
DEFAULT_SAMPLE = CENSUS_ROOT / "sample.json"
DEFAULT_CAPTURE = CENSUS_ROOT / "capture.json"
DEFAULT_MANIFEST = CENSUS_ROOT / "manifest.json"
DEFAULT_LICENSE_POLICY = Path(__file__).resolve().parent / "license-policy-v1.json"
DEFAULT_EVALUATION_PROTOCOL = Path(__file__).resolve().parent / "evaluation-protocol-v1.json"

SAMPLING_PLAN_SCHEMA = "stratadiff-review-churn-census-sampling-plan-v1"
SAMPLE_SCHEMA = "stratadiff-review-churn-census-sample-v1"
CAPTURE_SCHEMA = "stratadiff-review-churn-census-capture-v1"
MANIFEST_SCHEMA = "stratadiff-review-churn-census-manifest-v1"
LICENSE_POLICY_SCHEMA = "stratadiff-review-transition-license-policy-v1"
PLAN_SCHEMA = "stratadiff-review-transition-plan-v1"
CENSUS_DATASET_VERSION = "1.0.0"
LICENSE_POLICY_VERSION = "1.0.0"
PLAN_DATASET_VERSION = "0.1.0"
SELECTION_DOMAIN = b"stratadiff-review-transition-selection-v1"
ALLOWED_SPDX_IDS = [
    "0BSD",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "MIT",
]
PLAN_FIELDS = {
    "claim_boundary",
    "dataset_version",
    "inputs",
    "reviewer_pair_frame",
    "schema",
    "selected_cases",
    "selection",
    "summary",
}
SELECTION_FIELDS = {
    "algorithm",
    "domain",
    "eligible_pull_requests",
    "eligible_reviewer_pairs",
    "ordering",
    "per_pull_request_limit",
    "ranking_preimage",
    "requested_count",
    "seed_hex",
    "selected_count",
    "without_replacement",
}
SAMPLING_PLAN_FIELDS = {
    "actor_policy",
    "analysis",
    "bias_register",
    "checkpoint_policy",
    "claim_boundary",
    "dataset_version",
    "decision_thresholds",
    "event_policy",
    "formal_review_policy",
    "frame_construction",
    "merged_at_window",
    "method_status",
    "metrics",
    "name",
    "panel_kind",
    "prior_evidence_disclosure",
    "privacy",
    "repositories",
    "schema",
    "selection",
    "target_population",
    "target_pull_requests_per_repository",
}
SAMPLE_FIELDS = {
    "acquisition",
    "dataset_version",
    "generated_at",
    "merged_at_window",
    "repositories",
    "schema",
    "selection",
    "source_plan",
    "summary",
    "tool_version",
}
CAPTURE_FIELDS = {
    "acquisition",
    "capture_complete",
    "captured_at",
    "cases",
    "dataset_version",
    "schema",
    "source_sample",
    "summary",
    "tool_version",
}
MANIFEST_FIELDS = {
    "capture_sha256",
    "collection",
    "dataset_version",
    "generated_at",
    "pull_requests",
    "repositories",
    "sample_sha256",
    "sampling_plan_sha256",
    "schema",
    "tool_version",
}
SAMPLE_REPOSITORY_FIELDS = {
    "candidate_count",
    "candidates",
    "name",
    "name_with_owner",
    "owner",
    "requested_count",
    "selected_count",
    "selected_pull_request_numbers",
    "shortfall",
}
CAPTURE_CASE_FIELDS = {"id", "pagination", "pull_request", "repository", "reviews", "timeline_events"}
CAPTURE_REPOSITORY_FIELDS = {"name", "name_with_owner", "node_id", "owner", "url"}
CAPTURE_PULL_REQUEST_FIELDS = {
    "author",
    "commit_count",
    "head_matches_last_commit",
    "head_oid",
    "last_commit_oid",
    "merged_at",
    "node_id",
    "number",
}
CAPTURE_REVIEW_FIELDS = {
    "author",
    "comment_count",
    "commit_oid",
    "database_id",
    "node_id",
    "state",
    "submitted_at",
}
MANIFEST_PULL_REQUEST_FIELDS = {
    "author_class",
    "capture_complete",
    "classification",
    "counts",
    "final_head_oid",
    "id",
    "last_commit_oid",
    "merged_at",
    "node_id",
    "number",
    "repository",
    "reviewer_pairs",
}
REVIEWER_PAIR_FIELDS = {
    "commented_candidate_sessions",
    "commented_newer_commit_candidate",
    "commented_only",
    "completed_review_sessions",
    "formal_review_sessions",
    "latest_commented_candidate",
    "latest_completed_checkpoint",
    "reviewer_key",
}
CHECKPOINT_FIELDS = {
    "commit_oid",
    "completed_state",
    "current_state",
    "differs_from_final_head",
    "dismissal_event_id",
    "dismissed",
    "force_push_rereview",
    "post_completed_review_force_push",
    "post_latest_checkpoint_force_push",
    "review_id",
    "submitted_at",
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def require_object(value, label):
    require(isinstance(value, dict), f"{label} must be an object")
    return value


def require_array(value, label):
    require(isinstance(value, list), f"{label} must be an array")
    return value


def require_string(value, label):
    require(isinstance(value, str) and value, f"{label} must be a non-empty string")
    return value


def require_integer(value, label):
    require(isinstance(value, int) and not isinstance(value, bool), f"{label} must be an integer")
    return value


def require_exact_fields(value, fields, label):
    value = require_object(value, label)
    require(set(value) == set(fields), f"{label} fields differ: {sorted(value)}")
    return value


def unique_json_object(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value, f"duplicate JSON key: {key}")
        value[key] = item
    return value


def reject_json_constant(value):
    raise ValueError(f"non-finite JSON number is forbidden: {value}")


def canonical_json_bytes(value):
    return (
        json.dumps(value, allow_nan=False, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")


def load_json(path):
    raw = path.read_bytes()
    value = json.loads(
        raw.decode("utf-8"),
        object_pairs_hook=unique_json_object,
        parse_constant=reject_json_constant,
    )
    require(isinstance(value, dict), f"{path} must contain a JSON object")
    return raw, value


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(canonical_json_bytes(value))


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def validate_sha256(value, label):
    require(
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value),
        f"{label} must be 64 lowercase hexadecimal characters",
    )


def is_oid(value):
    return (
        isinstance(value, str)
        and len(value) == 40
        and all(character in "0123456789abcdef" for character in value)
    )


def validate_optional_oid(value, label):
    require(value is None or is_oid(value), f"{label} must be null or a full lowercase SHA-1 object ID")


def validate_seed(seed_hex):
    validate_sha256(seed_hex, "selection seed")
    return bytes.fromhex(seed_hex)


def validate_timestamp(value, label):
    require(isinstance(value, str) and value.endswith("Z"), f"{label} must be a UTC timestamp ending in Z")
    timestamp = datetime.fromisoformat(value.replace("Z", "+00:00"))
    require(timestamp.tzinfo is not None, f"{label} must include a timezone")
    return timestamp


def validate_database_id(value, label):
    if type(value) is int:
        require(value > 0, f"{label} must be positive")
        return value
    value = require_string(value, label)
    require(
        value.isascii() and value.isdigit() and not value.startswith("0") and len(value) <= 40,
        f"{label} must be canonical decimal",
    )
    return int(value)


def selection_digest(seed_bytes, case_id, reviewer_key):
    require_string(case_id, "case ID")
    require_string(reviewer_key, "reviewer key")
    preimage = b"\x00".join(
        (SELECTION_DOMAIN, seed_bytes, case_id.encode("utf-8"), reviewer_key.encode("utf-8"))
    )
    return hashlib.sha256(preimage).hexdigest()


def artifact_descriptor(name, value, raw):
    return {
        "artifact": name,
        "dataset_version": value["dataset_version"],
        "schema": value["schema"],
        "sha256": sha256_bytes(raw),
    }


def validate_headers(sampling_plan, sample, capture, manifest):
    require_exact_fields(sampling_plan, SAMPLING_PLAN_FIELDS, "sampling plan")
    require_exact_fields(sample, SAMPLE_FIELDS, "sample")
    require_exact_fields(capture, CAPTURE_FIELDS, "capture")
    manifest_fields = set(manifest)
    require(
        manifest_fields == MANIFEST_FIELDS or manifest_fields == MANIFEST_FIELDS | {"product_result"},
        "manifest top-level fields differ",
    )
    expected = (
        (sampling_plan, SAMPLING_PLAN_SCHEMA, "sampling plan"),
        (sample, SAMPLE_SCHEMA, "sample"),
        (capture, CAPTURE_SCHEMA, "capture"),
        (manifest, MANIFEST_SCHEMA, "manifest"),
    )
    for value, schema, label in expected:
        require(value["schema"] == schema, f"unsupported {label} schema")
        require(value["dataset_version"] == CENSUS_DATASET_VERSION, f"unsupported {label} dataset version")
    require(capture["capture_complete"] is True, "capture is not complete")
    capture_summary = require_exact_fields(
        capture["summary"], {"capture_failures", "captured_pull_requests", "selected_pull_requests"}, "capture summary"
    )
    require(capture_summary["capture_failures"] == 0, "capture reports failures")
    collection = require_exact_fields(
        manifest["collection"],
        {"capture_failures", "captured_at", "classified_pull_requests", "selected_pull_requests", "status"},
        "manifest collection",
    )
    require(collection["status"] == "complete", "manifest collection is not complete")
    require(collection["capture_failures"] == 0, "manifest collection reports capture failures")
    actor_policy = require_object(sampling_plan["actor_policy"], "sampling plan actor policy")
    require(actor_policy["exclude_pull_request_author"] is True, "sampling plan does not exclude PR authors")
    require(actor_policy["human_author_typenames"] == ["User"], "sampling plan human actor policy differs")
    plan_selection = require_exact_fields(
        sampling_plan["selection"],
        {
            "algorithm",
            "algorithm_version",
            "ordering",
            "quota",
            "ranking_preimage",
            "seed_hex",
            "unit",
            "without_replacement",
        },
        "sampling plan selection",
    )
    sample_selection = require_exact_fields(
        sample["selection"], {"algorithm", "algorithm_version", "seed_hex"}, "sample selection"
    )
    require(plan_selection["algorithm"] == "sha256_v1", "sampling plan selection algorithm differs")
    require(plan_selection["algorithm_version"] == "1", "sampling plan selection version differs")
    require(plan_selection["without_replacement"] is True, "sampling plan selection permits replacement")
    require(sample_selection["algorithm"] == plan_selection["algorithm"], "sample selection algorithm differs")
    require(
        sample_selection["algorithm_version"] == plan_selection["algorithm_version"],
        "sample selection version differs",
    )


def validate_hash_chain(raws, sampling_plan, sample, capture, manifest):
    plan_sha = sha256_bytes(raws["sampling_plan"])
    sample_sha = sha256_bytes(raws["sample"])
    capture_sha = sha256_bytes(raws["capture"])

    source_plan = require_exact_fields(
        sample["source_plan"], {"dataset_version", "schema", "sha256"}, "sample.source_plan"
    )
    require(source_plan["schema"] == SAMPLING_PLAN_SCHEMA, "sample source plan schema differs")
    require(source_plan["dataset_version"] == CENSUS_DATASET_VERSION, "sample source plan version differs")
    require(source_plan["sha256"] == plan_sha, "sample source plan SHA-256 differs from sampling-plan bytes")

    source_sample = require_exact_fields(
        capture["source_sample"], {"dataset_version", "schema", "sha256"}, "capture.source_sample"
    )
    require(source_sample["schema"] == SAMPLE_SCHEMA, "capture source sample schema differs")
    require(source_sample["dataset_version"] == CENSUS_DATASET_VERSION, "capture source sample version differs")
    require(source_sample["sha256"] == sample_sha, "capture source sample SHA-256 differs from sample bytes")

    require(manifest["sampling_plan_sha256"] == plan_sha, "manifest sampling-plan SHA-256 differs")
    require(manifest["sample_sha256"] == sample_sha, "manifest sample SHA-256 differs")
    require(manifest["capture_sha256"] == capture_sha, "manifest capture SHA-256 differs")


def sampling_plan_repositories(sampling_plan):
    repositories = []
    for index, repository in enumerate(require_array(sampling_plan["repositories"], "sampling plan repositories")):
        repository = require_exact_fields(repository, {"name", "owner"}, f"sampling plan repository {index}")
        owner = require_string(repository["owner"], f"sampling plan repository {index} owner")
        name = require_string(repository["name"], f"sampling plan repository {index} name")
        repositories.append(f"{owner}/{name}")
    require(len(repositories) == len(set(repositories)), "sampling plan contains duplicate repositories")
    return repositories


def sample_pull_requests(sample):
    selected = {}
    repositories = require_array(sample["repositories"], "sample repositories")
    for repository_index, repository in enumerate(repositories):
        repository = require_exact_fields(
            repository, SAMPLE_REPOSITORY_FIELDS, f"sample repository {repository_index}"
        )
        name = require_string(repository["name_with_owner"], f"sample repository {repository_index} name")
        candidates = {}
        for candidate_index, candidate in enumerate(require_array(repository["candidates"], f"{name} candidates")):
            candidate = require_exact_fields(
                candidate,
                {"merged_at", "node_id", "number", "selection_digest"},
                f"{name} candidate {candidate_index}",
            )
            number = require_integer(candidate["number"], f"{name} candidate number")
            require(number > 0, f"{name} candidate number must be positive")
            require(number not in candidates, f"{name} has duplicate candidate PR {number}")
            candidates[number] = require_string(candidate["node_id"], f"{name} PR {number} node ID")

        selected_numbers = require_array(repository["selected_pull_request_numbers"], f"{name} selected numbers")
        require(
            len(selected_numbers) == require_integer(repository["selected_count"], f"{name} selected count"),
            f"{name} selected count differs from selected number array",
        )
        require(len(selected_numbers) == len(set(selected_numbers)), f"{name} repeats a selected PR number")
        for number in selected_numbers:
            require_integer(number, f"{name} selected PR number")
            require(number in candidates, f"{name} selected PR {number} is absent from candidates")
            identity = (name, number, candidates[number])
            require(identity not in selected, f"duplicate sample PR identity: {identity}")
            selected[identity] = None
    return set(selected)


def capture_pull_requests(capture):
    identities = set()
    cases_by_identity = {}
    case_ids = set()
    for index, case in enumerate(require_array(capture["cases"], "capture cases")):
        case = require_exact_fields(case, CAPTURE_CASE_FIELDS, f"capture case {index}")
        case_id = require_string(case["id"], f"capture case {index} ID")
        require(case_id not in case_ids, f"duplicate capture case ID: {case_id}")
        case_ids.add(case_id)
        repository = require_exact_fields(case["repository"], CAPTURE_REPOSITORY_FIELDS, f"{case_id} repository")
        pull_request = require_exact_fields(
            case["pull_request"], CAPTURE_PULL_REQUEST_FIELDS, f"{case_id} pull request"
        )
        pull_request_author = pull_request["author"]
        if pull_request_author is not None:
            pull_request_author = require_exact_fields(
                pull_request_author, {"actor_key", "typename"}, f"{case_id} pull request author"
            )
            require_string(pull_request_author["typename"], f"{case_id} pull request author type")
            require(
                pull_request_author["actor_key"] is None
                or (
                    isinstance(pull_request_author["actor_key"], str)
                    and pull_request_author["actor_key"].startswith("actor-")
                    and len(pull_request_author["actor_key"]) == 30
                    and all(
                        character in "0123456789abcdef"
                        for character in pull_request_author["actor_key"][6:]
                    )
                ),
                f"{case_id} pull request author key is invalid",
            )
        identity = (
            require_string(repository["name_with_owner"], f"{case_id} repository name"),
            require_integer(pull_request["number"], f"{case_id} PR number"),
            require_string(pull_request["node_id"], f"{case_id} PR node ID"),
        )
        require(identity not in identities, f"duplicate capture PR identity: {identity}")
        identities.add(identity)
        cases_by_identity[identity] = case
        pagination = require_exact_fields(case["pagination"], {"reviews", "timeline_events"}, f"{case_id} pagination")
        review_pagination = require_exact_fields(
            pagination["reviews"],
            {"captured_node_count", "pages", "pagination_complete", "reported_total_count"},
            f"{case_id} review pagination",
        )
        timeline_pagination = require_exact_fields(
            pagination["timeline_events"],
            {
                "captured_filtered_node_count",
                "pages",
                "pagination_complete",
                "reported_total_count_unfiltered",
            },
            f"{case_id} timeline pagination",
        )
        require(review_pagination["pagination_complete"] is True, f"{case_id} review pagination is incomplete")
        require(timeline_pagination["pagination_complete"] is True, f"{case_id} timeline pagination is incomplete")
        reviews = require_array(case["reviews"], f"{case_id} reviews")
        timeline_events = require_array(case["timeline_events"], f"{case_id} timeline events")
        require(review_pagination["captured_node_count"] == len(reviews), f"{case_id} captured review count differs")
        require(review_pagination["reported_total_count"] == len(reviews), f"{case_id} reported review count differs")
        require(
            timeline_pagination["captured_filtered_node_count"] == len(timeline_events),
            f"{case_id} captured timeline count differs",
        )
        review_ids = set()
        for review_index, review in enumerate(reviews):
            review = require_exact_fields(review, CAPTURE_REVIEW_FIELDS, f"{case_id} review {review_index}")
            author = require_exact_fields(review["author"], {"actor_key", "typename"}, f"{case_id} review author")
            actor_key = require_string(author["actor_key"], f"{case_id} review actor key")
            require(
                actor_key.startswith("actor-")
                and len(actor_key) == 30
                and all(character in "0123456789abcdef" for character in actor_key[6:]),
                f"{case_id} review actor key is not pseudonymous",
            )
            require_string(author["typename"], f"{case_id} review actor type")
            review_id = require_string(review["node_id"], f"{case_id} review node ID")
            require(review_id not in review_ids, f"{case_id} repeats review node ID {review_id}")
            review_ids.add(review_id)
            require(
                review["state"] in ("APPROVED", "CHANGES_REQUESTED", "COMMENTED", "DISMISSED"),
                f"{case_id} review {review_id} state is invalid",
            )
            validate_timestamp(review["submitted_at"], f"{case_id} review {review_id} submitted_at")
            validate_database_id(review["database_id"], f"{case_id} review {review_id} database ID")
            validate_optional_oid(review["commit_oid"], f"{case_id} review {review_id} commit OID")
    return identities, cases_by_identity


def manifest_pull_requests(manifest):
    identities = set()
    pull_requests_by_identity = {}
    case_ids = set()
    for index, pull_request in enumerate(require_array(manifest["pull_requests"], "manifest pull requests")):
        pull_request = require_exact_fields(
            pull_request, MANIFEST_PULL_REQUEST_FIELDS, f"manifest pull request {index}"
        )
        case_id = require_string(pull_request["id"], f"manifest pull request {index} ID")
        require(case_id not in case_ids, f"duplicate manifest case ID: {case_id}")
        case_ids.add(case_id)
        identity = (
            require_string(pull_request["repository"], f"{case_id} repository"),
            require_integer(pull_request["number"], f"{case_id} PR number"),
            require_string(pull_request["node_id"], f"{case_id} PR node ID"),
        )
        require(identity not in identities, f"duplicate manifest PR identity: {identity}")
        identities.add(identity)
        pull_requests_by_identity[identity] = pull_request
    return identities, pull_requests_by_identity


def validate_pull_request_frame(sampling_plan, sample, capture, manifest):
    planned_repositories = sampling_plan_repositories(sampling_plan)
    sample_repositories = [repository["name_with_owner"] for repository in sample["repositories"]]
    require(len(sample_repositories) == len(set(sample_repositories)), "sample repeats a repository summary")
    require(
        set(sample_repositories) == set(planned_repositories),
        "sample repository set differs from sampling plan",
    )
    sample_ids = sample_pull_requests(sample)
    capture_ids, capture_by_identity = capture_pull_requests(capture)
    manifest_ids, manifest_by_identity = manifest_pull_requests(manifest)
    require(sample_ids == capture_ids, "sample selected PR set differs from capture cases")
    require(sample_ids == manifest_ids, "sample selected PR set differs from manifest pull requests")
    capture_summary = capture["summary"]
    require(capture_summary["selected_pull_requests"] == len(sample_ids), "capture selected count differs")
    require(capture_summary["captured_pull_requests"] == len(capture_ids), "capture case count differs")
    collection = manifest["collection"]
    require(collection["selected_pull_requests"] == len(sample_ids), "manifest selected count differs")
    require(collection["classified_pull_requests"] == len(manifest_ids), "manifest classified count differs")
    manifest_repositories = []
    for index, repository in enumerate(require_array(manifest["repositories"], "manifest repositories")):
        repository = require_exact_fields(
            repository,
            {"capture_failures", "frame_candidates", "name_with_owner", "selected", "target"},
            f"manifest repository {index}",
        )
        require(repository["capture_failures"] == 0, f"{repository['name_with_owner']} reports capture failures")
        require(repository["selected"] == repository["target"], f"{repository['name_with_owner']} has a shortfall")
        manifest_repositories.append(repository["name_with_owner"])
    require(
        set(manifest_repositories) == set(planned_repositories),
        "manifest repository set differs from sampling plan",
    )
    require(len(manifest_repositories) == len(set(manifest_repositories)), "manifest repeats a repository summary")
    return planned_repositories, capture_by_identity, manifest_by_identity


def validate_license_policy(policy, planned_repositories):
    require_exact_fields(
        policy,
        {"allowed_spdx_ids", "observed_at", "policy_version", "repositories", "schema", "source"},
        "license policy",
    )
    require(policy["schema"] == LICENSE_POLICY_SCHEMA, "unsupported license policy schema")
    require(policy["policy_version"] == LICENSE_POLICY_VERSION, "unsupported license policy version")
    require(policy["observed_at"] == "2026-09-06", "unexpected license observation date")
    require(policy["source"] == "github_repository_license_metadata", "unexpected license policy source")
    require(policy["allowed_spdx_ids"] == ALLOWED_SPDX_IDS, "license allowlist differs from v1")

    rows = require_array(policy["repositories"], "license policy repositories")
    require(
        rows == sorted(rows, key=lambda row: row["name_with_owner"].casefold()),
        "license policy repositories are not in canonical order",
    )
    licenses = {}
    for index, row in enumerate(rows):
        row = require_exact_fields(
            row, {"name_with_owner", "observed_spdx_id"}, f"license policy repository {index}"
        )
        name = require_string(row["name_with_owner"], f"license policy repository {index} name")
        spdx_id = require_string(row["observed_spdx_id"], f"license policy repository {name} SPDX ID")
        require("/" in name and name.count("/") == 1, f"invalid repository name: {name}")
        require(name not in licenses, f"duplicate license policy repository: {name}")
        licenses[name] = spdx_id
    require(set(licenses) == set(planned_repositories), "license policy repository set differs from sampling plan")
    return licenses


def validate_pull_request_link(pull_request, capture_case):
    case_id = pull_request["id"]
    captured_pull_request = capture_case["pull_request"]
    require(capture_case["id"] == case_id, f"{case_id} capture case ID differs")
    require(pull_request["capture_complete"] is True, f"{case_id} manifest capture is incomplete")
    require(captured_pull_request["head_matches_last_commit"] is True, f"{case_id} captured head is inconsistent")
    validate_optional_oid(pull_request["final_head_oid"], f"{case_id} final head OID")
    validate_optional_oid(pull_request["last_commit_oid"], f"{case_id} manifest last commit OID")
    validate_optional_oid(captured_pull_request["head_oid"], f"{case_id} captured head OID")
    validate_optional_oid(captured_pull_request["last_commit_oid"], f"{case_id} captured last commit OID")
    require(pull_request["final_head_oid"] == pull_request["last_commit_oid"], f"{case_id} manifest head differs")
    require(
        captured_pull_request["head_oid"] == captured_pull_request["last_commit_oid"],
        f"{case_id} captured head differs from its last commit",
    )
    require(pull_request["final_head_oid"] == captured_pull_request["head_oid"], f"{case_id} final head differs")
    require(pull_request["merged_at"] == captured_pull_request["merged_at"], f"{case_id} merge time differs")
    require(
        pull_request["repository"] == capture_case["repository"]["name_with_owner"],
        f"{case_id} repository differs",
    )
    author = capture_case["pull_request"]["author"]
    if author is None or author["actor_key"] is None:
        author_class = "unknown"
    elif author["typename"] == "User":
        author_class = "user"
    elif author["typename"] == "Bot":
        author_class = "bot"
    else:
        author_class = "other"
    require(pull_request["author_class"] == author_class, f"{case_id} author class differs")


def completed_review_candidates(capture_case, reviewer_key):
    case_id = capture_case["id"]
    dismissal_events = {}
    for event_index, event in enumerate(capture_case["timeline_events"]):
        event = require_object(event, f"{case_id} timeline event {event_index}")
        event_type = require_string(event["type"], f"{case_id} timeline event {event_index} type")
        if event_type != "ReviewDismissedEvent":
            continue
        event = require_exact_fields(
            event,
            {"created_at", "node_id", "previous_review_state", "review", "type"},
            f"{case_id} review dismissal event",
        )
        event_review = require_exact_fields(
            event["review"], CAPTURE_REVIEW_FIELDS, f"{case_id} dismissed event review"
        )
        review_id = require_string(event_review["node_id"], f"{case_id} dismissed review node ID")
        require(review_id not in dismissal_events, f"{case_id} repeats a dismissal for review {review_id}")
        require_string(event["node_id"], f"{case_id} dismissal event ID")
        require_string(event["previous_review_state"], f"{case_id} dismissal previous state")
        validate_timestamp(event["created_at"], f"{case_id} dismissal time")
        dismissal_events[review_id] = event

    candidates = []
    for review in capture_case["reviews"]:
        author = review["author"]
        if author["typename"] != "User" or author["actor_key"] != reviewer_key:
            continue
        completed_state = None
        dismissal_event_id = None
        if review["state"] in ("APPROVED", "CHANGES_REQUESTED"):
            require(
                review["node_id"] not in dismissal_events,
                f"{case_id} active completed review also has a dismissal event",
            )
            completed_state = review["state"]
        elif (
            review["state"] == "DISMISSED"
            and review["node_id"] in dismissal_events
            and dismissal_events[review["node_id"]]["previous_review_state"]
            in ("APPROVED", "CHANGES_REQUESTED")
        ):
            event = dismissal_events[review["node_id"]]
            require(event["review"] == review, f"{case_id} dismissal review snapshot differs")
            require(
                validate_timestamp(event["created_at"], f"{case_id} dismissal time")
                >= validate_timestamp(review["submitted_at"], f"{case_id} review submission time"),
                f"{case_id} dismissal predates its review",
            )
            completed_state = event["previous_review_state"]
            dismissal_event_id = event["node_id"]
        if completed_state is not None:
            candidates.append(
                {
                    "completed_state": completed_state,
                    "dismissal_event_id": dismissal_event_id,
                    "review": review,
                    "sort_key": (
                        validate_timestamp(review["submitted_at"], f"{case_id} review submission time"),
                        validate_database_id(review["database_id"], f"{case_id} review database ID"),
                    ),
                }
            )
    return sorted(candidates, key=lambda candidate: candidate["sort_key"])


def frame_row(pull_request, capture_case, pair, spdx_id, seed_bytes):
    case_id = require_string(pull_request["id"], "manifest case ID")
    reviewer_key = require_string(pair["reviewer_key"], f"{case_id} reviewer key")
    require(
        reviewer_key.startswith("actor-")
        and len(reviewer_key) == 30
        and all(character in "0123456789abcdef" for character in reviewer_key[6:]),
        f"{case_id} reviewer key is not pseudonymous",
    )
    pull_request_author = capture_case["pull_request"]["author"]
    if pull_request_author is not None and pull_request_author["actor_key"] is not None:
        require(
            pull_request_author["actor_key"] != reviewer_key,
            f"{case_id}/{reviewer_key} reviewer is the pull request author",
        )
    final_head_oid = pull_request["final_head_oid"]
    validate_optional_oid(final_head_oid, f"{case_id} final head OID")
    completed_candidates = completed_review_candidates(capture_case, reviewer_key)
    completed_sessions = require_integer(
        pair["completed_review_sessions"], f"{case_id}/{reviewer_key} completed review sessions"
    )
    require(completed_sessions >= 0, f"{case_id}/{reviewer_key} completed review sessions is negative")
    require(
        completed_sessions == len(completed_candidates),
        f"{case_id}/{reviewer_key} completed review session count differs from capture",
    )

    reasons = []
    checkpoint = pair["latest_completed_checkpoint"]
    output_checkpoint = None
    if checkpoint is None:
        require(not completed_candidates, f"{case_id}/{reviewer_key} omits a completed checkpoint")
        reasons.append("no_latest_completed_checkpoint")
    else:
        require(completed_candidates, f"{case_id}/{reviewer_key} checkpoint has no completed capture review")
        checkpoint = require_exact_fields(checkpoint, CHECKPOINT_FIELDS, f"{case_id}/{reviewer_key} checkpoint")
        review_id = require_string(checkpoint["review_id"], f"{case_id}/{reviewer_key} checkpoint review ID")
        commit_oid = checkpoint["commit_oid"]
        validate_optional_oid(commit_oid, f"{case_id}/{reviewer_key} checkpoint OID")
        reviews = [review for review in capture_case["reviews"] if review["node_id"] == review_id]
        require(len(reviews) == 1, f"{case_id}/{reviewer_key} checkpoint review join is not unique")
        review = require_exact_fields(reviews[0], CAPTURE_REVIEW_FIELDS, f"{case_id}/{reviewer_key} captured review")
        author = require_exact_fields(
            review["author"], {"actor_key", "typename"}, f"{case_id}/{reviewer_key} captured review author"
        )
        require(author["typename"] == "User", f"{case_id}/{reviewer_key} checkpoint author is not a User")
        require(author["actor_key"] == reviewer_key, f"{case_id}/{reviewer_key} checkpoint author differs")
        require(review["commit_oid"] == commit_oid, f"{case_id}/{reviewer_key} checkpoint OID differs from capture")
        require(review["state"] == checkpoint["current_state"], f"{case_id}/{reviewer_key} checkpoint state differs")
        require(
            checkpoint["current_state"] in ("APPROVED", "CHANGES_REQUESTED", "DISMISSED"),
            f"{case_id}/{reviewer_key} checkpoint current state is invalid",
        )
        require(
            checkpoint["completed_state"] in ("APPROVED", "CHANGES_REQUESTED"),
            f"{case_id}/{reviewer_key} checkpoint completed state is invalid",
        )
        require(
            review["submitted_at"] == checkpoint["submitted_at"],
            f"{case_id}/{reviewer_key} checkpoint submission time differs",
        )
        latest = completed_candidates[-1]
        require(latest["review"]["node_id"] == review_id, f"{case_id}/{reviewer_key} checkpoint is not latest")
        require(
            latest["completed_state"] == checkpoint["completed_state"],
            f"{case_id}/{reviewer_key} completed state differs from capture",
        )
        dismissed = checkpoint["current_state"] == "DISMISSED"
        require(checkpoint["dismissed"] is dismissed, f"{case_id}/{reviewer_key} dismissed flag differs")
        require(
            checkpoint["dismissal_event_id"] == latest["dismissal_event_id"],
            f"{case_id}/{reviewer_key} dismissal event differs",
        )
        for field in (
            "post_completed_review_force_push",
            "post_latest_checkpoint_force_push",
            "force_push_rereview",
        ):
            require(isinstance(checkpoint[field], bool), f"{case_id}/{reviewer_key} {field} must be boolean")
        if commit_oid is None:
            require(
                checkpoint["differs_from_final_head"] is None,
                f"{case_id}/{reviewer_key} drift flag must be null without a checkpoint OID",
            )
            reasons.append("checkpoint_oid_unavailable")
        elif final_head_oid is None:
            require(
                checkpoint["differs_from_final_head"] is None,
                f"{case_id}/{reviewer_key} drift flag must be null without a final head OID",
            )
            reasons.append("final_head_oid_unavailable")
        else:
            differs = commit_oid != final_head_oid
            require(
                checkpoint["differs_from_final_head"] is differs,
                f"{case_id}/{reviewer_key} checkpoint drift flag differs from OIDs",
            )
            if not differs:
                reasons.append("checkpoint_matches_final_head")
        output_checkpoint = {
            "commit_oid": commit_oid,
            "completed_state": checkpoint["completed_state"],
            "review_node_id": review_id,
            "state": checkpoint["current_state"],
        }

    if spdx_id not in ALLOWED_SPDX_IDS:
        reasons.append("repository_license_not_allowed")

    return {
        "case_id": case_id,
        "checkpoint": output_checkpoint,
        "disposition": "excluded" if reasons else "eligible_not_selected",
        "final_head_oid": final_head_oid,
        "pull_request": {
            "node_id": pull_request["node_id"],
            "number": pull_request["number"],
        },
        "reasons": reasons,
        "repository": pull_request["repository"],
        "repository_spdx_id": spdx_id,
        "reviewer_key": reviewer_key,
        "selection_digest": selection_digest(seed_bytes, case_id, reviewer_key),
    }


def selected_case(row):
    return {
        "case_id": row["case_id"],
        "checkpoint": copy.deepcopy(row["checkpoint"]),
        "final_head_oid": row["final_head_oid"],
        "pull_request": copy.deepcopy(row["pull_request"]),
        "repository": row["repository"],
        "repository_spdx_id": row["repository_spdx_id"],
        "reviewer_key": row["reviewer_key"],
        "selection_digest": row["selection_digest"],
    }


def assign_dispositions(rows, count):
    eligible = [row for row in rows if not row["reasons"]]
    eligible.sort(key=lambda row: (row["selection_digest"], row["case_id"], row["reviewer_key"]))
    representatives = []
    represented_cases = set()
    for row in eligible:
        if row["case_id"] in represented_cases:
            row["reasons"].append("lower_ranked_reviewer_same_pull_request")
        else:
            represented_cases.add(row["case_id"])
            representatives.append(row)

    require(
        count <= len(representatives),
        f"requested {count} transitions but only {len(representatives)} distinct eligible pull requests exist",
    )
    selected = representatives[:count]
    selected_ids = {(row["case_id"], row["reviewer_key"]) for row in selected}
    representative_ids = {(row["case_id"], row["reviewer_key"]) for row in representatives}
    for row in rows:
        identity = (row["case_id"], row["reviewer_key"])
        if identity in selected_ids:
            row["disposition"] = "selected"
        elif identity in representative_ids:
            row["disposition"] = "eligible_not_selected"
            row["reasons"].append("outside_requested_count")
        elif row["disposition"] != "excluded":
            row["disposition"] = "eligible_not_selected"
    return selected, len(eligible), len(representatives)


def build_plan(
    sampling_plan_path=DEFAULT_SAMPLING_PLAN,
    sample_path=DEFAULT_SAMPLE,
    capture_path=DEFAULT_CAPTURE,
    manifest_path=DEFAULT_MANIFEST,
    license_policy_path=DEFAULT_LICENSE_POLICY,
    count=30,
    seed_hex=None,
):
    require_integer(count, "requested count")
    require(count > 0, "requested count must be positive")
    paths = {
        "sampling_plan": Path(sampling_plan_path),
        "sample": Path(sample_path),
        "capture": Path(capture_path),
        "manifest": Path(manifest_path),
        "license_policy": Path(license_policy_path),
    }
    loaded = {name: load_json(path) for name, path in paths.items()}
    raws = {name: item[0] for name, item in loaded.items()}
    values = {name: item[1] for name, item in loaded.items()}
    sampling_plan = values["sampling_plan"]
    sample = values["sample"]
    capture = values["capture"]
    manifest = values["manifest"]
    policy = values["license_policy"]

    validate_headers(sampling_plan, sample, capture, manifest)
    validate_hash_chain(raws, sampling_plan, sample, capture, manifest)
    planned_repositories, capture_by_identity, manifest_by_identity = validate_pull_request_frame(
        sampling_plan, sample, capture, manifest
    )
    licenses = validate_license_policy(policy, planned_repositories)

    upstream_seed = sampling_plan["selection"]["seed_hex"]
    require(sample["selection"]["seed_hex"] == upstream_seed, "sample seed differs from sampling plan")
    validate_seed(upstream_seed)
    selected_seed = upstream_seed if seed_hex is None else seed_hex
    seed_bytes = validate_seed(selected_seed)

    rows = []
    for identity in sorted(manifest_by_identity):
        pull_request = manifest_by_identity[identity]
        capture_case = capture_by_identity[identity]
        repository = pull_request["repository"]
        validate_pull_request_link(pull_request, capture_case)
        reviewers = set()
        for pair in require_array(pull_request["reviewer_pairs"], f"{pull_request['id']} reviewer pairs"):
            pair = require_exact_fields(pair, REVIEWER_PAIR_FIELDS, f"{pull_request['id']} reviewer pair")
            reviewer_key = pair["reviewer_key"]
            require(reviewer_key not in reviewers, f"{pull_request['id']} repeats reviewer {reviewer_key}")
            reviewers.add(reviewer_key)
            rows.append(frame_row(pull_request, capture_case, pair, licenses[repository], seed_bytes))

    selected, eligible_pairs, eligible_pull_requests = assign_dispositions(rows, count)
    rows.sort(key=lambda row: (row["case_id"], row["reviewer_key"]))
    dispositions = Counter(row["disposition"] for row in rows)
    exclusion_reasons = Counter(
        reason for row in rows if row["disposition"] == "excluded" for reason in row["reasons"]
    )
    nonselection_reasons = Counter(
        reason for row in rows if row["disposition"] == "eligible_not_selected" for reason in row["reasons"]
    )

    inputs = {
        "capture": artifact_descriptor("capture.json", capture, raws["capture"]),
        "license_policy": {
            "artifact": "license-policy-v1.json",
            "observed_at": policy["observed_at"],
            "policy_version": policy["policy_version"],
            "schema": policy["schema"],
            "sha256": sha256_bytes(raws["license_policy"]),
        },
        "manifest": artifact_descriptor("manifest.json", manifest, raws["manifest"]),
        "sample": artifact_descriptor("sample.json", sample, raws["sample"]),
        "sampling_plan": artifact_descriptor("sampling-plan.json", sampling_plan, raws["sampling_plan"]),
    }
    plan = {
        "claim_boundary": {
            "dataset_kind": "post_hoc_diagnostic_transition_sample",
            "git_object_availability_guaranteed": False,
            "population_estimates_supported": False,
            "product_effectiveness_supported": False,
            "selection_claim": "deterministic selection from the exact bound Census artifacts and license policy only",
        },
        "dataset_version": PLAN_DATASET_VERSION,
        "inputs": inputs,
        "reviewer_pair_frame": rows,
        "schema": PLAN_SCHEMA,
        "selected_cases": [selected_case(row) for row in selected],
        "selection": {
            "algorithm": "sha256_domain_separated_rank_v1",
            "domain": SELECTION_DOMAIN.decode("ascii"),
            "eligible_pull_requests": eligible_pull_requests,
            "eligible_reviewer_pairs": eligible_pairs,
            "ordering": ["selection_digest", "case_id", "reviewer_key"],
            "per_pull_request_limit": 1,
            "ranking_preimage": "domain || 0x00 || seed_bytes || 0x00 || UTF-8(case_id) || 0x00 || UTF-8(reviewer_key)",
            "requested_count": count,
            "seed_hex": selected_seed,
            "selected_count": len(selected),
            "without_replacement": True,
        },
        "summary": {
            "dispositions": {key: dispositions[key] for key in sorted(dispositions)},
            "exclusion_reasons": {key: exclusion_reasons[key] for key in sorted(exclusion_reasons)},
            "nonselection_reasons": {key: nonselection_reasons[key] for key in sorted(nonselection_reasons)},
            "total_pull_requests": len(manifest_by_identity),
            "total_reviewer_pairs": len(rows),
        },
    }
    require(not contains_identity_key(plan), "transition plan contains a login or username field")
    return plan


def validate_plan_contract(plan):
    require_exact_fields(plan, PLAN_FIELDS, "transition plan")
    require(plan["schema"] == PLAN_SCHEMA, "unsupported transition plan schema")
    require(plan["dataset_version"] == PLAN_DATASET_VERSION, "unsupported transition plan dataset version")
    selection = require_exact_fields(plan["selection"], SELECTION_FIELDS, "transition plan selection")
    require_integer(selection["requested_count"], "transition plan requested count")
    validate_seed(selection["seed_hex"])


def verify_plan(
    plan_path,
    sampling_plan_path=DEFAULT_SAMPLING_PLAN,
    sample_path=DEFAULT_SAMPLE,
    capture_path=DEFAULT_CAPTURE,
    manifest_path=DEFAULT_MANIFEST,
    license_policy_path=DEFAULT_LICENSE_POLICY,
):
    raw, plan = load_json(Path(plan_path))
    require(raw == canonical_json_bytes(plan), "transition plan is not canonical JSON")
    validate_plan_contract(plan)
    expected = build_plan(
        sampling_plan_path=sampling_plan_path,
        sample_path=sample_path,
        capture_path=capture_path,
        manifest_path=manifest_path,
        license_policy_path=license_policy_path,
        count=plan["selection"]["requested_count"],
        seed_hex=plan["selection"]["seed_hex"],
    )
    require(plan == expected, "transition plan differs from deterministic regeneration")
    require(raw == canonical_json_bytes(expected), "transition plan bytes differ from deterministic regeneration")
    return {
        "plan_verified": True,
        "selected_cases": len(plan["selected_cases"]),
        "total_reviewer_pairs": len(plan["reviewer_pair_frame"]),
    }


def contains_identity_key(value):
    if isinstance(value, dict):
        return any("login" in key.casefold() or "username" in key.casefold() for key in value) or any(
            contains_identity_key(item) for item in value.values()
        )
    if isinstance(value, list):
        return any(contains_identity_key(item) for item in value)
    return False


def self_test():
    seed = bytes.fromhex("00" * 32)
    digest = selection_digest(seed, "case-a", "actor-000000000000000000000000")
    require(digest == "ebef3b0f1c71c1a2fa561321c7d8f5a07709623d0cfb53333b6420ea5f719441", "digest vector differs")

    first = build_plan()
    second = build_plan()
    require(first == second, "default selection is not deterministic")
    require(len(first["selected_cases"]) == 30, "default plan does not select 30 cases")
    require(len({case["case_id"] for case in first["selected_cases"]}) == 30, "default plan repeats a PR")
    require(not contains_identity_key(first), "default plan exposes a username field")

    protocol_raw, protocol = load_json(DEFAULT_EVALUATION_PROTOCOL)
    require(protocol_raw == canonical_json_bytes(protocol), "evaluation protocol is not canonical JSON")
    require(
        protocol["schema"] == "stratadiff-review-transition-evaluation-protocol-v1",
        "unsupported evaluation protocol schema",
    )
    require(protocol["protocol_version"] == "1.0.0", "unsupported evaluation protocol version")
    cohort = require_exact_fields(
        protocol["cohort"],
        {
            "expected_plan_sha256",
            "plan_dataset_version",
            "plan_schema",
            "replacement_policy",
            "selected_cases",
        },
        "evaluation protocol cohort",
    )
    require(cohort["plan_schema"] == PLAN_SCHEMA, "evaluation protocol plan schema differs")
    require(
        cohort["plan_dataset_version"] == PLAN_DATASET_VERSION,
        "evaluation protocol plan version differs",
    )
    require(cohort["selected_cases"] == 30, "evaluation protocol cohort size differs")
    require(cohort["replacement_policy"] == "forbidden", "evaluation protocol permits replacement")
    frozen_plan_sha256 = sha256_bytes(canonical_json_bytes(first))
    require(
        cohort["expected_plan_sha256"] == frozen_plan_sha256,
        "default plan bytes differ from the frozen evaluation protocol",
    )

    modified_manifest = copy.deepcopy(load_json(DEFAULT_MANIFEST)[1])
    modified_manifest["product_result"] = {"ignored": "first"}
    with tempfile.TemporaryDirectory(prefix="stratadiff-review-transition-") as directory:
        directory = Path(directory)
        manifest_path = directory / "manifest.json"
        write_json(manifest_path, modified_manifest)
        product_first = build_plan(manifest_path=manifest_path)
        modified_manifest["product_result"] = {"ignored": "second"}
        write_json(manifest_path, modified_manifest)
        product_second = build_plan(manifest_path=manifest_path)
        identity = lambda plan: [
            (case["case_id"], case["reviewer_key"], case["selection_digest"])
            for case in plan["selected_cases"]
        ]
        require(identity(product_first) == identity(product_second), "product result changed selection")

        plan_path = directory / "plan.json"
        write_json(plan_path, first)
        verify_plan(plan_path)
        tampered = copy.deepcopy(first)
        tampered["selected_cases"][0]["reviewer_key"] = "actor-ffffffffffffffffffffffff"
        write_json(plan_path, tampered)
        rejected = False
        try:
            verify_plan(plan_path)
        except ValueError:
            rejected = True
        require(rejected, "tampered selected identity was accepted")

    unavailable_rejected = False
    try:
        build_plan(count=first["selection"]["eligible_pull_requests"] + 1)
    except ValueError:
        unavailable_rejected = True
    require(unavailable_rejected, "selection silently relaxed an unavailable count")
    return {
        "default_selected_cases": 30,
        "digest_vector": digest,
        "frozen_plan_sha256": frozen_plan_sha256,
        "product_result_blind": True,
        "self_test": "passed",
        "tamper_rejected": True,
    }


def add_input_arguments(parser):
    parser.add_argument("--sampling-plan", type=Path, default=DEFAULT_SAMPLING_PLAN)
    parser.add_argument("--sample", type=Path, default=DEFAULT_SAMPLE)
    parser.add_argument("--capture", type=Path, default=DEFAULT_CAPTURE)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--license-policy", type=Path, default=DEFAULT_LICENSE_POLICY)


def parse_arguments():
    parser = argparse.ArgumentParser(description="Build and verify the ReviewTransition-30 selection plan")
    subparsers = parser.add_subparsers(dest="command", required=True)

    select_parser = subparsers.add_parser("select", help="build a deterministic transition plan")
    add_input_arguments(select_parser)
    select_parser.add_argument("--count", type=int, default=30)
    select_parser.add_argument("--seed-hex")
    select_parser.add_argument("--output", type=Path, required=True)

    verify_parser = subparsers.add_parser("verify", help="regenerate and verify a saved plan")
    add_input_arguments(verify_parser)
    verify_parser.add_argument("--plan", type=Path, required=True)

    subparsers.add_parser("self-test", help="run offline selector and tamper tests")
    return parser.parse_args()


def main():
    arguments = parse_arguments()
    if arguments.command == "select":
        plan = build_plan(
            sampling_plan_path=arguments.sampling_plan,
            sample_path=arguments.sample,
            capture_path=arguments.capture,
            manifest_path=arguments.manifest,
            license_policy_path=arguments.license_policy,
            count=arguments.count,
            seed_hex=arguments.seed_hex,
        )
        write_json(arguments.output, plan)
        result = {
            "eligible_pull_requests": plan["selection"]["eligible_pull_requests"],
            "output": str(arguments.output.resolve()),
            "selected_cases": len(plan["selected_cases"]),
        }
    elif arguments.command == "verify":
        result = verify_plan(
            arguments.plan,
            sampling_plan_path=arguments.sampling_plan,
            sample_path=arguments.sample,
            capture_path=arguments.capture,
            manifest_path=arguments.manifest,
            license_policy_path=arguments.license_policy,
        )
    elif arguments.command == "self-test":
        result = self_test()
    else:
        raise ValueError(f"unsupported command: {arguments.command}")
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main()
