#!/usr/bin/env python3

import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

import review_transition as selector


ROOT = Path(__file__).resolve().parents[2]
REFERENCE = ROOT / "tools" / "resumebench-github-live" / "resumebench_github_live.py"
SPEC = importlib.util.spec_from_file_location("stratadiff_independent_git", REFERENCE)
REFERENCE_MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE_MODULE)
MATERIALIZATION_SCHEMA = "stratadiff-review-transition-materialization-v1"
CASE_STATUSES = ("observed", "materialized", "failed")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def git(repository, arguments, *, token=None, lazy=False, timeout=120, check=True, input_bytes=None):
    return REFERENCE_MODULE.run_git(
        repository,
        arguments,
        token=token,
        allow_lazy_fetch=lazy,
        timeout=timeout,
        check=check,
        input_bytes=input_bytes,
    )


def repository_size(repository):
    return sum(path.stat().st_size for path in repository.rglob("*") if path.is_file())


def enforce_repository_size(repository, max_bytes):
    size = repository_size(repository)
    require(size <= max_bytes, f"repository byte limit exceeded: {size}")
    return size


def hydrate(repository, remote_url, object_ids, token, timeout, max_bytes):
    if not object_ids:
        return
    scratch = Path(tempfile.mkdtemp(prefix=".rt30-blobs-", dir=repository.parent))
    try:
        subprocess.run(
            ["git", "init", "--bare", "--quiet", scratch],
            env=REFERENCE_MODULE.isolated_environment(),
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        git(scratch, ["remote", "add", "origin", remote_url])
        ids = sorted(object_ids)
        for offset in range(0, len(ids), 128):
            batch = ids[offset : offset + 128]
            refs = [f"+{oid}:refs/rt30/blobs/{oid}" for oid in batch]
            git(
                scratch,
                ["fetch", "--quiet", "--no-tags", "origin", *refs],
                token=token,
                lazy=True,
                timeout=timeout,
            )
            enforce_repository_size(scratch, max_bytes)
        git(
            repository,
            ["fetch", "--quiet", "--no-tags", str(scratch), "+refs/rt30/blobs/*:refs/rt30/blobs/*"],
        )
        enforce_repository_size(repository, max_bytes)
        commands = "".join(f"delete refs/rt30/blobs/{oid}\n" for oid in ids).encode("ascii")
        git(repository, ["update-ref", "--stdin"], input_bytes=commands)
    finally:
        shutil.rmtree(scratch)


def object_type(repository, oid):
    result = git(repository, ["cat-file", "-t", oid], check=False)
    require(result.returncode == 0, f"required object unavailable: {oid}")
    return result.stdout.decode("ascii").strip()


def object_types(repository, object_ids):
    ids = list(object_ids)
    if not ids:
        return {}
    request = "".join(f"{oid}\n" for oid in ids).encode("ascii")
    result = git(
        repository,
        ["cat-file", "--batch-check=%(objectname) %(objecttype)"],
        input_bytes=request,
    )
    lines = result.stdout.decode("ascii").splitlines()
    require(len(lines) == len(ids), "batch object response count differs")
    types = {}
    for expected, line in zip(ids, lines):
        fields = line.split()
        require(len(fields) == 2 and fields[0] == expected, f"batch object response differs: {expected}")
        require(fields[1] != "missing", f"required object unavailable: {expected}")
        types[expected] = fields[1]
    return types


def require_case_id(case_id):
    require(isinstance(case_id, str) and case_id, "case ID is empty")
    require("/" not in case_id and "\\" not in case_id and case_id not in (".", ".."), "case ID is unsafe")


def case_relative_path(case_id):
    require_case_id(case_id)
    return f"repositories/{case_id}.git"


def require_offline_repository(repository):
    bare = REFERENCE_MODULE.git_stdout(
        repository, ["rev-parse", "--is-bare-repository"]
    ).decode("ascii").strip()
    require(bare == "true", "materialization repository is not bare")
    require(not REFERENCE_MODULE.git_stdout(repository, ["remote"]), "materialization retained a remote")
    shallow = REFERENCE_MODULE.git_stdout(
        repository, ["rev-parse", "--is-shallow-repository"]
    ).decode("ascii").strip()
    require(shallow == "false", "materialization is shallow")
    replacements = REFERENCE_MODULE.git_stdout(repository, ["for-each-ref", "--format=%(refname)", "refs/replace"])
    require(not replacements, "materialization retained replacement refs")
    require(not (repository / "objects" / "info" / "alternates").exists(), "materialization uses alternate objects")
    partial = git(
        repository,
        [
            "config",
            "--local",
            "--get-regexp",
            r"^(extensions\.partialclone|remote\..*\.(url|promisor|partialclonefilter))$",
        ],
        check=False,
    )
    require(partial.returncode == 1 and not partial.stdout and not partial.stderr, "materialization retained partial-clone configuration")


def materialize_case(row, destination, token, timeout, max_bytes, *, remote_url=None):
    repository = row["repository"]
    if remote_url is None:
        remote_url = f"https://github.com/{repository}.git"
    q = row["pull_request"]["base_oid"]
    b = row["checkpoint_oid"]
    d = row["final_head_oid"]
    subprocess.run(
        ["git", "init", "--bare", "--quiet", destination],
        env=REFERENCE_MODULE.isolated_environment(),
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    git(destination, ["remote", "add", "origin", remote_url])
    git(destination, ["config", "extensions.partialClone", "origin"])
    git(destination, ["config", "remote.origin.promisor", "true"])
    git(destination, ["config", "remote.origin.partialclonefilter", "blob:none"])
    refs = [f"+{oid}:refs/rt30/{label}" for label, oid in (("Q", q), ("B", b), ("D", d))]
    git(
        destination,
        ["fetch", "--quiet", "--no-tags", "--filter=blob:none", "origin", *refs],
        token=token,
        lazy=True,
        timeout=timeout,
    )
    enforce_repository_size(destination, max_bytes)
    shallow = REFERENCE_MODULE.git_stdout(
        destination, ["rev-parse", "--is-shallow-repository"]
    ).decode("ascii").strip()
    require(shallow == "false", f"{row['case_id']} materialization is shallow")
    for label, oid in (("Q", q), ("B", b), ("D", d)):
        require(
            REFERENCE_MODULE.resolve_commit(destination, f"refs/rt30/{label}") == oid,
            f"{row['case_id']} {label} differs",
        )
    a = REFERENCE_MODULE.unique_merge_base(destination, q, b)
    c = REFERENCE_MODULE.unique_merge_base(destination, q, d)
    snapshots = {"Q": q, "A": a, "B": b, "C": c, "D": d}
    for label, oid in (("A", a), ("C", c)):
        git(destination, ["update-ref", f"refs/rt30/{label}", oid])
    changes = []
    checkpoint = REFERENCE_MODULE.raw_diff(destination, a, b)[2]
    for left, right in ((a, b), (c, d), (a, c), (b, d)):
        changes.extend(REFERENCE_MODULE.raw_diff(destination, left, right)[2])
    blobs = REFERENCE_MODULE.change_blob_ids(changes)
    blobs.update(REFERENCE_MODULE.review_delta_snapshot_blob_ids(destination, snapshots, checkpoint))
    hydrate(destination, remote_url, blobs, token, timeout, max_bytes)
    git(destination, ["remote", "remove", "origin"])
    git(destination, ["config", "--unset-all", "extensions.partialClone"], check=False)
    require_offline_repository(destination)
    blob_types = object_types(destination, sorted(blobs))
    for oid in blobs:
        require(blob_types[oid] == "blob", f"required object is not a blob: {oid}")
    trees = {}
    for label, oid in snapshots.items():
        tree = REFERENCE_MODULE.git_stdout(
            destination, ["rev-parse", f"{oid}^{{tree}}"]
        ).decode("ascii").strip()
        require(object_type(destination, tree) == "tree", f"{label} tree unavailable")
        trees[label] = tree
    require(REFERENCE_MODULE.unique_merge_base(destination, q, b) == a, "offline A differs")
    require(REFERENCE_MODULE.unique_merge_base(destination, q, d) == c, "offline C differs")
    git(destination, ["rev-list", "--parents", q, b, d])
    size = enforce_repository_size(destination, max_bytes)
    return {
        "repository": repository,
        "path": destination.name,
        "snapshots": {
            label: {"commit_oid": snapshots[label], "tree_oid": trees[label]}
            for label in ("Q", "A", "B", "C", "D")
        },
        "required_blob_oids": sorted(blobs),
        "repository_bytes": size,
    }


def materialization_summary(cases):
    counts = {status: 0 for status in CASE_STATUSES}
    for case in cases:
        require(case["status"] in counts, f"unsupported materialization status: {case['status']}")
        counts[case["status"]] += 1
    return {
        "selected_cases": len(cases),
        "observed": counts["observed"],
        "materialized": counts["materialized"],
        "failed": counts["failed"],
    }


def refresh_manifest(manifest):
    manifest["summary"] = materialization_summary(manifest["cases"])
    manifest["complete"] = manifest["summary"]["materialized"] == manifest["summary"]["selected_cases"]


def new_manifest(observation):
    cases = []
    for row in observation["cases"]:
        require(row["observation_status"] == "observed", f"{row['case_id']} is not observed")
        require_case_id(row["case_id"])
        cases.append(
            {
                "case_id": row["case_id"],
                "repository": row["repository"],
                "status": "observed",
            }
        )
    manifest = {
        "schema": MATERIALIZATION_SCHEMA,
        "observation_sha256": selector.sha256_bytes(selector.canonical_json_bytes(observation)),
        "complete": False,
        "summary": {},
        "cases": cases,
    }
    refresh_manifest(manifest)
    return manifest


def write_manifest_atomic(root, manifest):
    path = root / "materialization.json"
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temporary.write_bytes(selector.canonical_json_bytes(manifest))
    temporary.replace(path)


def load_resumable_manifest(output, observation, legacy_attempt_timeout):
    path = output / "materialization.json"
    raw, manifest = selector.load_json(path)
    require(raw == selector.canonical_json_bytes(manifest), "materialization is not canonical JSON")
    verify(manifest, output, observation, require_complete=False)
    legacy_failures = [
        case
        for case in manifest["cases"]
        if case["status"] == "failed" and "attempts" not in case
    ]
    if legacy_failures:
        require(
            legacy_attempt_timeout is not None,
            "legacy failed cases require an explicit prior attempt timeout",
        )
        require(
            type(legacy_attempt_timeout) is int and legacy_attempt_timeout > 0,
            "legacy attempt timeout must be a positive integer",
        )
        for case in legacy_failures:
            case["attempts"] = [
                {
                    "attempt": 1,
                    "fetch_timeout_seconds": legacy_attempt_timeout,
                    "status": "failed",
                    "failure": case["failure"],
                }
            ]
        write_manifest_atomic(output, manifest)
        verify(manifest, output, observation, require_complete=False)
    return manifest


def select_attempt_indices(manifest, case_ids, max_cases):
    if case_ids:
        require(len(case_ids) == len(set(case_ids)), "duplicate --case-id")
        known = {case["case_id"] for case in manifest["cases"]}
        unknown = sorted(set(case_ids) - known)
        require(not unknown, f"unknown case IDs: {unknown}")
        requested = set(case_ids)
    else:
        requested = {case["case_id"] for case in manifest["cases"]}
    indices = [
        index
        for index, case in enumerate(manifest["cases"])
        if case["case_id"] in requested and case["status"] != "materialized"
    ]
    if max_cases is not None:
        require(type(max_cases) is int and max_cases > 0, "max cases must be a positive integer")
        indices = indices[:max_cases]
    return indices


def failure_value(error):
    return {
        "error_class": type(error).__name__,
        "message": str(error)[:1000],
        "phase": "materialization",
    }


def failure_row(row, failure, attempts):
    value = {
        "case_id": row["case_id"],
        "repository": row["repository"],
        "status": "failed",
        "failure": failure,
    }
    if attempts:
        value["attempts"] = attempts
    return value


def materialize(
    observation,
    output,
    token,
    timeout,
    max_bytes,
    *,
    case_ids=None,
    max_cases=None,
    legacy_attempt_timeout=None,
):
    output = Path(output).resolve()
    require(type(max_bytes) is int and max_bytes > 0, "repository byte limit must be positive")
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        require(output.is_dir(), f"materialization output is not a directory: {output}")
        manifest = load_resumable_manifest(output, observation, legacy_attempt_timeout)
    else:
        output.mkdir()
        (output / "repositories").mkdir()
        manifest = new_manifest(observation)
        write_manifest_atomic(output, manifest)
    repositories = output / "repositories"
    require(repositories.is_dir(), "materialization repositories directory is missing")
    indices = select_attempt_indices(manifest, case_ids, max_cases)
    rows = {row["case_id"]: row for row in observation["cases"]}
    for index in indices:
        current = manifest["cases"][index]
        row = rows[current["case_id"]]
        attempts = list(current["attempts"]) if "attempts" in current else []
        attempt_number = len(attempts) + 1
        relative = case_relative_path(row["case_id"])
        destination = output / relative
        require(not destination.exists(), f"untracked materialization repository exists: {destination}")
        staging = Path(tempfile.mkdtemp(prefix=f".{row['case_id']}.tmp-", dir=repositories))
        shutil.rmtree(staging)
        try:
            case = materialize_case(row, staging, token, timeout, max_bytes)
            case["path"] = relative
            case["case_id"] = row["case_id"]
            case["status"] = "materialized"
            attempts.append(
                {
                    "attempt": attempt_number,
                    "fetch_timeout_seconds": timeout,
                    "status": "materialized",
                    "repository_bytes": case["repository_bytes"],
                }
            )
            case["attempts"] = attempts
            staging.replace(destination)
            manifest["cases"][index] = case
        except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
            if staging.exists():
                shutil.rmtree(staging)
            failure = failure_value(error)
            attempts.append(
                {
                    "attempt": attempt_number,
                    "fetch_timeout_seconds": timeout,
                    "status": "failed",
                    "failure": failure,
                }
            )
            manifest["cases"][index] = failure_row(row, failure, attempts)
        except BaseException:
            if staging.exists():
                shutil.rmtree(staging)
            raise
        refresh_manifest(manifest)
        write_manifest_atomic(output, manifest)
    return manifest


def verify_materialized_case(case, row, root):
    fields = {
        "case_id",
        "repository",
        "status",
        "path",
        "snapshots",
        "required_blob_oids",
        "repository_bytes",
    }
    require(
        set(case) == fields or set(case) == fields | {"attempts"},
        "materialized case fields differ",
    )
    require(case["path"] == case_relative_path(row["case_id"]), "materialization repository path differs")
    repository = root / case["path"]
    require(repository.resolve().is_relative_to(root.resolve()), "repository path escapes materialization")
    require(repository.is_dir(), "materialization repository is missing")
    require_offline_repository(repository)
    snapshots = case["snapshots"]
    require(set(snapshots) == {"Q", "A", "B", "C", "D"}, "snapshot roles differ")
    require(snapshots["Q"]["commit_oid"] == row["pull_request"]["base_oid"], "Q commit differs from observation")
    require(snapshots["B"]["commit_oid"] == row["checkpoint_oid"], "B commit differs from observation")
    require(snapshots["D"]["commit_oid"] == row["final_head_oid"], "D commit differs from observation")
    for label in ("Q", "A", "B", "C", "D"):
        require(set(snapshots[label]) == {"commit_oid", "tree_oid"}, f"{label} snapshot fields differ")
        commit = snapshots[label]["commit_oid"]
        require(REFERENCE_MODULE.resolve_commit(repository, commit) == commit, f"{label} commit unavailable")
        tree = REFERENCE_MODULE.git_stdout(
            repository, ["rev-parse", f"{commit}^{{tree}}"]
        ).decode("ascii").strip()
        require(
            tree == snapshots[label]["tree_oid"] and object_type(repository, tree) == "tree",
            f"{label} tree differs",
        )
    require(
        REFERENCE_MODULE.unique_merge_base(
            repository, snapshots["Q"]["commit_oid"], snapshots["B"]["commit_oid"]
        )
        == snapshots["A"]["commit_oid"],
        "A merge base differs",
    )
    require(
        REFERENCE_MODULE.unique_merge_base(
            repository, snapshots["Q"]["commit_oid"], snapshots["D"]["commit_oid"]
        )
        == snapshots["C"]["commit_oid"],
        "C merge base differs",
    )
    require(case["required_blob_oids"] == sorted(set(case["required_blob_oids"])), "required blob OIDs are not canonical")
    for oid in case["required_blob_oids"]:
        require(selector.is_oid(oid), f"required blob OID is invalid: {oid}")
    blob_types = object_types(repository, case["required_blob_oids"])
    for oid in case["required_blob_oids"]:
        require(blob_types[oid] == "blob", f"required blob unavailable: {oid}")
    require(type(case["repository_bytes"]) is int and case["repository_bytes"] > 0, "repository byte count is invalid")
    require(repository_size(repository) == case["repository_bytes"], "repository byte count differs")
    if "attempts" in case:
        validate_attempts(case["attempts"], "materialized", case["repository_bytes"], None)
    git(
        repository,
        [
            "rev-list",
            "--parents",
            snapshots["Q"]["commit_oid"],
            snapshots["B"]["commit_oid"],
            snapshots["D"]["commit_oid"],
        ],
    )


def validate_attempts(attempts, final_status, repository_bytes, failure):
    require(isinstance(attempts, list) and attempts, "materialization attempts are empty")
    for index, attempt in enumerate(attempts, start=1):
        require(attempt["attempt"] == index, "materialization attempt order differs")
        require(
            type(attempt["fetch_timeout_seconds"]) is int and attempt["fetch_timeout_seconds"] > 0,
            "materialization attempt timeout is invalid",
        )
        require(attempt["status"] in ("materialized", "failed"), "materialization attempt status differs")
        if index < len(attempts):
            require(attempt["status"] == "failed", "only the final materialization attempt may succeed")
        if attempt["status"] == "materialized":
            require(
                set(attempt) == {"attempt", "fetch_timeout_seconds", "status", "repository_bytes"},
                "successful materialization attempt fields differ",
            )
            require(
                type(attempt["repository_bytes"]) is int and attempt["repository_bytes"] > 0,
                "successful materialization attempt byte count is invalid",
            )
        else:
            require(
                set(attempt) == {"attempt", "fetch_timeout_seconds", "status", "failure"},
                "failed materialization attempt fields differ",
            )
            validate_failure(attempt["failure"])
    last = attempts[-1]
    require(last["status"] == final_status, "final materialization attempt status differs")
    if final_status == "materialized":
        require(last["repository_bytes"] == repository_bytes, "final materialization attempt byte count differs")
    else:
        require(last["failure"] == failure, "final materialization failure differs")


def validate_failure(failure):
    require(set(failure) == {"error_class", "message", "phase"}, "materialization failure fields differ")
    for field in ("error_class", "message", "phase"):
        require(isinstance(failure[field], str) and failure[field], f"materialization failure {field} is empty")


def verify(manifest, root, observation, *, require_complete=True):
    require(
        set(manifest) == {"schema", "observation_sha256", "complete", "summary", "cases"},
        "materialization fields differ",
    )
    require(manifest["schema"] == MATERIALIZATION_SCHEMA, "materialization schema differs")
    require(
        manifest["observation_sha256"]
        == selector.sha256_bytes(selector.canonical_json_bytes(observation)),
        "observation digest differs",
    )
    require(len(manifest["cases"]) == len(observation["cases"]), "materialization case count differs")
    root = Path(root).resolve()
    for case, row in zip(manifest["cases"], observation["cases"]):
        require(row["observation_status"] == "observed", "materialization source row is not observed")
        require(case["case_id"] == row["case_id"], "materialization case order differs")
        require(case["repository"] == row["repository"], "materialization repository differs")
        require(case["status"] in CASE_STATUSES, "materialization case status differs")
        if case["status"] == "materialized":
            verify_materialized_case(case, row, root)
        elif case["status"] == "observed":
            require(set(case) == {"case_id", "repository", "status"}, "observed materialization fields differ")
        else:
            require(
                set(case) == {"case_id", "repository", "status", "failure"}
                or set(case) == {"case_id", "repository", "status", "failure", "attempts"},
                "failed materialization fields differ",
            )
            failure = case["failure"]
            validate_failure(failure)
            if "attempts" in case:
                validate_attempts(case["attempts"], "failed", None, failure)
    expected_summary = materialization_summary(manifest["cases"])
    require(manifest["summary"] == expected_summary, "materialization summary differs")
    expected_complete = expected_summary["materialized"] == expected_summary["selected_cases"]
    require(manifest["complete"] is expected_complete, "materialization complete flag differs")
    if require_complete:
        require(expected_complete, "materialization is incomplete")
    return manifest
