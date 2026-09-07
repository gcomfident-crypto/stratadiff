#!/usr/bin/env python3
"""Verify the frozen Merge Readiness Audit v1 diagnostic bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re


BUNDLE = Path(__file__).resolve().parent
MANIFEST = BUNDLE / "manifest.json"
REPORTS = BUNDLE / "reports"
MANIFEST_SCHEMA = "stratadiff-merge-readiness-benchmark-manifest-v1"
REPORT_SCHEMA = "stratadiff-merge-readiness-audit-v1"
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
OID_PATTERN = re.compile(r"^[0-9a-f]{40}$")
TIMESTAMP_PATTERN = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")


class BenchmarkError(RuntimeError):
    """A frozen benchmark asset violates the bundle contract."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise BenchmarkError(message)


def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        require(key not in value, f"duplicate JSON key: {key}")
        value[key] = item
    return value


def read_object(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object)
    require(type(value) is dict, f"{path} must contain one JSON object")
    return value


def require_object(value: object, label: str) -> dict[str, object]:
    require(type(value) is dict, f"{label} must be an object")
    return value


def require_array(value: object, label: str) -> list[object]:
    require(type(value) is list, f"{label} must be an array")
    return value


def require_string(value: object, label: str) -> str:
    require(type(value) is str and bool(value), f"{label} must be a non-empty string")
    return value


def require_int(value: object, label: str) -> int:
    require(type(value) is int and value >= 0, f"{label} must be a non-negative integer")
    return value


def require_exact_keys(value: dict[str, object], keys: set[str], label: str) -> None:
    require(set(value) == keys, f"{label} fields differ: {sorted(set(value) ^ keys)}")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_evidence(value: object, label: str) -> None:
    evidence = require_array(value, label)
    require(bool(evidence), f"{label} must not be empty")
    for index, item in enumerate(evidence):
        record = require_object(item, f"{label}[{index}]")
        require_exact_keys(record, {"description", "kind", "url"}, f"{label}[{index}]")
        require_string(record["kind"], f"{label}[{index}].kind")
        url = require_string(record["url"], f"{label}[{index}].url")
        require(url.startswith("https://github.com/"), f"{label}[{index}].url is not GitHub")
        require_string(record["description"], f"{label}[{index}].description")


def validate_report(
    report: dict[str, object], asset: dict[str, object], tool_version: str
) -> dict[str, int]:
    require_exact_keys(
        report,
        {
            "claim_boundary",
            "collection",
            "findings",
            "generated_at",
            "privacy",
            "schema",
            "scope",
            "summary",
            "tool_version",
            "unknowns",
        },
        asset["path"],
    )
    require(report["schema"] == REPORT_SCHEMA, f"{asset['path']} schema changed")
    require(report["tool_version"] == tool_version, f"{asset['path']} tool version changed")
    generated_at = require_string(report["generated_at"], f"{asset['path']}.generated_at")
    require(bool(TIMESTAMP_PATTERN.fullmatch(generated_at)), f"{asset['path']} timestamp invalid")

    scope = require_object(report["scope"], f"{asset['path']}.scope")
    require_exact_keys(
        scope,
        {
            "default_branch",
            "default_branch_head_sha",
            "provider_url",
            "repository",
            "sampled_pull_requests",
        },
        f"{asset['path']}.scope",
    )
    require(scope["provider_url"] == "https://github.com", f"{asset['path']} provider changed")
    require(scope["repository"] == asset["repository"], f"{asset['path']} repository changed")
    require_string(scope["default_branch"], f"{asset['path']}.scope.default_branch")
    head = require_string(scope["default_branch_head_sha"], f"{asset['path']}.scope.head")
    require(bool(OID_PATTERN.fullmatch(head)), f"{asset['path']} head SHA invalid")
    sampled_pull_requests = require_int(
        scope["sampled_pull_requests"], f"{asset['path']}.scope.sampled_pull_requests"
    )

    collection = require_object(report["collection"], f"{asset['path']}.collection")
    require_exact_keys(
        collection, {"api_calls", "gaps", "response_bytes", "status"}, f"{asset['path']}.collection"
    )
    require(collection["status"] == asset["collection_status"], f"{asset['path']} status changed")
    api_calls = require_int(collection["api_calls"], f"{asset['path']}.collection.api_calls")
    response_bytes = require_int(
        collection["response_bytes"], f"{asset['path']}.collection.response_bytes"
    )
    gaps = require_array(collection["gaps"], f"{asset['path']}.collection.gaps")
    for index, item in enumerate(gaps):
        gap = require_object(item, f"{asset['path']}.collection.gaps[{index}]")
        require_exact_keys(gap, {"reason", "surface"}, f"{asset['path']}.collection.gaps[{index}]")
        require_string(gap["reason"], f"{asset['path']}.collection.gaps[{index}].reason")
        require_string(gap["surface"], f"{asset['path']}.collection.gaps[{index}].surface")

    privacy = require_object(report["privacy"], f"{asset['path']}.privacy")
    require_exact_keys(
        privacy,
        {
            "check_run_output_text_collected",
            "commit_messages_collected",
            "commit_status_description_collected",
            "pull_request_text_collected",
            "repository_source_collected",
            "review_text_collected",
            "workflow_definitions_collected",
        },
        f"{asset['path']}.privacy",
    )
    require(all(type(item) is bool for item in privacy.values()), f"{asset['path']} privacy flags invalid")
    require(privacy["repository_source_collected"] is False, f"{asset['path']} persisted source")
    require(privacy["review_text_collected"] is False, f"{asset['path']} persisted review text")

    boundary = require_object(report["claim_boundary"], f"{asset['path']}.claim_boundary")
    require_exact_keys(
        boundary,
        {
            "cost_savings_supported",
            "current_configuration_snapshot_supported",
            "historical_configuration_at_merge_supported",
            "merge_safety_supported",
            "workflow_runtime_semantics_supported",
        },
        f"{asset['path']}.claim_boundary",
    )
    require(boundary["current_configuration_snapshot_supported"] is True, f"{asset['path']} claim changed")
    for key in boundary:
        if key != "current_configuration_snapshot_supported":
            require(boundary[key] is False, f"{asset['path']} overclaims {key}")

    findings = require_array(report["findings"], f"{asset['path']}.findings")
    for index, item in enumerate(findings):
        finding = require_object(item, f"{asset['path']}.findings[{index}]")
        require_exact_keys(
            finding,
            {
                "contexts",
                "evidence",
                "explanation",
                "remediation",
                "rule",
                "rulesets",
                "severity",
                "title",
                "workflow_paths",
            },
            f"{asset['path']}.findings[{index}]",
        )
        require(finding["severity"] in {"high", "medium"}, f"{asset['path']} severity invalid")
        validate_evidence(finding["evidence"], f"{asset['path']}.findings[{index}].evidence")

    unknowns = require_array(report["unknowns"], f"{asset['path']}.unknowns")
    source_unresolved = 0
    unknown_codes: set[str] = set()
    for index, item in enumerate(unknowns):
        unknown = require_object(item, f"{asset['path']}.unknowns[{index}]")
        require_exact_keys(
            unknown,
            {"code", "context", "evidence", "reason", "workflow_path"},
            f"{asset['path']}.unknowns[{index}]",
        )
        code = require_string(unknown["code"], f"{asset['path']}.unknowns[{index}].code")
        unknown_codes.add(code)
        source_unresolved += int(code == "required_check_source_unresolved")
        validate_evidence(unknown["evidence"], f"{asset['path']}.unknowns[{index}].evidence")

    summary = require_object(report["summary"], f"{asset['path']}.summary")
    require_exact_keys(
        summary,
        {
            "findings",
            "fully_observed_pull_requests",
            "high_findings",
            "medium_findings",
            "required_checks",
            "sampled_pull_requests",
            "unknowns",
            "verdict",
        },
        f"{asset['path']}.summary",
    )
    require(summary["verdict"] == asset["verdict"], f"{asset['path']} verdict changed")
    require(summary["findings"] == len(findings), f"{asset['path']} finding count disagrees")
    require(summary["unknowns"] == len(unknowns), f"{asset['path']} unknown count disagrees")
    require(
        summary["high_findings"] == sum(item["severity"] == "high" for item in findings),
        f"{asset['path']} high count disagrees",
    )
    require(
        summary["medium_findings"] == sum(item["severity"] == "medium" for item in findings),
        f"{asset['path']} medium count disagrees",
    )
    require(summary["sampled_pull_requests"] == sampled_pull_requests, f"{asset['path']} PR count disagrees")
    required_checks = require_int(summary["required_checks"], f"{asset['path']}.summary.required_checks")

    if collection["status"] == "partial":
        require(summary["verdict"] == "inconclusive", f"{asset['path']} partial scan is not inconclusive")
        require(bool(gaps), f"{asset['path']} partial scan has no gap")
        require("collection_incomplete" in unknown_codes, f"{asset['path']} hides incomplete collection")
    else:
        require(collection["status"] == "complete", f"{asset['path']} collection status invalid")
        require(not gaps, f"{asset['path']} complete scan has gaps")

    return {
        "action_required": int(summary["verdict"] == "action_required"),
        "admin_permission_reports": int(asset["viewer_permission"] == "ADMIN"),
        "api_calls": api_calls,
        "complete": int(collection["status"] == "complete"),
        "findings": len(findings),
        "inconclusive": int(summary["verdict"] == "inconclusive"),
        "partial": int(collection["status"] == "partial"),
        "read_permission_reports": int(asset["viewer_permission"] == "READ"),
        "reports": 1,
        "required_checks": required_checks,
        "response_bytes": response_bytes,
        "sampled_pull_requests": sampled_pull_requests,
        "source_unresolved_unknowns": source_unresolved,
        "unknowns": len(unknowns),
    }


def verify() -> dict[str, int]:
    manifest = read_object(MANIFEST)
    require_exact_keys(
        manifest,
        {
            "assets",
            "build",
            "capture",
            "claim_boundary",
            "dataset_version",
            "description",
            "expected_summary",
            "license",
            "name",
            "schema",
            "selection",
        },
        "manifest",
    )
    require(manifest["schema"] == MANIFEST_SCHEMA, "manifest schema changed")
    require(manifest["dataset_version"] == "1.0.0", "dataset version changed")
    require_string(manifest["name"], "manifest.name")
    require_string(manifest["description"], "manifest.description")
    require_string(manifest["license"], "manifest.license")
    boundaries = require_array(manifest["claim_boundary"], "manifest.claim_boundary")
    require(len(boundaries) >= 5, "manifest claim boundary is incomplete")
    for index, boundary in enumerate(boundaries):
        require_string(boundary, f"manifest.claim_boundary[{index}]")

    build = require_object(manifest["build"], "manifest.build")
    require_exact_keys(build, {"cargo_lock_sha256", "git_revision", "tool_version"}, "manifest.build")
    tool_version = require_string(build["tool_version"], "manifest.build.tool_version")
    require(bool(OID_PATTERN.fullmatch(require_string(build["git_revision"], "manifest.build.git_revision"))), "git revision invalid")
    require(
        bool(SHA256_PATTERN.fullmatch(require_string(build["cargo_lock_sha256"], "manifest.build.cargo_lock_sha256"))),
        "Cargo.lock digest invalid",
    )

    capture = require_object(manifest["capture"], "manifest.capture")
    require_exact_keys(
        capture,
        {"command_template", "provider_url", "window_end", "window_start"},
        "manifest.capture",
    )
    require_string(capture["command_template"], "manifest.capture.command_template")
    require(capture["provider_url"] == "https://github.com", "capture provider changed")
    window_start = require_string(capture["window_start"], "manifest.capture.window_start")
    window_end = require_string(capture["window_end"], "manifest.capture.window_end")
    require(bool(TIMESTAMP_PATTERN.fullmatch(window_start)), "capture window start invalid")
    require(bool(TIMESTAMP_PATTERN.fullmatch(window_end)), "capture window end invalid")
    require(window_start <= window_end, "capture window is reversed")

    selection = require_object(manifest["selection"], "manifest.selection")
    require_exact_keys(
        selection,
        {"designation", "external_repository_count", "owned_control_count", "randomized"},
        "manifest.selection",
    )
    require(selection["randomized"] is False, "selection must not claim randomization")

    assets = require_array(manifest["assets"], "manifest.assets")
    require(len(assets) == 13, "manifest must bind exactly 13 reports")
    totals = {key: 0 for key in require_object(manifest["expected_summary"], "manifest.expected_summary")}
    paths: set[str] = set()
    repositories: set[str] = set()
    for index, item in enumerate(assets):
        asset = require_object(item, f"manifest.assets[{index}]")
        require_exact_keys(
            asset,
            {"collection_status", "path", "repository", "sha256", "verdict", "viewer_permission"},
            f"manifest.assets[{index}]",
        )
        relative_path = require_string(asset["path"], f"manifest.assets[{index}].path")
        path = BUNDLE / relative_path
        require(path.parent == REPORTS, f"asset path escapes reports/: {relative_path}")
        require(relative_path not in paths, f"duplicate asset path: {relative_path}")
        paths.add(relative_path)
        repository = require_string(asset["repository"], f"manifest.assets[{index}].repository")
        require(repository not in repositories, f"duplicate repository: {repository}")
        repositories.add(repository)
        expected_hash = require_string(asset["sha256"], f"manifest.assets[{index}].sha256")
        require(bool(SHA256_PATTERN.fullmatch(expected_hash)), f"{relative_path} digest invalid")
        require(path.is_file(), f"missing report: {relative_path}")
        require(sha256(path) == expected_hash, f"report digest changed: {relative_path}")
        require(asset["viewer_permission"] in {"ADMIN", "READ"}, f"{relative_path} permission invalid")
        report_totals = validate_report(read_object(path), asset, tool_version)
        require(set(report_totals) == set(totals), f"{relative_path} aggregate fields differ")
        for key, value in report_totals.items():
            totals[key] += value

    actual_files = {f"reports/{path.name}" for path in REPORTS.glob("*.json")}
    require(actual_files == paths, "reports/ contains an unbound or missing JSON report")
    expected_summary = require_object(manifest["expected_summary"], "manifest.expected_summary")
    require(totals == expected_summary, f"aggregate changed: {totals!r}")
    require(selection["external_repository_count"] == totals["read_permission_reports"], "external count changed")
    require(selection["owned_control_count"] == totals["admin_permission_reports"], "control count changed")
    return totals


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("summary", "verify"))
    args = parser.parse_args()
    totals = verify()
    if args.command == "summary":
        print(json.dumps(totals, indent=2, sort_keys=True))
    else:
        print(f"verified {totals['reports']} merge-readiness reports")


if __name__ == "__main__":
    main()
