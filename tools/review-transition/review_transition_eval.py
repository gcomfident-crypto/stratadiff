#!/usr/bin/env python3

import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import urllib.request

import review_transition as selector
import review_transition_materialize as materializer
import review_transition_oracle as oracle_runner


TOOL_ROOT = Path(__file__).resolve().parent
DEFAULT_PROTOCOL = TOOL_ROOT / "evaluation-protocol-v1.json"
EXPECTED_PROTOCOL_SHA256 = "b804e08ff34d69e4e4d7820abe35d9bb27b82165db6a5f5af3dbd2177c29c194"
OBSERVATION_SCHEMA = "stratadiff-review-transition-provider-observation-v1"
GITHUB_API_ROOT = "https://api.github.com"
GITHUB_API_VERSION = "2022-11-28"
BINDING_DOMAIN = "stratadiff-review-transition-provider-binding-v1"
MAX_GITHUB_RESPONSE_BYTES = 4 * 1024 * 1024
DEFAULT_API_TIMEOUT_SECONDS = 30


class RejectRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        return None


GITHUB_API_OPENER = urllib.request.build_opener(RejectRedirectHandler())


def require(condition, message):
    if not condition:
        raise ValueError(message)


def exact_fields(value, fields, label):
    require(isinstance(value, dict), f"{label} must be an object")
    require(set(value) == set(fields), f"{label} fields differ: {sorted(value)}")
    return value


def canonical_json_bytes(value):
    return selector.canonical_json_bytes(value)


def compact_json_bytes(value):
    return json.dumps(value, allow_nan=False, ensure_ascii=True, sort_keys=True, separators=(",", ":")).encode(
        "ascii"
    )


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def now_utc():
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def validate_timestamp(value, label):
    selector.validate_timestamp(value, label)


def validate_oid(value, label):
    require(selector.is_oid(value), f"{label} must be a full lowercase SHA-1 object ID")


def load_canonical_json(path, label):
    raw, value = selector.load_json(Path(path))
    require(raw == canonical_json_bytes(value), f"{label} is not canonical JSON")
    return raw, value


def write_json_atomic(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temporary.write_bytes(canonical_json_bytes(value))
    temporary.replace(path)


def load_verified_plan(plan_path):
    plan_path = Path(plan_path).resolve()
    selector.verify_plan(plan_path)
    raw, plan = load_canonical_json(plan_path, "transition plan")
    require(len(plan["selected_cases"]) == 30, "evaluation requires the frozen 30-case cohort")
    return raw, plan


def load_protocol(protocol_path=DEFAULT_PROTOCOL):
    raw, protocol = load_canonical_json(Path(protocol_path), "evaluation protocol")
    require(sha256_bytes(raw) == EXPECTED_PROTOCOL_SHA256, "evaluation protocol digest differs")
    require(
        protocol["schema"] == "stratadiff-review-transition-evaluation-protocol-v1",
        "unsupported evaluation protocol schema",
    )
    require(protocol["protocol_version"] == "1.0.0", "unsupported evaluation protocol version")
    require(protocol["cohort"]["replacement_policy"] == "forbidden", "protocol permits case replacement")
    return raw, protocol


def validate_plan_protocol(plan_raw, plan, protocol):
    cohort = protocol["cohort"]
    require(cohort["plan_schema"] == plan["schema"], "protocol plan schema differs")
    require(cohort["plan_dataset_version"] == plan["dataset_version"], "protocol plan version differs")
    require(cohort["selected_cases"] == len(plan["selected_cases"]), "protocol cohort size differs")
    require(cohort["expected_plan_sha256"] == sha256_bytes(plan_raw), "plan digest differs from protocol")


def validate_token(token):
    require(
        isinstance(token, str)
        and token
        and token.isascii()
        and "\n" not in token
        and "\r" not in token,
        "invalid GitHub token",
    )


def github_json(path, token, timeout):
    require(path.startswith("/repos/"), "GitHub request path is outside the repository API")
    request = urllib.request.Request(
        f"{GITHUB_API_ROOT}{path}",
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "stratadiff-review-transition-v1",
            "X-GitHub-Api-Version": GITHUB_API_VERSION,
        },
        method="GET",
    )
    if token is not None:
        validate_token(token)
        request.add_header("Authorization", f"Bearer {token}")
    with GITHUB_API_OPENER.open(request, timeout=timeout) as response:
        require(response.status == 200, f"GitHub returned HTTP {response.status}")
        raw = response.read(MAX_GITHUB_RESPONSE_BYTES + 1)
    require(len(raw) <= MAX_GITHUB_RESPONSE_BYTES, "GitHub response exceeds byte limit")
    value = json.loads(raw.decode("utf-8"), object_pairs_hook=selector.unique_json_object)
    require(isinstance(value, dict), "GitHub response must be an object")
    return raw, value


def provider_binding(case, kind, request_path, response_sha256, summary):
    selector.validate_sha256(response_sha256, "provider response digest")
    payload = {
        "case_id": case["case_id"],
        "domain": BINDING_DOMAIN,
        "kind": kind,
        "pull_request_node_id": case["pull_request"]["node_id"],
        "request_path": request_path,
        "response_sha256": response_sha256,
        "response_summary": summary,
    }
    return sha256_bytes(compact_json_bytes(payload))


def summarize_pull_response(case, value):
    exact_fields(
        case["pull_request"],
        {"node_id", "number"},
        f"{case['case_id']} selected pull request",
    )
    require(value["node_id"] == case["pull_request"]["node_id"], f"{case['case_id']} PR node ID drifted")
    require(value["number"] == case["pull_request"]["number"], f"{case['case_id']} PR number drifted")
    require(value["head"]["sha"] == case["final_head_oid"], f"{case['case_id']} final head drifted")
    require(value["merged"] is True and value["state"] == "closed", f"{case['case_id']} is not merged")
    validate_oid(value["base"]["sha"], f"{case['case_id']} requested base")
    validate_oid(value["head"]["sha"], f"{case['case_id']} live head")
    validate_timestamp(value["updated_at"], f"{case['case_id']} pull update time")
    return {
        "base_oid": value["base"]["sha"],
        "head_oid": value["head"]["sha"],
        "merged": value["merged"],
        "node_id": value["node_id"],
        "number": value["number"],
        "state": value["state"],
        "updated_at": value["updated_at"],
    }


def summarize_compare_response(case_id, base_oid, head_oid, value, label):
    require(value["base_commit"]["sha"] == base_oid, f"{case_id} {label} response base differs")
    validate_oid(value["merge_base_commit"]["sha"], f"{case_id} {label} merge base")
    require(value["status"] in ("ahead", "behind", "diverged", "identical"), f"{case_id} {label} status differs")
    for field in ("ahead_by", "behind_by", "total_commits"):
        require(type(value[field]) is int and value[field] >= 0, f"{case_id} {label} {field} is invalid")
    return {
        "ahead_by": value["ahead_by"],
        "base_oid": base_oid,
        "behind_by": value["behind_by"],
        "head_oid": head_oid,
        "merge_base_oid": value["merge_base_commit"]["sha"],
        "status": value["status"],
        "total_commits": value["total_commits"],
    }


def route_for_observation(base_oid, checkpoint, head):
    accepted = ("ahead", "identical")
    if (
        checkpoint["status"] in accepted
        and head["status"] in accepted
        and checkpoint["merge_base_oid"] == base_oid
        and head["merge_base_oid"] == base_oid
    ):
        return "provider_attested_same_base"
    return "requires_full_ancestry"


def observed_case(case, request_json):
    repository = case["repository"]
    number = case["pull_request"]["number"]
    pull_path = f"/repos/{repository}/pulls/{number}"
    pull_raw, pull_value = request_json(pull_path)
    pull = summarize_pull_response(case, pull_value)
    base_oid = pull["base_oid"]

    comparisons = {}
    response_digests = {"pull_request": sha256_bytes(pull_raw)}
    bindings = {
        "pull_request": provider_binding(
            case,
            "pull_request",
            pull_path,
            response_digests["pull_request"],
            pull,
        )
    }
    for label, head_oid in (
        ("checkpoint", case["checkpoint"]["commit_oid"]),
        ("head", case["final_head_oid"]),
    ):
        path = f"/repos/{repository}/compare/{base_oid}...{head_oid}?per_page=1&page=2"
        raw, value = request_json(path)
        summary = summarize_compare_response(case["case_id"], base_oid, head_oid, value, label)
        comparisons[label] = summary
        response_digests[label] = sha256_bytes(raw)
        bindings[label] = provider_binding(case, label, path, response_digests[label], summary)

    return {
        "bindings": bindings,
        "case_id": case["case_id"],
        "checkpoint_oid": case["checkpoint"]["commit_oid"],
        "final_head_oid": case["final_head_oid"],
        "observation_status": "observed",
        "observed_at": now_utc(),
        "provider_response_sha256": response_digests,
        "pull_request": pull,
        "repository": repository,
        "route": route_for_observation(base_oid, comparisons["checkpoint"], comparisons["head"]),
        "comparisons": comparisons,
    }


def pending_case(case):
    return {
        "case_id": case["case_id"],
        "checkpoint_oid": case["checkpoint"]["commit_oid"],
        "final_head_oid": case["final_head_oid"],
        "observation_status": "pending",
        "pull_request": {
            "node_id": case["pull_request"]["node_id"],
            "number": case["pull_request"]["number"],
        },
        "repository": case["repository"],
    }


def failed_observation(case, phase, error):
    row = pending_case(case)
    row["observation_status"] = "failed"
    row["failure"] = {
        "error_class": type(error).__name__,
        "message": str(error)[:1000],
        "phase": phase,
    }
    row["observed_at"] = now_utc()
    return row


def observation_summary(cases):
    statuses = Counter(case["observation_status"] for case in cases)
    routes = Counter(case["route"] for case in cases if case["observation_status"] == "observed")
    return {
        "failed": statuses["failed"],
        "observed": statuses["observed"],
        "pending": statuses["pending"],
        "routes": {
            "provider_attested_same_base": routes["provider_attested_same_base"],
            "provider_metadata_unavailable": statuses["failed"],
            "requires_full_ancestry": routes["requires_full_ancestry"],
        },
        "selected_cases": len(cases),
    }


def new_observation(plan_raw, plan, protocol_raw):
    timestamp = now_utc()
    cases = [pending_case(case) for case in plan["selected_cases"]]
    return {
        "cases": cases,
        "complete": False,
        "dataset_version": plan["dataset_version"],
        "plan_sha256": sha256_bytes(plan_raw),
        "protocol_sha256": sha256_bytes(protocol_raw),
        "provider": {
            "api_root": GITHUB_API_ROOT,
            "api_version": GITHUB_API_VERSION,
            "host": "github.com",
        },
        "schema": OBSERVATION_SCHEMA,
        "started_at": timestamp,
        "summary": observation_summary(cases),
        "updated_at": timestamp,
    }


def validate_observed_row(row, selected):
    exact_fields(
        row,
        {
            "bindings",
            "case_id",
            "checkpoint_oid",
            "comparisons",
            "final_head_oid",
            "observation_status",
            "observed_at",
            "provider_response_sha256",
            "pull_request",
            "repository",
            "route",
        },
        f"{selected['case_id']} observed row",
    )
    validate_timestamp(row["observed_at"], f"{selected['case_id']} observation time")
    pull = exact_fields(
        row["pull_request"],
        {"base_oid", "head_oid", "merged", "node_id", "number", "state", "updated_at"},
        f"{selected['case_id']} pull response summary",
    )
    require(pull["node_id"] == selected["pull_request"]["node_id"], "observed PR node ID differs")
    require(pull["number"] == selected["pull_request"]["number"], "observed PR number differs")
    require(pull["head_oid"] == selected["final_head_oid"], "observed PR head differs")
    require(pull["merged"] is True and pull["state"] == "closed", "observed PR is not merged")
    validate_oid(pull["base_oid"], f"{selected['case_id']} observed base")
    validate_oid(pull["head_oid"], f"{selected['case_id']} observed head")
    validate_timestamp(pull["updated_at"], f"{selected['case_id']} pull update time")

    comparisons = exact_fields(row["comparisons"], {"checkpoint", "head"}, "comparison summaries")
    bindings = exact_fields(row["bindings"], {"checkpoint", "head", "pull_request"}, "provider bindings")
    response_digests = exact_fields(
        row["provider_response_sha256"],
        {"checkpoint", "head", "pull_request"},
        "provider response digests",
    )
    pull_path = f"/repos/{selected['repository']}/pulls/{selected['pull_request']['number']}"
    require(
        bindings["pull_request"]
        == provider_binding(
            selected,
            "pull_request",
            pull_path,
            response_digests["pull_request"],
            pull,
        ),
        f"{selected['case_id']} pull binding differs",
    )
    for digest in response_digests.values():
        selector.validate_sha256(digest, "provider response digest")
    for label, head_oid in (
        ("checkpoint", selected["checkpoint"]["commit_oid"]),
        ("head", selected["final_head_oid"]),
    ):
        comparison = exact_fields(
            comparisons[label],
            {"ahead_by", "base_oid", "behind_by", "head_oid", "merge_base_oid", "status", "total_commits"},
            f"{selected['case_id']} {label} comparison",
        )
        require(comparison["base_oid"] == pull["base_oid"], f"{selected['case_id']} {label} base differs")
        require(comparison["head_oid"] == head_oid, f"{selected['case_id']} {label} head differs")
        validate_oid(comparison["merge_base_oid"], f"{selected['case_id']} {label} merge base")
        require(comparison["status"] in ("ahead", "behind", "diverged", "identical"), "invalid compare status")
        for field in ("ahead_by", "behind_by", "total_commits"):
            require(type(comparison[field]) is int and comparison[field] >= 0, "invalid comparison count")
        path = f"/repos/{selected['repository']}/compare/{pull['base_oid']}...{head_oid}?per_page=1&page=2"
        require(
            bindings[label]
            == provider_binding(selected, label, path, response_digests[label], comparison),
            f"{selected['case_id']} {label} binding differs",
        )
    expected_route = route_for_observation(pull["base_oid"], comparisons["checkpoint"], comparisons["head"])
    require(row["route"] == expected_route, f"{selected['case_id']} route differs")


def validate_observation(observation, plan_raw, plan, protocol_raw, *, require_complete):
    exact_fields(
        observation,
        {
            "cases",
            "complete",
            "dataset_version",
            "plan_sha256",
            "protocol_sha256",
            "provider",
            "schema",
            "started_at",
            "summary",
            "updated_at",
        },
        "provider observation",
    )
    require(observation["schema"] == OBSERVATION_SCHEMA, "unsupported observation schema")
    require(observation["dataset_version"] == plan["dataset_version"], "observation version differs")
    require(observation["plan_sha256"] == sha256_bytes(plan_raw), "observation plan digest differs")
    require(observation["protocol_sha256"] == sha256_bytes(protocol_raw), "observation protocol digest differs")
    require(
        observation["provider"]
        == {"api_root": GITHUB_API_ROOT, "api_version": GITHUB_API_VERSION, "host": "github.com"},
        "observation provider differs",
    )
    validate_timestamp(observation["started_at"], "observation start time")
    validate_timestamp(observation["updated_at"], "observation update time")
    cases = observation["cases"]
    require(isinstance(cases, list) and len(cases) == 30, "observation must contain all 30 cases")
    for row, selected in zip(cases, plan["selected_cases"]):
        require(row["case_id"] == selected["case_id"], "observation case order or identity differs")
        require(row["repository"] == selected["repository"], "observation repository differs")
        require(row["checkpoint_oid"] == selected["checkpoint"]["commit_oid"], "checkpoint differs")
        require(row["final_head_oid"] == selected["final_head_oid"], "final head differs")
        status = row["observation_status"]
        require(status in ("pending", "failed", "observed"), f"unsupported observation status: {status}")
        if status == "observed":
            validate_observed_row(row, selected)
        elif status == "pending":
            exact_fields(
                row,
                {"case_id", "checkpoint_oid", "final_head_oid", "observation_status", "pull_request", "repository"},
                f"{selected['case_id']} pending observation",
            )
            require(row["pull_request"] == selected["pull_request"], "pending PR identity differs")
        else:
            exact_fields(
                row,
                {
                    "case_id",
                    "checkpoint_oid",
                    "failure",
                    "final_head_oid",
                    "observation_status",
                    "observed_at",
                    "pull_request",
                    "repository",
                },
                f"{selected['case_id']} failed observation",
            )
            validate_timestamp(row["observed_at"], f"{selected['case_id']} failed observation time")
            failure = exact_fields(row["failure"], {"error_class", "message", "phase"}, "observation failure")
            for field in ("error_class", "message", "phase"):
                require(isinstance(failure[field], str) and failure[field], f"observation failure {field} is empty")
            require(row["pull_request"] == selected["pull_request"], "failed PR identity differs")
    expected_summary = observation_summary(cases)
    require(observation["summary"] == expected_summary, "observation summary differs")
    expected_complete = expected_summary["observed"] == 30
    require(observation["complete"] is expected_complete, "observation complete flag differs")
    if require_complete:
        require(expected_complete, "provider observation is incomplete")


def observe(plan_path, protocol_path, output, token, timeout):
    plan_raw, plan = load_verified_plan(plan_path)
    protocol_raw, protocol = load_protocol(protocol_path)
    validate_plan_protocol(plan_raw, plan, protocol)
    output = Path(output).resolve()
    if output.exists():
        _, observation = load_canonical_json(output, "provider observation")
        validate_observation(observation, plan_raw, plan, protocol_raw, require_complete=False)
    else:
        observation = new_observation(plan_raw, plan, protocol_raw)
        write_json_atomic(output, observation)

    def request_json(path):
        return github_json(path, token, timeout)

    for index, selected in enumerate(plan["selected_cases"]):
        if observation["cases"][index]["observation_status"] == "observed":
            continue
        phase = "pull_request"
        try:
            repository = selected["repository"]
            number = selected["pull_request"]["number"]
            pull_path = f"/repos/{repository}/pulls/{number}"
            pull_raw, pull_value = request_json(pull_path)
            pull = summarize_pull_response(selected, pull_value)
            phase = "checkpoint_compare"
            checkpoint_oid = selected["checkpoint"]["commit_oid"]
            checkpoint_path = (
                f"/repos/{repository}/compare/{pull['base_oid']}...{checkpoint_oid}?per_page=1&page=2"
            )
            checkpoint_raw, checkpoint_value = request_json(checkpoint_path)
            checkpoint = summarize_compare_response(
                selected["case_id"], pull["base_oid"], checkpoint_oid, checkpoint_value, "checkpoint"
            )
            phase = "head_compare"
            head_oid = selected["final_head_oid"]
            head_path = f"/repos/{repository}/compare/{pull['base_oid']}...{head_oid}?per_page=1&page=2"
            head_raw, head_value = request_json(head_path)
            head = summarize_compare_response(
                selected["case_id"], pull["base_oid"], head_oid, head_value, "head"
            )
            observation["cases"][index] = {
                "bindings": {
                    "checkpoint": provider_binding(
                        selected,
                        "checkpoint",
                        checkpoint_path,
                        sha256_bytes(checkpoint_raw),
                        checkpoint,
                    ),
                    "head": provider_binding(
                        selected,
                        "head",
                        head_path,
                        sha256_bytes(head_raw),
                        head,
                    ),
                    "pull_request": provider_binding(
                        selected,
                        "pull_request",
                        pull_path,
                        sha256_bytes(pull_raw),
                        pull,
                    ),
                },
                "case_id": selected["case_id"],
                "checkpoint_oid": checkpoint_oid,
                "comparisons": {"checkpoint": checkpoint, "head": head},
                "final_head_oid": head_oid,
                "observation_status": "observed",
                "observed_at": now_utc(),
                "provider_response_sha256": {
                    "checkpoint": sha256_bytes(checkpoint_raw),
                    "head": sha256_bytes(head_raw),
                    "pull_request": sha256_bytes(pull_raw),
                },
                "pull_request": pull,
                "repository": repository,
                "route": route_for_observation(pull["base_oid"], checkpoint, head),
            }
        except (KeyError, TypeError, OSError, ValueError, RuntimeError) as error:
            observation["cases"][index] = failed_observation(selected, phase, error)
        observation["updated_at"] = now_utc()
        observation["summary"] = observation_summary(observation["cases"])
        observation["complete"] = observation["summary"]["observed"] == 30
        write_json_atomic(output, observation)
    validate_observation(observation, plan_raw, plan, protocol_raw, require_complete=False)
    return observation


def verify_observation(plan_path, protocol_path, observation_path):
    plan_raw, plan = load_verified_plan(plan_path)
    protocol_raw, protocol = load_protocol(protocol_path)
    validate_plan_protocol(plan_raw, plan, protocol)
    _, observation = load_canonical_json(Path(observation_path), "provider observation")
    validate_observation(observation, plan_raw, plan, protocol_raw, require_complete=True)
    return observation


def load_materialization(materialization, observation, *, require_complete):
    root = Path(materialization).resolve()
    raw, manifest = load_canonical_json(root / "materialization.json", "materialization")
    materializer.verify(manifest, root, observation, require_complete=require_complete)
    return raw, manifest


def load_oracle(path, manifest, materialization, materialization_raw):
    raw, bundle = load_canonical_json(Path(path), "independent oracle")
    oracle_runner.verify_oracle_bundle(
        bundle,
        manifest,
        Path(materialization).resolve(),
        materialization_raw,
    )
    return raw, bundle


def positive_seconds(value):
    seconds = int(value)
    require(seconds > 0, "value must be a positive integer")
    return seconds


def parse_arguments(argv=None):
    parser = argparse.ArgumentParser(
        description="Observe, materialize, and replay the frozen ReviewTransition-30 cohort"
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    observe_parser = subparsers.add_parser(
        "observe",
        help="capture or resume the bounded GitHub metadata observation",
    )
    observe_parser.add_argument("--plan", type=Path, required=True)
    observe_parser.add_argument("--protocol", type=Path, default=DEFAULT_PROTOCOL)
    observe_parser.add_argument("--output", type=Path, required=True)
    observe_parser.add_argument("--github-token-env", default="GITHUB_TOKEN")
    observe_parser.add_argument(
        "--api-timeout-seconds",
        type=positive_seconds,
        default=DEFAULT_API_TIMEOUT_SECONDS,
    )

    verify_parser = subparsers.add_parser(
        "verify-observation",
        help="verify a complete canonical provider observation",
    )
    verify_parser.add_argument("--plan", type=Path, required=True)
    verify_parser.add_argument("--protocol", type=Path, default=DEFAULT_PROTOCOL)
    verify_parser.add_argument("--observation", type=Path, required=True)
    materialize_parser = subparsers.add_parser("materialize", help="materialize offline Git repositories")
    materialize_parser.add_argument("--plan", type=Path, required=True)
    materialize_parser.add_argument("--protocol", type=Path, default=DEFAULT_PROTOCOL)
    materialize_parser.add_argument("--observation", type=Path, required=True)
    materialize_parser.add_argument("--output", type=Path, required=True)
    materialize_parser.add_argument("--github-token-env", default="GITHUB_TOKEN")
    materialize_parser.add_argument("--fetch-timeout-seconds", type=positive_seconds, default=120)
    materialize_parser.add_argument("--max-repository-bytes", type=positive_seconds, default=1073741824)
    materialize_parser.add_argument("--case-id", action="append")
    materialize_parser.add_argument("--max-cases", type=positive_seconds)
    materialize_parser.add_argument("--legacy-attempt-timeout-seconds", type=positive_seconds)
    materialize_verify = subparsers.add_parser("verify-materialization", help="verify offline Git materialization")
    materialize_verify.add_argument("--plan", type=Path, required=True)
    materialize_verify.add_argument("--protocol", type=Path, default=DEFAULT_PROTOCOL)
    materialize_verify.add_argument("--observation", type=Path, required=True)
    materialize_verify.add_argument("--materialization", type=Path, required=True)
    materialize_verify.add_argument("--allow-incomplete", action="store_true")

    oracle_parser = subparsers.add_parser(
        "generate-oracle",
        help="independently derive policy truth from offline Git objects",
    )
    oracle_parser.add_argument("--plan", type=Path, required=True)
    oracle_parser.add_argument("--protocol", type=Path, default=DEFAULT_PROTOCOL)
    oracle_parser.add_argument("--observation", type=Path, required=True)
    oracle_parser.add_argument("--materialization", type=Path, required=True)
    oracle_parser.add_argument("--output", type=Path, required=True)
    oracle_parser.add_argument("--case-id", action="append")
    oracle_parser.add_argument("--max-cases", type=positive_seconds)

    oracle_verify = subparsers.add_parser(
        "verify-oracle",
        help="recompute and verify a canonical independent oracle",
    )
    oracle_verify.add_argument("--plan", type=Path, required=True)
    oracle_verify.add_argument("--protocol", type=Path, default=DEFAULT_PROTOCOL)
    oracle_verify.add_argument("--observation", type=Path, required=True)
    oracle_verify.add_argument("--materialization", type=Path, required=True)
    oracle_verify.add_argument("--oracle", type=Path, required=True)

    replay_parser = subparsers.add_parser(
        "replay-product",
        help="run two clean offline product replays and compare normalized artifacts",
    )
    replay_parser.add_argument("--plan", type=Path, required=True)
    replay_parser.add_argument("--protocol", type=Path, default=DEFAULT_PROTOCOL)
    replay_parser.add_argument("--observation", type=Path, required=True)
    replay_parser.add_argument("--materialization", type=Path, required=True)
    replay_parser.add_argument("--oracle", type=Path, required=True)
    replay_parser.add_argument("--binary", type=Path, required=True)
    replay_parser.add_argument("--output", type=Path, required=True)
    replay_parser.add_argument("--case-id", action="append")
    replay_parser.add_argument("--max-cases", type=positive_seconds)
    replay_parser.add_argument("--timeout-seconds", type=positive_seconds, default=120)

    replay_verify = subparsers.add_parser(
        "verify-product-replay",
        help="verify normalized double-replay artifacts and their frozen bindings",
    )
    replay_verify.add_argument("--plan", type=Path, required=True)
    replay_verify.add_argument("--protocol", type=Path, default=DEFAULT_PROTOCOL)
    replay_verify.add_argument("--observation", type=Path, required=True)
    replay_verify.add_argument("--materialization", type=Path, required=True)
    replay_verify.add_argument("--oracle", type=Path, required=True)
    replay_verify.add_argument("--binary", type=Path, required=True)
    replay_verify.add_argument("--replay", type=Path, required=True)
    return parser.parse_args(argv)


def main(argv=None):
    arguments = parse_arguments(argv)
    if arguments.command == "observe":
        require(arguments.github_token_env in os.environ, f"missing token environment variable: {arguments.github_token_env}")
        token = os.environ[arguments.github_token_env]
        validate_token(token)
        observation = observe(
            arguments.plan,
            arguments.protocol,
            arguments.output,
            token,
            arguments.api_timeout_seconds,
        )
        result = {
            "complete": observation["complete"],
            "observation": str(arguments.output.resolve()),
            "summary": observation["summary"],
        }
        exit_code = 0 if observation["complete"] else 2
    elif arguments.command == "verify-observation":
        observation = verify_observation(
            arguments.plan,
            arguments.protocol,
            arguments.observation,
        )
        result = {
            "observation": str(arguments.observation.resolve()),
            "observation_verified": True,
            "summary": observation["summary"],
        }
        exit_code = 0
    elif arguments.command == "materialize":
        require(arguments.github_token_env in os.environ, f"missing token environment variable: {arguments.github_token_env}")
        observation = verify_observation(arguments.plan, arguments.protocol, arguments.observation)
        manifest = materializer.materialize(
            observation,
            arguments.output,
            os.environ[arguments.github_token_env],
            arguments.fetch_timeout_seconds,
            arguments.max_repository_bytes,
            case_ids=arguments.case_id,
            max_cases=arguments.max_cases,
            legacy_attempt_timeout=arguments.legacy_attempt_timeout_seconds,
        )
        result = {
            "materialization": str(arguments.output.resolve()),
            "complete": manifest["complete"],
            "summary": manifest["summary"],
        }
        exit_code = 0 if manifest["complete"] else 2
    elif arguments.command == "verify-materialization":
        observation = verify_observation(arguments.plan, arguments.protocol, arguments.observation)
        raw, manifest = load_materialization(
            arguments.materialization,
            observation,
            require_complete=not arguments.allow_incomplete,
        )
        result = {
            "materialization_verified": True,
            "sha256": sha256_bytes(raw),
            "complete": manifest["complete"],
            "summary": manifest["summary"],
        }
        exit_code = 0
    elif arguments.command == "generate-oracle":
        observation = verify_observation(arguments.plan, arguments.protocol, arguments.observation)
        materialization_raw, manifest = load_materialization(
            arguments.materialization,
            observation,
            require_complete=False,
        )
        require(not arguments.output.exists(), f"oracle output already exists: {arguments.output}")
        bundle = oracle_runner.generate_oracle_bundle(
            manifest,
            arguments.materialization,
            materialization_raw,
            case_ids=arguments.case_id,
            max_cases=arguments.max_cases,
        )
        write_json_atomic(arguments.output, bundle)
        result = {
            "oracle": str(arguments.output.resolve()),
            "complete": bundle["complete"],
            "summary": bundle["summary"],
            "sha256": sha256_bytes(canonical_json_bytes(bundle)),
        }
        exit_code = 0 if bundle["complete"] else 2
    elif arguments.command == "verify-oracle":
        observation = verify_observation(arguments.plan, arguments.protocol, arguments.observation)
        materialization_raw, manifest = load_materialization(
            arguments.materialization,
            observation,
            require_complete=False,
        )
        raw, bundle = load_oracle(
            arguments.oracle,
            manifest,
            arguments.materialization,
            materialization_raw,
        )
        result = {
            "oracle_verified": True,
            "complete": bundle["complete"],
            "summary": bundle["summary"],
            "sha256": sha256_bytes(raw),
        }
        exit_code = 0
    elif arguments.command == "replay-product":
        observation = verify_observation(arguments.plan, arguments.protocol, arguments.observation)
        materialization_raw, manifest = load_materialization(
            arguments.materialization,
            observation,
            require_complete=False,
        )
        oracle_raw, bundle = load_oracle(
            arguments.oracle,
            manifest,
            arguments.materialization,
            materialization_raw,
        )
        replay = oracle_runner.replay_product(
            bundle,
            manifest,
            arguments.materialization,
            materialization_raw,
            oracle_raw,
            arguments.binary,
            arguments.output,
            case_ids=arguments.case_id,
            max_cases=arguments.max_cases,
            timeout=arguments.timeout_seconds,
        )
        result = {
            "replay": str(arguments.output.resolve()),
            "complete": replay["complete"],
            "summary": replay["summary"],
        }
        exit_code = 0 if replay["complete"] else 2
    elif arguments.command == "verify-product-replay":
        observation = verify_observation(arguments.plan, arguments.protocol, arguments.observation)
        materialization_raw, manifest = load_materialization(
            arguments.materialization,
            observation,
            require_complete=False,
        )
        oracle_raw, bundle = load_oracle(
            arguments.oracle,
            manifest,
            arguments.materialization,
            materialization_raw,
        )
        replay_raw, replay = load_canonical_json(arguments.replay / "replay.json", "product replay")
        oracle_runner.verify_replay_bundle(
            replay,
            arguments.replay.resolve(),
            bundle,
            manifest,
            materialization_raw,
            oracle_raw,
            arguments.binary,
        )
        result = {
            "replay_verified": True,
            "complete": replay["complete"],
            "summary": replay["summary"],
            "sha256": sha256_bytes(replay_raw),
        }
        exit_code = 0
    else:
        raise ValueError(f"unsupported command: {arguments.command}")
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
