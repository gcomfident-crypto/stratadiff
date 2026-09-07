#!/usr/bin/env python3

from __future__ import annotations

import json
import sys
import tempfile
import unittest
import urllib.error
from datetime import datetime, timedelta, timezone
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parent))

from govern import (  # noqa: E402
    CODERABBIT_APP_ID,
    CODERABBIT_BOT_ID,
    CODERABBIT_HANDLE,
    CODERABBIT_LOGIN,
    CODERABBIT_REVIEW_MARKER,
    GitHubApi,
    GovernorError,
    Settings,
    collect_evidence,
    current_gate_lease,
    execute,
    gate_state,
    load_settings,
    run,
    write_evidence,
)


HEAD = "a" * 40
NEXT_HEAD = "b" * 40
BASE = "c" * 40
NEXT_BASE = "d" * 40
DISPATCH_AT = "2026-09-07T12:00:00Z"
ACK_AT = "2026-09-07T12:00:05Z"
COMPLETE_AT = "2026-09-07T12:02:00Z"
COMMAND_HANDLE = "12345678-1234-4abc-8def-123456789abc"


def settings(
    mode: str = "govern",
    debounce_seconds: int = 0,
    expected_base: str = BASE,
    timeout_seconds: int = 600,
) -> Settings:
    return Settings(
        mode=mode,
        repository="acme/widget",
        pull_request_number=7,
        expected_base=expected_base,
        expected_head=HEAD,
        api_url="https://api.github.test",
        token="secret-token",
        debounce_seconds=debounce_seconds,
        poll_seconds=5,
        timeout_seconds=timeout_seconds,
        command="full-review",
        status_context="CodeRabbit",
        reviewer_login=CODERABBIT_LOGIN,
        reviewer_handle=CODERABBIT_HANDLE,
        gate_context="StrataDiff Final Head",
        run_url="https://github.test/acme/widget/actions/runs/99",
    )


def pull(
    head: str = HEAD,
    base: str = BASE,
    state: str = "open",
    merged: bool = False,
    draft: bool = False,
) -> dict:
    return {
        "base": {"sha": base},
        "head": {"sha": head},
        "state": state,
        "merged": merged,
        "draft": draft,
    }


def coderabbit_user() -> dict:
    return {
        "id": CODERABBIT_BOT_ID,
        "login": CODERABBIT_LOGIN,
        "type": "Bot",
        "html_url": "https://github.com/apps/coderabbitai",
    }


def actions_user() -> dict:
    return {
        "id": 41898282,
        "login": "github-actions[bot]",
        "type": "Bot",
        "html_url": "https://github.com/apps/github-actions",
    }


def status(
    state: str = "success",
    description: str = "Review completed",
    identifier: int = 11,
    created_at: str = COMPLETE_AT,
    context: str = "CodeRabbit",
) -> dict:
    return {
        "id": identifier,
        "context": context,
        "state": state,
        "description": description,
        "avatar_url": f"https://avatars.githubusercontent.com/in/{CODERABBIT_APP_ID}?v=4",
        "creator": coderabbit_user(),
        "created_at": created_at,
        "updated_at": created_at,
        "target_url": "https://github.test/provider/run/1",
    }


def review(
    head: str = HEAD,
    state: str = "COMMENTED",
    identifier: int = 17,
    submitted_at: str = COMPLETE_AT,
    marker: bool = True,
) -> dict:
    body = "Review findings"
    if marker:
        body += f"\n\n{CODERABBIT_REVIEW_MARKER}"
    return {
        "id": identifier,
        "user": coderabbit_user(),
        "state": state,
        "commit_id": head,
        "submitted_at": submitted_at,
        "body": body,
    }


def acknowledgement(
    identifier: int = 23,
    created_at: str = ACK_AT,
    finished: bool = True,
) -> dict:
    body = f"<!-- CodeRabbit review command invocation: {COMMAND_HANDLE} -->"
    if finished:
        body += "\nFull review finished."
    return {
        "id": identifier,
        "user": coderabbit_user(),
        "created_at": created_at,
        "updated_at": COMPLETE_AT,
        "body": body,
    }


def command_comment(identifier: int = 29) -> dict:
    return {
        "id": identifier,
        "user": actions_user(),
        "created_at": DISPATCH_AT,
        "updated_at": DISPATCH_AT,
        "body": "@coderabbitai full review",
        "issue_url": "https://api.github.test/repos/acme/widget/issues/7",
    }


def gate_status(
    identifier: int = 31,
    description: str | None = None,
    created_at: str = "2026-09-07T12:00:01Z",
) -> dict:
    return {
        "id": identifier,
        "context": "StrataDiff Final Head",
        "state": "pending",
        "description": description or f"Reviewing base {BASE}; command 29",
        "creator": actions_user(),
        "created_at": created_at,
        "updated_at": created_at,
        "target_url": "https://github.test/acme/widget/actions/runs/98",
    }


class FakeApi:
    def __init__(
        self,
        pulls: list[dict],
        statuses: list[list[dict]] | None = None,
        reviews: list[list[dict]] | None = None,
        comments: list[list[dict]] | None = None,
        stored_comments: dict[int, dict] | None = None,
    ):
        self.pulls = pulls
        self.statuses = statuses or [[]]
        self.reviews = reviews or [[]]
        self.comments = comments or [[]]
        self.stored_comments = stored_comments or {}
        self.pull_calls = 0
        self.status_calls = 0
        self.review_calls = 0
        self.comment_calls = 0
        self.created_comments: list[str] = []
        self.gate_statuses: list[dict] = []

    @staticmethod
    def _next(values: list, index: int):
        return values[min(index, len(values) - 1)]

    def get_pull(self) -> dict:
        value = self._next(self.pulls, self.pull_calls)
        self.pull_calls += 1
        return value

    def list_statuses(self, head: str) -> list[dict]:
        self.asserted_head = head
        value = self._next(self.statuses, self.status_calls)
        self.status_calls += 1
        return value

    def list_reviews(self) -> list[dict]:
        value = self._next(self.reviews, self.review_calls)
        self.review_calls += 1
        return value

    def list_comments(self) -> list[dict]:
        value = self._next(self.comments, self.comment_calls)
        self.comment_calls += 1
        return value

    def get_comment(self, comment_id: int) -> dict:
        return self.stored_comments[comment_id]

    def create_comment(self, body: str) -> dict:
        self.created_comments.append(body)
        value = command_comment()
        value["body"] = body
        self.stored_comments[value["id"]] = value
        return value

    def create_status(
        self, head: str, state: str, context: str, description: str, target_url: str
    ) -> dict:
        created = {
            "id": 100 + len(self.gate_statuses),
            "head": head,
            "state": state,
            "context": context,
            "description": description,
            "target_url": target_url,
        }
        self.gate_statuses.append(created)
        return created


class FakeClock:
    def __init__(self, instant: datetime | None = None):
        self.value = 0.0
        self.instant = instant or datetime(2026, 9, 7, 12, 5, tzinfo=timezone.utc)
        self.sleeps: list[float] = []

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.value += seconds
        self.instant += timedelta(seconds=seconds)

    def monotonic(self) -> float:
        return self.value

    def now(self) -> datetime:
        return self.instant


class FakeResponse:
    def __init__(self, value: object, link: str = ""):
        self.status = 200
        self.payload = json.dumps(value).encode()
        self.headers = {"Link": link} if link else {}

    def read(self, amount: int) -> bytes:
        return self.payload[:amount]

    def __enter__(self):
        return self

    def __exit__(self, exception_type, exception, traceback) -> None:
        return None


class FakeOpener:
    def __init__(self, responses: list[FakeResponse | Exception]):
        self.responses = responses
        self.requests = []

    def open(self, request, timeout: int):
        self.requests.append((request, timeout))
        response = self.responses.pop(0)
        if isinstance(response, Exception):
            raise response
        return response


def successful_api(pulls: list[dict] | None = None) -> FakeApi:
    return FakeApi(
        pulls or [pull()],
        statuses=[[], [status()]],
        reviews=[[review()]],
        comments=[[acknowledgement()]],
    )


class GovernorTests(unittest.TestCase):
    def test_api_paginates_direct_status_endpoint_and_keeps_token_out_of_url(self) -> None:
        api = GitHubApi("https://api.github.test", "acme/widget", 7, "secret-token")
        api.opener = FakeOpener(
            [
                FakeResponse([status(identifier=1)], '<next>; rel="next"'),
                FakeResponse([status(identifier=2)]),
            ]
        )

        statuses = api.list_statuses(HEAD)

        self.assertEqual([item["id"] for item in statuses], [1, 2])
        for request, timeout in api.opener.requests:
            self.assertIn(f"/commits/{HEAD}/statuses", request.full_url)
            self.assertNotIn("secret-token", request.full_url)
            self.assertEqual(request.get_header("Authorization"), "Bearer secret-token")
            self.assertEqual(timeout, 30)

    def test_api_error_does_not_expose_token_or_redirect_it(self) -> None:
        api = GitHubApi("https://api.github.test", "acme/widget", 7, "secret-token")
        api.opener = FakeOpener(
            [urllib.error.HTTPError("https://evil.test/secret-token", 302, "moved", {}, None)]
        )

        with self.assertRaises(GovernorError) as raised:
            api.get_pull()

        self.assertNotIn("secret-token", str(raised.exception))
        self.assertEqual(len(api.opener.requests), 1)

    def test_observe_reports_head_evidence_but_never_a_base_bound_gate(self) -> None:
        api = FakeApi([pull()], statuses=[[status()]], reviews=[[review()]])

        outcome = run(settings(mode="observe"), api)

        self.assertEqual(outcome.outcome, "observed-provider-evidence")
        self.assertTrue(outcome.evidence.complete)
        self.assertFalse(outcome.evidence.gate_satisfied)
        self.assertEqual(api.created_comments, [])

    def test_success_status_without_exact_head_review_is_not_coverage(self) -> None:
        outcome = run(
            settings(mode="observe"),
            FakeApi([pull()], statuses=[[status()]], reviews=[[]]),
        )

        self.assertEqual(outcome.outcome, "observed-uncovered")
        self.assertFalse(outcome.evidence.complete)

    def test_review_for_an_older_head_is_not_coverage(self) -> None:
        outcome = run(
            settings(mode="observe"),
            FakeApi([pull()], statuses=[[status()]], reviews=[[review(NEXT_HEAD)]]),
        )

        self.assertEqual(outcome.outcome, "observed-uncovered")
        self.assertIsNone(outcome.evidence.review)

    def test_govern_dispatch_requires_ack_status_and_exact_head_review(self) -> None:
        api = successful_api()
        clock = FakeClock()

        outcome = run(settings(), api, clock.sleep, clock.monotonic, clock.now)

        self.assertEqual(outcome.outcome, "dispatched-covered")
        self.assertTrue(outcome.evidence.gate_satisfied)
        self.assertEqual(api.created_comments, ["@coderabbitai full review"])
        self.assertEqual(
            [item["state"] for item in api.gate_statuses], ["pending", "pending"]
        )
        self.assertEqual(outcome.dispatch["lease"], "created")

    def test_missing_command_acknowledgement_times_out_fail_closed(self) -> None:
        api = FakeApi(
            [pull()],
            statuses=[[], [status()]],
            reviews=[[review()]],
            comments=[[]],
        )
        clock = FakeClock()

        outcome = run(
            settings(timeout_seconds=10), api, clock.sleep, clock.monotonic, clock.now
        )

        self.assertEqual(outcome.outcome, "timed-out-uncovered")
        self.assertFalse(outcome.evidence.gate_satisfied)
        self.assertTrue(outcome.fail)

    def test_evidence_at_exact_dispatch_second_is_rejected(self) -> None:
        exact_status = status(created_at=DISPATCH_AT)
        exact_review = review(submitted_at=DISPATCH_AT)
        exact_ack = acknowledgement(created_at=DISPATCH_AT)

        evidence = collect_evidence(
            FakeApi(
                [pull()],
                statuses=[[exact_status]],
                reviews=[[exact_review]],
                comments=[[exact_ack]],
            ),
            settings(),
            DISPATCH_AT,
        )

        self.assertIsNone(evidence.status)
        self.assertIsNone(evidence.review)
        self.assertIsNone(evidence.acknowledgement)
        self.assertFalse(evidence.gate_satisfied)

    def test_newer_changes_requested_never_falls_back_to_old_green_pair(self) -> None:
        old_status = status(identifier=1, created_at="2026-09-07T12:01:00Z")
        old_review = review(identifier=2, submitted_at="2026-09-07T12:01:00Z")
        newer_review = review(
            state="CHANGES_REQUESTED",
            identifier=3,
            submitted_at="2026-09-07T12:10:00Z",
        )
        evidence = collect_evidence(
            FakeApi(
                [pull()],
                statuses=[[old_status]],
                reviews=[[old_review, newer_review]],
                comments=[[acknowledgement()]],
            ),
            settings(),
            DISPATCH_AT,
        )

        self.assertEqual(evidence.review["id"], 3)
        self.assertTrue(evidence.blocking)
        self.assertFalse(evidence.temporally_correlated)
        self.assertFalse(evidence.gate_satisfied)

    def test_newer_failed_status_never_falls_back_to_old_success(self) -> None:
        old = status(identifier=1, created_at="2026-09-07T12:01:00Z")
        newer = status(
            state="failure",
            description="Review skipped",
            identifier=2,
            created_at="2026-09-07T12:02:01Z",
        )
        evidence = collect_evidence(
            FakeApi(
                [pull()],
                statuses=[[old, newer]],
                reviews=[[review()]],
                comments=[[acknowledgement()]],
            ),
            settings(),
            DISPATCH_AT,
        )

        self.assertEqual(evidence.status["id"], 2)
        self.assertFalse(evidence.complete)
        self.assertFalse(evidence.gate_satisfied)

    def test_newer_dismissed_review_tombstones_old_review(self) -> None:
        old = review(identifier=1, submitted_at="2026-09-07T12:01:00Z")
        newer = review(
            state="DISMISSED",
            identifier=2,
            submitted_at="2026-09-07T12:02:01Z",
        )
        evidence = collect_evidence(
            FakeApi(
                [pull()],
                statuses=[[status()]],
                reviews=[[old, newer]],
                comments=[[acknowledgement()]],
            ),
            settings(),
            DISPATCH_AT,
        )

        self.assertEqual(evidence.review["id"], 2)
        self.assertFalse(evidence.complete)
        self.assertFalse(evidence.gate_satisfied)

    def test_newer_unfinished_ack_tombstones_old_finished_ack(self) -> None:
        old = acknowledgement(identifier=1, created_at="2026-09-07T12:00:03Z")
        newer = acknowledgement(identifier=2, created_at="2026-09-07T12:00:06Z", finished=False)
        evidence = collect_evidence(
            FakeApi(
                [pull()],
                statuses=[[status()]],
                reviews=[[review()]],
                comments=[[old, newer]],
            ),
            settings(),
            DISPATCH_AT,
        )

        self.assertEqual(evidence.acknowledgement["id"], 2)
        self.assertFalse(evidence.gate_satisfied)

    def test_acknowledgement_finish_update_must_correlate_with_completion(self) -> None:
        stale = acknowledgement()
        stale["updated_at"] = "2026-09-07T13:00:00Z"
        evidence = collect_evidence(
            FakeApi(
                [pull()],
                statuses=[[status()]],
                reviews=[[review()]],
                comments=[[stale]],
            ),
            settings(),
            DISPATCH_AT,
        )

        self.assertFalse(evidence.command_correlated)
        self.assertFalse(evidence.gate_satisfied)

    def test_debounce_detects_same_head_base_movement_without_dispatch(self) -> None:
        api = FakeApi(
            [pull(), pull(), pull(base=NEXT_BASE)],
            statuses=[[]],
        )
        clock = FakeClock()

        outcome = run(
            settings(debounce_seconds=15), api, clock.sleep, clock.monotonic, clock.now
        )

        self.assertEqual(outcome.outcome, "superseded")
        self.assertEqual(outcome.observed_base, NEXT_BASE)
        self.assertEqual(api.created_comments, [])
        self.assertEqual(clock.sleeps, [15])

    def test_base_moves_during_evidence_read_never_returns_covered(self) -> None:
        api = successful_api(
            [pull(), pull(), pull(), pull(), pull(), pull(base=NEXT_BASE)]
        )
        clock = FakeClock()

        outcome = run(settings(), api, clock.sleep, clock.monotonic, clock.now)

        self.assertEqual(outcome.outcome, "superseded")
        self.assertEqual(outcome.observed_base, NEXT_BASE)
        self.assertTrue(outcome.fail)

    def test_changes_requested_is_never_turned_green(self) -> None:
        api = FakeApi(
            [pull()],
            statuses=[[], [status()]],
            reviews=[[review(state="CHANGES_REQUESTED")]],
            comments=[[acknowledgement()]],
        )
        clock = FakeClock()

        outcome = run(settings(), api, clock.sleep, clock.monotonic, clock.now)

        self.assertEqual(outcome.outcome, "reviewed-blocking")
        self.assertEqual(gate_state(settings(), outcome), "failure")
        self.assertFalse(outcome.evidence.gate_satisfied)

    def test_active_same_pair_lease_is_adopted_without_duplicate_command(self) -> None:
        api = FakeApi(
            [pull()],
            statuses=[[gate_status()], [status()]],
            reviews=[[review()]],
            comments=[[acknowledgement()]],
            stored_comments={29: command_comment()},
        )
        clock = FakeClock()

        outcome = run(settings(), api, clock.sleep, clock.monotonic, clock.now)

        self.assertEqual(outcome.outcome, "dispatched-covered")
        self.assertEqual(outcome.dispatch["lease"], "adopted")
        self.assertEqual(api.created_comments, [])
        self.assertEqual(api.gate_statuses, [])

    def test_lease_command_must_belong_to_current_pull_request(self) -> None:
        foreign = command_comment()
        foreign["issue_url"] = "https://api.github.test/repos/acme/widget/issues/8"
        api = FakeApi(
            [pull()],
            statuses=[[gate_status()]],
            stored_comments={29: foreign},
        )
        clock = FakeClock()

        with self.assertRaises(GovernorError):
            current_gate_lease(api, settings(), clock.now)

    def test_expired_lease_is_not_adopted(self) -> None:
        old = gate_status(created_at="2026-09-07T11:00:00Z")
        api = FakeApi([pull()], statuses=[[old], [status()]], reviews=[[review()]], comments=[[acknowledgement()]])
        clock = FakeClock()

        outcome = run(settings(), api, clock.sleep, clock.monotonic, clock.now)

        self.assertEqual(outcome.outcome, "dispatched-covered")
        self.assertEqual(outcome.dispatch["lease"], "created")
        self.assertEqual(api.created_comments, ["@coderabbitai full review"])

    def test_execute_detects_base_change_before_success_publication(self) -> None:
        api = successful_api([pull()] * 6 + [pull(base=NEXT_BASE)])
        clock = FakeClock()

        outcome = execute(settings(), api, clock.sleep, clock.monotonic, clock.now)

        self.assertEqual(outcome.outcome, "superseded")
        self.assertEqual([item["state"] for item in api.gate_statuses], ["pending", "pending", "error"])

    def test_execute_overwrites_success_if_base_moves_during_publication(self) -> None:
        api = successful_api([pull()] * 7 + [pull(base=NEXT_BASE)])
        clock = FakeClock()

        outcome = execute(settings(), api, clock.sleep, clock.monotonic, clock.now)

        self.assertEqual(outcome.outcome, "superseded")
        self.assertEqual(
            [item["state"] for item in api.gate_statuses],
            ["pending", "pending", "success", "error"],
        )

    def test_execute_publishes_only_exact_pair_success(self) -> None:
        api = successful_api()
        clock = FakeClock()

        outcome = execute(settings(), api, clock.sleep, clock.monotonic, clock.now)

        self.assertEqual(outcome.outcome, "dispatched-covered")
        self.assertEqual(api.gate_statuses[-1]["state"], "success")
        self.assertEqual(api.gate_statuses[-1]["head"], HEAD)
        self.assertEqual(api.gate_statuses[-1]["description"], f"Covered base {BASE}")

    def test_evidence_shape_matches_closed_schema_contract(self) -> None:
        api = successful_api()
        clock = FakeClock()
        outcome = run(settings(), api, clock.sleep, clock.monotonic, clock.now)
        schema_path = Path(__file__).resolve().parent / "evidence-v1.schema.json"
        schema = json.loads(schema_path.read_bytes())
        with tempfile.TemporaryDirectory() as directory:
            evidence_path = write_evidence(settings(), outcome, Path(directory))
            encoded = evidence_path.read_text()
            document = json.loads(encoded)

        self.assertNotIn("secret-token", encoded)
        self.assertEqual(set(document), set(schema["required"]))
        coverage_schema = schema["properties"]["coverage"]
        self.assertEqual(set(document["coverage"]), set(coverage_schema["required"]))
        self.assertEqual(document["coverage"]["basis"], "post_dispatch_provider_evidence")
        self.assertTrue(document["coverage"]["gate_satisfied"])
        self.assertTrue(document["coverage"]["command_correlated"])
        self.assertEqual(document["coverage"]["acknowledgement"]["command_handle"], COMMAND_HANDLE)
        self.assertIn(document["outcome"], schema["properties"]["outcome"]["enum"])
        self.assertEqual(set(document["dispatch"]), {"comment_id", "created_at", "command", "lease", "lease_status_id"})
        self.assertEqual(set(document["gate"]), {"context", "state"})

    def test_event_defaults_and_fixed_provider_contract_load(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            event_path = Path(directory) / "event.json"
            event_path.write_text(
                json.dumps(
                    {
                        "pull_request": {
                            "number": 9,
                            "base": {"sha": BASE},
                            "head": {"sha": HEAD},
                        }
                    }
                )
            )
            environment = self.environment(event_path)
            loaded = load_settings(environment)

        self.assertEqual(loaded.expected_base, BASE)
        self.assertEqual(loaded.expected_head, HEAD)
        self.assertEqual(loaded.pull_request_number, 9)

    def test_api_url_cannot_redirect_token_to_input_host(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            event_path = Path(directory) / "event.json"
            event_path.write_text("{}")
            environment = self.environment(event_path, explicit=True)
            environment["INPUT_API_URL"] = "https://evil.test"

            with self.assertRaisesRegex(GovernorError, "exactly match"):
                load_settings(environment)

    def test_repository_input_cannot_redirect_token_to_another_repository(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            event_path = Path(directory) / "event.json"
            event_path.write_text("{}")
            environment = self.environment(event_path, explicit=True)
            environment["INPUT_REPOSITORY"] = "attacker/project"

            with self.assertRaisesRegex(GovernorError, "workflow repository"):
                load_settings(environment)

    def test_unverified_command_and_handle_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            event_path = Path(directory) / "event.json"
            event_path.write_text("{}")
            environment = self.environment(event_path, explicit=True)
            environment["INPUT_COMMAND"] = "review"
            with self.assertRaisesRegex(GovernorError, "full-review"):
                load_settings(environment)
            environment["INPUT_COMMAND"] = "full-review"
            environment["INPUT_REVIEWER_HANDLE"] = "other-bot"
            with self.assertRaisesRegex(GovernorError, "command route"):
                load_settings(environment)
            environment["INPUT_REVIEWER_HANDLE"] = CODERABBIT_HANDLE
            environment["INPUT_GATE_CONTEXT"] = "Other Gate"
            with self.assertRaisesRegex(GovernorError, "provider contract"):
                load_settings(environment)

    def test_action_and_example_never_execute_pull_request_code(self) -> None:
        directory = Path(__file__).resolve().parent
        manifest = (directory / "action.yml").read_text()
        readme = (directory / "README.md").read_text()
        example = (directory.parent.parent / "examples/review-governor-coderabbit.yml").read_text()

        self.assertNotIn("actions/checkout", manifest)
        self.assertNotIn("actions/checkout", example)
        self.assertIn("default: full-review", manifest)
        self.assertIn("push:", example)
        self.assertIn("reconciliation queued", example)
        self.assertIn("pull_request_review:", example)
        self.assertIn("invalidate-provider-review-revocation:", example)
        self.assertIn("invalidate-provider-status-regression:", example)
        self.assertIn("listPullRequestsAssociatedWithCommit", example)
        self.assertIn("statuses: write", example)
        self.assertIn("pull-requests: read", example)
        self.assertNotIn("contents: read", example)
        self.assertNotIn("contents: write", example)
        self.assertIn("actions/github-script@f28e40c7f34bde8b3046d885e986cb6290c5673b", example)
        self.assertIn("require branches to be up to date before merging", readme)
        self.assertIn("dedicated StrataDiff GitHub App", readme)

    def test_public_provider_contract_matches_runtime_constants(self) -> None:
        repository = Path(__file__).resolve().parents[2]
        provider = json.loads(
            (repository / "benchmarks/review-governor-benchmark-v0/provider-contract-v0.json").read_bytes()
        )
        contract = provider["contract"]

        self.assertEqual(contract["review_bot_id"], CODERABBIT_BOT_ID)
        self.assertEqual(contract["review_bot_login"], CODERABBIT_LOGIN)
        self.assertEqual(contract["status_avatar_url"], f"https://avatars.githubusercontent.com/in/{CODERABBIT_APP_ID}?v=4")
        self.assertEqual(len(provider["review_evidence"]), 36)
        self.assertEqual(len(provider["command_routes"]["conversation_successes"]), 3)
        self.assertTrue(
            all(
                row["command"]["is_full_review_command"]
                for row in provider["command_routes"]["conversation_successes"]
            )
        )

    @staticmethod
    def environment(event_path: Path, explicit: bool = False) -> dict[str, str]:
        return {
            "GITHUB_EVENT_PATH": str(event_path),
            "GITHUB_REPOSITORY": "acme/widget",
            "GITHUB_API_URL": "https://api.github.test",
            "INPUT_MODE": "observe",
            "INPUT_REPOSITORY": "acme/widget" if explicit else "",
            "INPUT_PULL_REQUEST_NUMBER": "9" if explicit else "",
            "INPUT_EXPECTED_BASE": BASE if explicit else "",
            "INPUT_EXPECTED_HEAD": HEAD if explicit else "",
            "INPUT_API_URL": "",
            "INPUT_GITHUB_TOKEN": "token",
            "INPUT_DEBOUNCE_SECONDS": "0",
            "INPUT_POLL_SECONDS": "5",
            "INPUT_TIMEOUT_SECONDS": "600",
            "INPUT_COMMAND": "full-review",
            "INPUT_STATUS_CONTEXT": "CodeRabbit",
            "INPUT_REVIEWER_LOGIN": CODERABBIT_LOGIN,
            "INPUT_REVIEWER_HANDLE": CODERABBIT_HANDLE,
            "INPUT_GATE_CONTEXT": "StrataDiff Final Head",
            "STRATADIFF_RUN_URL": "https://github.test/acme/widget/actions/runs/99",
        }


if __name__ == "__main__":
    unittest.main()
