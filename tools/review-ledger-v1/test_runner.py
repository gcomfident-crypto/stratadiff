#!/usr/bin/env python3

import importlib.util
import json
import unittest
from pathlib import Path


RUNNER_PATH = Path(__file__).with_name("runner.py")
SPEC = importlib.util.spec_from_file_location("review_ledger_v1_runner", RUNNER_PATH)
assert SPEC is not None
assert SPEC.loader is not None
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class RunnerTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.manifest = json.loads(runner.DEFAULT_MANIFEST.read_bytes())
        cls.cases = {case["id"]: case for case in cls.manifest["cases"]}

    def test_every_manifest_case_has_an_explicit_disposition(self) -> None:
        runner.validate_manifest(self.manifest)
        case_ids = set(self.cases)
        dispositions = runner.WEBHOOK_CASES | runner.HEAD_CASES | runner.COVERAGE_CASES
        self.assertEqual(case_ids, dispositions)

    def test_duplicate_redelivery_keeps_the_first_receive_time(self) -> None:
        case = self.cases["duplicate-redelivery-is-idempotent-across-receive-times"]
        specs = runner.webhook_specs(self.manifest, case)
        result = runner.independent_webhook_oracle(specs)
        self.assertEqual(result["ingest_outcomes"], ["applied", "duplicate"])
        self.assertEqual(result["delivery_count"], 1)
        self.assertEqual(result["canonical_received_at"], "2026-09-05T02:00:00Z")

    def test_dismissed_latest_review_does_not_fall_back(self) -> None:
        case = self.cases["latest-dismissed-review-does-not-fall-back"]
        specs = runner.webhook_specs(self.manifest, case)
        result = runner.independent_webhook_oracle(specs)
        self.assertEqual(result["audited_review_ids"], [400, 401])
        self.assertEqual(result["audited_dismissal_ids"], [401])
        self.assertEqual(result["active_review_ids"], [])

    def test_nullable_dismissal_tombstone_precedes_delayed_receipt(self) -> None:
        case = self.cases["dismiss-before-submit-cannot-reactivate-dismissed-review"]
        specs = runner.webhook_specs(self.manifest, case)
        dismissal = json.loads(specs[0]["body"])
        self.assertIsNone(dismissal["review"]["commit_id"])
        self.assertIsNone(dismissal["review"]["submitted_at"])
        result = runner.independent_webhook_oracle(specs)
        self.assertEqual(result["ingest_outcomes"], ["applied", "applied"])
        self.assertEqual(result["audited_dismissal_ids"], [401])
        self.assertEqual(
            result["dismissal_metadata"],
            [{"review_id": 401, "commit_id": None, "submitted_at": None}],
        )
        self.assertEqual(result["active_review_ids"], [])

    def test_authoritative_head_reducer_ignores_arrival_order(self) -> None:
        case = self.cases["out-of-order-synchronize-does-not-roll-back-current-head"]
        result = runner.independent_head_oracle(case)
        self.assertEqual(result["audited_transitions"], ["H0->H1", "H1->H2"])
        self.assertEqual(result["effective_base"], "C")
        self.assertEqual(result["effective_head"], "H2")
        self.assertEqual(
            result["reconciliation_checks"],
            ["disconnected_history_rejected", "stale_head_rejected"],
        )

    def test_historical_transition_base_does_not_override_current_base(self) -> None:
        case = self.cases["out-of-order-synchronize-does-not-roll-back-current-head"]
        deliveries = case["fixture"]["deliveries_in_arrival_order"]
        self.assertEqual(deliveries[0]["payload_base"], "C")
        self.assertEqual(deliveries[1]["payload_base"], "A")
        result = runner.independent_head_oracle(case)
        self.assertEqual(result["effective_base"], "C")

    def test_distinct_later_review_reactivates_coverage(self) -> None:
        case = self.cases["distinct-new-review-reactivates-coverage"]
        specs = runner.webhook_specs(self.manifest, case)
        result = runner.independent_webhook_oracle(specs)
        self.assertEqual(result["active_review_ids"], [402])

    def test_four_way_oracle_distinguishes_carry_from_followup(self) -> None:
        carried = runner.coverage_oracle(
            self.cases["noninteracting-base-drift-carries-by-four-way-replay"]
        )
        residue = runner.coverage_oracle(
            self.cases["genuine-author-edit-remains-owner-residue"]
        )
        self.assertEqual(carried["carried_paths"], ["src/shared.py"])
        self.assertEqual(carried["residue_paths"], [])
        self.assertEqual(residue["carried_paths"], [])
        self.assertEqual(residue["residue_paths"], ["src/shared.py"])

    def test_owner_alternatives_are_or_not_and(self) -> None:
        result = runner.coverage_oracle(
            self.cases["one-owner-alternative-satisfies-winning-rule"]
        )
        self.assertEqual(result["satisfying_owner"], "@alice")
        self.assertEqual(result["coverage"], "covered")
        self.assertEqual(result["required_check"], "green")

    def test_team_blockers_are_derived_from_snapshot_facts(self) -> None:
        expectations = {
            "missing-codeowner-team-fails-closed": "team_not_found",
            "secret-codeowner-team-fails-closed": "team_not_visible",
            "read-only-codeowner-team-fails-closed": "insufficient_repository_permission",
            "pending-only-team-membership-fails-closed": "no_eligible_team_members",
        }
        for case_id, blocker in expectations.items():
            with self.subTest(case_id=case_id):
                result = runner.coverage_oracle(self.cases[case_id])
                self.assertEqual(result["blockers"], [blocker])
                self.assertEqual(result["eligible_reviewer_ids"], [])


if __name__ == "__main__":
    unittest.main()
