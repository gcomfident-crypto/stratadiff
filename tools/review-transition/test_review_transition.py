#!/usr/bin/env python3

import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
TOOL = Path(__file__).with_name("review_transition.py")
EVALUATION_PROTOCOL = Path(__file__).with_name("evaluation-protocol-v1.json")
CENSUS = ROOT / "benchmarks" / "review-churn-census-v1"


def canonical_json(value: object) -> bytes:
    return (
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")


def changed_hex(value: str) -> str:
    replacement = "1" if value[0] == "0" else "0"
    return replacement + value[1:]


def selected_identity(case: dict[str, object]) -> tuple[object, ...]:
    pull_request = case["pull_request"]
    checkpoint = case["checkpoint"]
    assert isinstance(pull_request, dict)
    assert isinstance(checkpoint, dict)
    return (
        case["case_id"],
        case["repository"],
        pull_request["node_id"],
        pull_request["number"],
        case["reviewer_key"],
        checkpoint["review_node_id"],
        checkpoint["state"],
        checkpoint["completed_state"],
        checkpoint["commit_oid"],
        case["final_head_oid"],
        case["repository_spdx_id"],
        case["selection_digest"],
    )


class ReviewTransitionCliTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.temporary_directory = tempfile.TemporaryDirectory(
            prefix="stratadiff-review-transition-test-"
        )
        cls.addClassCleanup(cls.temporary_directory.cleanup)
        cls.work = Path(cls.temporary_directory.name)
        cls.baseline_path = cls.work / "baseline.json"
        result = cls.run_select(cls.baseline_path)
        if result.returncode != 0:
            raise AssertionError(
                "default ReviewTransition-30 selection failed:\n"
                + cls.command_output(result)
            )
        cls.baseline_bytes = cls.baseline_path.read_bytes()
        cls.baseline = json.loads(cls.baseline_bytes)

    @staticmethod
    def command_output(result: subprocess.CompletedProcess[str]) -> str:
        return (result.stdout + "\n" + result.stderr).strip()

    @classmethod
    def run_tool(cls, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, "-B", str(TOOL), *arguments],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    @classmethod
    def run_select(
        cls,
        output: Path,
        *,
        count: int = 30,
        extra_arguments: tuple[str, ...] = (),
    ) -> subprocess.CompletedProcess[str]:
        return cls.run_tool(
            "select",
            "--count",
            str(count),
            "--output",
            str(output),
            *extra_arguments,
        )

    def assert_success(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertEqual(result.returncode, 0, self.command_output(result))

    def test_real_default_census_selects_exactly_thirty_cases(self) -> None:
        self.assertEqual(
            self.baseline["schema"], "stratadiff-review-transition-plan-v1"
        )
        self.assertEqual(self.baseline["dataset_version"], "0.1.0")
        self.assertEqual(len(self.baseline["selected_cases"]), 30)

        selected_frame = [
            pair
            for pair in self.baseline["reviewer_pair_frame"]
            if pair["disposition"] == "selected"
        ]
        self.assertEqual(len(selected_frame), 30)

    def test_frozen_evaluation_protocol_binds_exact_plan_bytes(self) -> None:
        protocol = json.loads(EVALUATION_PROTOCOL.read_bytes())
        self.assertEqual(
            protocol["schema"],
            "stratadiff-review-transition-evaluation-protocol-v1",
        )
        self.assertEqual(protocol["protocol_version"], "1.0.0")
        self.assertEqual(
            protocol["protocol_status"],
            "preregistered_before_reviewtransition_30_product_evaluation",
        )
        cohort = protocol["cohort"]
        self.assertEqual(cohort["selected_cases"], 30)
        self.assertEqual(cohort["replacement_policy"], "forbidden")
        self.assertEqual(
            hashlib.sha256(self.baseline_bytes).hexdigest(),
            cohort["expected_plan_sha256"],
        )

        summary = self.baseline["summary"]
        self.assertEqual(summary["total_pull_requests"], 500)
        self.assertEqual(summary["total_reviewer_pairs"], 589)
        self.assertEqual(
            summary["dispositions"],
            {
                "eligible_not_selected": 25,
                "excluded": 534,
                "selected": 30,
            },
        )
        self.assertEqual(self.baseline["selection"]["eligible_pull_requests"], 47)
        self.assertEqual(self.baseline["selection"]["eligible_reviewer_pairs"], 55)

        self.assertEqual(protocol["metrics"]["false_skip"]["gate"], "zero cases")
        self.assertEqual(protocol["metrics"]["false_carry"]["gate"], "zero")
        self.assertEqual(
            protocol["cache_scope_contract"]["cacheable_unit"],
            "verified_review_input",
        )
        self.assertFalse(protocol["cache_scope_contract"]["verdict_reuse_evaluated"])
        self.assertEqual(
            protocol["metrics"]["route_distribution"]["denominator"],
            "30 selected cases",
        )
        self.assertEqual(
            protocol["metrics"]["ancestry_materialization_failure_rate"]["denominator"],
            "selected cases routed to full-history ancestry verification",
        )
        self.assertTrue(protocol["truth_contract"]["oracle_must_not_read_product_output"])
        self.assertTrue(protocol["materialization_contract"]["unique_merge_base_required"])

    def test_identical_inputs_produce_identical_bytes(self) -> None:
        repeated_path = self.work / "repeated.json"
        result = self.run_select(repeated_path)
        self.assert_success(result)
        self.assertEqual(repeated_path.read_bytes(), self.baseline_bytes)

    def test_output_contains_no_login_or_username_fields(self) -> None:
        forbidden_paths: list[str] = []

        def visit(value: object, path: tuple[str, ...] = ()) -> None:
            if isinstance(value, dict):
                for key, child in value.items():
                    lowered = key.lower()
                    if "login" in lowered or "username" in lowered:
                        forbidden_paths.append(".".join((*path, key)))
                    visit(child, (*path, key))
            elif isinstance(value, list):
                for index, child in enumerate(value):
                    visit(child, (*path, str(index)))

        visit(self.baseline)
        self.assertEqual(forbidden_paths, [])

    def test_verify_rejects_changed_selected_identity(self) -> None:
        baseline_verify = self.run_tool("verify", "--plan", str(self.baseline_path))
        self.assert_success(baseline_verify)

        forged = json.loads(self.baseline_bytes)
        selected = forged["selected_cases"][0]
        selected["checkpoint"]["commit_oid"] = changed_hex(
            selected["checkpoint"]["commit_oid"]
        )
        forged_path = self.work / "forged-selected-identity.json"
        forged_path.write_bytes(canonical_json(forged))

        result = self.run_tool("verify", "--plan", str(forged_path))
        self.assertNotEqual(result.returncode, 0, self.command_output(result))

    def test_changed_sample_without_rebound_hashes_is_rejected(self) -> None:
        sample = json.loads((CENSUS / "sample.json").read_bytes())
        sample["generated_at"] = "2026-09-05T12:22:13Z"
        changed_sample = self.work / "changed-sample.json"
        changed_sample.write_bytes(canonical_json(sample))

        output = self.work / "changed-sample-plan.json"
        result = self.run_select(
            output,
            extra_arguments=("--sample", str(changed_sample)),
        )
        diagnostic = self.command_output(result).lower()
        self.assertNotEqual(result.returncode, 0, diagnostic)
        self.assertIn("sample", diagnostic)
        self.assertIn("sha256", diagnostic)

    def test_product_result_is_blind_to_selection(self) -> None:
        first_manifest = json.loads((CENSUS / "manifest.json").read_bytes())
        first_manifest["product_result"] = {
            "metric": "transition_recovery",
            "value": 0.0,
        }
        second_manifest = json.loads((CENSUS / "manifest.json").read_bytes())
        second_manifest["product_result"] = {
            "metric": "transition_recovery",
            "value": 1.0,
        }

        first_manifest_path = self.work / "manifest-result-zero.json"
        second_manifest_path = self.work / "manifest-result-one.json"
        first_manifest_path.write_bytes(canonical_json(first_manifest))
        second_manifest_path.write_bytes(canonical_json(second_manifest))
        self.assertNotEqual(
            first_manifest_path.read_bytes(), second_manifest_path.read_bytes()
        )

        first_plan_path = self.work / "result-zero-plan.json"
        second_plan_path = self.work / "result-one-plan.json"
        first_result = self.run_select(
            first_plan_path,
            extra_arguments=("--manifest", str(first_manifest_path)),
        )
        second_result = self.run_select(
            second_plan_path,
            extra_arguments=("--manifest", str(second_manifest_path)),
        )
        self.assert_success(first_result)
        self.assert_success(second_result)

        first_plan = json.loads(first_plan_path.read_bytes())
        second_plan = json.loads(second_plan_path.read_bytes())
        first_identities = [
            selected_identity(case) for case in first_plan["selected_cases"]
        ]
        second_identities = [
            selected_identity(case) for case in second_plan["selected_cases"]
        ]
        self.assertEqual(first_identities, second_identities)

    def test_selected_cases_contain_at_most_one_reviewer_per_pull_request(self) -> None:
        pull_requests = [
            (case["repository"], case["pull_request"]["node_id"])
            for case in self.baseline["selected_cases"]
        ]
        self.assertEqual(len(pull_requests), len(set(pull_requests)))

    def test_count_above_eligible_pull_requests_fails_clearly(self) -> None:
        eligible_pull_requests = {
            pair["case_id"]
            for pair in self.baseline["reviewer_pair_frame"]
            if pair["disposition"] != "excluded"
        }
        output = self.work / "too-many.json"
        result = self.run_select(output, count=len(eligible_pull_requests) + 1)
        diagnostic = self.command_output(result).lower()
        self.assertNotEqual(result.returncode, 0, diagnostic)
        self.assertIn("eligible", diagnostic)

    def test_rebound_but_incomplete_capture_is_rejected(self) -> None:
        capture = json.loads((CENSUS / "capture.json").read_bytes())
        capture["capture_complete"] = False
        capture_path = self.work / "incomplete-capture.json"
        capture_bytes = canonical_json(capture)
        capture_path.write_bytes(capture_bytes)

        manifest = json.loads((CENSUS / "manifest.json").read_bytes())
        manifest["capture_sha256"] = hashlib.sha256(capture_bytes).hexdigest()
        manifest_path = self.work / "incomplete-capture-manifest.json"
        manifest_path.write_bytes(canonical_json(manifest))

        output = self.work / "incomplete-capture-plan.json"
        result = self.run_select(
            output,
            extra_arguments=(
                "--capture",
                str(capture_path),
                "--manifest",
                str(manifest_path),
            ),
        )
        diagnostic = self.command_output(result).lower()
        self.assertNotEqual(result.returncode, 0, diagnostic)
        self.assertIn("complete", diagnostic)

    def test_commented_review_cannot_masquerade_as_completed_checkpoint(self) -> None:
        capture = json.loads((CENSUS / "capture.json").read_bytes())
        manifest = json.loads((CENSUS / "manifest.json").read_bytes())
        cases = {case["id"]: case for case in capture["cases"]}
        forged = False
        for pull_request in manifest["pull_requests"]:
            case = cases[pull_request["id"]]
            for pair in pull_request["reviewer_pairs"]:
                checkpoint = pair["latest_completed_checkpoint"]
                if checkpoint is None:
                    continue
                candidates = [
                    review
                    for review in case["reviews"]
                    if review["author"]["actor_key"] == pair["reviewer_key"]
                    and review["author"]["typename"] == "User"
                    and review["state"] == "COMMENTED"
                ]
                if not candidates:
                    continue
                review = candidates[-1]
                checkpoint["review_id"] = review["node_id"]
                checkpoint["submitted_at"] = review["submitted_at"]
                checkpoint["commit_oid"] = review["commit_oid"]
                checkpoint["current_state"] = "COMMENTED"
                checkpoint["dismissed"] = False
                checkpoint["dismissal_event_id"] = None
                checkpoint["differs_from_final_head"] = review["commit_oid"] != pull_request["final_head_oid"]
                forged = True
                break
            if forged:
                break
        self.assertTrue(forged, "fixture has no completed reviewer with a COMMENTED review")

        manifest_path = self.work / "commented-checkpoint-manifest.json"
        manifest_path.write_bytes(canonical_json(manifest))
        output = self.work / "commented-checkpoint-plan.json"
        result = self.run_select(
            output,
            extra_arguments=("--manifest", str(manifest_path)),
        )
        diagnostic = self.command_output(result).lower()
        self.assertNotEqual(result.returncode, 0, diagnostic)
        self.assertIn("checkpoint", diagnostic)

    def test_non_finite_json_number_is_rejected(self) -> None:
        manifest_text = (CENSUS / "manifest.json").read_text(encoding="utf-8")
        malformed_path = self.work / "non-finite-manifest.json"
        malformed_path.write_text(
            manifest_text.replace("{", '{\n  "product_result": NaN,', 1),
            encoding="utf-8",
        )
        output = self.work / "non-finite-plan.json"
        result = self.run_select(
            output,
            extra_arguments=("--manifest", str(malformed_path)),
        )
        diagnostic = self.command_output(result).lower()
        self.assertNotEqual(result.returncode, 0, diagnostic)
        self.assertIn("non-finite", diagnostic)


if __name__ == "__main__":
    unittest.main()
