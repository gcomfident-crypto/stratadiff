#!/usr/bin/env python3
"""Collect and evaluate Review Churn Census v1 with the GitHub GraphQL API."""

from __future__ import annotations

import argparse
from datetime import datetime, timedelta, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_PLAN = ROOT / "benchmarks/review-churn-census-v1/sampling-plan.json"
DEFAULT_OUTPUT_DIR = ROOT / "target/review-churn-census-v1"
DEFAULT_SAMPLE = DEFAULT_OUTPUT_DIR / "sample.json"
DEFAULT_CAPTURE = DEFAULT_OUTPUT_DIR / "capture.json"
DEFAULT_MANIFEST = DEFAULT_OUTPUT_DIR / "manifest.json"
DEFAULT_AGGREGATE = DEFAULT_OUTPUT_DIR / "aggregate.json"

PLAN_SCHEMA = "stratadiff-review-churn-census-sampling-plan-v1"
SAMPLE_SCHEMA = "stratadiff-review-churn-census-sample-v1"
CAPTURE_SCHEMA = "stratadiff-review-churn-census-capture-v1"
MANIFEST_SCHEMA = "stratadiff-review-churn-census-manifest-v1"
AGGREGATE_SCHEMA = "stratadiff-review-churn-census-aggregate-v1"
AUDIT_SCHEMA = "stratadiff-review-memory-audit-v1"
DATASET_VERSION = "1.0.0"
TOOL_VERSION = "0.2.0"

SELECTION_ALGORITHM = "sha256_v1"
SELECTION_ALGORITHM_VERSION = "1"
FROZEN_SELECTION_SEED = "647bc818d8ee4d313d6735cf7c2e8d985e367a0fe1b2fe11c17aca3e15de491f"
FROZEN_PLAN_SHA256 = "cc7daf7e74590aa4b1c7afb67dfa241244fe0aa917d6812028785eb5d29d8e1e"
FROZEN_REPOSITORIES = (
    ("github", "gh-stack"),
    ("PostHog", "posthog"),
    ("microsoft", "vscode"),
    ("kubernetes", "kubernetes"),
    ("rust-lang", "rust"),
    ("home-assistant", "core"),
    ("grafana", "grafana"),
    ("vercel", "next.js"),
    ("elastic", "elasticsearch"),
    ("dotnet", "runtime"),
)
MAX_JSON_BYTES = 128 * 1024 * 1024
MAX_GRAPHQL_RESPONSE_BYTES = 32 * 1024 * 1024
GRAPHQL_TIMEOUT_SECONDS = 120
MAX_SEARCH_RESULTS_PER_SHARD = 1_000
MAX_CONNECTION_PAGES = 100
OID_PATTERN = re.compile(r"^[0-9a-f]{40}$")
TIMESTAMP_PATTERN = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$"
)
REPOSITORY_COMPONENT_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+$")

METRIC_IDS = (
    "formal_peer_reviewed_pr_rate",
    "completed_review_pr_rate",
    "checkpoint_oid_observability_rate",
    "checkpoint_pair_head_drift_rate",
    "completed_review_pair_post_force_push_rate",
    "checkpoint_pair_drift_without_observed_force_push_rate",
    "stranded_reviewer_pr_rate",
    "multi_round_completed_review_pr_rate",
    "completed_review_dismissal_pr_rate",
    "commented_only_pair_share",
    "commented_newer_commit_candidate_pair_rate",
    "completed_review_pair_force_push_rereview_rate",
    "bot_review_session_share",
)
AUDIT_METRIC_IDS = METRIC_IDS[:7]
AUDIT_SELECTION_METHOD = "latest_merged_at_desc_v1"
AUDIT_MINIMUM_OID_COVERAGE_BASIS_POINTS = 9_000

FORMAL_STATES = ("APPROVED", "CHANGES_REQUESTED")
REVIEW_STATES = ("APPROVED", "CHANGES_REQUESTED", "COMMENTED", "DISMISSED", "PENDING")


class CensusError(RuntimeError):
    """A transparent input, API, or invariant failure."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise CensusError(message)


def require_exact_keys(value: dict[str, object], expected: set[str], label: str) -> None:
    observed = set(value)
    missing = sorted(expected - observed)
    unknown = sorted(observed - expected)
    require(not missing and not unknown, f"{label} keys differ: missing={missing}, unknown={unknown}")


def progress(message: str) -> None:
    print(f"review-churn-census: {message}", file=sys.stderr, flush=True)


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


def exact_json_equal(left: object, right: object) -> bool:
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return set(left) == set(right) and all(
            exact_json_equal(left[key], right[key]) for key in left
        )
    if isinstance(left, list):
        return len(left) == len(right) and all(
            exact_json_equal(left_item, right_item)
            for left_item, right_item in zip(left, right)
        )
    return left == right


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def read_json(path: Path) -> tuple[bytes, object]:
    payload = path.read_bytes()
    require(len(payload) <= MAX_JSON_BYTES, f"JSON input exceeds {MAX_JSON_BYTES} bytes: {path}")
    return payload, json.loads(payload, object_pairs_hook=unique_json_object)


def atomic_write(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        os.fchmod(descriptor, 0o600)
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


def write_json(path: Path, value: object) -> None:
    atomic_write(path, canonical_json(value))


def require_object(value: object, label: str) -> dict[str, object]:
    require(isinstance(value, dict), f"{label} must be an object")
    return value


def require_array(value: object, label: str) -> list[object]:
    require(isinstance(value, list), f"{label} must be an array")
    return value


def require_string(value: object, label: str) -> str:
    require(isinstance(value, str) and value != "", f"{label} must be a non-empty string")
    return value


def require_bool(value: object, label: str) -> bool:
    require(type(value) is bool, f"{label} must be a boolean")
    return value


def require_int(value: object, label: str, minimum: int = 0) -> int:
    require(type(value) is int and value >= minimum, f"{label} must be an integer >= {minimum}")
    return value


def parse_utc_timestamp(value: object, label: str) -> datetime:
    timestamp = require_string(value, label)
    require(TIMESTAMP_PATTERN.fullmatch(timestamp) is not None, f"{label} must be an RFC3339 UTC timestamp")
    parsed = datetime.fromisoformat(timestamp[:-1] + "+00:00")
    require(parsed.tzinfo == timezone.utc, f"{label} must use UTC")
    return parsed


def now_timestamp() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def format_utc_timestamp(value: datetime) -> str:
    require(value.tzinfo == timezone.utc, "timestamp must use UTC")
    return value.isoformat().replace("+00:00", "Z")


def validate_oid(value: object, label: str, *, nullable: bool = False) -> str | None:
    if nullable and value is None:
        return None
    oid = require_string(value, label)
    require(OID_PATTERN.fullmatch(oid) is not None, f"{label} must be a lowercase SHA-1 object ID")
    return oid


def validate_repository_component(value: object, label: str) -> str:
    component = require_string(value, label)
    require(REPOSITORY_COMPONENT_PATTERN.fullmatch(component) is not None, f"invalid {label}: {component}")
    return component


def audit_repository_argument(value: str) -> str:
    components = value.split("/")
    if len(value) > 202 or len(components) != 2 or any(
        REPOSITORY_COMPONENT_PATTERN.fullmatch(component) is None
        for component in components
    ):
        raise argparse.ArgumentTypeError("repository must be in OWNER/REPO form")
    return value


def audit_hostname_argument(value: str) -> str:
    if (
        len(value) > 253
        or not value
        or value.endswith(".")
        or any(
            not label
            or len(label) > 63
            or re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?", label)
            is None
            for label in value.split(".")
        )
    ):
        raise argparse.ArgumentTypeError("hostname must be a DNS hostname without a scheme or port")
    return value.lower()


def audit_bounded_integer(value: str, label: str, maximum: int) -> int:
    try:
        parsed = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError(f"{label} must be an integer") from error
    if not 1 <= parsed <= maximum:
        raise argparse.ArgumentTypeError(f"{label} must be between 1 and {maximum}")
    return parsed


def audit_limit_argument(value: str) -> int:
    return audit_bounded_integer(value, "limit", 100)


def audit_days_argument(value: str) -> int:
    return audit_bounded_integer(value, "days", 365)


def audit_end_exclusive_argument(value: str) -> str:
    try:
        parse_utc_timestamp(value, "end-exclusive")
    except (CensusError, ValueError) as error:
        raise argparse.ArgumentTypeError(str(error)) from error
    return value


def validate_plan(plan: object) -> dict[str, object]:
    document = require_object(plan, "sampling plan")
    require(
        sha256_bytes(canonical_json(document)) == FROZEN_PLAN_SHA256,
        "sampling plan differs from the frozen v1 document",
    )
    require_exact_keys(
        document,
        {
            "schema",
            "dataset_version",
            "name",
            "panel_kind",
            "method_status",
            "prior_evidence_disclosure",
            "claim_boundary",
            "target_population",
            "merged_at_window",
            "target_pull_requests_per_repository",
            "selection",
            "repositories",
            "actor_policy",
            "formal_review_policy",
            "checkpoint_policy",
            "event_policy",
            "metrics",
            "analysis",
            "decision_thresholds",
            "privacy",
            "bias_register",
            "frame_construction",
        },
        "sampling plan",
    )
    require(document["schema"] == PLAN_SCHEMA, "unsupported sampling plan schema")
    require(document["dataset_version"] == DATASET_VERSION, "unsupported sampling plan dataset version")
    require(document["name"] == "Review Churn Census v1", "unexpected sampling plan name")
    require(
        document["panel_kind"] == "pilot_informed_prospective_randomized_target_segment_panel",
        "unsupported sampling panel kind",
    )
    method_status = require_object(document["method_status"], "method_status")
    require_exact_keys(
        method_status,
        {
            "amendment_policy",
            "designation",
            "frozen_on",
            "outcome_naive_preregistration",
            "prospective_boundary",
            "status",
        },
        "method_status",
    )
    require(
        method_status["designation"] == "pilot-informed prospective randomized panel"
        and method_status["status"]
        == "frozen_after_pilot_disclosure_before_randomized_panel_sampling"
        and method_status["frozen_on"] == "2026-09-05"
        and method_status["outcome_naive_preregistration"] is False,
        "v1 method-status disclosure differs",
    )
    require_string(method_status["amendment_policy"], "method_status.amendment_policy")
    require_string(method_status["prospective_boundary"], "method_status.prospective_boundary")
    prior = require_object(document["prior_evidence_disclosure"], "prior_evidence_disclosure")
    require(
        prior["disclosure_status"] == "frozen_before_v1_randomized_panel_sampling",
        "v1 prior-evidence disclosure status differs",
    )
    probes = require_array(prior["probes"], "prior_evidence_disclosure.probes")
    require(
        [require_object(probe, "prior probe")["id"] for probe in probes]
        == ["latest25_four_repository_probe", "newest50_four_repository_convenience_probe"],
        "v1 prior probes differ",
    )
    boundary = require_object(document["claim_boundary"], "claim_boundary")
    require_exact_keys(
        boundary,
        {
            "causal_product_effects_supported",
            "github_population_estimates_supported",
            "independent_confirmatory_test_supported",
            "interpretation",
            "issue_recall_or_semantic_safety_supported",
            "market_size_estimates_supported",
            "ordinary_push_identification_supported",
            "panel_period_descriptive_estimates_supported",
            "reviewer_time_savings_supported",
            "willingness_to_pay_supported",
        },
        "claim_boundary",
    )
    require_bool(
        boundary["github_population_estimates_supported"],
        "claim_boundary.github_population_estimates_supported",
    )
    require(
        boundary["github_population_estimates_supported"] is False,
        "GitHub population estimates must be forbidden",
    )
    for key in (
        "causal_product_effects_supported",
        "independent_confirmatory_test_supported",
        "issue_recall_or_semantic_safety_supported",
        "market_size_estimates_supported",
        "ordinary_push_identification_supported",
        "reviewer_time_savings_supported",
        "willingness_to_pay_supported",
    ):
        require_bool(boundary[key], f"claim_boundary.{key}")
        require(boundary[key] is False, f"claim_boundary.{key} must remain false")
    require(
        boundary["panel_period_descriptive_estimates_supported"] is True,
        "panel-period descriptive estimates must remain enabled",
    )
    require_string(boundary["interpretation"], "claim_boundary.interpretation")

    window = require_object(document["merged_at_window"], "merged_at_window")
    require_exact_keys(window, {"start", "end_exclusive", "field", "timezone"}, "merged_at_window")
    start = parse_utc_timestamp(window["start"], "merged_at_window.start")
    end = parse_utc_timestamp(window["end_exclusive"], "merged_at_window.end_exclusive")
    require(start < end, "merged_at_window must be non-empty")
    require(
        start.time() == datetime.min.time() and end.time() == datetime.min.time(),
        "merged_at_window boundaries must be UTC midnights",
    )
    require(end - start == timedelta(days=90), "v1 merged_at_window must span exactly 90 days")
    require(
        window["start"] == "2026-06-03T00:00:00Z"
        and window["end_exclusive"] == "2026-09-01T00:00:00Z",
        "v1 merge window differs from the frozen dates",
    )
    require(window["field"] == "PullRequest.mergedAt", "v1 merge-window field differs")
    require(window["timezone"] == "UTC", "v1 merge-window timezone differs")

    target = require_int(
        document["target_pull_requests_per_repository"],
        "target_pull_requests_per_repository",
        1,
    )
    require(target == 50, "v1 target_pull_requests_per_repository must equal 50")
    selection = require_object(document["selection"], "selection")
    require_exact_keys(
        selection,
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
        "selection",
    )
    require(selection["algorithm"] == SELECTION_ALGORITHM, "unsupported selection algorithm")
    require(
        selection["algorithm_version"] == SELECTION_ALGORITHM_VERSION,
        "unsupported selection algorithm version",
    )
    seed_hex = require_string(selection["seed_hex"], "selection.seed_hex")
    require(
        len(seed_hex) == 64 and all(character in "0123456789abcdef" for character in seed_hex),
        "selection.seed_hex must be 64 lowercase hexadecimal characters",
    )
    require(seed_hex == FROZEN_SELECTION_SEED, "v1 selection seed differs")
    require(selection["without_replacement"] is True, "v1 sampling must be without replacement")

    actor_policy = require_object(document["actor_policy"], "actor_policy")
    require(actor_policy["human_author_typenames"] == ["User"], "v1 human actor policy must be ['User']")
    require(actor_policy["bot_author_typenames"] == ["Bot"], "v1 bot actor policy must be ['Bot']")
    require_bool(actor_policy["exclude_pull_request_author"], "actor_policy.exclude_pull_request_author")
    require(actor_policy["exclude_pull_request_author"] is True, "v1 must exclude pull request authors")
    require_string(actor_policy["missing_author_class"], "actor_policy.missing_author_class")

    checkpoint = require_object(document["checkpoint_policy"], "checkpoint_policy")
    require(checkpoint["completed_states"] == ["APPROVED", "CHANGES_REQUESTED"], "unsupported completed checkpoint states")
    require(checkpoint["dismissed_previous_states"] == ["APPROVED", "CHANGES_REQUESTED"], "unsupported dismissed checkpoint states")
    require(checkpoint["commented_is_completed"] is False, "COMMENTED must not be completed")
    require(checkpoint["completed_pair_requires_commit_oid"] is False, "semantic completed pairs must retain missing commit OIDs")
    require(checkpoint["latest_missing_commit_fallback"] == "forbidden", "latest missing commit fallback must be forbidden")

    repositories = require_array(document["repositories"], "repositories")
    require(len(repositories) == len(FROZEN_REPOSITORIES), "v1 repository count differs")
    identities: set[str] = set()
    for index, repository_value in enumerate(repositories):
        repository = require_object(repository_value, f"repositories[{index}]")
        require_exact_keys(repository, {"owner", "name"}, f"repositories[{index}]")
        owner = validate_repository_component(repository["owner"], f"repositories[{index}].owner")
        name = validate_repository_component(repository["name"], f"repositories[{index}].name")
        require(
            (owner, name) == FROZEN_REPOSITORIES[index],
            f"repositories[{index}] differs from the frozen panel",
        )
        identity = f"{owner}/{name}".casefold()
        require(identity not in identities, f"duplicate repository: {owner}/{name}")
        identities.add(identity)

    metrics = require_array(document["metrics"], "metrics")
    observed_metric_ids = []
    for index, metric_value in enumerate(metrics):
        metric = require_object(metric_value, f"metrics[{index}]")
        require_exact_keys(metric, {"id", "unit", "numerator", "denominator", "purpose"}, f"metrics[{index}]")
        observed_metric_ids.append(require_string(metric["id"], f"metrics[{index}].id"))
        require_string(metric["unit"], f"metrics[{index}].unit")
        require_string(metric["numerator"], f"metrics[{index}].numerator")
        require_string(metric["denominator"], f"metrics[{index}].denominator")
        require_string(metric["purpose"], f"metrics[{index}].purpose")
    require(tuple(observed_metric_ids) == METRIC_IDS, "sampling plan metric order or IDs differ from v1")
    thresholds = require_object(document["decision_thresholds"], "decision_thresholds")
    require_exact_keys(
        thresholds,
        {
            "min_sampled_prs",
            "min_repositories_at_target",
            "min_completed_reviewed_prs",
            "max_capture_failures",
            "min_signal_denominator",
            "min_head_oid_observability_bps",
            "force_push_wedge_bps",
            "all_round_review_continuity_bps",
            "commented_partial_attention_bps",
            "wilson_confidence_basis_points",
            "signals",
            "non_decisions",
        },
        "decision_thresholds",
    )
    for key in (
        "min_sampled_prs",
        "min_repositories_at_target",
        "min_completed_reviewed_prs",
        "max_capture_failures",
        "min_signal_denominator",
        "min_head_oid_observability_bps",
        "force_push_wedge_bps",
        "all_round_review_continuity_bps",
        "commented_partial_attention_bps",
        "wilson_confidence_basis_points",
    ):
        require_int(thresholds[key], f"decision_thresholds.{key}")
    require(thresholds["min_sampled_prs"] == 400, "v1 min_sampled_prs must equal 400")
    require(thresholds["min_repositories_at_target"] == 8, "v1 min_repositories_at_target must equal 8")
    require(thresholds["min_completed_reviewed_prs"] == 200, "v1 min_completed_reviewed_prs must equal 200")
    require(thresholds["min_signal_denominator"] == 100, "v1 min_signal_denominator must equal 100")
    require(thresholds["min_head_oid_observability_bps"] == 9000, "v1 OID observability gate must equal 9000 bps")
    require(thresholds["wilson_confidence_basis_points"] == 9500, "v1 Wilson confidence must equal 9500 bps")
    require(thresholds["max_capture_failures"] == 0, "v1 permits no capture failures")
    require(thresholds["force_push_wedge_bps"] == 1000, "v1 force-push threshold differs")
    require(thresholds["all_round_review_continuity_bps"] == 2000, "v1 all-round threshold differs")
    require(thresholds["commented_partial_attention_bps"] == 1500, "v1 COMMENTED threshold differs")
    require_string(thresholds["non_decisions"], "decision_thresholds.non_decisions")
    signals = require_object(thresholds["signals"], "decision_thresholds.signals")
    require(
        exact_json_equal(
            signals,
            {
                "force_push_wedge": {
                    "metric_id": "completed_review_pair_post_force_push_rate",
                    "requires_head_oid_observability_gate": False,
                    "threshold_basis_points": 1000,
                },
                "all_round_review_continuity": {
                    "metric_id": "checkpoint_pair_drift_without_observed_force_push_rate",
                    "requires_head_oid_observability_gate": True,
                    "threshold_basis_points": 2000,
                },
                "commented_partial_attention": {
                    "metric_id": "commented_newer_commit_candidate_pair_rate",
                    "requires_head_oid_observability_gate": False,
                    "threshold_basis_points": 1500,
                },
            },
        ),
        "v1 product-signal mapping differs",
    )
    return document


def selection_digest(seed_hex: str, name_with_owner: str, number: int) -> str:
    material = (
        bytes.fromhex(seed_hex)
        + b"\0"
        + name_with_owner.casefold().encode("utf-8")
        + b"\0"
        + str(number).encode("ascii")
    )
    return sha256_bytes(material)


def select_candidates(
    candidates: list[dict[str, object]], seed_hex: str, name_with_owner: str, count: int
) -> tuple[list[dict[str, object]], list[int]]:
    ranked = []
    for candidate in candidates:
        number = require_int(candidate["number"], "candidate.number", 1)
        enriched = dict(candidate)
        enriched["selection_digest"] = selection_digest(seed_hex, name_with_owner, number)
        ranked.append(enriched)
    ranked.sort(key=lambda candidate: (candidate["selection_digest"], candidate["number"]))
    selected = ranked[:count]
    selected_numbers = [int(candidate["number"]) for candidate in selected]
    inventory = sorted(ranked, key=lambda candidate: int(candidate["number"]))
    return inventory, selected_numbers


def select_latest_candidates(
    candidates: list[dict[str, object]], limit: int
) -> list[dict[str, object]]:
    require(1 <= limit <= 100, "audit limit must be between 1 and 100")
    return sorted(
        candidates,
        key=lambda candidate: (
            parse_utc_timestamp(candidate["merged_at"], "candidate.merged_at"),
            require_int(candidate["number"], "candidate.number", 1),
        ),
        reverse=True,
    )[:limit]


SEARCH_QUERY = """
query ReviewChurnSearch($query: String!, $cursor: String) {
  search(query: $query, type: ISSUE, first: 100, after: $cursor) {
    issueCount
    pageInfo { hasNextPage endCursor }
    nodes {
      ... on PullRequest {
        id number state mergedAt
        repository { nameWithOwner }
      }
    }
  }
  rateLimit { cost remaining resetAt }
}
"""

CAPTURE_QUERY = """
query ReviewChurnCapture($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    id nameWithOwner url
    pullRequest(number: $number) {
      id number mergedAt headRefOid
      commits(last: 1) { totalCount nodes { commit { oid } } }
      author { __typename login }
      reviews(first: 100) {
        totalCount
        pageInfo { hasNextPage endCursor }
        nodes {
          id fullDatabaseId state submittedAt
          author { __typename login }
          commit { oid }
          comments(first: 1) { totalCount }
        }
      }
      timelineItems(first: 100, itemTypes: [HEAD_REF_FORCE_PUSHED_EVENT, REVIEW_DISMISSED_EVENT]) {
        totalCount
        pageInfo { hasNextPage endCursor }
        nodes {
          __typename
          ... on HeadRefForcePushedEvent {
            id createdAt beforeCommit { oid } afterCommit { oid }
          }
          ... on ReviewDismissedEvent {
            id createdAt previousReviewState
            review {
              id fullDatabaseId state submittedAt
              author { __typename login }
              commit { oid }
              comments(first: 1) { totalCount }
            }
          }
        }
      }
    }
  }
  rateLimit { cost remaining resetAt }
}
"""

REVIEWS_PAGE_QUERY = """
query ReviewChurnReviews($owner: String!, $name: String!, $number: Int!, $cursor: String!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviews(first: 100, after: $cursor) {
        totalCount
        pageInfo { hasNextPage endCursor }
        nodes {
          id fullDatabaseId state submittedAt
          author { __typename login }
          commit { oid }
          comments(first: 1) { totalCount }
        }
      }
    }
  }
  rateLimit { cost remaining resetAt }
}
"""

TIMELINE_PAGE_QUERY = """
query ReviewChurnTimeline($owner: String!, $name: String!, $number: Int!, $cursor: String!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      timelineItems(first: 100, after: $cursor, itemTypes: [HEAD_REF_FORCE_PUSHED_EVENT, REVIEW_DISMISSED_EVENT]) {
        totalCount
        pageInfo { hasNextPage endCursor }
        nodes {
          __typename
          ... on HeadRefForcePushedEvent {
            id createdAt beforeCommit { oid } afterCommit { oid }
          }
          ... on ReviewDismissedEvent {
            id createdAt previousReviewState
            review {
              id fullDatabaseId state submittedAt
              author { __typename login }
              commit { oid }
              comments(first: 1) { totalCount }
            }
          }
        }
      }
    }
  }
  rateLimit { cost remaining resetAt }
}
"""


class GithubGraphQL:
    def __init__(self, executable: str = "gh", hostname: str = "github.com") -> None:
        self.executable = executable
        self.hostname = hostname
        self.calls = 0
        self.minimum_remaining: int | None = None
        self.last_reset_at: str | None = None

    def call(self, query: str, variables: dict[str, object]) -> dict[str, object]:
        request = canonical_json({"query": query, "variables": variables})
        environment = os.environ.copy()
        environment.pop("GH_DEBUG", None)
        environment.pop("GH_TRACE", None)
        environment["GH_PROMPT_DISABLED"] = "1"
        environment["GH_PAGER"] = "cat"
        environment["PAGER"] = "cat"
        environment["NO_COLOR"] = "1"
        environment["LC_ALL"] = "C"
        result = subprocess.run(
            [
                self.executable,
                "api",
                "--hostname",
                self.hostname,
                "graphql",
                "--input",
                "-",
            ],
            input=request,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=GRAPHQL_TIMEOUT_SECONDS,
            env=environment,
        )
        self.calls += 1
        if result.returncode != 0:
            diagnostic = result.stderr.decode("utf-8", errors="replace").strip()
            if not diagnostic:
                diagnostic = result.stdout.decode("utf-8", errors="replace").strip()
            raise CensusError(
                f"gh api graphql failed with exit status {result.returncode}: {diagnostic}"
            )
        require(
            len(result.stdout) <= MAX_GRAPHQL_RESPONSE_BYTES,
            f"GraphQL response exceeds {MAX_GRAPHQL_RESPONSE_BYTES} bytes",
        )
        envelope = json.loads(result.stdout, object_pairs_hook=unique_json_object)
        envelope_object = require_object(envelope, "GraphQL response")
        if "errors" in envelope_object:
            raise CensusError(
                "GitHub GraphQL returned errors: "
                + json.dumps(envelope_object["errors"], ensure_ascii=False, sort_keys=True)
            )
        data = require_object(envelope_object["data"], "GraphQL response.data")
        rate = require_object(data["rateLimit"], "GraphQL response.data.rateLimit")
        remaining = require_int(rate["remaining"], "rateLimit.remaining")
        self.minimum_remaining = (
            remaining
            if self.minimum_remaining is None
            else min(self.minimum_remaining, remaining)
        )
        self.last_reset_at = require_string(rate["resetAt"], "rateLimit.resetAt")
        return data

    def acquisition(self) -> dict[str, object]:
        return {
            "graphql_calls": self.calls,
            "minimum_rate_limit_remaining": self.minimum_remaining,
            "last_rate_limit_reset_at": self.last_reset_at,
        }


def opaque_actor_key(case_id: str, login: str) -> str:
    digest = sha256_bytes((case_id + "\0" + login.casefold()).encode("utf-8"))
    return "actor-" + digest[:24]


def normalize_actor(
    actor_value: object, label: str, case_id: str
) -> dict[str, object] | None:
    if actor_value is None:
        return None
    actor = require_object(actor_value, label)
    typename = require_string(actor["__typename"], f"{label}.__typename")
    login = actor["login"]
    require(login is None or isinstance(login, str), f"{label}.login must be a string or null")
    return {
        "typename": typename,
        "actor_key": None
        if login is None or login == ""
        else opaque_actor_key(case_id, login),
    }


def normalize_candidate(raw_value: object, expected_repository: str) -> dict[str, object]:
    raw = require_object(raw_value, "pull request")
    number = require_int(raw["number"], "pull request number", 1)
    merged_at = require_string(raw["mergedAt"], f"PR #{number} mergedAt")
    parse_utc_timestamp(merged_at, f"PR #{number} mergedAt")
    repository = require_object(raw["repository"], f"PR #{number} repository")
    require(
        require_string(repository["nameWithOwner"], "repository.nameWithOwner").casefold()
        == expected_repository.casefold(),
        f"search returned PR #{number} from another repository",
    )
    require(raw["state"] == "MERGED", f"search returned non-merged PR #{number}")
    return {
        "node_id": require_string(raw["id"], f"PR #{number} id"),
        "number": number,
        "merged_at": merged_at,
    }


def normalize_captured_pull_request(
    raw_value: object, expected_number: int, case_id: str
) -> dict[str, object]:
    raw = require_object(raw_value, "pull request")
    number = require_int(raw["number"], "pull request number", 1)
    require(number == expected_number, "GitHub returned a different pull request number")
    merged_at = require_string(raw["mergedAt"], f"PR #{number} mergedAt")
    parse_utc_timestamp(merged_at, f"PR #{number} mergedAt")
    head_oid = validate_oid(raw["headRefOid"], f"PR #{number} headRefOid", nullable=True)
    commits = require_object(raw["commits"], f"PR #{number} commits")
    commit_count = require_int(commits["totalCount"], f"PR #{number} commits.totalCount")
    commit_nodes = require_array(commits["nodes"], f"PR #{number} commits.nodes")
    require(len(commit_nodes) <= 1, f"PR #{number} commits(last:1) returned multiple nodes")
    last_commit_oid = None
    if commit_nodes:
        commit_node = require_object(commit_nodes[0], f"PR #{number} commits.nodes[0]")
        commit = commit_node["commit"]
        if commit is not None:
            last_commit_oid = validate_oid(
                require_object(commit, f"PR #{number} last commit")["oid"],
                f"PR #{number} last commit oid",
            )
    if head_oid is not None and last_commit_oid is not None:
        require(head_oid == last_commit_oid, f"PR #{number} headRefOid differs from commits(last:1)")
    return {
        "node_id": require_string(raw["id"], f"PR #{number} id"),
        "number": number,
        "merged_at": merged_at,
        "head_oid": head_oid,
        "last_commit_oid": last_commit_oid,
        "commit_count": commit_count,
        "head_matches_last_commit": None
        if head_oid is None or last_commit_oid is None
        else True,
        "author": normalize_actor(raw["author"], f"PR #{number} author", case_id),
    }


def search_repository_candidates(
    api: GithubGraphQL,
    owner: str,
    name: str,
    start: datetime,
    end: datetime,
) -> list[dict[str, object]]:
    name_with_owner = f"{owner}/{name}"
    candidates: dict[int, dict[str, object]] = {}
    first_day = start.date()
    last_day = (end - timedelta(microseconds=1)).date()
    shards = [(first_day, last_day)]
    while shards:
        shard_start, shard_end = shards.pop(0)
        date_range = f"{shard_start.isoformat()}..{shard_end.isoformat()}"
        search = f"repo:{name_with_owner} is:pr is:merged merged:{date_range} sort:created-asc"
        cursor: str | None = None
        pages = 0
        observed_for_shard = 0
        issue_count: int | None = None
        while True:
            data = api.call(SEARCH_QUERY, {"query": search, "cursor": cursor})
            connection = require_object(data["search"], f"search {name_with_owner} {date_range}")
            current_issue_count = require_int(connection["issueCount"], "search.issueCount")
            if issue_count is None:
                issue_count = current_issue_count
                if issue_count > MAX_SEARCH_RESULTS_PER_SHARD:
                    require(
                        shard_start < shard_end,
                        f"{name_with_owner} {date_range} has {issue_count} merged PRs; a single UTC day exceeds GitHub Search's 1000-result limit",
                    )
                    span_days = (shard_end - shard_start).days
                    left_end = shard_start + timedelta(days=span_days // 2)
                    right_start = left_end + timedelta(days=1)
                    shards.insert(0, (right_start, shard_end))
                    shards.insert(0, (shard_start, left_end))
                    break
            else:
                require(current_issue_count == issue_count, f"search result count changed while paging {name_with_owner} {date_range}")
            require(issue_count <= MAX_SEARCH_RESULTS_PER_SHARD, "internal search shard was not partitioned")
            nodes = require_array(connection["nodes"], "search.nodes")
            observed_for_shard += len(nodes)
            for raw in nodes:
                candidate = normalize_candidate(raw, name_with_owner)
                merged = parse_utc_timestamp(candidate["merged_at"], f"PR #{candidate['number']} merged_at")
                if not start <= merged < end:
                    continue
                number = int(candidate["number"])
                require(number not in candidates, f"duplicate candidate PR #{number} in paginated search")
                candidates[number] = candidate
            page_info = require_object(connection["pageInfo"], "search.pageInfo")
            has_next = require_bool(page_info["hasNextPage"], "search.pageInfo.hasNextPage")
            pages += 1
            require(pages <= 10, f"search pagination exceeded 10 pages for {name_with_owner} {date_range}")
            if not has_next:
                break
            next_cursor = require_string(page_info["endCursor"], "search.pageInfo.endCursor")
            require(next_cursor != cursor, "search pagination cursor did not advance")
            cursor = next_cursor
        require(
            issue_count is not None
            and (
                issue_count > MAX_SEARCH_RESULTS_PER_SHARD
                or issue_count == observed_for_shard
            ),
            f"search pagination was incomplete for {name_with_owner} {date_range}: expected {issue_count}, observed {observed_for_shard}",
        )
    return sorted(candidates.values(), key=lambda item: int(item["number"]))


def build_sample(plan_bytes: bytes, plan: dict[str, object], api: GithubGraphQL) -> dict[str, object]:
    window = require_object(plan["merged_at_window"], "merged_at_window")
    start = parse_utc_timestamp(window["start"], "merged_at_window.start")
    end = parse_utc_timestamp(window["end_exclusive"], "merged_at_window.end_exclusive")
    selection = require_object(plan["selection"], "selection")
    seed_hex = require_string(selection["seed_hex"], "selection.seed_hex")
    target = require_int(plan["target_pull_requests_per_repository"], "target", 1)
    sampled_repositories = []
    repositories = require_array(plan["repositories"], "repositories")
    for index, repository_value in enumerate(repositories, 1):
        repository = require_object(repository_value, "repository")
        owner = require_string(repository["owner"], "repository.owner")
        name = require_string(repository["name"], "repository.name")
        name_with_owner = f"{owner}/{name}"
        progress(f"sampling {name_with_owner} ({index}/{len(repositories)})")
        candidates = search_repository_candidates(api, owner, name, start, end)
        inventory, selected_numbers = select_candidates(candidates, seed_hex, name_with_owner, target)
        sampled_repositories.append(
            {
                "owner": owner,
                "name": name,
                "name_with_owner": name_with_owner,
                "candidate_count": len(inventory),
                "requested_count": target,
                "selected_count": len(selected_numbers),
                "shortfall": target - len(selected_numbers),
                "candidates": inventory,
                "selected_pull_request_numbers": selected_numbers,
            }
        )
        progress(f"sampled {len(selected_numbers)}/{target} from {len(inventory)} candidates in {name_with_owner}")
    selected_total = sum(int(item["selected_count"]) for item in sampled_repositories)
    return {
        "schema": SAMPLE_SCHEMA,
        "dataset_version": DATASET_VERSION,
        "tool_version": TOOL_VERSION,
        "generated_at": now_timestamp(),
        "source_plan": {
            "schema": plan["schema"],
            "dataset_version": plan["dataset_version"],
            "sha256": sha256_bytes(plan_bytes),
        },
        "selection": {
            "algorithm": SELECTION_ALGORITHM,
            "algorithm_version": SELECTION_ALGORITHM_VERSION,
            "seed_hex": seed_hex,
        },
        "merged_at_window": dict(window),
        "repositories": sampled_repositories,
        "summary": {
            "repositories": len(sampled_repositories),
            "candidate_pull_requests": sum(int(item["candidate_count"]) for item in sampled_repositories),
            "requested_pull_requests": len(sampled_repositories) * target,
            "selected_pull_requests": selected_total,
            "sampling_shortfall": len(sampled_repositories) * target - selected_total,
        },
        "acquisition": api.acquisition(),
    }


def normalize_review(raw_value: object, label: str, case_id: str) -> dict[str, object]:
    raw = require_object(raw_value, label)
    state = require_string(raw["state"], f"{label}.state")
    require(
        state in REVIEW_STATES,
        f"{label} has an unsupported state: {state}",
    )
    submitted_at = raw["submittedAt"]
    require(submitted_at is None or isinstance(submitted_at, str), f"{label}.submittedAt must be string or null")
    if submitted_at is not None:
        parse_utc_timestamp(submitted_at, f"{label}.submittedAt")
    commit = raw["commit"]
    commit_oid = None if commit is None else validate_oid(require_object(commit, f"{label}.commit")["oid"], f"{label}.commit.oid")
    database_id = raw["fullDatabaseId"]
    require(database_id is None or isinstance(database_id, (str, int)), f"{label}.fullDatabaseId must be string, integer, or null")
    comments = require_object(raw["comments"], f"{label}.comments")
    return {
        "node_id": require_string(raw["id"], f"{label}.id"),
        "database_id": database_id,
        "state": state,
        "submitted_at": submitted_at,
        "author": normalize_actor(raw["author"], f"{label}.author", case_id),
        "commit_oid": commit_oid,
        "comment_count": require_int(comments["totalCount"], f"{label}.comments.totalCount"),
    }


def normalize_timeline_event(
    raw_value: object, label: str, case_id: str
) -> dict[str, object]:
    raw = require_object(raw_value, label)
    event_type = require_string(raw["__typename"], f"{label}.__typename")
    common = {
        "node_id": require_string(raw["id"], f"{label}.id"),
        "type": event_type,
        "created_at": require_string(raw["createdAt"], f"{label}.createdAt"),
    }
    parse_utc_timestamp(common["created_at"], f"{label}.createdAt")
    if event_type == "HeadRefForcePushedEvent":
        before = raw["beforeCommit"]
        after = raw["afterCommit"]
        common["before_oid"] = (
            None
            if before is None
            else validate_oid(
                require_object(before, f"{label}.beforeCommit")["oid"],
                f"{label}.beforeCommit.oid",
            )
        )
        common["after_oid"] = (
            None
            if after is None
            else validate_oid(
                require_object(after, f"{label}.afterCommit")["oid"],
                f"{label}.afterCommit.oid",
            )
        )
        return common
    require(event_type == "ReviewDismissedEvent", f"unexpected timeline event type: {event_type}")
    previous_state = raw["previousReviewState"]
    require(previous_state is None or previous_state in REVIEW_STATES, f"{label}.previousReviewState is invalid")
    common["previous_review_state"] = previous_state
    common["review"] = (
        None
        if raw["review"] is None
        else normalize_review(raw["review"], f"{label}.review", case_id)
    )
    return common


def collect_connection(
    api: GithubGraphQL,
    owner: str,
    name: str,
    number: int,
    initial: dict[str, object],
    connection_name: str,
    page_query: str,
) -> tuple[list[object], dict[str, object]]:
    connection = initial
    reported_total_count = require_int(connection["totalCount"], f"{connection_name}.totalCount")
    nodes = list(require_array(connection["nodes"], f"{connection_name}.nodes"))
    page_info = require_object(connection["pageInfo"], f"{connection_name}.pageInfo")
    pages = 1
    cursor: str | None = None
    while require_bool(page_info["hasNextPage"], f"{connection_name}.pageInfo.hasNextPage"):
        cursor = require_string(page_info["endCursor"], f"{connection_name}.pageInfo.endCursor")
        data = api.call(
            page_query,
            {"owner": owner, "name": name, "number": number, "cursor": cursor},
        )
        repository = require_object(data["repository"], f"{owner}/{name}")
        pull_request = require_object(repository["pullRequest"], f"{owner}/{name}#{number}")
        connection = require_object(pull_request[connection_name], connection_name)
        require(
            connection["totalCount"] == reported_total_count,
            f"{connection_name}.totalCount changed while paginating",
        )
        nodes.extend(require_array(connection["nodes"], f"{connection_name}.nodes"))
        next_page_info = require_object(connection["pageInfo"], f"{connection_name}.pageInfo")
        if require_bool(next_page_info["hasNextPage"], f"{connection_name}.pageInfo.hasNextPage"):
            next_cursor = require_string(next_page_info["endCursor"], f"{connection_name}.pageInfo.endCursor")
            require(next_cursor != cursor, f"{connection_name} pagination cursor did not advance")
        page_info = next_page_info
        pages += 1
        require(pages <= MAX_CONNECTION_PAGES, f"{connection_name} exceeds {MAX_CONNECTION_PAGES} pages")
    if connection_name == "reviews":
        require(
            len(nodes) == reported_total_count,
            f"reviews pagination incomplete: expected {reported_total_count}, captured {len(nodes)}",
        )
        return nodes, {
            "pages": pages,
            "reported_total_count": reported_total_count,
            "captured_node_count": len(nodes),
            "pagination_complete": True,
        }
    require(connection_name == "timelineItems", f"unsupported connection: {connection_name}")
    require(
        reported_total_count >= len(nodes),
        "timeline totalCount is smaller than the filtered node count",
    )
    return nodes, {
        "pages": pages,
        "reported_total_count_unfiltered": reported_total_count,
        "captured_filtered_node_count": len(nodes),
        "pagination_complete": True,
    }


def capture_pull_request(api: GithubGraphQL, owner: str, name: str, number: int) -> dict[str, object]:
    case_id = f"github-{owner.casefold()}-{name.casefold()}-pr-{number}"
    data = api.call(CAPTURE_QUERY, {"owner": owner, "name": name, "number": number})
    repository = require_object(data["repository"], f"repository {owner}/{name}")
    require(
        require_string(repository["nameWithOwner"], "repository.nameWithOwner").casefold()
        == f"{owner}/{name}".casefold(),
        "GitHub returned a different repository",
    )
    raw_pull_request = require_object(repository["pullRequest"], f"pull request {owner}/{name}#{number}")
    reviews_connection = require_object(raw_pull_request["reviews"], "reviews")
    timeline_connection = require_object(raw_pull_request["timelineItems"], "timelineItems")
    raw_reviews, review_pagination = collect_connection(
        api, owner, name, number, reviews_connection, "reviews", REVIEWS_PAGE_QUERY
    )
    raw_events, timeline_pagination = collect_connection(
        api, owner, name, number, timeline_connection, "timelineItems", TIMELINE_PAGE_QUERY
    )
    pull_request = normalize_captured_pull_request(raw_pull_request, number, case_id)
    reviews = [
        normalize_review(value, f"reviews[{index}]", case_id)
        for index, value in enumerate(raw_reviews)
    ]
    events = [
        normalize_timeline_event(value, f"timeline_events[{index}]", case_id)
        for index, value in enumerate(raw_events)
    ]
    review_ids = [str(review["node_id"]) for review in reviews]
    event_ids = [str(event["node_id"]) for event in events]
    require(len(review_ids) == len(set(review_ids)), f"duplicate review in {owner}/{name}#{number}")
    require(len(event_ids) == len(set(event_ids)), f"duplicate timeline event in {owner}/{name}#{number}")
    return {
        "id": case_id,
        "repository": {
            "node_id": require_string(repository["id"], "repository.id"),
            "owner": owner,
            "name": name,
            "name_with_owner": require_string(repository["nameWithOwner"], "repository.nameWithOwner"),
            "url": require_string(repository["url"], "repository.url"),
        },
        "pull_request": pull_request,
        "reviews": reviews,
        "timeline_events": events,
        "pagination": {
            "reviews": review_pagination,
            "timeline_events": timeline_pagination,
        },
    }


def validate_sha256(value: object, label: str) -> str:
    digest = require_string(value, label)
    require(
        len(digest) == 64
        and all(character in "0123456789abcdef" for character in digest),
        f"{label} must be 64 lowercase hexadecimal characters",
    )
    return digest


def validate_acquisition(value: object, label: str) -> dict[str, object]:
    acquisition = require_object(value, label)
    require_exact_keys(
        acquisition,
        {"graphql_calls", "minimum_rate_limit_remaining", "last_rate_limit_reset_at"},
        label,
    )
    require_int(acquisition["graphql_calls"], f"{label}.graphql_calls")
    remaining = acquisition["minimum_rate_limit_remaining"]
    require(
        remaining is None or (type(remaining) is int and remaining >= 0),
        f"{label}.minimum_rate_limit_remaining must be a nonnegative integer or null",
    )
    reset_at = acquisition["last_rate_limit_reset_at"]
    require(
        reset_at is None or isinstance(reset_at, str),
        f"{label}.last_rate_limit_reset_at must be a timestamp or null",
    )
    if reset_at is not None:
        parse_utc_timestamp(reset_at, f"{label}.last_rate_limit_reset_at")
    return acquisition


def validate_stored_actor(value: object, label: str) -> dict[str, object] | None:
    if value is None:
        return None
    actor = require_object(value, label)
    require_exact_keys(actor, {"typename", "actor_key"}, label)
    require_string(actor["typename"], f"{label}.typename")
    actor_key = actor["actor_key"]
    require(
        actor_key is None
        or (
            isinstance(actor_key, str)
            and re.fullmatch(r"actor-[0-9a-f]{24}", actor_key) is not None
        ),
        f"{label}.actor_key is invalid",
    )
    return actor


def validate_stored_review(value: object, label: str) -> dict[str, object]:
    review = require_object(value, label)
    require_exact_keys(
        review,
        {
            "node_id",
            "database_id",
            "state",
            "submitted_at",
            "author",
            "commit_oid",
            "comment_count",
        },
        label,
    )
    require_string(review["node_id"], f"{label}.node_id")
    database_id = review["database_id"]
    require(
        database_id is None
        or (type(database_id) is int and database_id > 0)
        or (
            isinstance(database_id, str)
            and database_id.isascii()
            and database_id.isdecimal()
            and not database_id.startswith("0")
        ),
        f"{label}.database_id is invalid",
    )
    require(review["state"] in REVIEW_STATES, f"{label}.state is invalid")
    submitted_at = review["submitted_at"]
    require(
        submitted_at is None or isinstance(submitted_at, str),
        f"{label}.submitted_at must be a timestamp or null",
    )
    if submitted_at is not None:
        parse_utc_timestamp(submitted_at, f"{label}.submitted_at")
    validate_stored_actor(review["author"], f"{label}.author")
    validate_oid(review["commit_oid"], f"{label}.commit_oid", nullable=True)
    require_int(review["comment_count"], f"{label}.comment_count")
    return review


def validate_sample(sample: object, plan_bytes: bytes, plan: dict[str, object]) -> dict[str, object]:
    document = require_object(sample, "sample")
    require_exact_keys(
        document,
        {
            "schema",
            "dataset_version",
            "tool_version",
            "generated_at",
            "source_plan",
            "selection",
            "merged_at_window",
            "repositories",
            "summary",
            "acquisition",
        },
        "sample",
    )
    require(document["schema"] == SAMPLE_SCHEMA, "unsupported sample schema")
    require(document["dataset_version"] == DATASET_VERSION, "unsupported sample dataset version")
    require(document["tool_version"] == TOOL_VERSION, "unsupported sample tool version")
    parse_utc_timestamp(document["generated_at"], "sample.generated_at")
    source = require_object(document["source_plan"], "sample.source_plan")
    require_exact_keys(source, {"schema", "dataset_version", "sha256"}, "sample.source_plan")
    require(source["schema"] == PLAN_SCHEMA, "sample source schema differs")
    require(source["dataset_version"] == DATASET_VERSION, "sample source version differs")
    validate_sha256(source["sha256"], "sample.source_plan.sha256")
    require(source["sha256"] == sha256_bytes(plan_bytes), "sample source plan hash differs")
    selection = require_object(document["selection"], "sample.selection")
    require_exact_keys(selection, {"algorithm", "algorithm_version", "seed_hex"}, "sample.selection")
    require(exact_json_equal(selection, {
        "algorithm": plan["selection"]["algorithm"],
        "algorithm_version": plan["selection"]["algorithm_version"],
        "seed_hex": plan["selection"]["seed_hex"],
    }), "sample selection differs from plan")
    window = require_object(document["merged_at_window"], "sample.merged_at_window")
    require_exact_keys(window, {"start", "end_exclusive", "field", "timezone"}, "sample.merged_at_window")
    require(exact_json_equal(window, plan["merged_at_window"]), "sample merge window differs from plan")
    start = parse_utc_timestamp(window["start"], "sample.merged_at_window.start")
    end = parse_utc_timestamp(window["end_exclusive"], "sample.merged_at_window.end_exclusive")
    repositories = require_array(document["repositories"], "sample.repositories")
    require(len(repositories) == len(plan["repositories"]), "sample repository count differs from plan")
    seed_hex = require_object(plan["selection"], "selection")["seed_hex"]
    target = int(plan["target_pull_requests_per_repository"])
    selected_total = 0
    candidate_total = 0
    for index, repository_value in enumerate(repositories):
        repository = require_object(repository_value, "sample repository")
        require_exact_keys(
            repository,
            {
                "owner",
                "name",
                "name_with_owner",
                "candidate_count",
                "requested_count",
                "selected_count",
                "shortfall",
                "candidates",
                "selected_pull_request_numbers",
            },
            f"sample.repositories[{index}]",
        )
        planned_repository = require_object(plan["repositories"][index], "planned repository")
        require(repository["owner"] == planned_repository["owner"], "sample repository owner differs from plan")
        require(repository["name"] == planned_repository["name"], "sample repository name differs from plan")
        name_with_owner = require_string(repository["name_with_owner"], "sample repository name")
        require(name_with_owner == f"{repository['owner']}/{repository['name']}", "sample repository identity is noncanonical")
        require_int(repository["candidate_count"], f"{name_with_owner}.candidate_count")
        require_int(repository["requested_count"], f"{name_with_owner}.requested_count", 1)
        require_int(repository["selected_count"], f"{name_with_owner}.selected_count")
        require_int(repository["shortfall"], f"{name_with_owner}.shortfall")
        candidates = [require_object(value, "candidate") for value in require_array(repository["candidates"], "candidates")]
        for candidate_index, candidate in enumerate(candidates):
            label = f"sample.repositories[{index}].candidates[{candidate_index}]"
            require_exact_keys(
                candidate,
                {"node_id", "number", "merged_at", "selection_digest"},
                label,
            )
            require_string(candidate["node_id"], f"{label}.node_id")
            merged_at = parse_utc_timestamp(candidate["merged_at"], f"{label}.merged_at")
            require(start <= merged_at < end, f"{label} is outside the frozen merge window")
            validate_sha256(candidate["selection_digest"], f"{label}.selection_digest")
        numbers = [require_int(candidate["number"], "candidate.number", 1) for candidate in candidates]
        node_ids = [require_string(candidate["node_id"], "candidate.node_id") for candidate in candidates]
        require(len(numbers) == len(set(numbers)), f"duplicate candidate in {name_with_owner}")
        require(len(node_ids) == len(set(node_ids)), f"duplicate candidate node ID in {name_with_owner}")
        inventory, selected = select_candidates(candidates, str(seed_hex), name_with_owner, target)
        require(inventory == candidates, f"candidate inventory differs from canonical order for {name_with_owner}")
        require(selected == repository["selected_pull_request_numbers"], f"selection differs for {name_with_owner}")
        require(repository["candidate_count"] == len(candidates), f"candidate count differs for {name_with_owner}")
        require(repository["requested_count"] == target, f"requested count differs for {name_with_owner}")
        require(repository["selected_count"] == len(selected), f"selected count differs for {name_with_owner}")
        require(repository["shortfall"] == target - len(selected), f"shortfall differs for {name_with_owner}")
        selected_total += len(selected)
        candidate_total += len(candidates)
    summary = require_object(document["summary"], "sample.summary")
    require(summary["repositories"] == len(repositories), "sample repository summary differs")
    for key in (
        "repositories",
        "candidate_pull_requests",
        "requested_pull_requests",
        "selected_pull_requests",
        "sampling_shortfall",
    ):
        require_int(summary[key], f"sample.summary.{key}")
    require(summary["candidate_pull_requests"] == candidate_total, "sample candidate summary differs")
    require(summary["selected_pull_requests"] == selected_total, "sample selection summary differs")
    require(summary["requested_pull_requests"] == len(repositories) * target, "sample request summary differs")
    require(summary["sampling_shortfall"] == len(repositories) * target - selected_total, "sample shortfall summary differs")
    require(selected_total > 0, "sample contains no selected pull requests")
    require_exact_keys(
        summary,
        {
            "repositories",
            "candidate_pull_requests",
            "requested_pull_requests",
            "selected_pull_requests",
            "sampling_shortfall",
        },
        "sample.summary",
    )
    validate_acquisition(document["acquisition"], "sample.acquisition")
    return document


def combined_acquisition(
    previous: dict[str, object] | None, api: GithubGraphQL
) -> dict[str, object]:
    current = api.acquisition()
    if previous is None:
        return current
    previous_remaining = previous["minimum_rate_limit_remaining"]
    current_remaining = current["minimum_rate_limit_remaining"]
    remaining_values = [
        value
        for value in (previous_remaining, current_remaining)
        if type(value) is int
    ]
    return {
        "graphql_calls": int(previous["graphql_calls"]) + int(current["graphql_calls"]),
        "minimum_rate_limit_remaining": None
        if not remaining_values
        else min(remaining_values),
        "last_rate_limit_reset_at": current["last_rate_limit_reset_at"]
        if current["last_rate_limit_reset_at"] is not None
        else previous["last_rate_limit_reset_at"],
    }


def capture_document(
    sample_bytes: bytes,
    sample: dict[str, object],
    cases: list[object],
    captured_at: str,
    acquisition: dict[str, object],
) -> dict[str, object]:
    selected = int(sample["summary"]["selected_pull_requests"])
    complete = len(cases) == selected
    return {
        "schema": CAPTURE_SCHEMA,
        "dataset_version": DATASET_VERSION,
        "tool_version": TOOL_VERSION,
        "captured_at": captured_at,
        "capture_complete": complete,
        "source_sample": {
            "schema": sample["schema"],
            "dataset_version": sample["dataset_version"],
            "sha256": sha256_bytes(sample_bytes),
        },
        "cases": cases,
        "summary": {
            "selected_pull_requests": selected,
            "captured_pull_requests": len(cases),
            "capture_failures": 0,
        },
        "acquisition": acquisition,
    }


def build_capture(
    sample_bytes: bytes,
    sample: dict[str, object],
    api: GithubGraphQL,
    *,
    output_path: Path | None = None,
    existing: dict[str, object] | None = None,
) -> dict[str, object]:
    cases = [] if existing is None else list(existing["cases"])
    captured_at = now_timestamp() if existing is None else str(existing["captured_at"])
    previous_acquisition = None if existing is None else require_object(existing["acquisition"], "capture.acquisition")
    captured_ids = {str(require_object(case, "capture case")["id"]) for case in cases}
    selected_total = int(sample["summary"]["selected_pull_requests"])
    if output_path is not None:
        write_json(
            output_path,
            capture_document(
                sample_bytes,
                sample,
                cases,
                captured_at,
                combined_acquisition(previous_acquisition, api),
            ),
        )
    for repository_value in require_array(sample["repositories"], "sample.repositories"):
        repository = require_object(repository_value, "sample repository")
        owner = require_string(repository["owner"], "sample repository.owner")
        name = require_string(repository["name"], "sample repository.name")
        for number_value in require_array(repository["selected_pull_request_numbers"], "selected_pull_request_numbers"):
            number = require_int(number_value, "selected pull request number", 1)
            case_id = f"github-{owner.casefold()}-{name.casefold()}-pr-{number}"
            if case_id in captured_ids:
                continue
            progress(f"capturing {owner}/{name}#{number} ({len(cases) + 1}/{selected_total})")
            cases.append(capture_pull_request(api, owner, name, number))
            captured_ids.add(case_id)
            if output_path is not None:
                write_json(
                    output_path,
                    capture_document(
                        sample_bytes,
                        sample,
                        cases,
                        captured_at,
                        combined_acquisition(previous_acquisition, api),
                    ),
                )
    return capture_document(
        sample_bytes,
        sample,
        cases,
        captured_at,
        combined_acquisition(previous_acquisition, api),
    )


def actor_class(review: dict[str, object], pull_request_author: object) -> str:
    author = review["author"]
    if author is None:
        return "unknown"
    actor = require_object(author, "review.author")
    typename = actor["typename"]
    actor_key = actor["actor_key"]
    if typename == "Bot":
        return "bot"
    if typename != "User":
        return "other"
    if not isinstance(actor_key, str) or actor_key == "":
        return "unknown"
    if pull_request_author is None:
        return "unknown"
    pr_actor = require_object(pull_request_author, "pull_request.author")
    pr_actor_key = pr_actor["actor_key"]
    if not isinstance(pr_actor_key, str) or pr_actor_key == "":
        return "unknown"
    if actor_key == pr_actor_key:
        return "self"
    return "peer_human"


def review_database_id(review: dict[str, object]) -> int:
    value = review["database_id"]
    if type(value) is int:
        require(value > 0, "review database ID must be positive")
        return value
    text = require_string(value, "submitted review database_id")
    require(text.isascii() and text.isdecimal() and not text.startswith("0"), "review database ID must be canonical decimal")
    return int(text)


def review_sort_key(review: dict[str, object]) -> tuple[datetime, int]:
    submitted_at = review["submitted_at"]
    require(isinstance(submitted_at, str), "submitted review is missing submitted_at")
    return parse_utc_timestamp(submitted_at, "review.submitted_at"), review_database_id(review)


def pull_request_author_class(author_value: object) -> str:
    if author_value is None:
        return "unknown"
    author = require_object(author_value, "pull_request.author")
    typename = author["typename"]
    actor_key = author["actor_key"]
    if not isinstance(actor_key, str) or actor_key == "":
        return "unknown"
    if typename == "User":
        return "user"
    if typename == "Bot":
        return "bot"
    return "other"


def completed_review_state(
    review: dict[str, object],
    dismissals: dict[str, dict[str, object]],
) -> tuple[str, dict[str, object] | None] | None:
    state = review["state"]
    if state in FORMAL_STATES:
        require(review["node_id"] not in dismissals, "active formal review also has a dismissal event")
        return str(state), None
    if state != "DISMISSED":
        return None
    review_id = str(review["node_id"])
    if review_id not in dismissals:
        return None
    event = dismissals[review_id]
    previous_state = event["previous_review_state"]
    if previous_state not in FORMAL_STATES:
        return None
    return str(previous_state), event


def classify_case(raw_case_value: object) -> dict[str, object]:
    raw_case = require_object(raw_case_value, "capture case")
    repository = require_object(raw_case["repository"], "case.repository")
    pull_request = require_object(raw_case["pull_request"], "case.pull_request")
    reviews = [require_object(value, "review") for value in require_array(raw_case["reviews"], "case.reviews")]
    events = [require_object(value, "timeline event") for value in require_array(raw_case["timeline_events"], "case.timeline_events")]

    submitted_reviews = [
        review
        for review in reviews
        if review["submitted_at"] is not None and review["state"] != "PENDING"
    ]
    classes = {"peer_human": [], "bot": [], "self": [], "other": [], "unknown": []}
    for review in submitted_reviews:
        classes[actor_class(review, pull_request["author"])].append(review)
    peer_reviews = classes["peer_human"]
    peer_reviews.sort(key=review_sort_key)
    all_human_reviews = []
    for review in submitted_reviews:
        author_value = review["author"]
        if author_value is None:
            continue
        author = require_object(author_value, "review.author")
        if (
            author["typename"] == "User"
            and isinstance(author["actor_key"], str)
            and author["actor_key"]
        ):
            all_human_reviews.append(review)

    dismissal_events = [event for event in events if event["type"] == "ReviewDismissedEvent"]
    force_push_events = [event for event in events if event["type"] == "HeadRefForcePushedEvent"]
    dismissals: dict[str, dict[str, object]] = {}
    invalid_dismissal_order = 0
    for event in dismissal_events:
        dismissed_review_value = event["review"]
        if dismissed_review_value is None:
            continue
        dismissed_review = require_object(dismissed_review_value, "dismissal.review")
        if dismissed_review["submitted_at"] is None:
            continue
        submitted = parse_utc_timestamp(dismissed_review["submitted_at"], "dismissed review submitted_at")
        dismissed = parse_utc_timestamp(event["created_at"], "dismissal created_at")
        if dismissed < submitted:
            invalid_dismissal_order += 1
            continue
        review_id = str(dismissed_review["node_id"])
        require(review_id not in dismissals, f"multiple dismissal events for review {review_id}")
        dismissals[review_id] = event

    reviews_by_actor: dict[str, list[dict[str, object]]] = {}
    for review in peer_reviews:
        author = require_object(review["author"], "review.author")
        reviewer_key = require_string(author["actor_key"], "review.author.actor_key")
        reviews_by_actor.setdefault(reviewer_key, []).append(review)

    final_head = pull_request["head_oid"]
    force_times = sorted(
        parse_utc_timestamp(event["created_at"], "force push created_at")
        for event in force_push_events
    )
    reviewer_pairs = []
    completed_commit_oids: set[str] = set()
    completed_dismissal_events_count = 0
    for reviewer_key in sorted(reviews_by_actor):
        reviewer_reviews = sorted(reviews_by_actor[reviewer_key], key=review_sort_key)
        completed = []
        for review in reviewer_reviews:
            completion = completed_review_state(review, dismissals)
            if completion is not None:
                completed_state, dismissal_event = completion
                completed.append((review, completed_state, dismissal_event))
                if dismissal_event is not None:
                    completed_dismissal_events_count += 1
                if review["commit_oid"] is not None:
                    completed_commit_oids.add(str(review["commit_oid"]))
        comments = []
        for review in reviewer_reviews:
            semantic_commented = review["state"] == "COMMENTED"
            if review["state"] == "DISMISSED" and review["node_id"] in dismissals:
                semantic_commented = (
                    dismissals[str(review["node_id"])]["previous_review_state"]
                    == "COMMENTED"
                )
            if semantic_commented and review["commit_oid"] is not None:
                comments.append(review)
        if not completed and not comments:
            continue
        latest_completed = None if not completed else completed[-1]
        checkpoint = None
        post_completed_force = False
        post_latest_force = False
        force_rereview = False
        commented_newer = False
        if latest_completed is not None:
            latest_review, completed_state, dismissal_event = latest_completed
            latest_time = review_sort_key(latest_review)[0]
            checkpoint_oid = latest_review["commit_oid"]
            comparable = checkpoint_oid is not None and final_head is not None
            differs = None if not comparable else checkpoint_oid != final_head
            completed_times = [review_sort_key(item[0])[0] for item in completed]
            post_completed_force = any(
                completed_time < force_time
                for completed_time in completed_times
                for force_time in force_times
            )
            post_latest_force = any(latest_time < force_time for force_time in force_times)
            force_rereview = any(
                any(before < force_time for before in completed_times)
                and any(force_time < after for after in completed_times)
                for force_time in force_times
            )
            commented_newer = checkpoint_oid is not None and any(
                latest_time < review_sort_key(comment)[0]
                and comment["commit_oid"] != checkpoint_oid
                for comment in comments
            )
            checkpoint = {
                "review_id": latest_review["node_id"],
                "submitted_at": latest_review["submitted_at"],
                "commit_oid": checkpoint_oid,
                "completed_state": completed_state,
                "current_state": latest_review["state"],
                "dismissed": dismissal_event is not None,
                "dismissal_event_id": None
                if dismissal_event is None
                else dismissal_event["node_id"],
                "differs_from_final_head": differs,
                "post_completed_review_force_push": post_completed_force,
                "post_latest_checkpoint_force_push": post_latest_force,
                "force_push_rereview": force_rereview,
            }
        latest_comment = None if not comments else comments[-1]
        reviewer_pairs.append(
            {
                "reviewer_key": reviewer_key,
                "formal_review_sessions": len(reviewer_reviews),
                "completed_review_sessions": len(completed),
                "latest_completed_checkpoint": checkpoint,
                "commented_candidate_sessions": len(comments),
                "latest_commented_candidate": None
                if latest_comment is None
                else {
                    "review_id": latest_comment["node_id"],
                    "submitted_at": latest_comment["submitted_at"],
                    "commit_oid": latest_comment["commit_oid"],
                    "comment_count": latest_comment["comment_count"],
                },
                "commented_only": checkpoint is None and bool(comments),
                "commented_newer_commit_candidate": commented_newer,
            }
        )

    completed_pairs = [
        pair for pair in reviewer_pairs if pair["latest_completed_checkpoint"] is not None
    ]
    comparable_pairs = [
        pair
        for pair in completed_pairs
        if pair["latest_completed_checkpoint"]["differs_from_final_head"] is not None
    ]
    drift_pairs = [
        pair
        for pair in comparable_pairs
        if pair["latest_completed_checkpoint"]["differs_from_final_head"] is True
    ]
    post_force_pairs = [
        pair
        for pair in completed_pairs
        if pair["latest_completed_checkpoint"]["post_completed_review_force_push"]
    ]
    rereview_pairs = [
        pair
        for pair in post_force_pairs
        if pair["latest_completed_checkpoint"]["force_push_rereview"]
    ]
    commented_only_pairs = [pair for pair in reviewer_pairs if pair["commented_only"]]
    commented_newer_pairs = [
        pair for pair in completed_pairs if pair["commented_newer_commit_candidate"]
    ]
    completed_pr = bool(completed_pairs)
    stranded = None
    if completed_pr and len(comparable_pairs) == len(completed_pairs):
        stranded = bool(drift_pairs)
    return {
        "id": raw_case["id"],
        "repository": repository["name_with_owner"],
        "number": pull_request["number"],
        "node_id": pull_request["node_id"],
        "merged_at": pull_request["merged_at"],
        "final_head_oid": final_head,
        "last_commit_oid": pull_request["last_commit_oid"],
        "author_class": pull_request_author_class(pull_request["author"]),
        "capture_complete": True,
        "counts": {
            "observed_reviews": len(reviews),
            "submitted_review_sessions": len(submitted_reviews),
            "all_human_review_sessions": len(all_human_reviews),
            "peer_human_review_sessions": len(peer_reviews),
            "bot_review_sessions": len(classes["bot"]),
            "self_review_sessions": len(classes["self"]),
            "other_actor_review_sessions": len(classes["other"]),
            "unknown_review_sessions": len(classes["unknown"]),
            "unsubmitted_or_pending_reviews": len(reviews) - len(submitted_reviews),
            "completed_review_sessions": sum(
                int(pair["completed_review_sessions"]) for pair in reviewer_pairs
            ),
            "distinct_completed_review_commits": len(completed_commit_oids),
            "completed_review_pairs": len(completed_pairs),
            "observable_checkpoint_oid_pairs": sum(
                pair["latest_completed_checkpoint"]["commit_oid"] is not None
                for pair in completed_pairs
            ),
            "comparable_checkpoint_pairs": len(comparable_pairs),
            "drifted_checkpoint_pairs": len(drift_pairs),
            "post_force_push_checkpoint_pairs": len(post_force_pairs),
            "force_push_rereview_pairs": len(rereview_pairs),
            "drift_without_observed_force_push_pairs": sum(
                not pair["latest_completed_checkpoint"]["post_latest_checkpoint_force_push"]
                for pair in drift_pairs
            ),
            "commented_only_pairs": len(commented_only_pairs),
            "commented_candidate_pairs": sum(
                pair["latest_commented_candidate"] is not None for pair in reviewer_pairs
            ),
            "commented_newer_commit_candidate_pairs": len(commented_newer_pairs),
            "force_push_events": len(force_push_events),
            "review_dismissal_events": len(dismissal_events),
            "completed_review_dismissal_events": completed_dismissal_events_count,
            "invalid_dismissal_order_events": invalid_dismissal_order,
        },
        "classification": {
            "formal_peer_reviewed": bool(peer_reviews),
            "completed_reviewed": completed_pr,
            "stranded_reviewer": stranded,
            "multi_round_completed_review": len(completed_commit_oids) >= 2,
            "completed_review_dismissal": any(
                pair["latest_completed_checkpoint"]["dismissed"]
                for pair in completed_pairs
            ),
        },
        "reviewer_pairs": reviewer_pairs,
    }


def validate_capture_case(
    case_value: object,
    expected: dict[str, tuple[dict[str, object], dict[str, object]]],
) -> str:
    case = require_object(case_value, "capture case")
    require_exact_keys(
        case,
        {"id", "repository", "pull_request", "reviews", "timeline_events", "pagination"},
        "capture case",
    )
    case_id = require_string(case["id"], "case.id")
    require(case_id in expected, f"capture contains an unsampled case: {case_id}")
    repository = require_object(case["repository"], f"{case_id}.repository")
    require_exact_keys(
        repository,
        {"node_id", "owner", "name", "name_with_owner", "url"},
        f"{case_id}.repository",
    )
    pull_request = require_object(case["pull_request"], f"{case_id}.pull_request")
    require_exact_keys(
        pull_request,
        {
            "node_id",
            "number",
            "merged_at",
            "head_oid",
            "last_commit_oid",
            "commit_count",
            "head_matches_last_commit",
            "author",
        },
        f"{case_id}.pull_request",
    )
    sampled_repository, sampled_candidate = expected[case_id]
    require(repository["owner"] == sampled_repository["owner"], f"{case_id} owner differs")
    require(repository["name"] == sampled_repository["name"], f"{case_id} name differs")
    require(repository["name_with_owner"] == sampled_repository["name_with_owner"], f"{case_id} identity differs")
    require(
        repository["url"] == f"https://github.com/{repository['name_with_owner']}",
        f"{case_id} repository URL differs",
    )
    require_string(repository["node_id"], f"{case_id}.repository.node_id")
    require(pull_request["number"] == sampled_candidate["number"], f"{case_id} number differs")
    require(pull_request["node_id"] == sampled_candidate["node_id"], f"{case_id} node ID differs")
    require(pull_request["merged_at"] == sampled_candidate["merged_at"], f"{case_id} merged_at differs")
    parse_utc_timestamp(pull_request["merged_at"], f"{case_id}.merged_at")
    head_oid = validate_oid(pull_request["head_oid"], f"{case_id}.head_oid", nullable=True)
    last_commit_oid = validate_oid(
        pull_request["last_commit_oid"], f"{case_id}.last_commit_oid", nullable=True
    )
    require_int(pull_request["commit_count"], f"{case_id}.commit_count")
    head_matches = pull_request["head_matches_last_commit"]
    require(
        head_matches is None or type(head_matches) is bool,
        f"{case_id}.head_matches_last_commit must be boolean or null",
    )
    if head_oid is not None and last_commit_oid is not None:
        require(head_matches is True and head_oid == last_commit_oid, f"{case_id} head evidence conflicts")
    else:
        require(head_matches is None, f"{case_id} missing head evidence must use null comparison")
    validate_stored_actor(pull_request["author"], f"{case_id}.pull_request.author")

    reviews = [
        validate_stored_review(review, f"{case_id}.reviews[{index}]")
        for index, review in enumerate(require_array(case["reviews"], f"{case_id}.reviews"))
    ]
    review_by_id = {str(review["node_id"]): review for review in reviews}
    require(len(review_by_id) == len(reviews), f"{case_id} contains duplicate reviews")
    events = require_array(case["timeline_events"], f"{case_id}.timeline_events")
    event_ids = set()
    dismissed_review_ids = set()
    for index, event_value in enumerate(events):
        label = f"{case_id}.timeline_events[{index}]"
        event = require_object(event_value, label)
        event_type = event["type"]
        if event_type == "HeadRefForcePushedEvent":
            require_exact_keys(
                event,
                {"node_id", "type", "created_at", "before_oid", "after_oid"},
                label,
            )
            validate_oid(event["before_oid"], f"{label}.before_oid", nullable=True)
            validate_oid(event["after_oid"], f"{label}.after_oid", nullable=True)
        else:
            require(event_type == "ReviewDismissedEvent", f"{label}.type is invalid")
            require_exact_keys(
                event,
                {"node_id", "type", "created_at", "previous_review_state", "review"},
                label,
            )
            previous_state = event["previous_review_state"]
            require(
                previous_state is None or previous_state in REVIEW_STATES,
                f"{label}.previous_review_state is invalid",
            )
            if event["review"] is not None:
                linked = validate_stored_review(event["review"], f"{label}.review")
                linked_id = str(linked["node_id"])
                require(linked_id in review_by_id, f"{label} links an unobserved review")
                require(linked == review_by_id[linked_id], f"{label} review snapshot conflicts")
                require(linked_id not in dismissed_review_ids, f"{case_id} has duplicate review dismissal")
                dismissed_review_ids.add(linked_id)
                require(linked["state"] == "DISMISSED", f"{label} linked review is not DISMISSED")
                if linked["submitted_at"] is not None:
                    require(
                        parse_utc_timestamp(event["created_at"], f"{label}.created_at")
                        >= parse_utc_timestamp(linked["submitted_at"], f"{label}.review.submitted_at"),
                        f"{label} predates its review",
                    )
        event_id = require_string(event["node_id"], f"{label}.node_id")
        require(event_id not in event_ids, f"{case_id} contains duplicate timeline events")
        event_ids.add(event_id)
        parse_utc_timestamp(event["created_at"], f"{label}.created_at")

    pagination = require_object(case["pagination"], f"{case_id}.pagination")
    require_exact_keys(pagination, {"reviews", "timeline_events"}, f"{case_id}.pagination")
    review_pagination = require_object(pagination["reviews"], f"{case_id}.pagination.reviews")
    require_exact_keys(
        review_pagination,
        {"pages", "reported_total_count", "captured_node_count", "pagination_complete"},
        f"{case_id}.pagination.reviews",
    )
    require_int(review_pagination["pages"], f"{case_id}.pagination.reviews.pages", 1)
    require(review_pagination["reported_total_count"] == len(reviews), f"{case_id} review total differs")
    require(review_pagination["captured_node_count"] == len(reviews), f"{case_id} review node count differs")
    require(review_pagination["pagination_complete"] is True, f"{case_id} review pagination is incomplete")
    timeline_pagination = require_object(
        pagination["timeline_events"], f"{case_id}.pagination.timeline_events"
    )
    require_exact_keys(
        timeline_pagination,
        {
            "pages",
            "reported_total_count_unfiltered",
            "captured_filtered_node_count",
            "pagination_complete",
        },
        f"{case_id}.pagination.timeline_events",
    )
    require_int(timeline_pagination["pages"], f"{case_id}.pagination.timeline_events.pages", 1)
    require_int(
        timeline_pagination["reported_total_count_unfiltered"],
        f"{case_id}.pagination.timeline_events.reported_total_count_unfiltered",
    )
    require(
        timeline_pagination["reported_total_count_unfiltered"] >= len(events),
        f"{case_id} timeline total is smaller than filtered events",
    )
    require(
        timeline_pagination["captured_filtered_node_count"] == len(events),
        f"{case_id} filtered timeline count differs",
    )
    require(
        timeline_pagination["pagination_complete"] is True,
        f"{case_id} timeline pagination is incomplete",
    )
    return case_id


def validate_capture(
    capture: object,
    sample_bytes: bytes,
    sample: dict[str, object],
    *,
    require_complete: bool = True,
) -> dict[str, object]:
    document = require_object(capture, "capture")
    require_exact_keys(
        document,
        {
            "schema",
            "dataset_version",
            "tool_version",
            "captured_at",
            "capture_complete",
            "source_sample",
            "cases",
            "summary",
            "acquisition",
        },
        "capture",
    )
    require(document["schema"] == CAPTURE_SCHEMA, "unsupported capture schema")
    require(document["dataset_version"] == DATASET_VERSION, "unsupported capture dataset version")
    require(document["tool_version"] == TOOL_VERSION, "unsupported capture tool version")
    parse_utc_timestamp(document["captured_at"], "capture.captured_at")
    require_bool(document["capture_complete"], "capture.capture_complete")
    if require_complete:
        require(document["capture_complete"] is True, "capture is incomplete")
    source = require_object(document["source_sample"], "capture.source_sample")
    require_exact_keys(source, {"schema", "dataset_version", "sha256"}, "capture.source_sample")
    require(source["schema"] == SAMPLE_SCHEMA, "capture source schema differs")
    require(source["dataset_version"] == DATASET_VERSION, "capture source version differs")
    validate_sha256(source["sha256"], "capture.source_sample.sha256")
    require(source["sha256"] == sha256_bytes(sample_bytes), "capture source sample hash differs")
    expected = {}
    for repository in sample["repositories"]:
        candidates = {int(candidate["number"]): candidate for candidate in repository["candidates"]}
        for number in repository["selected_pull_request_numbers"]:
            case_id = f"github-{str(repository['owner']).casefold()}-{str(repository['name']).casefold()}-pr-{number}"
            expected[case_id] = (repository, candidates[int(number)])
    cases = require_array(document["cases"], "capture.cases")
    observed_order = [validate_capture_case(case_value, expected) for case_value in cases]
    observed = set(observed_order)
    require(len(observed) == len(cases), "capture contains duplicate case IDs")
    expected_order = list(expected)
    if require_complete:
        require(
            observed_order == expected_order,
            "capture cases differ from sampled pull requests or canonical order",
        )
    else:
        require(
            observed_order == expected_order[: len(observed_order)],
            "partial capture is not a canonical sampled prefix",
        )
    summary = require_object(document["summary"], "capture.summary")
    require_exact_keys(
        summary,
        {"selected_pull_requests", "captured_pull_requests", "capture_failures"},
        "capture.summary",
    )
    for key in ("selected_pull_requests", "captured_pull_requests", "capture_failures"):
        require_int(summary[key], f"capture.summary.{key}")
    require(summary["selected_pull_requests"] == len(expected), "capture selected count differs")
    require(summary["captured_pull_requests"] == len(cases), "capture summary differs")
    require(summary["capture_failures"] == 0, "capture failures must be zero")
    require(
        document["capture_complete"] is (len(cases) == len(expected)),
        "capture_complete differs from captured case count",
    )
    validate_acquisition(document["acquisition"], "capture.acquisition")
    return document


def build_manifest(
    plan_bytes: bytes,
    sample_bytes: bytes,
    sample: dict[str, object],
    capture_bytes: bytes,
    capture: dict[str, object],
) -> dict[str, object]:
    cases = [classify_case(value) for value in require_array(capture["cases"], "capture.cases")]
    cases.sort(key=lambda case: (str(case["repository"]).casefold(), int(case["number"])))
    repositories = []
    for repository_value in require_array(sample["repositories"], "sample.repositories"):
        repository = require_object(repository_value, "sample repository")
        repositories.append(
            {
                "name_with_owner": repository["name_with_owner"],
                "frame_candidates": repository["candidate_count"],
                "target": repository["requested_count"],
                "selected": repository["selected_count"],
                "capture_failures": 0,
            }
        )
    return {
        "schema": MANIFEST_SCHEMA,
        "dataset_version": DATASET_VERSION,
        "tool_version": TOOL_VERSION,
        "generated_at": now_timestamp(),
        "sampling_plan_sha256": sha256_bytes(plan_bytes),
        "sample_sha256": sha256_bytes(sample_bytes),
        "capture_sha256": sha256_bytes(capture_bytes),
        "collection": {
            "status": "complete",
            "captured_at": capture["captured_at"],
            "selected_pull_requests": sample["summary"]["selected_pull_requests"],
            "classified_pull_requests": len(cases),
            "capture_failures": 0,
        },
        "repositories": repositories,
        "pull_requests": cases,
    }


def ratio(numerator: int, denominator: int) -> dict[str, object]:
    require(0 <= numerator <= denominator, "rate numerator must be between zero and denominator")
    if denominator == 0:
        return {
            "numerator": 0,
            "denominator": 0,
            "status": "undefined",
            "basis_points": None,
            "wilson_95_lower_basis_points": None,
            "wilson_95_upper_basis_points": None,
        }
    point = (numerator * 10_000 + denominator // 2) // denominator
    probability = numerator / denominator
    z = 1.959963984540054
    divisor = 1.0 + z * z / denominator
    center = (probability + z * z / (2.0 * denominator)) / divisor
    margin = (
        z
        / divisor
        * math.sqrt(
            probability * (1.0 - probability) / denominator
            + z * z / (4.0 * denominator * denominator)
        )
    )
    lower = math.floor(10_000 * max(0.0, center - margin))
    upper = math.ceil(10_000 * min(1.0, center + margin))
    require(0 <= lower <= point <= upper <= 10_000, "Wilson interval invariant failed")
    return {
        "numerator": numerator,
        "denominator": denominator,
        "status": "defined",
        "basis_points": point,
        "wilson_95_lower_basis_points": lower,
        "wilson_95_upper_basis_points": upper,
    }


def completed_pairs(case: dict[str, object]) -> list[dict[str, object]]:
    return [
        pair
        for pair in case["reviewer_pairs"]
        if pair["latest_completed_checkpoint"] is not None
    ]


def metric_counts(cases: list[dict[str, object]], metric_id: str) -> tuple[int, int]:
    all_completed_pairs = [pair for case in cases for pair in completed_pairs(case)]
    if metric_id == "formal_peer_reviewed_pr_rate":
        return sum(bool(case["classification"]["formal_peer_reviewed"]) for case in cases), len(cases)
    if metric_id == "completed_review_pr_rate":
        return sum(bool(case["classification"]["completed_reviewed"]) for case in cases), len(cases)
    if metric_id == "checkpoint_oid_observability_rate":
        comparable = sum(
            pair["latest_completed_checkpoint"]["differs_from_final_head"] is not None
            for pair in all_completed_pairs
        )
        return comparable, len(all_completed_pairs)
    if metric_id == "checkpoint_pair_head_drift_rate":
        comparable = [
            pair
            for pair in all_completed_pairs
            if pair["latest_completed_checkpoint"]["differs_from_final_head"] is not None
        ]
        return (
            sum(
                pair["latest_completed_checkpoint"]["differs_from_final_head"] is True
                for pair in comparable
            ),
            len(comparable),
        )
    if metric_id == "completed_review_pair_post_force_push_rate":
        return (
            sum(
                pair["latest_completed_checkpoint"]["post_completed_review_force_push"]
                for pair in all_completed_pairs
            ),
            len(all_completed_pairs),
        )
    if metric_id == "checkpoint_pair_drift_without_observed_force_push_rate":
        comparable = [
            pair
            for pair in all_completed_pairs
            if pair["latest_completed_checkpoint"]["differs_from_final_head"] is not None
        ]
        return (
            sum(
                pair["latest_completed_checkpoint"]["differs_from_final_head"] is True
                and not pair["latest_completed_checkpoint"]["post_latest_checkpoint_force_push"]
                for pair in comparable
            ),
            len(comparable),
        )
    if metric_id == "stranded_reviewer_pr_rate":
        observable = [
            case for case in cases if case["classification"]["stranded_reviewer"] is not None
        ]
        return sum(case["classification"]["stranded_reviewer"] is True for case in observable), len(observable)
    if metric_id == "multi_round_completed_review_pr_rate":
        completed = [case for case in cases if case["classification"]["completed_reviewed"]]
        return sum(case["classification"]["multi_round_completed_review"] for case in completed), len(completed)
    if metric_id == "completed_review_dismissal_pr_rate":
        completed = [case for case in cases if case["classification"]["completed_reviewed"]]
        return sum(case["classification"]["completed_review_dismissal"] for case in completed), len(completed)
    if metric_id == "commented_only_pair_share":
        known_pairs = [pair for case in cases for pair in case["reviewer_pairs"]]
        return sum(pair["commented_only"] for pair in known_pairs), len(known_pairs)
    if metric_id == "commented_newer_commit_candidate_pair_rate":
        observable = [
            pair
            for pair in all_completed_pairs
            if pair["latest_completed_checkpoint"]["commit_oid"] is not None
        ]
        return sum(pair["commented_newer_commit_candidate"] for pair in observable), len(observable)
    if metric_id == "completed_review_pair_force_push_rereview_rate":
        affected = [
            pair
            for pair in all_completed_pairs
            if pair["latest_completed_checkpoint"]["post_completed_review_force_push"]
        ]
        return sum(pair["latest_completed_checkpoint"]["force_push_rereview"] for pair in affected), len(affected)
    require(metric_id == "bot_review_session_share", f"unknown metric: {metric_id}")
    bot = sum(int(case["counts"]["bot_review_sessions"]) for case in cases)
    human = sum(int(case["counts"]["peer_human_review_sessions"]) for case in cases)
    return bot, bot + human


def audit_metric(cases: list[dict[str, object]], metric_id: str) -> dict[str, object]:
    numerator, denominator = metric_counts(cases, metric_id)
    require(0 <= numerator <= denominator, "rate numerator must be between zero and denominator")
    return {
        "id": metric_id,
        "numerator": numerator,
        "denominator": denominator,
        "status": "undefined" if denominator == 0 else "defined",
        "basis_points": None
        if denominator == 0
        else (numerator * 10_000 + denominator // 2) // denominator,
    }


def audit_finding(
    case: dict[str, object], hostname: str, repository: str
) -> dict[str, object] | None:
    pairs = completed_pairs(case)
    comparable = [
        pair
        for pair in pairs
        if pair["latest_completed_checkpoint"]["differs_from_final_head"] is not None
    ]
    drifted = [
        pair
        for pair in comparable
        if pair["latest_completed_checkpoint"]["differs_from_final_head"] is True
    ]
    unobservable = len(pairs) - len(comparable)
    if not drifted and unobservable == 0:
        return None
    reviewers = []
    for pair in drifted:
        checkpoint = pair["latest_completed_checkpoint"]
        reviewers.append(
            {
                "reviewer_key": pair["reviewer_key"],
                "checkpoint_oid": checkpoint["commit_oid"],
                "checkpoint_submitted_at": checkpoint["submitted_at"],
                "checkpoint_state": checkpoint["completed_state"],
                "dismissed": checkpoint["dismissed"],
                "post_completed_review_force_push": checkpoint[
                    "post_completed_review_force_push"
                ],
                "post_latest_checkpoint_force_push": checkpoint[
                    "post_latest_checkpoint_force_push"
                ],
            }
        )
    number = require_int(case["number"], "audit finding pull request number", 1)
    return {
        "number": number,
        "url": f"https://{hostname}/{repository}/pull/{number}",
        "merged_at": case["merged_at"],
        "final_head_oid": case["final_head_oid"],
        "completed_pair_count": len(pairs),
        "comparable_pair_count": len(comparable),
        "unobservable_pair_count": unobservable,
        "drifted_reviewers": reviewers,
    }


def build_review_memory_audit(
    repository: str,
    hostname: str,
    start: datetime,
    end_exclusive: datetime,
    requested_limit: int,
    candidate_count: int,
    cases: list[dict[str, object]],
    acquisition_value: object,
    generated_at: str,
) -> dict[str, object]:
    require(start < end_exclusive, "audit window must be non-empty")
    require(1 <= requested_limit <= 100, "audit limit must be between 1 and 100")
    require(candidate_count >= len(cases), "audit candidate count is smaller than selected count")
    require(len(cases) <= requested_limit, "audit selected count exceeds requested limit")
    parse_utc_timestamp(generated_at, "audit.generated_at")
    acquisition = validate_acquisition(acquisition_value, "audit acquisition")
    metrics = [audit_metric(cases, metric_id) for metric_id in AUDIT_METRIC_IDS]
    metrics_by_id = {str(metric["id"]): metric for metric in metrics}
    completed_pair_count = int(
        metrics_by_id["checkpoint_oid_observability_rate"]["denominator"]
    )
    comparable_pair_count = int(
        metrics_by_id["checkpoint_oid_observability_rate"]["numerator"]
    )
    drifted_pair_count = int(
        metrics_by_id["checkpoint_pair_head_drift_rate"]["numerator"]
    )
    findings = []
    affected_pull_requests = 0
    for case in cases:
        require(
            str(case["repository"]).casefold() == repository.casefold(),
            "classified audit case belongs to another repository",
        )
        finding = audit_finding(case, hostname, repository)
        if finding is not None:
            findings.append(finding)
        if any(
            pair["latest_completed_checkpoint"]["differs_from_final_head"] is True
            for pair in completed_pairs(case)
        ):
            affected_pull_requests += 1

    if completed_pair_count == 0:
        status = "no_eligible_reviews"
    elif comparable_pair_count * 10_000 < (
        completed_pair_count * AUDIT_MINIMUM_OID_COVERAGE_BASIS_POINTS
    ):
        status = "insufficient_evidence"
    elif drifted_pair_count == 0:
        status = "no_observed_drift"
    else:
        status = "affected"

    return {
        "schema": AUDIT_SCHEMA,
        "tool_version": TOOL_VERSION,
        "generated_at": generated_at,
        "scope": {
            "provider_url": f"https://{hostname}",
            "repository": repository,
            "window": {
                "start": format_utc_timestamp(start),
                "end_exclusive": format_utc_timestamp(end_exclusive),
            },
            "selection": {
                "method": AUDIT_SELECTION_METHOD,
                "requested_limit": requested_limit,
                "candidate_count": candidate_count,
                "selected_count": len(cases),
                "shortfall": requested_limit - len(cases),
            },
        },
        "collection": {
            "status": "complete",
            "graphql_calls": acquisition["graphql_calls"],
            "minimum_rate_limit_remaining": acquisition[
                "minimum_rate_limit_remaining"
            ],
            "last_rate_limit_reset_at": acquisition["last_rate_limit_reset_at"],
        },
        "privacy": {
            "source_collected": False,
            "pr_text_collected": False,
            "review_text_collected": False,
            "commit_messages_collected": False,
            "logins_persisted": False,
            "actor_identity": "pr_local_opaque_key",
        },
        "claim_boundary": {
            "repository_window_description_supported": True,
            "github_population_estimate_supported": False,
            "reviewer_time_savings_supported": False,
            "issue_recall_or_safety_supported": False,
            "willingness_to_pay_supported": False,
            "checkpoint_materialization_supported": False,
        },
        "summary": {
            "status": status,
            "selected_pull_requests": len(cases),
            "formal_peer_reviewed_pull_requests": metrics_by_id[
                "formal_peer_reviewed_pr_rate"
            ]["numerator"],
            "completed_reviewed_pull_requests": metrics_by_id[
                "completed_review_pr_rate"
            ]["numerator"],
            "completed_reviewer_pairs": completed_pair_count,
            "comparable_reviewer_pairs": comparable_pair_count,
            "unobservable_reviewer_pairs": completed_pair_count - comparable_pair_count,
            "drifted_reviewer_pairs": drifted_pair_count,
            "affected_pull_requests": affected_pull_requests,
        },
        "descriptive_metrics": metrics,
        "findings": findings,
    }


def collect_review_memory_audit(
    api: GithubGraphQL,
    repository: str,
    hostname: str,
    start: datetime,
    end_exclusive: datetime,
    limit: int,
) -> dict[str, object]:
    owner, name = repository.split("/")
    candidates = search_repository_candidates(api, owner, name, start, end_exclusive)
    selected = select_latest_candidates(candidates, limit)
    classified_cases = []
    for index, candidate in enumerate(selected, 1):
        number = require_int(candidate["number"], "candidate.number", 1)
        progress(f"auditing {repository}#{number} ({index}/{len(selected)})")
        captured = capture_pull_request(api, owner, name, number)
        pull_request = require_object(captured["pull_request"], "captured pull request")
        require(
            pull_request["node_id"] == candidate["node_id"],
            f"captured pull request {repository}#{number} has a different node ID",
        )
        require(
            pull_request["merged_at"] == candidate["merged_at"],
            f"captured pull request {repository}#{number} has a different merge time",
        )
        classified_cases.append(classify_case(captured))
    return build_review_memory_audit(
        repository,
        hostname,
        start,
        end_exclusive,
        limit,
        len(candidates),
        classified_cases,
        api.acquisition(),
        now_timestamp(),
    )


def render_review_memory_audit_markdown(report: dict[str, object]) -> str:
    scope = report["scope"]
    window = scope["window"]
    selection = scope["selection"]
    summary = report["summary"]
    status_messages = {
        "no_eligible_reviews": "No eligible completed external-peer reviews were observed in the selected pull requests.",
        "insufficient_evidence": "Checkpoint evidence is insufficient: fewer than 90% of completed reviewer pairs expose both checkpoint and final-head object IDs.",
        "no_observed_drift": "No reviewer-checkpoint drift was observed among the comparable completed reviewer pairs.",
        "affected": "Reviewer-checkpoint drift was observed in this repository window.",
    }
    status = str(summary["status"])
    require(status in status_messages, f"unsupported audit status: {status}")
    lines = [
        "# StrataDiff Review Memory Audit",
        "",
        f"- Repository: [{scope['repository']}]({scope['provider_url']}/{scope['repository']})",
        f"- Window: `{window['start']}` to `{window['end_exclusive']}` (end exclusive)",
        f"- Selection: newest {selection['selected_count']} of {selection['candidate_count']} merged pull requests found; requested limit {selection['requested_limit']}",
        f"- Status: `{status}`",
        "",
        status_messages[status],
    ]
    if int(summary["unobservable_reviewer_pairs"]) > 0:
        lines.extend(
            [
                "",
                f"{summary['unobservable_reviewer_pairs']} completed reviewer pair(s) could not be compared because a checkpoint or final-head object ID was unavailable. Unknown evidence is not evidence of no drift.",
            ]
        )
    lines.extend(
        [
            "",
            "## Summary",
            "",
            "| Selected PRs | Completed-review PRs | Completed pairs | Comparable pairs | Drifted pairs | Affected PRs |",
            "|---:|---:|---:|---:|---:|---:|",
            f"| {summary['selected_pull_requests']} | {summary['completed_reviewed_pull_requests']} | {summary['completed_reviewer_pairs']} | {summary['comparable_reviewer_pairs']} | {summary['drifted_reviewer_pairs']} | {summary['affected_pull_requests']} |",
            "",
            "## Descriptive metrics",
            "",
            "| Metric | Numerator | Denominator | Basis points |",
            "|---|---:|---:|---:|",
        ]
    )
    for metric in report["descriptive_metrics"]:
        basis_points = "unknown" if metric["basis_points"] is None else str(metric["basis_points"])
        lines.append(
            f"| `{metric['id']}` | {metric['numerator']} | {metric['denominator']} | {basis_points} |"
        )
    lines.extend(["", "## Findings", ""])
    if not report["findings"]:
        lines.append("No drifted or unobservable completed reviewer pairs were found.")
    for finding in report["findings"]:
        lines.extend(
            [
                f"### [Pull request #{finding['number']}]({finding['url']})",
                "",
                f"- Merged at: `{finding['merged_at']}`",
                f"- Final head: `{finding['final_head_oid'] if finding['final_head_oid'] is not None else 'unknown'}`",
                f"- Completed pairs: {finding['completed_pair_count']}; comparable: {finding['comparable_pair_count']}; unobservable: {finding['unobservable_pair_count']}",
            ]
        )
        if int(finding["unobservable_pair_count"]) > 0:
            lines.append(
                "- One or more completed pairs are unknown, not evidence of no drift."
            )
        if finding["drifted_reviewers"]:
            lines.extend(
                [
                    "",
                    "| Reviewer key | Checkpoint | Submitted | State | Dismissed | Post-review force-push | Post-checkpoint force-push |",
                    "|---|---|---|---|---|---|---|",
                ]
            )
            for reviewer in finding["drifted_reviewers"]:
                lines.append(
                    f"| `{reviewer['reviewer_key']}` | `{reviewer['checkpoint_oid']}` | `{reviewer['checkpoint_submitted_at']}` | `{reviewer['checkpoint_state']}` | {str(reviewer['dismissed']).lower()} | {str(reviewer['post_completed_review_force_push']).lower()} | {str(reviewer['post_latest_checkpoint_force_push']).lower()} |"
                )
        lines.append("")
    if lines[-1] != "":
        lines.append("")
    lines.extend(
        [
            "## Claim boundary",
            "",
            "This report describes only the selected pull requests in this repository and window. It is not a GitHub-wide prevalence estimate and does not establish reviewer time savings, issue recall, safety, or willingness to pay. It does not materialize checkpoint commits.",
            "",
        ]
    )
    return "\n".join(lines)


def aggregate_cases(
    cases: list[dict[str, object]], repository_names: list[str], minimum_denominator: int
) -> list[dict[str, object]]:
    repositories: dict[str, list[dict[str, object]]] = {
        identity: [] for identity in repository_names
    }
    for case in cases:
        identity = str(case["repository"])
        require(identity in repositories, f"manifest case uses unknown repository: {identity}")
        repositories[identity].append(case)
    metrics = []
    for metric_id in METRIC_IDS:
        by_repository = []
        for identity in sorted(repositories, key=str.casefold):
            repository_ratio = ratio(*metric_counts(repositories[identity], metric_id))
            by_repository.append({"repository": identity, **repository_ratio})
        pooled = ratio(*metric_counts(cases, metric_id))
        defined = [int(item["basis_points"]) for item in by_repository if item["basis_points"] is not None]
        median = None
        if defined:
            defined.sort()
            midpoint = len(defined) // 2
            median = (
                defined[midpoint]
                if len(defined) % 2 == 1
                else (defined[midpoint - 1] + defined[midpoint] + 1) // 2
            )
        metrics.append(
            {
                "id": metric_id,
                **pooled,
                "minimum_denominator": minimum_denominator,
                "minimum_denominator_met": int(pooled["denominator"])
                >= minimum_denominator,
                "repository_median_basis_points": median,
                "by_repository": by_repository,
            }
        )
    return metrics


def build_aggregate(
    manifest_bytes: bytes,
    manifest: dict[str, object],
    sample: dict[str, object],
    plan: dict[str, object],
) -> dict[str, object]:
    cases = [
        require_object(value, "manifest pull request")
        for value in require_array(manifest["pull_requests"], "manifest.pull_requests")
    ]
    repository_names = [str(repository["name_with_owner"]) for repository in manifest["repositories"]]
    thresholds = require_object(plan["decision_thresholds"], "decision_thresholds")
    metrics = aggregate_cases(
        cases, repository_names, int(thresholds["min_signal_denominator"])
    )
    metrics_by_id = {str(metric["id"]): metric for metric in metrics}
    target = int(plan["target_pull_requests_per_repository"])
    at_target = sum(int(repository["selected_count"]) == target for repository in sample["repositories"])
    capture_failures = int(manifest["collection"]["capture_failures"])
    completed_reviewed = int(metrics_by_id["completed_review_pr_rate"]["numerator"])
    collection_complete = manifest["collection"]["status"] == "complete"
    gates = {
        "collection_complete": collection_complete,
        "capture_failures_within_limit": capture_failures <= int(thresholds["max_capture_failures"]),
        "minimum_sampled_pull_requests_met": len(cases) >= int(thresholds["min_sampled_prs"]),
        "minimum_repositories_at_target_met": at_target >= int(thresholds["min_repositories_at_target"]),
        "minimum_completed_reviewed_pull_requests_met": completed_reviewed
        >= int(thresholds["min_completed_reviewed_prs"]),
    }
    gates["all_global_gates_passed"] = all(gates.values())
    oid_observability = metrics_by_id["checkpoint_oid_observability_rate"]["basis_points"]
    gates["checkpoint_oid_observability_basis_points"] = oid_observability
    gates["minimum_checkpoint_oid_observability_met"] = (
        oid_observability is not None
        and int(oid_observability) >= int(thresholds["min_head_oid_observability_bps"])
    )

    def product_signal(
        metric_id: str, threshold_key: str, *, requires_oid_observability: bool
    ) -> dict[str, object]:
        metric = metrics_by_id[metric_id]
        denominator_met = int(metric["denominator"]) >= int(thresholds["min_signal_denominator"])
        oid_met = (
            not requires_oid_observability
            or gates["minimum_checkpoint_oid_observability_met"]
        )
        evaluable = (
            gates["all_global_gates_passed"]
            and denominator_met
            and oid_met
            and metric["status"] == "defined"
        )
        lower = metric["wilson_95_lower_basis_points"]
        upper = metric["wilson_95_upper_basis_points"]
        threshold = int(thresholds[threshold_key])
        if not evaluable:
            status = "inconclusive"
        elif int(lower) >= threshold:
            status = "pass"
        elif int(upper) < threshold:
            status = "fail"
        else:
            status = "inconclusive"
        return {
            "metric_id": metric_id,
            "threshold_basis_points": threshold,
            "minimum_denominator": thresholds["min_signal_denominator"],
            "observed_denominator": metric["denominator"],
            "observed_basis_points": metric["basis_points"],
            "wilson_95_lower_basis_points": lower,
            "wilson_95_upper_basis_points": upper,
            "prerequisites": {
                "global_gates_passed": gates["all_global_gates_passed"],
                "minimum_denominator_met": denominator_met,
                "oid_observability_required": requires_oid_observability,
                "oid_observability_basis_points": oid_observability,
                "oid_observability_met": oid_met,
            },
            "evaluable": evaluable,
            "status": status,
        }

    signals = {
        "force_push_wedge": product_signal(
            "completed_review_pair_post_force_push_rate",
            "force_push_wedge_bps",
            requires_oid_observability=False,
        ),
        "all_round_review_continuity": product_signal(
            "checkpoint_pair_drift_without_observed_force_push_rate",
            "all_round_review_continuity_bps",
            requires_oid_observability=True,
        ),
        "commented_partial_attention": product_signal(
            "commented_newer_commit_candidate_pair_rate",
            "commented_partial_attention_bps",
            requires_oid_observability=False,
        ),
    }
    return {
        "schema": AGGREGATE_SCHEMA,
        "dataset_version": DATASET_VERSION,
        "tool_version": TOOL_VERSION,
        "generated_at": now_timestamp(),
        "sampling_plan_sha256": manifest["sampling_plan_sha256"],
        "manifest_sha256": sha256_bytes(manifest_bytes),
        "collection": {
            "status": manifest["collection"]["status"],
            "captured_at": manifest["collection"]["captured_at"],
            "selected_pull_requests": manifest["collection"]["selected_pull_requests"],
            "classified_pull_requests": manifest["collection"]["classified_pull_requests"],
            "capture_failures": capture_failures,
        },
        "claim_boundary": plan["claim_boundary"],
        "metrics": metrics,
        "gates": {
            **gates,
            "repositories_at_target": at_target,
            "required_repositories_at_target": thresholds["min_repositories_at_target"],
            "required_sampled_pull_requests": thresholds["min_sampled_prs"],
            "required_completed_reviewed_pull_requests": thresholds["min_completed_reviewed_prs"],
            "maximum_capture_failures": thresholds["max_capture_failures"],
            "required_checkpoint_oid_observability_basis_points": thresholds[
                "min_head_oid_observability_bps"
            ],
        },
        "signals": signals,
    }


def validate_manifest(
    manifest: object,
    plan_bytes: bytes,
    sample_bytes: bytes,
    sample: dict[str, object],
    capture_bytes: bytes,
    capture: dict[str, object],
) -> dict[str, object]:
    document = require_object(manifest, "manifest")
    require_exact_keys(
        document,
        {
            "schema",
            "dataset_version",
            "tool_version",
            "generated_at",
            "sampling_plan_sha256",
            "sample_sha256",
            "capture_sha256",
            "collection",
            "repositories",
            "pull_requests",
        },
        "manifest",
    )
    require(document["schema"] == MANIFEST_SCHEMA, "unsupported manifest schema")
    require(document["dataset_version"] == DATASET_VERSION, "unsupported manifest dataset version")
    require(document["tool_version"] == TOOL_VERSION, "unsupported manifest tool version")
    parse_utc_timestamp(document["generated_at"], "manifest.generated_at")
    validate_sha256(document["sampling_plan_sha256"], "manifest.sampling_plan_sha256")
    validate_sha256(document["sample_sha256"], "manifest.sample_sha256")
    validate_sha256(document["capture_sha256"], "manifest.capture_sha256")
    require(document["sampling_plan_sha256"] == sha256_bytes(plan_bytes), "manifest sampling plan hash differs")
    require(document["sample_sha256"] == sha256_bytes(sample_bytes), "manifest sample hash differs")
    require(document["capture_sha256"] == sha256_bytes(capture_bytes), "manifest capture hash differs")
    expected = build_manifest(plan_bytes, sample_bytes, sample, capture_bytes, capture)
    expected["generated_at"] = document["generated_at"]
    require(exact_json_equal(document, expected), "manifest differs from deterministic reclassification")
    return document


def validate_aggregate(
    aggregate: object,
    manifest_bytes: bytes,
    manifest: dict[str, object],
    sample: dict[str, object],
    plan: dict[str, object],
) -> dict[str, object]:
    document = require_object(aggregate, "aggregate")
    require_exact_keys(
        document,
        {
            "schema",
            "dataset_version",
            "tool_version",
            "generated_at",
            "sampling_plan_sha256",
            "manifest_sha256",
            "collection",
            "claim_boundary",
            "metrics",
            "gates",
            "signals",
        },
        "aggregate",
    )
    require(document["schema"] == AGGREGATE_SCHEMA, "unsupported aggregate schema")
    require(document["dataset_version"] == DATASET_VERSION, "unsupported aggregate dataset version")
    require(document["tool_version"] == TOOL_VERSION, "unsupported aggregate tool version")
    parse_utc_timestamp(document["generated_at"], "aggregate.generated_at")
    validate_sha256(document["sampling_plan_sha256"], "aggregate.sampling_plan_sha256")
    validate_sha256(document["manifest_sha256"], "aggregate.manifest_sha256")
    require(document["manifest_sha256"] == sha256_bytes(manifest_bytes), "aggregate manifest hash differs")
    expected = build_aggregate(manifest_bytes, manifest, sample, plan)
    expected["generated_at"] = document["generated_at"]
    require(exact_json_equal(document, expected), "aggregate differs from deterministic reevaluation")
    return document


def load_plan(path: Path) -> tuple[bytes, dict[str, object]]:
    payload, value = read_json(path)
    require(payload == canonical_json(value), "sampling plan JSON is not canonical")
    return payload, validate_plan(value)


def command_audit(arguments: argparse.Namespace) -> None:
    end_exclusive_text = (
        now_timestamp()
        if arguments.end_exclusive is None
        else arguments.end_exclusive
    )
    end_exclusive = parse_utc_timestamp(end_exclusive_text, "end-exclusive")
    start = end_exclusive - timedelta(days=arguments.days)
    api = GithubGraphQL(arguments.gh, arguments.hostname)
    report = collect_review_memory_audit(
        api,
        arguments.repository,
        arguments.hostname,
        start,
        end_exclusive,
        arguments.limit,
    )
    if arguments.format == "json":
        payload = canonical_json(report)
    else:
        require(arguments.format == "markdown", "unsupported audit output format")
        payload = render_review_memory_audit_markdown(report).encode("utf-8")
    if arguments.output is not None:
        atomic_write(arguments.output, payload)
        progress(f"wrote review memory audit to {arguments.output}")
    else:
        sys.stdout.write(payload.decode("utf-8"))


def command_sample(arguments: argparse.Namespace) -> None:
    plan_bytes, plan = load_plan(arguments.plan)
    api = GithubGraphQL(arguments.gh, "github.com")
    sample = build_sample(plan_bytes, plan, api)
    write_json(arguments.output, sample)
    print(
        f"sampled {sample['summary']['selected_pull_requests']}/"
        f"{sample['summary']['requested_pull_requests']} pull requests to {arguments.output}"
    )


def command_capture(arguments: argparse.Namespace) -> None:
    plan_bytes, plan = load_plan(arguments.plan)
    sample_bytes, sample_value = read_json(arguments.sample)
    sample = validate_sample(sample_value, plan_bytes, plan)
    api = GithubGraphQL(arguments.gh, "github.com")
    existing = None
    if arguments.resume and arguments.output.exists():
        existing_bytes, existing_value = read_json(arguments.output)
        require(existing_bytes == canonical_json(existing_value), "partial capture JSON is not canonical")
        existing = validate_capture(
            existing_value, sample_bytes, sample, require_complete=False
        )
        progress(
            f"resuming {len(existing['cases'])}/{sample['summary']['selected_pull_requests']} captured PRs"
        )
    capture = build_capture(
        sample_bytes,
        sample,
        api,
        output_path=arguments.output,
        existing=existing,
    )
    write_json(arguments.output, capture)
    print(f"captured {len(capture['cases'])} pull requests to {arguments.output}")


def command_classify(arguments: argparse.Namespace) -> None:
    plan_bytes, plan = load_plan(arguments.plan)
    sample_bytes, sample_value = read_json(arguments.sample)
    sample = validate_sample(sample_value, plan_bytes, plan)
    capture_bytes, capture_value = read_json(arguments.capture)
    capture = validate_capture(capture_value, sample_bytes, sample)
    manifest = build_manifest(plan_bytes, sample_bytes, sample, capture_bytes, capture)
    write_json(arguments.output, manifest)
    print(f"classified {len(manifest['pull_requests'])} pull requests to {arguments.output}")


def command_evaluate(arguments: argparse.Namespace) -> None:
    plan_bytes, plan = load_plan(arguments.plan)
    sample_bytes, sample_value = read_json(arguments.sample)
    sample = validate_sample(sample_value, plan_bytes, plan)
    capture_bytes, capture_value = read_json(arguments.capture)
    capture = validate_capture(capture_value, sample_bytes, sample)
    manifest_bytes, manifest_value = read_json(arguments.manifest)
    manifest = validate_manifest(
        manifest_value, plan_bytes, sample_bytes, sample, capture_bytes, capture
    )
    aggregate = build_aggregate(manifest_bytes, manifest, sample, plan)
    write_json(arguments.output, aggregate)
    print(f"evaluated {len(manifest['pull_requests'])} pull requests to {arguments.output}")


def command_verify(arguments: argparse.Namespace) -> None:
    plan_bytes, plan = load_plan(arguments.plan)
    sample_bytes, sample_value = read_json(arguments.sample)
    require(sample_bytes == canonical_json(sample_value), "sample JSON is not canonical")
    sample = validate_sample(sample_value, plan_bytes, plan)
    capture_bytes, capture_value = read_json(arguments.capture)
    require(capture_bytes == canonical_json(capture_value), "capture JSON is not canonical")
    capture = validate_capture(capture_value, sample_bytes, sample)
    manifest_bytes, manifest_value = read_json(arguments.manifest)
    require(manifest_bytes == canonical_json(manifest_value), "manifest JSON is not canonical")
    manifest = validate_manifest(
        manifest_value, plan_bytes, sample_bytes, sample, capture_bytes, capture
    )
    aggregate_bytes, aggregate_value = read_json(arguments.aggregate)
    require(aggregate_bytes == canonical_json(aggregate_value), "aggregate JSON is not canonical")
    aggregate = validate_aggregate(aggregate_value, manifest_bytes, manifest, sample, plan)
    print(
        f"verified Review Churn Census v1: {aggregate['collection']['classified_pull_requests']} PRs, "
        f"{len(manifest['repositories'])} repositories"
    )


def add_common_inputs(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--plan", type=Path, default=DEFAULT_PLAN)
    parser.add_argument("--sample", type=Path, default=DEFAULT_SAMPLE)


def parse_arguments(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    audit = subparsers.add_parser(
        "audit", help="audit recent review checkpoints in one repository"
    )
    audit.add_argument(
        "--repository",
        required=True,
        type=audit_repository_argument,
        help="GitHub repository in OWNER/REPO form",
    )
    audit.add_argument(
        "--hostname",
        required=True,
        type=audit_hostname_argument,
        help="GitHub hostname without a scheme or port",
    )
    audit.add_argument(
        "--limit",
        type=audit_limit_argument,
        default=50,
        help="newest merged pull requests to inspect (1-100; default: 50)",
    )
    audit.add_argument(
        "--days",
        type=audit_days_argument,
        default=90,
        help="days in the half-open audit window (1-365; default: 90)",
    )
    audit.add_argument(
        "--end-exclusive",
        type=audit_end_exclusive_argument,
        help="reproducible exclusive UTC window end as an RFC3339 timestamp",
    )
    audit.add_argument(
        "--format",
        choices=("markdown", "json"),
        default="markdown",
        help="report format (default: markdown)",
    )
    audit.add_argument("--output", type=Path, help="write the report to this path")
    audit.add_argument("--gh", default="gh", help="GitHub CLI executable (default: gh)")
    audit.set_defaults(function=command_audit)

    sample = subparsers.add_parser("sample", help="query and deterministically sample merged PRs")
    sample.add_argument("--plan", type=Path, default=DEFAULT_PLAN)
    sample.add_argument("--output", type=Path, default=DEFAULT_SAMPLE)
    sample.add_argument("--gh", default="gh")
    sample.set_defaults(function=command_sample)

    capture = subparsers.add_parser("capture", help="capture reviews and churn events")
    add_common_inputs(capture)
    capture.add_argument("--output", type=Path, default=DEFAULT_CAPTURE)
    capture.add_argument("--gh", default="gh")
    capture.add_argument(
        "--resume",
        action="store_true",
        help="resume an exact, validated partial capture at --output",
    )
    capture.set_defaults(function=command_capture)

    classify = subparsers.add_parser("classify", help="derive per-PR census facts")
    add_common_inputs(classify)
    classify.add_argument("--capture", type=Path, default=DEFAULT_CAPTURE)
    classify.add_argument("--output", type=Path, default=DEFAULT_MANIFEST)
    classify.set_defaults(function=command_classify)

    evaluate = subparsers.add_parser("evaluate", help="aggregate the classified census")
    add_common_inputs(evaluate)
    evaluate.add_argument("--capture", type=Path, default=DEFAULT_CAPTURE)
    evaluate.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    evaluate.add_argument("--output", type=Path, default=DEFAULT_AGGREGATE)
    evaluate.set_defaults(function=command_evaluate)

    verify = subparsers.add_parser("verify", help="verify hashes, canonical JSON, and recomputation")
    add_common_inputs(verify)
    verify.add_argument("--capture", type=Path, default=DEFAULT_CAPTURE)
    verify.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    verify.add_argument("--aggregate", type=Path, default=DEFAULT_AGGREGATE)
    verify.set_defaults(function=command_verify)
    return parser.parse_args(argv)


def main() -> None:
    arguments = parse_arguments()
    arguments.function(arguments)


if __name__ == "__main__":
    main()
