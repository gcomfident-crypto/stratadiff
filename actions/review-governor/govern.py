#!/usr/bin/env python3
"""Fail-closed final-head dispatch for CodeRabbit-backed pull requests."""

from __future__ import annotations

import hashlib
import json
import os
import re
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable


EVIDENCE_SCHEMA = "stratadiff-coderabbit-governor-evidence-v1"
MAX_RESPONSE_BYTES = 8 * 1024 * 1024
MAX_EVENT_BYTES = 8 * 1024 * 1024
MAX_REVIEW_PAGES = 10
PAGE_SIZE = 100
CODERABBIT_APP_ID = 347564
CODERABBIT_BOT_ID = 136622811
CODERABBIT_LOGIN = "coderabbitai[bot]"
CODERABBIT_HANDLE = "coderabbitai"
CODERABBIT_REVIEW_MARKER = "<!-- This is an auto-generated comment by CodeRabbit for review status -->"
CODERABBIT_COMMAND_INVOCATION_PATTERN = re.compile(
    r"<!-- CodeRabbit review command invocation: "
    r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}) -->"
)
CODERABBIT_FULL_REVIEW_FINISHED = "Full review finished."
MAX_EVIDENCE_SKEW_SECONDS = 300
GITHUB_ACTIONS_BOT_ID = 41898282
GITHUB_ACTIONS_LOGIN = "github-actions[bot]"
GITHUB_ACTIONS_APP_URL = "https://github.com/apps/github-actions"
GATE_CONTEXT = "StrataDiff Final Head"
REPOSITORY_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
OBJECT_ID_PATTERN = re.compile(r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$")
RUN_PATH_PATTERN = re.compile(r"^/[^/]+/[^/]+/actions/runs/[0-9]+$")


class GovernorError(RuntimeError):
    """A fail-closed governor decision with a safe user-facing explanation."""


@dataclass(frozen=True)
class Settings:
    mode: str
    repository: str
    pull_request_number: int
    expected_base: str
    expected_head: str
    api_url: str
    token: str
    debounce_seconds: int
    poll_seconds: int
    timeout_seconds: int
    command: str
    status_context: str
    reviewer_login: str
    reviewer_handle: str
    gate_context: str
    run_url: str


@dataclass(frozen=True)
class ProviderEvidence:
    status: dict | None
    review: dict | None
    acknowledgement: dict | None = None
    temporally_correlated: bool = False
    command_correlated: bool = False

    @property
    def complete(self) -> bool:
        return (
            self.status is not None
            and self.status["state"] == "success"
            and self.status["description"] == "Review completed"
            and self.review is not None
            and self.temporally_correlated
            and self.review["state"] in {"APPROVED", "CHANGES_REQUESTED", "COMMENTED"}
            and CODERABBIT_REVIEW_MARKER in self.review["body"]
        )

    @property
    def blocking(self) -> bool:
        return self.review is not None and self.review["state"] == "CHANGES_REQUESTED"

    @property
    def gate_satisfied(self) -> bool:
        return (
            self.complete
            and self.acknowledgement is not None
            and CODERABBIT_FULL_REVIEW_FINISHED in self.acknowledgement["body"]
            and self.command_correlated
            and not self.blocking
        )


@dataclass(frozen=True)
class Outcome:
    outcome: str
    expected_base: str
    observed_base: str
    expected_head: str
    observed_head: str
    reason: str
    evidence: ProviderEvidence
    dispatch: dict | None
    fail: bool = False


@dataclass(frozen=True)
class PullSnapshot:
    base: str
    head: str
    state: str
    merged: bool
    draft: bool


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class GitHubApi:
    def __init__(self, api_url: str, repository: str, pull_request_number: int, token: str):
        self.api_url = api_url.rstrip("/")
        self.repository = repository
        self.pull_request_number = pull_request_number
        self.token = token
        self.opener = urllib.request.build_opener(NoRedirect())

    def _request(self, method: str, path: str, body: dict | None = None) -> tuple[object, str]:
        encoded = None
        headers = {
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {self.token}",
            "User-Agent": "stratadiff-review-governor-v1",
            "X-GitHub-Api-Version": "2022-11-28",
        }
        if body is not None:
            encoded = json.dumps(body, separators=(",", ":")).encode()
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            f"{self.api_url}{path}", data=encoded, headers=headers, method=method
        )
        try:
            response = self.opener.open(request, timeout=30)
        except urllib.error.HTTPError as error:
            raise GovernorError(
                f"GitHub API returned HTTP {error.code} for {method} {path}"
            ) from error
        except urllib.error.URLError as error:
            raise GovernorError(f"GitHub API request failed for {method} {path}") from error
        with response:
            if response.status not in {200, 201}:
                raise GovernorError(
                    f"GitHub API returned HTTP {response.status} for {method} {path}"
                )
            payload = response.read(MAX_RESPONSE_BYTES + 1)
            if len(payload) > MAX_RESPONSE_BYTES:
                raise GovernorError(f"GitHub API response exceeded {MAX_RESPONSE_BYTES} bytes")
            link = response.headers["Link"] if "Link" in response.headers else ""
        decoded = json.loads(payload)
        return decoded, link

    def get_pull(self) -> dict:
        value, _ = self._request(
            "GET", f"/repos/{self.repository}/pulls/{self.pull_request_number}"
        )
        if not isinstance(value, dict):
            raise GovernorError("GitHub pull request response must be an object")
        return value

    def _list_pages(self, path: str) -> list[dict]:
        items = []
        for page in range(1, MAX_REVIEW_PAGES + 1):
            separator = "&" if "?" in path else "?"
            value, link = self._request(
                "GET", f"{path}{separator}per_page={PAGE_SIZE}&page={page}"
            )
            if not isinstance(value, list) or not all(isinstance(item, dict) for item in value):
                raise GovernorError("GitHub paginated response must be an array of objects")
            items.extend(value)
            if 'rel="next"' not in link:
                return items
        raise GovernorError(
            f"GitHub pagination exceeded the {MAX_REVIEW_PAGES * PAGE_SIZE}-item safety limit"
        )

    def list_reviews(self) -> list[dict]:
        return self._list_pages(
            f"/repos/{self.repository}/pulls/{self.pull_request_number}/reviews"
        )

    def list_statuses(self, head: str) -> list[dict]:
        return self._list_pages(
            f"/repos/{self.repository}/commits/{head}/statuses"
        )

    def list_comments(self) -> list[dict]:
        return self._list_pages(
            f"/repos/{self.repository}/issues/{self.pull_request_number}/comments"
        )

    def get_comment(self, comment_id: int) -> dict:
        value, _ = self._request(
            "GET", f"/repos/{self.repository}/issues/comments/{comment_id}"
        )
        if not isinstance(value, dict):
            raise GovernorError("GitHub issue-comment response must be an object")
        return value

    def create_comment(self, body: str) -> dict:
        value, _ = self._request(
            "POST",
            f"/repos/{self.repository}/issues/{self.pull_request_number}/comments",
            {"body": body},
        )
        if not isinstance(value, dict):
            raise GovernorError("GitHub create-comment response must be an object")
        return value

    def create_status(
        self, head: str, state: str, context: str, description: str, target_url: str
    ) -> dict:
        body = {
            "state": state,
            "context": context,
            "description": description,
            "target_url": target_url,
        }
        value, _ = self._request(
            "POST", f"/repos/{self.repository}/statuses/{head}", body
        )
        if not isinstance(value, dict):
            raise GovernorError("GitHub create-status response must be an object")
        return value


def utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def utc_datetime() -> datetime:
    return datetime.now(timezone.utc)


def parse_github_time(value: str) -> datetime:
    if not isinstance(value, str):
        raise GovernorError("GitHub timestamp must be a string")
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise GovernorError("GitHub timestamp must include a UTC offset")
    return parsed


def bounded_file(path: Path, limit: int, label: str) -> bytes:
    with path.open("rb") as handle:
        value = handle.read(limit + 1)
    if len(value) > limit:
        raise GovernorError(f"{label} exceeds {limit} bytes")
    return value


def positive_integer(name: str, value: str, minimum: int, maximum: int) -> int:
    if not value.isascii() or not value.isdigit():
        raise GovernorError(f"{name} must be an integer")
    parsed = int(value)
    if parsed < minimum or parsed > maximum:
        raise GovernorError(f"{name} must be between {minimum} and {maximum}")
    return parsed


def event_identity(environment: dict[str, str]) -> tuple[str, str, int]:
    event_path = Path(environment["GITHUB_EVENT_PATH"])
    event = json.loads(bounded_file(event_path, MAX_EVENT_BYTES, "GitHub event payload"))
    pull_request = event["pull_request"]
    return pull_request["base"]["sha"], pull_request["head"]["sha"], pull_request["number"]


def validated_api_url(value: str, label: str) -> str:
    parsed = urllib.parse.urlsplit(value)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
    ):
        raise GovernorError(f"{label} must be an HTTPS URL without credentials, query, or fragment")
    return urllib.parse.urlunsplit(
        (parsed.scheme, parsed.netloc, parsed.path.rstrip("/"), "", "")
    )


def load_settings(environment: dict[str, str]) -> Settings:
    event_base = ""
    event_head = ""
    event_number = 0
    if (
        not environment["INPUT_EXPECTED_BASE"]
        or not environment["INPUT_EXPECTED_HEAD"]
        or not environment["INPUT_PULL_REQUEST_NUMBER"]
    ):
        event_base, event_head, event_number = event_identity(environment)

    mode = environment["INPUT_MODE"]
    if mode not in {"observe", "govern"}:
        raise GovernorError("mode must be observe or govern")
    workflow_repository = environment["GITHUB_REPOSITORY"]
    if not REPOSITORY_PATTERN.fullmatch(workflow_repository):
        raise GovernorError("GITHUB_REPOSITORY must use OWNER/REPO form")
    repository = environment["INPUT_REPOSITORY"] or workflow_repository
    if not REPOSITORY_PATTERN.fullmatch(repository):
        raise GovernorError("repository must use OWNER/REPO form")
    if repository.lower() != workflow_repository.lower():
        raise GovernorError("repository must exactly match the current workflow repository")
    expected_base = (environment["INPUT_EXPECTED_BASE"] or event_base).lower()
    if not OBJECT_ID_PATTERN.fullmatch(expected_base):
        raise GovernorError("expected-base must be a full lowercase Git object ID")
    expected_head = (environment["INPUT_EXPECTED_HEAD"] or event_head).lower()
    if not OBJECT_ID_PATTERN.fullmatch(expected_head):
        raise GovernorError("expected-head must be a full lowercase Git object ID")
    pull_request_number = positive_integer(
        "pull-request-number",
        environment["INPUT_PULL_REQUEST_NUMBER"] or str(event_number),
        1,
        2**63 - 1,
    )
    api_url = validated_api_url(environment["GITHUB_API_URL"], "GITHUB_API_URL")
    requested_api_url = environment["INPUT_API_URL"]
    if requested_api_url and validated_api_url(requested_api_url, "api-url") != api_url:
        raise GovernorError("api-url must exactly match the current GITHUB_API_URL")
    token = environment["INPUT_GITHUB_TOKEN"]
    if not token or "\n" in token or "\r" in token:
        raise GovernorError("github-token must be non-empty and contain no line breaks")
    command = environment["INPUT_COMMAND"]
    if command != "full-review":
        raise GovernorError("the CodeRabbit adapter requires the verified full-review command")
    status_context = environment["INPUT_STATUS_CONTEXT"]
    reviewer_login = environment["INPUT_REVIEWER_LOGIN"]
    reviewer_handle = environment["INPUT_REVIEWER_HANDLE"]
    gate_context = environment["INPUT_GATE_CONTEXT"]
    if status_context != "CodeRabbit":
        raise GovernorError("status-context must match the verified CodeRabbit provider contract")
    if reviewer_login != CODERABBIT_LOGIN:
        raise GovernorError("reviewer-login must match the verified CodeRabbit bot identity")
    if reviewer_handle != CODERABBIT_HANDLE:
        raise GovernorError("reviewer-handle must match the verified CodeRabbit command route")
    if gate_context != GATE_CONTEXT:
        raise GovernorError("gate-context must match the fixed StrataDiff provider contract")
    run_url = environment["STRATADIFF_RUN_URL"]
    parsed_run_url = urllib.parse.urlsplit(run_url)
    if (
        parsed_run_url.scheme != "https"
        or not parsed_run_url.hostname
        or parsed_run_url.username is not None
        or parsed_run_url.password is not None
        or parsed_run_url.query
        or parsed_run_url.fragment
        or not RUN_PATH_PATTERN.fullmatch(parsed_run_url.path)
        or parsed_run_url.path.split("/", 3)[1:3]
        != repository.split("/", 1)
    ):
        raise GovernorError("the GitHub workflow run URL is invalid")
    return Settings(
        mode=mode,
        repository=repository,
        pull_request_number=pull_request_number,
        expected_base=expected_base,
        expected_head=expected_head,
        api_url=api_url,
        token=token,
        debounce_seconds=positive_integer(
            "debounce-seconds", environment["INPUT_DEBOUNCE_SECONDS"], 0, 3600
        ),
        poll_seconds=positive_integer(
            "poll-seconds", environment["INPUT_POLL_SECONDS"], 5, 300
        ),
        timeout_seconds=positive_integer(
            "timeout-seconds", environment["INPUT_TIMEOUT_SECONDS"], 60, 7200
        ),
        command=command,
        status_context=status_context,
        reviewer_login=reviewer_login,
        reviewer_handle=reviewer_handle,
        gate_context=gate_context,
        run_url=run_url,
    )


def pull_state(pull: dict) -> PullSnapshot:
    base = pull["base"]["sha"].lower()
    head = pull["head"]["sha"].lower()
    if not OBJECT_ID_PATTERN.fullmatch(base) or not OBJECT_ID_PATTERN.fullmatch(head):
        raise GovernorError("GitHub returned an invalid pull request base or head")
    state = pull["state"]
    merged = pull["merged"]
    draft = pull["draft"]
    if state not in {"open", "closed"} or not isinstance(merged, bool) or not isinstance(draft, bool):
        raise GovernorError("GitHub returned an invalid pull request state")
    return PullSnapshot(base, head, state, merged, draft)


def provider_user(user: dict | None, expected_login: str) -> bool:
    return (
        user is not None
        and user["id"] == CODERABBIT_BOT_ID
        and user["login"] == expected_login
        and user["type"] == "Bot"
        and user["html_url"] == "https://github.com/apps/coderabbitai"
    )


def github_actions_user(user: dict | None) -> bool:
    return (
        user is not None
        and user["id"] == GITHUB_ACTIONS_BOT_ID
        and user["login"] == GITHUB_ACTIONS_LOGIN
        and user["type"] == "Bot"
        and user["html_url"] == GITHUB_ACTIONS_APP_URL
    )


def collect_evidence(
    api: GitHubApi, settings: Settings, not_before: str | None = None
) -> ProviderEvidence:
    watermark = parse_github_time(not_before) if not_before is not None else None
    statuses = []
    for status in api.list_statuses(settings.expected_head):
        if status["context"] != settings.status_context:
            continue
        creator = status["creator"]
        avatar = urllib.parse.urlsplit(status["avatar_url"])
        if (
            not provider_user(creator, settings.reviewer_login)
            or avatar.scheme != "https"
            or avatar.hostname != "avatars.githubusercontent.com"
            or avatar.path != f"/in/{CODERABBIT_APP_ID}"
        ):
            continue
        if status["state"] not in {"error", "failure", "pending", "success"}:
            raise GovernorError("GitHub returned an unsupported commit status state")
        if watermark is not None and parse_github_time(status["created_at"]) <= watermark:
            continue
        statuses.append(status)
    latest_status = max(
        statuses,
        key=lambda item: (item["created_at"], item["id"]),
        default=None,
    )

    reviews = []
    for review in api.list_reviews():
        user = review["user"]
        if not provider_user(user, settings.reviewer_login):
            continue
        commit_id = review["commit_id"]
        submitted_at = review["submitted_at"]
        body = review["body"]
        if commit_id is None:
            continue
        if commit_id.lower() != settings.expected_head:
            continue
        if submitted_at is None or not isinstance(body, str):
            raise GovernorError("CodeRabbit returned malformed exact-head review evidence")
        if watermark is not None and parse_github_time(submitted_at) <= watermark:
            continue
        if review["state"] not in {"APPROVED", "CHANGES_REQUESTED", "COMMENTED", "DISMISSED"}:
            raise GovernorError("GitHub returned an unsupported pull request review state")
        reviews.append(review)
    latest_review = max(
        reviews, key=lambda item: (item["submitted_at"], item["id"]), default=None
    )
    temporally_correlated = False
    completion_time = None
    if latest_status is not None and latest_review is not None:
        status_time = parse_github_time(latest_status["created_at"])
        review_time = parse_github_time(latest_review["submitted_at"])
        temporally_correlated = (
            abs((status_time - review_time).total_seconds())
            <= MAX_EVIDENCE_SKEW_SECONDS
        )
        if temporally_correlated:
            completion_time = max(status_time, review_time)

    acknowledgements = []
    if watermark is not None:
        for comment in api.list_comments():
            if not provider_user(comment["user"], settings.reviewer_login):
                continue
            body = comment["body"]
            created_at = comment["created_at"]
            if not isinstance(body, str) or not isinstance(created_at, str):
                continue
            created_time = parse_github_time(created_at)
            updated_time = parse_github_time(comment["updated_at"])
            if updated_time < created_time:
                raise GovernorError("CodeRabbit acknowledgement update predates its creation")
            if created_time <= watermark:
                continue
            if not CODERABBIT_COMMAND_INVOCATION_PATTERN.search(body):
                continue
            acknowledgements.append(comment)
    acknowledgement = max(
        acknowledgements,
        key=lambda item: (item["created_at"], item["id"]),
        default=None,
    )
    command_correlated = False
    if acknowledgement is not None and completion_time is not None:
        acknowledgement_created = parse_github_time(acknowledgement["created_at"])
        acknowledgement_finished = parse_github_time(acknowledgement["updated_at"])
        finish_lag = (acknowledgement_finished - completion_time).total_seconds()
        command_correlated = (
            acknowledgement_created <= completion_time
            and 0 <= finish_lag <= MAX_EVIDENCE_SKEW_SECONDS
        )
    return ProviderEvidence(
        latest_status,
        latest_review,
        acknowledgement,
        temporally_correlated,
        command_correlated,
    )


def waiting_description(base: str) -> str:
    return f"Waiting to review base {base}"


def reviewing_description(base: str, comment_id: int) -> str:
    return f"Reviewing base {base}; command {comment_id}"


def covered_description(base: str) -> str:
    return f"Covered base {base}"


def trusted_run_target(value: str, settings: Settings) -> bool:
    if not isinstance(value, str):
        return False
    candidate = urllib.parse.urlsplit(value)
    current = urllib.parse.urlsplit(settings.run_url)
    return (
        candidate.scheme == current.scheme
        and candidate.netloc == current.netloc
        and candidate.username is None
        and candidate.password is None
        and not candidate.query
        and not candidate.fragment
        and RUN_PATH_PATTERN.fullmatch(candidate.path) is not None
        and candidate.path.split("/", 3)[1:3] == settings.repository.split("/", 1)
    )


def current_gate_lease(
    api: GitHubApi,
    settings: Settings,
    now: Callable[[], datetime],
) -> dict | None:
    statuses = []
    for status in api.list_statuses(settings.expected_head):
        if status["context"] != settings.gate_context:
            continue
        if not github_actions_user(status["creator"]):
            continue
        if not trusted_run_target(status["target_url"], settings):
            continue
        statuses.append(status)
    latest = max(
        statuses,
        key=lambda item: (item["created_at"], item["id"]),
        default=None,
    )
    if latest is None:
        return None
    match = re.fullmatch(
        rf"Reviewing base {re.escape(settings.expected_base)}; command ([0-9]+)",
        latest["description"],
    )
    if latest["state"] != "pending" or match is None:
        return None
    created = parse_github_time(latest["created_at"])
    age = (now() - created).total_seconds()
    if age < -60 or age > settings.timeout_seconds + settings.poll_seconds:
        return None
    comment_id = int(match.group(1))
    comment = api.get_comment(comment_id)
    expected_body = f"@{CODERABBIT_HANDLE} full review"
    if (
        comment["id"] != comment_id
        or comment["body"] != expected_body
        or not github_actions_user(comment["user"])
        or comment["issue_url"]
        != (
            f"{settings.api_url}/repos/{settings.repository}/issues/"
            f"{settings.pull_request_number}"
        )
        or parse_github_time(comment["created_at"]) > created
    ):
        raise GovernorError("the active Governor lease does not bind a valid command comment")
    dispatch = {
        "comment_id": comment_id,
        "created_at": comment["created_at"],
        "command": expected_body,
        "lease": "adopted",
        "lease_status_id": latest["id"],
    }
    return dispatch


def current_outcome(
    pull: dict, settings: Settings, evidence: ProviderEvidence, dispatch: dict | None
) -> Outcome | None:
    snapshot = pull_state(pull)
    if snapshot.base != settings.expected_base or snapshot.head != settings.expected_head:
        return Outcome(
            "superseded",
            settings.expected_base,
            snapshot.base,
            settings.expected_head,
            snapshot.head,
            "the pull request base or head changed before exact-diff coverage completed",
            evidence,
            dispatch,
            True,
        )
    if dispatch is not None and evidence.blocking:
        return Outcome(
            "reviewed-blocking",
            settings.expected_base,
            snapshot.base,
            settings.expected_head,
            snapshot.head,
            "the post-dispatch provider review for the expected base/head requested changes",
            evidence,
            dispatch,
            True,
        )
    if dispatch is not None and evidence.gate_satisfied:
        return Outcome(
            "dispatched-covered",
            settings.expected_base,
            snapshot.base,
            settings.expected_head,
            snapshot.head,
            "post-dispatch provider evidence binds the live expected base/head observation",
            evidence,
            dispatch,
        )
    if snapshot.state == "closed":
        return Outcome(
            "merged-uncovered" if snapshot.merged else "closed-uncovered",
            settings.expected_base,
            snapshot.base,
            settings.expected_head,
            snapshot.head,
            "the pull request became terminal without exact-diff review evidence",
            evidence,
            dispatch,
            True,
        )
    if snapshot.draft:
        return Outcome(
            "deferred-draft",
            settings.expected_base,
            snapshot.base,
            settings.expected_head,
            snapshot.head,
            "draft pull requests are observed but not dispatched",
            evidence,
            dispatch,
        )
    return None


def inspect_current(
    api: GitHubApi,
    settings: Settings,
    dispatch: dict | None,
    not_before: str | None = None,
) -> tuple[dict, ProviderEvidence, Outcome | None]:
    before = api.get_pull()
    before_snapshot = pull_state(before)
    if (
        before_snapshot.base != settings.expected_base
        or before_snapshot.head != settings.expected_head
    ):
        evidence = ProviderEvidence(None, None)
        return before, evidence, current_outcome(before, settings, evidence, dispatch)
    evidence = collect_evidence(api, settings, not_before)
    after = api.get_pull()
    return after, evidence, current_outcome(after, settings, evidence, dispatch)


def inspect_identity(
    api: GitHubApi, settings: Settings
) -> tuple[dict, Outcome | None]:
    pull = api.get_pull()
    evidence = ProviderEvidence(None, None)
    return pull, current_outcome(pull, settings, evidence, None)


def run(
    settings: Settings,
    api: GitHubApi,
    sleep: Callable[[float], None] = time.sleep,
    monotonic: Callable[[], float] = time.monotonic,
    now: Callable[[], datetime] = utc_datetime,
) -> Outcome:
    if settings.mode == "observe":
        pull, evidence, decided = inspect_current(api, settings, None)
        if decided is not None:
            return decided
        snapshot = pull_state(pull)
        if evidence.complete:
            return Outcome(
                "observed-provider-evidence",
                settings.expected_base,
                snapshot.base,
                settings.expected_head,
                snapshot.head,
                "provider evidence binds the head, but no Governor receipt proves its base",
                evidence,
                None,
            )
        return Outcome(
            "observed-uncovered",
            settings.expected_base,
            snapshot.base,
            settings.expected_head,
            snapshot.head,
            "the live head does not yet have complete provider evidence",
            evidence,
            None,
        )

    pull, decided = inspect_identity(api, settings)
    if decided is not None:
        return decided
    dispatch = current_gate_lease(api, settings, now)
    pull, decided = inspect_identity(api, settings)
    if decided is not None:
        return decided
    if dispatch is None:
        publish_gate(
            api,
            settings,
            "pending",
            waiting_description(settings.expected_base),
        )
        if settings.debounce_seconds:
            sleep(settings.debounce_seconds)
        pull, decided = inspect_identity(api, settings)
        if decided is not None:
            return decided

        body = f"@{CODERABBIT_HANDLE} full review"
        comment = api.create_comment(body)
        if (
            comment["body"] != body
            or not github_actions_user(comment["user"])
            or not isinstance(comment["id"], int)
            or comment["id"] < 1
        ):
            raise GovernorError("GitHub did not create the verified provider command comment")
        parse_github_time(comment["created_at"])
        dispatch = {
            "comment_id": comment["id"],
            "created_at": comment["created_at"],
            "command": body,
            "lease": "created",
            "lease_status_id": None,
        }
        pull, decided = inspect_identity(api, settings)
        if decided is not None:
            return Outcome(
                decided.outcome,
                decided.expected_base,
                decided.observed_base,
                decided.expected_head,
                decided.observed_head,
                decided.reason,
                decided.evidence,
                dispatch,
                decided.fail,
            )
        lease_status = publish_gate(
            api,
            settings,
            "pending",
            reviewing_description(settings.expected_base, comment["id"]),
        )
        dispatch["lease_status_id"] = lease_status["id"]

    elapsed = max(0.0, (now() - parse_github_time(dispatch["created_at"])).total_seconds())
    deadline = monotonic() + max(0.0, settings.timeout_seconds - elapsed)
    while True:
        pull, evidence, decided = inspect_current(
            api, settings, dispatch, dispatch["created_at"]
        )
        if decided is not None:
            return decided
        remaining = deadline - monotonic()
        if remaining <= 0:
            snapshot = pull_state(pull)
            return Outcome(
                "timed-out-uncovered",
                settings.expected_base,
                snapshot.base,
                settings.expected_head,
                snapshot.head,
                "post-dispatch provider evidence did not complete before the configured timeout",
                evidence,
                dispatch,
                True,
            )
        sleep(min(settings.poll_seconds, remaining))


def public_status(status: dict | None) -> dict | None:
    if status is None:
        return None
    return {
        "id": status["id"],
        "context": status["context"],
        "state": status["state"],
        "description": status["description"],
        "provider_app_id": CODERABBIT_APP_ID,
        "creator_id": status["creator"]["id"],
        "creator_login": status["creator"]["login"],
        "created_at": status["created_at"],
        "updated_at": status["updated_at"],
    }


def public_review(review: dict | None) -> dict | None:
    if review is None:
        return None
    return {
        "id": review["id"],
        "state": review["state"],
        "commit_id": review["commit_id"],
        "submitted_at": review["submitted_at"],
        "body_bytes": len(review["body"].encode()),
        "body_sha256": hashlib.sha256(review["body"].encode()).hexdigest(),
        "reviewer_id": review["user"]["id"],
        "reviewer_login": review["user"]["login"],
    }


def public_acknowledgement(comment: dict | None) -> dict | None:
    if comment is None:
        return None
    match = CODERABBIT_COMMAND_INVOCATION_PATTERN.search(comment["body"])
    if match is None:
        raise GovernorError("the selected CodeRabbit acknowledgement has no command handle")
    return {
        "id": comment["id"],
        "created_at": comment["created_at"],
        "updated_at": comment["updated_at"],
        "body_bytes": len(comment["body"].encode()),
        "body_sha256": hashlib.sha256(comment["body"].encode()).hexdigest(),
        "command_handle": match.group(1),
        "full_review_finished": CODERABBIT_FULL_REVIEW_FINISHED in comment["body"],
        "reviewer_id": comment["user"]["id"],
        "reviewer_login": comment["user"]["login"],
    }


def gate_state(settings: Settings, outcome: Outcome) -> str | None:
    if settings.mode == "observe":
        return None
    if outcome.outcome == "dispatched-covered":
        return "success"
    if outcome.outcome == "deferred-draft":
        return "pending"
    if outcome.outcome == "reviewed-blocking":
        return "failure"
    return "error"


def publish_gate(
    api: GitHubApi, settings: Settings, state: str, description: str
) -> dict:
    return api.create_status(
        settings.expected_head,
        state,
        settings.gate_context,
        description[:140],
        settings.run_url,
    )


def execute(
    settings: Settings,
    api: GitHubApi,
    sleep: Callable[[float], None] = time.sleep,
    monotonic: Callable[[], float] = time.monotonic,
    now: Callable[[], datetime] = utc_datetime,
) -> Outcome:
    try:
        outcome = run(settings, api, sleep, monotonic, now)
    except GovernorError as error:
        outcome = Outcome(
            "blocked",
            settings.expected_base,
            settings.expected_base,
            settings.expected_head,
            settings.expected_head,
            str(error),
            ProviderEvidence(None, None),
            None,
            True,
        )
    final_gate_state = gate_state(settings, outcome)
    if final_gate_state == "success":
        live = pull_state(api.get_pull())
        if live.base != settings.expected_base or live.head != settings.expected_head:
            outcome = Outcome(
                "superseded",
                settings.expected_base,
                live.base,
                settings.expected_head,
                live.head,
                "the pull request base or head changed before gate publication",
                outcome.evidence,
                outcome.dispatch,
                True,
            )
            final_gate_state = "error"
    if final_gate_state is not None:
        description = (
            covered_description(settings.expected_base)
            if final_gate_state == "success"
            else (
                waiting_description(settings.expected_base)
                if final_gate_state == "pending"
                else outcome.reason
            )
        )
        publish_gate(api, settings, final_gate_state, description)
    if final_gate_state == "success":
        live = pull_state(api.get_pull())
        if live.base != settings.expected_base or live.head != settings.expected_head:
            outcome = Outcome(
                "superseded",
                settings.expected_base,
                live.base,
                settings.expected_head,
                live.head,
                "the pull request base or head changed while the success gate was published",
                outcome.evidence,
                outcome.dispatch,
                True,
            )
            publish_gate(api, settings, "error", outcome.reason)
    return outcome


def write_evidence(settings: Settings, outcome: Outcome, runner_temp: Path) -> Path:
    document = {
        "schema": EVIDENCE_SCHEMA,
        "generated_at": utc_now(),
        "provider": "coderabbit",
        "repository": settings.repository,
        "pull_request_number": settings.pull_request_number,
        "expected_base_sha": outcome.expected_base,
        "observed_base_sha": outcome.observed_base,
        "expected_head_sha": outcome.expected_head,
        "observed_head_sha": outcome.observed_head,
        "outcome": outcome.outcome,
        "reason": outcome.reason,
        "coverage": {
            "complete": outcome.evidence.complete,
            "gate_satisfied": outcome.evidence.gate_satisfied,
            "basis": (
                "post_dispatch_provider_evidence"
                if outcome.evidence.gate_satisfied
                else "none"
            ),
            "temporally_correlated": outcome.evidence.temporally_correlated,
            "command_correlated": outcome.evidence.command_correlated,
            "maximum_evidence_skew_seconds": MAX_EVIDENCE_SKEW_SECONDS,
            "not_before": (
                None if outcome.dispatch is None else outcome.dispatch["created_at"]
            ),
            "required_evidence": [
                "coderabbit_app_review_completed_status",
                "substantive_exact_head_coderabbit_bot_review",
                "post_dispatch_coderabbit_command_acknowledgement",
                "bounded_completion_time_skew",
            ],
            "status": public_status(outcome.evidence.status),
            "review": public_review(outcome.evidence.review),
            "acknowledgement": public_acknowledgement(
                outcome.evidence.acknowledgement
            ),
        },
        "dispatch": outcome.dispatch,
        "gate": (
            None
            if settings.mode == "observe"
            else {
                "context": settings.gate_context,
                "state": gate_state(settings, outcome),
            }
        ),
        "attestation": None,
        "claim_boundary": (
            "This is a live GitHub observation, not a signed review receipt and not evidence "
            "that CodeRabbit's private context was cached or reproduced. The public status and "
            "review bind the head only; a post-dispatch CodeRabbit acknowledgement exposes a "
            "provider command handle, while the Governor's before/after observations and gate "
            "publication supply the base binding within this invocation."
        ),
    }
    runner_temp.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(
        prefix="stratadiff-review-governor-", suffix=".json", dir=runner_temp
    )
    path = Path(name)
    with os.fdopen(descriptor, "wb") as handle:
        handle.write((json.dumps(document, indent=2, sort_keys=True) + "\n").encode())
    return path.resolve()


def append_outputs(path: Path, values: dict[str, str]) -> None:
    with path.open("a", encoding="utf-8") as handle:
        for name, value in values.items():
            if "\n" in value or "\r" in value:
                raise GovernorError(f"output {name} cannot contain a line break")
            handle.write(f"{name}={value}\n")


def append_summary(path: Path, outcome: Outcome, evidence_path: Path) -> None:
    status = outcome.evidence.status
    review = outcome.evidence.review
    acknowledgement = outcome.evidence.acknowledgement
    lines = [
        "## StrataDiff final-head governor",
        "",
        f"- Outcome: `{outcome.outcome}`",
        f"- Expected base: `{outcome.expected_base}`",
        f"- Observed base: `{outcome.observed_base}`",
        f"- Expected head: `{outcome.expected_head}`",
        f"- Observed head: `{outcome.observed_head}`",
        f"- Reason: {outcome.reason}",
        f"- CodeRabbit status: `{status['state']}`" if status is not None else "- CodeRabbit status: missing",
        f"- Exact-head CodeRabbit review: `{review['id']}`" if review is not None else "- Exact-head CodeRabbit review: missing",
        (
            f"- Post-dispatch CodeRabbit acknowledgement: `{acknowledgement['id']}`"
            if acknowledgement is not None
            else "- Post-dispatch CodeRabbit acknowledgement: missing"
        ),
        f"- Evidence: `{evidence_path}`",
        "",
        "A successful status alone is insufficient because CodeRabbit can report success while a review is paused.",
    ]
    with path.open("a", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")


def main() -> int:
    environment = dict(os.environ)
    settings = load_settings(environment)
    api = GitHubApi(
        settings.api_url,
        settings.repository,
        settings.pull_request_number,
        settings.token,
    )
    outcome = execute(settings, api)
    evidence_path = write_evidence(settings, outcome, Path(environment["RUNNER_TEMP"]))
    append_outputs(
        Path(environment["GITHUB_OUTPUT"]),
        {
            "outcome": outcome.outcome,
            "expected_base": outcome.expected_base,
            "observed_base": outcome.observed_base,
            "expected_head": outcome.expected_head,
            "observed_head": outcome.observed_head,
            "evidence": str(evidence_path),
        },
    )
    append_summary(Path(environment["GITHUB_STEP_SUMMARY"]), outcome, evidence_path)
    print(f"StrataDiff governor: {outcome.outcome}: {outcome.reason}", file=sys.stderr)
    return 1 if outcome.fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
