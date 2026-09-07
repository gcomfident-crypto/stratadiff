#!/usr/bin/env python3
"""Independent metrics oracle for the Review Governor event contract.

This module intentionally does not import the product evaluator. It replays the
published policy contract with a separate state representation and returns only
the metrics that the verifier compares.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from datetime import datetime
from pathlib import Path


def seconds(value: str) -> int:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError(f"timestamp lacks timezone: {value}")
    return int(parsed.timestamp())


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def oracle_case(case: dict, policy: dict) -> dict:
    runtime = case["calibration"]["review_runtime_seconds"]
    current_sha = None
    current_diff = None
    final_head_at = None
    pending = None
    jobs = []
    reviewed_diffs = {}
    reviewed_heads = set()
    coverage = []
    covered_when_finalized = False

    def has_coverage(head_sha: str, diff_hash: str) -> bool:
        if head_sha in reviewed_heads:
            return True
        return policy["dedupe_completed_whole_diff"] and diff_hash in reviewed_diffs

    def add_coverage(at: int, head_sha: str, diff_hash: str) -> None:
        reviewed_heads.add(head_sha)
        coverage.append((at, head_sha, diff_hash))

    def launch(at: int) -> None:
        nonlocal jobs
        if current_sha is None or current_diff is None:
            raise ValueError("oracle dispatch without current head")
        if policy["dedupe_completed_whole_diff"] and current_diff in reviewed_diffs:
            add_coverage(at, current_sha, current_diff)
            return
        jobs.append(
            {
                "head": current_sha,
                "diff": current_diff,
                "start": at,
                "finish": at + runtime,
                "status": "running",
                "superseded": False,
                "work": None,
            }
        )

    def finish_job(job: dict, at: int) -> None:
        job["status"] = "completed"
        job["work"] = at - job["start"]
        reviewed_diffs[job["diff"]] = at
        if job["superseded"]:
            return
        if job["head"] == current_sha:
            add_coverage(at, job["head"], job["diff"])
        elif policy["dedupe_completed_whole_diff"] and job["diff"] == current_diff:
            add_coverage(at, current_sha, current_diff)
        else:
            job["superseded"] = True

    def settle(until: int) -> None:
        nonlocal pending
        while True:
            completions = [job["finish"] for job in jobs if job["status"] == "running"]
            times = completions[:]
            if pending is not None:
                times.append(pending["at"])
            if not times or min(times) > until:
                return
            moment = min(times)
            for job in jobs:
                if job["status"] == "running" and job["finish"] == moment:
                    finish_job(job, moment)
            if pending is not None and pending["at"] == moment:
                timer = pending
                pending = None
                if timer["head"] == current_sha:
                    launch(moment)

    for event in case["events"]:
        kind = event["kind"]
        if kind == "observed_review":
            continue
        moment = seconds(event["at"])
        settle(moment)
        if kind == "head":
            current_sha = event["head_sha"]
            current_diff = event["whole_diff_sha256"]
            final_head_at = moment
            pending = None
            for job in jobs:
                if job["status"] != "running" or job["head"] == current_sha:
                    continue
                job["superseded"] = True
                if policy["cancel_in_progress"]:
                    job["status"] = "cancelled"
                    job["work"] = moment - job["start"]
            delay = policy["dispatch_delay_seconds"]
            if delay == 0:
                launch(moment)
            else:
                pending = {"at": moment + delay, "head": current_sha}
        elif kind == "finalize":
            if current_sha != event["head_sha"]:
                raise ValueError(f"{case['case_id']}: oracle finalize head mismatch")
            covered_when_finalized = has_coverage(current_sha, current_diff)
            if policy["final_head_flush"]:
                pending = None
                running_final = any(
                    job["status"] == "running" and job["head"] == current_sha for job in jobs
                )
                if not has_coverage(current_sha, current_diff) and not running_final:
                    launch(moment)
        else:
            raise ValueError(f"{case['case_id']}: oracle unknown event {kind}")

    while pending is not None or any(job["status"] == "running" for job in jobs):
        remaining = [job["finish"] for job in jobs if job["status"] == "running"]
        if pending is not None:
            remaining.append(pending["at"])
        settle(max(remaining))

    if current_sha is None or current_diff is None or final_head_at is None:
        raise ValueError(f"{case['case_id']}: oracle saw no head")
    final_covered = has_coverage(current_sha, current_diff)
    final_times = [
        at
        for at, head_sha, diff_hash in coverage
        if at >= final_head_at
        and (
            head_sha == current_sha
            or (policy["dedupe_completed_whole_diff"] and diff_hash == current_diff)
        )
    ]
    return {
        "case_id": case["case_id"],
        "policy_id": policy["id"],
        "metrics": {
            "dispatched": len(jobs),
            "completed": sum(job["status"] == "completed" for job in jobs),
            "cancelled": sum(job["status"] == "cancelled" for job in jobs),
            "superseded": sum(job["superseded"] for job in jobs),
            "billed_invocation_proxy": len(jobs),
            "work_seconds_proxy": sum(job["work"] for job in jobs),
            "final_head_covered_at_finalize": covered_when_finalized,
            "final_head_covered": final_covered,
            "final_head_coverage_lag_seconds": (
                min(final_times) - final_head_at if final_times else None
            ),
        },
    }


def build_oracle(trace: dict, policies: dict, trace_hash: str, policy_hash: str) -> dict:
    rows = [
        oracle_case(case, policy)
        for case in trace["cases"]
        for policy in policies["policies"]
    ]
    return {
        "schema_version": "review-governor-oracle-v0",
        "input_trace_sha256": trace_hash,
        "input_policies_sha256": policy_hash,
        "cases": rows,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trace", required=True, type=Path)
    parser.add_argument("--policies", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    arguments = parser.parse_args()
    trace_bytes = arguments.trace.read_bytes()
    policy_bytes = arguments.policies.read_bytes()
    output = build_oracle(
        json.loads(trace_bytes),
        json.loads(policy_bytes),
        digest(trace_bytes),
        digest(policy_bytes),
    )
    encoded = (json.dumps(output, indent=2, sort_keys=True) + "\n").encode()
    if arguments.output is None:
        print(encoded.decode(), end="")
    else:
        arguments.output.write_bytes(encoded)


if __name__ == "__main__":
    main()
