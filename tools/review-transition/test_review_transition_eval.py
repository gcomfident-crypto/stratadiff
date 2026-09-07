#!/usr/bin/env python3

import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import review_transition as selector
import review_transition_eval as evaluator
import review_transition_materialize as materializer


class ReviewTransitionObservationTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.plan = selector.build_plan()
        cls.plan_raw = selector.canonical_json_bytes(cls.plan)
        cls.protocol_raw, cls.protocol = evaluator.load_protocol()
        evaluator.validate_plan_protocol(cls.plan_raw, cls.plan, cls.protocol)

    @staticmethod
    def requested_base(case):
        return hashlib.sha256(f"requested-base:{case['case_id']}".encode("utf-8")).hexdigest()[:40]

    @classmethod
    def provider_responses(cls):
        responses = {}
        for case in cls.plan["selected_cases"]:
            repository = case["repository"]
            number = case["pull_request"]["number"]
            base_oid = cls.requested_base(case)
            pull_path = f"/repos/{repository}/pulls/{number}"
            responses[pull_path] = {
                "base": {"sha": base_oid},
                "head": {"sha": case["final_head_oid"]},
                "merged": True,
                "node_id": case["pull_request"]["node_id"],
                "number": number,
                "state": "closed",
                "updated_at": "2026-09-06T00:00:00Z",
            }
            for label, head_oid in (
                ("checkpoint", case["checkpoint"]["commit_oid"]),
                ("head", case["final_head_oid"]),
            ):
                path = f"/repos/{repository}/compare/{base_oid}...{head_oid}?per_page=1&page=2"
                responses[path] = {
                    "ahead_by": 1,
                    "base_commit": {"sha": base_oid},
                    "behind_by": 0,
                    "merge_base_commit": {"sha": base_oid},
                    "status": "ahead",
                    "total_commits": 1,
                }
        return responses

    @staticmethod
    def response_bytes(value):
        return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")

    def first_observed_row(self):
        case = self.plan["selected_cases"][0]
        responses = self.provider_responses()

        def request_json(path):
            value = responses[path]
            return self.response_bytes(value), value

        return evaluator.observed_case(case, request_json)

    def observation_with_first_row(self):
        observation = evaluator.new_observation(self.plan_raw, self.plan, self.protocol_raw)
        observation["cases"][0] = self.first_observed_row()
        observation["summary"] = evaluator.observation_summary(observation["cases"])
        observation["updated_at"] = observation["cases"][0]["observed_at"]
        return observation

    def complete_observation(self):
        responses = self.provider_responses()

        def request_json(path):
            value = responses[path]
            return self.response_bytes(value), value

        observation = evaluator.new_observation(self.plan_raw, self.plan, self.protocol_raw)
        observation["cases"] = [
            evaluator.observed_case(case, request_json) for case in self.plan["selected_cases"]
        ]
        observation["summary"] = evaluator.observation_summary(observation["cases"])
        observation["complete"] = True
        observation["updated_at"] = observation["cases"][-1]["observed_at"]
        return observation

    def test_observed_row_binds_q_b_d_and_same_base_route(self):
        row = self.first_observed_row()
        case = self.plan["selected_cases"][0]
        base_oid = self.requested_base(case)
        self.assertEqual(row["pull_request"]["base_oid"], base_oid)
        self.assertEqual(row["checkpoint_oid"], case["checkpoint"]["commit_oid"])
        self.assertEqual(row["final_head_oid"], case["final_head_oid"])
        self.assertEqual(row["comparisons"]["checkpoint"]["merge_base_oid"], base_oid)
        self.assertEqual(row["comparisons"]["head"]["merge_base_oid"], base_oid)
        self.assertEqual(row["route"], "provider_attested_same_base")

        observation = self.observation_with_first_row()
        evaluator.validate_observation(
            observation,
            self.plan_raw,
            self.plan,
            self.protocol_raw,
            require_complete=False,
        )

    def test_compare_response_must_name_requested_base(self):
        case = self.plan["selected_cases"][0]
        base_oid = self.requested_base(case)
        response = {
            "ahead_by": 1,
            "base_commit": {"sha": "f" * 40},
            "behind_by": 0,
            "merge_base_commit": {"sha": base_oid},
            "status": "ahead",
            "total_commits": 1,
        }
        with self.assertRaisesRegex(ValueError, "response base differs"):
            evaluator.summarize_compare_response(
                case["case_id"],
                base_oid,
                case["final_head_oid"],
                response,
                "head",
            )

    def test_binding_tamper_is_rejected(self):
        observation = self.observation_with_first_row()
        observation["cases"][0]["comparisons"]["head"]["ahead_by"] = 2
        observation["cases"][0]["comparisons"]["head"]["total_commits"] = 2
        with self.assertRaisesRegex(ValueError, "head binding differs"):
            evaluator.validate_observation(
                observation,
                self.plan_raw,
                self.plan,
                self.protocol_raw,
                require_complete=False,
            )

    def test_raw_response_digest_tamper_is_rejected(self):
        observation = self.observation_with_first_row()
        observation["cases"][0]["provider_response_sha256"]["head"] = "f" * 64
        with self.assertRaisesRegex(ValueError, "head binding differs"):
            evaluator.validate_observation(
                observation,
                self.plan_raw,
                self.plan,
                self.protocol_raw,
                require_complete=False,
            )

    def test_pending_identity_tamper_is_rejected(self):
        observation = evaluator.new_observation(self.plan_raw, self.plan, self.protocol_raw)
        observation["cases"][0]["pull_request"]["number"] += 1
        with self.assertRaisesRegex(ValueError, "pending PR identity differs"):
            evaluator.validate_observation(
                observation,
                self.plan_raw,
                self.plan,
                self.protocol_raw,
                require_complete=False,
            )

    def test_malformed_provider_response_is_recorded_and_resumable(self):
        responses = self.provider_responses()
        first = self.plan["selected_cases"][0]
        base_oid = self.requested_base(first)
        malformed_path = (
            f"/repos/{first['repository']}/compare/{base_oid}..."
            f"{first['checkpoint']['commit_oid']}?per_page=1&page=2"
        )
        malformed = copy.deepcopy(responses)
        del malformed[malformed_path]["base_commit"]

        with tempfile.TemporaryDirectory(prefix="review-transition-observation-test-") as directory:
            directory = Path(directory)
            plan_path = directory / "plan.json"
            output_path = directory / "observation.json"
            plan_path.write_bytes(self.plan_raw)

            def malformed_request(path, token, timeout):
                value = malformed[path]
                return self.response_bytes(value), value

            with mock.patch.object(evaluator, "github_json", side_effect=malformed_request):
                first_run = evaluator.observe(
                    plan_path,
                    evaluator.DEFAULT_PROTOCOL,
                    output_path,
                    None,
                    1,
                )
            self.assertFalse(first_run["complete"])
            self.assertEqual(first_run["summary"]["failed"], 1)
            self.assertEqual(first_run["summary"]["routes"]["provider_metadata_unavailable"], 1)
            self.assertEqual(first_run["cases"][0]["failure"]["phase"], "checkpoint_compare")
            self.assertEqual(first_run["cases"][0]["failure"]["error_class"], "KeyError")

            calls = []

            def valid_request(path, token, timeout):
                calls.append(path)
                value = responses[path]
                return self.response_bytes(value), value

            with mock.patch.object(evaluator, "github_json", side_effect=valid_request):
                second_run = evaluator.observe(
                    plan_path,
                    evaluator.DEFAULT_PROTOCOL,
                    output_path,
                    None,
                    1,
                )
            self.assertTrue(second_run["complete"])
            self.assertEqual(second_run["summary"]["observed"], 30)
            self.assertEqual(len(calls), 3)

    def test_protocol_route_names_match_observation_output(self):
        names = self.protocol["metrics"]["route_distribution"]["required_counts"]
        self.assertIn("provider_attested_same_base", names)
        self.assertIn("requires_full_ancestry", names)
        self.assertIn("provider_metadata_unavailable", names)

    def test_same_version_protocol_tamper_is_rejected(self):
        changed = copy.deepcopy(self.protocol)
        changed["truth_contract"]["oracle_must_not_read_product_output"] = False
        with tempfile.TemporaryDirectory(prefix="review-transition-protocol-test-") as directory:
            path = Path(directory) / "protocol.json"
            path.write_bytes(selector.canonical_json_bytes(changed))
            with self.assertRaisesRegex(ValueError, "protocol digest differs"):
                evaluator.load_protocol(path)

    def test_verify_observation_cli_accepts_complete_bound_artifact(self):
        with tempfile.TemporaryDirectory(prefix="review-transition-cli-test-") as directory:
            directory = Path(directory)
            plan_path = directory / "plan.json"
            observation_path = directory / "observation.json"
            plan_path.write_bytes(self.plan_raw)
            observation_path.write_bytes(selector.canonical_json_bytes(self.complete_observation()))
            result = subprocess.run(
                [
                    sys.executable,
                    str(Path(evaluator.__file__)),
                    "verify-observation",
                    "--plan",
                    str(plan_path),
                    "--observation",
                    str(observation_path),
                ],
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", errors="replace"))
            output = json.loads(result.stdout)
            self.assertTrue(output["observation_verified"])
            self.assertEqual(output["summary"]["observed"], 30)

    def test_materialize_case_is_complete_offline_and_remote_free(self):
        with tempfile.TemporaryDirectory(prefix="review-transition-materialize-test-") as directory:
            root = Path(directory)
            source = root / "source"
            destination = root / "case.git"
            subprocess.run(["git", "init", "--quiet", source], check=True)
            subprocess.run(["git", "-C", source, "config", "user.name", "test"], check=True)
            subprocess.run(["git", "-C", source, "config", "user.email", "test@example.com"], check=True)
            subprocess.run(["git", "-C", source, "config", "uploadpack.allowAnySHA1InWant", "true"], check=True)
            commits = []
            for content in ("base\n", "reviewed\n"):
                (source / "file.txt").write_text(content, encoding="utf-8")
                subprocess.run(["git", "-C", source, "add", "file.txt"], check=True)
                subprocess.run(["git", "-C", source, "commit", "--quiet", "-m", content.strip()], check=True)
                commits.append(subprocess.check_output(["git", "-C", source, "rev-parse", "HEAD"], text=True).strip())
            subprocess.run(["git", "-C", source, "branch", "base", commits[0]], check=True)
            subprocess.run(["git", "-C", source, "branch", "checkpoint"], check=True)
            subprocess.run(["git", "-C", source, "checkout", "--quiet", "--detach", commits[0]], check=True)
            (source / "file.txt").write_text("current\n", encoding="utf-8")
            subprocess.run(["git", "-C", source, "commit", "--all", "--quiet", "-m", "current"], check=True)
            head = subprocess.check_output(["git", "-C", source, "rev-parse", "HEAD"], text=True).strip()
            subprocess.run(["git", "-C", source, "branch", "current"], check=True)
            row = {
                "case_id": "fixture",
                "repository": "local/fixture",
                "pull_request": {"base_oid": commits[0]},
                "checkpoint_oid": commits[1],
                "final_head_oid": head,
            }
            case = materializer.materialize_case(
                row,
                destination,
                None,
                30,
                100_000_000,
                remote_url=str(source),
            )
            self.assertEqual(case["snapshots"]["A"]["commit_oid"], commits[0])
            self.assertEqual(case["snapshots"]["C"]["commit_oid"], commits[0])
            self.assertTrue(case["required_blob_oids"])
            self.assertEqual(subprocess.check_output(["git", "-C", destination, "remote"]), b"")
            self.assertEqual(
                subprocess.check_output(["git", "-C", destination, "rev-parse", "--is-shallow-repository"]).strip(),
                b"false",
            )


if __name__ == "__main__":
    unittest.main()
