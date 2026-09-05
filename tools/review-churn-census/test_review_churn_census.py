#!/usr/bin/env python3

from datetime import datetime, timezone
import importlib.util
import io
import json
import os
from pathlib import Path
import re
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


class InboxTest(unittest.TestCase):
    reviewer_node_id = "U_alice"
    acquisition = {
        "graphql_calls": 4,
        "minimum_rate_limit_remaining": 4996,
        "last_rate_limit_reset_at": "2026-09-05T01:00:00Z",
    }

    def normalized_review(
        self,
        review_id,
        database_id,
        state,
        submitted_at,
        commit_oid,
    ):
        return {
            "node_id": review_id,
            "database_id": database_id,
            "state": state,
            "submitted_at": submitted_at,
            "commit_oid": commit_oid,
        }

    def normalized_pull_request(
        self,
        number,
        updated_at,
        head_oid,
        reviews,
        *,
        is_draft=False,
    ):
        return {
            "node_id": f"PR_{number}",
            "number": number,
            "url": f"https://ghe.example/acme/widgets/pull/{number}",
            "is_draft": is_draft,
            "updated_at": updated_at,
            "head_oid": head_oid,
            "total_review_count": len(reviews),
            "reviews": reviews,
        }

    def build_report(self, pull_requests):
        eligible = sum(
            census.latest_inbox_checkpoint(pull_request["reviews"]) is not None
            for pull_request in pull_requests
        )
        return census.build_review_inbox(
            "acme/widgets",
            "ghe.example",
            "alice",
            self.reviewer_node_id,
            pull_requests,
            {
                "open_pull_request_pages": 1,
                "review_pages": len(pull_requests) + eligible,
                "open_pull_request_count": len(pull_requests),
                "revalidated_review_prs": eligible,
            },
            self.acquisition,
            {
                "graphql_call_limit": census.MAX_INBOX_GRAPHQL_CALLS,
                "captured_node_limit": census.MAX_INBOX_CAPTURED_NODES,
                "response_byte_limit": census.MAX_INBOX_RESPONSE_BYTES,
                "wall_time_seconds_limit": census.MAX_INBOX_WALL_TIME_SECONDS,
                "resume_review_limit": census.MAX_RESUME_GITHUB_REVIEWS,
                "captured_nodes": len(pull_requests),
                "response_bytes": 1,
            },
            "2026-09-05T00:00:00Z",
            "2026-09-05T00:00:00Z",
        )

    def raw_review(
        self,
        review_id,
        database_id,
        state,
        submitted_at,
        commit_oid,
        *,
        reviewer="alice",
        reviewer_node_id=None,
        typename="User",
    ):
        if reviewer_node_id is None:
            reviewer_node_id = f"U_{reviewer}"
        return {
            "id": review_id,
            "fullDatabaseId": database_id,
            "state": state,
            "submittedAt": submitted_at,
            "author": {
                "__typename": typename,
                "id": reviewer_node_id,
                "login": reviewer,
            },
            "commit": None if commit_oid is None else {"oid": commit_oid},
        }

    def raw_pull_request(
        self,
        number,
        head_oid,
        reviews,
        *,
        total_count=None,
        has_next=False,
        end_cursor=None,
        state="OPEN",
        all_review_count=None,
    ):
        review_count = len(reviews) if total_count is None else total_count
        if all_review_count is None:
            all_review_count = review_count
        return {
            "id": f"PR_{number}",
            "number": number,
            "state": state,
            "url": f"https://ghe.example/acme/widgets/pull/{number}",
            "isDraft": False,
            "updatedAt": f"2026-09-{number:02d}T00:00:00Z",
            "headRefOid": head_oid,
            "allReviews": {"totalCount": all_review_count},
            "reviews": {
                "totalCount": review_count,
                "pageInfo": {
                    "hasNextPage": has_next,
                    "endCursor": end_cursor,
                },
                "nodes": reviews,
            },
        }

    def repository(self, **extra):
        return {
            "id": "R_1",
            "nameWithOwner": "acme/widgets",
            "url": "https://ghe.example/acme/widgets",
            **extra,
        }

    def test_report_contract_latest_checkpoint_and_sorting(self):
        older = self.normalized_review(
            "review-old", 11, "APPROVED", "2026-09-01T00:00:00Z", OID_A
        )
        same_time_lower_id = self.normalized_review(
            "review-low", 12, "APPROVED", "2026-09-02T00:00:00Z", OID_A
        )
        latest = self.normalized_review(
            "review-latest",
            13,
            "CHANGES_REQUESTED",
            "2026-09-02T00:00:00Z",
            OID_B,
        )
        commented = self.normalized_review(
            "review-commented", 14, "COMMENTED", "2026-09-04T00:00:00Z", OID_C
        )
        dismissed = self.normalized_review(
            "review-dismissed", 15, "DISMISSED", "2026-09-05T00:00:00Z", OID_C
        )
        report = self.build_report(
            [
                self.normalized_pull_request(
                    7,
                    "2026-09-03T00:00:00Z",
                    OID_C,
                    [older, same_time_lower_id, latest, commented, dismissed],
                ),
                self.normalized_pull_request(
                    8,
                    "2026-09-04T00:00:00Z",
                    OID_B,
                    [
                        self.normalized_review(
                            "review-8", 20, "APPROVED", "2026-09-03T00:00:00Z", OID_A
                        )
                    ],
                    is_draft=True,
                ),
            ]
        )
        self.assertEqual(
            set(report),
            {
                "schema",
                "tool_version",
                "generated_at",
                "scope",
                "collection",
                "privacy",
                "summary",
                "actionable",
                "unobservable",
            },
        )
        self.assertEqual(report["schema"], "stratadiff-review-inbox-v1")
        self.assertEqual(report["tool_version"], "1.0.0")
        self.assertEqual(
            report["scope"],
            {
                "provider_url": "https://ghe.example",
                "repository": "acme/widgets",
                "reviewer": {
                    "login": "alice",
                    "node_id": self.reviewer_node_id,
                    "source": "authenticated_viewer",
                },
            },
        )
        self.assertEqual(
            report["summary"],
            {
                "status": "actionable",
                "open_prs": 2,
                "eligible_review_prs": 2,
                "comparable_review_prs": 2,
                "up_to_date_prs": 0,
                "resume_available_prs": 2,
                "unobservable_review_prs": 0,
            },
        )
        self.assertEqual(
            [item["number"] for item in report["actionable"]], [8, 7]
        )
        item = report["actionable"][1]
        self.assertEqual(
            item["checkpoint"],
            {
                "oid": OID_B,
                "submitted_at": "2026-09-02T00:00:00Z",
                "state": "CHANGES_REQUESTED",
            },
        )
        self.assertEqual(
            item["resume_argv"],
            [
                "gh",
                "stratadiff",
                "resume",
                "7",
                "-R",
                "ghe.example/acme/widgets",
                "--reviewer",
                "alice",
            ],
        )
        self.assertEqual(
            report["privacy"],
            {
                "source_collected": False,
                "pr_text_collected": False,
                "review_text_collected": False,
                "commit_messages_collected": False,
                "logins_persisted": True,
                "actor_identity": "authenticated_viewer_node_id_and_login",
            },
        )

    def test_latest_eligible_missing_oid_is_unobservable_without_fallback(self):
        report = self.build_report(
            [
                self.normalized_pull_request(
                    7,
                    "2026-09-03T00:00:00Z",
                    OID_C,
                    [
                        self.normalized_review(
                            "older", 1, "APPROVED", "2026-09-01T00:00:00Z", OID_A
                        ),
                        self.normalized_review(
                            "latest",
                            2,
                            "CHANGES_REQUESTED",
                            "2026-09-02T00:00:00Z",
                            None,
                        ),
                    ],
                )
            ]
        )
        self.assertEqual(report["summary"]["status"], "insufficient_evidence")
        self.assertEqual(report["summary"]["comparable_review_prs"], 0)
        self.assertEqual(report["actionable"], [])
        self.assertEqual(report["unobservable"][0]["checkpoint"]["oid"], None)
        self.assertEqual(
            report["unobservable"][0]["reason"], "checkpoint_oid_unavailable"
        )

    def test_fractional_timestamp_orders_the_same_second_exactly(self):
        whole_second = self.normalized_review(
            "whole", 9, "APPROVED", "2026-09-01T00:00:00Z", OID_A
        )
        fractional = self.normalized_review(
            "fractional",
            1,
            "CHANGES_REQUESTED",
            "2026-09-01T00:00:00.000000001Z",
            OID_B,
        )
        checkpoint = census.latest_inbox_checkpoint([whole_second, fractional])
        self.assertIsNotNone(checkpoint)
        self.assertEqual(checkpoint["node_id"], "fractional")

        fractional["submitted_at"] = "2026-09-01T00:00:00.0000000001Z"
        with self.assertRaisesRegex(census.CensusError, "RFC3339 UTC timestamp"):
            census.latest_inbox_checkpoint([fractional])

    def test_resume_limit_never_emits_a_command_that_resume_will_reject(self):
        schema = json.loads(
            (census.ROOT / "schema" / "review-inbox-v1.schema.json").read_text(
                encoding="utf-8"
            )
        )
        self.assertEqual(
            schema["$defs"]["collection"]["properties"]["resource_budget"][
                "properties"
            ]["resume_review_limit"]["const"],
            census.MAX_RESUME_GITHUB_REVIEWS,
        )
        pull_request = self.normalized_pull_request(
            7,
            "2026-09-03T00:00:00Z",
            OID_C,
            [
                self.normalized_review(
                    "review", 1, "APPROVED", "2026-09-01T00:00:00Z", OID_A
                )
            ],
        )
        pull_request["total_review_count"] = census.MAX_RESUME_GITHUB_REVIEWS + 1
        report = self.build_report([pull_request])
        self.assertEqual(report["actionable"], [])
        self.assertEqual(report["summary"]["status"], "insufficient_evidence")
        self.assertEqual(report["summary"]["comparable_review_prs"], 1)
        self.assertEqual(
            report["unobservable"][0]["reason"],
            "resume_review_limit_exceeded",
        )

    def test_resume_limit_does_not_hide_a_provably_up_to_date_checkpoint(self):
        pull_request = self.normalized_pull_request(
            7,
            "2026-09-03T00:00:00Z",
            OID_A,
            [
                self.normalized_review(
                    "review", 1, "APPROVED", "2026-09-01T00:00:00Z", OID_A
                )
            ],
        )
        pull_request["total_review_count"] = census.MAX_RESUME_GITHUB_REVIEWS + 1
        report = self.build_report([pull_request])
        self.assertEqual(report["summary"]["status"], "up_to_date")
        self.assertEqual(report["summary"]["comparable_review_prs"], 1)
        self.assertEqual(report["summary"]["up_to_date_prs"], 1)
        self.assertEqual(report["actionable"], [])
        self.assertEqual(report["unobservable"], [])

    def test_status_precedence_and_all_statuses(self):
        no_eligible = self.build_report(
            [
                self.normalized_pull_request(
                    1,
                    "2026-09-01T00:00:00Z",
                    OID_A,
                    [
                        self.normalized_review(
                            "comment", 1, "COMMENTED", "2026-08-01T00:00:00Z", OID_A
                        )
                    ],
                )
            ]
        )
        up_to_date = self.build_report(
            [
                self.normalized_pull_request(
                    2,
                    "2026-09-02T00:00:00Z",
                    OID_A,
                    [
                        self.normalized_review(
                            "approved", 2, "APPROVED", "2026-08-02T00:00:00Z", OID_A
                        )
                    ],
                )
            ]
        )
        mixed = self.build_report(
            [
                self.normalized_pull_request(
                    3,
                    "2026-09-03T00:00:00Z",
                    OID_C,
                    [
                        self.normalized_review(
                            "drifted", 3, "APPROVED", "2026-08-03T00:00:00Z", OID_A
                        )
                    ],
                ),
                self.normalized_pull_request(
                    4,
                    "2026-09-04T00:00:00Z",
                    OID_C,
                    [
                        self.normalized_review(
                            "unknown", 4, "APPROVED", "2026-08-04T00:00:00Z", None
                        )
                    ],
                ),
            ]
        )
        self.assertEqual(no_eligible["summary"]["status"], "no_eligible_reviews")
        self.assertEqual(up_to_date["summary"]["status"], "up_to_date")
        self.assertEqual(mixed["summary"]["status"], "actionable")
        self.assertEqual(mixed["summary"]["unobservable_review_prs"], 1)

    def test_collection_pages_all_open_prs_and_each_viewer_review_connection(self):
        first_review = self.raw_review(
            "RVR_1", "1", "APPROVED", "2026-08-01T00:00:00Z", OID_A
        )
        later_comment = self.raw_review(
            "RVR_2", "2", "COMMENTED", "2026-08-02T00:00:00Z", OID_B
        )
        first_pr = self.raw_pull_request(
            1,
            OID_B,
            [first_review],
            total_count=2,
            has_next=True,
            end_cursor="reviews-1",
        )
        second_pr = self.raw_pull_request(2, OID_C, [])
        revalidated_first_pr = self.raw_pull_request(
            1, OID_B, [first_review, later_comment]
        )
        viewer = {"id": self.reviewer_node_id, "login": "alice"}
        responses = [
            {
                "viewer": viewer,
                "repository": self.repository(pullRequests={"totalCount": 2}),
            },
            {
                "viewer": viewer,
                "repository": self.repository(
                    pullRequests={
                        "totalCount": 2,
                        "pageInfo": {"hasNextPage": True, "endCursor": "prs-1"},
                        "nodes": [first_pr],
                    }
                )
            },
            {
                "viewer": viewer,
                "repository": self.repository(
                    pullRequest={
                        "id": "PR_1",
                        "number": 1,
                        "state": "OPEN",
                        "headRefOid": OID_B,
                        "allReviews": {"totalCount": 2},
                        "reviews": {
                            "totalCount": 2,
                            "pageInfo": {"hasNextPage": False, "endCursor": None},
                            "nodes": [later_comment],
                        },
                    }
                )
            },
            {
                "viewer": viewer,
                "repository": self.repository(
                    pullRequests={
                        "totalCount": 2,
                        "pageInfo": {"hasNextPage": False, "endCursor": None},
                        "nodes": [second_pr],
                    }
                )
            },
            {
                "viewer": viewer,
                "repository": self.repository(pullRequest=revalidated_first_pr),
            },
            {
                "viewer": viewer,
                "repository": self.repository(pullRequests={"totalCount": 2}),
            },
        ]

        class FakeApi:
            def __init__(self, values):
                self.values = values
                self.calls = []
                self.last_response_bytes = 0

            def call(self, query, variables):
                self.calls.append((query, variables))
                value = self.values.pop(0)
                self.last_response_bytes = len(census.canonical_json(value))
                return value

            def acquisition(self):
                return {
                    "graphql_calls": len(self.calls),
                    "minimum_rate_limit_remaining": 4996,
                    "last_rate_limit_reset_at": "2026-09-05T01:00:00Z",
                }

        api = FakeApi(responses)
        with mock.patch.object(
            census, "now_timestamp", return_value="2026-09-05T00:00:00Z"
        ):
            report = census.collect_review_inbox(
                api, "acme/widgets", "ghe.example"
            )
        self.assertEqual(api.values, [])
        self.assertEqual(report["summary"]["open_prs"], 2)
        self.assertEqual(report["summary"]["resume_available_prs"], 1)
        self.assertEqual(report["collection"]["open_pull_request_pages"], 2)
        self.assertEqual(report["collection"]["review_pages"], 4)
        self.assertEqual(report["collection"]["revalidated_review_prs"], 1)
        self.assertEqual(api.calls[1][1]["viewer"], "alice")
        self.assertIsNone(api.calls[1][1]["cursor"])
        self.assertEqual(api.calls[2][1]["cursor"], "reviews-1")
        self.assertEqual(api.calls[3][1]["cursor"], "prs-1")
        self.assertIn("states: [OPEN]", census.INBOX_PULL_REQUESTS_QUERY)
        self.assertIn(
            "orderBy: {field: CREATED_AT, direction: DESC}",
            census.INBOX_PULL_REQUESTS_QUERY,
        )
        for query in (
            census.INBOX_PULL_REQUESTS_QUERY,
            census.INBOX_REVIEWS_PAGE_QUERY,
            census.INBOX_REVALIDATE_QUERY,
        ):
            self.assertIn("author: $viewer", query)
            self.assertIn("author { __typename login ... on User { id } }", query)
            self.assertIn("allReviews: reviews { totalCount }", query)
            self.assertIsNone(
                re.search(r"\b(?:title|body|comments|diff|source)\b", query)
            )

    def test_incomplete_pull_request_or_review_pagination_fails_closed(self):
        class FakeApi:
            def __init__(self, response):
                self.response = response
                self.calls = 0
                self.last_response_bytes = 0

            def call(self, _query, _variables):
                self.calls += 1
                self.last_response_bytes = len(census.canonical_json(self.response))
                return self.response

            def acquisition(self):
                return {
                    "graphql_calls": self.calls,
                    "minimum_rate_limit_remaining": 4999,
                    "last_rate_limit_reset_at": "2026-09-05T01:00:00Z",
                }

        incomplete_prs = self.repository(
            pullRequests={
                "totalCount": 2,
                "pageInfo": {"hasNextPage": False, "endCursor": None},
                "nodes": [self.raw_pull_request(1, OID_B, [])],
            }
        )
        with self.assertRaisesRegex(census.CensusError, "pagination incomplete"):
            census.collect_inbox_pull_requests(
                census.InboxResourceBudget(
                    FakeApi(
                        {
                            "viewer": {"id": self.reviewer_node_id, "login": "alice"},
                            "repository": incomplete_prs,
                        }
                    )
                ),
                "acme",
                "widgets",
                "ghe.example",
                "R_1",
                "alice",
                self.reviewer_node_id,
            )

        incomplete_reviews = self.raw_pull_request(
            1,
            OID_B,
            [
                self.raw_review(
                    "RVR_1", "1", "APPROVED", "2026-08-01T00:00:00Z", OID_A
                )
            ],
            total_count=2,
        )
        normalized, connection = census.normalize_inbox_pull_request(
            incomplete_reviews, "acme/widgets", "ghe.example"
        )
        with self.assertRaisesRegex(census.CensusError, "pagination incomplete"):
            census.collect_inbox_reviews(
                census.InboxResourceBudget(FakeApi({})),
                "acme",
                "widgets",
                "ghe.example",
                "R_1",
                "alice",
                self.reviewer_node_id,
                normalized,
                connection,
            )

    def test_candidate_head_change_during_revalidation_fails_closed(self):
        review = self.raw_review(
            "RVR_1", "1", "APPROVED", "2026-08-01T00:00:00Z", OID_A
        )
        initial = self.raw_pull_request(1, OID_B, [review])
        changed = self.raw_pull_request(1, OID_C, [review])
        viewer = {"id": self.reviewer_node_id, "login": "alice"}
        responses = [
            {
                "viewer": viewer,
                "repository": self.repository(pullRequests={"totalCount": 1}),
            },
            {
                "viewer": viewer,
                "repository": self.repository(
                    pullRequests={
                        "totalCount": 1,
                        "pageInfo": {"hasNextPage": False, "endCursor": None},
                        "nodes": [initial],
                    }
                ),
            },
            {
                "viewer": viewer,
                "repository": self.repository(pullRequest=changed),
            },
        ]

        class FakeApi:
            def __init__(self, values):
                self.values = values
                self.calls = 0
                self.last_response_bytes = 0

            def call(self, _query, _variables):
                self.calls += 1
                value = self.values.pop(0)
                self.last_response_bytes = len(census.canonical_json(value))
                return value

            def acquisition(self):
                return {
                    "graphql_calls": self.calls,
                    "minimum_rate_limit_remaining": 4997,
                    "last_rate_limit_reset_at": "2026-09-05T01:00:00Z",
                }

        with self.assertRaisesRegex(census.CensusError, "changed while.*revalidated"):
            census.collect_review_inbox(
                FakeApi(responses), "acme/widgets", "ghe.example"
            )

    def test_viewer_node_id_change_at_final_context_fails_closed(self):
        responses = [
            {
                "viewer": {"id": self.reviewer_node_id, "login": "alice"},
                "repository": self.repository(pullRequests={"totalCount": 0}),
            },
            {
                "viewer": {"id": self.reviewer_node_id, "login": "alice"},
                "repository": self.repository(
                    pullRequests={
                        "totalCount": 0,
                        "pageInfo": {"hasNextPage": False, "endCursor": None},
                        "nodes": [],
                    }
                ),
            },
            {
                "viewer": {"id": "U_different", "login": "alice"},
                "repository": self.repository(pullRequests={"totalCount": 0}),
            },
        ]

        class FakeApi:
            def __init__(self, values):
                self.values = values
                self.calls = 0
                self.last_response_bytes = 0

            def call(self, _query, _variables):
                self.calls += 1
                value = self.values.pop(0)
                self.last_response_bytes = len(census.canonical_json(value))
                return value

            def acquisition(self):
                return {
                    "graphql_calls": self.calls,
                    "minimum_rate_limit_remaining": 4997,
                    "last_rate_limit_reset_at": "2026-09-05T01:00:00Z",
                }

        with self.assertRaisesRegex(census.CensusError, "viewer identity changed"):
            census.collect_review_inbox(
                FakeApi(responses), "acme/widgets", "ghe.example"
            )

    def test_global_inbox_resource_budgets_fail_closed(self):
        class FakeApi:
            def __init__(self, response_bytes=19):
                self.calls = 0
                self.response_bytes = response_bytes
                self.last_response_bytes = 0

            def call(self, _query, _variables):
                self.calls += 1
                self.last_response_bytes = self.response_bytes
                return {"value": "bounded"}

            def acquisition(self):
                return {
                    "graphql_calls": self.calls,
                    "minimum_rate_limit_remaining": 4999,
                    "last_rate_limit_reset_at": "2026-09-05T01:00:00Z",
                }

        with mock.patch.object(census, "MAX_INBOX_GRAPHQL_CALLS", 1):
            budget = census.InboxResourceBudget(FakeApi())
            budget.call("query", {})
            with self.assertRaisesRegex(census.CensusError, "1-call"):
                budget.call("query", {})

        with mock.patch.object(census, "MAX_INBOX_CAPTURED_NODES", 1):
            budget = census.InboxResourceBudget(FakeApi())
            with self.assertRaisesRegex(census.CensusError, "1-node"):
                budget.consume_nodes(2)

        with mock.patch.object(census, "MAX_INBOX_RESPONSE_BYTES", 1):
            budget = census.InboxResourceBudget(FakeApi())
            with self.assertRaisesRegex(census.CensusError, "1-byte"):
                budget.call("query", {})

        with mock.patch.object(census, "MAX_INBOX_RESPONSE_BYTES", 10):
            budget = census.InboxResourceBudget(FakeApi(response_bytes=6))
            budget.call("query", {})
            with self.assertRaisesRegex(census.CensusError, "10-byte"):
                budget.call("query", {})

        with mock.patch.object(
            census.time, "monotonic", side_effect=[0.0, 601.0]
        ):
            budget = census.InboxResourceBudget(FakeApi())
            with self.assertRaisesRegex(census.CensusError, "600-second"):
                budget.call("query", {})

    def test_schema_and_checkpoint_ordering_fail_closed(self):
        self.assertEqual(
            census.validate_inbox_login("alice_example", "reviewer"),
            "alice_example",
        )
        with self.assertRaisesRegex(census.CensusError, "accepted by Review Resume"):
            census.validate_inbox_oid("a" * 64, "checkpoint")
        missing_field = self.raw_pull_request(1, OID_A, [])
        del missing_field["headRefOid"]
        with self.assertRaisesRegex(census.CensusError, "missing=.*headRefOid"):
            census.normalize_inbox_pull_request(
                missing_field, "acme/widgets", "ghe.example"
            )
        closed = self.raw_pull_request(1, OID_A, [], state="CLOSED")
        with self.assertRaisesRegex(census.CensusError, "non-open"):
            census.normalize_inbox_pull_request(
                closed, "acme/widgets", "ghe.example"
            )
        missing_database_id = self.raw_review(
            "RVR_1", None, "APPROVED", "2026-08-01T00:00:00Z", OID_A
        )
        with self.assertRaisesRegex(census.CensusError, "must be present"):
            census.normalize_inbox_review(
                missing_database_id, "review", "alice", self.reviewer_node_id
            )

        wrong_reviewer = self.raw_review(
            "RVR_2",
            "2",
            "APPROVED",
            "2026-08-01T00:00:00Z",
            OID_A,
            reviewer="mallory",
        )
        with self.assertRaisesRegex(census.CensusError, "authenticated viewer"):
            census.normalize_inbox_review(
                wrong_reviewer, "review", "alice", self.reviewer_node_id
            )

        wrong_identity = self.raw_review(
            "RVR_2",
            "2",
            "APPROVED",
            "2026-08-01T00:00:00Z",
            OID_A,
            reviewer_node_id="U_different",
        )
        with self.assertRaisesRegex(census.CensusError, "author identity"):
            census.normalize_inbox_review(
                wrong_identity, "review", "alice", self.reviewer_node_id
            )

        bot_reviewer = self.raw_review(
            "RVR_3",
            "3",
            "APPROVED",
            "2026-08-01T00:00:00Z",
            OID_A,
            typename="Bot",
        )
        with self.assertRaisesRegex(census.CensusError, "GitHub User"):
            census.normalize_inbox_review(
                bot_reviewer, "review", "alice", self.reviewer_node_id
            )

    def test_markdown_contains_copyable_resume_and_unknown_warning(self):
        report = self.build_report(
            [
                self.normalized_pull_request(
                    7,
                    "2026-09-03T00:00:00Z",
                    OID_C,
                    [
                        self.normalized_review(
                            "drift", 1, "APPROVED", "2026-08-01T00:00:00Z", OID_A
                        )
                    ],
                ),
                self.normalized_pull_request(
                    8,
                    "2026-09-04T00:00:00Z",
                    OID_C,
                    [
                        self.normalized_review(
                            "unknown", 2, "APPROVED", "2026-08-02T00:00:00Z", None
                        )
                    ],
                ),
            ]
        )
        rendered = census.render_review_inbox_markdown(report)
        self.assertIn("# StrataDiff Review Inbox", rendered)
        self.assertIn(
            "gh stratadiff resume 7 -R ghe.example/acme/widgets --reviewer alice",
            rendered,
        )
        self.assertIn("unknown, not evidence", rendered)
        self.assertIn("Run Resume commands from any directory", rendered)
        self.assertIn("isolated temporary repository", rendered)
        self.assertNotIn("review-old", rendered)

    def test_markdown_does_not_call_insufficient_evidence_up_to_date(self):
        report = self.build_report(
            [
                self.normalized_pull_request(
                    8,
                    "2026-09-04T00:00:00Z",
                    OID_C,
                    [
                        self.normalized_review(
                            "unknown", 2, "APPROVED", "2026-08-02T00:00:00Z", None
                        )
                    ],
                )
            ]
        )
        rendered = census.render_review_inbox_markdown(report)
        self.assertIn(
            "The available evidence cannot establish that every review is current",
            rendered,
        )
        self.assertNotIn("No exact review resume is currently required", rendered)

    def test_empty_markdown_inbox_offers_the_offline_demo(self):
        report = self.build_report([])
        rendered = census.render_review_inbox_markdown(report)
        self.assertIn("No eligible review currently needs Resume.", rendered)
        self.assertIn("Try: gh stratadiff demo", rendered)

    def test_public_metadata_seed_matches_product_classification(self):
        oracle_path = (
            Path(__file__).resolve().parents[2]
            / "benchmarks"
            / "review-inbox-v1"
            / "oracle-v1.json"
        )
        oracle = json.loads(oracle_path.read_text(encoding="utf-8"))
        for case in oracle["cases"]:
            with self.subTest(case=case["id"]):
                checkpoint = case["selected_checkpoint"]
                reviews = [
                    self.normalized_review(
                        f"review-{case['number']}-checkpoint",
                        int(checkpoint["database_id"]),
                        checkpoint["state"],
                        checkpoint["submitted_at"],
                        checkpoint["oid"],
                    )
                ]
                for index in range(case["later_non_completed_review_count"]):
                    reviews.append(
                        self.normalized_review(
                            f"review-{case['number']}-comment-{index}",
                            int(checkpoint["database_id"]) + index + 1,
                            "COMMENTED",
                            f"2026-09-05T00:00:{index:02d}Z",
                            case["head_oid"],
                        )
                    )
                pull_request = {
                    "node_id": f"PR_{case['number']}",
                    "number": case["number"],
                    "url": case["url"],
                    "is_draft": case["is_draft"],
                    "updated_at": case["updated_at"],
                    "head_oid": case["head_oid"],
                    "total_review_count": len(reviews),
                    "reviews": reviews,
                }
                report = census.build_review_inbox(
                    case["repository"],
                    "github.com",
                    case["reviewer"],
                    "U_oracle_reviewer",
                    [pull_request],
                    {
                        "open_pull_request_pages": 1,
                        "review_pages": 2,
                        "open_pull_request_count": 1,
                        "revalidated_review_prs": 1,
                    },
                    self.acquisition,
                    {
                        "graphql_call_limit": census.MAX_INBOX_GRAPHQL_CALLS,
                        "captured_node_limit": census.MAX_INBOX_CAPTURED_NODES,
                        "response_byte_limit": census.MAX_INBOX_RESPONSE_BYTES,
                        "wall_time_seconds_limit": census.MAX_INBOX_WALL_TIME_SECONDS,
                        "resume_review_limit": census.MAX_RESUME_GITHUB_REVIEWS,
                        "captured_nodes": 1,
                        "response_bytes": 1,
                    },
                    oracle["captured_at"],
                    oracle["captured_at"],
                )
                self.assertEqual(report["summary"]["status"], case["expected"])
                if case["expected"] == "actionable":
                    self.assertEqual(
                        report["actionable"][0]["checkpoint"]["oid"],
                        checkpoint["oid"],
                    )
                else:
                    self.assertEqual(report["actionable"], [])

    def test_inbox_argument_defaults_validation_and_private_output(self):
        arguments = census.parse_arguments(["inbox", "-R", "acme/widgets"])
        self.assertEqual(arguments.repository, "acme/widgets")
        self.assertEqual(arguments.hostname, "github.com")
        self.assertEqual(arguments.format, "markdown")
        for extra in (
            ["-R", "three/part/name"],
            ["-R", "acme/widgets", "--hostname", "https://github.com"],
            ["-R", "acme/widgets", "--format", "yaml"],
        ):
            with self.subTest(extra=extra), mock.patch(
                "sys.stderr", new=io.StringIO()
            ):
                with self.assertRaises(SystemExit) as raised:
                    census.parse_arguments(["inbox", *extra])
                self.assertEqual(raised.exception.code, 2)

        report = self.build_report([])
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "inbox.json"
            command_arguments = census.argparse.Namespace(
                repository="acme/widgets",
                hostname="ghe.example",
                format="json",
                output=output,
                gh="custom-gh",
            )
            with mock.patch.object(
                census, "collect_review_inbox", return_value=report
            ) as collect:
                census.command_inbox(command_arguments)
            self.assertEqual(output.read_bytes(), census.canonical_json(report))
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            self.assertEqual(collect.call_args.args[0].executable, "custom-gh")
            self.assertEqual(collect.call_args.args[0].hostname, "ghe.example")


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
    @staticmethod
    def completed_run(stdout_bytes, stderr_bytes=b"", returncode=0):
        def run(arguments, **keywords):
            keywords["stdout"].write(stdout_bytes)
            keywords["stderr"].write(stderr_bytes)
            return subprocess.CompletedProcess(arguments, returncode)

        return run

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
        with mock.patch.dict(
            os.environ, {"GH_DEBUG": "api", "GH_TRACE": "1"}, clear=False
        ), mock.patch.object(
            census.subprocess,
            "run",
            side_effect=self.completed_run(json.dumps(response).encode()),
        ) as run:
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
        self.assertIsNot(keywords["stdout"], subprocess.PIPE)
        self.assertIsNot(keywords["stderr"], subprocess.PIPE)

    def test_graphql_errors_are_not_hidden(self):
        response = {
            "data": {"rateLimit": {"cost": 1, "remaining": 99, "resetAt": "2026-09-05T00:00:00Z"}},
            "errors": [{"message": "boom"}],
        }
        with mock.patch.object(
            census.subprocess,
            "run",
            side_effect=self.completed_run(json.dumps(response).encode()),
        ):
            with self.assertRaisesRegex(census.CensusError, "boom"):
                census.GithubGraphQL().call("query { viewer { login } }", {})

    def test_inbox_budget_counts_the_raw_graphql_envelope(self):
        response = {
            "data": {
                "rateLimit": {
                    "cost": 1,
                    "remaining": 99,
                    "resetAt": "2026-09-05T00:00:00Z",
                }
            },
            "ignoredEnvelopePadding": "x" * 1024,
        }
        response_bytes = json.dumps(response).encode()
        api = census.GithubGraphQL()
        with mock.patch.object(
            census.subprocess,
            "run",
            side_effect=self.completed_run(response_bytes),
        ), mock.patch.object(
            census, "MAX_INBOX_RESPONSE_BYTES", len(response_bytes) - 1
        ):
            budget = census.InboxResourceBudget(api)
            with self.assertRaisesRegex(census.CensusError, "response budget"):
                budget.call("query { rateLimit { remaining } }", {})
        self.assertEqual(api.last_response_bytes, len(response_bytes))

    def test_graphql_transport_and_json_failures_are_clear(self):
        with mock.patch.object(
            census.subprocess,
            "run",
            side_effect=subprocess.TimeoutExpired(["gh"], 120),
        ):
            with self.assertRaisesRegex(census.CensusError, "timed out after 120"):
                census.GithubGraphQL().call("query { viewer { login } }", {})

        with mock.patch.object(
            census.subprocess,
            "run",
            side_effect=self.completed_run(b"not-json"),
        ):
            with self.assertRaisesRegex(census.CensusError, "invalid JSON"):
                census.GithubGraphQL().call("query { viewer { login } }", {})

        with mock.patch.object(
            census, "MAX_GRAPHQL_RESPONSE_BYTES", 4
        ), mock.patch.object(
            census.subprocess,
            "run",
            side_effect=self.completed_run(b"12345"),
        ):
            with self.assertRaisesRegex(census.CensusError, "exceeds 4 bytes"):
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
