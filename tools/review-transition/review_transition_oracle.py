#!/usr/bin/env python3

import json
from pathlib import Path
import shutil
import subprocess
import tempfile

import review_transition as selector
import review_transition_materialize as materializer


REFERENCE = materializer.REFERENCE_MODULE
ORACLE_SCHEMA = "stratadiff-review-transition-oracle-v1"
REPLAY_SCHEMA = "stratadiff-review-transition-product-replay-v1"
BUILD_INFO_SCHEMA = "stratadiff-build-info-v1"
REPORT_MATCH_BASIS = "exact_git_change_identity_or_noninteracting_four_way_byte_replay"
LOCAL_COMMAND_TIMEOUT_SECONDS = 120
MAX_PRODUCT_JSON_BYTES = 64 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def canonical_digest(value):
    return selector.sha256_bytes(selector.canonical_json_bytes(value))


def snapshot_commits(case):
    return {
        label: case["snapshots"][label]["commit_oid"]
        for label in ("Q", "A", "B", "C", "D")
    }


def display_path(change):
    path = change["after_path"] if change["after_path"] is not None else change["before_path"]
    require(path is not None, "Git change has no path")
    return path


def path_fields(path):
    fields = {"path_base64": REFERENCE.path_base64(path)}
    try:
        fields["path_utf8"] = path.decode("utf-8")
    except UnicodeDecodeError:
        pass
    return fields


def sorted_identities(changes):
    values = [REFERENCE.identity(change) for change in changes]
    values.sort(key=REFERENCE.canonical_identity)
    return values


def generate_case_oracle(case, repository):
    repository = Path(repository).resolve()
    materializer.require_offline_repository(repository)
    commits = snapshot_commits(case)
    for label, commit in commits.items():
        require(REFERENCE.resolve_commit(repository, commit) == commit, f"{case['case_id']} {label} differs")
    require(
        REFERENCE.unique_merge_base(repository, commits["Q"], commits["B"]) == commits["A"],
        f"{case['case_id']} A merge base differs",
    )
    require(
        REFERENCE.unique_merge_base(repository, commits["Q"], commits["D"]) == commits["C"],
        f"{case['case_id']} C merge base differs",
    )
    checkpoint_raw, _, checkpoint_changes = REFERENCE.raw_diff(repository, commits["A"], commits["B"])
    current_raw, _, current_changes = REFERENCE.raw_diff(repository, commits["C"], commits["D"])
    base_raw, _, base_changes = REFERENCE.raw_diff(repository, commits["A"], commits["C"])
    naive_raw, _, _ = REFERENCE.raw_diff(repository, commits["B"], commits["D"])

    matched_checkpoint_indices = set()
    classifications = []
    witnesses = []
    exact_carries = 0
    replay_carries = 0
    for current in current_changes:
        exact_indices = [
            index
            for index, checkpoint in enumerate(checkpoint_changes)
            if REFERENCE.identity_key(checkpoint) == REFERENCE.identity_key(current)
        ]
        current_identity = REFERENCE.identity(current)
        classification = {
            **path_fields(display_path(current)),
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
                if REFERENCE.replay_candidate_metadata_matches(checkpoint, current)
            ]
            witness = None
            candidate_index = None
            if len(candidate_indices) == 1 and candidate_indices[0] not in matched_checkpoint_indices:
                candidate_index = candidate_indices[0]
                try:
                    display_path(current).decode("utf-8")
                except UnicodeDecodeError:
                    pass
                else:
                    witness = REFERENCE.four_way_replay_witness(
                        repository,
                        checkpoint_changes[candidate_index],
                        current,
                        commits,
                    )
            if witness is None:
                classification["checkpoint_state"] = "needs_review_now"
            else:
                matched_checkpoint_indices.add(candidate_index)
                checkpoint_identity = REFERENCE.identity(checkpoint_changes[candidate_index])
                classification["checkpoint_state"] = "unchanged_since_checkpoint"
                classification["checkpoint_match_basis"] = "exact_noninteracting_four_way_byte_replay"
                classification["checkpoint_identity_sha256"] = checkpoint_identity["identity_sha256"]
                witnesses.append(witness)
                replay_carries += 1
        classifications.append(classification)

    classifications.sort(key=lambda item: (item["path_base64"], item["current_identity_sha256"]))
    witnesses.sort(key=lambda item: item["path_base64"])
    retired = [
        REFERENCE.identity(change)
        for index, change in enumerate(checkpoint_changes)
        if index not in matched_checkpoint_indices
    ]
    retired.sort(key=REFERENCE.canonical_identity)
    unresolved_retired = []
    for index, change in enumerate(checkpoint_changes):
        if index in matched_checkpoint_indices:
            continue
        path = display_path(change)
        try:
            path.decode("utf-8")
        except UnicodeDecodeError:
            unresolved_retired.append(
                {
                    **path_fields(path),
                    "checkpoint_identity_sha256": REFERENCE.identity(change)["identity_sha256"],
                    "reason": "non_utf8_git_path",
                }
            )
    unresolved_retired.sort(key=lambda item: item["path_base64"])
    base_drift = sorted_identities(base_changes)
    summary = {
        "current_change_identities": len(current_changes),
        "exact_identity_carries": exact_carries,
        "four_way_replay_carries": replay_carries,
        "needs_review_identities": len(current_changes) - exact_carries - replay_carries,
        "retired_checkpoint_changes": len(retired),
        "unresolved_retired_changes": len(unresolved_retired),
        "base_drift_obligations": len(base_drift),
    }
    return {
        "oracle_kind": "independent_exact_policy_conformance",
        "human_priority_ground_truth": "absent",
        "snapshots": commits,
        "snapshot_trees": {
            label: case["snapshots"][label]["tree_oid"]
            for label in ("Q", "A", "B", "C", "D")
        },
        "raw_diff_sha256": {
            "checkpoint_A_to_B": selector.sha256_bytes(checkpoint_raw),
            "current_C_to_D": selector.sha256_bytes(current_raw),
            "base_drift_A_to_C": selector.sha256_bytes(base_raw),
            "naive_B_to_D": selector.sha256_bytes(naive_raw),
        },
        "checkpoint_identities": sorted_identities(checkpoint_changes),
        "current_identities": sorted_identities(current_changes),
        "classification": classifications,
        "replay_witnesses": witnesses,
        "retired_checkpoint_identities": retired,
        "unresolved_retired_changes": unresolved_retired,
        "base_drift_obligations": base_drift,
        "summary": summary,
    }


def select_case_ids(cases, case_ids, max_cases):
    if case_ids:
        require(len(case_ids) == len(set(case_ids)), "duplicate --case-id")
        known = {case["case_id"] for case in cases}
        unknown = sorted(set(case_ids) - known)
        require(not unknown, f"unknown case IDs: {unknown}")
        selected = [case_id for case_id in case_ids]
    else:
        selected = [case["case_id"] for case in cases if case["status"] == "materialized"]
    if max_cases is not None:
        require(type(max_cases) is int and max_cases > 0, "max cases must be a positive integer")
        selected = selected[:max_cases]
    return set(selected)


def not_evaluated_reason(case, selected):
    if case["status"] == "observed":
        return "materialization_observed"
    if case["status"] == "failed":
        return "materialization_failed"
    require(case["status"] == "materialized", "unsupported materialization status")
    require(case["case_id"] not in selected, "selected materialized case is not evaluated")
    return "not_selected"


def oracle_summary(cases):
    evaluated = sum(case["status"] == "evaluated" for case in cases)
    return {
        "selected_cases": len(cases),
        "evaluated": evaluated,
        "not_evaluated": len(cases) - evaluated,
    }


def generate_oracle_bundle(manifest, materialization_root, materialization_raw, *, case_ids=None, max_cases=None):
    materialization_root = Path(materialization_root).resolve()
    require(
        materialization_raw == selector.canonical_json_bytes(manifest),
        "materialization bytes are not canonical or do not match the manifest",
    )
    selected = select_case_ids(manifest["cases"], case_ids, max_cases)
    cases = []
    for case in manifest["cases"]:
        if case["case_id"] in selected:
            require(case["status"] == "materialized", f"selected case is not materialized: {case['case_id']}")
            repository = materialization_root / case["path"]
            cases.append(
                {
                    "case_id": case["case_id"],
                    "status": "evaluated",
                    "oracle": generate_case_oracle(case, repository),
                }
            )
        else:
            cases.append(
                {
                    "case_id": case["case_id"],
                    "status": "not_evaluated",
                    "reason": not_evaluated_reason(case, selected),
                }
            )
    summary = oracle_summary(cases)
    return {
        "schema": ORACLE_SCHEMA,
        "materialization_sha256": selector.sha256_bytes(materialization_raw),
        "complete": summary["evaluated"] == summary["selected_cases"],
        "summary": summary,
        "cases": cases,
    }


def verify_oracle_bundle(bundle, manifest, materialization_root, materialization_raw):
    require(
        set(bundle) == {"schema", "materialization_sha256", "complete", "summary", "cases"},
        "oracle bundle fields differ",
    )
    require(bundle["schema"] == ORACLE_SCHEMA, "oracle bundle schema differs")
    require(
        materialization_raw == selector.canonical_json_bytes(manifest),
        "materialization bytes are not canonical or do not match the manifest",
    )
    require(
        bundle["materialization_sha256"] == selector.sha256_bytes(materialization_raw),
        "oracle materialization digest differs",
    )
    require(len(bundle["cases"]) == len(manifest["cases"]), "oracle case count differs")
    selected = []
    for row, case in zip(bundle["cases"], manifest["cases"]):
        require(row["case_id"] == case["case_id"], "oracle case order differs")
        require(row["status"] in ("evaluated", "not_evaluated"), "oracle case status differs")
        if row["status"] == "evaluated":
            require(set(row) == {"case_id", "status", "oracle"}, "evaluated oracle row fields differ")
            selected.append(row["case_id"])
        else:
            require(set(row) == {"case_id", "status", "reason"}, "not-evaluated oracle row fields differ")
    expected = generate_oracle_bundle(
        manifest,
        materialization_root,
        materialization_raw,
        case_ids=selected,
    )
    require(bundle == expected, "oracle bundle differs from independent Git recomputation")
    return bundle


def parse_json_bytes(raw, label):
    value = json.loads(
        raw.decode("utf-8"),
        object_pairs_hook=selector.unique_json_object,
        parse_constant=selector.reject_json_constant,
    )
    require(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def run_binary(binary, arguments, *, timeout=LOCAL_COMMAND_TIMEOUT_SECONDS):
    result = subprocess.run(
        [str(binary), *arguments],
        env=REFERENCE.isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )
    if result.returncode != 0:
        diagnostic = result.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"stratadiff {' '.join(arguments)} failed: {diagnostic}")
    return result


def binary_build_info(binary):
    result = run_binary(binary, ["build-info"])
    require(not result.stderr, "stratadiff build-info produced diagnostics")
    value = parse_json_bytes(result.stdout, "build info")
    require(value["schema"] == BUILD_INFO_SCHEMA, "unsupported StrataDiff build-info schema")
    return value


def verify_repository_copy(case, repository):
    materializer.require_offline_repository(repository)
    require(not any(path.is_symlink() for path in repository.rglob("*")), "replay repository contains a symlink")
    require(
        materializer.repository_size(repository) == case["repository_bytes"],
        "replay repository byte count differs",
    )
    for label, snapshot in case["snapshots"].items():
        commit = snapshot["commit_oid"]
        require(REFERENCE.resolve_commit(repository, commit) == commit, f"replay copy {label} differs")
    for oid in case["required_blob_oids"]:
        require(materializer.object_type(repository, oid) == "blob", f"replay copy blob differs: {oid}")


def run_product_once(binary, case, source_repository, run_root, timeout):
    repository = run_root / "repository.git"
    require(
        not any(path.is_symlink() for path in source_repository.rglob("*")),
        "materialized repository contains a symlink",
    )
    shutil.copytree(source_repository, repository, symlinks=True, copy_function=shutil.copy2)
    require(repository.resolve() != source_repository.resolve(), "replay did not create an independent repository")
    verify_repository_copy(case, repository)
    report_path = run_root / "report.raw.json"
    delta_path = run_root / "delta.raw.json"
    commits = snapshot_commits(case)
    result = run_binary(
        binary,
        [
            "review",
            commits["Q"],
            commits["D"],
            "--checkpoint",
            commits["B"],
            "--repo",
            str(repository),
            "--format",
            "json",
            "--output",
            str(report_path),
            "--review-delta-output",
            str(delta_path),
        ],
        timeout=timeout,
    )
    require(not result.stdout, "stratadiff review produced unexpected stdout")
    expected_diagnostics = {
        (
            f"wrote repository review to {report_path}\n"
            f"wrote review delta to {delta_path}\n"
        ).encode("utf-8"),
        (
            f'wrote repository review to "{report_path}"\n'
            f'wrote review delta to "{delta_path}"\n'
        ).encode("utf-8"),
    }
    require(result.stderr in expected_diagnostics, "stratadiff review produced unexpected diagnostics")
    require(report_path.stat().st_size <= MAX_PRODUCT_JSON_BYTES, "product report exceeds byte limit")
    require(delta_path.stat().st_size <= MAX_PRODUCT_JSON_BYTES, "product review delta exceeds byte limit")
    report = parse_json_bytes(report_path.read_bytes(), "product report")
    delta = parse_json_bytes(delta_path.read_bytes(), "product review delta")
    report_bytes = selector.canonical_json_bytes(report)
    delta_bytes = selector.canonical_json_bytes(delta)
    return report, delta, report_bytes, delta_bytes


def expected_classifications(oracle):
    values = {}
    for item in oracle["classification"]:
        identity_sha256 = item["current_identity_sha256"]
        require(identity_sha256 not in values, f"duplicate oracle classification: {identity_sha256}")
        values[identity_sha256] = item
    return values


def product_conformance(case_id, oracle, report, delta):
    snapshots = oracle["snapshots"]
    require(report["requested_base"] == snapshots["Q"], f"{case_id} product Q differs")
    require(report["base_commit"] == snapshots["C"], f"{case_id} product C differs")
    require(report["requested_head"] == snapshots["D"], f"{case_id} product requested D differs")
    require(report["head_commit"] == snapshots["D"], f"{case_id} product D differs")
    require(report["checkpoint"]["commit"] == snapshots["B"], f"{case_id} product B differs")
    require(report["checkpoint"]["base_commit"] == snapshots["A"], f"{case_id} product A differs")
    expected_policy = "exact_git_change_identity" if snapshots["A"] == snapshots["C"] else REPORT_MATCH_BASIS
    require(report["checkpoint"]["match_basis"] == expected_policy, f"{case_id} product policy differs")
    require(delta["old_base_commit"] == snapshots["A"], f"{case_id} delta A differs")
    require(delta["checkpoint_commit"] == snapshots["B"], f"{case_id} delta B differs")
    require(delta["current_base_commit"] == snapshots["C"], f"{case_id} delta C differs")
    require(delta["head_commit"] == snapshots["D"], f"{case_id} delta D differs")

    expected = expected_classifications(oracle)
    observed = {}
    duplicates = []
    for file in report["files"]:
        value = REFERENCE.product_identity(file)
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
        if expected_state == "needs_review_now" and observed_state == "unchanged_since_checkpoint":
            false_carry.append(identity_sha256)
        elif expected_state == "unchanged_since_checkpoint" and observed_state == "needs_review_now":
            false_invalidation.append(identity_sha256)
        if expected_state == "unchanged_since_checkpoint" and observed_state == "unchanged_since_checkpoint":
            if observed_file["checkpoint_match_basis"] != expected_file["checkpoint_match_basis"]:
                basis_mismatches.append(identity_sha256)
        if expected_state == "needs_review_now" and "checkpoint_match_basis" in observed_file:
            basis_mismatches.append(identity_sha256)
    omissions = sorted(expected_ids - observed_ids)
    extras = sorted(observed_ids - expected_ids)
    checkpoint_summary = report["summary"]["checkpoint"]
    oracle_summary_value = oracle["summary"]
    summary_mismatch = (
        report["summary"]["changed_files"] != oracle_summary_value["current_change_identities"]
        or checkpoint_summary["unchanged_since_checkpoint_files"]
        != oracle_summary_value["exact_identity_carries"] + oracle_summary_value["four_way_replay_carries"]
        or checkpoint_summary["needs_review_now_files"] != oracle_summary_value["needs_review_identities"]
        or checkpoint_summary["retired_change_count"] != oracle_summary_value["retired_checkpoint_changes"]
    )
    passed = not (
        false_carry
        or false_invalidation
        or basis_mismatches
        or omissions
        or extras
        or duplicates
        or summary_mismatch
    )
    return {
        "passed": passed,
        "false_carry": false_carry,
        "false_invalidation": false_invalidation,
        "basis_mismatches": sorted(set(basis_mismatches)),
        "identity_omissions": omissions,
        "identity_extras": extras,
        "duplicate_product_identities": sorted(duplicates),
        "summary_mismatch": summary_mismatch,
    }


def artifact_record(relative_path, raw):
    return {"path": relative_path.as_posix(), "sha256": selector.sha256_bytes(raw), "bytes": len(raw)}


def replay_summary(cases):
    return {
        "selected_cases": len(cases),
        "evaluated": sum(case["status"] == "evaluated" for case in cases),
        "not_evaluated": sum(case["status"] == "not_evaluated" for case in cases),
        "failed": sum(case["status"] == "failed" for case in cases),
        "deterministic": sum(case["status"] == "evaluated" and case["deterministic"] for case in cases),
        "oracle_conformant": sum(
            case["status"] == "evaluated" and case["conformance"]["passed"] for case in cases
        ),
    }


def replay_product(bundle, manifest, materialization_root, materialization_raw, oracle_raw, binary, output, *, case_ids=None, max_cases=None, timeout=LOCAL_COMMAND_TIMEOUT_SECONDS):
    materialization_root = Path(materialization_root).resolve()
    verify_oracle_bundle(bundle, manifest, materialization_root, materialization_raw)
    require(oracle_raw == selector.canonical_json_bytes(bundle), "oracle bytes are not canonical or do not match the bundle")
    binary = Path(binary).resolve()
    require(binary.is_file(), f"StrataDiff binary does not exist: {binary}")
    binary_sha256 = selector.sha256_bytes(binary.read_bytes())
    build_info = binary_build_info(binary)
    output = Path(output).resolve()
    require(not output.exists(), f"replay output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    selected = select_case_ids(manifest["cases"], case_ids, max_cases)
    oracle_by_id = {row["case_id"]: row for row in bundle["cases"]}
    stage = Path(tempfile.mkdtemp(prefix=f".{output.name}.tmp-", dir=output.parent))
    try:
        cases = []
        for case in manifest["cases"]:
            oracle_row = oracle_by_id[case["case_id"]]
            if case["case_id"] not in selected or oracle_row["status"] != "evaluated":
                reason = "not_selected" if case["case_id"] not in selected else "oracle_not_evaluated"
                cases.append({"case_id": case["case_id"], "status": "not_evaluated", "reason": reason})
                continue
            require(case["status"] == "materialized", f"replay case is not materialized: {case['case_id']}")
            source_repository = materialization_root / case["path"]
            case_root = stage / "cases" / case["case_id"]
            case_root.mkdir(parents=True)
            run_values = []
            try:
                for run_number in (1, 2):
                    run_root = Path(tempfile.mkdtemp(prefix=f"run-{run_number}-", dir=case_root))
                    try:
                        report, delta, report_bytes, delta_bytes = run_product_once(
                            binary,
                            case,
                            source_repository,
                            run_root,
                            timeout,
                        )
                    finally:
                        if run_root.exists():
                            shutil.rmtree(run_root)
                    artifact_root = case_root / f"run-{run_number}"
                    artifact_root.mkdir()
                    report_path = artifact_root / "report.json"
                    delta_path = artifact_root / "review-delta.json"
                    report_path.write_bytes(report_bytes)
                    delta_path.write_bytes(delta_bytes)
                    run_values.append(
                        {
                            "report": report,
                            "delta": delta,
                            "report_bytes": report_bytes,
                            "delta_bytes": delta_bytes,
                            "artifacts": {
                                "report": artifact_record(report_path.relative_to(stage), report_bytes),
                                "review_delta": artifact_record(delta_path.relative_to(stage), delta_bytes),
                            },
                        }
                    )
            except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
                if case_root.exists():
                    shutil.rmtree(case_root)
                cases.append(
                    {
                        "case_id": case["case_id"],
                        "status": "failed",
                        "failure": {
                            "error_class": type(error).__name__,
                            "message": str(error)[:1000],
                            "phase": "product_invocation",
                        },
                    }
                )
                continue
            try:
                first, second = run_values
                report_bytes_equal = first["report_bytes"] == second["report_bytes"]
                delta_bytes_equal = first["delta_bytes"] == second["delta_bytes"]
                report_summary_equal = first["report"]["summary"] == second["report"]["summary"]
                delta_summary_equal = first["delta"]["summary"] == second["delta"]["summary"]
                deterministic = report_bytes_equal and delta_bytes_equal and report_summary_equal and delta_summary_equal
                conformance = product_conformance(
                    case["case_id"], oracle_row["oracle"], first["report"], first["delta"]
                )
                result_digest = canonical_digest(
                    {
                        "report_sha256": first["artifacts"]["report"]["sha256"],
                        "review_delta_sha256": first["artifacts"]["review_delta"]["sha256"],
                    }
                )
            except (KeyError, TypeError, ValueError) as error:
                if case_root.exists():
                    shutil.rmtree(case_root)
                cases.append(
                    {
                        "case_id": case["case_id"],
                        "status": "failed",
                        "failure": {
                            "error_class": type(error).__name__,
                            "message": str(error)[:1000],
                            "phase": "output_validation",
                        },
                    }
                )
                continue
            cases.append(
                {
                    "case_id": case["case_id"],
                    "status": "evaluated",
                    "deterministic": deterministic,
                    "comparisons": {
                        "report_bytes_equal": report_bytes_equal,
                        "review_delta_bytes_equal": delta_bytes_equal,
                        "report_summary_equal": report_summary_equal,
                        "review_delta_summary_equal": delta_summary_equal,
                    },
                    "result_sha256": result_digest,
                    "report_summary": first["report"]["summary"],
                    "review_delta_summary": first["delta"]["summary"],
                    "conformance": conformance,
                    "runs": [first["artifacts"], second["artifacts"]],
                }
            )
        summary = replay_summary(cases)
        replay = {
            "schema": REPLAY_SCHEMA,
            "materialization_sha256": selector.sha256_bytes(materialization_raw),
            "oracle_sha256": selector.sha256_bytes(oracle_raw),
            "binary_sha256": binary_sha256,
            "build_info": build_info,
            "complete": summary["evaluated"] == summary["selected_cases"]
            and summary["deterministic"] == summary["selected_cases"],
            "summary": summary,
            "cases": cases,
        }
        require(selector.sha256_bytes(binary.read_bytes()) == binary_sha256, "StrataDiff binary changed during replay")
        (stage / "replay.json").write_bytes(selector.canonical_json_bytes(replay))
        stage.replace(output)
        return replay
    except BaseException:
        if stage.exists():
            shutil.rmtree(stage)
        raise


def safe_artifact(root, record):
    require(set(record) == {"path", "sha256", "bytes"}, "replay artifact fields differ")
    relative = Path(record["path"])
    require(not relative.is_absolute() and ".." not in relative.parts, "replay artifact path escapes output")
    path = root / relative
    require(path.resolve().is_relative_to(root.resolve()), "replay artifact path escapes output")
    raw, value = selector.load_json(path)
    require(raw == selector.canonical_json_bytes(value), "replay artifact is not canonical JSON")
    require(selector.sha256_bytes(raw) == record["sha256"], "replay artifact digest differs")
    require(len(raw) == record["bytes"], "replay artifact byte count differs")
    return raw, value


def verify_replay_bundle(replay, replay_root, bundle, manifest, materialization_raw, oracle_raw, binary):
    require(
        set(replay)
        == {
            "schema",
            "materialization_sha256",
            "oracle_sha256",
            "binary_sha256",
            "build_info",
            "complete",
            "summary",
            "cases",
        },
        "product replay fields differ",
    )
    require(replay["schema"] == REPLAY_SCHEMA, "product replay schema differs")
    require(oracle_raw == selector.canonical_json_bytes(bundle), "oracle bytes are not canonical or do not match the bundle")
    require(replay["materialization_sha256"] == selector.sha256_bytes(materialization_raw), "replay materialization digest differs")
    require(replay["oracle_sha256"] == selector.sha256_bytes(oracle_raw), "replay oracle digest differs")
    binary = Path(binary).resolve()
    require(replay["binary_sha256"] == selector.sha256_bytes(binary.read_bytes()), "replay binary digest differs")
    require(replay["build_info"] == binary_build_info(binary), "replay build info differs")
    require(len(replay["cases"]) == len(manifest["cases"]), "product replay case count differs")
    oracle_by_id = {row["case_id"]: row for row in bundle["cases"]}
    for row, case in zip(replay["cases"], manifest["cases"]):
        require(row["case_id"] == case["case_id"], "product replay case order differs")
        require(row["status"] in ("evaluated", "not_evaluated", "failed"), "product replay status differs")
        if row["status"] == "not_evaluated":
            require(set(row) == {"case_id", "status", "reason"}, "not-evaluated replay fields differ")
            require(row["reason"] in ("not_selected", "oracle_not_evaluated"), "not-evaluated replay reason differs")
            continue
        if row["status"] == "failed":
            require(set(row) == {"case_id", "status", "failure"}, "failed replay fields differ")
            require(set(row["failure"]) == {"error_class", "message", "phase"}, "replay failure fields differ")
            continue
        require(
            set(row)
            == {
                "case_id",
                "status",
                "deterministic",
                "comparisons",
                "result_sha256",
                "report_summary",
                "review_delta_summary",
                "conformance",
                "runs",
            },
            "evaluated replay fields differ",
        )
        require(case["status"] == "materialized", "evaluated replay case is not materialized")
        oracle_row = oracle_by_id[row["case_id"]]
        require(oracle_row["status"] == "evaluated", "evaluated replay has no frozen oracle")
        require(len(row["runs"]) == 2, "product replay must contain exactly two runs")
        values = []
        for run in row["runs"]:
            require(set(run) == {"report", "review_delta"}, "product replay run fields differ")
            report_raw, report = safe_artifact(Path(replay_root), run["report"])
            delta_raw, delta = safe_artifact(Path(replay_root), run["review_delta"])
            values.append((report_raw, report, delta_raw, delta))
        first, second = values
        comparisons = {
            "report_bytes_equal": first[0] == second[0],
            "review_delta_bytes_equal": first[2] == second[2],
            "report_summary_equal": first[1]["summary"] == second[1]["summary"],
            "review_delta_summary_equal": first[3]["summary"] == second[3]["summary"],
        }
        require(row["comparisons"] == comparisons, "product replay comparisons differ")
        require(row["deterministic"] is all(comparisons.values()), "product replay deterministic flag differs")
        require(row["report_summary"] == first[1]["summary"], "product replay report summary differs")
        require(row["review_delta_summary"] == first[3]["summary"], "product replay delta summary differs")
        expected_digest = canonical_digest(
            {
                "report_sha256": row["runs"][0]["report"]["sha256"],
                "review_delta_sha256": row["runs"][0]["review_delta"]["sha256"],
            }
        )
        require(row["result_sha256"] == expected_digest, "product replay result digest differs")
        conformance = product_conformance(row["case_id"], oracle_row["oracle"], first[1], first[3])
        require(row["conformance"] == conformance, "product replay conformance differs")
    expected_summary = replay_summary(replay["cases"])
    require(replay["summary"] == expected_summary, "product replay summary differs")
    expected_complete = expected_summary["evaluated"] == expected_summary["selected_cases"] and expected_summary["deterministic"] == expected_summary["selected_cases"]
    require(replay["complete"] is expected_complete, "product replay complete flag differs")
    return replay
