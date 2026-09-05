#!/usr/bin/env python3

import argparse
import base64
from datetime import datetime, timezone
import difflib
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import urllib.request


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = ROOT / "benchmarks" / "resumebench-github-live-v1" / "manifest.json"
MANIFEST_SCHEMA = "stratadiff-resumebench-github-live-manifest-v1"
ORACLE_SCHEMA = "stratadiff-resumebench-github-live-oracle-v1"
EVALUATION_SCHEMA = "stratadiff-resumebench-github-live-evaluation-v1"
MATERIALIZATION_SCHEMA = "stratadiff-resumebench-github-live-materialization-v1"
BUILD_INFO_SCHEMA = "stratadiff-build-info-v1"
REPORT_MATCH_BASIS = "exact_git_change_identity_or_noninteracting_four_way_byte_replay"
EVALUATION_CLAIM_BOUNDARY = "Five purposefully selected GitHub force-push histories; exact policy conformance only, with no human-priority, reviewer-time, defect-recall, semantic-safety, or prevalence ground truth."
MAX_REPLAY_SOURCE_BYTES = 16 * 1024 * 1024
BYTE_DIFF_BUDGET = 64 * 1024
LINE_ANCHOR_BUDGET = 64 * 1024
LARGE_REGION_EDIT_BUDGET = 4 * 1024
PATCH_EDIT_BUDGET = 64 * 1024
LOCAL_COMMAND_TIMEOUT_SECONDS = 120
MAX_GITHUB_RESPONSE_BYTES = 4 * 1024 * 1024
STATUS_NAMES = {
    "added": "A",
    "copied": "C",
    "deleted": "D",
    "modified": "M",
    "renamed": "R",
    "type_changed": "T",
}
STATUS_RANK = {"A": 0, "C": 1, "D": 2, "M": 3, "R": 4, "T": 5}


class RejectRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        return None


GITHUB_API_OPENER = urllib.request.build_opener(RejectRedirectHandler())


def require(condition, message):
    if not condition:
        raise ValueError(message)


def unique_json_object(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value, f"duplicate JSON key: {key}")
        value[key] = item
    return value


def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_json_object)


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def validate_oid(value, label):
    require(
        isinstance(value, str)
        and len(value) == 40
        and all(character in "0123456789abcdef" for character in value),
        f"{label} is not a full lowercase SHA-1 object ID: {value}",
    )


def parse_time(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def validate_github_token(token):
    require(
        isinstance(token, str)
        and token
        and token.isascii()
        and "\n" not in token
        and "\r" not in token,
        "invalid GitHub token",
    )


def github_git_authorization(token):
    validate_github_token(token)
    credential = f"x-access-token:{token}".encode("ascii")
    encoded = base64.b64encode(credential).decode("ascii")
    return f"Authorization: Basic {encoded}"


def isolated_environment(*, allow_lazy_fetch=False, token=None):
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.upper().startswith("GIT_") and name not in ("GH_TOKEN", "GITHUB_TOKEN")
    }
    environment["LC_ALL"] = "C"
    environment["GIT_CONFIG_GLOBAL"] = os.devnull
    environment["GIT_CONFIG_NOSYSTEM"] = "1"
    environment["GIT_NO_REPLACE_OBJECTS"] = "1"
    environment["GIT_TERMINAL_PROMPT"] = "0"
    if not allow_lazy_fetch:
        environment["GIT_NO_LAZY_FETCH"] = "1"
    if token is not None:
        environment["GIT_CONFIG_COUNT"] = "2"
        environment["GIT_CONFIG_KEY_0"] = "http.https://github.com/.extraHeader"
        environment["GIT_CONFIG_VALUE_0"] = github_git_authorization(token)
        environment["GIT_CONFIG_KEY_1"] = "http.followRedirects"
        environment["GIT_CONFIG_VALUE_1"] = "false"
    return environment


def run_git(
    repository,
    arguments,
    *,
    input_bytes=None,
    allow_lazy_fetch=False,
    token=None,
    timeout=LOCAL_COMMAND_TIMEOUT_SECONDS,
    check=True,
):
    result = subprocess.run(
        ["git", "--no-replace-objects", "-C", str(repository), *arguments],
        env=isolated_environment(allow_lazy_fetch=allow_lazy_fetch, token=token),
        input=input_bytes,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )
    if check and result.returncode != 0:
        diagnostic = result.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"git {' '.join(arguments)} failed: {diagnostic}")
    return result


def git_stdout(repository, arguments, **options):
    result = run_git(repository, arguments, **options)
    allowed_warning = b"warning: lazy fetching disabled; some objects may not be available\n"
    require(
        result.stderr in (b"", allowed_warning),
        f"git {' '.join(arguments)} produced diagnostics",
    )
    return result.stdout


def validate_manifest(manifest):
    require(manifest["schema"] == MANIFEST_SCHEMA, "unsupported manifest schema")
    require(manifest["dataset_version"] == "1.0.0", "unsupported dataset version")
    require(manifest["capture_date"] == "2026-09-05", "unexpected capture date")
    require(manifest["dataset_kind"] == "purposefully_selected_diagnostic_cases", "invalid dataset kind")
    boundary = manifest["claim_boundary"]
    require(boundary["diagnostic_sample"] is True, "dataset must remain diagnostic")
    require(boundary["population_estimates_supported"] is False, "population claims are forbidden")
    require(boundary["human_priority_ground_truth"] == "absent", "human priority ground truth must be absent")
    require(boundary["policy_ground_truth"] == "frozen_independent_oracles", "policy oracle state is not frozen")
    cases = manifest["cases"]
    require(len(cases) == 5, "v1 must contain exactly five cases")
    ids = set()
    totals = {
        "current_pr_files": 0,
        "exact_carries": 0,
        "four_way_carries": 0,
        "needs_review_now": 0,
        "retired_checkpoint_changes": 0,
        "naive_snapshot_paths": 0,
        "naive_extra_paths": 0,
        "naive_missing_current_paths": 0,
    }
    roles = {
        "Q": "requested_base",
        "A": "checkpoint_merge_base",
        "B": "reviewed_checkpoint",
        "C": "current_merge_base",
        "D": "captured_final_head",
    }
    for case in cases:
        case_id = case["id"]
        require(
            case_id and all(character in "abcdefghijklmnopqrstuvwxyz0123456789-" for character in case_id),
            f"invalid case ID slug: {case_id}",
        )
        require(case_id not in ids, f"duplicate case ID: {case_id}")
        ids.add(case_id)
        snapshots = case["snapshots"]
        for label, role in roles.items():
            require(snapshots[label]["role"] == role, f"{case_id} snapshot {label} role differs")
            validate_oid(snapshots[label]["commit"], f"{case_id} snapshot {label}")
        require(case["pull_request"]["requested_base"] == snapshots["Q"]["commit"], f"{case_id} Q differs")
        require(case["pull_request"]["captured_head"] == snapshots["D"]["commit"], f"{case_id} D differs")
        repository = case["repository"]
        for field in ("owner", "name"):
            require(
                repository[field]
                and all(character.isascii() and (character.isalnum() or character in ".-_") for character in repository[field]),
                f"{case_id} repository {field} is invalid",
            )
        require(
            repository["git_url"] == f"https://github.com/{repository['owner']}/{repository['name']}.git",
            f"{case_id} repository URL is not the exact GitHub HTTPS origin",
        )
        require(repository["object_format"] == "sha1", f"{case_id} object format differs")
        license_observation = repository["license_observation"]
        license_path = Path(license_observation["path"])
        require(
            not license_path.is_absolute()
            and license_path.parts
            and ".." not in license_path.parts,
            f"{case_id} license path escapes the repository",
        )
        validate_oid(license_observation["blob_oid"], f"{case_id} license blob")
        require(
            isinstance(license_observation["content_sha256"], str)
            and len(license_observation["content_sha256"]) == 64
            and all(character in "0123456789abcdef" for character in license_observation["content_sha256"]),
            f"{case_id} license content digest is invalid",
        )
        require(license_observation["spdx_id"], f"{case_id} license SPDX observation is empty")
        review = case["checkpoint_review"]
        require(review["commit"] == snapshots["B"]["commit"], f"{case_id} B differs")
        require(review["state"] in ("APPROVED", "CHANGES_REQUESTED"), f"{case_id} review state is ineligible")
        require(review["account_type"] == "User", f"{case_id} review is not human")
        require(
            parse_time(review["submitted_at"]) <= parse_time(case["pull_request"]["captured_at"]),
            f"{case_id} checkpoint review postdates capture",
        )
        require(
            review["author_association"] in ("COLLABORATOR", "CONTRIBUTOR", "MEMBER", "OWNER"),
            f"{case_id} review author association is invalid",
        )
        observation = case["capture_observation"]
        required_observation_fields = {
            "commit_api_available",
            "exact_sha_fetch_available",
            "checkpoint_ancestor_of_captured_head",
            "checkpoint_advertised_as_ref_tip",
            "latest_eligible_review_at_capture",
            "captured_head_relation_to_last_force_after",
        }
        optional_observation_fields = {"commits_ahead_of_last_force_after"}
        require(
            required_observation_fields <= set(observation)
            and set(observation) <= required_observation_fields | optional_observation_fields,
            f"{case_id} capture observation fields differ",
        )
        require(observation["commit_api_available"] is True, f"{case_id} checkpoint commit was unavailable at capture")
        require(observation["exact_sha_fetch_available"] is True, f"{case_id} exact SHA was unavailable at capture")
        require(observation["checkpoint_ancestor_of_captured_head"] is False, f"{case_id} checkpoint must not be an ancestor of D")
        require(observation["checkpoint_advertised_as_ref_tip"] is False, f"{case_id} checkpoint must not be an advertised ref tip")
        has_later_review = "later_eligible_review_at_capture" in case
        expected_latest_review = not has_later_review
        require(
            observation["latest_eligible_review_at_capture"] is expected_latest_review,
            f"{case_id} latest-review observation differs",
        )
        require(
            observation["captured_head_relation_to_last_force_after"] in ("same_commit", "descendant"),
            f"{case_id} captured-head relation is invalid",
        )
        if has_later_review:
            later_review = case["later_eligible_review_at_capture"]
            require(later_review["id"] != review["id"], f"{case_id} later review ID is not distinct")
            require(later_review["reviewer_login"] == review["reviewer_login"], f"{case_id} later reviewer differs")
            require(later_review["state"] in ("APPROVED", "CHANGES_REQUESTED"), f"{case_id} later review state is ineligible")
            require(later_review["account_type"] == "User", f"{case_id} later review is not human")
            validate_oid(later_review["commit"], f"{case_id} later review commit")
            require(later_review["commit"] == snapshots["D"]["commit"], f"{case_id} later review is not attached to D")
            require(parse_time(later_review["submitted_at"]) > parse_time(review["submitted_at"]), f"{case_id} later review predates checkpoint review")
            require(
                parse_time(later_review["submitted_at"]) <= parse_time(case["pull_request"]["captured_at"]),
                f"{case_id} later review postdates capture",
            )
        chain = case["force_push_chain"]
        require(chain, f"{case_id} has no post-review force-push evidence")
        require(chain[0]["before_commit"] == snapshots["B"]["commit"], f"{case_id} force chain does not begin at B")
        require(parse_time(chain[0]["created_at"]) > parse_time(review["submitted_at"]), f"{case_id} force push predates review")
        for index, event in enumerate(chain):
            require(event["node_id"].startswith("HRFPE_"), f"{case_id} force-push node ID is invalid")
            validate_oid(event["before_commit"], f"{case_id} force-push before")
            validate_oid(event["after_commit"], f"{case_id} force-push after")
            require(parse_time(event["created_at"]) <= parse_time(case["pull_request"]["captured_at"]), f"{case_id} force push postdates capture")
            if index > 0:
                require(
                    chain[index - 1]["after_commit"] == event["before_commit"],
                    f"{case_id} force-push chain is discontinuous",
                )
        relation = observation["captured_head_relation_to_last_force_after"]
        if relation == "same_commit":
            require(chain[-1]["after_commit"] == snapshots["D"]["commit"], f"{case_id} final force-push commit differs from D")
            require("commits_ahead_of_last_force_after" not in observation, f"{case_id} same-commit observation has an ahead count")
        else:
            require(chain[-1]["after_commit"] != snapshots["D"]["commit"], f"{case_id} descendant observation equals D")
            require(observation["commits_ahead_of_last_force_after"] > 0, f"{case_id} descendant observation has no positive ahead count")
        observed = case["observed"]
        require(
            observed["current_pr_files"]
            == observed["exact_carries"]
            + observed["four_way_carries"]
            + observed["needs_review_now"],
            f"{case_id} observed partition is inconsistent",
        )
        for key in totals:
            totals[key] += observed[key]
        oracle = Path(case["expectation"]["oracle"])
        require(not oracle.is_absolute() and oracle.parts[0] == "oracles" and ".." not in oracle.parts, f"{case_id} oracle path escapes the bundle")
        require(case["expectation"]["oracle_kind"] == "exact_policy_conformance", f"{case_id} oracle kind differs")
        require(case["expectation"]["human_priority_ground_truth"] == "absent", f"{case_id} human ground truth differs")
    observed_totals = manifest["observed_totals"]
    require(observed_totals["case_count"] == len(cases), "observed case total differs")
    for key, value in totals.items():
        require(observed_totals[key] == value, f"observed total differs: {key}")
    require(
        observed_totals["carried_files"]
        == observed_totals["exact_carries"] + observed_totals["four_way_carries"],
        "observed carried total differs",
    )


def load_manifest(path):
    manifest = load_json(path)
    validate_manifest(manifest)
    return manifest


def expected_summary(case):
    observed = case["observed"]
    return {
        "current_pr_files": observed["current_pr_files"],
        "carried": observed["exact_carries"] + observed["four_way_carries"],
        "exactly_carried": observed["exact_carries"],
        "replay_carried": observed["four_way_carries"],
        "needs_review_now": observed["needs_review_now"],
        "retired_checkpoint_changes": observed["retired_checkpoint_changes"],
        "naive_snapshot_paths": observed["naive_snapshot_paths"],
        "naive_extra_paths": observed["naive_extra_paths"],
        "naive_missing_current_paths": observed["naive_missing_current_paths"],
    }


def github_json(url, token, timeout, *, payload=None):
    require(
        url == "https://api.github.com/graphql" or url.startswith("https://api.github.com/repos/"),
        f"GitHub API URL is outside the fixed origin: {url}",
    )
    body = None if payload is None else canonical_json_bytes(payload)
    request = urllib.request.Request(
        url,
        data=body,
        method="GET" if body is None else "POST",
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "User-Agent": "stratadiff-resumebench-github-live-v1",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with GITHUB_API_OPENER.open(request, timeout=timeout) as response:
        require(response.status == 200, f"GitHub API returned HTTP {response.status}")
        raw = response.read(MAX_GITHUB_RESPONSE_BYTES + 1)
        require(len(raw) <= MAX_GITHUB_RESPONSE_BYTES, "GitHub API response exceeds size limit")
        return json.loads(raw, object_pairs_hook=unique_json_object)


def verify_review_provenance(case, review, pull_url, token, timeout, label):
    review_url = f"{pull_url}/reviews/{review['id']}"
    live_review = github_json(review_url, token, timeout)
    require(live_review["id"] == review["id"], f"{case['id']} {label} review ID differs")
    require(live_review["html_url"] == review["url"], f"{case['id']} {label} review URL differs")
    require(live_review["pull_request_url"] == pull_url, f"{case['id']} {label} review is bound to another pull request")
    require(live_review["user"]["login"] == review["reviewer_login"], f"{case['id']} {label} reviewer login differs")
    require(live_review["user"]["type"] == review["account_type"], f"{case['id']} {label} reviewer account type differs")
    if "author_association" in review:
        require(live_review["author_association"] == review["author_association"], f"{case['id']} {label} reviewer association differs")
    require(live_review["state"] == review["state"], f"{case['id']} {label} review state differs")
    require(live_review["commit_id"] == review["commit"], f"{case['id']} {label} review commit differs")
    require(parse_time(live_review["submitted_at"]) == parse_time(review["submitted_at"]), f"{case['id']} {label} review timestamp differs")


def verify_provenance_case(case, token, timeout):
    repository = case["repository"]
    owner = repository["owner"]
    name = repository["name"]
    number = case["pull_request"]["number"]
    api_root = f"https://api.github.com/repos/{owner}/{name}"
    pull_url = f"{api_root}/pulls/{number}"
    pull = github_json(pull_url, token, timeout)
    require(pull["number"] == number, f"{case['id']} pull request number differs")
    require(pull["html_url"] == case["pull_request"]["url"], f"{case['id']} pull request URL differs")
    require(pull["base"]["repo"]["full_name"] == f"{owner}/{name}", f"{case['id']} pull request repository differs")
    require(pull["base"]["sha"] == case["snapshots"]["Q"]["commit"], f"{case['id']} live Q differs")
    require(pull["head"]["sha"] == case["snapshots"]["D"]["commit"], f"{case['id']} live D differs")
    require(pull["base"]["ref"] == case["pull_request"]["base_ref_name"], f"{case['id']} base ref differs")
    require(pull["head"]["ref"] == case["pull_request"]["head_ref_name"], f"{case['id']} head ref differs")
    expected_merged = case["pull_request"]["state_at_capture"] == "MERGED"
    require(pull["merged"] is expected_merged, f"{case['id']} merged state differs")
    for field in ("created_at", "closed_at", "merged_at"):
        require(parse_time(pull[field]) == parse_time(case["pull_request"][field]), f"{case['id']} pull request {field} differs")

    review = case["checkpoint_review"]
    verify_review_provenance(case, review, pull_url, token, timeout, "checkpoint")
    verified_reviews = 1
    if "later_eligible_review_at_capture" in case:
        verify_review_provenance(
            case,
            case["later_eligible_review_at_capture"],
            pull_url,
            token,
            timeout,
            "later eligible",
        )
        verified_reviews += 1

    for label in ("Q", "B", "D"):
        commit = case["snapshots"][label]["commit"]
        live_commit = github_json(f"{api_root}/git/commits/{commit}", token, timeout)
        require(live_commit["sha"] == commit, f"{case['id']} commit endpoint returned another {label}")

    return {
        "id": case["id"],
        "repository": f"{owner}/{name}",
        "pull_request": number,
        "review_id": review["id"],
        "verified_reviews": verified_reviews,
        "verified_commits": [case["snapshots"][label]["commit"] for label in ("Q", "B", "D")],
        "force_push_events": len(case["force_push_chain"]),
    }


def verify_provenance(manifest_path, token, timeout):
    manifest = load_manifest(manifest_path)
    validate_github_token(token)
    require(timeout > 0, "GitHub API timeout must be positive")
    cases = [verify_provenance_case(case, token, timeout) for case in manifest["cases"]]
    event_ids = [event["node_id"] for case in manifest["cases"] for event in case["force_push_chain"]]
    graphql = github_json(
        "https://api.github.com/graphql",
        token,
        timeout,
        payload={
            "query": "query($ids: [ID!]!) { nodes(ids: $ids) { __typename ... on HeadRefForcePushedEvent { id createdAt beforeCommit { oid } afterCommit { oid } pullRequest { number repository { nameWithOwner } } } } }",
            "variables": {"ids": event_ids},
        },
    )
    require("errors" not in graphql, "GitHub GraphQL provenance query returned errors")
    nodes = graphql["data"]["nodes"]
    require(len(nodes) == len(event_ids), "GitHub GraphQL force-push result count differs")
    by_id = {}
    for node in nodes:
        require(node is not None and node["__typename"] == "HeadRefForcePushedEvent", "GitHub force-push node is unavailable or has another type")
        require(node["id"] not in by_id, f"duplicate GitHub force-push node: {node['id']}")
        by_id[node["id"]] = node
    require(set(by_id) == set(event_ids), "GitHub force-push node coverage differs")
    for case in manifest["cases"]:
        expected_repository = f"{case['repository']['owner']}/{case['repository']['name']}"
        for event in case["force_push_chain"]:
            node = by_id[event["node_id"]]
            require(parse_time(node["createdAt"]) == parse_time(event["created_at"]), f"{case['id']} force-push timestamp differs")
            require(node["beforeCommit"]["oid"] == event["before_commit"], f"{case['id']} force-push before commit differs")
            require(node["afterCommit"]["oid"] == event["after_commit"], f"{case['id']} force-push after commit differs")
            require(node["pullRequest"]["number"] == case["pull_request"]["number"], f"{case['id']} force-push pull request differs")
            require(node["pullRequest"]["repository"]["nameWithOwner"] == expected_repository, f"{case['id']} force-push repository differs")
    return {
        "verified_cases": len(cases),
        "verified_reviews": sum(case["verified_reviews"] for case in cases),
        "verified_commits": sum(len(case["verified_commits"]) for case in cases),
        "verified_force_push_events": len(event_ids),
    }


def optional_oid(value):
    return None if set(value) == {"0"} else value


def optional_mode(value):
    return None if value == "000000" else value


def raw_change(status, path, before_mode, after_mode, before_oid, after_oid):
    return {
        "status": status,
        "similarity_percent": None,
        "before_path": None if status == "A" else path,
        "after_path": None if status == "D" else path,
        "before_mode": optional_mode(before_mode),
        "after_mode": optional_mode(after_mode),
        "before_object_id": optional_oid(before_oid),
        "after_object_id": optional_oid(after_oid),
    }


def parse_raw_diff(raw):
    fields = raw.split(b"\0")
    require(fields[-1] == b"", "raw Git diff is not NUL terminated")
    fields.pop()
    changes = []
    index = 0
    while index < len(fields):
        columns = fields[index].decode("ascii").split()
        index += 1
        require(len(columns) == 5 and columns[0].startswith(":"), "invalid raw Git diff header")
        status = columns[4]
        require(status in ("A", "D", "M", "T"), f"unsupported --no-renames status: {status}")
        require(index < len(fields) and fields[index], "raw Git diff record is missing a path")
        path = fields[index]
        index += 1
        changes.append(
            raw_change(
                status,
                path,
                columns[0][1:],
                columns[1],
                columns[2],
                columns[3],
            )
        )
    return changes


def normalize_exact_relocations(changes):
    candidates = {}
    for index, change in enumerate(changes):
        if change["status"] == "D":
            key = (change["before_object_id"], change["before_mode"])
            if key not in candidates:
                candidates[key] = {"deleted": [], "added": []}
            candidates[key]["deleted"].append(index)
        elif change["status"] == "A":
            key = (change["after_object_id"], change["after_mode"])
            if key not in candidates:
                candidates[key] = {"deleted": [], "added": []}
            candidates[key]["added"].append(index)
    replacements = {}
    removed = set()
    for indexes in candidates.values():
        deleted = indexes["deleted"]
        added = indexes["added"]
        if len(deleted) == 1 and len(added) == 1:
            deleted_index = deleted[0]
            added_index = added[0]
            before = changes[deleted_index]
            after = changes[added_index]
            replacements[min(deleted_index, added_index)] = {
                "status": "R",
                "similarity_percent": 100,
                "before_path": before["before_path"],
                "after_path": after["after_path"],
                "before_mode": before["before_mode"],
                "after_mode": after["after_mode"],
                "before_object_id": before["before_object_id"],
                "after_object_id": after["after_object_id"],
            }
            removed.add(deleted_index)
            removed.add(added_index)
    output = []
    for index, change in enumerate(changes):
        if index in replacements:
            output.append(replacements[index])
        elif index not in removed:
            output.append(change)
    output.sort(key=change_sort_key)
    return output


def path_sort_value(path):
    return b"" if path is None else path


def oid_sort_value(value):
    return "" if value is None else value


def change_sort_key(change):
    return (
        path_sort_value(change["before_path"]),
        path_sort_value(change["after_path"]),
        STATUS_RANK[change["status"]],
        oid_sort_value(change["before_object_id"]),
        oid_sort_value(change["after_object_id"]),
    )


def path_base64(path):
    return None if path is None else base64.b64encode(path).decode("ascii")


def identity_payload(change):
    return {
        "status": change["status"],
        "similarity_percent": change["similarity_percent"],
        "before_path_base64": path_base64(change["before_path"]),
        "after_path_base64": path_base64(change["after_path"]),
        "before_mode": change["before_mode"],
        "after_mode": change["after_mode"],
        "before_object_id": change["before_object_id"],
        "after_object_id": change["after_object_id"],
    }


def canonical_json_bytes(value):
    return json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":")).encode("ascii")


def identity(change):
    payload = identity_payload(change)
    return {"identity_sha256": sha256_bytes(canonical_json_bytes(payload)), **payload}


def identity_key(change):
    return canonical_json_bytes(identity_payload(change))


def raw_diff(repository, left, right):
    raw = git_stdout(
        repository,
        [
            "diff",
            "--raw",
            "-z",
            "--no-abbrev",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--ignore-submodules=none",
            left,
            right,
            "--",
        ],
    )
    raw_changes = parse_raw_diff(raw)
    return raw, raw_changes, normalize_exact_relocations(raw_changes)


def resolve_commit(repository, revision):
    resolved = git_stdout(repository, ["rev-parse", "--verify", "--end-of-options", f"{revision}^{{commit}}"]).decode("ascii").strip()
    validate_oid(resolved, f"resolved commit {revision}")
    return resolved


def unique_merge_base(repository, left, right):
    values = git_stdout(repository, ["merge-base", "--all", left, right]).decode("ascii").split()
    require(len(values) == 1, f"expected one merge base for {left} and {right}")
    validate_oid(values[0], "merge base")
    return values[0]


def is_ancestor(repository, ancestor, descendant):
    result = run_git(repository, ["merge-base", "--is-ancestor", ancestor, descendant], check=False)
    allowed_warning = b"warning: lazy fetching disabled; some objects may not be available\n"
    require(result.returncode in (0, 1), f"Git ancestry check failed: {ancestor}..{descendant}")
    require(not result.stdout and result.stderr in (b"", allowed_warning), f"Git ancestry check produced diagnostics: {ancestor}..{descendant}")
    return result.returncode == 0


def read_blob(repository, object_id):
    object_type = git_stdout(repository, ["cat-file", "-t", object_id]).decode("ascii").strip()
    require(object_type == "blob", f"object is not a blob: {object_id}")
    size = int(git_stdout(repository, ["cat-file", "-s", object_id]).decode("ascii").strip())
    require(size <= MAX_REPLAY_SOURCE_BYTES, f"blob exceeds replay limit: {object_id}")
    content = git_stdout(repository, ["cat-file", "blob", object_id])
    require(len(content) == size, f"blob size changed while reading: {object_id}")
    return content


def tree_blob_oid(repository, commit, path):
    entry = tree_entry(repository, commit, path)
    if entry is None:
        return None
    require(entry[1] == "blob", f"license path is not a blob: {commit}:{path}")
    return entry[2]


def tree_entry(repository, commit, path):
    raw = git_stdout(
        repository,
        ["ls-tree", "-z", commit, "--", f":(literal){path}"],
    )
    if not raw:
        return None
    records = raw.split(b"\0")
    require(records[-1] == b"", f"tree entry is not NUL terminated: {commit}:{path}")
    records.pop()
    require(len(records) == 1, f"tree path resolved ambiguously: {commit}:{path}")
    metadata, separator, raw_path = records[0].partition(b"\t")
    require(separator == b"\t" and raw_path == path.encode("utf-8"), f"tree path differs: {commit}:{path}")
    columns = metadata.decode("ascii").split()
    require(len(columns) == 3, f"tree entry metadata differs: {commit}:{path}")
    validate_oid(columns[2], f"tree blob {commit}:{path}")
    return columns[0], columns[1], columns[2]


def license_observation_evidence(case, repository, commits, *, read_content):
    observation = case["repository"]["license_observation"]
    present_snapshots = []
    for label in ("Q", "A", "B", "C", "D"):
        object_id = tree_blob_oid(repository, commits[label], observation["path"])
        if object_id is None:
            continue
        require(object_id == observation["blob_oid"], f"{case['id']} license blob differs at {label}")
        if read_content:
            require(
                sha256_bytes(read_blob(repository, object_id)) == observation["content_sha256"],
                f"{case['id']} license content differs at {label}",
            )
        present_snapshots.append(label)
    require(present_snapshots, f"{case['id']} license path is absent from every snapshot")
    return {
        "source": observation["source"],
        "spdx_id": observation["spdx_id"],
        "path": observation["path"],
        "blob_oid": observation["blob_oid"],
        "content_sha256": observation["content_sha256"],
        "present_snapshots": present_snapshots,
    }


def split_newline_inclusive(source):
    lines = []
    start = 0
    while start < len(source):
        end = source.find(b"\n", start)
        if end == -1:
            lines.append(source[start:])
            break
        lines.append(source[start : end + 1])
        start = end + 1
    return lines


def offsets(parts):
    values = [0]
    for part in parts:
        values.append(values[-1] + len(part))
    return values


def common_prefix_length(left, right):
    limit = min(len(left), len(right))
    index = 0
    while index < limit and left[index] == right[index]:
        index += 1
    return index


def common_suffix_length(left, right, prefix):
    limit = min(len(left), len(right)) - prefix
    index = 0
    while index < limit and left[len(left) - index - 1] == right[len(right) - index - 1]:
        index += 1
    return index


def make_edit(source, start, end, replacement):
    return {
        "start": start,
        "end": end,
        "old_sha256": sha256_bytes(source[start:end]),
        "replacement": replacement,
    }


def refine_region(before, after, old_start, old_end, new_start, new_end, edits):
    old = before[old_start:old_end]
    new = after[new_start:new_end]
    prefix = common_prefix_length(old, new)
    suffix = common_suffix_length(old, new, prefix)
    trimmed_old_start = old_start + prefix
    trimmed_old_end = old_end - suffix
    trimmed_new_start = new_start + prefix
    trimmed_new_end = new_end - suffix
    trimmed_old = before[trimmed_old_start:trimmed_old_end]
    trimmed_new = after[trimmed_new_start:trimmed_new_end]
    if len(trimmed_old) + len(trimmed_new) <= BYTE_DIFF_BUDGET:
        matcher = difflib.SequenceMatcher(None, trimmed_old, trimmed_new, autojunk=False)
        for tag, before_start, before_end, after_start, after_end in matcher.get_opcodes():
            if tag != "equal":
                require(len(edits) < PATCH_EDIT_BUDGET, "patch edit budget exceeded")
                edits.append(
                    make_edit(
                        before,
                        trimmed_old_start + before_start,
                        trimmed_old_start + before_end,
                        trimmed_new[after_start:after_end],
                    )
                )
    elif len(trimmed_old) == len(trimmed_new):
        cursor = 0
        emitted = 0
        while cursor < len(trimmed_old):
            while cursor < len(trimmed_old) and trimmed_old[cursor] == trimmed_new[cursor]:
                cursor += 1
            if cursor == len(trimmed_old):
                break
            mismatch_start = cursor
            while cursor < len(trimmed_old) and trimmed_old[cursor] != trimmed_new[cursor]:
                cursor += 1
            require(len(edits) < PATCH_EDIT_BUDGET, "patch edit budget exceeded")
            if emitted + 1 == LARGE_REGION_EDIT_BUDGET:
                edits.append(
                    make_edit(
                        before,
                        trimmed_old_start + mismatch_start,
                        trimmed_old_end,
                        trimmed_new[mismatch_start:],
                    )
                )
                break
            edits.append(
                make_edit(
                    before,
                    trimmed_old_start + mismatch_start,
                    trimmed_old_start + cursor,
                    trimmed_new[mismatch_start:cursor],
                )
            )
            emitted += 1
    elif trimmed_old or trimmed_new:
        require(len(edits) < PATCH_EDIT_BUDGET, "patch edit budget exceeded")
        edits.append(make_edit(before, trimmed_old_start, trimmed_old_end, trimmed_new))


def create_patch(before, after):
    before_lines = split_newline_inclusive(before)
    after_lines = split_newline_inclusive(after)
    edits = []
    if len(before_lines) + len(after_lines) > LINE_ANCHOR_BUDGET:
        refine_region(before, after, 0, len(before), 0, len(after), edits)
    else:
        before_offsets = offsets(before_lines)
        after_offsets = offsets(after_lines)
        matcher = difflib.SequenceMatcher(None, before_lines, after_lines, autojunk=False)
        for tag, before_start, before_end, after_start, after_end in matcher.get_opcodes():
            if tag != "equal":
                refine_region(
                    before,
                    after,
                    before_offsets[before_start],
                    before_offsets[before_end],
                    after_offsets[after_start],
                    after_offsets[after_end],
                    edits,
                )
    require(apply_edits(before, edits) == after, "independent patch does not reconstruct target")
    return edits


def validate_edits(source, edits):
    previous_end = 0
    for index, edit in enumerate(edits):
        start = edit["start"]
        end = edit["end"]
        require(0 <= start <= end <= len(source), "edit is outside its source")
        require(index == 0 or previous_end <= start, "edits overlap or are out of order")
        require(sha256_bytes(source[start:end]) == edit["old_sha256"], "edit old bytes differ")
        previous_end = end


def apply_edits(source, edits):
    validate_edits(source, edits)
    output = bytearray()
    position = 0
    for edit in edits:
        output.extend(source[position : edit["start"]])
        output.extend(edit["replacement"])
        position = edit["end"]
    output.extend(source[position:])
    return bytes(output)


def strictly_separated(left, right):
    return left["end"] < right["start"] or right["end"] < left["start"]


def patches_interact(left, right):
    return any(not strictly_separated(left_edit, right_edit) for left_edit in left for right_edit in right)


def translate_patch(patch, preceding):
    translated = []
    for edit in patch:
        delta = 0
        for other in preceding:
            require(strictly_separated(edit, other), "cannot translate interacting patches")
            if other["end"] < edit["start"]:
                delta += len(other["replacement"]) - (other["end"] - other["start"])
        translated.append(
            {
                "start": edit["start"] + delta,
                "end": edit["end"] + delta,
                "old_sha256": edit["old_sha256"],
                "replacement": edit["replacement"],
            }
        )
    return translated


def replay_candidate_metadata_matches(checkpoint, current):
    if checkpoint["status"] != "M" or current["status"] != "M":
        return False
    path = checkpoint["before_path"]
    if path is None:
        return False
    if checkpoint["after_path"] != path or current["before_path"] != path or current["after_path"] != path:
        return False
    mode = checkpoint["before_mode"]
    return (
        mode in ("100644", "100755")
        and checkpoint["after_mode"] == mode
        and current["before_mode"] == mode
        and current["after_mode"] == mode
    )


def snapshot_evidence(object_id, content):
    return {
        "blob_oid": object_id,
        "byte_len": len(content),
        "content_sha256": sha256_bytes(content),
    }


def witness_edit(edit):
    return {
        "start": edit["start"],
        "end": edit["end"],
        "old_sha256": edit["old_sha256"],
        "replacement_byte_len": len(edit["replacement"]),
        "replacement_sha256": sha256_bytes(edit["replacement"]),
    }


def four_way_replay_witness(repository, checkpoint, current, snapshots):
    if not replay_candidate_metadata_matches(checkpoint, current):
        return None
    object_ids = {
        "A": checkpoint["before_object_id"],
        "B": checkpoint["after_object_id"],
        "C": current["before_object_id"],
        "D": current["after_object_id"],
    }
    contents = {label: read_blob(repository, object_id) for label, object_id in object_ids.items()}
    if any(b"\0" in content for content in contents.values()):
        return None
    checkpoint_edits = create_patch(contents["A"], contents["B"])
    upstream_edits = create_patch(contents["A"], contents["C"])
    if patches_interact(checkpoint_edits, upstream_edits):
        return None
    checkpoint_on_current = translate_patch(checkpoint_edits, upstream_edits)
    upstream_on_checkpoint = translate_patch(upstream_edits, checkpoint_edits)
    if apply_edits(contents["C"], checkpoint_on_current) != contents["D"]:
        return None
    if apply_edits(contents["B"], upstream_on_checkpoint) != contents["D"]:
        return None
    combined = sorted([*checkpoint_edits, *upstream_edits], key=lambda edit: (edit["start"], edit["end"]))
    if apply_edits(contents["A"], combined) != contents["D"]:
        return None
    path = checkpoint["before_path"]
    path_utf8 = path.decode("utf-8")
    return {
        "algorithm": "independent_python_sequence_matcher_v1",
        "path_base64": path_base64(path),
        "path_utf8": path_utf8,
        "regular_mode": checkpoint["before_mode"],
        "snapshots": {
            label: snapshot_evidence(object_ids[label], contents[label])
            for label in ("A", "B", "C", "D")
        },
        "checkpoint_edits_A_to_B": [witness_edit(edit) for edit in checkpoint_edits],
        "upstream_edits_A_to_C": [witness_edit(edit) for edit in upstream_edits],
        "verified_relations": ["A_to_B", "A_to_C", "C_plus_checkpoint_to_D", "B_plus_upstream_to_D"],
        "snapshot_commits": snapshots,
    }


def display_path(change):
    path = change["after_path"] if change["after_path"] is not None else change["before_path"]
    require(path is not None, "Git change has no path")
    return path


def all_raw_paths(changes):
    paths = set()
    for change in changes:
        if change["before_path"] is not None:
            paths.add(change["before_path"])
        if change["after_path"] is not None:
            paths.add(change["after_path"])
    return paths


def path_set_sha256(paths):
    encoded = [base64.b64encode(path).decode("ascii") for path in sorted(paths)]
    return sha256_bytes(canonical_json_bytes(encoded))


def canonical_identity(identity_value):
    return json.dumps(identity_value, ensure_ascii=True, sort_keys=True, separators=(",", ":"))


def generate_case_oracle(manifest, case, repository):
    repository = Path(repository).resolve()
    require(repository.is_dir(), f"repository does not exist: {repository}")
    is_bare = git_stdout(repository, ["rev-parse", "--is-bare-repository"]).decode("ascii").strip()
    require(is_bare == "true", f"repository is not bare: {repository}")
    shallow = git_stdout(repository, ["rev-parse", "--is-shallow-repository"]).decode("ascii").strip()
    require(shallow == "false", f"repository is shallow: {repository}")
    commits = {label: case["snapshots"][label]["commit"] for label in ("Q", "A", "B", "C", "D")}
    for label, commit in commits.items():
        require(resolve_commit(repository, commit) == commit, f"{case['id']} snapshot {label} did not resolve exactly")
    require(unique_merge_base(repository, commits["Q"], commits["B"]) == commits["A"], f"{case['id']} A is not merge-base(Q,B)")
    require(unique_merge_base(repository, commits["Q"], commits["D"]) == commits["C"], f"{case['id']} C is not merge-base(Q,D)")
    require(not is_ancestor(repository, commits["B"], commits["D"]), f"{case['id']} B unexpectedly became an ancestor of D")
    observation = case["capture_observation"]
    last_force_after = case["force_push_chain"][-1]["after_commit"]
    if observation["captured_head_relation_to_last_force_after"] == "same_commit":
        require(last_force_after == commits["D"], f"{case['id']} final force-push commit differs from D")
    else:
        require(is_ancestor(repository, last_force_after, commits["D"]), f"{case['id']} last force-push commit is not an ancestor of D")
        ahead = int(git_stdout(repository, ["rev-list", "--count", f"{last_force_after}..{commits['D']}"]).decode("ascii").strip())
        require(ahead == observation["commits_ahead_of_last_force_after"], f"{case['id']} commits after final force-push differ")
    license_evidence = license_observation_evidence(case, repository, commits, read_content=True)

    checkpoint_raw, checkpoint_raw_changes, checkpoint_changes = raw_diff(repository, commits["A"], commits["B"])
    current_raw, current_raw_changes, current_changes = raw_diff(repository, commits["C"], commits["D"])
    naive_raw, naive_raw_changes, _ = raw_diff(repository, commits["B"], commits["D"])
    checkpoint_identity_values = [identity(change) for change in checkpoint_changes]
    current_identity_values = [identity(change) for change in current_changes]

    matched_checkpoint_indices = set()
    classifications = []
    witnesses = []
    exact_carries = 0
    replay_carries = 0
    for current in current_changes:
        exact_indices = [
            index
            for index, checkpoint in enumerate(checkpoint_changes)
            if identity_key(checkpoint) == identity_key(current)
        ]
        current_identity = identity(current)
        classification = {
            "path_base64": path_base64(display_path(current)),
            "path_utf8": display_path(current).decode("utf-8"),
            "current_identity_sha256": current_identity["identity_sha256"],
        }
        if exact_indices:
            matched_checkpoint_indices.update(exact_indices)
            classification["checkpoint_state"] = "unchanged_since_checkpoint"
            classification["checkpoint_match_basis"] = "exact_git_change_identity"
            exact_carries += 1
        else:
            candidate_indices = [
                index
                for index, checkpoint in enumerate(checkpoint_changes)
                if replay_candidate_metadata_matches(checkpoint, current)
            ]
            witness = None
            candidate_index = None
            if len(candidate_indices) == 1 and candidate_indices[0] not in matched_checkpoint_indices:
                candidate_index = candidate_indices[0]
                witness = four_way_replay_witness(
                    repository,
                    checkpoint_changes[candidate_index],
                    current,
                    commits,
                )
            if witness is not None:
                matched_checkpoint_indices.add(candidate_index)
                checkpoint_identity = identity(checkpoint_changes[candidate_index])
                classification["checkpoint_state"] = "unchanged_since_checkpoint"
                classification["checkpoint_match_basis"] = "exact_noninteracting_four_way_byte_replay"
                classification["checkpoint_identity_sha256"] = checkpoint_identity["identity_sha256"]
                witnesses.append(witness)
                replay_carries += 1
            else:
                classification["checkpoint_state"] = "needs_review_now"
        classifications.append(classification)

    classifications.sort(key=lambda item: item["path_base64"])
    witnesses.sort(key=lambda item: item["path_base64"])
    retired = [
        identity(change)
        for index, change in enumerate(checkpoint_changes)
        if index not in matched_checkpoint_indices
    ]
    retired.sort(key=canonical_identity)
    naive_paths = all_raw_paths(naive_raw_changes)
    current_paths = all_raw_paths(current_raw_changes)
    extra_paths = naive_paths - current_paths
    missing_paths = current_paths - naive_paths
    summary = {
        "current_pr_files": len(current_changes),
        "carried": exact_carries + replay_carries,
        "exactly_carried": exact_carries,
        "replay_carried": replay_carries,
        "needs_review_now": len(current_changes) - exact_carries - replay_carries,
        "retired_checkpoint_changes": len(retired),
        "naive_snapshot_paths": len(naive_paths),
        "naive_extra_paths": len(extra_paths),
        "naive_missing_current_paths": len(missing_paths),
    }
    require(summary == expected_summary(case), f"{case['id']} independently derived summary differs from manifest observation: {summary}")
    return {
        "schema": ORACLE_SCHEMA,
        "dataset_version": manifest["dataset_version"],
        "case_id": case["id"],
        "oracle_kind": "exact_policy_conformance",
        "human_priority_ground_truth": "absent",
        "snapshots": commits,
        "license_observation": license_evidence,
        "raw_diff_sha256": {
            "checkpoint_A_to_B": sha256_bytes(checkpoint_raw),
            "current_C_to_D": sha256_bytes(current_raw),
            "naive_B_to_D": sha256_bytes(naive_raw),
        },
        "checkpoint_identities": checkpoint_identity_values,
        "current_identities": current_identity_values,
        "classification": classifications,
        "replay_witnesses": witnesses,
        "retired_checkpoint_identities": retired,
        "naive_path_set": {
            "paths": len(naive_paths),
            "extra_paths": len(extra_paths),
            "missing_current_paths": len(missing_paths),
            "path_set_sha256": path_set_sha256(naive_paths),
            "extra_path_set_sha256": path_set_sha256(extra_paths),
            "missing_current_path_set_sha256": path_set_sha256(missing_paths),
        },
        "summary": summary,
    }


def oracle_path(manifest_path, case):
    return manifest_path.parent / case["expectation"]["oracle"]


def repository_arguments(values):
    repositories = {}
    for value in values:
        case_id, separator, raw_path = value.partition("=")
        require(separator == "=" and case_id and raw_path, f"invalid --repository value: {value}")
        require(case_id not in repositories, f"duplicate repository mapping: {case_id}")
        repositories[case_id] = Path(raw_path).resolve()
    return repositories


def repositories_from_materialization(manifest, manifest_path, materialization):
    root = Path(materialization).resolve()
    metadata = load_json(root / "materialization.json")
    require(metadata["schema"] == MATERIALIZATION_SCHEMA, "unsupported materialization schema")
    require(metadata["dataset_version"] == manifest["dataset_version"], "materialization version differs")
    require(metadata["manifest_sha256"] == sha256_bytes(manifest_path.read_bytes()), "materialization manifest digest differs")
    require(metadata["offline_ready"] is True, "materialization is not offline-ready")
    repositories = {}
    for case in manifest["cases"]:
        relative = Path(metadata["repositories"][case["id"]])
        require(not relative.is_absolute() and ".." not in relative.parts, f"materialized repository path escapes its root: {relative}")
        repository = root / relative
        object_ids = required_review_delta_blob_ids(case, repository)
        verify_required_review_delta_blobs(case, repository, object_ids, LOCAL_COMMAND_TIMEOUT_SECONDS)
        repositories[case["id"]] = repository
    return repositories


def resolve_repositories(manifest, manifest_path, repository_values, materialization):
    if materialization is not None:
        repositories = repositories_from_materialization(manifest, manifest_path, materialization)
    else:
        require(repository_values is not None, "repository mappings are required")
        repositories = repository_arguments(repository_values)
    expected = {case["id"] for case in manifest["cases"]}
    require(set(repositories) == expected, f"repository mappings differ: expected {sorted(expected)}")
    return repositories


def generate_all(manifest_path, repositories):
    manifest = load_manifest(manifest_path)
    paths = []
    for case in manifest["cases"]:
        value = generate_case_oracle(manifest, case, repositories[case["id"]])
        path = oracle_path(manifest_path, case)
        write_json(path, value)
        paths.append(path)
    return paths


def verify_all(manifest_path, repositories):
    manifest = load_manifest(manifest_path)
    for case in manifest["cases"]:
        expected = load_json(oracle_path(manifest_path, case))
        actual = generate_case_oracle(manifest, case, repositories[case["id"]])
        require(actual == expected, f"oracle drift: {case['id']}")
    return len(manifest["cases"])


def decode_product_path(file, side):
    path_field = f"{side}_path"
    encoding_field = f"{side}_path_encoding"
    if path_field not in file:
        return None
    display = file[path_field]
    encoding = file[encoding_field]
    if encoding == "utf8":
        return display.encode("utf-8")
    require(encoding == "git_bytes_percent_encoded" and display.startswith("git-bytes:"), f"unsupported product path encoding: {encoding}")
    encoded = display[len("git-bytes:") :]
    require(len(encoded) % 3 == 0, "malformed percent-encoded Git path")
    chunks = [encoded[index : index + 3] for index in range(0, len(encoded), 3)]
    require(all(len(chunk) == 3 and chunk[0] == "%" for chunk in chunks), "malformed percent-encoded Git path")
    return bytes(int(chunk[1:], 16) for chunk in chunks)


def product_optional(file, field):
    return file[field] if field in file else None


def product_identity(file):
    change = {
        "status": STATUS_NAMES[file["status"]],
        "similarity_percent": product_optional(file, "similarity_percent"),
        "before_path": decode_product_path(file, "before"),
        "after_path": decode_product_path(file, "after"),
        "before_mode": product_optional(file, "before_mode"),
        "after_mode": product_optional(file, "after_mode"),
        "before_object_id": product_optional(file, "before_blob"),
        "after_object_id": product_optional(file, "after_blob"),
    }
    return identity(change)


def run_binary(binary, arguments):
    result = subprocess.run(
        [str(binary), *arguments],
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=LOCAL_COMMAND_TIMEOUT_SECONDS,
        check=False,
    )
    if result.returncode != 0:
        diagnostic = result.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"stratadiff {' '.join(arguments)} failed: {diagnostic}")
    require(not result.stderr, f"stratadiff {' '.join(arguments)} produced diagnostics")
    return result.stdout


def read_build_info(binary):
    info = json.loads(run_binary(binary, ["build-info"]), object_pairs_hook=unique_json_object)
    require(info["schema"] == BUILD_INFO_SCHEMA, "unsupported build-info schema")
    require(info["git_dirty"] is False, "evaluation requires a clean StrataDiff binary")
    require(info["build_profile"] == "release", "evaluation requires a release StrataDiff binary")
    validate_oid(info["git_revision"], "StrataDiff build revision")
    require(len(info["cargo_lock_sha256"]) == 64, "invalid Cargo.lock digest")
    require(info["rustc_version"].startswith("rustc "), "invalid rustc provenance")
    return info


def verifier_runtime():
    result = subprocess.run(
        ["git", "--version"],
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=LOCAL_COMMAND_TIMEOUT_SECONDS,
        check=True,
    )
    require(not result.stderr, "git --version produced diagnostics")
    git_version = result.stdout.decode("utf-8").strip()
    require(git_version.startswith("git version "), "unexpected Git version output")
    return {"python_version": platform.python_version(), "git_version": git_version}


def expected_classifications(oracle):
    output = {}
    for item in oracle["classification"]:
        identity_sha256 = item["current_identity_sha256"]
        require(identity_sha256 not in output, f"duplicate oracle classification: {identity_sha256}")
        output[identity_sha256] = item
    return output


def evaluate_case(case, oracle, repository, binary):
    snapshots = oracle["snapshots"]
    report = json.loads(
        run_binary(
            binary,
            [
                "review",
                "--repo",
                str(Path(repository).resolve()),
                "--checkpoint",
                snapshots["B"],
                "--format",
                "json",
                "--",
                snapshots["Q"],
                snapshots["D"],
            ],
        ),
        object_pairs_hook=unique_json_object,
    )
    require(report["requested_base"] == snapshots["Q"], f"{case['id']} product Q differs")
    require(report["base_commit"] == snapshots["C"], f"{case['id']} product C differs")
    require(report["requested_head"] == snapshots["D"], f"{case['id']} product requested D differs")
    require(report["head_commit"] == snapshots["D"], f"{case['id']} product D differs")
    require(report["checkpoint"]["commit"] == snapshots["B"], f"{case['id']} product B differs")
    require(report["checkpoint"]["base_commit"] == snapshots["A"], f"{case['id']} product A differs")
    expected_policy = (
        "exact_git_change_identity"
        if snapshots["A"] == snapshots["C"]
        else REPORT_MATCH_BASIS
    )
    require(report["checkpoint"]["match_basis"] == expected_policy, f"{case['id']} product policy differs")

    expected = expected_classifications(oracle)
    observed = {}
    duplicates = []
    for file in report["files"]:
        value = product_identity(file)
        identity_sha256 = value["identity_sha256"]
        if identity_sha256 in observed:
            duplicates.append(identity_sha256)
        observed[identity_sha256] = file
    expected_ids = set(expected)
    observed_ids = set(observed)
    false_carry = []
    false_invalidation = []
    basis_mismatches = []
    for identity_sha256 in sorted(expected_ids & observed_ids):
        expected_file = expected[identity_sha256]
        observed_file = observed[identity_sha256]
        expected_state = expected_file["checkpoint_state"]
        observed_state = observed_file["checkpoint_state"]
        require(
            expected_state in ("needs_review_now", "unchanged_since_checkpoint"),
            f"{case['id']} oracle checkpoint state is unsupported: {expected_state}",
        )
        require(
            observed_state in ("needs_review_now", "unchanged_since_checkpoint"),
            f"{case['id']} product checkpoint state is unsupported: {observed_state}",
        )
        if expected_state == "needs_review_now" and observed_state == "unchanged_since_checkpoint":
            false_carry.append(identity_sha256)
        elif expected_state == "unchanged_since_checkpoint" and observed_state == "needs_review_now":
            false_invalidation.append(identity_sha256)
        if expected_state == "unchanged_since_checkpoint" and observed_state == "unchanged_since_checkpoint":
            require("checkpoint_match_basis" in expected_file, "carried oracle file has no basis")
            if "checkpoint_match_basis" not in observed_file or observed_file["checkpoint_match_basis"] != expected_file["checkpoint_match_basis"]:
                basis_mismatches.append(identity_sha256)
        if expected_state == "needs_review_now" and "checkpoint_match_basis" in observed_file:
            basis_mismatches.append(identity_sha256)
    omissions = sorted(expected_ids - observed_ids)
    extras = sorted(observed_ids - expected_ids)
    checkpoint_summary = report["summary"]["checkpoint"]
    summary_mismatch = (
        report["summary"]["changed_files"] != oracle["summary"]["current_pr_files"]
        or checkpoint_summary["unchanged_since_checkpoint_files"] != oracle["summary"]["carried"]
        or checkpoint_summary["needs_review_now_files"] != oracle["summary"]["needs_review_now"]
        or checkpoint_summary["retired_change_count"] != oracle["summary"]["retired_checkpoint_changes"]
    )
    passed = not false_carry and not false_invalidation and not basis_mismatches and not omissions and not extras and not duplicates and not summary_mismatch
    return {
        "id": case["id"],
        "passed": passed,
        "summary": oracle["summary"],
        "false_carry": false_carry,
        "false_invalidation": false_invalidation,
        "basis_mismatches": basis_mismatches,
        "identity_omissions": omissions,
        "identity_extras": extras,
        "duplicate_product_identities": sorted(duplicates),
        "summary_mismatch": summary_mismatch,
    }


def evaluate_all(manifest_path, repositories, binary):
    manifest = load_manifest(manifest_path)
    binary = Path(binary).resolve()
    require(binary.is_file(), f"StrataDiff binary does not exist: {binary}")
    engine = read_build_info(binary)
    cases = []
    oracle_digests = {}
    for case in manifest["cases"]:
        path = oracle_path(manifest_path, case)
        oracle = load_json(path)
        actual = generate_case_oracle(manifest, case, repositories[case["id"]])
        require(actual == oracle, f"oracle drift before evaluation: {case['id']}")
        cases.append(evaluate_case(case, oracle, repositories[case["id"]], binary))
        oracle_digests[case["id"]] = sha256_bytes(path.read_bytes())
    summary = {
        "cases": len(cases),
        "passed_cases": sum(1 for case in cases if case["passed"]),
        "current_pr_files": sum(case["summary"]["current_pr_files"] for case in cases),
        "carried": sum(case["summary"]["carried"] for case in cases),
        "exactly_carried": sum(case["summary"]["exactly_carried"] for case in cases),
        "replay_carried": sum(case["summary"]["replay_carried"] for case in cases),
        "needs_review_now": sum(case["summary"]["needs_review_now"] for case in cases),
        "retired_checkpoint_changes": sum(case["summary"]["retired_checkpoint_changes"] for case in cases),
        "false_carry": sum(len(case["false_carry"]) for case in cases),
        "false_invalidation": sum(len(case["false_invalidation"]) for case in cases),
        "basis_mismatches": sum(len(case["basis_mismatches"]) for case in cases),
        "identity_omissions": sum(len(case["identity_omissions"]) for case in cases),
        "identity_extras": sum(len(case["identity_extras"]) for case in cases),
    }
    benchmark_complete = summary["passed_cases"] == summary["cases"]
    evaluation = {
        "schema": EVALUATION_SCHEMA,
        "dataset_version": manifest["dataset_version"],
        "evaluated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "benchmark_complete": benchmark_complete,
        "claim_boundary": EVALUATION_CLAIM_BOUNDARY,
        "provenance": {
            "manifest_sha256": sha256_bytes(manifest_path.read_bytes()),
            "oracle_sha256": oracle_digests,
            "verifier_sha256": sha256_bytes(Path(__file__).read_bytes()),
            "stratadiff_binary_sha256": sha256_bytes(binary.read_bytes()),
            "engine": engine,
            "verifier_runtime": verifier_runtime(),
        },
        "summary": summary,
        "cases": cases,
    }
    require(evaluation["benchmark_complete"], "StrataDiff evaluation is incomplete")
    return evaluation


def identity_hashes(values, label):
    hashes = [value["identity_sha256"] for value in values]
    require(len(hashes) == len(set(hashes)), f"duplicate identities in {label}")
    return set(hashes)


def verify_oracle_structure(case, oracle):
    case_id = case["id"]
    summary = oracle["summary"]
    require(summary == expected_summary(case), f"oracle summary differs from manifest: {case_id}")
    require(
        summary["current_pr_files"]
        == summary["exactly_carried"] + summary["replay_carried"] + summary["needs_review_now"],
        f"oracle current partition differs: {case_id}",
    )
    require(
        summary["carried"] == summary["exactly_carried"] + summary["replay_carried"],
        f"oracle carried partition differs: {case_id}",
    )

    current_ids = identity_hashes(oracle["current_identities"], f"{case_id} current identities")
    checkpoint_ids = identity_hashes(oracle["checkpoint_identities"], f"{case_id} checkpoint identities")
    retired_ids = identity_hashes(oracle["retired_checkpoint_identities"], f"{case_id} retired identities")
    require(len(current_ids) == summary["current_pr_files"], f"oracle current identity count differs: {case_id}")
    require(len(retired_ids) == summary["retired_checkpoint_changes"], f"oracle retired count differs: {case_id}")

    classified_ids = set()
    classified_paths = set()
    exact_checkpoint_ids = set()
    replay_checkpoint_ids = set()
    exact_paths = set()
    replay_paths = set()
    residue_paths = set()
    for classification in oracle["classification"]:
        identity_sha256 = classification["current_identity_sha256"]
        path = classification["path_base64"]
        require(identity_sha256 not in classified_ids, f"duplicate oracle classification: {case_id}")
        require(path not in classified_paths, f"duplicate oracle classification path: {case_id}")
        classified_ids.add(identity_sha256)
        classified_paths.add(path)
        state = classification["checkpoint_state"]
        if state == "needs_review_now":
            require("checkpoint_match_basis" not in classification, f"residue has a carry basis: {case_id}")
            require("checkpoint_identity_sha256" not in classification, f"residue has a checkpoint identity: {case_id}")
            residue_paths.add(path)
            continue
        require(state == "unchanged_since_checkpoint", f"unsupported oracle state: {case_id}")
        basis = classification["checkpoint_match_basis"]
        if basis == "exact_git_change_identity":
            require("checkpoint_identity_sha256" not in classification, f"exact carry has redundant identity: {case_id}")
            exact_checkpoint_ids.add(identity_sha256)
            exact_paths.add(path)
        else:
            require(basis == "exact_noninteracting_four_way_byte_replay", f"unsupported oracle carry basis: {case_id}")
            replay_checkpoint_id = classification["checkpoint_identity_sha256"]
            require(replay_checkpoint_id not in replay_checkpoint_ids, f"duplicate replay checkpoint identity: {case_id}")
            replay_checkpoint_ids.add(replay_checkpoint_id)
            replay_paths.add(path)
    require(classified_ids == current_ids, f"oracle classification coverage differs: {case_id}")
    require(len(exact_paths) == summary["exactly_carried"], f"oracle exact count differs: {case_id}")
    require(len(replay_paths) == summary["replay_carried"], f"oracle replay count differs: {case_id}")
    require(len(residue_paths) == summary["needs_review_now"], f"oracle residue count differs: {case_id}")
    require(
        exact_checkpoint_ids | replay_checkpoint_ids | retired_ids == checkpoint_ids,
        f"oracle checkpoint identity coverage differs: {case_id}",
    )
    require(
        not (exact_checkpoint_ids & replay_checkpoint_ids)
        and not (exact_checkpoint_ids & retired_ids)
        and not (replay_checkpoint_ids & retired_ids),
        f"oracle checkpoint identity partitions overlap: {case_id}",
    )

    witness_paths = set()
    for witness in oracle["replay_witnesses"]:
        path = witness["path_base64"]
        require(path not in witness_paths, f"duplicate replay witness: {case_id}")
        require(witness["snapshot_commits"] == oracle["snapshots"], f"replay witness snapshots differ: {case_id}")
        witness_paths.add(path)
    require(witness_paths == replay_paths, f"replay witness coverage differs: {case_id}")

    naive = oracle["naive_path_set"]
    require(naive["paths"] == summary["naive_snapshot_paths"], f"oracle naive count differs: {case_id}")
    require(naive["extra_paths"] == summary["naive_extra_paths"], f"oracle naive extra count differs: {case_id}")
    require(naive["missing_current_paths"] == summary["naive_missing_current_paths"], f"oracle naive missing count differs: {case_id}")

    observed_license = oracle["license_observation"]
    manifest_license = case["repository"]["license_observation"]
    for field in ("source", "spdx_id", "path", "blob_oid", "content_sha256"):
        require(observed_license[field] == manifest_license[field], f"oracle license observation differs: {case_id} {field}")
    labels = ("Q", "A", "B", "C", "D")
    present = observed_license["present_snapshots"]
    require(present and len(present) == len(set(present)), f"oracle license snapshot coverage is empty or duplicated: {case_id}")
    require(present == [label for label in labels if label in present], f"oracle license snapshot order differs: {case_id}")
    return summary


def bundle_paths(manifest_path, manifest):
    paths = {
        manifest_path.parent / "README.md",
        manifest_path,
        manifest_path.parent / "evaluation-v1.0.0.json",
    }
    for case in manifest["cases"]:
        paths.add(oracle_path(manifest_path, case))
    return sorted(paths, key=lambda path: path.relative_to(manifest_path.parent).as_posix())


def write_checksums(manifest_path):
    manifest = load_manifest(manifest_path)
    lines = []
    for path in bundle_paths(manifest_path, manifest):
        require(path.is_file(), f"checksum target is missing: {path}")
        relative = path.relative_to(manifest_path.parent).as_posix()
        lines.append(f"{sha256_bytes(path.read_bytes())}  {relative}")
    checksum_path = manifest_path.parent / "SHA256SUMS"
    checksum_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return checksum_path


def verify_bundle(manifest_path):
    manifest = load_manifest(manifest_path)
    expected_paths = {
        path.relative_to(manifest_path.parent).as_posix()
        for path in bundle_paths(manifest_path, manifest)
    }
    checksum_path = manifest_path.parent / "SHA256SUMS"
    observed_bundle_files = {
        path.relative_to(manifest_path.parent).as_posix()
        for path in manifest_path.parent.rglob("*")
        if path.is_file()
    }
    require(observed_bundle_files == expected_paths | {"SHA256SUMS"}, "bundle file set differs")
    observed_paths = set()
    for line in checksum_path.read_text(encoding="utf-8").splitlines():
        digest, separator, relative = line.partition("  ")
        require(separator == "  ", "invalid SHA256SUMS line")
        require(relative in expected_paths, f"unexpected checksum target: {relative}")
        require(relative not in observed_paths, f"duplicate checksum target: {relative}")
        require(
            len(digest) == 64 and all(character in "0123456789abcdef" for character in digest),
            f"invalid checksum: {relative}",
        )
        require(sha256_bytes((manifest_path.parent / relative).read_bytes()) == digest, f"checksum mismatch: {relative}")
        observed_paths.add(relative)
    require(observed_paths == expected_paths, "checksum coverage differs")

    oracle_digests = {}
    oracle_by_id = {}
    for case in manifest["cases"]:
        path = oracle_path(manifest_path, case)
        oracle = load_json(path)
        require(oracle["schema"] == ORACLE_SCHEMA, f"unsupported oracle schema: {case['id']}")
        require(oracle["dataset_version"] == manifest["dataset_version"], f"oracle version differs: {case['id']}")
        require(oracle["case_id"] == case["id"], f"oracle case ID differs: {case['id']}")
        require(oracle["human_priority_ground_truth"] == "absent", f"oracle claim boundary differs: {case['id']}")
        for label in ("Q", "A", "B", "C", "D"):
            require(oracle["snapshots"][label] == case["snapshots"][label]["commit"], f"oracle snapshot differs: {case['id']} {label}")
        verify_oracle_structure(case, oracle)
        oracle_by_id[case["id"]] = oracle
        oracle_digests[case["id"]] = sha256_bytes(path.read_bytes())

    evaluation_path = manifest_path.parent / "evaluation-v1.0.0.json"
    evaluation = load_json(evaluation_path)
    require(evaluation["schema"] == EVALUATION_SCHEMA, "unsupported evaluation schema")
    require(evaluation["dataset_version"] == manifest["dataset_version"], "evaluation version differs")
    require(evaluation["benchmark_complete"] is True, "frozen evaluation is incomplete")
    require(evaluation["claim_boundary"] == EVALUATION_CLAIM_BOUNDARY, "evaluation claim boundary differs")
    evaluated_at = parse_time(evaluation["evaluated_at"])
    require(evaluated_at.utcoffset() is not None, "evaluation timestamp has no timezone")
    require(evaluated_at >= parse_time(manifest["captured_at"]), "evaluation predates capture")
    require(evaluation["provenance"]["manifest_sha256"] == sha256_bytes(manifest_path.read_bytes()), "evaluation manifest digest differs")
    require(evaluation["provenance"]["oracle_sha256"] == oracle_digests, "evaluation oracle digests differ")
    require(evaluation["provenance"]["verifier_sha256"] == sha256_bytes(Path(__file__).read_bytes()), "evaluation verifier digest differs")
    require(
        len(evaluation["provenance"]["stratadiff_binary_sha256"]) == 64
        and all(character in "0123456789abcdef" for character in evaluation["provenance"]["stratadiff_binary_sha256"]),
        "evaluation binary digest is invalid",
    )
    producer = manifest["evaluation_producer"]
    engine = evaluation["provenance"]["engine"]
    for field in ("schema", "engine_version", "git_revision", "git_dirty", "cargo_lock_sha256", "build_profile", "rustc_version"):
        require(engine[field] == producer[field], f"evaluation engine provenance differs: {field}")
    runtime = evaluation["provenance"]["verifier_runtime"]
    require(runtime["python_version"], "evaluation Python version is empty")
    require(runtime["git_version"].startswith("git version "), "evaluation Git version is invalid")

    case_ids = [case["id"] for case in manifest["cases"]]
    result_ids = [result["id"] for result in evaluation["cases"]]
    require(result_ids == case_ids, "evaluation case order or coverage differs")
    for result in evaluation["cases"]:
        case_id = result["id"]
        require(result["passed"] is True, f"evaluation case failed: {case_id}")
        require(result["summary"] == oracle_by_id[case_id]["summary"], f"evaluation case summary differs: {case_id}")
        for field in (
            "false_carry",
            "false_invalidation",
            "basis_mismatches",
            "identity_omissions",
            "identity_extras",
            "duplicate_product_identities",
        ):
            require(result[field] == [], f"evaluation case contains {field}: {case_id}")
        require(result["summary_mismatch"] is False, f"evaluation case summary mismatch: {case_id}")

    summaries = [oracle_by_id[case_id]["summary"] for case_id in case_ids]
    expected_evaluation_summary = {
        "cases": len(case_ids),
        "passed_cases": len(case_ids),
        "current_pr_files": sum(summary["current_pr_files"] for summary in summaries),
        "carried": sum(summary["carried"] for summary in summaries),
        "exactly_carried": sum(summary["exactly_carried"] for summary in summaries),
        "replay_carried": sum(summary["replay_carried"] for summary in summaries),
        "needs_review_now": sum(summary["needs_review_now"] for summary in summaries),
        "retired_checkpoint_changes": sum(summary["retired_checkpoint_changes"] for summary in summaries),
        "false_carry": 0,
        "false_invalidation": 0,
        "basis_mismatches": 0,
        "identity_omissions": 0,
        "identity_extras": 0,
    }
    require(evaluation["summary"] == expected_evaluation_summary, "evaluation aggregate summary differs")
    return len(manifest["cases"])


def change_blob_ids(changes):
    object_ids = set()
    for change in changes:
        for object_field, mode_field in (
            ("before_object_id", "before_mode"),
            ("after_object_id", "after_mode"),
        ):
            if change[object_field] is not None and change[mode_field] != "160000":
                object_ids.add(change[object_field])
    return object_ids


def review_delta_snapshot_blob_ids(repository, commits, checkpoint_changes):
    paths = {
        path
        for change in checkpoint_changes
        for path in (change["before_path"], change["after_path"])
        if path is not None
    }
    object_ids = set()
    for raw_path in paths:
        try:
            path = raw_path.decode("utf-8")
        except UnicodeDecodeError:
            # StrataDiff records non-UTF-8 retired paths as unresolved instead of
            # claiming that their source can be displayed.
            continue
        for label in ("A", "B", "C", "D"):
            entry = tree_entry(repository, commits[label], path)
            if entry is not None and entry[1] == "blob":
                object_ids.add(entry[2])
    return object_ids


def required_review_delta_blob_ids(case, repository):
    commits = {label: case["snapshots"][label]["commit"] for label in ("Q", "A", "B", "C", "D")}
    checkpoint = raw_diff(repository, commits["A"], commits["B"])[2]
    current = raw_diff(repository, commits["C"], commits["D"])[2]
    object_ids = change_blob_ids([*checkpoint, *current])
    object_ids.update(review_delta_snapshot_blob_ids(repository, commits, checkpoint))
    license_evidence = license_observation_evidence(case, repository, commits, read_content=False)
    object_ids.add(license_evidence["blob_oid"])
    return object_ids


def verify_required_review_delta_blobs(case, repository, object_ids, timeout):
    sorted_ids = sorted(object_ids)
    request = "".join(f"{object_id}\n" for object_id in sorted_ids).encode("ascii")
    output = git_stdout(
        repository,
        ["cat-file", "--batch-check=%(objectname) %(objecttype) %(objectsize)"],
        input_bytes=request,
        timeout=timeout,
    ).decode("ascii").splitlines()
    require(len(output) == len(object_ids), f"{case['id']} review-delta blob closure count differs")
    for line, expected in zip(output, sorted_ids):
        columns = line.split()
        require(
            len(columns) == 3 and columns[0] == expected and columns[1] == "blob",
            f"{case['id']} is not offline-ready; review-delta blob is unavailable: {expected}",
        )


def hydrate_required_blobs(manifest, case, repository, token, timeout):
    object_ids = required_review_delta_blob_ids(case, repository)
    sorted_ids = sorted(object_ids)
    scratch = Path(tempfile.mkdtemp(prefix=".resumebench-blobs-", dir=repository.parent))
    try:
        subprocess.run(
            ["git", "init", "--bare", "--quiet", str(scratch)],
            env=isolated_environment(),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=LOCAL_COMMAND_TIMEOUT_SECONDS,
            check=True,
        )
        run_git(scratch, ["remote", "add", "origin", case["repository"]["git_url"]])
        refspecs = [f"+{object_id}:refs/resumebench/blobs/{object_id}" for object_id in sorted_ids]
        fetch = run_git(
            scratch,
            ["fetch", "--quiet", "--force", "--no-tags", "origin", *refspecs],
            allow_lazy_fetch=True,
            token=token,
            timeout=timeout,
        )
        require(not fetch.stdout and not fetch.stderr, f"{case['id']} blob fetch produced output")
        run_git(
            repository,
            ["fetch", "--quiet", "--force", "--no-tags", str(scratch), "+refs/resumebench/blobs/*:refs/resumebench/blobs/*"],
            timeout=timeout,
        )
        deletion = "".join(f"delete refs/resumebench/blobs/{object_id}\n" for object_id in sorted_ids).encode("ascii")
        run_git(repository, ["update-ref", "--stdin"], input_bytes=deletion)
    finally:
        shutil.rmtree(scratch)
    verify_required_review_delta_blobs(case, repository, object_ids, timeout)
    return object_ids


def materialize_case(manifest, case, repository, token, timeout):
    subprocess.run(
        ["git", "init", "--bare", "--quiet", str(repository)],
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=LOCAL_COMMAND_TIMEOUT_SECONDS,
        check=True,
    )
    run_git(repository, ["remote", "add", "origin", case["repository"]["git_url"]])
    run_git(repository, ["config", "extensions.partialClone", "origin"])
    run_git(repository, ["config", "remote.origin.promisor", "true"])
    run_git(repository, ["config", "remote.origin.partialclonefilter", "blob:none"])
    snapshots = case["snapshots"]
    refspecs = [
        f"+{snapshots[label]['commit']}:refs/resumebench/{label}"
        for label in ("Q", "B", "D")
    ]
    fetch = run_git(
        repository,
        ["fetch", "--quiet", "--force", "--no-tags", "--filter=blob:none", "origin", *refspecs],
        allow_lazy_fetch=True,
        token=token,
        timeout=timeout,
    )
    require(not fetch.stdout, f"{case['id']} exact-SHA fetch produced output")
    for label in ("Q", "B", "D"):
        expected = snapshots[label]["commit"]
        resolved = resolve_commit(repository, f"refs/resumebench/{label}")
        require(resolved == expected, f"{case['id']} exact-SHA fetch resolved a different {label}")
    a_commit = unique_merge_base(repository, snapshots["Q"]["commit"], snapshots["B"]["commit"])
    c_commit = unique_merge_base(repository, snapshots["Q"]["commit"], snapshots["D"]["commit"])
    require(a_commit == snapshots["A"]["commit"], f"{case['id']} materialized A differs")
    require(c_commit == snapshots["C"]["commit"], f"{case['id']} materialized C differs")
    run_git(repository, ["update-ref", "refs/resumebench/A", a_commit])
    run_git(repository, ["update-ref", "refs/resumebench/C", c_commit])
    run_git(repository, ["symbolic-ref", "HEAD", "refs/resumebench/D"])
    object_ids = hydrate_required_blobs(manifest, case, repository, token, timeout)
    generate_case_oracle(manifest, case, repository)
    run_git(repository, ["remote", "remove", "origin"])
    unset = run_git(repository, ["config", "--unset-all", "extensions.partialClone"], check=False)
    require(unset.returncode in (0, 5), f"{case['id']} failed to clear partial clone owner")
    require(not git_stdout(repository, ["remote"]), f"{case['id']} materialization retained a remote")
    verify_required_review_delta_blobs(case, repository, object_ids, timeout)
    generate_case_oracle(manifest, case, repository)


def materialize(manifest_path, output, token, timeout):
    manifest = load_manifest(manifest_path)
    output = Path(output).resolve()
    require(not output.exists(), f"materialization output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=f".{output.name}.tmp-", dir=output.parent))
    completed = False
    try:
        repositories = {}
        for case in manifest["cases"]:
            relative = Path("repositories") / f"{case['id']}.git"
            repository = stage / relative
            repository.parent.mkdir(parents=True, exist_ok=True)
            materialize_case(manifest, case, repository, token, timeout)
            repositories[case["id"]] = relative.as_posix()
        write_json(
            stage / "materialization.json",
            {
                "schema": MATERIALIZATION_SCHEMA,
                "dataset_version": manifest["dataset_version"],
                "manifest_sha256": sha256_bytes(manifest_path.read_bytes()),
                "offline_ready": True,
                "repositories": repositories,
            },
        )
        stage.replace(output)
        completed = True
    finally:
        if not completed and stage.exists():
            shutil.rmtree(stage)
    return output


def self_test():
    base = b"alpha\nbeta\ngamma\n"
    reviewed = b"alpha\nBETA\ngamma\n"
    upstream = b"prefix\nalpha\nbeta\ngamma\n"
    final = b"prefix\nalpha\nBETA\ngamma\n"
    reviewed_patch = create_patch(base, reviewed)
    upstream_patch = create_patch(base, upstream)
    require(not patches_interact(reviewed_patch, upstream_patch), "self-test patches interact")
    require(apply_edits(upstream, translate_patch(reviewed_patch, upstream_patch)) == final, "self-test reviewed replay failed")
    require(apply_edits(reviewed, translate_patch(upstream_patch, reviewed_patch)) == final, "self-test upstream replay failed")
    adjacent_left = [make_edit(b"abc", 1, 1, b"x")]
    adjacent_right = [make_edit(b"abc", 1, 2, b"B")]
    require(patches_interact(adjacent_left, adjacent_right), "self-test accepted adjacent edits")
    require(path_set_sha256({b"b", b"a"}) == path_set_sha256({b"a", b"b"}), "self-test path digest is unstable")
    test_token = "self-test-token"
    expected_header = "Authorization: Basic eC1hY2Nlc3MtdG9rZW46c2VsZi10ZXN0LXRva2Vu"
    require(github_git_authorization(test_token) == expected_header, "self-test GitHub Git authorization differs")
    environment = isolated_environment(token=test_token)
    require(environment["GIT_CONFIG_COUNT"] == "2", "self-test GitHub Git config count differs")
    require(environment["GIT_CONFIG_KEY_0"] == "http.https://github.com/.extraHeader", "self-test GitHub Git authorization is not URL-scoped")
    require(environment["GIT_CONFIG_VALUE_0"] == expected_header, "self-test GitHub Git header environment differs")
    require(environment["GIT_CONFIG_KEY_1"] == "http.followRedirects", "self-test GitHub Git redirect policy is absent")
    require(environment["GIT_CONFIG_VALUE_1"] == "false", "self-test GitHub Git redirects are not disabled")
    require(test_token not in environment["GIT_CONFIG_VALUE_0"], "self-test exposed the raw GitHub token")
    require("GH_TOKEN" not in environment and "GITHUB_TOKEN" not in environment, "self-test inherited a GitHub token variable")
    require(
        RejectRedirectHandler().redirect_request(None, None, 302, "Found", {}, "https://example.com/") is None,
        "self-test GitHub API redirects are not rejected",
    )

    with tempfile.TemporaryDirectory(prefix="stratadiff-github-live-self-test-") as temporary:
        repository = Path(temporary)
        run_git(repository, ["init", "--quiet"])
        run_git(repository, ["config", "user.name", "StrataDiff Self Test"])
        run_git(repository, ["config", "user.email", "stratadiff@example.test"])
        (repository / "retired.txt").write_bytes(b"base\n")
        run_git(repository, ["add", "retired.txt"])
        run_git(repository, ["commit", "--quiet", "-m", "base"])
        a_commit = resolve_commit(repository, "HEAD")

        (repository / "retired.txt").write_bytes(b"reviewed\n")
        run_git(repository, ["commit", "--quiet", "-am", "reviewed"])
        b_commit = resolve_commit(repository, "HEAD")

        run_git(repository, ["checkout", "--quiet", "--detach", a_commit])
        (repository / "retired.txt").write_bytes(b"upstream\n")
        run_git(repository, ["commit", "--quiet", "-am", "new base"])
        c_commit = resolve_commit(repository, "HEAD")
        (repository / "current.txt").write_bytes(b"current PR\n")
        run_git(repository, ["add", "current.txt"])
        run_git(repository, ["commit", "--quiet", "-m", "current head"])
        d_commit = resolve_commit(repository, "HEAD")

        checkpoint_changes = raw_diff(repository, a_commit, b_commit)[2]
        current_changes = raw_diff(repository, c_commit, d_commit)[2]
        diff_blob_ids = change_blob_ids([*checkpoint_changes, *current_changes])
        snapshot_blob_ids = review_delta_snapshot_blob_ids(
            repository,
            {"A": a_commit, "B": b_commit, "C": c_commit, "D": d_commit},
            checkpoint_changes,
        )
        retired_head_blob = tree_entry(repository, d_commit, "retired.txt")[2]
        require(retired_head_blob not in diff_blob_ids, "self-test fixture unexpectedly covered retired head blob")
        require(retired_head_blob in snapshot_blob_ids, "review-delta blob closure omitted retired head blob")

    return {"tests": 14, "passed": 14}


def add_manifest_argument(parser):
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)


def add_repository_source(parser):
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--repository", action="append", metavar="CASE_ID=BARE_REPOSITORY")
    group.add_argument("--materialization", type=Path)


def parse_arguments():
    parser = argparse.ArgumentParser(description="Build and verify ResumeBench-GitHub-Live v1")
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("self-test")

    bundle_parser = subparsers.add_parser("verify-bundle")
    add_manifest_argument(bundle_parser)

    provenance_parser = subparsers.add_parser("verify-provenance")
    add_manifest_argument(provenance_parser)
    provenance_parser.add_argument("--github-token-env", required=True)
    provenance_parser.add_argument("--api-timeout-seconds", type=int, default=60)

    materialize_parser = subparsers.add_parser("materialize")
    add_manifest_argument(materialize_parser)
    materialize_parser.add_argument("--output", type=Path, required=True)
    materialize_parser.add_argument("--github-token-env")
    materialize_parser.add_argument("--fetch-timeout-seconds", type=int, default=300)

    for command in ("generate", "verify"):
        command_parser = subparsers.add_parser(command)
        add_manifest_argument(command_parser)
        add_repository_source(command_parser)

    evaluate_parser = subparsers.add_parser("evaluate")
    add_manifest_argument(evaluate_parser)
    add_repository_source(evaluate_parser)
    evaluate_parser.add_argument("--stratadiff", type=Path, required=True)
    evaluate_parser.add_argument("--output", type=Path, required=True)

    freeze_parser = subparsers.add_parser("freeze")
    add_manifest_argument(freeze_parser)
    add_repository_source(freeze_parser)
    freeze_parser.add_argument("--stratadiff", type=Path, required=True)
    return parser.parse_args()


def main():
    arguments = parse_arguments()
    if arguments.command == "self-test":
        value = self_test()
    elif arguments.command == "verify-bundle":
        value = {"bundle_verified": True, "cases": verify_bundle(arguments.manifest.resolve())}
    elif arguments.command == "verify-provenance":
        token = os.environ[arguments.github_token_env]
        value = verify_provenance(arguments.manifest.resolve(), token, arguments.api_timeout_seconds)
    elif arguments.command == "materialize":
        require(arguments.fetch_timeout_seconds > 0, "fetch timeout must be positive")
        token = None
        if arguments.github_token_env is not None:
            token = os.environ[arguments.github_token_env]
        output = materialize(arguments.manifest.resolve(), arguments.output, token, arguments.fetch_timeout_seconds)
        value = {"cases": 5, "offline_ready": True, "output": str(output)}
    elif arguments.command in ("generate", "verify", "evaluate", "freeze"):
        manifest_path = arguments.manifest.resolve()
        manifest = load_manifest(manifest_path)
        repositories = resolve_repositories(
            manifest,
            manifest_path,
            arguments.repository,
            arguments.materialization,
        )
        if arguments.command == "generate":
            value = {"generated_oracles": len(generate_all(manifest_path, repositories))}
        elif arguments.command == "verify":
            value = {"verified_cases": verify_all(manifest_path, repositories)}
        elif arguments.command == "evaluate":
            evaluation = evaluate_all(manifest_path, repositories, arguments.stratadiff)
            write_json(arguments.output.resolve(), evaluation)
            value = {"benchmark_complete": evaluation["benchmark_complete"], "output": str(arguments.output.resolve())}
        else:
            generate_all(manifest_path, repositories)
            evaluation_path = manifest_path.parent / "evaluation-v1.0.0.json"
            evaluation = evaluate_all(manifest_path, repositories, arguments.stratadiff)
            write_json(evaluation_path, evaluation)
            checksum_path = write_checksums(manifest_path)
            verify_bundle(manifest_path)
            value = {
                "benchmark_complete": evaluation["benchmark_complete"],
                "cases": len(manifest["cases"]),
                "evaluation": str(evaluation_path),
                "checksums": str(checksum_path),
            }
    else:
        raise ValueError(f"unsupported command: {arguments.command}")
    print(json.dumps(value, ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main()
