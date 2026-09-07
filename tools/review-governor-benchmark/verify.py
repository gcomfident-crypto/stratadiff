#!/usr/bin/env python3
"""Verify frozen hashes, deterministic replay, and the independent oracle."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from datetime import datetime
from pathlib import Path

from evaluator import canonical_json, evaluate, sha256_bytes
from oracle import build_oracle


HEX_SHA256 = re.compile(r"^[0-9a-f]{64}$")
CODERABBIT_USER = {
    "id": 136622811,
    "login": "coderabbitai[bot]",
    "type": "Bot",
    "html_url": "https://github.com/apps/coderabbitai",
}
GITHUB_ACTIONS_USER = {
    "id": 41898282,
    "login": "github-actions[bot]",
    "type": "Bot",
    "html_url": "https://github.com/apps/github-actions",
}
FULL_REVIEW_BODY_SHA256 = "e5cabf5e4c3d622694329a24a3b45b3d90f6001d7cdda827186af5fca07623f1"
INLINE_SKIP_BODY_SHA256 = "3ec7b88a44f4ae95fa887d17653312f8f40675620cd1631059ba64555f7d3694"


def parse_time(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError(f"timestamp lacks timezone: {value}")
    return parsed


def verify_body_fingerprint(body: dict) -> None:
    if body["bytes"] < 1:
        raise ValueError("provider body fingerprint must have positive byte length")
    if not HEX_SHA256.fullmatch(body["sha256"]):
        raise ValueError("provider body fingerprint must have a SHA-256 digest")


def verify_provider_contract(provider: dict, trace: dict) -> dict:
    if provider["schema_version"] != "review-governor-provider-contract-v0":
        raise ValueError("unsupported provider contract schema")
    if provider["provider"] != "coderabbit":
        raise ValueError("provider contract names an unsupported provider")
    contract = provider["contract"]
    if contract != {
        "status_context": "CodeRabbit",
        "status_description": "Review completed",
        "status_avatar_url": "https://avatars.githubusercontent.com/in/347564?v=4",
        "review_bot_id": 136622811,
        "review_bot_login": "coderabbitai[bot]",
        "review_bot_app_url": "https://github.com/apps/coderabbitai",
        "review_marker_sha256": "bff88b5424ccb0637d9154915c72108250078a478b89b043f1709ff026a94c00",
        "maximum_completion_skew_seconds": 300,
    }:
        raise ValueError("provider identity contract changed")

    expected_reviews = {
        (case["case_id"], event["source_review_id"]): event
        for case in trace["cases"]
        for event in case["events"]
        if event["kind"] == "observed_review"
    }
    observed_reviews = {}
    skews = []
    for row in provider["review_evidence"]:
        review = row["review"]
        status = row["status"]
        key = (row["case_id"], review["id"])
        if key in observed_reviews:
            raise ValueError(f"duplicate provider review evidence: {key}")
        observed_reviews[key] = row
        if key not in expected_reviews:
            raise ValueError(f"provider review is absent from trace: {key}")
        event = expected_reviews[key]
        if review["commit_id"] != row["head_sha"] or event["head_sha"] != row["head_sha"]:
            raise ValueError(f"provider review head mismatch: {key}")
        if review["submitted_at"] != event["at"]:
            raise ValueError(f"provider review timestamp mismatch: {key}")
        if review["user"] != CODERABBIT_USER or review["state"] not in {
            "APPROVED",
            "CHANGES_REQUESTED",
            "COMMENTED",
        }:
            raise ValueError(f"provider review identity or state mismatch: {key}")
        if not review["marker_present"]:
            raise ValueError(f"provider review lacks the substantive marker: {key}")
        verify_body_fingerprint(review["body"])
        if (
            status["state"] != "success"
            or status["context"] != "CodeRabbit"
            or status["description"] != "Review completed"
            or status["avatar_url"] != "https://avatars.githubusercontent.com/in/347564?v=4"
            or status["creator"] != CODERABBIT_USER
        ):
            raise ValueError(f"provider completion status mismatch: {key}")
        calculated_skew = int(
            abs(
                (
                    parse_time(status["created_at"])
                    - parse_time(review["submitted_at"])
                ).total_seconds()
            )
        )
        if calculated_skew != row["completion_skew_seconds"]:
            raise ValueError(f"provider completion skew mismatch: {key}")
        if calculated_skew > contract["maximum_completion_skew_seconds"]:
            raise ValueError(f"provider completion skew exceeds contract: {key}")
        skews.append(calculated_skew)
    if set(observed_reviews) != set(expected_reviews):
        raise ValueError("provider contract review set differs from trace")

    successes = provider["command_routes"]["conversation_successes"]
    if len(successes) != 3:
        raise ValueError("provider contract must retain three conversation-route successes")
    acknowledgement_update_skews = []
    for row in successes:
        command = row["command"]
        acknowledgement = row["acknowledgement"]
        review = row["review"]
        if row["route"] != "pull_request_conversation":
            raise ValueError("provider success used the wrong command route")
        if command["user"] != GITHUB_ACTIONS_USER:
            raise ValueError("provider command was not posted by GitHub Actions")
        if (
            not command["is_full_review_command"]
            or command["body"]["sha256"] != FULL_REVIEW_BODY_SHA256
        ):
            raise ValueError("provider command body changed")
        if acknowledgement["user"] != CODERABBIT_USER:
            raise ValueError("provider acknowledgement identity changed")
        if (
            not acknowledgement["command_invocation_marker_present"]
            or not acknowledgement["full_review_finished_present"]
        ):
            raise ValueError("provider acknowledgement lacks invocation evidence")
        if review["user"] != CODERABBIT_USER or not review["marker_present"]:
            raise ValueError("conversation command lacks a substantive provider review")
        verify_body_fingerprint(command["body"])
        verify_body_fingerprint(acknowledgement["body"])
        verify_body_fingerprint(review["body"])
        calculated_latency = int(
            (
                parse_time(review["submitted_at"])
                - parse_time(command["created_at"])
            ).total_seconds()
        )
        if calculated_latency != row["command_to_review_seconds"] or calculated_latency < 0:
            raise ValueError("provider conversation command latency mismatch")
        calculated_update_skew = int(
            (
                parse_time(acknowledgement["updated_at"])
                - parse_time(review["submitted_at"])
            ).total_seconds()
        )
        if (
            calculated_update_skew
            != row["review_to_acknowledgement_update_seconds"]
            or calculated_update_skew < 0
            or calculated_update_skew > contract["maximum_completion_skew_seconds"]
        ):
            raise ValueError("provider acknowledgement update skew mismatch")
        acknowledgement_update_skews.append(calculated_update_skew)

    skipped = provider["command_routes"]["inline_bot_reply_skip"]
    if skipped["route"] != "inline_review_thread":
        raise ValueError("provider skip boundary used the wrong route")
    if skipped["root"]["user"] != CODERABBIT_USER:
        raise ValueError("provider inline root identity changed")
    if skipped["trigger"]["user"]["type"] != "Bot":
        raise ValueError("provider inline skip trigger was not bot-authored")
    if skipped["trigger"]["user"] == CODERABBIT_USER:
        raise ValueError("provider inline skip trigger must come from another bot")
    root_id = skipped["root"]["id"]
    if (
        skipped["trigger"]["in_reply_to_id"] != root_id
        or skipped["response"]["in_reply_to_id"] != root_id
    ):
        raise ValueError("provider inline skip thread identity changed")
    if (
        skipped["response"]["user"] != CODERABBIT_USER
        or not skipped["response"]["skip_text_present"]
        or skipped["response"]["body"]["sha256"] != INLINE_SKIP_BODY_SHA256
    ):
        raise ValueError("provider inline skip response changed")
    verify_body_fingerprint(skipped["trigger"]["body"])
    verify_body_fingerprint(skipped["response"]["body"])

    return {
        "review_evidence_count": len(observed_reviews),
        "maximum_completion_skew_seconds": max(skews),
        "maximum_acknowledgement_update_skew_seconds": max(
            acknowledgement_update_skews
        ),
        "conversation_success_count": len(successes),
        "inline_bot_reply_skip_count": 1,
    }


def metrics_projection(evaluation: dict) -> dict:
    return {
        (row["case_id"], row["policy_id"]): row["metrics"]
        for row in evaluation["cases"]
    }


def verify_checksums(directory: Path, manifest: dict) -> None:
    lines = (directory / "SHA256SUMS").read_text().splitlines()
    recorded = {}
    for line in lines:
        digest, name = line.split("  ", 1)
        recorded[name] = digest
    expected_names = set(manifest["artifacts"])
    expected_names.add("manifest.json")
    if set(recorded) != expected_names:
        raise ValueError("SHA256SUMS file set differs from manifest artifact set")
    for name, expected_digest in recorded.items():
        actual = hashlib.sha256((directory / name).read_bytes()).hexdigest()
        if actual != expected_digest:
            raise ValueError(f"checksum mismatch for {name}: {actual} != {expected_digest}")
    for name, metadata in manifest["artifacts"].items():
        payload = (directory / name).read_bytes()
        if hashlib.sha256(payload).hexdigest() != metadata["sha256"]:
            raise ValueError(f"manifest digest mismatch for {name}")
        if len(payload) != metadata["size_bytes"]:
            raise ValueError(f"manifest size mismatch for {name}")


def verify(directory: Path) -> dict:
    manifest = json.loads((directory / "manifest.json").read_bytes())
    if manifest["schema_version"] != "review-governor-manifest-v0":
        raise ValueError("unsupported manifest schema")
    verify_checksums(directory, manifest)

    trace_bytes = (directory / "trace-v0.json").read_bytes()
    policy_bytes = (directory / "policies-v0.json").read_bytes()
    trace = json.loads(trace_bytes)
    policies = json.loads(policy_bytes)
    provider = json.loads((directory / "provider-contract-v0.json").read_bytes())
    provider_summary = verify_provider_contract(provider, trace)
    evaluated = evaluate(
        trace,
        policies,
        sha256_bytes(trace_bytes),
        sha256_bytes(policy_bytes),
    )
    frozen_evaluation = json.loads((directory / "evaluation-v0.json").read_bytes())
    if canonical_json(evaluated) != canonical_json(frozen_evaluation):
        raise ValueError("frozen evaluation differs from deterministic evaluator replay")

    independently_observed = build_oracle(
        trace,
        policies,
        sha256_bytes(trace_bytes),
        sha256_bytes(policy_bytes),
    )
    frozen_oracle = json.loads((directory / "oracle-v0.json").read_bytes())
    if canonical_json(independently_observed) != canonical_json(frozen_oracle):
        raise ValueError("frozen oracle differs from independent replay")
    if metrics_projection(evaluated) != metrics_projection(independently_observed):
        raise ValueError("evaluator metrics disagree with independent oracle")

    for row in evaluated["cases"]:
        metrics = row["metrics"]
        if metrics["dispatched"] != metrics["completed"] + metrics["cancelled"]:
            raise ValueError(f"{row['case_id']} {row['policy_id']}: run accounting mismatch")
        if metrics["billed_invocation_proxy"] != metrics["dispatched"]:
            raise ValueError(f"{row['case_id']} {row['policy_id']}: proxy contract mismatch")
        if not metrics["final_head_covered"]:
            raise ValueError(f"{row['case_id']} {row['policy_id']}: final head not covered")

    aggregate = {row["policy_id"]: row for row in evaluated["aggregate"]}
    governor = aggregate["governor_stable_head_final_head"]
    per_push = aggregate["per_push"]
    baseline = aggregate["github_concurrency_5m_whole_diff"]
    if governor["billed_invocation_proxy"] >= per_push["billed_invocation_proxy"]:
        raise ValueError("frozen v0 no longer shows fewer Governor invocations than per-push")
    if governor["billed_invocation_proxy"] >= baseline["billed_invocation_proxy"]:
        raise ValueError("frozen v0 no longer shows fewer Governor invocations than baseline")

    return {
        "case_count": len(trace["cases"]),
        "head_event_count": sum(
            event["kind"] == "head" for case in trace["cases"] for event in case["events"]
        ),
        "observed_review_event_count": sum(
            event["kind"] == "observed_review"
            for case in trace["cases"]
            for event in case["events"]
        ),
        "provider_contract": provider_summary,
        "aggregate": evaluated["aggregate"],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("benchmark", type=Path)
    arguments = parser.parse_args()
    result = verify(arguments.benchmark)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
