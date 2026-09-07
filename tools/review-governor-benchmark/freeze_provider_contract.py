#!/usr/bin/env python3
"""Freeze the public CodeRabbit evidence contract used by the governor."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from datetime import datetime
from pathlib import Path


SCHEMA_VERSION = "review-governor-provider-contract-v0"
STATUS_CONTEXT = "CodeRabbit"
STATUS_DESCRIPTION = "Review completed"
STATUS_AVATAR_URL = "https://avatars.githubusercontent.com/in/347564?v=4"
CODERABBIT_BOT_ID = 136622811
CODERABBIT_LOGIN = "coderabbitai[bot]"
CODERABBIT_APP_URL = "https://github.com/apps/coderabbitai"
REVIEW_MARKER = "<!-- This is an auto-generated comment by CodeRabbit for review status -->"
COMMAND_INVOCATION_PATTERN = re.compile(
    r"<!-- CodeRabbit review command invocation: "
    r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12} -->"
)
INLINE_SKIP_TEXT = "> Skipped: comment is from another GitHub bot."
MAX_SKEW_SECONDS = 300

CONVERSATION_CASES = [
    {
        "repository": "jarvis236/autoresearch",
        "pr_number": 1,
        "command_comment_id": 4624139462,
        "acknowledgement_comment_id": 4624140026,
        "review_id": 4429619199,
    },
    {
        "repository": "jarvis236/hermes-agent",
        "pr_number": 1,
        "command_comment_id": 4624141524,
        "acknowledgement_comment_id": 4624142146,
        "review_id": 4429631728,
    },
    {
        "repository": "jarvis236/opensrc",
        "pr_number": 1,
        "command_comment_id": 4624143477,
        "acknowledgement_comment_id": 4624144172,
        "review_id": 4429630163,
    },
]

INLINE_SKIP_CASE = {
    "repository": "Comfy-Org/ComfyUI_frontend",
    "pr_number": 15293,
    "root_comment_id": 3787594687,
    "trigger_comment_id": 3787634776,
    "response_comment_id": 3787635253,
}


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def parse_time(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError(f"timestamp lacks timezone: {value}")
    return parsed


def gh_json(endpoint: str) -> dict:
    completed = subprocess.run(
        ["gh", "api", "--cache", "24h", endpoint],
        check=True,
        stdout=subprocess.PIPE,
    )
    value = json.loads(completed.stdout)
    if not isinstance(value, dict):
        raise ValueError(f"GitHub response is not an object: {endpoint}")
    return value


def gh_paginated_list(endpoint: str) -> list[dict]:
    completed = subprocess.run(
        [
            "gh",
            "api",
            "--cache",
            "24h",
            "--paginate",
            "--slurp",
            endpoint,
        ],
        check=True,
        stdout=subprocess.PIPE,
    )
    pages = json.loads(completed.stdout)
    return [item for page in pages for item in page]


def public_body(body: str) -> dict:
    return {
        "bytes": len(body.encode()),
        "sha256": sha256(body.encode()),
    }


def public_user(user: dict) -> dict:
    return {
        "id": user["id"],
        "login": user["login"],
        "type": user["type"],
        "html_url": user["html_url"],
    }


def nearest_completed_status(statuses: list[dict], submitted_at: str) -> tuple[dict, int]:
    review_time = parse_time(submitted_at)
    candidates = [
        status
        for status in statuses
        if status["context"] == STATUS_CONTEXT
        and status["state"] == "success"
        and status["description"] == STATUS_DESCRIPTION
    ]
    if not candidates:
        raise ValueError("review head has no successful Review completed status")
    status = min(
        candidates,
        key=lambda item: (
            abs((parse_time(item["created_at"]) - review_time).total_seconds()),
            item["created_at"],
            item["id"],
        ),
    )
    skew = int(abs((parse_time(status["created_at"]) - review_time).total_seconds()))
    return status, skew


def freeze_review_contract(trace: dict) -> list[dict]:
    frozen = []
    for case in trace["cases"]:
        repository = case["repository"]
        pr_number = case["pr_number"]
        reviews = gh_paginated_list(
            f"repos/{repository}/pulls/{pr_number}/reviews?per_page=100"
        )
        reviews_by_id = {review["id"]: review for review in reviews}
        statuses_by_head = {}
        for event in case["events"]:
            if event["kind"] != "observed_review":
                continue
            review = reviews_by_id[event["source_review_id"]]
            head = event["head_sha"]
            if head not in statuses_by_head:
                statuses_by_head[head] = gh_paginated_list(
                    f"repos/{repository}/commits/{head}/statuses?per_page=100"
                )
            status, skew = nearest_completed_status(
                statuses_by_head[head], review["submitted_at"]
            )
            body = review["body"]
            frozen.append(
                {
                    "case_id": case["case_id"],
                    "repository": repository,
                    "pr_number": pr_number,
                    "head_sha": head,
                    "review": {
                        "id": review["id"],
                        "state": review["state"],
                        "commit_id": review["commit_id"],
                        "submitted_at": review["submitted_at"],
                        "body": public_body(body),
                        "marker_present": REVIEW_MARKER in body,
                        "user": public_user(review["user"]),
                        "source_url": (
                            f"https://github.com/{repository}/pull/{pr_number}"
                            f"#pullrequestreview-{review['id']}"
                        ),
                    },
                    "status": {
                        "id": status["id"],
                        "state": status["state"],
                        "context": status["context"],
                        "description": status["description"],
                        "created_at": status["created_at"],
                        "updated_at": status["updated_at"],
                        "avatar_url": status["avatar_url"],
                        "creator": public_user(status["creator"]),
                        "source_url": (
                            f"https://api.github.com/repos/{repository}/commits/"
                            f"{head}/statuses"
                        ),
                    },
                    "completion_skew_seconds": skew,
                }
            )
    return frozen


def freeze_conversation_case(selection: dict) -> dict:
    repository = selection["repository"]
    pr_number = selection["pr_number"]
    command = gh_json(
        f"repos/{repository}/issues/comments/{selection['command_comment_id']}"
    )
    acknowledgement = gh_json(
        f"repos/{repository}/issues/comments/{selection['acknowledgement_comment_id']}"
    )
    review = gh_json(
        f"repos/{repository}/pulls/{pr_number}/reviews/{selection['review_id']}"
    )
    command_body = command["body"]
    acknowledgement_body = acknowledgement["body"]
    review_body = review["body"]
    return {
        **selection,
        "route": "pull_request_conversation",
        "command": {
            "id": command["id"],
            "created_at": command["created_at"],
            "body": public_body(command_body),
            "is_full_review_command": command_body.strip() == "@coderabbitai full review",
            "user": public_user(command["user"]),
            "source_url": command["html_url"],
        },
        "acknowledgement": {
            "id": acknowledgement["id"],
            "created_at": acknowledgement["created_at"],
            "updated_at": acknowledgement["updated_at"],
            "body": public_body(acknowledgement_body),
            "command_invocation_marker_present": bool(
                COMMAND_INVOCATION_PATTERN.search(acknowledgement_body)
            ),
            "full_review_finished_present": "Full review finished." in acknowledgement_body,
            "user": public_user(acknowledgement["user"]),
            "source_url": acknowledgement["html_url"],
        },
        "review": {
            "id": review["id"],
            "commit_id": review["commit_id"],
            "state": review["state"],
            "submitted_at": review["submitted_at"],
            "body": public_body(review_body),
            "marker_present": REVIEW_MARKER in review_body,
            "user": public_user(review["user"]),
            "source_url": review["html_url"],
        },
        "command_to_review_seconds": int(
            (parse_time(review["submitted_at"]) - parse_time(command["created_at"])).total_seconds()
        ),
        "review_to_acknowledgement_update_seconds": int(
            (
                parse_time(acknowledgement["updated_at"])
                - parse_time(review["submitted_at"])
            ).total_seconds()
        ),
    }


def freeze_inline_skip_case(selection: dict) -> dict:
    repository = selection["repository"]
    root = gh_json(
        f"repos/{repository}/pulls/comments/{selection['root_comment_id']}"
    )
    trigger = gh_json(
        f"repos/{repository}/pulls/comments/{selection['trigger_comment_id']}"
    )
    response = gh_json(
        f"repos/{repository}/pulls/comments/{selection['response_comment_id']}"
    )
    return {
        **selection,
        "route": "inline_review_thread",
        "root": {
            "id": root["id"],
            "user": public_user(root["user"]),
            "source_url": root["html_url"],
        },
        "trigger": {
            "id": trigger["id"],
            "in_reply_to_id": trigger["in_reply_to_id"],
            "created_at": trigger["created_at"],
            "body": public_body(trigger["body"]),
            "user": public_user(trigger["user"]),
            "source_url": trigger["html_url"],
        },
        "response": {
            "id": response["id"],
            "in_reply_to_id": response["in_reply_to_id"],
            "created_at": response["created_at"],
            "body": public_body(response["body"]),
            "skip_text_present": INLINE_SKIP_TEXT in response["body"],
            "user": public_user(response["user"]),
            "source_url": response["html_url"],
        },
    }


def freeze(trace: dict, frozen_at: str) -> dict:
    return {
        "schema_version": SCHEMA_VERSION,
        "frozen_at": frozen_at,
        "provider": "coderabbit",
        "contract": {
            "status_context": STATUS_CONTEXT,
            "status_description": STATUS_DESCRIPTION,
            "status_avatar_url": STATUS_AVATAR_URL,
            "review_bot_id": CODERABBIT_BOT_ID,
            "review_bot_login": CODERABBIT_LOGIN,
            "review_bot_app_url": CODERABBIT_APP_URL,
            "review_marker_sha256": sha256(REVIEW_MARKER.encode()),
            "maximum_completion_skew_seconds": MAX_SKEW_SECONDS,
        },
        "review_evidence": freeze_review_contract(trace),
        "command_routes": {
            "conversation_successes": [
                freeze_conversation_case(selection) for selection in CONVERSATION_CASES
            ],
            "inline_bot_reply_skip": freeze_inline_skip_case(INLINE_SKIP_CASE),
        },
        "claim_boundary": (
            "These public observations fingerprint one CodeRabbit deployment period. They do not "
            "promise future provider behavior, prove private model execution, or show that every "
            "bot-authored command will be accepted under every repository policy and quota state."
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trace", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--frozen-at", required=True)
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args()
    trace = json.loads(arguments.trace.read_bytes())
    encoded = canonical_json(freeze(trace, arguments.frozen_at))
    if arguments.check:
        if arguments.output.read_bytes() != encoded:
            raise SystemExit("frozen provider contract differs from live public reconstruction")
        print(f"verified {arguments.output}")
    else:
        arguments.output.write_bytes(encoded)
        print(f"wrote {arguments.output} sha256={sha256(encoded)}")


if __name__ == "__main__":
    main()
