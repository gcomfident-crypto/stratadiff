#!/usr/bin/env python3
"""Verify and score the controlled Doctor Workflow Trigger v1 corpus."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
from typing import Callable


BUNDLE = Path(__file__).resolve().parent
DEFAULT_CASES = BUNDLE / "cases.json"
DEFAULT_MANIFEST = BUNDLE / "manifest.json"
DEFAULT_ORACLE = BUNDLE / "oracle.json"
CHECKSUMS = BUNDLE / "SHA256SUMS"

CASES_SCHEMA = "stratadiff-doctor-workflow-trigger-cases-v1"
MANIFEST_SCHEMA = "stratadiff-doctor-workflow-trigger-manifest-v1"
ORACLE_SCHEMA = "stratadiff-doctor-workflow-trigger-oracle-v1"
PREDICTIONS_SCHEMA = "stratadiff-doctor-workflow-trigger-predictions-v1"
DATASET_VERSION = "1.0.0"

CASE_ID_PATTERN = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
OID_PATTERN = re.compile(r"^[0-9a-f]{40}$")
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
WORKFLOW_PATH_PATTERN = re.compile(r"^\.github/workflows/[^/]+\.ya?ml$")

EXPECTED_CHECKSUM_FILES = {
    "README.md",
    "cases.json",
    "manifest.json",
    "oracle.json",
    "verify.py",
}
EXPECTED_CASE_KEYS = {"description", "id", "input", "provenance"}
EXPECTED_INPUT_KEYS = {
    "changed_files",
    "collection_gaps",
    "expected_app",
    "historical_check_names",
    "last_activity",
    "provider_capability",
    "pull_request",
    "required_context",
    "runs",
    "target",
    "workflows",
}
FORBIDDEN_CASE_KEYS = {"cause_code", "confidence", "diagnosis", "expected", "fix", "oracle"}

CAUSE_CODES = {
    "duplicate_job_name_ambiguous",
    "fork_approval_possible",
    "fork_approval_required",
    "merge_group_trigger_missing",
    "none",
    "provider_did_not_emit_merge_group_status",
    "provider_runtime_delivery_gap",
    "pull_request_merge_conflict",
    "required_context_not_produced",
    "workflow_activity_excludes_synchronize",
    "workflow_branch_filter_excluded",
    "workflow_definition_invalid",
    "workflow_disabled",
    "workflow_path_filter_excluded",
    "workflow_trigger_unknown",
}
CONFIDENCE_LEVELS = {"certain", "high", "uncertain"}
EXPECTED_APPS = {"github-actions", "third-party"}
PROVIDER_CAPABILITIES = {"not_applicable", "unknown", "unsupported_merge_group"}
TARGET_KINDS = {"merge_group", "pull_request_head"}
WORKFLOW_STATES = {"active", "disabled_inactivity", "disabled_manually"}
WORKFLOW_SYNTAX = {"invalid", "valid"}
RUN_EVENTS = {"merge_group", "pull_request"}
RUN_STATUSES = {"completed", "in_progress", "queued", "requested", "waiting"}
RUN_CONCLUSIONS = {
    None,
    "action_required",
    "cancelled",
    "failure",
    "skipped",
    "startup_failure",
    "success",
}
CLAIM_BOUNDARY = {
    "controlled_trigger_semantics_supported": True,
    "developer_time_saved_supported": False,
    "failure_prevalence_supported": False,
    "live_collector_conformance_supported": False,
    "market_demand_supported": False,
    "merge_safety_supported": False,
    "production_accuracy_supported": False,
}


class BenchmarkError(RuntimeError):
    """A benchmark asset violates the frozen bundle contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise BenchmarkError(message)


def require_exact_keys(value: dict[str, object], keys: set[str], label: str) -> None:
    require(set(value) == keys, f"{label} fields differ: {sorted(set(value) ^ keys)}")


def require_object(value: object, label: str) -> dict[str, object]:
    require(type(value) is dict, f"{label} must be an object")
    assert isinstance(value, dict)
    return value


def require_array(value: object, label: str) -> list[object]:
    require(type(value) is list, f"{label} must be an array")
    assert isinstance(value, list)
    return value


def require_string(value: object, label: str) -> str:
    require(type(value) is str and bool(value), f"{label} must be a non-empty string")
    assert isinstance(value, str)
    return value


def require_bool(value: object, label: str) -> bool:
    require(type(value) is bool, f"{label} must be a boolean")
    assert isinstance(value, bool)
    return value


def require_int(value: object, label: str, minimum: int = 0) -> int:
    require(type(value) is int and value >= minimum, f"{label} must be an integer >= {minimum}")
    assert isinstance(value, int)
    return value


def unique_json_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        require(key not in value, f"duplicate JSON key: {key}")
        value[key] = item
    return value


def read_json(path: Path) -> tuple[bytes, dict[str, object]]:
    payload = path.read_bytes()
    value = json.loads(payload, object_pairs_hook=unique_json_object)
    require(type(value) is dict, f"{path} must contain one JSON object")
    assert isinstance(value, dict)
    return payload, value


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def validate_string_array(value: object, label: str) -> list[str]:
    values = require_array(value, label)
    result: list[str] = []
    for index, item in enumerate(values):
        result.append(require_string(item, f"{label}[{index}]"))
    require(len(result) == len(set(result)), f"{label} must not contain duplicates")
    return result


def reject_oracle_leak(value: object, label: str) -> None:
    if isinstance(value, dict):
        for key, item in value.items():
            require(key not in FORBIDDEN_CASE_KEYS, f"oracle field leaked into {label}.{key}")
            reject_oracle_leak(item, f"{label}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            reject_oracle_leak(item, f"{label}[{index}]")


def validate_trigger(value: object, label: str, merge_group: bool) -> dict[str, object] | None:
    if value is None:
        return None
    trigger = require_object(value, label)
    if merge_group:
        require_exact_keys(trigger, {"types"}, label)
        types = validate_string_array(trigger["types"], f"{label}.types")
        require(set(types) <= {"checks_requested"}, f"{label}.types has unsupported values")
    else:
        require_exact_keys(
            trigger,
            {"branches", "branches_ignore", "paths", "paths_ignore", "types"},
            label,
        )
        for field in ["branches", "branches_ignore", "paths", "paths_ignore", "types"]:
            validate_string_array(trigger[field], f"{label}.{field}")
        require(
            not (trigger["paths"] and trigger["paths_ignore"]),
            f"{label} cannot combine paths and paths_ignore",
        )
        require(
            not (trigger["branches"] and trigger["branches_ignore"]),
            f"{label} cannot combine branches and branches_ignore",
        )
    return trigger


def validate_job(value: object, label: str) -> dict[str, object]:
    job = require_object(value, label)
    require_exact_keys(job, {"condition", "id", "name", "name_static", "reusable"}, label)
    require_string(job["condition"], f"{label}.condition")
    require_string(job["id"], f"{label}.id")
    require_string(job["name"], f"{label}.name")
    require_bool(job["name_static"], f"{label}.name_static")
    require_bool(job["reusable"], f"{label}.reusable")
    return job


def validate_workflow(value: object, label: str) -> dict[str, object]:
    workflow = require_object(value, label)
    require_exact_keys(workflow, {"jobs", "path", "state", "syntax", "triggers"}, label)
    path = require_string(workflow["path"], f"{label}.path")
    require(bool(WORKFLOW_PATH_PATTERN.fullmatch(path)), f"{label}.path is not a workflow path")
    require(workflow["state"] in WORKFLOW_STATES, f"{label}.state is invalid")
    require(workflow["syntax"] in WORKFLOW_SYNTAX, f"{label}.syntax is invalid")
    jobs = require_array(workflow["jobs"], f"{label}.jobs")
    for index, job in enumerate(jobs):
        validate_job(job, f"{label}.jobs[{index}]")
    triggers = require_object(workflow["triggers"], f"{label}.triggers")
    require_exact_keys(triggers, {"merge_group", "pull_request"}, f"{label}.triggers")
    validate_trigger(triggers["merge_group"], f"{label}.triggers.merge_group", True)
    validate_trigger(triggers["pull_request"], f"{label}.triggers.pull_request", False)
    if workflow["syntax"] == "invalid":
        require(not jobs, f"{label} invalid syntax must not claim parsed jobs")
    return workflow


def validate_run(value: object, label: str, target_sha: str) -> dict[str, object]:
    run = require_object(value, label)
    require_exact_keys(run, {"conclusion", "event", "head_sha", "status", "workflow_path"}, label)
    require(run["event"] in RUN_EVENTS, f"{label}.event is invalid")
    head_sha = require_string(run["head_sha"], f"{label}.head_sha")
    require(head_sha == target_sha, f"{label} is not bound to the exact target")
    require(run["status"] in RUN_STATUSES, f"{label}.status is invalid")
    require(run["conclusion"] in RUN_CONCLUSIONS, f"{label}.conclusion is invalid")
    require_string(run["workflow_path"], f"{label}.workflow_path")
    return run


def validate_case(value: object, index: int, provenance_ids: set[str]) -> dict[str, object]:
    label = f"cases[{index}]"
    case = require_object(value, label)
    require_exact_keys(case, EXPECTED_CASE_KEYS, label)
    reject_oracle_leak(case, label)
    case_id = require_string(case["id"], f"{label}.id")
    require(bool(CASE_ID_PATTERN.fullmatch(case_id)), f"{label}.id is invalid")
    require_string(case["description"], f"{label}.description")
    sources = validate_string_array(case["provenance"], f"{label}.provenance")
    require(bool(sources), f"{label} must cite provenance")
    require(set(sources) <= provenance_ids, f"{label} cites unknown provenance")

    data = require_object(case["input"], f"{label}.input")
    require_exact_keys(data, EXPECTED_INPUT_KEYS, f"{label}.input")
    require(data["expected_app"] in EXPECTED_APPS, f"{label}.input.expected_app is invalid")
    require(
        data["provider_capability"] in PROVIDER_CAPABILITIES,
        f"{label}.input.provider_capability is invalid",
    )
    if data["expected_app"] == "github-actions":
        require(
            data["provider_capability"] == "not_applicable",
            f"{label} Actions case cannot assert third-party capability",
        )
    validate_string_array(data["collection_gaps"], f"{label}.input.collection_gaps")
    validate_string_array(data["historical_check_names"], f"{label}.input.historical_check_names")
    require_string(data["last_activity"], f"{label}.input.last_activity")
    require_string(data["required_context"], f"{label}.input.required_context")

    changed = require_object(data["changed_files"], f"{label}.input.changed_files")
    require_exact_keys(
        changed,
        {"complete", "github_filter_file_limit_reached", "paths", "total"},
        f"{label}.input.changed_files",
    )
    require_bool(changed["complete"], f"{label}.input.changed_files.complete")
    limit_reached = require_bool(
        changed["github_filter_file_limit_reached"],
        f"{label}.input.changed_files.github_filter_file_limit_reached",
    )
    paths = validate_string_array(changed["paths"], f"{label}.input.changed_files.paths")
    total = require_int(changed["total"], f"{label}.input.changed_files.total")
    require(total >= len(paths), f"{label}.input.changed_files.total is too small")
    if not limit_reached:
        require(total == len(paths), f"{label} non-truncated paths must be complete")
    else:
        require(total > 300, f"{label} filter limit requires more than 300 files")

    pr = require_object(data["pull_request"], f"{label}.input.pull_request")
    require_exact_keys(
        pr,
        {"base_ref", "base_sha", "head_ref", "head_repository_is_fork", "head_sha", "mergeable_state", "number"},
        f"{label}.input.pull_request",
    )
    require_string(pr["base_ref"], f"{label}.input.pull_request.base_ref")
    require_string(pr["head_ref"], f"{label}.input.pull_request.head_ref")
    for field in ["base_sha", "head_sha"]:
        oid = require_string(pr[field], f"{label}.input.pull_request.{field}")
        require(bool(OID_PATTERN.fullmatch(oid)), f"{label}.input.pull_request.{field} is invalid")
    require_bool(pr["head_repository_is_fork"], f"{label}.input.pull_request.head_repository_is_fork")
    require(pr["mergeable_state"] in {"conflicting", "mergeable", "unknown"}, f"{label}.input.pull_request.mergeable_state is invalid")
    require_int(pr["number"], f"{label}.input.pull_request.number", 1)

    target = require_object(data["target"], f"{label}.input.target")
    require_exact_keys(target, {"kind", "sha"}, f"{label}.input.target")
    require(target["kind"] in TARGET_KINDS, f"{label}.input.target.kind is invalid")
    target_sha = require_string(target["sha"], f"{label}.input.target.sha")
    require(bool(OID_PATTERN.fullmatch(target_sha)), f"{label}.input.target.sha is invalid")
    if target["kind"] == "pull_request_head":
        require(target_sha == pr["head_sha"], f"{label} PR target is not the PR head")
    else:
        require(target_sha != pr["head_sha"], f"{label} merge-group target reused PR head")

    workflows = require_array(data["workflows"], f"{label}.input.workflows")
    workflow_paths: set[str] = set()
    for workflow_index, workflow_value in enumerate(workflows):
        workflow = validate_workflow(workflow_value, f"{label}.input.workflows[{workflow_index}]")
        path = workflow["path"]
        assert isinstance(path, str)
        require(path not in workflow_paths, f"{label} repeats workflow path {path}")
        workflow_paths.add(path)

    runs = require_array(data["runs"], f"{label}.input.runs")
    for run_index, run in enumerate(runs):
        validate_run(run, f"{label}.input.runs[{run_index}]", target_sha)
    return case


def validate_cases(value: dict[str, object], provenance_ids: set[str]) -> list[dict[str, object]]:
    require_exact_keys(value, {"cases", "dataset_version", "schema"}, "cases asset")
    require(value["schema"] == CASES_SCHEMA, "cases schema changed")
    require(value["dataset_version"] == DATASET_VERSION, "cases version changed")
    cases_raw = require_array(value["cases"], "cases")
    cases: list[dict[str, object]] = []
    ids: set[str] = set()
    for index, item in enumerate(cases_raw):
        case = validate_case(item, index, provenance_ids)
        case_id = case["id"]
        assert isinstance(case_id, str)
        require(case_id not in ids, f"duplicate case id: {case_id}")
        ids.add(case_id)
        cases.append(case)
    return cases


def glob_regex(pattern: str) -> re.Pattern[str]:
    result = ""
    index = 0
    while index < len(pattern):
        character = pattern[index]
        if character == "*":
            if index + 1 < len(pattern) and pattern[index + 1] == "*":
                result += ".*"
                index += 2
            else:
                result += "[^/]*"
                index += 1
        elif character == "?":
            result += "[^/]"
            index += 1
        else:
            result += re.escape(character)
            index += 1
    return re.compile(f"^{result}$")


def matches(pattern: str, value: str) -> bool:
    return bool(glob_regex(pattern).fullmatch(value))


def ordered_path_selected(patterns: list[str], path: str) -> bool:
    selected = False
    for pattern in patterns:
        negative = pattern.startswith("!")
        effective = pattern[1:] if negative else pattern
        if matches(effective, path):
            selected = not negative
    return selected


def fix(action_code: str, argv: list[str], requires_human_edit: bool) -> dict[str, object]:
    return {
        "action_code": action_code,
        "argv": argv,
        "requires_human_edit": requires_human_edit,
    }


def outcome(
    cause_code: str,
    confidence: str,
    evidence: list[str],
    repair: dict[str, object],
    target_sha: str,
) -> dict[str, object]:
    return {
        "cause_code": cause_code,
        "confidence": confidence,
        "evidence": evidence,
        "fix": repair,
        "target_sha": target_sha,
    }


def derive_case(case: dict[str, object]) -> dict[str, object]:
    data = case["input"]
    assert isinstance(data, dict)
    target = data["target"]
    pr = data["pull_request"]
    changed = data["changed_files"]
    assert isinstance(target, dict) and isinstance(pr, dict) and isinstance(changed, dict)
    target_sha = target["sha"]
    required = data["required_context"]
    assert isinstance(target_sha, str) and isinstance(required, str)
    workflows = data["workflows"]
    runs = data["runs"]
    gaps = data["collection_gaps"]
    historical = data["historical_check_names"]
    assert isinstance(workflows, list) and isinstance(runs, list)
    assert isinstance(gaps, list) and isinstance(historical, list)

    if data["expected_app"] == "third-party":
        if data["provider_capability"] == "unsupported_merge_group":
            return outcome(
                "provider_did_not_emit_merge_group_status",
                "high",
                ["target:merge_group", "expected_app:third-party", "provider_capability:unsupported_merge_group"],
                fix("replace_or_ungate_unsupported_provider", [], True),
                target_sha,
            )
        return outcome(
            "provider_runtime_delivery_gap",
            "uncertain",
            ["expected_app:third-party", "exact_target_status:absent", "gap:provider_webhook_delivery_unavailable"],
            fix("inspect_provider_webhook_delivery", [], False),
            target_sha,
        )

    startup_failure = any(
        run["status"] == "completed" and run["conclusion"] == "startup_failure"
        for run in runs
    )
    invalid = [workflow for workflow in workflows if workflow["syntax"] == "invalid"]
    if invalid and startup_failure:
        return outcome(
            "workflow_definition_invalid",
            "certain",
            ["workflow_syntax:invalid", "run_conclusion:startup_failure"],
            fix("repair_workflow_syntax", [], True),
            target_sha,
        )

    dynamic = any(
        not job["name_static"]
        for workflow in workflows
        for job in workflow["jobs"]
    )
    reusable = any(
        job["reusable"]
        for workflow in workflows
        for job in workflow["jobs"]
    )
    if dynamic and "dynamic_job_name" in gaps:
        return outcome(
            "workflow_trigger_unknown",
            "uncertain",
            ["job_name:dynamic", "gap:dynamic_job_name"],
            fix("collect_expanded_job_names", [], False),
            target_sha,
        )
    if reusable and "reusable_workflow_definition_unavailable" in gaps:
        return outcome(
            "workflow_trigger_unknown",
            "uncertain",
            ["job:reusable", "gap:reusable_workflow_definition_unavailable"],
            fix("collect_reusable_workflow_definition", [], False),
            target_sha,
        )

    matching: list[tuple[dict[str, object], dict[str, object]]] = []
    for workflow in workflows:
        for job in workflow["jobs"]:
            if job["name_static"] and job["name"] == required:
                matching.append((workflow, job))
    if not matching and required in historical:
        current_names = [
            job["name"]
            for workflow in workflows
            for job in workflow["jobs"]
            if job["name_static"]
        ]
        require(bool(current_names), f"{case['id']} rename case needs a current static job")
        return outcome(
            "required_context_not_produced",
            "high",
            [f"required:{required}", f"historical:{required}", f"current_job:{current_names[0]}"],
            fix("synchronize_required_context", [], True),
            target_sha,
        )
    require(bool(matching), f"{case['id']} has no diagnosable required job")

    active_matching = [pair for pair in matching if pair[0]["state"] == "active"]
    if not active_matching:
        workflow = matching[0][0]
        return outcome(
            "workflow_disabled",
            "certain",
            [f"workflow:{workflow['path']}", f"workflow_state:{workflow['state']}"],
            fix("enable_workflow", ["gh", "workflow", "enable", workflow["path"]], False),
            target_sha,
        )
    if len(active_matching) > 1:
        evidence = [f"job:{required}@{workflow['path']}" for workflow, _ in active_matching]
        return outcome(
            "duplicate_job_name_ambiguous",
            "certain",
            evidence,
            fix("make_required_job_names_unique", [], True),
            target_sha,
        )

    workflow, job = active_matching[0]
    action_required = any(
        run["conclusion"] == "action_required" and run["head_sha"] == target_sha
        for run in runs
    )
    if pr["head_repository_is_fork"] and action_required:
        return outcome(
            "fork_approval_required",
            "certain",
            ["head_repository:fork", "run_conclusion:action_required", "run_head:exact"],
            fix("approve_fork_workflow", [], False),
            target_sha,
        )

    if target["kind"] == "pull_request_head" and pr["mergeable_state"] == "conflicting":
        return outcome(
            "pull_request_merge_conflict",
            "certain",
            ["mergeable_state:conflicting", "trigger:pull_request", "exact_head_run:absent"],
            fix("resolve_merge_conflict", [], True),
            target_sha,
        )

    triggers = workflow["triggers"]
    assert isinstance(triggers, dict)
    if target["kind"] == "merge_group":
        merge_group = triggers["merge_group"]
        if merge_group is None:
            return outcome(
                "merge_group_trigger_missing",
                "certain",
                [f"target:{target['kind']}", f"workflow:{workflow['path']}", "trigger:merge_group_absent"],
                fix("add_merge_group_trigger", [], True),
                target_sha,
            )
        return outcome(
            "none",
            "certain",
            ["target:merge_group", "trigger:merge_group_checks_requested", "run:queued"],
            fix("none", [], False),
            target_sha,
        )

    if pr["head_repository_is_fork"] and "fork_approval_policy_unavailable" in gaps:
        return outcome(
            "fork_approval_possible",
            "uncertain",
            ["head_repository:fork", "exact_head_run:absent", "gap:fork_approval_policy_unavailable"],
            fix(
                "inspect_fork_approval_policy",
                ["gh", "api", "repos/{owner}/{repo}/actions/permissions/fork-pr-contributor-approval"],
                False,
            ),
            target_sha,
        )

    if changed["github_filter_file_limit_reached"]:
        return outcome(
            "workflow_trigger_unknown",
            "uncertain",
            [
                f"changed_files:{changed['total']}",
                "github_filter_limit:reached",
                "gap:github_path_filter_evaluation_truncated",
            ],
            fix("inspect_github_filter_evaluation", [], False),
            target_sha,
        )

    pull_request = triggers["pull_request"]
    require(pull_request is not None, f"{case['id']} matching workflow lacks pull_request trigger")
    assert isinstance(pull_request, dict)
    types = pull_request["types"]
    assert isinstance(types, list)
    default_types = ["opened", "reopened", "synchronize"]
    effective_types = types if types else default_types
    if data["last_activity"] not in effective_types:
        return outcome(
            "workflow_activity_excludes_synchronize",
            "certain",
            [f"last_activity:{data['last_activity']}", f"types:{','.join(types)}", "exact_head_run:absent"],
            fix("add_pull_request_synchronize", [], True),
            target_sha,
        )

    branches = pull_request["branches"]
    branches_ignore = pull_request["branches_ignore"]
    assert isinstance(branches, list) and isinstance(branches_ignore, list)
    base_ref = pr["base_ref"]
    assert isinstance(base_ref, str)
    branch_excluded = bool(branches) and not any(matches(pattern, base_ref) for pattern in branches)
    branch_excluded = branch_excluded or any(matches(pattern, base_ref) for pattern in branches_ignore)
    if branch_excluded:
        pattern = branches[0] if branches else branches_ignore[0]
        return outcome(
            "workflow_branch_filter_excluded",
            "certain",
            [f"base:{base_ref}", f"branches:{pattern}", f"workflow:{workflow['path']}"],
            fix("align_required_base_branch_filter", [], True),
            target_sha,
        )

    paths = pull_request["paths"]
    paths_ignore = pull_request["paths_ignore"]
    changed_paths = changed["paths"]
    assert isinstance(paths, list) and isinstance(paths_ignore, list) and isinstance(changed_paths, list)
    path_excluded = bool(paths) and not any(
        ordered_path_selected(paths, path) for path in changed_paths
    )
    path_excluded = path_excluded or (
        bool(paths_ignore)
        and bool(changed_paths)
        and all(any(matches(pattern, path) for pattern in paths_ignore) for path in changed_paths)
    )
    if path_excluded:
        if paths:
            evidence = [f"changed:{changed_paths[0]}", f"paths:{paths[0]}", f"workflow:{workflow['path']}"]
        else:
            evidence = [f"changed:{changed_paths[0]}", f"paths_ignore:{paths_ignore[0]}", f"workflow:{workflow['path']}"]
        return outcome(
            "workflow_path_filter_excluded",
            "certain",
            evidence,
            fix("move_required_filter_inside_workflow", [], True),
            target_sha,
        )

    if job["condition"] != "always":
        return outcome(
            "none",
            "certain",
            ["workflow:triggered", "job:conditional", "run:skipped_success"],
            fix("none", [], False),
            target_sha,
        )
    if branches:
        return outcome(
            "none",
            "certain",
            [f"base:{base_ref}", f"head:{pr['head_ref']}", f"branches:{branches[0]}", "run:queued"],
            fix("none", [], False),
            target_sha,
        )
    if any(pattern.startswith("!") for pattern in paths):
        return outcome(
            "none",
            "certain",
            [f"changed:{changed_paths[0]}", "paths:ordered_reinclude", "run:queued"],
            fix("none", [], False),
            target_sha,
        )
    if paths:
        return outcome(
            "none",
            "certain",
            [f"changed:{changed_paths[0]}", f"paths:{paths[0]}", "run:queued"],
            fix("none", [], False),
            target_sha,
        )
    raise BenchmarkError(f"{case['id']} does not reach a frozen diagnostic branch")


def derive_oracle(cases: list[dict[str, object]]) -> dict[str, object]:
    return {
        "cases": {case["id"]: derive_case(case) for case in cases},
        "dataset_version": DATASET_VERSION,
        "schema": ORACLE_SCHEMA,
    }


def validate_outcome(value: object, label: str) -> dict[str, object]:
    result = require_object(value, label)
    require_exact_keys(result, {"cause_code", "confidence", "evidence", "fix", "target_sha"}, label)
    require(result["cause_code"] in CAUSE_CODES, f"{label}.cause_code is invalid")
    require(result["confidence"] in CONFIDENCE_LEVELS, f"{label}.confidence is invalid")
    evidence = validate_string_array(result["evidence"], f"{label}.evidence")
    require(bool(evidence), f"{label}.evidence must not be empty")
    target_sha = require_string(result["target_sha"], f"{label}.target_sha")
    require(bool(OID_PATTERN.fullmatch(target_sha)), f"{label}.target_sha is invalid")
    repair = require_object(result["fix"], f"{label}.fix")
    require_exact_keys(repair, {"action_code", "argv", "requires_human_edit"}, f"{label}.fix")
    require_string(repair["action_code"], f"{label}.fix.action_code")
    validate_string_array(repair["argv"], f"{label}.fix.argv")
    require_bool(repair["requires_human_edit"], f"{label}.fix.requires_human_edit")
    if repair["requires_human_edit"]:
        require(not repair["argv"], f"{label} human edit must not pretend to be executable")
    if result["cause_code"] == "none":
        require(repair["action_code"] == "none", f"{label} control must not propose a repair")
    if result["cause_code"] == "fork_approval_possible":
        require(result["confidence"] == "uncertain", f"{label} fork possibility cannot be certain")
    return result


def validate_oracle(value: dict[str, object], case_ids: set[str], schema: str) -> dict[str, object]:
    require_exact_keys(value, {"cases", "dataset_version", "schema"}, "oracle")
    require(value["schema"] == schema, "oracle schema changed")
    require(value["dataset_version"] == DATASET_VERSION, "oracle version changed")
    results = require_object(value["cases"], "oracle.cases")
    require(set(results) == case_ids, "oracle case ids differ from cases")
    for case_id, result in results.items():
        validate_outcome(result, f"oracle.cases.{case_id}")
    return value


def summarize(cases: list[dict[str, object]], oracle: dict[str, object]) -> dict[str, object]:
    results = oracle["cases"]
    assert isinstance(results, dict)
    causes: dict[str, int] = {}
    confidence: dict[str, int] = {}
    human_edits = 0
    for result in results.values():
        assert isinstance(result, dict)
        cause = result["cause_code"]
        level = result["confidence"]
        assert isinstance(cause, str) and isinstance(level, str)
        causes[cause] = causes.setdefault(cause, 0) + 1
        confidence[level] = confidence.setdefault(level, 0) + 1
        repair = result["fix"]
        assert isinstance(repair, dict)
        if repair["requires_human_edit"]:
            human_edits += 1
    provenance_refs = sum(len(case["provenance"]) for case in cases)
    return {
        "cases": len(cases),
        "causes": dict(sorted(causes.items())),
        "confidence": dict(sorted(confidence.items())),
        "human_edits": human_edits,
        "provenance_refs": provenance_refs,
    }


def validate_manifest_shape(value: dict[str, object]) -> set[str]:
    require_exact_keys(
        value,
        {
            "acceptance_gates",
            "assets",
            "claim_boundary",
            "dataset_license",
            "dataset_version",
            "description",
            "designation",
            "expected_summary",
            "name",
            "provenance",
            "required_case_ids",
            "schema",
        },
        "manifest",
    )
    require(value["schema"] == MANIFEST_SCHEMA, "manifest schema changed")
    require(value["dataset_version"] == DATASET_VERSION, "manifest version changed")
    require(value["dataset_license"] == "MIT", "dataset license changed")
    require(value["designation"] == "controlled_semantic_regression", "designation changed")
    require_string(value["description"], "manifest.description")
    require_string(value["name"], "manifest.name")
    require(value["claim_boundary"] == CLAIM_BOUNDARY, "claim boundary changed")

    gates = require_object(value["acceptance_gates"], "manifest.acceptance_gates")
    require_exact_keys(
        gates,
        {"minimum_cases", "minimum_control_cases", "minimum_real_incident_sources", "minimum_uncertain_cases"},
        "manifest.acceptance_gates",
    )
    for key in gates:
        require_int(gates[key], f"manifest.acceptance_gates.{key}", 1)

    assets = require_object(value["assets"], "manifest.assets")
    require_exact_keys(assets, {"cases", "oracle"}, "manifest.assets")
    for name in ["cases", "oracle"]:
        asset = require_object(assets[name], f"manifest.assets.{name}")
        require_exact_keys(asset, {"path", "sha256"}, f"manifest.assets.{name}")
        require(asset["path"] == f"{name}.json", f"manifest.assets.{name}.path changed")
        digest = require_string(asset["sha256"], f"manifest.assets.{name}.sha256")
        require(bool(SHA256_PATTERN.fullmatch(digest)), f"manifest.assets.{name}.sha256 is invalid")

    provenance = require_array(value["provenance"], "manifest.provenance")
    provenance_ids: set[str] = set()
    for index, source_value in enumerate(provenance):
        label = f"manifest.provenance[{index}]"
        source = require_object(source_value, label)
        require_exact_keys(source, {"id", "kind", "supports", "title", "url"}, label)
        source_id = require_string(source["id"], f"{label}.id")
        require(source_id not in provenance_ids, f"duplicate provenance id: {source_id}")
        provenance_ids.add(source_id)
        require(source["kind"] in {"official_documentation", "public_incident"}, f"{label}.kind is invalid")
        validate_string_array(source["supports"], f"{label}.supports")
        require_string(source["title"], f"{label}.title")
        url = require_string(source["url"], f"{label}.url")
        require(url.startswith("https://docs.github.com/") or url.startswith("https://github.com/"), f"{label}.url is not GitHub evidence")
    return provenance_ids


def validate_bundle_data(
    cases_bytes: bytes,
    cases_asset: dict[str, object],
    oracle_bytes: bytes,
    oracle: dict[str, object],
    manifest: dict[str, object],
) -> tuple[list[dict[str, object]], dict[str, object]]:
    provenance_ids = validate_manifest_shape(manifest)
    cases = validate_cases(cases_asset, provenance_ids)
    case_ids = {case["id"] for case in cases}
    required_case_ids = set(validate_string_array(manifest["required_case_ids"], "manifest.required_case_ids"))
    require(case_ids == required_case_ids, "required case ids differ from cases")
    validate_oracle(oracle, case_ids, ORACLE_SCHEMA)

    assets = manifest["assets"]
    assert isinstance(assets, dict)
    cases_asset_meta = assets["cases"]
    oracle_asset_meta = assets["oracle"]
    assert isinstance(cases_asset_meta, dict) and isinstance(oracle_asset_meta, dict)
    require(cases_asset_meta["sha256"] == sha256_bytes(cases_bytes), "manifest cases hash mismatch")
    require(oracle_asset_meta["sha256"] == sha256_bytes(oracle_bytes), "manifest oracle hash mismatch")

    derived = derive_oracle(cases)
    require(derived == oracle, "oracle differs from independent reference evaluation")
    summary = summarize(cases, oracle)
    require(summary == manifest["expected_summary"], "manifest expected summary differs")

    gates = manifest["acceptance_gates"]
    assert isinstance(gates, dict)
    require(summary["cases"] >= gates["minimum_cases"], "minimum case gate failed")
    causes = summary["causes"]
    confidence = summary["confidence"]
    assert isinstance(causes, dict) and isinstance(confidence, dict)
    require(causes["none"] >= gates["minimum_control_cases"], "minimum control gate failed")
    require(confidence["uncertain"] >= gates["minimum_uncertain_cases"], "minimum uncertain gate failed")
    public_incidents = sum(
        1
        for source in manifest["provenance"]
        if source["kind"] == "public_incident"
    )
    require(public_incidents >= gates["minimum_real_incident_sources"], "incident provenance gate failed")
    return cases, summary


def parse_checksum_text(value: str) -> dict[str, str]:
    entries: dict[str, str] = {}
    for line_number, line in enumerate(value.splitlines(), 1):
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9_.-]+)", line)
        require(match is not None, f"SHA256SUMS line {line_number} is malformed")
        assert match is not None
        digest, name = match.groups()
        require(name not in entries, f"SHA256SUMS repeats {name}")
        entries[name] = digest
    require(set(entries) == EXPECTED_CHECKSUM_FILES, "SHA256SUMS file set differs")
    return entries


def validate_checksums() -> None:
    entries = parse_checksum_text(CHECKSUMS.read_text(encoding="utf-8"))
    for name, digest in entries.items():
        require(sha256_file(BUNDLE / name) == digest, f"SHA256SUMS mismatch for {name}")


def load_bundle(
    cases_path: Path, oracle_path: Path, manifest_path: Path
) -> tuple[bytes, dict[str, object], bytes, dict[str, object], dict[str, object]]:
    cases_bytes, cases_asset = read_json(cases_path)
    oracle_bytes, oracle = read_json(oracle_path)
    _, manifest = read_json(manifest_path)
    return cases_bytes, cases_asset, oracle_bytes, oracle, manifest


def rebound_manifest(
    manifest: dict[str, object], cases_asset: dict[str, object], oracle: dict[str, object]
) -> dict[str, object]:
    rebound = copy.deepcopy(manifest)
    assets = rebound["assets"]
    assert isinstance(assets, dict)
    cases_meta = assets["cases"]
    oracle_meta = assets["oracle"]
    assert isinstance(cases_meta, dict) and isinstance(oracle_meta, dict)
    cases_meta["sha256"] = sha256_bytes(canonical_json(cases_asset))
    oracle_meta["sha256"] = sha256_bytes(canonical_json(oracle))
    return rebound


def validate_mutation(
    cases_asset: dict[str, object], oracle: dict[str, object], manifest: dict[str, object]
) -> None:
    rebound = rebound_manifest(manifest, cases_asset, oracle)
    validate_bundle_data(
        canonical_json(cases_asset),
        cases_asset,
        canonical_json(oracle),
        oracle,
        rebound,
    )


def expect_failure(action: Callable[[], object], label: str) -> None:
    failed = False
    try:
        action()
    except (BenchmarkError, json.JSONDecodeError):
        failed = True
    require(failed, f"self-test mutation unexpectedly passed: {label}")


def find_case(cases_asset: dict[str, object], case_id: str) -> dict[str, object]:
    cases = cases_asset["cases"]
    assert isinstance(cases, list)
    for case in cases:
        assert isinstance(case, dict)
        if case["id"] == case_id:
            return case
    raise BenchmarkError(f"missing mutation case: {case_id}")


def command_verify(arguments: argparse.Namespace) -> None:
    cases_bytes, cases_asset, oracle_bytes, oracle, manifest = load_bundle(
        arguments.cases, arguments.oracle, arguments.manifest
    )
    _, summary = validate_bundle_data(cases_bytes, cases_asset, oracle_bytes, oracle, manifest)
    validate_checksums()
    print(
        "verified Doctor Workflow Trigger v1: "
        f"{summary['cases']} cases, {summary['causes']['none']} controls, "
        f"{summary['confidence']['uncertain']} fail-closed unknowns"
    )


def command_score(arguments: argparse.Namespace) -> None:
    cases_bytes, cases_asset, oracle_bytes, oracle, manifest = load_bundle(
        arguments.cases, arguments.oracle, arguments.manifest
    )
    cases, _ = validate_bundle_data(cases_bytes, cases_asset, oracle_bytes, oracle, manifest)
    _, candidate = read_json(arguments.candidate)
    case_ids = {case["id"] for case in cases}
    validate_oracle(candidate, case_ids, PREDICTIONS_SCHEMA)
    require(candidate["cases"] == oracle["cases"], "candidate diagnoses differ from oracle")
    print(f"candidate passed: {len(case_ids)} exact diagnoses")


def command_self_test(arguments: argparse.Namespace) -> None:
    cases_bytes, cases_asset, oracle_bytes, oracle, manifest = load_bundle(
        arguments.cases, arguments.oracle, arguments.manifest
    )
    validate_bundle_data(cases_bytes, cases_asset, oracle_bytes, oracle, manifest)
    validate_checksums()

    forged = copy.deepcopy(oracle)
    forged["cases"]["merge-group-trigger-missing"]["cause_code"] = "none"
    expect_failure(lambda: validate_mutation(cases_asset, forged, manifest), "forged oracle cause")

    omitted = copy.deepcopy(cases_asset)
    omitted["cases"].pop()
    expect_failure(lambda: validate_mutation(omitted, oracle, manifest), "omitted case")

    extra_key = copy.deepcopy(cases_asset)
    extra_key["cases"][0]["unexpected"] = True
    expect_failure(lambda: validate_mutation(extra_key, oracle, manifest), "extra case key")

    leaked = copy.deepcopy(cases_asset)
    leaked["cases"][0]["input"]["cause_code"] = "merge_group_trigger_missing"
    expect_failure(lambda: validate_mutation(leaked, oracle, manifest), "oracle leaked into cases")

    added_trigger = copy.deepcopy(cases_asset)
    trigger_case = find_case(added_trigger, "merge-group-trigger-missing")
    trigger_case["input"]["workflows"][0]["triggers"]["merge_group"] = {"types": ["checks_requested"]}
    expect_failure(lambda: validate_mutation(added_trigger, oracle, manifest), "missing merge-group trigger repaired")

    swapped_branch = copy.deepcopy(cases_asset)
    branch_case = find_case(swapped_branch, "branch-filter-excluded")
    branch_case["input"]["pull_request"]["base_ref"] = "main"
    expect_failure(lambda: validate_mutation(swapped_branch, oracle, manifest), "base and head semantics collapsed")

    skipped_as_missing = copy.deepcopy(oracle)
    skipped_as_missing["cases"]["job-condition-skipped-control"]["cause_code"] = "workflow_path_filter_excluded"
    expect_failure(lambda: validate_mutation(cases_asset, skipped_as_missing, manifest), "skipped job called missing")

    fork_overclaim = copy.deepcopy(oracle)
    fork_overclaim["cases"]["fork-approval-possible"]["confidence"] = "certain"
    expect_failure(lambda: validate_mutation(cases_asset, fork_overclaim, manifest), "fork possibility promoted to certain")

    dynamic_rename = copy.deepcopy(oracle)
    dynamic_rename["cases"]["dynamic-job-name-unknown"]["cause_code"] = "required_context_not_produced"
    expect_failure(lambda: validate_mutation(cases_asset, dynamic_rename, manifest), "dynamic name miscalled rename")

    false_complete = copy.deepcopy(cases_asset)
    large_case = find_case(false_complete, "path-filter-over-300-unknown")
    large_case["input"]["changed_files"]["github_filter_file_limit_reached"] = False
    expect_failure(lambda: validate_mutation(false_complete, oracle, manifest), "300-file boundary erased")

    weak_boundary = copy.deepcopy(manifest)
    weak_boundary["claim_boundary"]["production_accuracy_supported"] = True
    expect_failure(
        lambda: validate_bundle_data(cases_bytes, cases_asset, oracle_bytes, oracle, weak_boundary),
        "claim boundary weakened",
    )

    stale_hash = copy.deepcopy(manifest)
    stale_hash["assets"]["cases"]["sha256"] = "0" * 64
    expect_failure(
        lambda: validate_bundle_data(cases_bytes, cases_asset, oracle_bytes, oracle, stale_hash),
        "stale manifest hash",
    )

    missing_source = copy.deepcopy(manifest)
    missing_source["provenance"].pop()
    expect_failure(
        lambda: validate_bundle_data(cases_bytes, cases_asset, oracle_bytes, oracle, missing_source),
        "provenance source removed",
    )

    expect_failure(
        lambda: json.loads(
            '{"schema":"one","schema":"two"}',
            object_pairs_hook=unique_json_object,
        ),
        "duplicate JSON key",
    )

    checksum_text = CHECKSUMS.read_text(encoding="utf-8").replace("README.md", "README-copy.md", 1)
    expect_failure(lambda: parse_checksum_text(checksum_text), "checksum file substitution")

    print("self-test passed: 15 integrity and fail-closed mutations rejected")


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser(description=__doc__)
    value.add_argument("--cases", type=Path, default=DEFAULT_CASES)
    value.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    value.add_argument("--oracle", type=Path, default=DEFAULT_ORACLE)
    subcommands = value.add_subparsers(dest="command", required=True)
    subcommands.add_parser("verify").set_defaults(handler=command_verify)
    subcommands.add_parser("self-test").set_defaults(handler=command_self_test)
    score = subcommands.add_parser("score")
    score.add_argument("candidate", type=Path)
    score.set_defaults(handler=command_score)
    return value


def main() -> None:
    arguments = parser().parse_args()
    arguments.handler(arguments)


if __name__ == "__main__":
    main()
