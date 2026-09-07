#!/usr/bin/env python3

import json
import unittest
from pathlib import Path

from evaluator import evaluate_case, validate_trace
from oracle import oracle_case


ROOT = Path(__file__).resolve().parents[2]
BENCHMARK = ROOT / "benchmarks" / "review-governor-benchmark-v0"


def policy(identifier: str, delay: int, cancel: bool, dedupe: bool, flush: bool) -> dict:
    return {
        "id": identifier,
        "dispatch_delay_seconds": delay,
        "cancel_in_progress": cancel,
        "dedupe_completed_whole_diff": dedupe,
        "final_head_flush": flush,
    }


def case(events: list[dict], runtime: int = 600) -> dict:
    return {
        "case_id": "synthetic-contract-test",
        "final_head_sha": next(
            event["head_sha"] for event in reversed(events) if event["kind"] == "head"
        ),
        "calibration": {"review_runtime_seconds": runtime},
        "events": events,
    }


class ReviewGovernorEvaluatorTest(unittest.TestCase):
    def assert_oracle_parity(self, fixture: dict, selected_policy: dict) -> dict:
        evaluated = evaluate_case(fixture, selected_policy)["metrics"]
        independently_observed = oracle_case(fixture, selected_policy)["metrics"]
        self.assertEqual(evaluated, independently_observed)
        return evaluated

    def test_debounce_cancels_obsolete_in_progress_run(self) -> None:
        fixture = case(
            [
                {
                    "kind": "head",
                    "at": "2026-01-01T00:00:00Z",
                    "head_sha": "a",
                    "whole_diff_sha256": "1" * 64,
                },
                {
                    "kind": "head",
                    "at": "2026-01-01T00:06:40Z",
                    "head_sha": "b",
                    "whole_diff_sha256": "2" * 64,
                },
                {"kind": "finalize", "at": "2026-01-01T00:30:00Z", "head_sha": "b"},
            ]
        )
        metrics = self.assert_oracle_parity(
            fixture,
            policy("baseline", 300, True, True, False),
        )
        self.assertEqual(metrics["dispatched"], 2)
        self.assertEqual(metrics["cancelled"], 1)
        self.assertEqual(metrics["superseded"], 1)
        self.assertEqual(metrics["work_seconds_proxy"], 700)
        self.assertTrue(metrics["final_head_covered"])

    def test_whole_diff_cache_carries_only_completed_content(self) -> None:
        fixture = case(
            [
                {
                    "kind": "head",
                    "at": "2026-01-01T00:00:00Z",
                    "head_sha": "a",
                    "whole_diff_sha256": "1" * 64,
                },
                {
                    "kind": "head",
                    "at": "2026-01-01T00:02:00Z",
                    "head_sha": "b",
                    "whole_diff_sha256": "1" * 64,
                },
                {"kind": "finalize", "at": "2026-01-01T00:03:00Z", "head_sha": "b"},
            ],
            runtime=60,
        )
        metrics = self.assert_oracle_parity(
            fixture,
            policy("hash", 0, True, True, False),
        )
        self.assertEqual(metrics["dispatched"], 1)
        self.assertTrue(metrics["final_head_covered_at_finalize"])
        self.assertEqual(metrics["final_head_coverage_lag_seconds"], 0)

    def test_final_flush_replaces_a_pending_stability_timer(self) -> None:
        fixture = case(
            [
                {
                    "kind": "head",
                    "at": "2026-01-01T00:00:00Z",
                    "head_sha": "a",
                    "whole_diff_sha256": "1" * 64,
                },
                {"kind": "finalize", "at": "2026-01-01T00:05:00Z", "head_sha": "a"},
            ]
        )
        metrics = self.assert_oracle_parity(
            fixture,
            policy("governor", 900, True, True, True),
        )
        self.assertEqual(metrics["dispatched"], 1)
        self.assertFalse(metrics["final_head_covered_at_finalize"])
        self.assertTrue(metrics["final_head_covered"])
        self.assertEqual(metrics["final_head_coverage_lag_seconds"], 900)

    def test_frozen_artifacts_match_independent_oracle(self) -> None:
        from verify import verify

        result = verify(BENCHMARK)
        self.assertEqual(result["case_count"], 3)
        self.assertEqual(result["head_event_count"], 55)
        self.assertEqual(result["observed_review_event_count"], 36)
        self.assertEqual(result["provider_contract"]["review_evidence_count"], 36)
        self.assertEqual(result["provider_contract"]["maximum_completion_skew_seconds"], 18)
        self.assertEqual(
            result["provider_contract"]["maximum_acknowledgement_update_skew_seconds"],
            16,
        )
        self.assertEqual(result["provider_contract"]["conversation_success_count"], 3)

    def test_provider_contract_rejects_missing_substantive_marker(self) -> None:
        from verify import verify_provider_contract

        trace = json.loads((BENCHMARK / "trace-v0.json").read_bytes())
        provider = json.loads((BENCHMARK / "provider-contract-v0.json").read_bytes())
        provider["review_evidence"][0]["review"]["marker_present"] = False
        with self.assertRaisesRegex(ValueError, "substantive marker"):
            verify_provider_contract(provider, trace)

    def test_provider_contract_keeps_command_routes_distinct(self) -> None:
        from verify import verify_provider_contract

        trace = json.loads((BENCHMARK / "trace-v0.json").read_bytes())
        provider = json.loads((BENCHMARK / "provider-contract-v0.json").read_bytes())
        provider["command_routes"]["inline_bot_reply_skip"]["route"] = (
            "pull_request_conversation"
        )
        with self.assertRaisesRegex(ValueError, "wrong route"):
            verify_provider_contract(provider, trace)

    def test_provider_contract_rejects_stale_acknowledgement_update(self) -> None:
        from verify import verify_provider_contract

        trace = json.loads((BENCHMARK / "trace-v0.json").read_bytes())
        provider = json.loads((BENCHMARK / "provider-contract-v0.json").read_bytes())
        provider["command_routes"]["conversation_successes"][0]["acknowledgement"][
            "updated_at"
        ] = "2026-06-04T17:30:47Z"
        with self.assertRaisesRegex(ValueError, "acknowledgement update skew"):
            verify_provider_contract(provider, trace)

    def test_trace_validation_rejects_wrong_final_head(self) -> None:
        trace = json.loads((BENCHMARK / "trace-v0.json").read_bytes())
        trace["cases"][0]["final_head_sha"] = "0" * 40
        with self.assertRaisesRegex(ValueError, "final head"):
            validate_trace(trace)


if __name__ == "__main__":
    unittest.main()
