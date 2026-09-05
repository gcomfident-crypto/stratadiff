#!/usr/bin/env python3

import argparse
import base64
from datetime import datetime, timezone
import difflib
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import selectors
import shutil
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
MANIFEST_PATH = REPOSITORY_ROOT / "benchmarks" / "review-delta-v1" / "manifest.json"
MANIFEST_SCHEMA = "stratadiff-review-delta-benchmark-manifest-v1"
EVALUATION_SCHEMA = "stratadiff-review-delta-benchmark-evaluation-v1"
REVIEW_SCHEMA = (
    "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/"
    "schema/review-v1.schema.json"
)
REVIEW_DELTA_SCHEMA = (
    "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/"
    "schema/review-delta-v1.schema.json"
)
SNAPSHOT_LABELS = ("A", "B", "C", "D")
PARENT_LABELS = {"B": "A", "C": "A", "D": "C"}
ALLOWED_MODES = {"100644": 0o644, "100755": 0o755}
REQUIRED_COVERAGE = {
    "pure_rebase",
    "absorbed_retired_change",
    "noninteracting_author_followup",
    "new_current_change",
    "drop_or_revert",
    "overlap_fallback",
    "adjacent_fallback",
    "binary_fallback",
    "add_fallback",
    "delete_fallback",
    "rename_fallback",
    "dropped_rename",
    "mode_fallback",
    "full_scope_c_to_d",
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def git_blob_oid(value):
    header = f"blob {len(value)}\0".encode("ascii")
    return hashlib.sha1(header + value).hexdigest()


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def optional_field(mapping, key):
    return mapping[key] if key in mapping else None


def isolated_environment(extra=None):
    environment = {
        name: value
        for name, value in os.environ.items()
        if not name.upper().startswith("GIT_")
    }
    environment["LC_ALL"] = "C"
    environment["GIT_CONFIG_GLOBAL"] = os.devnull
    environment["GIT_CONFIG_NOSYSTEM"] = "1"
    environment["GIT_NO_REPLACE_OBJECTS"] = "1"
    environment["GIT_NO_LAZY_FETCH"] = "1"
    environment["GIT_TERMINAL_PROMPT"] = "0"
    if extra is not None:
        environment.update(extra)
    return environment


def run_git(repository, arguments, *, check=True, extra_environment=None):
    return subprocess.run(
        ["git", "--no-replace-objects", "-C", str(repository), *arguments],
        env=isolated_environment(extra_environment),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=check,
    )


def git_bytes(repository, arguments):
    return run_git(repository, arguments).stdout


def git_text(repository, arguments):
    return git_bytes(repository, arguments).decode("utf-8").strip()


def validate_oid(value, label):
    require(
        len(value) == 40 and all(character in "0123456789abcdef" for character in value),
        f"{label} is not a full lowercase SHA-1 object ID: {value}",
    )


def decode_content(specification):
    content = specification["content"]
    encoding = content["encoding"]
    value = content["value"]
    if encoding == "utf8":
        return value.encode("utf-8")
    if encoding == "base64":
        return base64.b64decode(value, validate=True)
    raise ValueError(f"unsupported content encoding: {encoding}")


def independent_line_counts(before, after):
    try:
        before_text = before.decode("utf-8")
        after_text = after.decode("utf-8")
    except UnicodeDecodeError:
        return (None, None)
    before_lines = before_text.splitlines(keepends=True)
    after_lines = after_text.splitlines(keepends=True)
    matcher = difflib.SequenceMatcher(
        None,
        before_lines,
        after_lines,
        autojunk=False,
    )
    additions = 0
    deletions = 0
    for tag, before_start, before_end, after_start, after_end in matcher.get_opcodes():
        if tag == "equal":
            continue
        deletions += before_end - before_start
        additions += after_end - after_start
    return (additions, deletions)


def independent_byte_edits(before, after):
    matcher = difflib.SequenceMatcher(None, before, after, autojunk=False)
    return [
        (before_start, before_end, after[after_start:after_end])
        for tag, before_start, before_end, after_start, after_end in matcher.get_opcodes()
        if tag != "equal"
    ]


def independent_edits_interact(left, right):
    return any(
        not (left_end < right_start or right_end < left_start)
        for left_start, left_end, _ in left
        for right_start, right_end, _ in right
    )


def independent_translate_edits(edits, preceding):
    translated = []
    for start, end, replacement in edits:
        offset = sum(
            len(preceding_replacement) - (preceding_end - preceding_start)
            for preceding_start, preceding_end, preceding_replacement in preceding
            if preceding_end < start
        )
        translated.append((start + offset, end + offset, replacement))
    return translated


def independent_apply_edits(source, edits):
    output = bytearray(source)
    for start, end, replacement in reversed(edits):
        require(0 <= start <= end <= len(output), "independent byte edit is out of range")
        output[start:end] = replacement
    return bytes(output)


def independent_bidirectional_replay(old_base, reviewed, current_base):
    reviewed_edits = independent_byte_edits(old_base, reviewed)
    upstream_edits = independent_byte_edits(old_base, current_base)
    if independent_edits_interact(reviewed_edits, upstream_edits):
        return None
    reviewed_on_current = independent_apply_edits(
        current_base,
        independent_translate_edits(reviewed_edits, upstream_edits),
    )
    upstream_on_reviewed = independent_apply_edits(
        reviewed,
        independent_translate_edits(upstream_edits, reviewed_edits),
    )
    require(
        reviewed_on_current == upstream_on_reviewed,
        "independent replay orders produced different bytes",
    )
    return reviewed_on_current


def validate_relative_path(value, label):
    path = PurePosixPath(value)
    require(value != "", f"{label} is empty")
    require(not path.is_absolute(), f"{label} is absolute: {value}")
    require(".." not in path.parts, f"{label} escapes its repository: {value}")
    require("." not in path.parts, f"{label} is not normalized: {value}")
    require("\\" not in value, f"{label} must use Git '/' separators: {value}")
    require(str(path) == value, f"{label} is not normalized: {value}")


def validate_tree(tree, label):
    files = tree["files"]
    require(isinstance(files, dict), f"{label}.files must be an object")
    for path, specification in files.items():
        validate_relative_path(path, f"{label} path")
        mode = specification["mode"]
        require(mode in ALLOWED_MODES, f"{label}:{path} has unsupported mode {mode}")
        content = specification["content"]
        require(
            content["encoding"] in ("utf8", "base64"),
            f"{label}:{path} has unsupported content encoding",
        )
        decoded = decode_content(specification)
        require(isinstance(decoded, bytes), f"{label}:{path} did not decode to bytes")


def validate_expected_entry(entry, case_id):
    require(
        entry["status"]
        in ("added", "deleted", "modified", "renamed", "type_changed"),
        f"{case_id} has an unsupported expected status",
    )
    require(
        entry["before_path"] is not None or entry["after_path"] is not None,
        f"{case_id} expected entry has no path",
    )
    if entry["before_path"] is not None:
        validate_relative_path(entry["before_path"], f"{case_id} before_path")
    if entry["after_path"] is not None:
        validate_relative_path(entry["after_path"], f"{case_id} after_path")
    for field in ("additions", "deletions"):
        value = entry[field]
        require(
            value is None or isinstance(value, int) and value >= 0,
            f"{case_id} {field} must be null or a non-negative integer",
        )


def validate_manifest(manifest):
    require(manifest["schema"] == MANIFEST_SCHEMA, "unsupported manifest schema")
    require(manifest["dataset_version"] == "1.0.0", "unsupported dataset version")
    require(manifest["dataset_license"] == "MIT", "unexpected dataset license")
    require(
        manifest["file_scope"]
        == "Regular Git blobs with modes 100644 and 100755; symlinks and gitlinks are outside v1.",
        "unexpected benchmark file scope",
    )
    require(
        set(manifest["required_coverage"]) == REQUIRED_COVERAGE,
        "manifest required_coverage does not match the v1 contract",
    )
    require(manifest["cases"], "manifest has no cases")
    observed_ids = set()
    observed_coverage = set()
    for case in manifest["cases"]:
        case_id = case["id"]
        require(
            re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", case_id) is not None,
            f"invalid case ID: {case_id}",
        )
        require(case_id not in observed_ids, f"duplicate case ID: {case_id}")
        observed_ids.add(case_id)
        require(case["description"], f"{case_id} has no description")
        require(case["covers"], f"{case_id} has no coverage labels")
        observed_coverage.update(case["covers"])
        require(
            set(case["history"]) == set(SNAPSHOT_LABELS),
            f"{case_id} must define exactly A, B, C, and D",
        )
        for snapshot in SNAPSHOT_LABELS:
            validate_tree(case["history"][snapshot], f"{case_id}.{snapshot}")
        expected = case["expected"]
        review_summary = expected["review_summary"]
        for field in (
            "changed_files",
            "needs_review_now_files",
            "unchanged_since_checkpoint_files",
            "retired_change_count",
        ):
            require(
                isinstance(review_summary[field], int) and review_summary[field] >= 0,
                f"{case_id} review_summary.{field} must be a non-negative integer",
            )
        for entry in expected["full_scope"]:
            validate_expected_entry(entry, case_id)
        delta = expected["delta"]
        require(
            delta["comparison"] == "per_file_review_baseline_to_head",
            f"{case_id} must exercise the base-drift review delta",
        )
        summary = delta["summary"]
        require(
            summary["displayable_files"] == len(delta["entries"]),
            f"{case_id} delta displayable_files does not match entries",
        )
        require(
            summary["needs_review_files"]
            == summary["displayable_files"] + summary["unresolved_retired_changes"],
            f"{case_id} delta needs_review_files is inconsistent",
        )
        require(
            summary["gate_passed"] == (summary["needs_review_files"] == 0),
            f"{case_id} delta gate outcome is inconsistent",
        )
        for entry in delta["entries"]:
            validate_expected_entry(entry, case_id)
            require(
                entry["baseline_basis"]
                in (
                    "reconstructed_review_baseline",
                    "current_base_fallback",
                    "current_base_no_checkpoint_change",
                    "checkpoint_head_fallback",
                ),
                f"{case_id} has an unsupported baseline basis",
            )
            require(
                entry["before_source_kind"]
                in ("git_object", "reconstructed_bytes", "empty"),
                f"{case_id} has an unsupported before source",
            )
            require(
                entry["after_source_kind"] in ("git_object", "empty"),
                f"{case_id} has an unsupported after source",
            )
            reconstructed = entry["reconstructed_baseline_utf8"]
            if entry["baseline_basis"] == "reconstructed_review_baseline":
                require(
                    isinstance(reconstructed, str),
                    f"{case_id} reconstructed entry needs exact baseline bytes",
                )
                require(
                    entry["fallback_reason"] is None,
                    f"{case_id} reconstructed entry cannot have a fallback reason",
                )
            else:
                require(
                    reconstructed is None,
                    f"{case_id} fallback entry cannot claim a reconstructed baseline",
                )
        verify_independent_case_oracle(case)
    require(
        observed_coverage == REQUIRED_COVERAGE,
        f"coverage mismatch: missing={sorted(REQUIRED_COVERAGE - observed_coverage)}, "
        f"unexpected={sorted(observed_coverage - REQUIRED_COVERAGE)}",
    )


def load_manifest():
    manifest = load_json(MANIFEST_PATH)
    validate_manifest(manifest)
    return manifest


def clear_worktree(repository):
    for child in repository.iterdir():
        if child.name == ".git":
            continue
        if child.is_dir() and not child.is_symlink():
            shutil.rmtree(child)
        else:
            child.unlink()


def write_tree(repository, tree):
    clear_worktree(repository)
    for relative, specification in sorted(tree["files"].items()):
        target = repository.joinpath(*PurePosixPath(relative).parts)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(decode_content(specification))
        target.chmod(ALLOWED_MODES[specification["mode"]])


def commit_snapshot(repository, case_id, case_index, snapshot_index, label):
    run_git(repository, ["add", "--all"])
    day = case_index + 1
    hour = snapshot_index
    timestamp = f"2001-01-{day:02d}T{hour:02d}:00:00Z"
    identity = {
        "GIT_AUTHOR_NAME": "StrataDiff Benchmark",
        "GIT_AUTHOR_EMAIL": "benchmark@stratadiff.invalid",
        "GIT_AUTHOR_DATE": timestamp,
        "GIT_COMMITTER_NAME": "StrataDiff Benchmark",
        "GIT_COMMITTER_EMAIL": "benchmark@stratadiff.invalid",
        "GIT_COMMITTER_DATE": timestamp,
    }
    run_git(
        repository,
        ["commit", "-q", "--allow-empty", "-m", f"{case_id} snapshot {label}"],
        extra_environment=identity,
    )
    commit = git_text(repository, ["rev-parse", "HEAD"])
    validate_oid(commit, f"{case_id} snapshot {label}")
    run_git(repository, ["tag", f"snapshot-{label}", commit])
    return commit


def materialize_case(case, case_index, destination):
    case_root = destination / case["id"]
    repository = case_root / "repository"
    repository.mkdir(parents=True)
    run_git(repository, ["init", "-q"])
    run_git(repository, ["config", "user.name", "StrataDiff Benchmark"])
    run_git(repository, ["config", "user.email", "benchmark@stratadiff.invalid"])

    snapshots = {}
    write_tree(repository, case["history"]["A"])
    snapshots["A"] = commit_snapshot(repository, case["id"], case_index, 0, "A")
    write_tree(repository, case["history"]["B"])
    snapshots["B"] = commit_snapshot(repository, case["id"], case_index, 1, "B")
    run_git(repository, ["checkout", "-q", "--detach", snapshots["A"]])
    write_tree(repository, case["history"]["C"])
    snapshots["C"] = commit_snapshot(repository, case["id"], case_index, 2, "C")
    write_tree(repository, case["history"]["D"])
    snapshots["D"] = commit_snapshot(repository, case["id"], case_index, 3, "D")

    metadata = {
        "schema": "stratadiff-review-delta-benchmark-materialized-case-v1",
        "case_id": case["id"],
        "repository": "repository",
        "snapshots": snapshots,
    }
    (case_root / "snapshots.json").write_text(
        json.dumps(metadata, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return metadata


def materialize(manifest, destination):
    require(not destination.exists(), f"materialization output already exists: {destination}")
    destination.mkdir(parents=True)
    materialized_cases = []
    for index, case in enumerate(manifest["cases"]):
        materialized_cases.append(materialize_case(case, index, destination))
    metadata = {
        "schema": "stratadiff-review-delta-benchmark-materialization-v1",
        "dataset_version": manifest["dataset_version"],
        "manifest_sha256": sha256_file(MANIFEST_PATH),
        "cases": materialized_cases,
    }
    (destination / "materialization.json").write_text(
        json.dumps(metadata, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return metadata


def zero_to_none(value):
    return None if all(character == "0" for character in value) else value


def parse_raw_diff(raw):
    fields = raw.split(b"\0")
    require(fields[-1] == b"", "raw Git diff is not NUL terminated")
    fields.pop()
    changes = []
    index = 0
    while index < len(fields):
        columns = fields[index].decode("ascii").split()
        index += 1
        require(
            len(columns) == 5 and columns[0].startswith(":"),
            f"invalid raw Git header: {columns}",
        )
        status_code = columns[4][0]
        require(status_code in "ADMT", f"unsupported raw Git status: {columns[4]}")
        require(index < len(fields), "raw Git record is missing its path")
        path = fields[index].decode("utf-8")
        index += 1
        status = {
            "A": "added",
            "D": "deleted",
            "M": "modified",
            "T": "type_changed",
        }[status_code]
        changes.append(
            {
                "status": status,
                "similarity_percent": None,
                "before_path": None if status_code == "A" else path,
                "after_path": None if status_code == "D" else path,
                "before_mode": zero_to_none(columns[0][1:]),
                "after_mode": zero_to_none(columns[1]),
                "before_blob": zero_to_none(columns[2]),
                "after_blob": zero_to_none(columns[3]),
            }
        )
    return pair_unique_exact_relocations(changes)


def pair_unique_exact_relocations(changes):
    candidates = {}
    for index, change in enumerate(changes):
        if change["status"] == "deleted":
            key = (change["before_blob"], change["before_mode"])
            if key not in candidates:
                candidates[key] = {"deleted": [], "added": []}
            candidates[key]["deleted"].append(index)
        elif change["status"] == "added":
            key = (change["after_blob"], change["after_mode"])
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
            replacement_index = min(deleted_index, added_index)
            replacements[replacement_index] = {
                "status": "renamed",
                "similarity_percent": 100,
                "before_path": before["before_path"],
                "after_path": after["after_path"],
                "before_mode": before["before_mode"],
                "after_mode": after["after_mode"],
                "before_blob": before["before_blob"],
                "after_blob": after["after_blob"],
            }
            removed.add(deleted_index)
            removed.add(added_index)

    output = []
    for index, change in enumerate(changes):
        if index in replacements:
            output.append(replacements[index])
        elif index not in removed:
            output.append(change)
    return output


def raw_full_scope(repository, snapshots):
    raw = git_bytes(
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
            snapshots["C"],
            snapshots["D"],
            "--",
        ],
    )
    return parse_raw_diff(raw)


def report_identity(file):
    before_path = optional_field(file, "before_path")
    after_path = optional_field(file, "after_path")
    if before_path is not None:
        require(file["before_path_encoding"] == "utf8", "benchmark path is not UTF-8")
    if after_path is not None:
        require(file["after_path_encoding"] == "utf8", "benchmark path is not UTF-8")
    return {
        "status": file["status"],
        "similarity_percent": optional_field(file, "similarity_percent"),
        "before_path": before_path,
        "after_path": after_path,
        "before_mode": optional_field(file, "before_mode"),
        "after_mode": optional_field(file, "after_mode"),
        "before_blob": optional_field(file, "before_blob"),
        "after_blob": optional_field(file, "after_blob"),
    }


def identity_sort_key(identity):
    before_path = identity["before_path"] if identity["before_path"] is not None else ""
    after_path = identity["after_path"] if identity["after_path"] is not None else ""
    return (before_path, after_path, identity["status"])


def line_counts(file):
    if "line_change_envelope" not in file:
        return (None, None)
    envelope = file["line_change_envelope"]
    return (envelope["additions"], envelope["deletions"])


def source_bytes_from_tree(case, snapshot, path):
    if path is None:
        return b""
    files = case["history"][snapshot]["files"]
    require(path in files, f"{case['id']} {snapshot} has no expected source path {path}")
    return decode_content(files[path])


def source_identity_from_tree(case, snapshot, path):
    if path is None:
        return (None, None)
    files = case["history"][snapshot]["files"]
    require(path in files, f"{case['id']} {snapshot} has no expected identity path {path}")
    specification = files[path]
    return (specification["mode"], git_blob_oid(decode_content(specification)))


def expected_change_identity(
    case,
    entry,
    before_snapshot,
    after_snapshot,
    *,
    synthetic_before=False,
):
    before_mode, before_blob = source_identity_from_tree(
        case,
        before_snapshot,
        entry["before_path"],
    )
    after_mode, after_blob = source_identity_from_tree(
        case,
        after_snapshot,
        entry["after_path"],
    )
    if synthetic_before:
        require(before_blob is not None, f"{case['id']} synthetic baseline has no path")
        before_blob = None
    similarity_percent = None
    if entry["status"] == "renamed":
        require(
            before_blob == after_blob and before_mode == after_mode,
            f"{case['id']} expected rename is not an exact relocation",
        )
        similarity_percent = 100
    return {
        "status": entry["status"],
        "similarity_percent": similarity_percent,
        "before_path": entry["before_path"],
        "after_path": entry["after_path"],
        "before_mode": before_mode,
        "after_mode": after_mode,
        "before_blob": before_blob,
        "after_blob": after_blob,
    }


def expected_full_identities(case):
    return sorted(
        [
            expected_change_identity(case, entry, "C", "D")
            for entry in case["expected"]["full_scope"]
        ],
        key=identity_sort_key,
    )


def expected_full_scope(case):
    output = []
    for entry in case["expected"]["full_scope"]:
        before = source_bytes_from_tree(case, "C", entry["before_path"])
        after = source_bytes_from_tree(case, "D", entry["after_path"])
        output.append(
            {
                "status": entry["status"],
                "before_path": entry["before_path"],
                "after_path": entry["after_path"],
                "additions": entry["additions"],
                "deletions": entry["deletions"],
                "before_sha256": sha256_bytes(before),
                "after_sha256": sha256_bytes(after),
            }
        )
    return sorted(output, key=identity_sort_key)


def actual_full_scope(report, sources):
    require(len(report["files"]) == len(sources), "Full source count differs from report")
    output = []
    for file, source_pair in zip(report["files"], sources, strict=True):
        additions, deletions = line_counts(file)
        output.append(
            {
                "status": file["status"],
                "before_path": optional_field(file, "before_path"),
                "after_path": optional_field(file, "after_path"),
                "additions": additions,
                "deletions": deletions,
                "before_sha256": sha256_bytes(source_pair[0]),
                "after_sha256": sha256_bytes(source_pair[1]),
            }
        )
    return sorted(output, key=identity_sort_key)


def expected_delta_source_bytes(case, entry, side):
    if side == "before":
        if entry["before_source_kind"] == "empty":
            return b""
        if entry["before_source_kind"] == "reconstructed_bytes":
            return entry["reconstructed_baseline_utf8"].encode("utf-8")
        if entry["baseline_basis"] in (
            "current_base_fallback",
            "current_base_no_checkpoint_change",
        ):
            return source_bytes_from_tree(case, "C", entry["before_path"])
        if entry["baseline_basis"] in ("checkpoint_head_fallback", "checkpoint_snapshot"):
            return source_bytes_from_tree(case, "B", entry["before_path"])
        raise ValueError(f"{case['id']} has no before-source rule for {entry['baseline_basis']}")
    if entry["after_source_kind"] == "empty":
        return b""
    return source_bytes_from_tree(case, "D", entry["after_path"])


def independent_reconstructed_source(case, entry):
    path = entry["before_path"]
    require(
        path is not None and entry["after_path"] == path,
        f"{case['id']} reconstructed entry is not a same-path modification",
    )
    reconstructed = independent_bidirectional_replay(
        source_bytes_from_tree(case, "A", path),
        source_bytes_from_tree(case, "B", path),
        source_bytes_from_tree(case, "C", path),
    )
    require(reconstructed is not None, f"{case['id']} independent replay found interaction")
    return reconstructed


def expected_delta_identity(case, entry):
    basis = entry["baseline_basis"]
    if basis == "reconstructed_review_baseline":
        return expected_change_identity(
            case,
            entry,
            "C",
            "D",
            synthetic_before=True,
        )
    if basis in ("current_base_fallback", "current_base_no_checkpoint_change"):
        return expected_change_identity(case, entry, "C", "D")
    if basis in ("checkpoint_head_fallback", "checkpoint_snapshot"):
        return expected_change_identity(case, entry, "B", "D")
    raise ValueError(f"{case['id']} has no identity rule for {basis}")


def expected_delta_entries(case):
    output = []
    for entry in case["expected"]["delta"]["entries"]:
        reconstructed = entry["reconstructed_baseline_utf8"]
        before = expected_delta_source_bytes(case, entry, "before")
        after = expected_delta_source_bytes(case, entry, "after")
        if entry["baseline_basis"] == "reconstructed_review_baseline":
            independent_before = independent_reconstructed_source(case, entry)
            require(
                before == independent_before,
                f"{case['id']} frozen reconstruction differs from the independent byte replay",
            )
        identity = expected_delta_identity(case, entry)
        output.append(
            {
                "status": entry["status"],
                "similarity_percent": identity["similarity_percent"],
                "before_path": entry["before_path"],
                "after_path": entry["after_path"],
                "before_mode": identity["before_mode"],
                "after_mode": identity["after_mode"],
                "before_blob": identity["before_blob"],
                "after_blob": identity["after_blob"],
                "baseline_basis": entry["baseline_basis"],
                "fallback_reason": entry["fallback_reason"],
                "before_source_kind": entry["before_source_kind"],
                "after_source_kind": entry["after_source_kind"],
                "additions": entry["additions"],
                "deletions": entry["deletions"],
                "reconstructed_baseline_byte_len": (
                    len(reconstructed.encode("utf-8")) if reconstructed is not None else None
                ),
                "reconstruction_hashes_agree": True if reconstructed is not None else None,
                "before_sha256": sha256_bytes(before),
                "after_sha256": sha256_bytes(after),
            }
        )
    return sorted(output, key=identity_sort_key)


def replay_candidate_path(case):
    histories = case["history"]
    shared_paths = set(histories["A"]["files"])
    for snapshot in ("B", "C", "D"):
        shared_paths &= set(histories[snapshot]["files"])
    candidates = []
    for path in sorted(shared_paths):
        specifications = [histories[label]["files"][path] for label in SNAPSHOT_LABELS]
        if len({specification["mode"] for specification in specifications}) != 1:
            continue
        old_base = decode_content(specifications[0])
        reviewed = decode_content(specifications[1])
        current_base = decode_content(specifications[2])
        if old_base != reviewed and old_base != current_base:
            candidates.append(path)
    require(
        len(candidates) == 1,
        f"{case['id']} expected exactly one byte-replay candidate, observed {candidates}",
    )
    return candidates[0]


def verify_independent_case_oracle(case):
    case_id = case["id"]
    covers = set(case["covers"])
    expected = case["expected"]
    require(
        expected["review_summary"]["changed_files"] == len(expected["full_scope"]),
        f"{case_id} Full summary count differs from its frozen scope",
    )

    for entry in expected["full_scope"]:
        before = source_bytes_from_tree(case, "C", entry["before_path"])
        after = source_bytes_from_tree(case, "D", entry["after_path"])
        require(
            independent_line_counts(before, after)
            == (entry["additions"], entry["deletions"]),
            f"{case_id} Full line counts disagree with the independent line oracle",
        )

    for entry in expected["delta"]["entries"]:
        if entry["baseline_basis"] == "reconstructed_review_baseline":
            before = independent_reconstructed_source(case, entry)
            require(
                before == entry["reconstructed_baseline_utf8"].encode("utf-8"),
                f"{case_id} frozen baseline disagrees with independent byte replay",
            )
        else:
            before = expected_delta_source_bytes(case, entry, "before")
        after = expected_delta_source_bytes(case, entry, "after")
        require(
            independent_line_counts(before, after)
            == (entry["additions"], entry["deletions"]),
            f"{case_id} Resume line counts disagree with the independent line oracle",
        )

    replay_labels = {
        "pure_rebase",
        "noninteracting_author_followup",
        "drop_or_revert",
        "overlap_fallback",
        "adjacent_fallback",
        "binary_fallback",
    }
    if covers & replay_labels and "dropped_rename" not in covers:
        path = replay_candidate_path(case)
        old_base = source_bytes_from_tree(case, "A", path)
        reviewed = source_bytes_from_tree(case, "B", path)
        current_base = source_bytes_from_tree(case, "C", path)
        head = source_bytes_from_tree(case, "D", path)
        reviewed_edits = independent_byte_edits(old_base, reviewed)
        upstream_edits = independent_byte_edits(old_base, current_base)
        interacts = independent_edits_interact(reviewed_edits, upstream_edits)

        if "overlap_fallback" in covers:
            strictly_overlaps = any(
                left_start < right_end and right_start < left_end
                for left_start, left_end, _ in reviewed_edits
                for right_start, right_end, _ in upstream_edits
            )
            require(interacts and strictly_overlaps, f"{case_id} is not an overlap case")
        elif "adjacent_fallback" in covers:
            adjacent = any(
                left_end == right_start or right_end == left_start
                for left_start, left_end, _ in reviewed_edits
                for right_start, right_end, _ in upstream_edits
            )
            require(interacts and adjacent, f"{case_id} is not an adjacent-edit case")
        elif "binary_fallback" in covers:
            require(
                any(b"\0" in value for value in (old_base, reviewed, current_base, head)),
                f"{case_id} binary fallback has no NUL byte",
            )
        else:
            reconstructed = independent_bidirectional_replay(
                old_base,
                reviewed,
                current_base,
            )
            require(reconstructed is not None, f"{case_id} independent replay failed")
            if "pure_rebase" in covers:
                require(reconstructed == head, f"{case_id} is not a pure carried rebase")
                require(
                    not expected["delta"]["entries"],
                    f"{case_id} pure rebase unexpectedly has Resume entries",
                )
            if "noninteracting_author_followup" in covers:
                require(reconstructed != head, f"{case_id} has no author follow-up residue")
            if "drop_or_revert" in covers:
                require(current_base == head, f"{case_id} current PR is not empty")
                require(reconstructed != head, f"{case_id} did not drop the reviewed edit")

    if "add_fallback" in covers:
        require(
            any(
                path not in case["history"]["A"]["files"]
                and path in case["history"]["B"]["files"]
                and path not in case["history"]["C"]["files"]
                and path in case["history"]["D"]["files"]
                for path in case["history"]["B"]["files"]
            ),
            f"{case_id} does not contain the claimed dual-branch addition",
        )
    if "delete_fallback" in covers:
        require(
            any(
                path not in case["history"]["B"]["files"]
                and path in case["history"]["C"]["files"]
                and path not in case["history"]["D"]["files"]
                for path in case["history"]["A"]["files"]
            ),
            f"{case_id} does not contain the claimed dual-branch deletion",
        )
    if "dropped_rename" in covers:
        require(not expected["full_scope"], f"{case_id} dropped rename has non-empty Full scope")
        require(
            {
                (entry["status"], entry["before_path"], entry["after_path"])
                for entry in expected["delta"]["entries"]
            }
            == {("deleted", "new.py", None), ("added", None, "old.py")},
            f"{case_id} does not expose both sides of the dropped rename",
        )
        require(
            all(
                entry["baseline_basis"] == "checkpoint_head_fallback"
                for entry in expected["delta"]["entries"]
            ),
            f"{case_id} dropped rename does not use checkpoint-to-head sources",
        )
    if "absorbed_retired_change" in covers:
        absorbed_paths = [
            path
            for path in case["history"]["B"]["files"]
            if path not in case["history"]["A"]["files"]
            and path in case["history"]["C"]["files"]
            and path in case["history"]["D"]["files"]
            and source_bytes_from_tree(case, "B", path)
            == source_bytes_from_tree(case, "C", path)
            == source_bytes_from_tree(case, "D", path)
        ]
        require(
            len(absorbed_paths) == 1,
            f"{case_id} does not contain one byte-identical absorbed addition",
        )
        require(
            expected["review_summary"]["retired_change_count"] > 0
            and not expected["full_scope"]
            and not expected["delta"]["entries"]
            and expected["delta"]["summary"]["gate_passed"] is True,
            f"{case_id} does not disprove a retired-count-only gate",
        )
    if "new_current_change" in covers:
        require(
            len(expected["delta"]["entries"]) == 1
            and expected["delta"]["entries"][0]["baseline_basis"]
            == "current_base_no_checkpoint_change",
            f"{case_id} does not isolate one post-checkpoint current change",
        )


def actual_delta_entries(delta, sources):
    require(len(delta["entries"]) == len(sources), "Resume source count differs from delta")
    output = []
    for entry, source_pair in zip(delta["entries"], sources, strict=True):
        file = entry["file"]
        additions, deletions = line_counts(file)
        reconstruction = optional_field(entry, "baseline_reconstruction")
        if reconstruction is None:
            reconstruction_byte_len = None
            hashes_agree = None
        else:
            reconstruction_byte_len = reconstruction["byte_len"]
            hashes_agree = (
                reconstruction["reviewed_on_current_base_blake3"]
                == reconstruction["upstream_on_checkpoint_blake3"]
                == reconstruction["reconstructed_blake3"]
                == entry["before_source"]["blake3"]
            )
            require(
                reconstruction["algorithm"] == "bidirectional_noninteracting_byte_replay_v1",
                "unexpected reconstructed-baseline algorithm",
            )
            require("before_blob" not in file, "synthetic baseline is represented as a Git blob")
        output.append(
            {
                "status": file["status"],
                "similarity_percent": optional_field(file, "similarity_percent"),
                "before_path": optional_field(file, "before_path"),
                "after_path": optional_field(file, "after_path"),
                "before_mode": optional_field(file, "before_mode"),
                "after_mode": optional_field(file, "after_mode"),
                "before_blob": optional_field(file, "before_blob"),
                "after_blob": optional_field(file, "after_blob"),
                "baseline_basis": entry["baseline_basis"],
                "fallback_reason": optional_field(entry, "fallback_reason"),
                "before_source_kind": entry["before_source"]["kind"],
                "after_source_kind": entry["after_source"]["kind"],
                "additions": additions,
                "deletions": deletions,
                "reconstructed_baseline_byte_len": reconstruction_byte_len,
                "reconstruction_hashes_agree": hashes_agree,
                "before_sha256": sha256_bytes(source_pair[0]),
                "after_sha256": sha256_bytes(source_pair[1]),
            }
        )
    return sorted(output, key=identity_sort_key)


def git_object_bytes(repository, object_id):
    validate_oid(object_id, "Git source object")
    return git_bytes(repository, ["cat-file", "blob", object_id])


def validate_delta_source_references(repository, snapshots, delta, sources):
    for entry, source_pair in zip(delta["entries"], sources, strict=True):
        file = entry["file"]
        basis = entry["baseline_basis"]
        before_source = entry["before_source"]
        after_source = entry["after_source"]
        if before_source["kind"] == "git_object":
            expected_commit = (
                snapshots["B"]
                if basis in ("checkpoint_head_fallback", "checkpoint_snapshot")
                else snapshots["C"]
            )
            require(before_source["commit"] == expected_commit, "before source has wrong commit")
            require(
                before_source["object_id"] == file["before_blob"],
                "before source and file blob differ",
            )
            require(
                source_pair[0] == git_object_bytes(repository, before_source["object_id"]),
                "viewer before bytes differ from the recorded Git object",
            )
        elif before_source["kind"] == "empty":
            require(source_pair[0] == b"", "empty before source returned non-empty bytes")
        else:
            require(
                before_source["kind"] == "reconstructed_bytes",
                "unsupported before source kind",
            )
            require(
                before_source["byte_len"] == len(source_pair[0]),
                "reconstructed before source length differs",
            )

        if after_source["kind"] == "git_object":
            require(after_source["commit"] == snapshots["D"], "after source has wrong commit")
            require(
                after_source["object_id"] == file["after_blob"],
                "after source and file blob differ",
            )
            require(
                source_pair[1] == git_object_bytes(repository, after_source["object_id"]),
                "viewer after bytes differ from the recorded Git object",
            )
        else:
            require(after_source["kind"] == "empty", "unsupported after source kind")
            require(source_pair[1] == b"", "empty after source returned non-empty bytes")


def fetch_url(url):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(url, timeout=15) as response:
        require(response.status == 200, f"viewer returned HTTP {response.status}")
        return response.read()


def workbench_sources(binary, repository, snapshots, report, delta):
    command = [
        str(binary),
        "review",
        "--repo",
        str(repository),
        "--checkpoint",
        snapshots["B"],
        "--workbench",
        "--port",
        "0",
        "--no-open",
        "--",
        snapshots["C"],
        snapshots["D"],
    ]
    process = subprocess.Popen(
        command,
        env=isolated_environment(),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    require(process.stderr is not None, "workbench stderr pipe was not created")
    try:
        selector = selectors.DefaultSelector()
        selector.register(process.stderr, selectors.EVENT_READ)
        require(selector.select(timeout=20), "workbench did not publish its URL within 20 seconds")
        first_line = process.stderr.readline().decode("utf-8").strip()
        prefix = "StrataDiff Review Resume Workbench: "
        require(first_line.startswith(prefix), f"unexpected workbench startup line: {first_line}")
        parsed = urllib.parse.urlparse(first_line[len(prefix):])
        query = urllib.parse.parse_qs(parsed.query)
        require(parsed.hostname == "127.0.0.1", "workbench did not bind to loopback")
        require("token" in query and len(query["token"]) == 1, "workbench URL has no token")
        token = query["token"][0]
        base_url = f"http://{parsed.netloc}"
        session_url = f"{base_url}/api/session?{urllib.parse.urlencode({'token': token})}"
        session = json.loads(fetch_url(session_url))
        require(session["review"] == report, "workbench Full report differs from CLI artifact")
        require(session["resume_delta"] == delta, "workbench Resume delta differs from CLI artifact")

        def scope_sources(scope, count):
            output = []
            for index in range(count):
                parameters = urllib.parse.urlencode(
                    {"token": token, "file": str(index), "scope": scope}
                )
                before = fetch_url(f"{base_url}/api/source/before?{parameters}")
                after = fetch_url(f"{base_url}/api/source/after?{parameters}")
                output.append((before, after))
            return output

        full_sources = scope_sources("full", len(report["files"]))
        resume_sources = scope_sources("resume", len(delta["entries"]))
        return full_sources, resume_sources
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def verify_materialized_case(case, case_root, metadata):
    repository = case_root / "repository"
    snapshots = metadata["snapshots"]
    require(metadata["case_id"] == case["id"], "materialized case ID differs")
    for label in SNAPSHOT_LABELS:
        validate_oid(snapshots[label], f"{case['id']} {label}")
        require(
            git_text(repository, ["rev-parse", f"snapshot-{label}^{{commit}}"])
            == snapshots[label],
            f"{case['id']} tag snapshot-{label} differs",
        )
        records = git_bytes(repository, ["ls-tree", "-r", "-z", snapshots[label]])
        observed = {}
        for record in records.split(b"\0"):
            if not record:
                continue
            metadata_bytes, path_bytes = record.split(b"\t", 1)
            mode, object_type, object_id = metadata_bytes.decode("ascii").split()
            require(object_type == "blob", f"{case['id']} contains a non-blob tree entry")
            path = path_bytes.decode("utf-8")
            observed[path] = {
                "mode": mode,
                "content": git_object_bytes(repository, object_id),
            }
        expected_files = case["history"][label]["files"]
        require(set(observed) == set(expected_files), f"{case['id']} {label} paths differ")
        for path, specification in expected_files.items():
            require(
                observed[path]["mode"] == specification["mode"],
                f"{case['id']} {label}:{path} mode differs",
            )
            require(
                observed[path]["content"] == decode_content(specification),
                f"{case['id']} {label}:{path} bytes differ",
            )

    parents = git_text(repository, ["rev-list", "--parents", "-n", "1", snapshots["A"]]).split()
    require(parents == [snapshots["A"]], f"{case['id']} A is not a root commit")
    for label, parent_label in PARENT_LABELS.items():
        parents = git_text(
            repository, ["rev-list", "--parents", "-n", "1", snapshots[label]]
        ).split()
        require(
            parents == [snapshots[label], snapshots[parent_label]],
            f"{case['id']} {label} does not descend directly from {parent_label}",
        )
    require(
        git_text(repository, ["merge-base", snapshots["B"], snapshots["D"]])
        == snapshots["A"],
        f"{case['id']} old merge base is not A",
    )
    require(
        git_text(repository, ["merge-base", snapshots["C"], snapshots["D"]])
        == snapshots["C"],
        f"{case['id']} current merge base is not C",
    )
    require(
        git_text(repository, ["rev-parse", f"{snapshots['A']}^{{tree}}"])
        != git_text(repository, ["rev-parse", f"{snapshots['C']}^{{tree}}"]),
        f"{case['id']} does not contain actual base drift",
    )
    git_scope = raw_full_scope(repository, snapshots)
    require(
        sorted(git_scope, key=identity_sort_key) == expected_full_identities(case),
        f"{case['id']} expected Full identities do not match its C->D Git history",
    )


def review_summary(report):
    checkpoint = report["summary"]["checkpoint"]
    return {
        "changed_files": report["summary"]["changed_files"],
        "needs_review_now_files": checkpoint["needs_review_now_files"],
        "unchanged_since_checkpoint_files": checkpoint["unchanged_since_checkpoint_files"],
        "retired_change_count": checkpoint["retired_change_count"],
    }


def run_cli(binary, arguments, *, check=True):
    return subprocess.run(
        [str(binary), *arguments],
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=check,
    )


def evaluate_case(binary, case, case_root, metadata):
    repository = case_root / "repository"
    snapshots = metadata["snapshots"]
    report_path = case_root / "full-review.json"
    delta_path = case_root / "review-delta.json"
    output = run_cli(
        binary,
        [
            "review",
            "--repo",
            str(repository),
            "--checkpoint",
            snapshots["B"],
            "--format=json",
            f"--output={report_path}",
            f"--review-delta-output={delta_path}",
            "--",
            snapshots["C"],
            snapshots["D"],
        ],
    )
    require(output.returncode == 0, f"{case['id']} review command failed")
    report = load_json(report_path)
    delta = load_json(delta_path)
    require(report["schema"] == REVIEW_SCHEMA, f"{case['id']} review schema differs")
    require(delta["schema"] == REVIEW_DELTA_SCHEMA, f"{case['id']} delta schema differs")
    require(report["engine_version"] == delta["engine_version"], "engine versions differ")
    require(report["base_commit"] == snapshots["C"], f"{case['id']} Full base is not C")
    require(report["head_commit"] == snapshots["D"], f"{case['id']} Full head is not D")
    require(report["comparison"] == "merge_base_to_head", "unexpected Full comparison")
    require(report["checkpoint"]["base_commit"] == snapshots["A"], "old base is not A")
    require(report["checkpoint"]["commit"] == snapshots["B"], "checkpoint is not B")
    require(delta["old_base_commit"] == snapshots["A"], "delta old base is not A")
    require(delta["checkpoint_commit"] == snapshots["B"], "delta checkpoint is not B")
    require(delta["current_base_commit"] == snapshots["C"], "delta current base is not C")
    require(delta["head_commit"] == snapshots["D"], "delta head is not D")

    git_identities = sorted(raw_full_scope(repository, snapshots), key=identity_sort_key)
    report_identities = sorted(
        [report_identity(file) for file in report["files"]], key=identity_sort_key
    )
    require(
        report_identities == git_identities,
        f"{case['id']} Full scope is not the exact C->D Git identity set",
    )
    require(
        git_identities == expected_full_identities(case),
        f"{case['id']} independent Full identity oracle differs",
    )
    actual_summary = review_summary(report)
    require(
        actual_summary == case["expected"]["review_summary"],
        f"{case['id']} review summary differs: {actual_summary}",
    )
    require(
        delta["comparison"] == case["expected"]["delta"]["comparison"],
        f"{case['id']} delta comparison differs",
    )
    require(
        delta["summary"] == case["expected"]["delta"]["summary"],
        f"{case['id']} delta summary differs: {delta['summary']}",
    )
    require(
        len(delta["unresolved_retired_changes"])
        == delta["summary"]["unresolved_retired_changes"],
        f"{case['id']} unresolved count differs",
    )

    full_sources, resume_sources = workbench_sources(
        binary, repository, snapshots, report, delta
    )
    for file, source_pair in zip(report["files"], full_sources, strict=True):
        before_blob = optional_field(file, "before_blob")
        after_blob = optional_field(file, "after_blob")
        expected_before = b"" if before_blob is None else git_object_bytes(repository, before_blob)
        expected_after = b"" if after_blob is None else git_object_bytes(repository, after_blob)
        require(source_pair[0] == expected_before, f"{case['id']} Full before source differs")
        require(source_pair[1] == expected_after, f"{case['id']} Full after source differs")
    validate_delta_source_references(repository, snapshots, delta, resume_sources)

    normalized_full = actual_full_scope(report, full_sources)
    expected_full = expected_full_scope(case)
    require(normalized_full == expected_full, f"{case['id']} Full outcome differs")
    normalized_delta = actual_delta_entries(delta, resume_sources)
    expected_delta = expected_delta_entries(case)
    require(normalized_delta == expected_delta, f"{case['id']} delta entries differ")

    gate_path = case_root / "gate-review.json"
    gate = run_cli(
        binary,
        [
            "review",
            "--repo",
            str(repository),
            "--checkpoint",
            snapshots["B"],
            "--format=json",
            f"--output={gate_path}",
            "--fail-on-review-residue",
            "--",
            snapshots["C"],
            snapshots["D"],
        ],
        check=False,
    )
    expected_gate_passed = case["expected"]["delta"]["summary"]["gate_passed"]
    require(
        (gate.returncode == 0) == expected_gate_passed,
        f"{case['id']} gate exit code contradicts the expected delta",
    )
    if not expected_gate_passed:
        require(
            b"review delta gate is open" in gate.stderr,
            f"{case['id']} failing gate omitted its reason",
        )

    return {
        "id": case["id"],
        "snapshots": snapshots,
        "review_summary": actual_summary,
        "full_scope": normalized_full,
        "full_scope_git_identities": git_identities,
        "full_scope_report_identities": report_identities,
        "delta": {
            "comparison": delta["comparison"],
            "summary": delta["summary"],
            "entries": normalized_delta,
        },
        "gate_exit_code": gate.returncode,
        "workbench": {
            "full_files_verified": len(full_sources),
            "resume_files_verified": len(resume_sources),
        },
    }


def verify_evaluation(manifest, evaluation):
    require(evaluation["schema"] == EVALUATION_SCHEMA, "unsupported evaluation schema")
    require(
        evaluation["dataset_version"] == manifest["dataset_version"],
        "evaluation dataset version differs",
    )
    require(
        evaluation["manifest_sha256"] == sha256_file(MANIFEST_PATH),
        "evaluation was produced from another manifest",
    )
    require(evaluation["workbench_verified"] is True, "workbench sources were not verified")
    build_info = evaluation["build_info"]
    require(build_info["schema"] == "stratadiff-build-info-v1", "invalid build-info schema")
    require(build_info["engine_version"], "build-info has no engine version")
    require(build_info["git_revision"], "build-info has no Git revision")
    require(build_info["cargo_lock_sha256"], "build-info has no Cargo.lock digest")
    require(build_info["build_profile"], "build-info has no build profile")
    require(build_info["rustc_version"], "build-info has no Rust compiler version")
    if evaluation["runner_policy"]["require_clean"]:
        require(build_info["git_dirty"] is False, "publishable evaluation used a dirty build")
        require(build_info["build_profile"] == "release", "publishable evaluation is not release")
    require(
        evaluation["summary"]
        == {"cases": len(manifest["cases"]), "passed": len(manifest["cases"]), "failed": 0},
        "evaluation summary is not a complete pass",
    )
    require(
        len(evaluation["cases"]) == len(manifest["cases"]),
        "evaluation case count differs",
    )
    observed_ids = set()
    manifest_by_id = {case["id"]: case for case in manifest["cases"]}
    for result in evaluation["cases"]:
        case_id = result["id"]
        require(case_id in manifest_by_id, f"unknown evaluation case: {case_id}")
        require(case_id not in observed_ids, f"duplicate evaluation case: {case_id}")
        observed_ids.add(case_id)
        case = manifest_by_id[case_id]
        for label in SNAPSHOT_LABELS:
            validate_oid(result["snapshots"][label], f"{case_id} snapshot {label}")
        require(
            result["review_summary"] == case["expected"]["review_summary"],
            f"{case_id} recorded review summary differs",
        )
        require(
            result["full_scope"] == expected_full_scope(case),
            f"{case_id} recorded Full scope differs",
        )
        require(
            result["full_scope_git_identities"] == result["full_scope_report_identities"],
            f"{case_id} recorded Full identities do not prove C->D",
        )
        require(
            result["full_scope_git_identities"] == expected_full_identities(case),
            f"{case_id} recorded Full identities differ from the manifest history",
        )
        expected_delta = case["expected"]["delta"]
        require(
            result["delta"]["comparison"] == expected_delta["comparison"],
            f"{case_id} recorded delta comparison differs",
        )
        require(
            result["delta"]["summary"] == expected_delta["summary"],
            f"{case_id} recorded delta summary differs",
        )
        require(
            result["delta"]["entries"] == expected_delta_entries(case),
            f"{case_id} recorded delta entries differ",
        )
        gate_passed = expected_delta["summary"]["gate_passed"]
        require(
            (result["gate_exit_code"] == 0) == gate_passed,
            f"{case_id} recorded gate exit differs",
        )
        require(
            result["workbench"]["full_files_verified"] == len(result["full_scope"]),
            f"{case_id} did not verify every Full source pair",
        )
        require(
            result["workbench"]["resume_files_verified"]
            == len(result["delta"]["entries"]),
            f"{case_id} did not verify every Resume source pair",
        )
    require(observed_ids == set(manifest_by_id), "evaluation omitted cases")


def command_self_test(manifest):
    with tempfile.TemporaryDirectory(prefix="stratadiff-review-delta-v1-") as temporary:
        first = Path(temporary) / "first"
        second = Path(temporary) / "second"
        first_metadata = materialize(manifest, first)
        second_metadata = materialize(manifest, second)
        first_snapshots = {
            case["case_id"]: case["snapshots"] for case in first_metadata["cases"]
        }
        second_snapshots = {
            case["case_id"]: case["snapshots"] for case in second_metadata["cases"]
        }
        require(first_snapshots == second_snapshots, "synthetic commit IDs are not deterministic")
        metadata_by_id = {
            case["case_id"]: case for case in first_metadata["cases"]
        }
        for case in manifest["cases"]:
            verify_materialized_case(case, first / case["id"], metadata_by_id[case["id"]])
    print(f"review-delta-v1 self-test passed: {len(manifest['cases'])} deterministic histories")


def command_run(manifest, binary, output, workdir, require_clean):
    binary = binary.resolve()
    require(binary.is_file(), f"StrataDiff binary does not exist: {binary}")
    version = run_cli(binary, ["--version"]).stdout.decode("utf-8").strip()
    build_info = json.loads(run_cli(binary, ["build-info"]).stdout)
    require(build_info["schema"] == "stratadiff-build-info-v1", "invalid build-info schema")
    if require_clean:
        require(build_info["git_dirty"] is False, "--require-clean rejected a dirty build")
        require(
            build_info["build_profile"] == "release",
            "--require-clean requires a release build",
        )
    if workdir is None:
        temporary = tempfile.TemporaryDirectory(prefix="stratadiff-review-delta-v1-run-")
        materialization_root = Path(temporary.name) / "materialized"
    else:
        temporary = None
        materialization_root = workdir.resolve()
    try:
        metadata = materialize(manifest, materialization_root)
        metadata_by_id = {
            case["case_id"]: case for case in metadata["cases"]
        }
        results = []
        for case in manifest["cases"]:
            case_root = materialization_root / case["id"]
            case_metadata = metadata_by_id[case["id"]]
            verify_materialized_case(case, case_root, case_metadata)
            results.append(evaluate_case(binary, case, case_root, case_metadata))
        evaluation = {
            "schema": EVALUATION_SCHEMA,
            "dataset_version": manifest["dataset_version"],
            "manifest_sha256": sha256_file(MANIFEST_PATH),
            "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
            "tool": {
                "path": str(binary),
                "sha256": sha256_file(binary),
                "version": version,
            },
            "build_info": build_info,
            "runner_policy": {"require_clean": require_clean},
            "workbench_verified": True,
            "summary": {"cases": len(results), "passed": len(results), "failed": 0},
            "cases": results,
        }
        verify_evaluation(manifest, evaluation)
        encoded = json.dumps(evaluation, indent=2, sort_keys=True) + "\n"
        if output is None:
            sys.stdout.write(encoded)
        else:
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(encoded, encoding="utf-8")
            print(f"review-delta-v1 passed {len(results)} cases; wrote {output}")
    finally:
        if temporary is not None:
            temporary.cleanup()


def build_parser():
    parser = argparse.ArgumentParser(
        description="Materialize and verify the controlled review-delta-v1 histories."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("validate", help="validate the checked-in manifest")
    subparsers.add_parser("self-test", help="verify deterministic history materialization")

    materialize_parser = subparsers.add_parser(
        "materialize", help="create the synthetic Git repositories"
    )
    materialize_parser.add_argument("--output", type=Path, required=True)

    run_parser = subparsers.add_parser(
        "run", help="run StrataDiff, its gate, and both workbench scopes"
    )
    run_parser.add_argument("--stratadiff", type=Path, required=True)
    run_parser.add_argument("--output", type=Path)
    run_parser.add_argument(
        "--workdir",
        type=Path,
        help="retain the materialized histories at this new path",
    )
    run_parser.add_argument(
        "--require-clean",
        action="store_true",
        help="require a clean release build for a publishable evaluation",
    )

    verify_parser = subparsers.add_parser(
        "verify", help="verify a previously written evaluation artifact"
    )
    verify_parser.add_argument("--evaluation", type=Path, required=True)
    return parser


def run_entry(arguments=None):
    parser = argparse.ArgumentParser(
        description="Run the controlled review-delta-v1 histories."
    )
    parser.add_argument("--stratadiff", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--workdir",
        type=Path,
        help="retain the materialized histories at this new path",
    )
    parser.add_argument(
        "--require-clean",
        action="store_true",
        help="require a clean release build for a publishable evaluation",
    )
    args = parser.parse_args(arguments)
    command_run(
        load_manifest(),
        args.stratadiff,
        args.output,
        args.workdir,
        args.require_clean,
    )
    return 0


def verify_entry(arguments=None):
    parser = argparse.ArgumentParser(
        description="Verify a saved review-delta-v1 evaluation artifact."
    )
    parser.add_argument("--evaluation", type=Path, required=True)
    args = parser.parse_args(arguments)
    manifest = load_manifest()
    evaluation = load_json(args.evaluation)
    verify_evaluation(manifest, evaluation)
    print(f"review-delta-v1 evaluation verified: {len(evaluation['cases'])} cases")
    return 0


def main(arguments=None):
    parser = build_parser()
    args = parser.parse_args(arguments)
    manifest = load_manifest()
    if args.command == "validate":
        print(f"review-delta-v1 manifest valid: {len(manifest['cases'])} cases")
        return 0
    if args.command == "self-test":
        command_self_test(manifest)
        return 0
    if args.command == "materialize":
        metadata = materialize(manifest, args.output.resolve())
        print(f"materialized {len(metadata['cases'])} histories at {args.output.resolve()}")
        return 0
    if args.command == "run":
        command_run(
            manifest,
            args.stratadiff,
            args.output,
            args.workdir,
            args.require_clean,
        )
        return 0
    if args.command == "verify":
        evaluation = load_json(args.evaluation)
        verify_evaluation(manifest, evaluation)
        print(f"review-delta-v1 evaluation verified: {len(evaluation['cases'])} cases")
        return 0
    parser.error(f"unsupported command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
