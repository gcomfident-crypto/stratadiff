#!/usr/bin/env python3

import argparse
import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SOURCE = (
    ROOT / "benchmarks" / "resumebench-github-live-v1" / "evaluation-v1.0.0.json"
)
DEFAULT_EVALUATION = (
    ROOT / "benchmarks" / "reviewer-value-v1" / "evaluation-v1.0.0.json"
)
SOURCE_SCHEMA = "stratadiff-resumebench-github-live-evaluation-v1"
EVALUATION_SCHEMA = "stratadiff-reviewer-value-evaluation-v1"
MAX_INPUT_BYTES = 16 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def unique_json_object(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value, f"duplicate JSON key: {key}")
        value[key] = item
    return value


def read_json(path):
    payload = path.read_bytes()
    require(len(payload) <= MAX_INPUT_BYTES, f"input exceeds {MAX_INPUT_BYTES} bytes: {path}")
    return payload, json.loads(payload, object_pairs_hook=unique_json_object)


def ratio_basis_points(numerator, denominator):
    require(denominator > 0, "ratio denominator must be positive")
    return (numerator * 10_000 + denominator // 2) // denominator


def validate_source(source):
    require(source["schema"] == SOURCE_SCHEMA, "unsupported source evaluation schema")
    require(source["dataset_version"] == "1.0.0", "unsupported source dataset version")
    require(source["benchmark_complete"] is True, "source benchmark is incomplete")
    require(source["cases"], "source benchmark has no cases")
    require(
        source["summary"]["passed_cases"] == source["summary"]["cases"],
        "source benchmark contains a failed case",
    )
    require(
        len(source["cases"]) == source["summary"]["cases"],
        "source case count differs from its summary",
    )


def compute_evaluation(source_bytes, source):
    validate_source(source)
    cases = []
    totals = {
        "current_pr_files": 0,
        "current_files_carried": 0,
        "current_files_needing_review": 0,
        "retired_residue_files": 0,
        "exact_resume_queue_items": 0,
        "naive_snapshot_paths": 0,
        "naive_extra_paths": 0,
        "naive_missing_current_paths": 0,
    }
    cases_with_current_carry = 0
    cases_with_no_current_carry = 0
    for source_case in source["cases"]:
        summary = source_case["summary"]
        current_files = summary["current_pr_files"]
        carried = summary["carried"]
        needs_review = summary["needs_review_now"]
        retired = summary["retired_checkpoint_changes"]
        naive_paths = summary["naive_snapshot_paths"]
        queue_items = needs_review + retired
        require(
            carried + needs_review == current_files,
            f"{source_case['id']} current-file partition is inconsistent",
        )
        require(not source_case["false_carry"], f"{source_case['id']} has a false carry")
        require(not source_case["false_invalidation"], f"{source_case['id']} has a false invalidation")
        require(source_case["passed"] is True, f"{source_case['id']} did not pass")
        if carried > 0:
            cases_with_current_carry += 1
        else:
            cases_with_no_current_carry += 1
        case = {
            "id": source_case["id"],
            "current_pr_files": current_files,
            "current_files_carried": carried,
            "current_files_needing_review": needs_review,
            "current_recheck_reduction_basis_points": ratio_basis_points(
                carried, current_files
            ),
            "retired_residue_files": retired,
            "exact_resume_queue_items": queue_items,
            "naive_snapshot_paths": naive_paths,
            "naive_extra_paths": summary["naive_extra_paths"],
            "naive_missing_current_paths": summary["naive_missing_current_paths"],
            "snapshot_path_count_minus_exact_queue": naive_paths - queue_items,
        }
        cases.append(case)
        for key in totals:
            totals[key] += case[key]

    totals["cases"] = len(cases)
    totals["cases_with_current_carry"] = cases_with_current_carry
    totals["cases_with_no_current_carry"] = cases_with_no_current_carry
    totals["current_recheck_reduction_basis_points"] = ratio_basis_points(
        totals["current_files_carried"], totals["current_pr_files"]
    )
    totals["snapshot_path_count_minus_exact_queue"] = (
        totals["naive_snapshot_paths"] - totals["exact_resume_queue_items"]
    )
    totals["snapshot_path_count_reduction_basis_points"] = ratio_basis_points(
        totals["snapshot_path_count_minus_exact_queue"],
        totals["naive_snapshot_paths"],
    )
    return {
        "schema": EVALUATION_SCHEMA,
        "dataset_version": "1.0.0",
        "source": {
            "schema": source["schema"],
            "dataset_version": source["dataset_version"],
            "sha256": hashlib.sha256(source_bytes).hexdigest(),
        },
        "claim_boundary": {
            "sample": "five purposefully selected public GitHub force-push histories",
            "measures": "deterministic file-level review-surface counts",
            "human_time_savings_measured": False,
            "defect_recall_measured": False,
            "population_estimates_supported": False,
        },
        "cases": cases,
        "summary": totals,
    }


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode(
        "utf-8"
    )


def parse_arguments():
    parser = argparse.ArgumentParser(
        description="Derive honest reviewer-surface metrics from ResumeBench-GitHub-Live v1"
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    evaluate = subparsers.add_parser("evaluate")
    evaluate.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    evaluate.add_argument("--output", type=Path, required=True)
    verify = subparsers.add_parser("verify")
    verify.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    verify.add_argument("--evaluation", type=Path, default=DEFAULT_EVALUATION)
    return parser.parse_args()


def main():
    arguments = parse_arguments()
    source_bytes, source = read_json(arguments.source)
    evaluation = compute_evaluation(source_bytes, source)
    if arguments.command == "evaluate":
        arguments.output.write_bytes(encoded(evaluation))
        print(f"wrote reviewer-value evaluation to {arguments.output}")
        return
    evaluation_bytes, observed = read_json(arguments.evaluation)
    require(observed == evaluation, "reviewer-value evaluation differs from recomputation")
    require(evaluation_bytes == encoded(observed), "evaluation JSON is not canonical")
    print(
        "verified reviewer-value v1: "
        f"{observed['summary']['current_files_carried']}/"
        f"{observed['summary']['current_pr_files']} current files carried; "
        f"{observed['summary']['naive_extra_paths']} naive extra paths exposed"
    )


if __name__ == "__main__":
    main()
