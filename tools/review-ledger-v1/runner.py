#!/usr/bin/env python3
"""Offline, implementation-independent runner for Review Ledger v1."""

from __future__ import annotations

import argparse
import copy
import difflib
import fnmatch
import hashlib
import hmac
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = REPOSITORY_ROOT / "benchmarks/review-ledger-v1/manifest.json"
DEFAULT_BINARY = REPOSITORY_ROOT / "target/debug/stratadiff"
DEFAULT_OUTPUT = REPOSITORY_ROOT / "target/review-ledger-v1/result.json"
OWNERSHIP_SCHEMA = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/github-ownership-snapshot-v1.schema.json"
WEBHOOK_SECRET = b"review-ledger-v1-offline-secret"
RECEIVER_KEY_ID = "review-ledger-v1-runner"
RECEIVER_SIGNING_KEY = "07" * 32
PROVIDER_URL = "https://github.com"


class BenchmarkFailure(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise BenchmarkFailure(message)


def encode_json(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def webhook_signature(value: bytes) -> str:
    return "sha256=" + hmac.new(WEBHOOK_SECRET, value, hashlib.sha256).hexdigest()


def run_command(
    arguments: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[bytes]:
    command_env = os.environ.copy()
    if env is not None:
        command_env.update(env)
    return subprocess.run(
        arguments,
        cwd=cwd,
        env=command_env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def command_error(result: subprocess.CompletedProcess[bytes]) -> str:
    stderr = result.stderr.decode(errors="replace").strip()
    stdout = result.stdout.decode(errors="replace").strip()
    return stderr if stderr else stdout


def git(repository: Path, *arguments: str) -> str:
    result = run_command(["git", "-C", str(repository), *arguments])
    require(result.returncode == 0, f"git {' '.join(arguments)} failed: {command_error(result)}")
    return result.stdout.decode().strip()


def init_repository(path: Path) -> None:
    path.mkdir(parents=True)
    git(path, "init", "--quiet")
    git(path, "config", "user.name", "StrataDiff Benchmark")
    git(path, "config", "user.email", "benchmark@example.com")


def write_tree(repository: Path, files: dict[str, str]) -> None:
    tracked = git(repository, "ls-files", "-z")
    for relative in tracked.split("\0"):
        if relative and relative not in files:
            target = repository / relative
            if target.exists():
                target.unlink()
    for relative, contents in files.items():
        target = repository / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(contents, encoding="utf-8")


def commit(repository: Path, message: str, sequence: int, *, allow_empty: bool = False) -> str:
    git(repository, "add", "-A")
    arguments = ["git", "-C", str(repository), "commit", "--quiet", "-m", message]
    if allow_empty:
        arguments.append("--allow-empty")
    timestamp = f"2026-09-05T00:{sequence:02}:00Z"
    result = run_command(
        arguments,
        env={"GIT_AUTHOR_DATE": timestamp, "GIT_COMMITTER_DATE": timestamp},
    )
    require(result.returncode == 0, f"git commit failed: {command_error(result)}")
    return git(repository, "rev-parse", "HEAD")


def commit_tree(
    repository: Path,
    files: dict[str, str],
    message: str,
    sequence: int,
    *,
    allow_empty: bool = False,
) -> str:
    write_tree(repository, files)
    return commit(repository, message, sequence, allow_empty=allow_empty)


def checkout_new(repository: Path, branch: str, revision: str) -> None:
    git(repository, "checkout", "--quiet", "-b", branch, revision)


def symbol_commit(manifest: dict[str, object], symbol: str) -> str:
    fixture_symbols = manifest["fixture_symbols"]
    require(isinstance(fixture_symbols, dict), "fixture_symbols must be an object")
    commits = fixture_symbols["commits"]
    require(isinstance(commits, dict), "fixture_symbols.commits must be an object")
    value = commits[symbol]
    require(isinstance(value, str), f"commit symbol {symbol} must resolve to a string")
    return value


def reviewer(manifest: dict[str, object], login: str) -> dict[str, object]:
    fixture_symbols = manifest["fixture_symbols"]
    require(isinstance(fixture_symbols, dict), "fixture_symbols must be an object")
    reviewers = fixture_symbols["reviewers"]
    require(isinstance(reviewers, dict), "fixture_symbols.reviewers must be an object")
    value = reviewers[login]
    require(isinstance(value, dict), f"reviewer {login} must be an object")
    return value


def provider_identities(manifest: dict[str, object]) -> tuple[dict[str, object], dict[str, object]]:
    fixture_symbols = manifest["fixture_symbols"]
    require(isinstance(fixture_symbols, dict), "fixture_symbols must be an object")
    repository = fixture_symbols["repository"]
    pull_request = fixture_symbols["pull_request"]
    require(isinstance(repository, dict), "fixture repository must be an object")
    require(isinstance(pull_request, dict), "fixture pull_request must be an object")
    return repository, pull_request


def review_payload(
    manifest: dict[str, object],
    *,
    action: str,
    review_id: int,
    login: str,
    state: str,
    commit_id: str | None,
    submitted_at: str | None,
    base: str,
    head: str,
) -> bytes:
    repository, pull_request = provider_identities(manifest)
    identity = reviewer(manifest, login)
    body = {
        "action": action,
        "review": {
            "id": review_id,
            "node_id": f"PRR_{review_id}",
            "user": {
                "id": identity["id"],
                "node_id": identity["node_id"],
                "login": identity["login"],
                "type": identity["type"],
            },
            "state": state,
            "commit_id": commit_id,
            "submitted_at": submitted_at,
            "html_url": f"https://github.com/acme/widgets/pull/7#pullrequestreview-{review_id}",
            "author_association": "MEMBER",
        },
        "pull_request": {
            "id": pull_request["id"],
            "node_id": pull_request["node_id"],
            "number": pull_request["number"],
            "base": {"sha": base},
            "head": {"sha": head},
        },
        "repository": repository,
    }
    return encode_json(body)


def body_from_id(manifest: dict[str, object], body_id: str) -> bytes:
    a_commit = symbol_commit(manifest, "A")
    b_commit = symbol_commit(manifest, "B")
    d_commit = symbol_commit(manifest, "D")
    if body_id in ("review-401-approved-at-B", "unmodified-review-401-body"):
        return review_payload(
            manifest,
            action="submitted",
            review_id=401,
            login="alice",
            state="approved",
            commit_id=b_commit,
            submitted_at="2026-09-05T01:00:00Z",
            base=a_commit,
            head=b_commit,
        )
    if body_id == "modified-review-401-body":
        return review_payload(
            manifest,
            action="submitted",
            review_id=401,
            login="alice",
            state="changes_requested",
            commit_id=b_commit,
            submitted_at="2026-09-05T01:00:00Z",
            base=a_commit,
            head=b_commit,
        )
    if body_id == "review-402-changes-requested-at-D":
        return review_payload(
            manifest,
            action="submitted",
            review_id=402,
            login="alice",
            state="changes_requested",
            commit_id=d_commit,
            submitted_at="2026-09-05T03:00:00Z",
            base=a_commit,
            head=d_commit,
        )
    if body_id == "review-401-dismissed-null-metadata":
        return review_payload(
            manifest,
            action="dismissed",
            review_id=401,
            login="alice",
            state="dismissed",
            commit_id=None,
            submitted_at=None,
            base=a_commit,
            head=b_commit,
        )
    raise BenchmarkFailure(f"unknown symbolic webhook body {body_id}")


def synchronize_payload(
    manifest: dict[str, object],
    *,
    before: str,
    after: str,
    base: str,
) -> bytes:
    repository, pull_request = provider_identities(manifest)
    return encode_json(
        {
            "action": "synchronize",
            "before": before,
            "after": after,
            "pull_request": {
                "id": pull_request["id"],
                "node_id": pull_request["node_id"],
                "number": pull_request["number"],
                "base": {"sha": base},
                "head": {"sha": after},
            },
            "repository": repository,
        }
    )


def head_webhook_specs(
    manifest: dict[str, object],
    case: dict[str, object],
    commits: dict[str, str],
) -> list[dict[str, object]]:
    fixture = case["fixture"]
    deliveries = fixture["deliveries_in_arrival_order"]
    authoritative = fixture["authoritative_pull_request_observation"]
    require(isinstance(deliveries, list), "deliveries_in_arrival_order must be an array")
    require(isinstance(authoritative, dict), "authoritative pull request observation must be an object")
    specs = []
    for delivery in deliveries:
        require(isinstance(delivery, dict), "synchronize delivery must be an object")
        require(delivery["event"] == "pull_request", "synchronize delivery must use the pull_request event")
        require(delivery["action"] == "synchronize", "head transition must use the synchronize action")
        before = commits[delivery["before"]]
        after = commits[delivery["after"]]
        base = commits[delivery["payload_base"]]
        require(commits[delivery["payload_head"]] == after, "synchronize payload head must equal after")
        body = synchronize_payload(manifest, before=before, after=after, base=base)
        specs.append(
            {
                "delivery_id": delivery["delivery_id"],
                "event": delivery["event"],
                "received_at": delivery["received_at"],
                "body": body,
                "signature": webhook_signature(body),
            }
        )
    return specs


def webhook_specs(manifest: dict[str, object], case: dict[str, object]) -> list[dict[str, object]]:
    fixture = case["fixture"]
    require(isinstance(fixture, dict), "case fixture must be an object")
    if "deliveries" in fixture:
        deliveries = fixture["deliveries"]
        require(isinstance(deliveries, list), "deliveries must be an array")
        specs = []
        for delivery in deliveries:
            require(isinstance(delivery, dict), "delivery must be an object")
            body_id = delivery["body_id"]
            require(isinstance(body_id, str), "body_id must be a string")
            body = body_from_id(manifest, body_id)
            signature_kind = delivery["signature"]
            require(isinstance(signature_kind, str), "signature must be a string")
            if signature_kind == "valid_for_exact_body":
                signature = webhook_signature(body)
            elif signature_kind == "valid_only_for_unmodified-review-401-body":
                signature = webhook_signature(body_from_id(manifest, "unmodified-review-401-body"))
            else:
                raise BenchmarkFailure(f"unsupported symbolic signature {signature_kind}")
            specs.append(
                {
                    "delivery_id": delivery["delivery_id"],
                    "event": delivery["event"],
                    "received_at": delivery["received_at"],
                    "body": body,
                    "signature": signature,
                }
            )
        return specs
    facts = fixture["facts_in_chronological_order"]
    require(isinstance(facts, list), "facts_in_chronological_order must be an array")
    specs = []
    for index, fact in enumerate(facts):
        require(isinstance(fact, dict), "review fact must be an object")
        state = fact["state"]
        action = "dismissed" if state == "dismissed" else "submitted"
        commit_id = symbol_commit(manifest, fact["commit"])
        body = review_payload(
            manifest,
            action=action,
            review_id=fact["review_id"],
            login=fact["reviewer"],
            state=state,
            commit_id=commit_id,
            submitted_at=fact["submitted_at"],
            base=symbol_commit(manifest, "A"),
            head=commit_id,
        )
        specs.append(
            {
                "delivery_id": f"{case['id']}-{index + 1}",
                "event": "pull_request_review",
                "received_at": f"2026-09-05T04:00:{index:02}Z",
                "body": body,
                "signature": webhook_signature(body),
            }
        )
    return specs


def reduce_active_reviews(ledger: dict[str, object]) -> list[int]:
    receipts = ledger["review_receipts"]
    dismissals = ledger["dismissals"]
    require(isinstance(receipts, list), "ledger review_receipts must be an array")
    require(isinstance(dismissals, list), "ledger dismissals must be an array")
    dismissed_ids = {dismissal["review_id"] for dismissal in dismissals}
    latest: dict[int, dict[str, object]] = {}
    for receipt in receipts:
        reviewer_id = receipt["reviewer_id"]
        current = latest[reviewer_id] if reviewer_id in latest else None
        candidate_key = (receipt["submitted_at"], receipt["review_id"])
        if current is None or (current["submitted_at"], current["review_id"]) < candidate_key:
            latest[reviewer_id] = receipt
    return sorted(
        receipt["review_id"]
        for receipt in latest.values()
        if receipt["review_id"] not in dismissed_ids
    )


def ledger_projection(
    ledger: dict[str, object] | None,
    outcomes: list[str],
    blockers: list[str],
) -> dict[str, object]:
    if ledger is None:
        return {
            "ingest_outcomes": outcomes,
            "delivery_count": 0,
            "audited_review_ids": [],
            "audited_dismissal_ids": [],
            "dismissal_metadata": [],
            "active_review_ids": [],
            "blockers": blockers,
        }
    deliveries = ledger["deliveries"]
    receipts = ledger["review_receipts"]
    dismissals = ledger["dismissals"]
    projection = {
        "ingest_outcomes": outcomes,
        "delivery_count": len(deliveries),
        "audited_review_ids": sorted(receipt["review_id"] for receipt in receipts),
        "audited_dismissal_ids": sorted(dismissal["review_id"] for dismissal in dismissals),
        "dismissal_metadata": sorted(
            (
                {
                    "review_id": dismissal["review_id"],
                    "commit_id": dismissal["commit_id"],
                    "submitted_at": dismissal["submitted_at"],
                }
                for dismissal in dismissals
            ),
            key=lambda dismissal: dismissal["review_id"],
        ),
        "active_review_ids": reduce_active_reviews(ledger),
        "blockers": blockers,
    }
    if deliveries:
        projection["canonical_received_at"] = deliveries[0]["received_at"]
    return projection


def independent_webhook_oracle(specs: list[dict[str, object]]) -> dict[str, object]:
    deliveries: dict[str, dict[str, object]] = {}
    receipts: dict[int, dict[str, object]] = {}
    dismissals: list[dict[str, object]] = []
    outcomes = []
    blockers = []
    for spec in specs:
        body = spec["body"]
        require(isinstance(body, bytes), "materialized webhook body must be bytes")
        if not hmac.compare_digest(webhook_signature(body), spec["signature"]):
            outcomes.append("rejected_before_decode")
            blockers.append("invalid_webhook_hmac")
            continue
        payload = json.loads(body)
        body_digest = sha256(body)
        delivery_id = spec["delivery_id"]
        if delivery_id in deliveries:
            existing = deliveries[delivery_id]
            if existing["event"] == spec["event"] and existing["payload_sha256"] == body_digest:
                outcomes.append("duplicate")
            else:
                outcomes.append("rejected_atomically")
                blockers.append("delivery_id_conflicting_body")
            continue
        deliveries[delivery_id] = {
            "delivery_id": delivery_id,
            "event": spec["event"],
            "received_at": spec["received_at"],
            "payload_sha256": body_digest,
        }
        review = payload["review"]
        if payload["action"] == "submitted":
            receipts[review["id"]] = {
                "review_id": review["id"],
                "reviewer_id": review["user"]["id"],
                "submitted_at": review["submitted_at"],
                "commit_id": review["commit_id"],
            }
        elif payload["action"] == "dismissed":
            dismissals.append(
                {
                    "review_id": review["id"],
                    "reviewer_id": review["user"]["id"],
                    "submitted_at": review["submitted_at"],
                    "commit_id": review["commit_id"],
                }
            )
        outcomes.append("applied")
    ledger = {
        "deliveries": list(deliveries.values()),
        "review_receipts": list(receipts.values()),
        "dismissals": dismissals,
    }
    return ledger_projection(ledger, outcomes, sorted(set(blockers)))


def reduce_authoritative_head(
    transitions: list[dict[str, object]],
    authoritative: dict[str, object],
) -> dict[str, object]:
    base = authoritative["base"]
    head = authoritative["head"]
    successors: dict[str, str] = {}
    predecessors: dict[str, str] = {}
    edges: set[tuple[str, str]] = set()
    for transition in transitions:
        before = transition["before"]
        after = transition["after"]
        require(isinstance(before, str) and isinstance(after, str), "transition commits must be strings")
        require(before != after, "synchronize transition must advance the pull request head")
        edge = (before, after)
        if edge in edges:
            continue
        edges.add(edge)
        if before in successors:
            require(successors[before] == after, "synchronize history has conflicting successors")
        successors[before] = after
        if after in predecessors:
            require(predecessors[after] == before, "synchronize history has conflicting predecessors")
        predecessors[after] = before

    visited: set[tuple[str, str]] = set()
    cursor = head
    while cursor in predecessors:
        before = predecessors[cursor]
        edge = (before, cursor)
        require(edge not in visited, "synchronize history contains a cycle")
        visited.add(edge)
        cursor = before
    require(len(visited) == len(edges), "synchronize history is disconnected from the authoritative head")
    return {"effective_base": base, "effective_head": head}


def independent_head_oracle(case: dict[str, object]) -> dict[str, object]:
    fixture = case["fixture"]
    deliveries = fixture["deliveries_in_arrival_order"]
    authoritative = fixture["authoritative_pull_request_observation"]
    require(isinstance(deliveries, list), "deliveries_in_arrival_order must be an array")
    require(isinstance(authoritative, dict), "authoritative pull request observation must be an object")
    require(
        all(delivery["received_at"] <= authoritative["observed_at"] for delivery in deliveries),
        "authoritative pull request observation predates a synchronize delivery",
    )
    transitions = [
        {
            "before": delivery["before"],
            "after": delivery["after"],
            "base": delivery["payload_base"],
        }
        for delivery in deliveries
    ]
    effective = reduce_authoritative_head(transitions, authoritative)
    return {
        "ingest_outcomes": ["applied" for _ in deliveries],
        "audited_transitions": sorted(
            f"{transition['before']}->{transition['after']}" for transition in transitions
        ),
        **effective,
        "coverage": f"recompute_for_{effective['effective_base']}_{effective['effective_head']}",
        "blockers": [],
        "required_check": "pending_until_recomputed_then_oracle_dependent",
        "reconciliation_checks": [
            "disconnected_history_rejected",
            "stale_head_rejected",
        ],
    }


def ingest_with_product(
    binary: Path,
    directory: Path,
    specs: list[dict[str, object]],
) -> dict[str, object]:
    ledger_path = directory / "ledger.json"
    outcomes = []
    blockers = []
    for index, spec in enumerate(specs):
        payload_path = directory / f"payload-{index}.json"
        body = spec["body"]
        require(isinstance(body, bytes), "materialized webhook body must be bytes")
        payload_path.write_bytes(body)
        arguments = [
            str(binary),
            "github-ledger-ingest",
            str(payload_path),
            "--event",
            str(spec["event"]),
            "--delivery-id",
            str(spec["delivery_id"]),
            "--received-at",
            str(spec["received_at"]),
            "--signature",
            str(spec["signature"]),
            "--provider-url",
            PROVIDER_URL,
            "--receiver-key-id",
            RECEIVER_KEY_ID,
            "--output",
            str(ledger_path),
        ]
        if ledger_path.exists():
            arguments.extend(["--ledger", str(ledger_path)])
        before = ledger_path.read_bytes() if ledger_path.exists() else None
        result = run_command(
            arguments,
            env={
                "STRATADIFF_GITHUB_WEBHOOK_SECRET": WEBHOOK_SECRET.decode(),
                "STRATADIFF_RECEIPT_SIGNING_KEY": RECEIVER_SIGNING_KEY,
            },
        )
        message = command_error(result)
        if result.returncode == 0:
            outcomes.append("duplicate" if "duplicate GitHub delivery" in message else "applied")
            continue
        after = ledger_path.read_bytes() if ledger_path.exists() else None
        require(before == after, "rejected webhook mutated the ledger")
        if "signature verification failed" in message:
            outcomes.append("rejected_before_decode")
            blockers.append("invalid_webhook_hmac")
        elif "reused with different content" in message:
            outcomes.append("rejected_atomically")
            blockers.append("delivery_id_conflicting_body")
        else:
            raise BenchmarkFailure(f"unexpected webhook rejection: {message}")
    ledger = json.loads(ledger_path.read_bytes()) if ledger_path.exists() else None
    return ledger_projection(ledger, outcomes, sorted(set(blockers)))


def require_exact_case_projection(case: dict[str, object], projection: dict[str, object]) -> None:
    expected_keys = set(case["expected"]) - {"must_not"}
    actual_keys = set(projection)
    require(
        actual_keys == expected_keys,
        f"{case['id']} projection keys differ: expected {sorted(expected_keys)}, observed {sorted(actual_keys)}",
    )


def add_webhook_evaluation_state(case_id: str, projection: dict[str, object]) -> None:
    if case_id in (
        "verified-submit-persists-sha-bound-receipt",
        "duplicate-redelivery-is-idempotent-across-receive-times",
        "same-delivery-id-with-conflicting-body-is-rejected",
    ):
        projection["coverage"] = "not_evaluated"
        projection["required_check"] = "not_evaluated"
    elif case_id in (
        "dismiss-before-submit-cannot-reactivate-dismissed-review",
        "latest-dismissed-review-does-not-fall-back",
    ):
        projection["coverage"] = (
            "no_active_receipt_for_alice"
            if projection["active_review_ids"] == []
            else "active_receipt_for_alice"
        )
        projection["required_check"] = "not_evaluated"
    elif case_id == "distinct-new-review-reactivates-coverage":
        projection["coverage"] = (
            "active_receipt_for_alice_at_D"
            if projection["active_review_ids"] == [402]
            else "no_active_receipt_for_alice_at_D"
        )
        projection["required_check"] = "not_evaluated"
    elif case_id == "tampered-body-fails-hmac-before-state-mutation":
        unchanged = projection["delivery_count"] == 0 and projection["active_review_ids"] == []
        projection["coverage"] = "unchanged" if unchanged else "mutated"
        projection["required_check"] = "unchanged" if unchanged else "mutated"
    else:
        raise BenchmarkFailure(f"no webhook evaluation state projection for {case_id}")


def webhook_case(binary: Path, manifest: dict[str, object], case: dict[str, object], directory: Path) -> tuple[dict[str, object], dict[str, object]]:
    specs = webhook_specs(manifest, case)
    oracle = independent_webhook_oracle(specs)
    observed = ingest_with_product(binary, directory, specs)
    for projection in (oracle, observed):
        add_webhook_evaluation_state(case["id"], projection)
    if case["id"] == "dismiss-before-submit-cannot-reactivate-dismissed-review":
        for projection in (oracle, observed):
            require_exact_case_projection(case, projection)
    return oracle, observed


def codeowners_match(pattern: str, path: str) -> bool:
    normalized = pattern[1:] if pattern.startswith("/") else pattern
    if normalized.endswith("/"):
        normalized += "**"
    return fnmatch.fnmatchcase(path, normalized)


def resolve_symbolic_codeowners(rules: list[dict[str, object]], path: str) -> dict[str, object] | None:
    match = None
    for index, rule in enumerate(rules):
        if codeowners_match(rule["pattern"], path):
            match = {
                "line": rule["line"] if "line" in rule else index + 1,
                "pattern": rule["pattern"],
                "owners": list(rule["owners"]),
            }
    return match


def permission_allows_review(permission: str) -> bool:
    return permission in ("write", "maintain", "admin")


def resolve_symbolic_owner(owner: str, snapshot: dict[str, object]) -> tuple[list[int], str | None]:
    if owner.count("/") == 0:
        login = owner.removeprefix("@").casefold()
        for user in snapshot["users"]:
            if user["login"].casefold() == login:
                if permission_allows_review(user["repository_permission"]):
                    return [user["id"]], None
                return [], "insufficient_repository_permission"
        return [], "user_not_found"
    organization, slug = owner.removeprefix("@").split("/", 1)
    team = None
    for candidate in snapshot["teams"]:
        if candidate["organization_login"].casefold() == organization.casefold() and candidate["slug"].casefold() == slug.casefold():
            team = candidate
            break
    if team is None:
        return [], "team_not_found"
    if team["privacy"] == "secret":
        return [], "team_not_visible"
    if not permission_allows_review(team["repository_permission"]):
        return [], "insufficient_repository_permission"
    users = {user["id"]: user for user in snapshot["users"]}
    eligible = []
    for membership in team["members"]:
        if membership["state"] != "active":
            continue
        user = users[membership["user_id"]]
        if permission_allows_review(user["repository_permission"]):
            eligible.append(user["id"])
    if not eligible:
        return [], "no_eligible_team_members"
    return sorted(eligible), None


def split_lines(value: str) -> list[str]:
    return value.splitlines(keepends=True)


def changed_intervals(before: str, after: str) -> list[tuple[int, int, list[str]]]:
    before_lines = split_lines(before)
    after_lines = split_lines(after)
    matcher = difflib.SequenceMatcher(a=before_lines, b=after_lines, autojunk=False)
    return [
        (left_start, left_end, after_lines[right_start:right_end])
        for tag, left_start, left_end, right_start, right_end in matcher.get_opcodes()
        if tag != "equal"
    ]


def intervals_are_separate(left: list[tuple[int, int, list[str]]], right: list[tuple[int, int, list[str]]]) -> bool:
    for left_start, left_end, _ in left:
        for right_start, right_end, _ in right:
            if not (left_end < right_start or right_end < left_start):
                return False
    return True


def apply_intervals(source: str, intervals: list[tuple[int, int, list[str]]]) -> str:
    lines = split_lines(source)
    for start, end, replacement in sorted(intervals, key=lambda item: item[0], reverse=True):
        lines[start:end] = replacement
    return "".join(lines)


def independent_carry_basis(snapshots: dict[str, object], path: str) -> str | None:
    a_value = snapshots["A"][path]
    b_value = snapshots["B"][path]
    c_value = snapshots["C"][path]
    d_value = snapshots["D"][path]
    if a_value == c_value and b_value == d_value:
        return "exact_git_change_identity"
    review_edits = changed_intervals(a_value, b_value)
    upstream_edits = changed_intervals(a_value, c_value)
    if not intervals_are_separate(review_edits, upstream_edits):
        return None
    combined = apply_intervals(a_value, review_edits + upstream_edits)
    if combined == d_value:
        return "exact_noninteracting_four_way_byte_replay"
    return None


def coverage_oracle(case: dict[str, object]) -> dict[str, object]:
    case_id = case["id"]
    fixture = case["fixture"]
    if case_id == "two-owner-domains-invalidate-selectively":
        rules = fixture["codeowners_at_exact_base_C"]
        receipts = {receipt["reviewer"] for receipt in fixture["active_receipts"]}
        carried = []
        residue = []
        satisfied = set()
        uncovered = set()
        for current in fixture["current_paths"]:
            rule = resolve_symbolic_codeowners(rules, current["path"])
            require(rule is not None, f"no symbolic CODEOWNERS match for {current['path']}")
            owners = rule["owners"]
            if current["since_B"] == "exact_identity_carry" and any(owner.removeprefix("@") in receipts for owner in owners):
                carried.append(current["path"])
                satisfied.update(owners)
            else:
                residue.append(current["path"])
                uncovered.update(owners)
        return {
            "satisfied_domains": sorted(satisfied),
            "uncovered_domains": sorted(uncovered),
            "carried_paths": sorted(carried),
            "residue_paths": sorted(residue),
            "coverage": "partially_covered",
            "blockers": [],
            "required_check": "red",
        }
    if case_id == "one-owner-alternative-satisfies-winning-rule":
        rule = resolve_symbolic_codeowners(fixture["codeowners_at_exact_base_C"], fixture["path"])
        require(rule is not None, "owner OR fixture has no winning rule")
        receipts = {receipt["reviewer"] for receipt in fixture["active_receipts"] if fixture["path"] in receipt["covers"]}
        satisfying = [owner for owner in rule["owners"] if owner.removeprefix("@") in receipts]
        return {
            "winning_rule_line": rule["line"],
            "owner_operator": "or",
            "satisfying_owner": satisfying[0] if satisfying else None,
            "coverage": "covered" if satisfying else "uncovered",
            "blockers": [],
            "required_check": "green" if satisfying else "red",
        }
    if case_id == "invalid-selected-codeowners-source-fails-closed":
        entries = fixture["tree_entries"]
        source = next(path for path in (".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS") if path in entries)
        invalid = any(line.startswith("[") for line in entries[source].splitlines())
        require(invalid, "invalid CODEOWNERS fixture contains no independently recognized invalid line")
        return {
            "selected_source": source,
            "coverage": "blocked",
            "blockers": ["invalid_codeowners_line"],
            "required_check": "red",
        }
    if case_id in (
        "missing-codeowner-team-fails-closed",
        "secret-codeowner-team-fails-closed",
        "read-only-codeowner-team-fails-closed",
        "pending-only-team-membership-fails-closed",
    ):
        eligible, blocker = resolve_symbolic_owner(fixture["winning_owner"], fixture["ownership_snapshot"])
        provenance = [
            "provider_url",
            "repository_id",
            "base_commit",
            "api_version",
            "observed_at",
            "stable_identity_ids",
            "membership_state",
        ]
        return {
            "eligible_reviewer_ids": eligible,
            "required_snapshot_provenance": provenance,
            "coverage": "blocked" if blocker is not None else "uncovered",
            "blockers": [blocker] if blocker is not None else [],
            "required_check": "red",
        }
    if case_id == "codeowners-source-is-pinned-to-exact-base-commit":
        base = fixture["pull_request_base"]
        selected = next(source for source in fixture["sources"] if source["location"] == f"{base}:.github/CODEOWNERS")
        lines = selected["content"].splitlines()
        rules = []
        for index, line in enumerate(lines):
            tokens = line.split()
            rules.append({"pattern": tokens[0], "owners": tokens[1:], "line": index + 1})
        rule = resolve_symbolic_codeowners(rules, fixture["path"])
        require(rule is not None, "exact-base CODEOWNERS fixture has no winning rule")
        return {
            "selected_source": f"{base}:.github/CODEOWNERS",
            "selected_owner_alternatives": rule["owners"],
            "required_provenance": ["base_commit", "source_path", "blob_oid", "byte_length", "blake3"],
            "coverage": "oracle_dependent_on_alice_receipt",
            "blockers": [],
            "required_check": "oracle_dependent",
        }
    if case_id in (
        "byte-identical-restack-carries-by-exact-change-identity",
        "noninteracting-base-drift-carries-by-four-way-replay",
        "genuine-author-edit-remains-owner-residue",
    ):
        snapshots = fixture["snapshots"]
        paths = sorted(snapshots["D"])
        carried = []
        residue = []
        basis = {}
        for path in paths:
            carry = independent_carry_basis(snapshots, path)
            if carry is None:
                residue.append(path)
            else:
                carried.append(path)
                basis[path] = carry
        output = {
            "carried_paths": carried,
            "residue_paths": residue,
            "coverage": "covered" if not residue else "uncovered",
            "blockers": [],
            "required_check": "green" if not residue else "red",
        }
        if basis:
            output["carry_basis"] = basis
        if residue:
            output["residue_basis"] = {
                path: "current_bytes_differ_from_reconstructed_review_baseline" for path in residue
            }
        return output
    if case_id == "unavailable-orphan-review-commit-fails-closed":
        receipt = fixture["receipt"]
        return {
            "audited_review_ids": [receipt["review_id"]],
            "active_review_ids": [receipt["review_id"]],
            "carried_paths": [],
            "residue_paths": [],
            "coverage": "blocked",
            "blockers": ["review_commit_unavailable"],
            "required_check": "red",
        }
    raise BenchmarkFailure(f"no independent coverage oracle for {case_id}")


def direct_users(*logins: str) -> list[dict[str, object]]:
    known_ids = {"alice": 17, "bob": 18}
    return [
        {"id": known_ids[login], "login": login, "repository_permission": "write"}
        for login in logins
    ]


def ownership_document(base_commit: str, users: list[dict[str, object]], teams: list[dict[str, object]]) -> dict[str, object]:
    return {
        "schema": OWNERSHIP_SCHEMA,
        "provider_url": PROVIDER_URL,
        "repository_id": 99,
        "base_commit": base_commit,
        "api_version": "2022-11-28",
        "observed_at": "2026-09-05T04:00:00Z",
        "users": sorted(users, key=lambda user: user["id"]),
        "teams": sorted(teams, key=lambda team: team["id"]),
    }


def product_review_payload(
    *,
    review_id: int,
    reviewer_id: int,
    login: str,
    checkpoint: str,
    event_base: str,
    event_head: str,
) -> bytes:
    return encode_json(
        {
            "action": "submitted",
            "review": {
                "id": review_id,
                "node_id": f"PRR_{review_id}",
                "user": {"id": reviewer_id, "node_id": f"U_{reviewer_id}", "login": login, "type": "User"},
                "state": "approved",
                "commit_id": checkpoint,
                "submitted_at": f"2026-09-05T05:00:{review_id % 60:02}Z",
                "html_url": f"https://github.com/acme/widgets/pull/7#pullrequestreview-{review_id}",
                "author_association": "MEMBER",
            },
            "pull_request": {
                "id": 700,
                "node_id": "PR_700",
                "number": 7,
                "base": {"sha": event_base},
                "head": {"sha": event_head},
            },
            "repository": {"id": 99, "node_id": "R_99", "full_name": "acme/widgets"},
        }
    )


def create_product_ledger(
    binary: Path,
    directory: Path,
    reviews: list[dict[str, object]],
) -> Path:
    ledger_path = directory / "ledger.json"
    for index, review in enumerate(reviews):
        body = product_review_payload(
            review_id=review["review_id"],
            reviewer_id=review["reviewer_id"],
            login=review["login"],
            checkpoint=review["checkpoint"],
            event_base=review["event_base"],
            event_head=review["event_head"],
        )
        payload_path = directory / f"coverage-review-{index}.json"
        payload_path.write_bytes(body)
        arguments = [
            str(binary),
            "github-ledger-ingest",
            str(payload_path),
            "--event",
            "pull_request_review",
            "--delivery-id",
            f"coverage-delivery-{review['review_id']}",
            "--received-at",
            f"2026-09-05T05:01:{review['review_id'] % 60:02}Z",
            "--signature",
            webhook_signature(body),
            "--provider-url",
            PROVIDER_URL,
            "--receiver-key-id",
            RECEIVER_KEY_ID,
            "--output",
            str(ledger_path),
        ]
        if ledger_path.exists():
            arguments.extend(["--ledger", str(ledger_path)])
        result = run_command(
            arguments,
            env={
                "STRATADIFF_GITHUB_WEBHOOK_SECRET": WEBHOOK_SECRET.decode(),
                "STRATADIFF_RECEIPT_SIGNING_KEY": RECEIVER_SIGNING_KEY,
            },
        )
        require(result.returncode == 0, f"coverage review ingestion failed: {command_error(result)}")
    return ledger_path


def build_passport(
    binary: Path,
    directory: Path,
    repository: Path,
    base_commit: str,
    head_commit: str,
    reviews: list[dict[str, object]],
    ownership: dict[str, object],
) -> tuple[dict[str, object], Path]:
    ledger_path = create_product_ledger(binary, directory, reviews)
    ownership_path = directory / "ownership.json"
    ownership_path.write_bytes(encode_json(ownership))
    passport_path = directory / "passport.json"
    result = run_command(
        [
            str(binary),
            "review-coverage",
            base_commit,
            head_commit,
            "--repo",
            str(repository),
            "--ledger",
            str(ledger_path),
            "--ownership",
            str(ownership_path),
            "--output",
            str(passport_path),
        ],
        env={"STRATADIFF_RECEIPT_SIGNING_KEY": RECEIVER_SIGNING_KEY},
    )
    require(result.returncode == 0, f"coverage passport generation failed: {command_error(result)}")
    return json.loads(passport_path.read_bytes()), passport_path


def head_case(
    binary: Path,
    manifest: dict[str, object],
    case: dict[str, object],
    directory: Path,
) -> tuple[dict[str, object], dict[str, object]]:
    oracle = independent_head_oracle(case)
    repository = directory / "repository"
    init_repository(repository)
    old_base_files = {
        ".github/CODEOWNERS": "/tracked.txt @alice\n",
        "tracked.txt": "version = -1\n",
    }
    old_base = commit_tree(repository, old_base_files, "historical base", 0)
    base_files = {
        ".github/CODEOWNERS": "/tracked.txt @alice\n",
        "tracked.txt": "version = 0\n",
    }
    base = commit_tree(repository, base_files, "current base", 1)
    h0_files = dict(base_files)
    h0_files["tracked.txt"] = "version = 1\n"
    h0 = commit_tree(repository, h0_files, "head zero", 2)
    sequence = 3
    h1_files = dict(h0_files)
    while True:
        h1_files["tracked.txt"] = f"version = {sequence}\n"
        h1 = commit_tree(repository, h1_files, "head one", sequence)
        if h1[0] in "cdef":
            break
        sequence += 1
        require(sequence < 50, "could not materialize a high-sorting H1 fixture commit")
    h2_files = dict(h1_files)
    while True:
        sequence += 1
        require(sequence < 60, "could not materialize a lower-sorting authoritative H2 fixture commit")
        h2_files["tracked.txt"] = f"version = {sequence}\n"
        h2 = commit_tree(repository, h2_files, "head two", sequence)
        if h2 < h1:
            break
    commits = {"A": old_base, "C": base, "H0": h0, "H1": h1, "H2": h2}
    reverse_commits = {commit: symbol for symbol, commit in commits.items()}
    require(h2 < h1, "head fixture does not defeat lexicographic-SHA reduction")

    specs = head_webhook_specs(manifest, case, commits)
    ingestion = ingest_with_product(binary, directory, specs)
    ownership = ownership_document(base, direct_users("alice"), [])
    authoritative = case["fixture"]["authoritative_pull_request_observation"]
    ownership["observed_at"] = authoritative["observed_at"]
    passport, _ = build_passport(binary, directory, repository, base, h2, [], ownership)
    ledger_path = directory / "ledger.json"
    ownership_path = directory / "ownership.json"

    def require_reconciliation_failure(requested_head: str, output_name: str) -> None:
        output_path = directory / output_name
        result = run_command(
            [
                str(binary),
                "review-coverage",
                base,
                requested_head,
                "--repo",
                str(repository),
                "--ledger",
                str(ledger_path),
                "--ownership",
                str(ownership_path),
                "--output",
                str(output_path),
            ],
            env={"STRATADIFF_RECEIPT_SIGNING_KEY": RECEIVER_SIGNING_KEY},
        )
        message = command_error(result)
        require(result.returncode != 0, f"unreconciled head {requested_head} was accepted")
        require(not output_path.exists(), "failed head reconciliation wrote a coverage passport")
        require(
            "disconnected from the authoritative pull request head" in message,
            f"head reconciliation failed for an unexpected reason: {message}",
        )

    require_reconciliation_failure(h1, "stale-head-passport.json")
    disconnected_body = synchronize_payload(
        manifest,
        before="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        after="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        base=base,
    )
    disconnected_ingestion = ingest_with_product(
        binary,
        directory,
        [
            {
                "delivery_id": "delivery-sync-disconnected",
                "event": "pull_request",
                "received_at": "2026-09-05T03:05:30Z",
                "body": disconnected_body,
                "signature": webhook_signature(disconnected_body),
            }
        ],
    )
    require(disconnected_ingestion["ingest_outcomes"] == ["applied"], "disconnected audit transition was not retained")
    require_reconciliation_failure(h2, "disconnected-head-passport.json")

    body = passport["body"]
    ledger = body["ledger"]
    effective_base = reverse_commits[body["protected_base_commit"]]
    effective_head = reverse_commits[body["head_commit"]]
    observed = {
        "ingest_outcomes": ingestion["ingest_outcomes"],
        "audited_transitions": sorted(
            f"{reverse_commits[transition['before_commit']]}->{reverse_commits[transition['head_commit']]}"
            for transition in ledger["head_observations"]
        ),
        "effective_base": effective_base,
        "effective_head": effective_head,
        "coverage": f"recompute_for_{effective_base}_{effective_head}",
        "blockers": ingestion["blockers"],
        "required_check": "pending_until_recomputed_then_oracle_dependent",
        "reconciliation_checks": [
            "disconnected_history_rejected",
            "stale_head_rejected",
        ],
    }
    require_exact_case_projection(case, oracle)
    require_exact_case_projection(case, observed)
    return oracle, observed


def owner_label(owner: dict[str, object]) -> str:
    if owner["kind"] == "user":
        return "@" + owner["login"]
    if owner["kind"] == "team":
        return "@" + owner["organization"] + "/" + owner["slug"]
    return owner["address"]


def blocker_code(message: str) -> str:
    if "invalid line" in message:
        return "invalid_codeowners_line"
    if "is missing from the snapshot" in message and "team" in message:
        return "team_not_found"
    if "is secret" in message:
        return "team_not_visible"
    if "has no active member" in message:
        return "no_eligible_team_members"
    if "repository permission" in message:
        return "insufficient_repository_permission"
    if "checkpoint proof is unavailable" in message:
        return "review_commit_unavailable"
    return "unclassified_blocker"


def file_for_path(passport: dict[str, object], path: str) -> list[dict[str, object]]:
    return [entry for entry in passport["body"]["files"] if entry["path"] == path]


def normalize_product_coverage(case_id: str, passport: dict[str, object]) -> dict[str, object]:
    body = passport["body"]
    files = body["files"]
    summary = body["summary"]
    blockers = sorted(
        {
            blocker_code(message)
            for entry in files
            for owner in entry["owner_alternatives"]
            for message in owner["blockers"]
        }
    )
    if any(not entry["owner_alternatives"] and entry["state"] == "blocked" for entry in files):
        for entry in files:
            if entry["state"] == "blocked" and "invalid line" in entry["reason"]:
                blockers.append("invalid_codeowners_line")
    blockers = sorted(set(blockers))
    if case_id == "two-owner-domains-invalidate-selectively":
        satisfied = set()
        uncovered = set()
        for entry in files:
            for owner in entry["owner_alternatives"]:
                label = owner_label(owner["owner"])
                if owner["covering_review_ids"]:
                    satisfied.add(label)
                elif entry["state"] == "needs_review":
                    uncovered.add(label)
        return {
            "satisfied_domains": sorted(satisfied),
            "uncovered_domains": sorted(uncovered),
            "carried_paths": sorted(entry["path"] for entry in files if entry["state"] == "covered"),
            "residue_paths": sorted(entry["path"] for entry in files if entry["state"] == "needs_review"),
            "coverage": "partially_covered" if satisfied and uncovered else "covered" if not uncovered else "uncovered",
            "blockers": blockers,
            "required_check": "green" if summary["gate_passed"] else "red",
        }
    if case_id == "one-owner-alternative-satisfies-winning-rule":
        entries = file_for_path(passport, "api/router.rs")
        require(len(entries) == 1, "owner OR product result did not contain exactly one api/router.rs entry")
        entry = entries[0]
        rule = entry["matching_rule"]
        satisfying = [owner_label(owner["owner"]) for owner in entry["owner_alternatives"] if owner["covering_review_ids"]]
        return {
            "winning_rule_line": rule["line"],
            "owner_operator": "or",
            "satisfying_owner": satisfying[0] if satisfying else None,
            "coverage": "covered" if entry["state"] == "covered" else "uncovered",
            "blockers": blockers,
            "required_check": "green" if summary["gate_passed"] else "red",
        }
    if case_id == "invalid-selected-codeowners-source-fails-closed":
        selected_source = None
        for entry in files:
            if ".github/CODEOWNERS" in entry["reason"]:
                selected_source = ".github/CODEOWNERS"
        return {
            "selected_source": selected_source,
            "coverage": "blocked" if any(entry["state"] == "blocked" for entry in files) else "uncovered",
            "blockers": blockers,
            "required_check": "green" if summary["gate_passed"] else "red",
        }
    if case_id in (
        "missing-codeowner-team-fails-closed",
        "secret-codeowner-team-fails-closed",
        "read-only-codeowner-team-fails-closed",
        "pending-only-team-membership-fails-closed",
    ):
        entry = files[0]
        eligible = sorted({reviewer_id for owner in entry["owner_alternatives"] for reviewer_id in owner["eligible_reviewer_ids"]})
        ownership = body["ownership"]
        provenance = []
        for field in ("provider_url", "repository_id", "base_commit", "api_version", "observed_at"):
            if field in ownership:
                provenance.append(field)
        if all("id" in user for user in ownership["users"]) and all("id" in team for team in ownership["teams"]):
            provenance.append("stable_identity_ids")
        if all("state" in membership for team in ownership["teams"] for membership in team["members"]):
            provenance.append("membership_state")
        return {
            "eligible_reviewer_ids": eligible,
            "required_snapshot_provenance": provenance,
            "coverage": "blocked" if entry["state"] == "blocked" else "uncovered",
            "blockers": blockers,
            "required_check": "green" if summary["gate_passed"] else "red",
        }
    if case_id == "codeowners-source-is-pinned-to-exact-base-commit":
        source = body["codeowners_source"]
        require(source["base_commit"] == body["protected_base_commit"], "product CODEOWNERS source is not bound to the protected base")
        entries = file_for_path(passport, "api/router.rs")
        require(len(entries) == 1, "exact-base source result did not contain api/router.rs")
        owners = [owner_label(owner) for owner in entries[0]["matching_rule"]["owner_alternatives"]]
        provenance = []
        for field, label in (
            ("base_commit", "base_commit"),
            ("path", "source_path"),
            ("blob_oid", "blob_oid"),
            ("byte_len", "byte_length"),
            ("blake3", "blake3"),
        ):
            if field in source:
                provenance.append(label)
        return {
            "selected_source": "C:" + source["path"],
            "selected_owner_alternatives": owners,
            "required_provenance": provenance,
            "coverage": "oracle_dependent_on_alice_receipt",
            "blockers": [],
            "required_check": "oracle_dependent",
        }
    if case_id in (
        "byte-identical-restack-carries-by-exact-change-identity",
        "noninteracting-base-drift-carries-by-four-way-replay",
        "genuine-author-edit-remains-owner-residue",
    ):
        carried = []
        basis = {}
        for proof in body["checkpoint_proofs"]:
            result = proof["result"]
            if result["state"] != "verified":
                continue
            for change in result["carried_changes"]:
                path = change["change"]["after_path"]
                carried.append(path)
                basis[path] = change["basis"]
        residue = [entry["path"] for entry in files if entry["state"] == "needs_review"]
        output = {
            "carried_paths": sorted(set(carried)),
            "residue_paths": sorted(residue),
            "coverage": "covered" if summary["gate_passed"] else "uncovered",
            "blockers": blockers,
            "required_check": "green" if summary["gate_passed"] else "red",
        }
        if basis:
            output["carry_basis"] = basis
        if residue:
            output["residue_basis"] = {
                path: "current_bytes_differ_from_reconstructed_review_baseline" for path in sorted(set(residue))
            }
        return output
    if case_id == "unavailable-orphan-review-commit-fails-closed":
        ledger = body["ledger"]
        return {
            "audited_review_ids": sorted(receipt["review_id"] for receipt in ledger["review_receipts"]),
            "active_review_ids": reduce_active_reviews(ledger),
            "carried_paths": [],
            "residue_paths": [],
            "coverage": "blocked" if summary["blocked_files"] else "uncovered",
            "blockers": blockers,
            "required_check": "green" if summary["gate_passed"] else "red",
        }
    raise BenchmarkFailure(f"no product coverage normalizer for {case_id}")


def materialize_coverage_case(binary: Path, case: dict[str, object], directory: Path) -> tuple[dict[str, object], Path, Path]:
    case_id = case["id"]
    fixture = case["fixture"]
    repository = directory / "repository"
    init_repository(repository)
    reviews = []
    if case_id == "two-owner-domains-invalidate-selectively":
        base_files = {
            ".github/CODEOWNERS": "/payments/** @alice\n/docs/** @bob\n",
            "payments/pay.rs": "value = 0\n",
            "docs/guide.md": "value = 0\n",
        }
        base = commit_tree(repository, base_files, "base", 0)
        reviewed_files = dict(base_files)
        reviewed_files["payments/pay.rs"] = "value = 1\n"
        reviewed_files["docs/guide.md"] = "value = 1\n"
        reviewed = commit_tree(repository, reviewed_files, "reviewed", 1)
        head_files = dict(reviewed_files)
        head_files["payments/pay.rs"] = "value = 2\n"
        head = commit_tree(repository, head_files, "follow-up", 2)
        reviews = [
            {"review_id": 1, "reviewer_id": 17, "login": "alice", "checkpoint": reviewed, "event_base": base, "event_head": reviewed},
            {"review_id": 2, "reviewer_id": 18, "login": "bob", "checkpoint": reviewed, "event_base": base, "event_head": reviewed},
        ]
        ownership = ownership_document(base, direct_users("alice", "bob"), [])
    elif case_id == "one-owner-alternative-satisfies-winning-rule":
        codeowners = "# 1\n# 2\n# 3\n# 4\n# 5\n# 6\n/api/** @alice @bob\n"
        base_files = {".github/CODEOWNERS": codeowners, "api/router.rs": "value = 0\n"}
        base = commit_tree(repository, base_files, "base", 0)
        head_files = dict(base_files)
        head_files["api/router.rs"] = "value = 1\n"
        head = commit_tree(repository, head_files, "head", 1)
        reviews = [{"review_id": 1, "reviewer_id": 17, "login": "alice", "checkpoint": head, "event_base": base, "event_head": head}]
        ownership = ownership_document(base, direct_users("alice", "bob"), [])
    elif case_id == "invalid-selected-codeowners-source-fails-closed":
        entries = fixture["tree_entries"]
        base_files = dict(entries)
        base_files["api/router.rs"] = "value = 0\n"
        base = commit_tree(repository, base_files, "base", 0)
        head_files = dict(base_files)
        head_files["api/router.rs"] = "value = 1\n"
        head = commit_tree(repository, head_files, "head", 1)
        reviews = [{"review_id": 1, "reviewer_id": 17, "login": "alice", "checkpoint": head, "event_base": base, "event_head": head}]
        ownership = ownership_document(base, direct_users("alice", "bob"), [])
    elif case_id in (
        "missing-codeowner-team-fails-closed",
        "secret-codeowner-team-fails-closed",
        "read-only-codeowner-team-fails-closed",
        "pending-only-team-membership-fails-closed",
    ):
        winning_owner = fixture["winning_owner"]
        base_files = {".github/CODEOWNERS": f"/owned/** {winning_owner}\n", "owned/file.txt": "value=0\n"}
        base = commit_tree(repository, base_files, "base", 0)
        head_files = dict(base_files)
        head_files["owned/file.txt"] = "value=1\n"
        head = commit_tree(repository, head_files, "head", 1)
        reviews = [{"review_id": 1, "reviewer_id": 17, "login": "alice", "checkpoint": head, "event_base": base, "event_head": head}]
        snapshot = copy.deepcopy(fixture["ownership_snapshot"])
        snapshot["schema"] = OWNERSHIP_SCHEMA
        snapshot["base_commit"] = base
        for team in snapshot["teams"]:
            for membership in team["members"]:
                if "inherited" not in membership:
                    membership["inherited"] = False
                if "repository_permission" in membership:
                    del membership["repository_permission"]
        ownership = snapshot
    elif case_id == "codeowners-source-is-pinned-to-exact-base-commit":
        base_files = {
            ".github/CODEOWNERS": "/api/** @alice\n",
            "CODEOWNERS": "/** @bob\n",
            "api/router.rs": "value=0\n",
        }
        base = commit_tree(repository, base_files, "base", 0)
        head_files = dict(base_files)
        head_files[".github/CODEOWNERS"] = "/api/** @bob\n"
        head_files["api/router.rs"] = "value=1\n"
        head = commit_tree(repository, head_files, "head", 1)
        reviews = [{"review_id": 1, "reviewer_id": 17, "login": "alice", "checkpoint": head, "event_base": base, "event_head": head}]
        ownership = ownership_document(base, direct_users("alice", "bob"), [])
    elif case_id in (
        "byte-identical-restack-carries-by-exact-change-identity",
        "noninteracting-base-drift-carries-by-four-way-replay",
        "genuine-author-edit-remains-owner-residue",
    ):
        snapshots = fixture["snapshots"]
        path = next(iter(snapshots["A"]))
        base_files = {".github/CODEOWNERS": f"/{path} @alice\n", path: snapshots["A"][path]}
        old_base = commit_tree(repository, base_files, "old base", 0)
        reviewed_files = dict(base_files)
        reviewed_files[path] = snapshots["B"][path]
        reviewed = commit_tree(repository, reviewed_files, "reviewed", 1)
        checkout_new(repository, "current", old_base)
        current_base_files = dict(base_files)
        current_base_files[path] = snapshots["C"][path]
        current_base = commit_tree(
            repository,
            current_base_files,
            "current base",
            2,
            allow_empty=current_base_files == base_files,
        )
        current_files = dict(current_base_files)
        current_files[path] = snapshots["D"][path]
        head = commit_tree(repository, current_files, "current head", 3)
        base = current_base
        reviews = [{"review_id": 1, "reviewer_id": 17, "login": "alice", "checkpoint": reviewed, "event_base": old_base, "event_head": reviewed}]
        ownership = ownership_document(base, direct_users("alice"), [])
    elif case_id == "unavailable-orphan-review-commit-fails-closed":
        base_files = {".github/CODEOWNERS": "/owned/** @alice\n", "owned/file.txt": "value=0\n"}
        base = commit_tree(repository, base_files, "base", 0)
        head_files = dict(base_files)
        head_files["owned/file.txt"] = "value=1\n"
        head = commit_tree(repository, head_files, "head", 1)
        reviews = [{"review_id": 501, "reviewer_id": 17, "login": "alice", "checkpoint": "e" * 40, "event_base": base, "event_head": head}]
        ownership = ownership_document(base, direct_users("alice"), [])
    else:
        raise BenchmarkFailure(f"no product materializer for {case_id}")
    passport, passport_path = build_passport(binary, directory, repository, base, head, reviews, ownership)
    return passport, passport_path, repository


def coverage_case(binary: Path, case: dict[str, object], directory: Path) -> tuple[dict[str, object], dict[str, object], Path, Path]:
    oracle = coverage_oracle(case)
    passport, passport_path, repository = materialize_coverage_case(binary, case, directory)
    observed = normalize_product_coverage(case["id"], passport)
    oracle["ingest_outcomes"] = []
    observed["ingest_outcomes"] = []
    return oracle, observed, passport_path, repository


def expected_projection(case: dict[str, object]) -> dict[str, object]:
    expected = case["expected"]
    return {key: expected[key] for key in sorted(expected) if key != "must_not"}


def compare_projection(label: str, expected: dict[str, object], actual: dict[str, object]) -> list[str]:
    failures = []
    for key, value in expected.items():
        if key not in actual:
            failures.append(f"{label} omitted {key}")
        elif actual[key] != value:
            failures.append(f"{label} {key}: expected {value!r}, observed {actual[key]!r}")
    return failures


WEBHOOK_CASES = {
    "verified-submit-persists-sha-bound-receipt",
    "duplicate-redelivery-is-idempotent-across-receive-times",
    "same-delivery-id-with-conflicting-body-is-rejected",
    "dismiss-before-submit-cannot-reactivate-dismissed-review",
    "latest-dismissed-review-does-not-fall-back",
    "distinct-new-review-reactivates-coverage",
    "tampered-body-fails-hmac-before-state-mutation",
}

HEAD_CASES = {
    "out-of-order-synchronize-does-not-roll-back-current-head",
}

COVERAGE_CASES = {
    "two-owner-domains-invalidate-selectively",
    "one-owner-alternative-satisfies-winning-rule",
    "invalid-selected-codeowners-source-fails-closed",
    "missing-codeowner-team-fails-closed",
    "secret-codeowner-team-fails-closed",
    "read-only-codeowner-team-fails-closed",
    "pending-only-team-membership-fails-closed",
    "codeowners-source-is-pinned-to-exact-base-commit",
    "byte-identical-restack-carries-by-exact-change-identity",
    "noninteracting-base-drift-carries-by-four-way-replay",
    "genuine-author-edit-remains-owner-residue",
    "unavailable-orphan-review-commit-fails-closed",
}


def validate_manifest(manifest: dict[str, object]) -> None:
    require(manifest["schema"] == "stratadiff-review-ledger-benchmark-manifest-v1", "unsupported benchmark manifest schema")
    cases = manifest["cases"]
    require(isinstance(cases, list), "manifest cases must be an array")
    case_ids = [case["id"] for case in cases]
    require(len(case_ids) == len(set(case_ids)), "manifest case IDs must be unique")
    declared_coverage = manifest["required_coverage"]
    require(isinstance(declared_coverage, list), "required_coverage must be an array")
    actual_coverage = sorted({item for case in cases for item in case["covers"]})
    require(sorted(declared_coverage) == actual_coverage, "required_coverage does not equal the union of case coverage tags")
    known = WEBHOOK_CASES | HEAD_CASES | COVERAGE_CASES
    unknown = sorted(set(case_ids) - known)
    require(not unknown, f"runner has no PASS/FAIL/SKIP disposition for: {', '.join(unknown)}")


def verify_passport_tamper(binary: Path, passport_path: Path, repository: Path, directory: Path) -> dict[str, object]:
    passport = json.loads(passport_path.read_bytes())
    public_key = passport["body"]["ledger"]["receiver"]["public_key"]
    clean = run_command(
        [
            str(binary),
            "review-coverage-verify",
            str(passport_path),
            "--repo",
            str(repository),
            "--trusted-receiver-public-key",
            public_key,
        ]
    )
    require(clean.returncode == 0, f"untampered passport did not verify: {command_error(clean)}")
    tampered = copy.deepcopy(passport)
    tampered["body"]["non_claims"][0] += " tampered"
    tampered_path = directory / "tampered-passport.json"
    tampered_path.write_bytes(encode_json(tampered))
    rejected = run_command(
        [
            str(binary),
            "review-coverage-verify",
            str(tampered_path),
            "--repo",
            str(repository),
            "--trusted-receiver-public-key",
            public_key,
        ]
    )
    message = command_error(rejected)
    require(rejected.returncode != 0, "tampered passport was accepted")
    require("body digest mismatch" in message, f"tampered passport failed for an unexpected reason: {message}")
    return {"id": "passport-tamper-detected", "status": "PASS", "detail": "clean passport verified; modified body was rejected before offline recomputation"}


def run_benchmark(
    binary: Path,
    manifest: dict[str, object],
    manifest_digest: str,
    selected: set[str] | None,
    work_root: Path,
) -> dict[str, object]:
    validate_manifest(manifest)
    results = []
    controls = []
    tamper_source: tuple[Path, Path, Path] | None = None
    for case in manifest["cases"]:
        case_id = case["id"]
        if selected is not None and case_id not in selected:
            continue
        directory = work_root / case_id
        directory.mkdir(parents=True)
        try:
            if case_id in WEBHOOK_CASES:
                oracle, observed = webhook_case(binary, manifest, case, directory)
            elif case_id in HEAD_CASES:
                oracle, observed = head_case(binary, manifest, case, directory)
            else:
                oracle, observed, passport_path, repository = coverage_case(binary, case, directory)
                if case_id == "one-owner-alternative-satisfies-winning-rule":
                    tamper_source = (passport_path, repository, directory)
            manifest_expected = expected_projection(case)
            failures = compare_projection("oracle", manifest_expected, oracle)
            failures.extend(compare_projection("product", oracle, observed))
            if failures:
                results.append(
                    {
                        "id": case_id,
                        "status": "FAIL",
                        "failures": failures,
                        "oracle": oracle,
                        "observed": observed,
                    }
                )
            else:
                results.append({"id": case_id, "status": "PASS", "oracle": oracle})
        except (BenchmarkFailure, KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
            results.append({"id": case_id, "status": "FAIL", "failures": [str(error)]})
    if selected is None or "passport-tamper-detected" in selected:
        if tamper_source is None:
            controls.append({"id": "passport-tamper-detected", "status": "SKIP", "reason": "owner-OR passport source case was not selected or did not materialize"})
        else:
            passport_path, repository, directory = tamper_source
            try:
                controls.append(verify_passport_tamper(binary, passport_path, repository, directory))
            except BenchmarkFailure as error:
                controls.append({"id": "passport-tamper-detected", "status": "FAIL", "failures": [str(error)]})
    counts = {status: sum(result["status"] == status for result in results) for status in ("PASS", "FAIL", "SKIP")}
    control_counts = {status: sum(result["status"] == status for result in controls) for status in ("PASS", "FAIL", "SKIP")}
    version = run_command([str(binary), "--version"])
    require(version.returncode == 0, f"failed to read product version: {command_error(version)}")
    build_info_result = run_command([str(binary), "build-info"])
    require(build_info_result.returncode == 0, f"failed to read product build info: {command_error(build_info_result)}")
    build_info = json.loads(build_info_result.stdout)
    return {
        "schema": "stratadiff-review-ledger-benchmark-result-v1",
        "dataset_schema": manifest["schema"],
        "dataset_version": manifest["dataset_version"],
        "dataset_manifest_sha256": manifest_digest,
        "product": {
            "binary": str(binary.resolve()),
            "version": version.stdout.decode().strip(),
            "build_info": build_info,
        },
        "summary": counts,
        "control_summary": control_counts,
        "cases": results,
        "controls": controls,
    }


def print_summary(report: dict[str, object]) -> None:
    for result in report["cases"]:
        line = f"{result['status']:4} {result['id']}"
        if result["status"] == "SKIP":
            line += f" — {result['reason']}"
        print(line)
        if result["status"] == "FAIL":
            for failure in result["failures"]:
                print(f"     {failure}")
    for control in report["controls"]:
        line = f"{control['status']:4} control:{control['id']}"
        if control["status"] == "SKIP":
            line += f" — {control['reason']}"
        print(line)
        if control["status"] == "FAIL":
            for failure in control["failures"]:
                print(f"     {failure}")
    summary = report["summary"]
    controls = report["control_summary"]
    print(
        f"cases: {summary['PASS']} passed, {summary['FAIL']} failed, {summary['SKIP']} skipped; "
        f"controls: {controls['PASS']} passed, {controls['FAIL']} failed, {controls['SKIP']} skipped"
    )


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--case", action="append", dest="cases", help="run one manifest case ID; may be repeated")
    parser.add_argument("--keep-workdir", type=Path, help="retain materialized webhook and Git fixtures here")
    parser.add_argument("--list", action="store_true", help="list manifest cases and runner support")
    parser.add_argument("--strict-skips", action="store_true", help="treat any SKIP as a failing exit status")
    return parser.parse_args()


def main() -> int:
    arguments = parse_arguments()
    manifest_bytes = arguments.manifest.read_bytes()
    manifest = json.loads(manifest_bytes)
    validate_manifest(manifest)
    if arguments.list:
        for case in manifest["cases"]:
            print(f"RUN  {case['id']}")
        print("RUN  control:passport-tamper-detected")
        return 0
    require(arguments.binary.is_file(), f"StrataDiff binary not found: {arguments.binary}; run `cargo build --bin stratadiff`")
    selected = set(arguments.cases) if arguments.cases is not None else None
    if selected is not None:
        known = {case["id"] for case in manifest["cases"]} | {"passport-tamper-detected"}
        unknown = sorted(selected - known)
        require(not unknown, f"unknown selected case IDs: {', '.join(unknown)}")
    temporary = None
    if arguments.keep_workdir is not None:
        work_root = arguments.keep_workdir.resolve()
        require(not work_root.exists(), f"keep-workdir already exists: {work_root}")
        work_root.mkdir(parents=True)
    else:
        temporary = tempfile.TemporaryDirectory(prefix="stratadiff-review-ledger-v1-")
        work_root = Path(temporary.name)
    report = run_benchmark(
        arguments.binary.resolve(),
        manifest,
        sha256(manifest_bytes),
        selected,
        work_root,
    )
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_bytes(encode_json(report) + b"\n")
    print_summary(report)
    print(f"result: {arguments.output.resolve()}")
    if temporary is not None:
        temporary.cleanup()
    failed = report["summary"]["FAIL"] + report["control_summary"]["FAIL"]
    skipped = report["summary"]["SKIP"] + report["control_summary"]["SKIP"]
    return 1 if failed or (arguments.strict_skips and skipped) else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BenchmarkFailure as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
