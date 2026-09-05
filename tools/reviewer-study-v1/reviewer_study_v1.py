#!/usr/bin/env python3

import argparse
import copy
import hashlib
import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_PREREGISTRATION = ROOT / "benchmarks/reviewer-study-v1/preregistration.json"
DATA_SCHEMA = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/benchmarks/reviewer-study-v1/study-data.schema.json"
PREREGISTRATION_SCHEMA = "stratadiff-reviewer-study-preregistration-v1"
AGGREGATE_SCHEMA = "stratadiff-reviewer-study-aggregate-v1"
MAX_INPUT_BYTES = 32 * 1024 * 1024
PARTICIPANT_ID = re.compile(r"p_[0-9a-f]{12}")
PAIR_ID = re.compile(r"pair_[0-9a-f]{12}")
TASK_ID = re.compile(r"task_[0-9a-f]{12}")
STUDY_ID = re.compile(r"study_[a-z0-9][a-z0-9_-]{2,63}")
ASSIGNMENT_CELLS = (
    "baseline_then_resume:a",
    "baseline_then_resume:b",
    "resume_then_baseline:a",
    "resume_then_baseline:b",
)


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


def require_object(value, keys, label):
    require(type(value) is dict, f"{label} must be an object")
    expected = set(keys)
    observed = set(value)
    require(not (expected - observed), f"{label} is missing required fields: {sorted(expected - observed)}")
    require(not (observed - expected), f"{label} has unknown fields: {sorted(observed - expected)}")


def require_string(value, label):
    require(type(value) is str and value != "", f"{label} must be a non-empty string")


def require_boolean(value, label):
    require(type(value) is bool, f"{label} must be a boolean")


def require_integer(value, minimum, maximum, label):
    require(type(value) is int, f"{label} must be an integer")
    require(minimum <= value <= maximum, f"{label} must be between {minimum} and {maximum}")


def require_identifier(value, pattern, label):
    require_string(value, label)
    require(pattern.fullmatch(value) is not None, f"{label} is not a privacy-safe opaque identifier")


def sha256(payload):
    return hashlib.sha256(payload).hexdigest()


def validate_preregistration(preregistration):
    require_object(
        preregistration,
        {"schema", "protocol_version", "design", "minimums", "gates", "analysis", "privacy"},
        "preregistration",
    )
    require(preregistration["schema"] == PREREGISTRATION_SCHEMA, "unsupported preregistration schema")
    require(preregistration["protocol_version"] == "1.0.0", "unsupported protocol version")

    design = preregistration["design"]
    require_object(
        design,
        {"assignment", "unit_of_analysis", "primary_population", "follow_up_window_days", "counterbalance"},
        "preregistration.design",
    )
    require(
        design["assignment"] == "randomized_counterbalanced_within_reviewer_matched_task_pairs",
        "unsupported assignment design",
    )
    require(design["unit_of_analysis"] == "completed_baseline_resume_pair", "unsupported unit of analysis")
    require(
        design["primary_population"] == "all_schema_valid_complete_pairs_locked_before_analysis",
        "unsupported primary population",
    )
    require_integer(design["follow_up_window_days"], 1, 365, "follow_up_window_days")
    counterbalance = design["counterbalance"]
    require_object(
        counterbalance,
        {
            "cell_definition",
            "global_max_cell_count_difference",
            "participant_max_cell_count_difference",
            "task_family_max_cell_count_difference",
            "minimum_observations_per_task_family",
            "participant_task_family_reuse",
        },
        "preregistration.design.counterbalance",
    )
    require(
        counterbalance["cell_definition"] == "assignment_order_crossed_with_baseline_variant",
        "unsupported counterbalance cell definition",
    )
    for key in (
        "global_max_cell_count_difference",
        "participant_max_cell_count_difference",
        "task_family_max_cell_count_difference",
    ):
        require_integer(counterbalance[key], 1, 1, f"counterbalance {key}")
    require_integer(
        counterbalance["minimum_observations_per_task_family"],
        4,
        4,
        "counterbalance minimum_observations_per_task_family",
    )
    require(
        counterbalance["participant_task_family_reuse"] == "forbidden",
        "participant task-family reuse must be forbidden",
    )

    minimums = preregistration["minimums"]
    require_object(
        minimums,
        {"eligible_pairs", "unique_participants", "adjudicated_carry_units"},
        "preregistration.minimums",
    )
    require_integer(minimums["eligible_pairs"], 1, 10000, "minimum eligible_pairs")
    require_integer(minimums["unique_participants"], 1, 500, "minimum unique_participants")
    require_integer(minimums["adjudicated_carry_units"], 1, 1000000, "minimum adjudicated_carry_units")

    gates = preregistration["gates"]
    require_object(
        gates,
        {
            "median_completion_reduction_basis_points",
            "median_reopened_files_reduction_basis_points",
            "issue_recall_noninferiority_margin_basis_points",
            "maximum_confirmed_false_carries",
            "repeat_use_rate_basis_points",
        },
        "preregistration.gates",
    )
    for key in (
        "median_completion_reduction_basis_points",
        "median_reopened_files_reduction_basis_points",
        "issue_recall_noninferiority_margin_basis_points",
        "repeat_use_rate_basis_points",
    ):
        require_integer(gates[key], 0, 10000, f"gate {key}")
    require_integer(
        gates["maximum_confirmed_false_carries"],
        0,
        100000,
        "gate maximum_confirmed_false_carries",
    )

    analysis = preregistration["analysis"]
    require_object(
        analysis,
        {
            "completion_and_reopen_reduction",
            "issue_recall",
            "false_carry_unit",
            "reopened_lines_role",
            "missing_data",
        },
        "preregistration.analysis",
    )
    for key in analysis:
        require_string(analysis[key], f"preregistration.analysis.{key}")
    require(analysis["false_carry_unit"] == "carried_file_change", "unsupported false-carry unit")

    privacy = preregistration["privacy"]
    require_object(
        privacy,
        {"identifier_policy", "prohibited_fields", "retained_granularity"},
        "preregistration.privacy",
    )
    require_string(privacy["identifier_policy"], "preregistration.privacy.identifier_policy")
    require_string(privacy["retained_granularity"], "preregistration.privacy.retained_granularity")
    require(type(privacy["prohibited_fields"]) is list, "prohibited_fields must be an array")
    require(privacy["prohibited_fields"], "prohibited_fields must not be empty")
    for index, field in enumerate(privacy["prohibited_fields"]):
        require_string(field, f"prohibited_fields[{index}]")
    require(
        len(privacy["prohibited_fields"]) == len(set(privacy["prohibited_fields"])),
        "prohibited_fields contains duplicates",
    )


def validate_measurement(measurement, label, baseline):
    require_object(
        measurement,
        {"completion_seconds", "issues_found", "seeded_issues", "reopened_files", "reopened_lines"},
        label,
    )
    require_integer(measurement["completion_seconds"], 1, 86400, f"{label}.completion_seconds")
    require_integer(measurement["issues_found"], 0, 10000, f"{label}.issues_found")
    require_integer(measurement["seeded_issues"], 1, 10000, f"{label}.seeded_issues")
    require(
        measurement["issues_found"] <= measurement["seeded_issues"],
        f"{label}.issues_found exceeds seeded_issues",
    )
    minimum_reopened = 1 if baseline else 0
    require_integer(measurement["reopened_files"], minimum_reopened, 100000, f"{label}.reopened_files")
    require_integer(measurement["reopened_lines"], minimum_reopened, 100000000, f"{label}.reopened_lines")


def assignment_cell(observation):
    return f"{observation['assignment_order']}:{observation['baseline_variant']}"


def assignment_cell_counts(observations):
    counts = {cell: 0 for cell in ASSIGNMENT_CELLS}
    for observation in observations:
        cell = assignment_cell(observation)
        require(cell in counts, f"unsupported assignment cell: {cell}")
        counts[cell] += 1
    return counts


def cell_count_difference(observations):
    counts = assignment_cell_counts(observations)
    return max(counts.values()) - min(counts.values())


def observations_by(observations, field):
    grouped = {}
    for observation in observations:
        value = observation[field]
        if value not in grouped:
            grouped[value] = []
        grouped[value].append(observation)
    return grouped


def validate_counterbalance(data, preregistration):
    observations = data["paired_observations"]
    contract = preregistration["design"]["counterbalance"]
    require(
        cell_count_difference(observations) <= contract["global_max_cell_count_difference"],
        "global assignment cells are not counterbalanced",
    )
    participant_groups = observations_by(observations, "participant_id")
    for participant_id, participant_observations in sorted(participant_groups.items()):
        require(
            cell_count_difference(participant_observations)
            <= contract["participant_max_cell_count_difference"],
            f"participant assignment cells are not counterbalanced: {participant_id}",
        )
    task_family_groups = observations_by(observations, "task_family_id")
    for task_family_id, task_family_observations in sorted(task_family_groups.items()):
        require(
            len(task_family_observations) >= contract["minimum_observations_per_task_family"],
            f"task family has too few observations for counterbalancing: {task_family_id}",
        )
        require(
            cell_count_difference(task_family_observations)
            <= contract["task_family_max_cell_count_difference"],
            f"task-family assignment cells are not counterbalanced: {task_family_id}",
        )


def validate_dataset(data, preregistration_bytes, preregistration):
    validate_preregistration(preregistration)
    require_object(
        data,
        {
            "schema",
            "study_id",
            "protocol_version",
            "preregistration_sha256",
            "synthetic",
            "collection_status",
            "participants",
            "paired_observations",
        },
        "study dataset",
    )
    require(data["schema"] == DATA_SCHEMA, "unsupported study data schema")
    require_identifier(data["study_id"], STUDY_ID, "study_id")
    require(data["protocol_version"] == preregistration["protocol_version"], "dataset protocol version mismatch")
    require(
        data["preregistration_sha256"] == sha256(preregistration_bytes),
        "dataset preregistration SHA-256 mismatch",
    )
    require_boolean(data["synthetic"], "synthetic")
    require(data["collection_status"] in ("open", "locked"), "unsupported collection_status")
    if data["synthetic"]:
        require(data["study_id"].startswith("study_synthetic_"), "synthetic study_id must start with study_synthetic_")

    participants = data["participants"]
    require(type(participants) is list and participants, "participants must be a non-empty array")
    require(len(participants) <= 500, "participants exceeds 500 entries")
    participant_ids = set()
    for index, participant in enumerate(participants):
        label = f"participants[{index}]"
        require_object(participant, {"participant_id", "repeat_use"}, label)
        require_identifier(participant["participant_id"], PARTICIPANT_ID, f"{label}.participant_id")
        require(participant["participant_id"] not in participant_ids, "duplicate participant_id")
        participant_ids.add(participant["participant_id"])
        repeat_use = participant["repeat_use"]
        require_object(
            repeat_use,
            {"invited_again", "follow_up_complete", "used_within_28_days"},
            f"{label}.repeat_use",
        )
        for key in ("invited_again", "follow_up_complete", "used_within_28_days"):
            require_boolean(repeat_use[key], f"{label}.repeat_use.{key}")
        require(
            not repeat_use["used_within_28_days"]
            or (repeat_use["invited_again"] and repeat_use["follow_up_complete"]),
            f"{label}.repeat_use records use without a completed invitation window",
        )
        if data["collection_status"] == "locked":
            require(repeat_use["invited_again"], f"{label}.repeat_use invitation is incomplete")
            require(repeat_use["follow_up_complete"], f"{label}.repeat_use follow-up is incomplete")

    observations = data["paired_observations"]
    require(type(observations) is list and observations, "paired_observations must be a non-empty array")
    require(len(observations) <= 10000, "paired_observations exceeds 10000 entries")
    pair_ids = set()
    participant_task_families = set()
    referenced_participants = set()
    for index, observation in enumerate(observations):
        label = f"paired_observations[{index}]"
        require_object(
            observation,
            {
                "pair_id",
                "participant_id",
                "task_family_id",
                "assignment_order",
                "baseline_variant",
                "resume_variant",
                "baseline",
                "resume",
                "false_carry_adjudication",
            },
            label,
        )
        require_identifier(observation["pair_id"], PAIR_ID, f"{label}.pair_id")
        require(observation["pair_id"] not in pair_ids, "duplicate pair_id")
        pair_ids.add(observation["pair_id"])
        require_identifier(observation["participant_id"], PARTICIPANT_ID, f"{label}.participant_id")
        require(observation["participant_id"] in participant_ids, f"{label} references an unknown participant")
        referenced_participants.add(observation["participant_id"])
        require_identifier(observation["task_family_id"], TASK_ID, f"{label}.task_family_id")
        participant_task_family = (observation["participant_id"], observation["task_family_id"])
        require(
            participant_task_family not in participant_task_families,
            f"{label} repeats a task family for one participant",
        )
        participant_task_families.add(participant_task_family)
        require(
            observation["assignment_order"] in ("baseline_then_resume", "resume_then_baseline"),
            f"{label}.assignment_order is unsupported",
        )
        require(observation["baseline_variant"] in ("a", "b"), f"{label}.baseline_variant is unsupported")
        require(observation["resume_variant"] in ("a", "b"), f"{label}.resume_variant is unsupported")
        require(observation["baseline_variant"] != observation["resume_variant"], f"{label} reuses one task variant")
        validate_measurement(observation["baseline"], f"{label}.baseline", True)
        validate_measurement(observation["resume"], f"{label}.resume", False)
        require(
            observation["baseline"]["seeded_issues"] == observation["resume"]["seeded_issues"],
            f"{label} arms have different seeded_issues",
        )

        adjudication = observation["false_carry_adjudication"]
        require_object(
            adjudication,
            {
                "unit",
                "carried_units",
                "adjudicated_units",
                "confirmed_false_carries",
                "adjudicator_count",
                "all_disagreements_resolved",
            },
            f"{label}.false_carry_adjudication",
        )
        require(adjudication["unit"] == "carried_file_change", f"{label} has an unsupported false-carry unit")
        for key in ("carried_units", "adjudicated_units", "confirmed_false_carries"):
            require_integer(adjudication[key], 0, 100000, f"{label}.false_carry_adjudication.{key}")
        require_integer(adjudication["adjudicator_count"], 2, 20, f"{label}.false_carry_adjudication.adjudicator_count")
        require_boolean(
            adjudication["all_disagreements_resolved"],
            f"{label}.false_carry_adjudication.all_disagreements_resolved",
        )
        require(adjudication["all_disagreements_resolved"], f"{label} has unresolved adjudication disagreements")
        require(
            adjudication["adjudicated_units"] == adjudication["carried_units"],
            f"{label} has incomplete false-carry adjudication",
        )
        require(
            adjudication["confirmed_false_carries"] <= adjudication["adjudicated_units"],
            f"{label} confirmed_false_carries exceeds adjudicated_units",
        )
    require(referenced_participants == participant_ids, "participants must each contribute at least one paired observation")
    if data["collection_status"] == "locked":
        validate_counterbalance(data, preregistration)


def rounded_divide(numerator, denominator):
    require(denominator > 0, "rounded division denominator must be positive")
    sign = -1 if numerator < 0 else 1
    quotient, remainder = divmod(abs(numerator), denominator)
    if remainder * 2 >= denominator:
        quotient += 1
    return sign * quotient


def ratio_basis_points(numerator, denominator):
    require(denominator > 0, "ratio denominator must be positive")
    return rounded_divide(numerator * 10000, denominator)


def median_integer(values):
    require(values, "median requires at least one value")
    ordered = sorted(values)
    middle = len(ordered) // 2
    if len(ordered) % 2 == 1:
        return ordered[middle]
    return rounded_divide(ordered[middle - 1] + ordered[middle], 2)


def paired_reduction_basis_points(observation, field):
    baseline = observation["baseline"][field]
    resume = observation["resume"][field]
    return ratio_basis_points(baseline - resume, baseline)


def require_ready_for_aggregation(data, preregistration, allow_synthetic):
    require(data["collection_status"] == "locked", "study collection must be locked before aggregation")
    require(not data["synthetic"] or allow_synthetic, "synthetic input requires --allow-synthetic")
    observations = data["paired_observations"]
    participants = data["participants"]
    minimums = preregistration["minimums"]
    require(len(observations) >= minimums["eligible_pairs"], "paired observation threshold not reached")
    require(len(participants) >= minimums["unique_participants"], "unique participant threshold not reached")
    adjudicated_units = sum(
        observation["false_carry_adjudication"]["adjudicated_units"] for observation in observations
    )
    require(
        adjudicated_units >= minimums["adjudicated_carry_units"],
        "adjudicated carry-unit threshold not reached",
    )
    validate_counterbalance(data, preregistration)


def aggregate_dataset(source_bytes, data, preregistration_bytes, preregistration, allow_synthetic):
    validate_dataset(data, preregistration_bytes, preregistration)
    require_ready_for_aggregation(data, preregistration, allow_synthetic)
    observations = data["paired_observations"]
    participants = data["participants"]
    completion_reductions = [
        paired_reduction_basis_points(observation, "completion_seconds") for observation in observations
    ]
    file_reductions = [
        paired_reduction_basis_points(observation, "reopened_files") for observation in observations
    ]
    line_reductions = [
        paired_reduction_basis_points(observation, "reopened_lines") for observation in observations
    ]

    baseline_issues_found = sum(observation["baseline"]["issues_found"] for observation in observations)
    baseline_seeded_issues = sum(observation["baseline"]["seeded_issues"] for observation in observations)
    resume_issues_found = sum(observation["resume"]["issues_found"] for observation in observations)
    resume_seeded_issues = sum(observation["resume"]["seeded_issues"] for observation in observations)
    carried_units = sum(observation["false_carry_adjudication"]["carried_units"] for observation in observations)
    adjudicated_units = sum(
        observation["false_carry_adjudication"]["adjudicated_units"] for observation in observations
    )
    confirmed_false_carries = sum(
        observation["false_carry_adjudication"]["confirmed_false_carries"] for observation in observations
    )
    repeat_eligible = sum(participant["repeat_use"]["invited_again"] for participant in participants)
    repeat_users = sum(participant["repeat_use"]["used_within_28_days"] for participant in participants)

    completion_median = median_integer(completion_reductions)
    files_median = median_integer(file_reductions)
    lines_median = median_integer(line_reductions)
    baseline_recall = ratio_basis_points(baseline_issues_found, baseline_seeded_issues)
    resume_recall = ratio_basis_points(resume_issues_found, resume_seeded_issues)
    repeat_rate = ratio_basis_points(repeat_users, repeat_eligible)
    participant_groups = observations_by(observations, "participant_id")
    task_family_groups = observations_by(observations, "task_family_id")
    gates = preregistration["gates"]
    recall_noninferior = (
        resume_issues_found * baseline_seeded_issues * 10000
        + gates["issue_recall_noninferiority_margin_basis_points"]
        * resume_seeded_issues
        * baseline_seeded_issues
        >= baseline_issues_found * resume_seeded_issues * 10000
    )
    criteria = {
        "completion_time": completion_median >= gates["median_completion_reduction_basis_points"],
        "reopened_files": files_median >= gates["median_reopened_files_reduction_basis_points"],
        "issue_recall_noninferior": recall_noninferior,
        "false_carry": confirmed_false_carries <= gates["maximum_confirmed_false_carries"],
        "repeat_use": repeat_users * 10000 >= gates["repeat_use_rate_basis_points"] * repeat_eligible,
    }
    criteria_met = all(criteria.values())
    return {
        "schema": AGGREGATE_SCHEMA,
        "protocol_version": preregistration["protocol_version"],
        "study_id": data["study_id"],
        "source": {
            "sha256": sha256(source_bytes),
            "preregistration_sha256": sha256(preregistration_bytes),
        },
        "synthetic": data["synthetic"],
        "claim_boundary": {
            "contains_individual_records": False,
            "publication_eligible": not data["synthetic"],
            "interpretation": (
                "synthetic toolchain exercise; not an observed human-study result"
                if data["synthetic"]
                else "prospective paired human-study aggregate"
            ),
        },
        "readiness": {
            "paired_observations": len(observations),
            "unique_participants": len(participants),
            "adjudicated_carry_units": adjudicated_units,
            "minimum_paired_observations": preregistration["minimums"]["eligible_pairs"],
            "minimum_unique_participants": preregistration["minimums"]["unique_participants"],
            "minimum_adjudicated_carry_units": preregistration["minimums"]["adjudicated_carry_units"],
            "counterbalance": {
                "cell_definition": preregistration["design"]["counterbalance"]["cell_definition"],
                "global_cell_counts": assignment_cell_counts(observations),
                "global_cell_count_difference": cell_count_difference(observations),
                "participant_max_cell_count_difference": max(
                    cell_count_difference(group) for group in participant_groups.values()
                ),
                "task_families": len(task_family_groups),
                "minimum_observations_in_task_family": min(len(group) for group in task_family_groups.values()),
                "task_family_max_cell_count_difference": max(
                    cell_count_difference(group) for group in task_family_groups.values()
                ),
            },
        },
        "metrics": {
            "completion_seconds": {
                "baseline_total": sum(observation["baseline"]["completion_seconds"] for observation in observations),
                "resume_total": sum(observation["resume"]["completion_seconds"] for observation in observations),
                "median_paired_reduction_basis_points": completion_median,
            },
            "issue_recall": {
                "baseline_issues_found": baseline_issues_found,
                "baseline_seeded_issues": baseline_seeded_issues,
                "baseline_basis_points": baseline_recall,
                "resume_issues_found": resume_issues_found,
                "resume_seeded_issues": resume_seeded_issues,
                "resume_basis_points": resume_recall,
                "resume_minus_baseline_basis_points": resume_recall - baseline_recall,
            },
            "reopened_files": {
                "baseline_total": sum(observation["baseline"]["reopened_files"] for observation in observations),
                "resume_total": sum(observation["resume"]["reopened_files"] for observation in observations),
                "median_paired_reduction_basis_points": files_median,
            },
            "reopened_lines": {
                "baseline_total": sum(observation["baseline"]["reopened_lines"] for observation in observations),
                "resume_total": sum(observation["resume"]["reopened_lines"] for observation in observations),
                "median_paired_reduction_basis_points": lines_median,
                "role": "exploratory",
            },
            "false_carry_adjudication": {
                "unit": "carried_file_change",
                "carried_units": carried_units,
                "adjudicated_units": adjudicated_units,
                "confirmed_false_carries": confirmed_false_carries,
                "confirmed_rate_basis_points": ratio_basis_points(confirmed_false_carries, adjudicated_units),
            },
            "repeat_use": {
                "window_days": preregistration["design"]["follow_up_window_days"],
                "eligible_participants": repeat_eligible,
                "repeat_users": repeat_users,
                "rate_basis_points": repeat_rate,
            },
        },
        "go_no": {
            "criteria": criteria,
            "criteria_met": criteria_met,
            "advance": criteria_met and not data["synthetic"],
        },
    }


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")


def set_assignment_cell(observation, cell):
    require(cell in ASSIGNMENT_CELLS, f"unsupported self-test assignment cell: {cell}")
    assignment_order, baseline_variant = cell.split(":")
    observation["assignment_order"] = assignment_order
    observation["baseline_variant"] = baseline_variant
    observation["resume_variant"] = "b" if baseline_variant == "a" else "a"


def self_test_observation(data, participant_index, task_family_index):
    participant_id = f"p_{participant_index + 1:012x}"
    task_family_id = f"task_{task_family_index + 1:012x}"
    matches = [
        observation
        for observation in data["paired_observations"]
        if observation["participant_id"] == participant_id
        and observation["task_family_id"] == task_family_id
    ]
    require(len(matches) == 1, "self-test observation lookup must be unique")
    return matches[0]


def build_self_test_dataset(preregistration_bytes, preregistration):
    minimums = preregistration["minimums"]
    require(minimums["eligible_pairs"] == 100, "self-test fixture expects 100 eligible pairs")
    require(minimums["unique_participants"] == 20, "self-test fixture expects 20 participants")
    participant_count = minimums["unique_participants"]
    task_family_count = minimums["eligible_pairs"] // participant_count
    require(
        participant_count * task_family_count == minimums["eligible_pairs"],
        "self-test pair minimum must divide evenly across participants",
    )
    participants = []
    for participant_index in range(participant_count):
        participants.append(
            {
                "participant_id": f"p_{participant_index + 1:012x}",
                "repeat_use": {
                    "invited_again": True,
                    "follow_up_complete": True,
                    "used_within_28_days": participant_index < participant_count // 2,
                },
            }
        )
    observations = []
    for participant_index in range(participant_count):
        for task_family_index in range(task_family_count):
            pair_index = participant_index * task_family_count + task_family_index
            observation = {
                "pair_id": f"pair_{pair_index + 1:012x}",
                "participant_id": f"p_{participant_index + 1:012x}",
                "task_family_id": f"task_{task_family_index + 1:012x}",
                "assignment_order": "baseline_then_resume",
                "baseline_variant": "a",
                "resume_variant": "b",
                "baseline": {
                    "completion_seconds": 100,
                    "issues_found": 8,
                    "seeded_issues": 10,
                    "reopened_files": 100,
                    "reopened_lines": 1000,
                },
                "resume": {
                    "completion_seconds": 80,
                    "issues_found": 8,
                    "seeded_issues": 10,
                    "reopened_files": 60,
                    "reopened_lines": 500,
                },
                "false_carry_adjudication": {
                    "unit": "carried_file_change",
                    "carried_units": 1,
                    "adjudicated_units": 1,
                    "confirmed_false_carries": 0,
                    "adjudicator_count": 2,
                    "all_disagreements_resolved": True,
                },
            }
            set_assignment_cell(observation, ASSIGNMENT_CELLS[(participant_index + task_family_index) % 4])
            observations.append(observation)
    return {
        "schema": DATA_SCHEMA,
        "study_id": "study_synthetic_selftest",
        "protocol_version": preregistration["protocol_version"],
        "preregistration_sha256": sha256(preregistration_bytes),
        "synthetic": True,
        "collection_status": "locked",
        "participants": participants,
        "paired_observations": observations,
    }


def require_rejected(action, message_fragment):
    try:
        action()
    except ValueError as error:
        require(
            message_fragment in str(error),
            f"self-test rejection mismatch: expected {message_fragment!r}, observed {str(error)!r}",
        )
        return
    raise ValueError(f"self-test expected rejection containing: {message_fragment}")


def aggregate_self_test_dataset(data, preregistration_bytes, preregistration):
    source_bytes = encoded(data)
    return aggregate_dataset(source_bytes, data, preregistration_bytes, preregistration, True)


def run_self_test(preregistration_bytes, preregistration):
    validate_preregistration(preregistration)
    data = build_self_test_dataset(preregistration_bytes, preregistration)
    validate_dataset(data, preregistration_bytes, preregistration)
    require(
        len({observation["task_family_id"] for observation in data["paired_observations"]})
        < len(data["paired_observations"]),
        "self-test must prove task families can be reused across participants",
    )

    aggregate = aggregate_self_test_dataset(data, preregistration_bytes, preregistration)
    require(aggregate["go_no"]["criteria_met"], "exact gate boundaries must pass")
    require(not aggregate["go_no"]["advance"], "synthetic input must never advance")
    require(not aggregate["claim_boundary"]["publication_eligible"], "synthetic aggregate must not be publishable")
    require(
        aggregate["metrics"]["completion_seconds"]["median_paired_reduction_basis_points"] == 2000,
        "completion boundary calculation changed",
    )
    require(
        aggregate["metrics"]["reopened_files"]["median_paired_reduction_basis_points"] == 4000,
        "reopened-file boundary calculation changed",
    )
    require(aggregate["metrics"]["repeat_use"]["rate_basis_points"] == 5000, "repeat-use boundary changed")
    require(
        set(aggregate["readiness"]["counterbalance"]["global_cell_counts"].values()) == {25},
        "self-test assignment cells must be exactly balanced",
    )
    require_rejected(
        lambda: aggregate_dataset(encoded(data), data, preregistration_bytes, preregistration, False),
        "synthetic input requires --allow-synthetic",
    )

    below_completion = copy.deepcopy(data)
    for observation in below_completion["paired_observations"]:
        observation["resume"]["completion_seconds"] = 81
    require(
        not aggregate_self_test_dataset(below_completion, preregistration_bytes, preregistration)["go_no"][
            "criteria"
        ]["completion_time"],
        "completion result below the preregistered threshold must fail",
    )

    below_reopened_files = copy.deepcopy(data)
    for observation in below_reopened_files["paired_observations"]:
        observation["resume"]["reopened_files"] = 61
    require(
        not aggregate_self_test_dataset(below_reopened_files, preregistration_bytes, preregistration)["go_no"][
            "criteria"
        ]["reopened_files"],
        "reopened-file result below the preregistered threshold must fail",
    )

    below_repeat_use = copy.deepcopy(data)
    for participant_index, participant in enumerate(below_repeat_use["participants"]):
        participant["repeat_use"]["used_within_28_days"] = participant_index < 9
    require(
        not aggregate_self_test_dataset(below_repeat_use, preregistration_bytes, preregistration)["go_no"][
            "criteria"
        ]["repeat_use"],
        "repeat use below the preregistered threshold must fail",
    )

    inferior_recall = copy.deepcopy(data)
    for observation in inferior_recall["paired_observations"]:
        observation["resume"]["issues_found"] = 7
    require(
        not aggregate_self_test_dataset(inferior_recall, preregistration_bytes, preregistration)["go_no"][
            "criteria"
        ]["issue_recall_noninferior"],
        "inferior issue recall must fail",
    )

    false_carry = copy.deepcopy(data)
    false_carry["paired_observations"][0]["false_carry_adjudication"]["confirmed_false_carries"] = 1
    require(
        not aggregate_self_test_dataset(false_carry, preregistration_bytes, preregistration)["go_no"]["criteria"][
            "false_carry"
        ],
        "a confirmed false carry must fail",
    )

    unknown_field = copy.deepcopy(data)
    unknown_field["paired_observations"][0]["baseline"]["free_text"] = "prohibited"
    require_rejected(
        lambda: validate_dataset(unknown_field, preregistration_bytes, preregistration),
        "unknown fields",
    )

    repeated_task_family = copy.deepcopy(data)
    repeated_task_family["paired_observations"][1]["task_family_id"] = repeated_task_family[
        "paired_observations"
    ][0]["task_family_id"]
    require_rejected(
        lambda: validate_dataset(repeated_task_family, preregistration_bytes, preregistration),
        "repeats a task family for one participant",
    )

    underused_task_family = copy.deepcopy(data)
    underused_task_family["paired_observations"][0]["task_family_id"] = "task_ffffffffffff"
    require_rejected(
        lambda: validate_dataset(underused_task_family, preregistration_bytes, preregistration),
        "task family has too few observations for counterbalancing",
    )

    global_imbalance = copy.deepcopy(data)
    set_assignment_cell(global_imbalance["paired_observations"][0], ASSIGNMENT_CELLS[1])
    require_rejected(
        lambda: validate_dataset(global_imbalance, preregistration_bytes, preregistration),
        "global assignment cells are not counterbalanced",
    )
    global_imbalance["collection_status"] = "open"
    validate_dataset(global_imbalance, preregistration_bytes, preregistration)

    participant_imbalance = copy.deepcopy(data)
    set_assignment_cell(self_test_observation(participant_imbalance, 0, 1), ASSIGNMENT_CELLS[0])
    set_assignment_cell(self_test_observation(participant_imbalance, 1, 3), ASSIGNMENT_CELLS[1])
    require_rejected(
        lambda: validate_dataset(participant_imbalance, preregistration_bytes, preregistration),
        "participant assignment cells are not counterbalanced",
    )

    task_family_imbalance = copy.deepcopy(data)
    set_assignment_cell(self_test_observation(task_family_imbalance, 0, 0), ASSIGNMENT_CELLS[1])
    set_assignment_cell(self_test_observation(task_family_imbalance, 0, 1), ASSIGNMENT_CELLS[0])
    require_rejected(
        lambda: validate_dataset(task_family_imbalance, preregistration_bytes, preregistration),
        "task-family assignment cells are not counterbalanced",
    )

    print("reviewer-study v1 self-test passed; no human dataset is checked in")


def parse_arguments():
    parser = argparse.ArgumentParser(description="Validate and aggregate Reviewer Study v1 data")
    parser.add_argument("--preregistration", type=Path, default=DEFAULT_PREREGISTRATION)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("self-test")
    validate = subparsers.add_parser("validate")
    validate.add_argument("--input", type=Path, required=True)
    aggregate = subparsers.add_parser("aggregate")
    aggregate.add_argument("--input", type=Path, required=True)
    aggregate.add_argument("--output", type=Path, required=True)
    aggregate.add_argument("--allow-synthetic", action="store_true")
    verify = subparsers.add_parser("verify")
    verify.add_argument("--input", type=Path, required=True)
    verify.add_argument("--aggregate", type=Path, required=True)
    verify.add_argument("--allow-synthetic", action="store_true")
    return parser.parse_args()


def main():
    arguments = parse_arguments()
    preregistration_bytes, preregistration = read_json(arguments.preregistration)
    if arguments.command == "self-test":
        run_self_test(preregistration_bytes, preregistration)
        return
    source_bytes, data = read_json(arguments.input)
    validate_dataset(data, preregistration_bytes, preregistration)
    if arguments.command == "validate":
        label = "synthetic" if data["synthetic"] else "prospective"
        print(
            f"valid {label} reviewer-study dataset: "
            f"{len(data['paired_observations'])} paired observations, {len(data['participants'])} participants"
        )
        return
    aggregate = aggregate_dataset(
        source_bytes,
        data,
        preregistration_bytes,
        preregistration,
        arguments.allow_synthetic,
    )
    if arguments.command == "aggregate":
        arguments.output.write_bytes(encoded(aggregate))
        print(f"wrote reviewer-study aggregate to {arguments.output}")
        return
    aggregate_bytes, observed = read_json(arguments.aggregate)
    require(observed == aggregate, "reviewer-study aggregate differs from recomputation")
    require(aggregate_bytes == encoded(observed), "reviewer-study aggregate JSON is not canonical")
    print(
        f"verified reviewer-study aggregate: {aggregate['readiness']['paired_observations']} pairs, "
        f"synthetic={str(aggregate['synthetic']).lower()}"
    )


if __name__ == "__main__":
    main()
