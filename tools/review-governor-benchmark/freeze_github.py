#!/usr/bin/env python3
"""Freeze a small public GitHub PR head/review trace into the v0 contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
import subprocess
from datetime import datetime
from pathlib import Path


def parse_time(value: str) -> int:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError(f"timestamp lacks timezone: {value}")
    return int(parsed.timestamp())


def gh_json(endpoint: str, accept: str | None = None) -> object:
    command = ["gh", "api", "--cache", "24h", endpoint]
    if accept is not None:
        command.extend(["-H", f"Accept: {accept}"])
    completed = subprocess.run(command, check=True, stdout=subprocess.PIPE)
    return json.loads(completed.stdout)


def gh_bytes(endpoint: str, accept: str) -> bytes:
    completed = subprocess.run(
        ["gh", "api", "--cache", "24h", endpoint, "-H", f"Accept: {accept}"],
        check=True,
        stdout=subprocess.PIPE,
    )
    return completed.stdout


def gh_paginated_list(endpoint: str, accept: str) -> list:
    completed = subprocess.run(
        [
            "gh",
            "api",
            "--cache",
            "24h",
            "--paginate",
            "--slurp",
            endpoint,
            "-H",
            f"Accept: {accept}",
        ],
        check=True,
        stdout=subprocess.PIPE,
    )
    pages = json.loads(completed.stdout)
    return [item for page in pages for item in page]


def sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def head_observation(repository: str, sha: str) -> dict:
    response = gh_json(f"repos/{repository}/commits/{sha}/check-suites?per_page=100")
    suites = [suite for suite in response["check_suites"] if suite["app"]["slug"] == "github-actions"]
    if not suites:
        raise ValueError(f"{repository}@{sha}: no public GitHub Actions check suite")
    suites.sort(key=lambda suite: (suite["created_at"], suite["id"]))
    earliest_at = suites[0]["created_at"]
    earliest = [suite for suite in suites if suite["created_at"] == earliest_at]
    return {
        "at": earliest_at,
        "check_suite_ids": [suite["id"] for suite in earliest],
        "source_url": f"https://github.com/{repository}/commit/{sha}/checks",
    }


def freeze_case(selection: dict) -> dict:
    repository = selection["repository"]
    pr_number = selection["pr_number"]
    pull = gh_json(f"repos/{repository}/pulls/{pr_number}")
    timeline = gh_paginated_list(
        f"repos/{repository}/issues/{pr_number}/timeline?per_page=100",
        "application/vnd.github+json",
    )
    if pull["merged_at"] is None:
        raise ValueError(f"{repository}#{pr_number}: v0 selection must be merged")

    force_events = {}
    commit_candidates = set()
    review_events = []
    for event in timeline:
        if event["event"] == "head_ref_force_pushed":
            sha = event["commit_id"]
            force_events[sha] = event
            commit_candidates.add(sha)
        elif event["event"] == "committed":
            commit_candidates.add(event["sha"])
        elif event["event"] == "reviewed" and event["user"]["login"] == "coderabbitai[bot]":
            commit_candidates.add(event["commit_id"])
            review_events.append(event)

    final_head = pull["head"]["sha"]
    commit_candidates.add(final_head)
    opened_at = pull["created_at"]
    opened_seconds = parse_time(opened_at)
    finalized_seconds = parse_time(pull["merged_at"])
    head_events = []
    head_times = {}

    for sha in sorted(commit_candidates):
        observation = head_observation(repository, sha)
        observed_seconds = parse_time(observation["at"])
        evidence_kinds = []
        if sha in force_events:
            force = force_events[sha]
            at = force["created_at"]
            time_basis = "github_head_ref_force_pushed_event"
            source_event_id = force["id"]
            evidence_kinds.append("force_push")
        else:
            at = observation["at"] if observed_seconds >= opened_seconds else opened_at
            time_basis = (
                "github_actions_check_suite_created_at"
                if observed_seconds >= opened_seconds
                else "pull_request_opened_at_lower_bound"
            )
            source_event_id = None
        if any(review["commit_id"] == sha for review in review_events):
            evidence_kinds.append("reviewed_head")
        if sha == final_head:
            evidence_kinds.append("final_head")
        committed_only = not evidence_kinds
        if committed_only and observed_seconds < opened_seconds:
            continue
        at_seconds = parse_time(at)
        if at_seconds > finalized_seconds:
            raise ValueError(f"{repository}#{pr_number}@{sha}: head observed after merge")
        diff = gh_bytes(
            f"repos/{repository}/compare/{pull['base']['sha']}...{sha}",
            "application/vnd.github.v3.diff",
        )
        if not diff.startswith(b"diff --git "):
            raise ValueError(f"{repository}#{pr_number}@{sha}: compare response was not a Git diff")
        event = {
            "kind": "head",
            "at": at,
            "head_sha": sha,
            "whole_diff_sha256": sha256(diff),
            "whole_diff_bytes": len(diff),
            "observation": {
                "time_basis": time_basis,
                "check_suite_created_at": observation["at"],
                "check_suite_ids": observation["check_suite_ids"],
                "check_suite_url": observation["source_url"],
                "timeline_event_id": source_event_id,
                "evidence_kinds": evidence_kinds,
                "immutable_diff_url": (
                    f"https://api.github.com/repos/{repository}/compare/"
                    f"{pull['base']['sha']}...{sha}"
                ),
            },
        }
        head_events.append(event)
        head_times[sha] = at_seconds

    head_events.sort(key=lambda event: (parse_time(event["at"]), event["head_sha"]))
    if head_events[-1]["head_sha"] != final_head:
        raise ValueError(f"{repository}#{pr_number}: last head is not the merged head")

    frozen_reviews = []
    for review in review_events:
        if review["commit_id"] not in head_times:
            raise ValueError(
                f"{repository}#{pr_number}: reviewed head {review['commit_id']} was not materialized"
            )
        frozen_reviews.append(
            {
                "kind": "observed_review",
                "at": review["submitted_at"],
                "head_sha": review["commit_id"],
                "reviewer": review["user"]["login"],
                "state": review["state"],
                "source_review_id": review["id"],
                "source_url": (
                    f"https://github.com/{repository}/pull/{pr_number}"
                    f"#pullrequestreview-{review['id']}"
                ),
            }
        )

    first_review_by_head = {}
    for review in frozen_reviews:
        review_at = parse_time(review["at"])
        sha = review["head_sha"]
        if sha not in first_review_by_head or review_at < first_review_by_head[sha]:
            first_review_by_head[sha] = review_at
    lags = [
        review_at - head_times[sha]
        for sha, review_at in first_review_by_head.items()
        if review_at >= head_times[sha]
    ]
    if not lags:
        raise ValueError(f"{repository}#{pr_number}: no non-negative public review lag")

    finalize = {
        "kind": "finalize",
        "at": pull["merged_at"],
        "head_sha": final_head,
        "outcome": "merged",
        "merge_commit_sha": pull["merge_commit_sha"],
        "source_url": f"https://github.com/{repository}/pull/{pr_number}",
    }
    kind_order = {"head": 0, "observed_review": 1, "finalize": 2}
    events = head_events + frozen_reviews + [finalize]
    events.sort(key=lambda event: (parse_time(event["at"]), kind_order[event["kind"]]))
    return {
        "case_id": f"{repository}#{pr_number}",
        "repository": repository,
        "pr_number": pr_number,
        "pr_url": f"https://github.com/{repository}/pull/{pr_number}",
        "opened_at": opened_at,
        "finalized_at": pull["merged_at"],
        "base_sha": pull["base"]["sha"],
        "final_head_sha": final_head,
        "calibration": {
            "review_runtime_seconds": int(statistics.median(lags)),
            "basis": "median first CodeRabbit review submission minus public head observation",
            "observation_count": len(lags),
            "observed_lag_seconds": sorted(lags),
        },
        "selection_rationale": selection["rationale"],
        "events": events,
    }


def freeze(selection_document: dict) -> dict:
    if selection_document["schema_version"] != "review-governor-selection-v0":
        raise ValueError("unsupported selection schema")
    cases = [freeze_case(selection) for selection in selection_document["cases"]]
    return {
        "schema_version": "review-governor-trace-v0",
        "frozen_at": selection_document["frozen_at"],
        "provenance": selection_document["provenance"],
        "observation_contract": {
            "head": (
                "force-push event time when public; otherwise earliest public GitHub Actions "
                "check-suite creation time, with PR-open time as a lower bound for a pre-open check"
            ),
            "review": "public GitHub pull-request review submission by coderabbitai[bot]",
            "whole_diff": "SHA-256 of GitHub's immutable base-SHA...head-SHA raw diff response",
            "finalize": "public merged_at and immutable final head SHA",
        },
        "cases": cases,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--selection", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args()
    selection = json.loads(arguments.selection.read_bytes())
    encoded = canonical_json(freeze(selection))
    if arguments.check:
        if arguments.output.read_bytes() != encoded:
            raise SystemExit("frozen trace differs from live immutable-source reconstruction")
        print(f"verified {arguments.output}")
    else:
        arguments.output.write_bytes(encoded)
        print(f"wrote {arguments.output} sha256={sha256(encoded)}")


if __name__ == "__main__":
    main()
