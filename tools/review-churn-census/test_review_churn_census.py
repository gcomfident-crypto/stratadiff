#!/usr/bin/env python3

from datetime import datetime, timezone
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("review_churn_census.py")
SPEC = importlib.util.spec_from_file_location("review_churn_census", MODULE_PATH)
assert SPEC is not None
assert SPEC.loader is not None
census = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(census)

OID_A = "a" * 40
OID_B = "b" * 40
OID_C = "c" * 40


def actor(typename, identity):
    return {
        "typename": typename,
        "actor_key": None
        if identity is None
        else census.opaque_actor_key("github-acme-widgets-pr-7", identity),
    }


def review(review_id, database_id, state, submitted_at, author, commit_oid, comment_count=0):
    return {
        "node_id": review_id,
        "database_id": str(database_id),
        "state": state,
        "submitted_at": submitted_at,
        "author": author,
        "commit_oid": commit_oid,
        "comment_count": comment_count,
    }


def capture_case(
    reviews,
    events,
    *,
    pr_author="alice",
    head_oid=OID_C,
    number=7,
    merged_at="2026-08-01T06:00:00Z",
):
    return {
        "id": f"github-acme-widgets-pr-{number}",
        "repository": {
            "node_id": "R_1",
            "owner": "acme",
            "name": "widgets",
            "name_with_owner": "acme/widgets",
            "url": "https://github.com/acme/widgets",
        },
        "pull_request": {
            "node_id": f"PR_{number}",
            "number": number,
            "merged_at": merged_at,
            "head_oid": head_oid,
            "last_commit_oid": head_oid,
            "commit_count": 3,
            "head_matches_last_commit": None if head_oid is None else True,
            "author": None if pr_author is None else actor("User", pr_author),
        },
        "reviews": reviews,
        "timeline_events": events,
        "pagination": {
            "reviews": {
                "pages": 1,
                "reported_total_count": len(reviews),
                "captured_node_count": len(reviews),
                "pagination_complete": True,
            },
            "timeline_events": {
                "pages": 1,
                "reported_total_count_unfiltered": len(events),
                "captured_filtered_node_count": len(events),
                "pagination_complete": True,
            },
        },
    }


def valid_empty_sample(plan_bytes, plan):
    repositories = []
    target = plan["target_pull_requests_per_repository"]
    for repository in plan["repositories"]:
        repositories.append(
            {
                "owner": repository["owner"],
                "name": repository["name"],
                "name_with_owner": f"{repository['owner']}/{repository['name']}",
                "candidate_count": 0,
                "requested_count": target,
                "selected_count": 0,
                "shortfall": target,
                "candidates": [],
                "selected_pull_request_numbers": [],
            }
        )
    return {
        "schema": census.SAMPLE_SCHEMA,
        "dataset_version": census.DATASET_VERSION,
        "tool_version": census.TOOL_VERSION,
        "generated_at": "2026-09-05T00:00:00Z",
        "source_plan": {
            "schema": plan["schema"],
            "dataset_version": plan["dataset_version"],
            "sha256": census.sha256_bytes(plan_bytes),
        },
        "selection": {
            "algorithm": plan["selection"]["algorithm"],
            "algorithm_version": plan["selection"]["algorithm_version"],
            "seed_hex": plan["selection"]["seed_hex"],
        },
        "merged_at_window": dict(plan["merged_at_window"]),
        "repositories": repositories,
        "summary": {
            "repositories": len(repositories),
            "candidate_pull_requests": 0,
            "requested_pull_requests": len(repositories) * target,
            "selected_pull_requests": 0,
            "sampling_shortfall": len(repositories) * target,
        },
        "acquisition": {
            "graphql_calls": 0,
            "minimum_rate_limit_remaining": None,
            "last_rate_limit_reset_at": None,
        },
    }


def valid_one_sample(plan_bytes, plan):
    sample = valid_empty_sample(plan_bytes, plan)
    repository = sample["repositories"][0]
    candidate = {
        "node_id": "PR_7",
        "number": 7,
        "merged_at": "2026-08-01T06:00:00Z",
    }
    inventory, selected = census.select_candidates(
        [candidate],
        plan["selection"]["seed_hex"],
        repository["name_with_owner"],
        plan["target_pull_requests_per_repository"],
    )
    repository["candidates"] = inventory
    repository["selected_pull_request_numbers"] = selected
    repository["candidate_count"] = 1
    repository["selected_count"] = 1
    repository["shortfall"] -= 1
    sample["summary"]["candidate_pull_requests"] = 1
    sample["summary"]["selected_pull_requests"] = 1
    sample["summary"]["sampling_shortfall"] -= 1
    return sample


class SelectionTest(unittest.TestCase):
    def test_selection_is_case_insensitive_and_deterministic(self):
        candidates = [
            {"node_id": f"PR_{number}", "number": number, "merged_at": "2026-08-01T00:00:00Z"}
            for number in (8, 2, 5, 1)
        ]
        seed = "01" * 32
        first_inventory, first_selected = census.select_candidates(candidates, seed, "Acme/Widgets", 2)
        second_inventory, second_selected = census.select_candidates(
            list(reversed(candidates)), seed, "acme/widgets", 2
        )
        self.assertEqual(first_inventory, second_inventory)
        self.assertEqual(first_selected, second_selected)

    def test_duplicate_candidate_across_pages_is_rejected(self):
        raw = {
            "id": "PR_1",
            "number": 1,
            "state": "MERGED",
            "mergedAt": "2026-08-01T01:00:00Z",
            "repository": {"nameWithOwner": "acme/widgets"},
        }

        class FakeApi:
            def __init__(self):
                self.calls = 0

            def call(self, query, variables):
                self.calls += 1
                return {
                    "search": {
                        "issueCount": 2,
                        "nodes": [raw],
                        "pageInfo": {
                            "hasNextPage": self.calls == 1,
                            "endCursor": "cursor-1" if self.calls == 1 else None,
                        },
                    }
                }

        with self.assertRaisesRegex(census.CensusError, "duplicate candidate"):
            census.search_repository_candidates(
                FakeApi(),
                "acme",
                "widgets",
                datetime(2026, 8, 1, tzinfo=timezone.utc),
                datetime(2026, 8, 2, tzinfo=timezone.utc),
            )

    def test_sample_requested_count_and_window_are_strict(self):
        plan_bytes, plan = census.read_json(census.DEFAULT_PLAN)
        sample = valid_one_sample(plan_bytes, plan)
        census.validate_sample(sample, plan_bytes, plan)
        sample["repositories"][0]["requested_count"] = 99
        with self.assertRaisesRegex(census.CensusError, "requested count differs"):
            census.validate_sample(sample, plan_bytes, plan)


class ClassificationTest(unittest.TestCase):
    def test_completed_checkpoints_are_per_reviewer(self):
        bob_approval = review(
            "bob-approved", 10, "APPROVED", "2026-08-01T01:00:00Z", actor("User", "bob"), OID_A
        )
        bob_comment = review(
            "bob-commented", 11, "COMMENTED", "2026-08-01T03:00:00Z", actor("User", "bob"), OID_B, 1
        )
        carol_comment = review(
            "carol-commented", 12, "COMMENTED", "2026-08-01T04:00:00Z", actor("User", "carol"), OID_B, 1
        )
        dave_dismissed = review(
            "dave-dismissed", 13, "DISMISSED", "2026-08-01T01:10:00Z", actor("User", "dave"), OID_A
        )
        erin_first = review(
            "erin-first", 14, "CHANGES_REQUESTED", "2026-08-01T00:30:00Z", actor("User", "erin"), OID_A
        )
        erin_rereview = review(
            "erin-rereview", 15, "APPROVED", "2026-08-01T05:00:00Z", actor("User", "erin"), OID_C
        )
        events = [
            {
                "node_id": "force-1",
                "type": "HeadRefForcePushedEvent",
                "created_at": "2026-08-01T02:00:00Z",
                "before_oid": OID_A,
                "after_oid": OID_B,
            },
            {
                "node_id": "dismiss-1",
                "type": "ReviewDismissedEvent",
                "created_at": "2026-08-01T01:20:00Z",
                "previous_review_state": "APPROVED",
                "review": dave_dismissed,
            },
        ]
        result = census.classify_case(
            capture_case(
                [
                    review("self", 1, "COMMENTED", "2026-08-01T00:10:00Z", actor("User", "alice"), OID_A),
                    review("bot", 2, "COMMENTED", "2026-08-01T00:20:00Z", actor("Bot", "robot"), OID_A),
                    bob_approval,
                    bob_comment,
                    carol_comment,
                    dave_dismissed,
                    erin_first,
                    erin_rereview,
                ],
                events,
            )
        )
        pairs = {pair["reviewer_key"]: pair for pair in result["reviewer_pairs"]}
        bob = pairs[actor("User", "bob")["actor_key"]]
        self.assertEqual(bob["latest_completed_checkpoint"]["review_id"], "bob-approved")
        self.assertTrue(bob["commented_newer_commit_candidate"])
        self.assertTrue(pairs[actor("User", "carol")["actor_key"]]["commented_only"])
        dave = pairs[actor("User", "dave")["actor_key"]]
        self.assertEqual(dave["latest_completed_checkpoint"]["completed_state"], "APPROVED")
        self.assertTrue(dave["latest_completed_checkpoint"]["dismissed"])
        erin = pairs[actor("User", "erin")["actor_key"]]
        self.assertEqual(erin["latest_completed_checkpoint"]["commit_oid"], OID_C)
        self.assertTrue(erin["latest_completed_checkpoint"]["post_completed_review_force_push"])
        self.assertTrue(erin["latest_completed_checkpoint"]["force_push_rereview"])
        self.assertEqual(result["counts"]["completed_review_pairs"], 3)
        self.assertEqual(result["counts"]["drifted_checkpoint_pairs"], 2)
        self.assertEqual(result["counts"]["commented_only_pairs"], 1)
        self.assertTrue(result["classification"]["completed_review_dismissal"])
        expected_metrics = {
            "formal_peer_reviewed_pr_rate": (1, 1),
            "completed_review_pr_rate": (1, 1),
            "checkpoint_oid_observability_rate": (3, 3),
            "checkpoint_pair_head_drift_rate": (2, 3),
            "completed_review_pair_post_force_push_rate": (3, 3),
            "checkpoint_pair_drift_without_observed_force_push_rate": (0, 3),
            "stranded_reviewer_pr_rate": (1, 1),
            "multi_round_completed_review_pr_rate": (1, 1),
            "completed_review_dismissal_pr_rate": (1, 1),
            "commented_only_pair_share": (1, 4),
            "commented_newer_commit_candidate_pair_rate": (1, 3),
            "completed_review_pair_force_push_rereview_rate": (1, 3),
            "bot_review_session_share": (1, 7),
        }
        for metric_id, expected in expected_metrics.items():
            with self.subTest(metric_id=metric_id):
                self.assertEqual(census.metric_counts([result], metric_id), expected)

    def test_latest_completed_missing_oid_never_falls_back(self):
        result = census.classify_case(
            capture_case(
                [
                    review("older", 1, "APPROVED", "2026-08-01T01:00:00Z", actor("User", "bob"), OID_A),
                    review("latest", 2, "CHANGES_REQUESTED", "2026-08-01T02:00:00Z", actor("User", "bob"), None),
                ],
                [],
            )
        )
        checkpoint = result["reviewer_pairs"][0]["latest_completed_checkpoint"]
        self.assertEqual(checkpoint["review_id"], "latest")
        self.assertIsNone(checkpoint["commit_oid"])
        self.assertIsNone(checkpoint["differs_from_final_head"])

    def test_earlier_force_push_does_not_explain_drift_after_latest_checkpoint(self):
        result = census.classify_case(
            capture_case(
                [
                    review("first", 1, "APPROVED", "2026-08-01T01:00:00Z", actor("User", "bob"), OID_A),
                    review("latest", 2, "APPROVED", "2026-08-01T03:00:00Z", actor("User", "bob"), OID_B),
                ],
                [
                    {
                        "node_id": "force-1",
                        "type": "HeadRefForcePushedEvent",
                        "created_at": "2026-08-01T02:00:00Z",
                        "before_oid": OID_A,
                        "after_oid": OID_B,
                    }
                ],
            )
        )
        checkpoint = result["reviewer_pairs"][0]["latest_completed_checkpoint"]
        self.assertTrue(checkpoint["post_completed_review_force_push"])
        self.assertFalse(checkpoint["post_latest_checkpoint_force_push"])
        self.assertEqual(
            census.metric_counts(
                [result], "checkpoint_pair_drift_without_observed_force_push_rate"
            ),
            (1, 1),
        )

    def test_missing_pr_author_keeps_user_out_of_peer_metrics(self):
        result = census.classify_case(
            capture_case(
                [review("review-1", 1, "APPROVED", "2026-08-01T01:00:00Z", actor("User", "bob"), OID_A)],
                [],
                pr_author=None,
            )
        )
        self.assertEqual(result["counts"]["all_human_review_sessions"], 1)
        self.assertEqual(result["counts"]["peer_human_review_sessions"], 0)
        self.assertEqual(result["counts"]["unknown_review_sessions"], 1)


class AuditTest(unittest.TestCase):
    start = datetime(2026, 6, 3, tzinfo=timezone.utc)
    end = datetime(2026, 9, 1, tzinfo=timezone.utc)
    acquisition = {
        "graphql_calls": 3,
        "minimum_rate_limit_remaining": 4997,
        "last_rate_limit_reset_at": "2026-09-05T01:00:00Z",
    }

    def build_report(self, cases):
        return census.build_review_memory_audit(
            "acme/widgets",
            "ghe.example",
            self.start,
            self.end,
            50,
            len(cases),
            cases,
            self.acquisition,
            "2026-09-05T00:00:00Z",
        )

    def classified(self, checkpoint_oid=OID_A, *, head_oid=OID_C):
        return census.classify_case(
            capture_case(
                [
                    review(
                        "review-1",
                        1,
                        "APPROVED",
                        "2026-08-01T01:00:00Z",
                        actor("User", "bob"),
                        checkpoint_oid,
                    )
                ],
                [],
                head_oid=head_oid,
            )
        )

    def test_latest_selection_orders_by_merge_time_then_number(self):
        candidates = [
            {"node_id": "PR_3", "number": 3, "merged_at": "2026-08-03T00:00:00Z"},
            {"node_id": "PR_9", "number": 9, "merged_at": "2026-08-04T00:00:00Z"},
            {"node_id": "PR_7", "number": 7, "merged_at": "2026-08-04T00:00:00Z"},
            {"node_id": "PR_8", "number": 8, "merged_at": "2026-08-02T00:00:00Z"},
        ]
        selected = census.select_latest_candidates(candidates, 3)
        self.assertEqual([candidate["number"] for candidate in selected], [9, 7, 3])

    def test_collection_captures_only_newest_candidates_in_order(self):
        candidates = [
            {"node_id": "PR_3", "number": 3, "merged_at": "2026-08-03T00:00:00Z"},
            {"node_id": "PR_9", "number": 9, "merged_at": "2026-08-04T00:00:00Z"},
            {"node_id": "PR_7", "number": 7, "merged_at": "2026-08-04T00:00:00Z"},
        ]

        class FakeApi:
            def acquisition(self):
                return AuditTest.acquisition

        def captured(_api, _owner, _name, number):
            merged_at = next(
                candidate["merged_at"]
                for candidate in candidates
                if candidate["number"] == number
            )
            return capture_case([], [], number=number, merged_at=merged_at)

        with mock.patch.object(
            census, "search_repository_candidates", return_value=candidates
        ), mock.patch.object(census, "capture_pull_request", side_effect=captured) as capture:
            report = census.collect_review_memory_audit(
                FakeApi(),
                "acme/widgets",
                "github.com",
                self.start,
                self.end,
                2,
            )
        self.assertEqual(
            [call.args[3] for call in capture.call_args_list],
            [9, 7],
        )
        self.assertEqual(report["scope"]["selection"]["candidate_count"], 3)
        self.assertEqual(report["scope"]["selection"]["selected_count"], 2)
        self.assertEqual(report["scope"]["selection"]["shortfall"], 0)

    def test_collection_does_not_publish_a_partial_report(self):
        candidates = [
            {"node_id": "PR_7", "number": 7, "merged_at": "2026-08-04T00:00:00Z"}
        ]

        class FakeApi:
            def acquisition(self):
                return AuditTest.acquisition

        with mock.patch.object(
            census, "search_repository_candidates", return_value=candidates
        ), mock.patch.object(
            census,
            "capture_pull_request",
            side_effect=census.CensusError("capture failed"),
        ):
            with self.assertRaisesRegex(census.CensusError, "capture failed"):
                census.collect_review_memory_audit(
                    FakeApi(),
                    "acme/widgets",
                    "github.com",
                    self.start,
                    self.end,
                    1,
                )

    def test_report_contract_metrics_findings_and_privacy(self):
        report = self.build_report([self.classified()])
        self.assertEqual(
            set(report),
            {
                "schema",
                "tool_version",
                "generated_at",
                "scope",
                "collection",
                "privacy",
                "claim_boundary",
                "summary",
                "descriptive_metrics",
                "findings",
            },
        )
        self.assertEqual(report["schema"], "stratadiff-review-memory-audit-v1")
        self.assertEqual(report["tool_version"], census.AUDIT_TOOL_VERSION)
        self.assertEqual(
            report["scope"]["selection"],
            {
                "method": "latest_merged_at_desc_v1",
                "requested_limit": 50,
                "candidate_count": 1,
                "selected_count": 1,
                "shortfall": 49,
            },
        )
        self.assertEqual(report["summary"]["status"], "affected")
        self.assertEqual(report["summary"]["drifted_reviewer_pairs"], 1)
        self.assertEqual(len(report["descriptive_metrics"]), 7)
        self.assertEqual(
            [metric["id"] for metric in report["descriptive_metrics"]],
            list(census.METRIC_IDS[:7]),
        )
        for metric in report["descriptive_metrics"]:
            self.assertEqual(
                set(metric), {"id", "numerator", "denominator", "status", "basis_points"}
            )
        finding = report["findings"][0]
        self.assertEqual(finding["url"], "https://ghe.example/acme/widgets/pull/7")
        self.assertEqual(len(finding["drifted_reviewers"]), 1)
        self.assertEqual(
            finding["drifted_reviewers"][0]["reviewer_key"],
            actor("User", "bob")["actor_key"],
        )
        encoded = census.canonical_json(report).decode("utf-8")
        self.assertNotIn("bob", encoded)
        self.assertNotIn("review-1", encoded)
        self.assertEqual(
            report["privacy"],
            {
                "source_collected": False,
                "pr_text_collected": False,
                "review_text_collected": False,
                "commit_messages_collected": False,
                "logins_persisted": False,
                "actor_identity": "pr_local_opaque_key",
            },
        )

    def test_all_complete_audit_statuses(self):
        no_eligible = self.build_report([census.classify_case(capture_case([], []))])
        insufficient = self.build_report([self.classified(None)])
        no_drift = self.build_report([self.classified(OID_C)])
        affected = self.build_report([self.classified(OID_A)])
        self.assertEqual(no_eligible["summary"]["status"], "no_eligible_reviews")
        self.assertEqual(insufficient["summary"]["status"], "insufficient_evidence")
        self.assertEqual(no_drift["summary"]["status"], "no_observed_drift")
        self.assertEqual(affected["summary"]["status"], "affected")
        self.assertEqual(insufficient["findings"][0]["drifted_reviewers"], [])
        self.assertEqual(insufficient["findings"][0]["unobservable_pair_count"], 1)

        ninety_percent = self.build_report(
            [
                census.classify_case(
                    capture_case(
                        [
                            review(
                                f"review-{index}",
                                index,
                                "APPROVED",
                                "2026-08-01T01:00:00Z",
                                actor("User", f"reviewer-{index}"),
                                None if index == 10 else OID_C,
                            )
                            for index in range(1, 11)
                        ],
                        [],
                    )
                )
            ]
        )
        self.assertEqual(ninety_percent["summary"]["status"], "no_observed_drift")
        self.assertEqual(ninety_percent["summary"]["unobservable_reviewer_pairs"], 1)

        known_drift_with_unknowns = self.build_report(
            [
                census.classify_case(
                    capture_case(
                        [
                            review(
                                f"mixed-review-{index}",
                                index,
                                "APPROVED",
                                "2026-08-01T01:00:00Z",
                                actor("User", f"mixed-reviewer-{index}"),
                                OID_A if index == 1 else None,
                            )
                            for index in range(1, 11)
                        ],
                        [],
                    )
                )
            ]
        )
        self.assertEqual(known_drift_with_unknowns["summary"]["status"], "affected")
        self.assertEqual(
            known_drift_with_unknowns["summary"]["drifted_reviewer_pairs"], 1
        )
        self.assertEqual(
            known_drift_with_unknowns["summary"]["unobservable_reviewer_pairs"], 9
        )

    def test_markdown_is_derived_from_report_and_calls_unknown_out(self):
        report = self.build_report([self.classified(None)])
        rendered = census.render_review_memory_audit_markdown(report)
        self.assertIn("# StrataDiff Review Memory Audit", rendered)
        self.assertIn("`insufficient_evidence`", rendered)
        self.assertIn("Unknown evidence is not evidence of no drift.", rendered)
        self.assertIn("https://ghe.example/acme/widgets/pull/7", rendered)
        self.assertNotIn("bob", rendered)

    def test_audit_argument_defaults_and_invalid_values(self):
        arguments = census.parse_arguments(
            ["audit", "--repository", "acme/widgets", "--hostname", "GHE.EXAMPLE"]
        )
        self.assertEqual(arguments.repository, "acme/widgets")
        self.assertEqual(arguments.hostname, "ghe.example")
        self.assertEqual(arguments.limit, 50)
        self.assertEqual(arguments.days, 90)
        self.assertEqual(arguments.format, "markdown")
        invalid = (
            ["--limit", "0"],
            ["--limit", "101"],
            ["--days", "0"],
            ["--days", "366"],
            ["--repository", "three/part/name"],
            ["--hostname", "https://github.com"],
            ["--end-exclusive", "2026-09-01"],
            ["--days", "365", "--end-exclusive", "0001-01-01T00:00:00Z"],
            ["--format", "yaml"],
        )
        for extra in invalid:
            argv = [
                "audit",
                "--repository",
                "acme/widgets",
                "--hostname",
                "github.com",
                *extra,
            ]
            with self.subTest(extra=extra), mock.patch("sys.stderr", new=io.StringIO()):
                with self.assertRaises(SystemExit) as raised:
                    census.parse_arguments(argv)
                self.assertEqual(raised.exception.code, 2)

    def test_command_writes_canonical_private_json(self):
        report = self.build_report([self.classified()])
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "audit.json"
            arguments = census.argparse.Namespace(
                repository="acme/widgets",
                hostname="github.com",
                limit=50,
                days=90,
                end_exclusive="2026-09-01T00:00:00Z",
                format="json",
                output=output,
                gh="custom-gh",
            )
            with mock.patch.object(
                census, "collect_review_memory_audit", return_value=report
            ) as collect:
                census.command_audit(arguments)
            self.assertEqual(output.read_bytes(), census.canonical_json(report))
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            call = collect.call_args.args
            self.assertEqual(call[0].executable, "custom-gh")
            self.assertEqual(call[0].hostname, "github.com")
            self.assertEqual(call[3], self.start)
            self.assertEqual(call[4], self.end)


class AggregateTest(unittest.TestCase):
    def test_wilson_interval_contains_point_and_zero_is_undefined(self):
        observed = census.ratio(20, 100)
        self.assertLessEqual(observed["wilson_95_lower_basis_points"], 2000)
        self.assertGreaterEqual(observed["wilson_95_upper_basis_points"], 2000)
        self.assertEqual(census.ratio(0, 0)["status"], "undefined")

    def test_zero_case_repository_is_retained(self):
        metrics = census.aggregate_cases([], ["a/one", "b/two"], 100)
        for metric in metrics:
            self.assertEqual([row["repository"] for row in metric["by_repository"]], ["a/one", "b/two"])
            self.assertTrue(all(row["status"] == "undefined" for row in metric["by_repository"]))
            self.assertIsNone(metric["repository_median_basis_points"])

    def test_repository_median_uses_defined_repositories_only(self):
        classified = census.classify_case(
            capture_case(
                [review("approved", 1, "APPROVED", "2026-08-01T01:00:00Z", actor("User", "bob"), OID_A)],
                [],
            )
        )
        metrics = {
            metric["id"]: metric
            for metric in census.aggregate_cases(
                [classified], ["acme/widgets", "empty/repository"], 100
            )
        }
        observed = metrics["formal_peer_reviewed_pr_rate"]
        self.assertEqual(observed["repository_median_basis_points"], 10_000)
        self.assertEqual(
            [row["status"] for row in observed["by_repository"]],
            ["defined", "undefined"],
        )

    def test_signal_with_small_denominator_is_inconclusive(self):
        _, plan = census.read_json(census.DEFAULT_PLAN)
        classified = census.classify_case(
            capture_case(
                [review("approved", 1, "APPROVED", "2026-08-01T01:00:00Z", actor("User", "bob"), OID_A)],
                [],
            )
        )
        manifest = {
            "sampling_plan_sha256": "0" * 64,
            "collection": {
                "status": "complete",
                "captured_at": "2026-09-05T00:00:00Z",
                "selected_pull_requests": 1,
                "classified_pull_requests": 1,
                "capture_failures": 0,
            },
            "repositories": [
                {
                    "name_with_owner": "acme/widgets",
                    "frame_candidates": 1,
                    "target": 50,
                    "selected": 1,
                    "capture_failures": 0,
                }
            ],
            "pull_requests": [classified],
        }
        sample = {"repositories": [{"selected_count": 1}]}
        aggregate = census.build_aggregate(b"manifest\n", manifest, sample, plan)
        for signal in aggregate["signals"].values():
            self.assertFalse(signal["evaluable"])
            self.assertEqual(signal["status"], "inconclusive")

    def test_wilson_signal_can_pass_or_fail_only_after_all_gates(self):
        _, plan = census.read_json(census.DEFAULT_PLAN)
        repository_names = [
            f"{repository['owner']}/{repository['name']}" for repository in plan["repositories"]
        ]
        cases = []
        for index in range(400):
            checkpoint = {
                "commit_oid": OID_A,
                "differs_from_final_head": True,
                "post_completed_review_force_push": True,
                "post_latest_checkpoint_force_push": True,
                "force_push_rereview": False,
            }
            cases.append(
                {
                    "repository": repository_names[index % len(repository_names)],
                    "counts": {"bot_review_sessions": 0, "peer_human_review_sessions": 1},
                    "classification": {
                        "formal_peer_reviewed": True,
                        "completed_reviewed": True,
                        "stranded_reviewer": True,
                        "multi_round_completed_review": False,
                        "completed_review_dismissal": False,
                    },
                    "reviewer_pairs": [
                        {
                            "latest_completed_checkpoint": checkpoint,
                            "commented_only": False,
                            "commented_newer_commit_candidate": True,
                        }
                    ],
                }
            )
        manifest = {
            "sampling_plan_sha256": "0" * 64,
            "collection": {
                "status": "complete",
                "captured_at": "2026-09-05T00:00:00Z",
                "selected_pull_requests": 400,
                "classified_pull_requests": 400,
                "capture_failures": 0,
            },
            "repositories": [
                {
                    "name_with_owner": name,
                    "frame_candidates": 50,
                    "target": 50,
                    "selected": 50,
                    "capture_failures": 0,
                }
                for name in repository_names
            ],
            "pull_requests": cases,
        }
        sample = {"repositories": [{"selected_count": 50} for _ in repository_names]}
        aggregate = census.build_aggregate(b"manifest\n", manifest, sample, plan)
        self.assertEqual(aggregate["signals"]["force_push_wedge"]["status"], "pass")
        self.assertEqual(
            aggregate["signals"]["all_round_review_continuity"]["status"], "fail"
        )
        self.assertEqual(
            aggregate["signals"]["commented_partial_attention"]["status"], "pass"
        )


class CaptureContractTest(unittest.TestCase):
    def test_actor_is_pseudonymized_before_capture(self):
        stored = census.normalize_actor(
            {"__typename": "User", "login": "alice"}, "actor", "github-acme-widgets-pr-7"
        )
        self.assertEqual(set(stored), {"typename", "actor_key"})
        self.assertNotIn("alice", json.dumps(stored))

    def test_head_and_last_commit_mismatch_is_rejected(self):
        raw = {
            "id": "PR_7",
            "number": 7,
            "mergedAt": "2026-08-01T00:00:00Z",
            "headRefOid": OID_A,
            "commits": {"totalCount": 2, "nodes": [{"commit": {"oid": OID_B}}]},
            "author": {"__typename": "User", "login": "alice"},
        }
        with self.assertRaisesRegex(census.CensusError, "differs"):
            census.normalize_captured_pull_request(raw, 7, "github-acme-widgets-pr-7")

    def test_review_total_count_is_enforced(self):
        initial = {
            "totalCount": 2,
            "nodes": [{"id": "only"}],
            "pageInfo": {"hasNextPage": False, "endCursor": None},
        }
        with self.assertRaisesRegex(census.CensusError, "pagination incomplete"):
            census.collect_connection(None, "a", "b", 1, initial, "reviews", "query")

    def test_unknown_capture_field_is_rejected(self):
        plan_bytes, plan = census.read_json(census.DEFAULT_PLAN)
        sample = valid_empty_sample(plan_bytes, plan)
        sample_bytes = census.canonical_json(sample)
        capture = census.capture_document(
            sample_bytes,
            sample,
            [],
            "2026-09-05T00:00:00Z",
            {"graphql_calls": 0, "minimum_rate_limit_remaining": None, "last_rate_limit_reset_at": None},
        )
        capture["access_token"] = "forbidden"
        with self.assertRaisesRegex(census.CensusError, "access_token"):
            census.validate_capture(capture, sample_bytes, sample, require_complete=False)

    def test_manifest_rejects_non_timestamp_and_bool_integer_alias(self):
        plan_bytes, plan = census.read_json(census.DEFAULT_PLAN)
        sample = valid_one_sample(plan_bytes, plan)
        sample_bytes = census.canonical_json(sample)
        case = capture_case([], [])
        first_repository = sample["repositories"][0]
        case["id"] = (
            f"github-{first_repository['owner'].casefold()}-"
            f"{first_repository['name'].casefold()}-pr-7"
        )
        case["repository"]["owner"] = first_repository["owner"]
        case["repository"]["name"] = first_repository["name"]
        case["repository"]["name_with_owner"] = first_repository["name_with_owner"]
        case["repository"]["url"] = f"https://github.com/{first_repository['name_with_owner']}"
        capture = census.capture_document(
            sample_bytes,
            sample,
            [case],
            "2026-09-05T00:00:00Z",
            {"graphql_calls": 1, "minimum_rate_limit_remaining": 4999, "last_rate_limit_reset_at": "2026-09-05T01:00:00Z"},
        )
        capture_bytes = census.canonical_json(capture)
        manifest = census.build_manifest(
            plan_bytes, sample_bytes, sample, capture_bytes, capture
        )
        manifest["generated_at"] = 17
        with self.assertRaisesRegex(census.CensusError, "generated_at"):
            census.validate_manifest(
                manifest, plan_bytes, sample_bytes, sample, capture_bytes, capture
            )
        manifest = census.build_manifest(
            plan_bytes, sample_bytes, sample, capture_bytes, capture
        )
        manifest["collection"]["classified_pull_requests"] = True
        with self.assertRaisesRegex(census.CensusError, "reclassification"):
            census.validate_manifest(
                manifest, plan_bytes, sample_bytes, sample, capture_bytes, capture
            )


class GraphQLTest(unittest.TestCase):
    def test_graphql_uses_explicit_hostname_and_scrubs_debug_environment(self):
        response = {
            "data": {
                "viewer": {"login": "alice"},
                "rateLimit": {
                    "cost": 1,
                    "remaining": 99,
                    "resetAt": "2026-09-05T00:00:00Z",
                },
            }
        }
        completed = subprocess.CompletedProcess(
            ["custom-gh"], 0, stdout=json.dumps(response).encode(), stderr=b""
        )
        with mock.patch.dict(
            os.environ, {"GH_DEBUG": "api", "GH_TRACE": "1"}, clear=False
        ), mock.patch.object(census.subprocess, "run", return_value=completed) as run:
            result = census.GithubGraphQL("custom-gh", "ghe.example").call(
                "query { viewer { login } }", {}
            )
        self.assertEqual(result["viewer"]["login"], "alice")
        arguments, keywords = run.call_args
        self.assertEqual(
            arguments[0],
            [
                "custom-gh",
                "api",
                "--hostname",
                "ghe.example",
                "graphql",
                "--input",
                "-",
            ],
        )
        self.assertNotIn("GH_DEBUG", keywords["env"])
        self.assertNotIn("GH_TRACE", keywords["env"])
        self.assertEqual(keywords["env"]["GH_PROMPT_DISABLED"], "1")

    def test_graphql_errors_are_not_hidden(self):
        response = {
            "data": {"rateLimit": {"cost": 1, "remaining": 99, "resetAt": "2026-09-05T00:00:00Z"}},
            "errors": [{"message": "boom"}],
        }
        completed = subprocess.CompletedProcess(["gh"], 0, stdout=json.dumps(response).encode(), stderr=b"")
        with mock.patch.object(census.subprocess, "run", return_value=completed):
            with self.assertRaisesRegex(census.CensusError, "boom"):
                census.GithubGraphQL().call("query { viewer { login } }", {})

    def test_atomic_write_leaves_canonical_complete_file(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "nested" / "result.json"
            census.write_json(path, {"z": 1, "a": 2})
            self.assertEqual(path.read_bytes(), b'{\n  "a": 2,\n  "z": 1\n}\n')
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(list(path.parent.glob(".*.tmp")), [])


class PlanTest(unittest.TestCase):
    def test_checked_in_plan_matches_cli_contract(self):
        _, plan = census.read_json(census.DEFAULT_PLAN)
        census.validate_plan(plan)

    def test_frozen_plan_rejects_threshold_repository_and_signal_drift(self):
        _, original = census.read_json(census.DEFAULT_PLAN)
        mutations = (
            ("force threshold", lambda plan: plan["decision_thresholds"].__setitem__("force_push_wedge_bps", 0)),
            ("repository panel", lambda plan: plan["repositories"].pop()),
            (
                "actor policy",
                lambda plan: plan["actor_policy"].__setitem__("login_suffix_heuristics", True),
            ),
            (
                "signal mapping",
                lambda plan: plan["decision_thresholds"]["signals"]["force_push_wedge"].__setitem__(
                    "metric_id", "formal_peer_reviewed_pr_rate"
                ),
            ),
            ("unknown root field", lambda plan: plan.__setitem__("access_token", "forbidden")),
        )
        for label, mutate in mutations:
            with self.subTest(label=label):
                plan = json.loads(json.dumps(original))
                mutate(plan)
                with self.assertRaises(census.CensusError):
                    census.validate_plan(plan)


if __name__ == "__main__":
    unittest.main()
