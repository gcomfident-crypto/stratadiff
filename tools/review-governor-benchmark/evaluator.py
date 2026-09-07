#!/usr/bin/env python3
"""Deterministic event replay for Review Governor scheduling policies."""

from __future__ import annotations

import argparse
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path


def parse_time(value: str) -> int:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError(f"timestamp must include an offset: {value}")
    return int(parsed.timestamp())


def format_time(value: int) -> str:
    return datetime.fromtimestamp(value, timezone.utc).isoformat().replace("+00:00", "Z")


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def validate_trace(trace: dict) -> None:
    if trace["schema_version"] != "review-governor-trace-v0":
        raise ValueError("unsupported trace schema")
    if not trace["cases"]:
        raise ValueError("trace must contain at least one case")
    for case in trace["cases"]:
        heads = [event for event in case["events"] if event["kind"] == "head"]
        finalizes = [event for event in case["events"] if event["kind"] == "finalize"]
        if not heads:
            raise ValueError(f"{case['case_id']}: no head events")
        if len(finalizes) != 1:
            raise ValueError(f"{case['case_id']}: expected exactly one finalize event")
        if heads[-1]["head_sha"] != case["final_head_sha"]:
            raise ValueError(f"{case['case_id']}: final head is not the last observed head")
        if finalizes[0]["head_sha"] != case["final_head_sha"]:
            raise ValueError(f"{case['case_id']}: finalize head mismatch")
        ordered = [parse_time(event["at"]) for event in case["events"]]
        if ordered != sorted(ordered):
            raise ValueError(f"{case['case_id']}: events are not chronological")
        for event in heads:
            if len(event["whole_diff_sha256"]) != 64:
                raise ValueError(f"{case['case_id']}: malformed whole-diff hash")


def validate_policies(document: dict) -> None:
    if document["schema_version"] != "review-governor-policies-v0":
        raise ValueError("unsupported policy schema")
    identifiers = [policy["id"] for policy in document["policies"]]
    required = {
        "per_push",
        "github_concurrency_5m_whole_diff",
        "governor_stable_head_final_head",
    }
    if not required.issubset(set(identifiers)):
        raise ValueError(f"missing required policies: {sorted(required - set(identifiers))}")
    if len(identifiers) != len(set(identifiers)):
        raise ValueError("policy identifiers must be unique")
    for policy in document["policies"]:
        if policy["dispatch_delay_seconds"] < 0:
            raise ValueError(f"{policy['id']}: negative dispatch delay")


class Replay:
    def __init__(self, case: dict, policy: dict):
        self.case = case
        self.policy = policy
        self.runtime = case["calibration"]["review_runtime_seconds"]
        self.current_head = None
        self.current_hash = None
        self.current_head_at = None
        self.pending = None
        self.active = []
        self.runs = []
        self.coverage_events = []
        self.completed_hashes = {}
        self.covered_heads = set()
        self.next_run_number = 1
        self.finalized_at = None
        self.final_head_covered_at_finalize = False

    def _record_coverage(self, at: int, head_sha: str, diff_hash: str, source: str) -> None:
        self.covered_heads.add(head_sha)
        self.coverage_events.append(
            {
                "at": format_time(at),
                "head_sha": head_sha,
                "source": source,
                "whole_diff_sha256": diff_hash,
            }
        )

    def _hash_is_covered(self, diff_hash: str) -> bool:
        return self.policy["dedupe_completed_whole_diff"] and diff_hash in self.completed_hashes

    def _head_is_covered(self, head_sha: str, diff_hash: str) -> bool:
        return head_sha in self.covered_heads or self._hash_is_covered(diff_hash)

    def _dispatch(self, at: int, trigger: str) -> None:
        if self.current_head is None or self.current_hash is None:
            raise ValueError("cannot dispatch without a current head")
        if self._hash_is_covered(self.current_hash):
            self._record_coverage(at, self.current_head, self.current_hash, "whole_diff_cache")
            return
        run = {
            "run_id": f"run-{self.next_run_number:03d}",
            "head_sha": self.current_head,
            "whole_diff_sha256": self.current_hash,
            "dispatch_at": format_time(at),
            "completion_at": format_time(at + self.runtime),
            "trigger": trigger,
            "execution_status": "running",
            "superseded": False,
            "superseded_at": None,
            "ended_at": None,
            "work_seconds_proxy": None,
        }
        self.next_run_number += 1
        self.runs.append(run)
        self.active.append(run)

    def _complete(self, run: dict, at: int) -> None:
        run["execution_status"] = "completed"
        run["ended_at"] = format_time(at)
        run["work_seconds_proxy"] = at - parse_time(run["dispatch_at"])
        self.completed_hashes[run["whole_diff_sha256"]] = at
        if not run["superseded"]:
            if run["head_sha"] == self.current_head:
                self._record_coverage(at, run["head_sha"], run["whole_diff_sha256"], "review_run")
            elif self.policy["dedupe_completed_whole_diff"] and run["whole_diff_sha256"] == self.current_hash:
                self._record_coverage(at, self.current_head, self.current_hash, "review_run_whole_diff")
            else:
                run["superseded"] = True
                run["superseded_at"] = format_time(at)

    def _cancel(self, run: dict, at: int) -> None:
        run["execution_status"] = "cancelled"
        run["superseded"] = True
        run["superseded_at"] = format_time(at)
        run["ended_at"] = format_time(at)
        run["work_seconds_proxy"] = at - parse_time(run["dispatch_at"])

    def _advance(self, target: int) -> None:
        while True:
            completion_times = [
                parse_time(run["completion_at"])
                for run in self.active
                if run["execution_status"] == "running"
            ]
            next_completion = min(completion_times) if completion_times else None
            next_dispatch = self.pending["due_at"] if self.pending is not None else None
            candidates = [value for value in (next_completion, next_dispatch) if value is not None]
            if not candidates or min(candidates) > target:
                return
            next_at = min(candidates)
            if next_completion is not None and next_completion == next_at:
                completing = [
                    run
                    for run in self.active
                    if run["execution_status"] == "running"
                    and parse_time(run["completion_at"]) == next_at
                ]
                for run in completing:
                    self._complete(run, next_at)
                    self.active.remove(run)
            if self.pending is not None and self.pending["due_at"] == next_at:
                pending = self.pending
                self.pending = None
                if pending["head_sha"] == self.current_head:
                    self._dispatch(next_at, "stability_timer")

    def _supersede_active(self, at: int) -> None:
        for run in list(self.active):
            if run["head_sha"] == self.current_head:
                continue
            if not run["superseded"]:
                run["superseded"] = True
                run["superseded_at"] = format_time(at)
            if self.policy["cancel_in_progress"]:
                self._cancel(run, at)
                self.active.remove(run)

    def on_head(self, event: dict) -> None:
        at = parse_time(event["at"])
        self._advance(at)
        self.current_head = event["head_sha"]
        self.current_hash = event["whole_diff_sha256"]
        self.current_head_at = at
        self.pending = None
        self._supersede_active(at)
        delay = self.policy["dispatch_delay_seconds"]
        if delay == 0:
            self._dispatch(at, "head_event")
        else:
            self.pending = {
                "due_at": at + delay,
                "head_sha": self.current_head,
                "whole_diff_sha256": self.current_hash,
            }

    def on_finalize(self, event: dict) -> None:
        at = parse_time(event["at"])
        self._advance(at)
        self.finalized_at = at
        if event["head_sha"] != self.current_head:
            raise ValueError(f"{self.case['case_id']}: finalize did not name the current head")
        self.final_head_covered_at_finalize = self._head_is_covered(self.current_head, self.current_hash)
        if not self.policy["final_head_flush"]:
            return
        self.pending = None
        if self._head_is_covered(self.current_head, self.current_hash):
            return
        matching = [
            run
            for run in self.active
            if run["execution_status"] == "running"
            and run["head_sha"] == self.current_head
        ]
        if not matching:
            self._dispatch(at, "final_head_flush")

    def drain(self) -> None:
        while self.pending is not None or self.active:
            times = []
            if self.pending is not None:
                times.append(self.pending["due_at"])
            times.extend(
                parse_time(run["completion_at"])
                for run in self.active
                if run["execution_status"] == "running"
            )
            if not times:
                break
            self._advance(max(times))

    def result(self) -> dict:
        if self.finalized_at is None:
            raise ValueError(f"{self.case['case_id']}: trace was not finalized")
        final_hash = next(
            event["whole_diff_sha256"]
            for event in reversed(self.case["events"])
            if event["kind"] == "head"
        )
        final_covered = self._head_is_covered(self.case["final_head_sha"], final_hash)
        final_head_at = next(
            parse_time(event["at"])
            for event in reversed(self.case["events"])
            if event["kind"] == "head"
        )
        final_coverage_times = [
            parse_time(event["at"])
            for event in self.coverage_events
            if parse_time(event["at"]) >= final_head_at
            and (
                event["head_sha"] == self.case["final_head_sha"]
                or (
                    self.policy["dedupe_completed_whole_diff"]
                    and event["whole_diff_sha256"] == final_hash
                )
            )
        ]
        return {
            "case_id": self.case["case_id"],
            "policy_id": self.policy["id"],
            "metrics": {
                "dispatched": len(self.runs),
                "completed": sum(run["execution_status"] == "completed" for run in self.runs),
                "cancelled": sum(run["execution_status"] == "cancelled" for run in self.runs),
                "superseded": sum(run["superseded"] for run in self.runs),
                "billed_invocation_proxy": len(self.runs),
                "work_seconds_proxy": sum(run["work_seconds_proxy"] for run in self.runs),
                "final_head_covered_at_finalize": self.final_head_covered_at_finalize,
                "final_head_covered": final_covered,
                "final_head_coverage_lag_seconds": (
                    min(final_coverage_times) - final_head_at if final_coverage_times else None
                ),
            },
            "runs": self.runs,
            "coverage_events": self.coverage_events,
        }


def evaluate_case(case: dict, policy: dict) -> dict:
    replay = Replay(case, policy)
    for event in case["events"]:
        if event["kind"] == "head":
            replay.on_head(event)
        elif event["kind"] == "finalize":
            replay.on_finalize(event)
        elif event["kind"] != "observed_review":
            raise ValueError(f"{case['case_id']}: unknown event kind {event['kind']}")
    replay.drain()
    return replay.result()


def aggregate(results: list[dict], policies: list[dict], case_count: int) -> list[dict]:
    output = []
    for policy in policies:
        rows = [row for row in results if row["policy_id"] == policy["id"]]
        if len(rows) != case_count:
            raise ValueError(f"{policy['id']}: missing case results")
        output.append(
            {
                "policy_id": policy["id"],
                "case_count": case_count,
                "dispatched": sum(row["metrics"]["dispatched"] for row in rows),
                "completed": sum(row["metrics"]["completed"] for row in rows),
                "cancelled": sum(row["metrics"]["cancelled"] for row in rows),
                "superseded": sum(row["metrics"]["superseded"] for row in rows),
                "billed_invocation_proxy": sum(
                    row["metrics"]["billed_invocation_proxy"] for row in rows
                ),
                "work_seconds_proxy": sum(row["metrics"]["work_seconds_proxy"] for row in rows),
                "final_head_covered_cases": sum(
                    row["metrics"]["final_head_covered"] for row in rows
                ),
                "final_head_covered_at_finalize_cases": sum(
                    row["metrics"]["final_head_covered_at_finalize"] for row in rows
                ),
            }
        )
    return output


def evaluate(trace: dict, policies_document: dict, trace_sha256: str, policy_sha256: str) -> dict:
    validate_trace(trace)
    validate_policies(policies_document)
    results = [
        evaluate_case(case, policy)
        for case in trace["cases"]
        for policy in policies_document["policies"]
    ]
    return {
        "schema_version": "review-governor-evaluation-v0",
        "input_trace_sha256": trace_sha256,
        "input_policies_sha256": policy_sha256,
        "metric_contract": {
            "billed_invocation_proxy": "one unit per dispatched reviewer invocation; not money, tokens, or a vendor bill",
            "work_seconds_proxy": "simulated active reviewer seconds; not measured compute or latency",
            "superseded": "a dispatched run whose target ceased to be current before useful coverage completed",
        },
        "cases": results,
        "aggregate": aggregate(results, policies_document["policies"], len(trace["cases"])),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trace", required=True, type=Path)
    parser.add_argument("--policies", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    arguments = parser.parse_args()

    trace_bytes = arguments.trace.read_bytes()
    policy_bytes = arguments.policies.read_bytes()
    result = evaluate(
        json.loads(trace_bytes),
        json.loads(policy_bytes),
        sha256_bytes(trace_bytes),
        sha256_bytes(policy_bytes),
    )
    encoded = canonical_json(result)
    if arguments.output is None:
        print(encoded.decode(), end="")
    else:
        arguments.output.write_bytes(encoded)


if __name__ == "__main__":
    main()
