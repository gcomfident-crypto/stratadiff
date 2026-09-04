#!/usr/bin/env python3

import argparse
import base64
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import urllib.request


ROOT = Path(__file__).resolve().parent
MANIFEST_PATH = ROOT / "manifest.json"
ORACLE_PATH = ROOT / "oracle.json"
CHECKSUM_PATH = ROOT / "SHA256SUMS"
MANIFEST_SCHEMA = "stratadiff-resumebench-real-manifest-v1"
ORACLE_SCHEMA = "stratadiff-resumebench-real-oracle-v1"
BUILD_INFO_SCHEMA = "stratadiff-build-info-v1"
REPORT_MATCH_BASIS = "exact_git_change_identity_or_noninteracting_four_way_byte_replay"
MAX_REPLAY_SOURCE_BYTES = 16 * 1024 * 1024
MAX_REPLAY_EDITS = 250_000
STATUS_NAMES = {
    "added": "A",
    "deleted": "D",
    "modified": "M",
    "renamed": "R",
    "type_changed": "T",
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def isolated_environment():
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
    return environment


def run_git(repository, arguments, *, check=True):
    return subprocess.run(
        ["git", "--no-replace-objects", "-C", str(repository), *arguments],
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=check,
    )


def git_bytes(repository, arguments):
    return run_git(repository, arguments).stdout


def run_external_git(arguments):
    subprocess.run(["git", *arguments], env=isolated_environment(), check=True)


def verify_bundle_checksums():
    required = {
        "README.md",
        "evaluation-v1.0.0.json",
        "manifest.json",
        "oracle.json",
        "verify.py",
    }
    observed = set()
    for line in CHECKSUM_PATH.read_text(encoding="utf-8").splitlines():
        digest, relative = line.split("  ", 1)
        require(relative in required, f"unexpected checksum target: {relative}")
        require(relative not in observed, f"duplicate checksum target: {relative}")
        require(len(digest) == 64, f"invalid SHA-256 for {relative}")
        require(
            sha256_bytes((ROOT / relative).read_bytes()) == digest,
            f"bundle checksum mismatch: {relative}",
        )
        observed.add(relative)
    require(observed == required, f"checksum coverage mismatch: {sorted(required - observed)}")


def load_bundle():
    verify_bundle_checksums()
    manifest = load_json(MANIFEST_PATH)
    oracle = load_json(ORACLE_PATH)
    require(manifest["schema"] == MANIFEST_SCHEMA, "unsupported manifest schema")
    require(oracle["schema"] == ORACLE_SCHEMA, "unsupported oracle schema")
    require(
        manifest["dataset_version"] == oracle["dataset_version"],
        "manifest and oracle versions differ",
    )
    require(manifest["case"]["id"] == oracle["case_id"], "manifest and oracle case IDs differ")
    for label in ("A", "B", "C", "D"):
        commit = manifest["case"]["snapshots"][label]["commit"]
        require(commit == oracle["snapshots"][label], f"snapshot {label} differs across bundle")
        validate_oid(commit, f"snapshot {label}")
    require(
        manifest["case"]["expected_summary"] == oracle["summary"],
        "manifest and oracle summaries differ",
    )
    return manifest, oracle


def validate_oid(value, label):
    require(
        len(value) == 40 and all(character in "0123456789abcdef" for character in value),
        f"{label} is not a full lowercase SHA-1: {value}",
    )


def optional_oid(value):
    return None if set(value) == {"0"} else value


def optional_mode(value):
    return None if value == "000000" else value


def parse_raw_diff(raw):
    fields = raw.split(b"\0")
    require(fields[-1] == b"", "raw diff is not NUL terminated")
    fields.pop()
    changes = []
    index = 0
    while index < len(fields):
        columns = fields[index].decode("ascii").split()
        index += 1
        require(len(columns) == 5 and columns[0].startswith(":"), "invalid raw diff header")
        status = columns[4]
        require(status in ("A", "D", "M", "T"), f"unsupported raw status: {status}")
        require(index < len(fields) and fields[index], "raw diff record is missing a path")
        path = fields[index]
        index += 1
        encoded_path = base64.b64encode(path).decode("ascii")
        changes.append(
            {
                "status": status,
                "similarity_percent": None,
                "before_path_base64": None if status == "A" else encoded_path,
                "after_path_base64": None if status == "D" else encoded_path,
                "before_mode": optional_mode(columns[0][1:]),
                "after_mode": optional_mode(columns[1]),
                "before_object_id": optional_oid(columns[2]),
                "after_object_id": optional_oid(columns[3]),
            }
        )
    return changes


def diff_identities(repository, left, right):
    raw = git_bytes(
        repository,
        [
            "diff-tree",
            "-r",
            "--raw",
            "-z",
            "--no-commit-id",
            "--no-abbrev",
            "--no-renames",
            left,
            right,
            "--",
        ],
    )
    return raw, parse_raw_diff(raw)


def identity_path(identity):
    before = identity["before_path_base64"]
    after = identity["after_path_base64"]
    if before is not None and after is not None:
        require(before == after, "v1 expects same-path non-rename identities")
    path = after if after is not None else before
    require(path is not None, "identity has no path")
    return path


def identities_by_path(identities):
    output = {}
    for identity in identities:
        path = identity_path(identity)
        require(path not in output, f"duplicate identity path: {path}")
        output[path] = identity
    return output


def resolve_commit(repository, revision):
    value = git_bytes(repository, ["rev-parse", "--verify", f"{revision}^{{commit}}"]).decode(
        "ascii"
    ).strip()
    validate_oid(value, f"resolved commit {revision}")
    return value


def commit_parents(repository, commit):
    values = git_bytes(repository, ["rev-list", "--parents", "-n", "1", commit]).decode(
        "ascii"
    ).split()
    require(values and values[0] == commit, f"could not inspect parents for {commit}")
    for parent in values[1:]:
        validate_oid(parent, f"parent of {commit}")
    return values[1:]


def unique_merge_base(repository, left, right):
    values = git_bytes(repository, ["merge-base", "--all", left, right]).decode("ascii").split()
    require(len(values) == 1, f"expected one merge base for {left} and {right}")
    validate_oid(values[0], "merge base")
    return values[0]


def verify_source_license(repository, manifest, commits):
    license_metadata = manifest["source_repository"]["license"]
    require(license_metadata["spdx_expression"] == "Apache-2.0", "source license changed")
    for commit in commits:
        specifier = f"{commit}:{license_metadata['path']}"
        object_id = git_bytes(repository, ["rev-parse", specifier]).decode("ascii").strip()
        require(object_id == license_metadata["blob_oid"], f"license blob differs at {commit}")
        content = git_bytes(repository, ["show", specifier])
        require(
            sha256_bytes(content) == license_metadata["content_sha256"],
            f"license content differs at {commit}",
        )


def decode_edit(edit):
    replacement = base64.b64decode(edit["replacement_base64"], validate=True)
    require(sha256_bytes(replacement) == edit["replacement_sha256"], "replacement digest mismatch")
    return {
        "start": edit["start"],
        "end": edit["end"],
        "old_sha256": edit["old_sha256"],
        "replacement": replacement,
    }


def validate_edits(source, edits):
    require(len(edits) <= MAX_REPLAY_EDITS, "replay edit count exceeds the engine limit")
    previous_end = 0
    for index, edit in enumerate(edits):
        start = edit["start"]
        end = edit["end"]
        require(isinstance(start, int) and isinstance(end, int), "edit offsets must be integers")
        require(0 <= start <= end <= len(source), "edit range is outside its source")
        require(index == 0 or previous_end <= start, "edits overlap or are out of order")
        require(
            sha256_bytes(source[start:end]) == edit["old_sha256"],
            "edit old-byte digest mismatch",
        )
        previous_end = end


def apply_edits(source, edits):
    ordered = sorted(edits, key=lambda edit: (edit["start"], edit["end"]))
    validate_edits(source, ordered)
    output = bytearray()
    position = 0
    for edit in ordered:
        output.extend(source[position : edit["start"]])
        output.extend(edit["replacement"])
        position = edit["end"]
    output.extend(source[position:])
    return bytes(output)


def strictly_separated(left, right):
    return left["end"] < right["start"] or right["end"] < left["start"]


def shifted_edits(edits, already_applied):
    shifted = []
    for edit in edits:
        offset = 0
        for other in already_applied:
            require(strictly_separated(edit, other), "cross-patch edits overlap or are adjacent")
            if other["end"] < edit["start"]:
                offset += len(other["replacement"]) - (other["end"] - other["start"])
        shifted.append(
            {
                "start": edit["start"] + offset,
                "end": edit["end"] + offset,
                "old_sha256": edit["old_sha256"],
                "replacement": edit["replacement"],
            }
        )
    return shifted


def verify_replay_bytes(a_bytes, b_bytes, c_bytes, d_bytes, checkpoint_edits, upstream_edits):
    for checkpoint_edit in checkpoint_edits:
        for upstream_edit in upstream_edits:
            require(
                strictly_separated(checkpoint_edit, upstream_edit),
                "checkpoint and upstream edits overlap or are adjacent",
            )
    require(apply_edits(a_bytes, checkpoint_edits) == b_bytes, "A->B witness does not replay")
    require(apply_edits(a_bytes, upstream_edits) == c_bytes, "A->C witness does not replay")
    require(
        apply_edits(c_bytes, shifted_edits(checkpoint_edits, upstream_edits)) == d_bytes,
        "A->C then checkpoint replay does not produce D",
    )
    require(
        apply_edits(b_bytes, shifted_edits(upstream_edits, checkpoint_edits)) == d_bytes,
        "A->B then upstream replay does not produce D",
    )
    require(
        apply_edits(a_bytes, [*checkpoint_edits, *upstream_edits]) == d_bytes,
        "combined base-coordinate witness does not produce D",
    )


def verify_replay_witness(repository, oracle):
    witness = oracle["replay_witness"]
    path = base64.b64decode(witness["path_base64"], validate=True).decode("utf-8")
    require(path == witness["path_utf8"], "replay witness path encoding mismatch")
    snapshots = {}
    for label in ("A", "B", "C", "D"):
        commit = oracle["snapshots"][label]
        evidence = witness["snapshots"][label]
        object_id = git_bytes(repository, ["rev-parse", f"{commit}:{path}"]).decode("ascii").strip()
        require(object_id == evidence["blob_oid"], f"snapshot {label} blob differs")
        content = git_bytes(repository, ["cat-file", "blob", object_id])
        require(len(content) == evidence["byte_len"], f"snapshot {label} byte length differs")
        require(
            len(content) <= MAX_REPLAY_SOURCE_BYTES,
            f"snapshot {label} exceeds the engine replay limit",
        )
        require(
            sha256_bytes(content) == evidence["content_sha256"],
            f"snapshot {label} content digest differs",
        )
        require(b"\0" not in content, f"snapshot {label} contains NUL")
        snapshots[label] = content
    checkpoint_edits = [decode_edit(edit) for edit in witness["checkpoint_edits_A_to_B"]]
    upstream_edits = [decode_edit(edit) for edit in witness["upstream_edits_A_to_C"]]
    verify_replay_bytes(
        snapshots["A"],
        snapshots["B"],
        snapshots["C"],
        snapshots["D"],
        checkpoint_edits,
        upstream_edits,
    )


def canonical_identity(identity):
    return json.dumps(identity, sort_keys=True, separators=(",", ":"))


def verify_oracle(repository):
    manifest, oracle = load_bundle()
    repository = Path(repository).resolve()
    case = manifest["case"]
    snapshots = oracle["snapshots"]
    for label in ("A", "B", "C", "D"):
        require(
            resolve_commit(repository, snapshots[label]) == snapshots[label],
            f"snapshot {label} did not resolve exactly",
        )
    require(commit_parents(repository, snapshots["B"]) == [snapshots["A"]], "B parent is not A")
    require(commit_parents(repository, snapshots["D"]) == [snapshots["C"]], "D parent is not C")
    require(
        unique_merge_base(repository, snapshots["A"], snapshots["C"]) == snapshots["A"],
        "A is not the unique ancestor merge base of C",
    )
    require(
        unique_merge_base(repository, snapshots["B"], snapshots["C"]) == snapshots["A"],
        "checkpoint and current base do not have A as their unique merge base",
    )
    verify_source_license(
        repository,
        manifest,
        [snapshots[label] for label in ("A", "B", "C", "D")],
    )

    checkpoint_raw, checkpoint = diff_identities(repository, snapshots["A"], snapshots["B"])
    current_raw, current = diff_identities(repository, snapshots["C"], snapshots["D"])
    require(
        sha256_bytes(checkpoint_raw) == oracle["raw_diff_sha256"]["checkpoint_A_to_B"],
        "checkpoint raw diff digest changed",
    )
    require(
        sha256_bytes(current_raw) == oracle["raw_diff_sha256"]["current_C_to_D"],
        "current raw diff digest changed",
    )
    require(checkpoint == oracle["checkpoint_identities"], "checkpoint identities changed")
    require(current == oracle["current_identities"], "current identities changed")

    checkpoint_by_path = identities_by_path(checkpoint)
    current_by_path = identities_by_path(current)
    classifications = identities_by_classification_path(oracle["classification"])
    require(
        set(current_by_path) == set(classifications),
        "classification does not cover current delta",
    )
    require(
        set(checkpoint_by_path) == set(classifications),
        "checkpoint and current path sets differ",
    )

    exact_paths = set()
    replay_paths = set()
    needs_paths = set()
    for path, classification in classifications.items():
        decoded = base64.b64decode(path, validate=True).decode("utf-8")
        require(decoded == classification["path_utf8"], "classification path encoding mismatch")
        state = classification["checkpoint_state"]
        if state == "needs_review_now":
            require("checkpoint_match_basis" not in classification, "needs-review file has a basis")
            needs_paths.add(path)
        else:
            require(state == "unchanged_since_checkpoint", f"unsupported checkpoint state: {state}")
            basis = classification["checkpoint_match_basis"]
            if basis == "exact_git_change_identity":
                require(
                    checkpoint_by_path[path] == current_by_path[path],
                    f"exact carry identity changed: {decoded}",
                )
                exact_paths.add(path)
            elif basis == "exact_noninteracting_four_way_byte_replay":
                require(
                    checkpoint_by_path[path] != current_by_path[path],
                    f"replay case unexpectedly has an exact identity: {decoded}",
                )
                replay_paths.add(path)
            else:
                raise ValueError(f"unsupported checkpoint match basis: {basis}")

    provider_paths = {
        base64.b64encode(path.encode("utf-8")).decode("ascii")
        for path in case["provenance"]["submission_event"]["changed_since_approval_paths"]
    }
    require(needs_paths == provider_paths, "needs-review paths differ from Gerrit evidence")
    for path in needs_paths:
        require(
            checkpoint_by_path[path] != current_by_path[path],
            "provider-labeled needs-review path retained its exact identity",
        )

    replay_path = oracle["replay_witness"]["path_base64"]
    regular_mode = oracle["replay_witness"]["regular_mode"]
    require(regular_mode in ("100644", "100755"), "replay witness mode is not regular")
    for identity in (checkpoint_by_path[replay_path], current_by_path[replay_path]):
        require(identity["status"] == "M", "replay witness is not a modification")
        require(identity["before_mode"] == regular_mode, "replay before mode differs")
        require(identity["after_mode"] == regular_mode, "replay after mode differs")
    verify_replay_witness(repository, oracle)
    require(
        replay_paths == {oracle["replay_witness"]["path_base64"]},
        "replay classification and witness differ",
    )
    carried_paths = exact_paths | replay_paths
    retired = [checkpoint_by_path[path] for path in sorted(set(checkpoint_by_path) - carried_paths)]
    expected_retired = sorted(oracle["retired_checkpoint_identities"], key=canonical_identity)
    require(
        sorted(retired, key=canonical_identity) == expected_retired,
        "retired identities changed",
    )

    summary = {
        "current_pr_files": len(current),
        "carried": len(carried_paths),
        "exactly_carried": len(exact_paths),
        "replay_carried": len(replay_paths),
        "needs_review_now": len(needs_paths),
        "retired_checkpoint_changes": len(retired),
    }
    require(summary == oracle["summary"], "recomputed summary differs from oracle")
    return {
        "case_id": oracle["case_id"],
        "summary": summary,
        "classification": oracle["classification"],
    }


def identities_by_classification_path(classifications):
    output = {}
    for classification in classifications:
        path = classification["path_base64"]
        require(path not in output, f"duplicate classification path: {path}")
        output[path] = classification
    return output


def decode_product_path(file, side):
    path_field = f"{side}_path"
    encoding_field = f"{side}_path_encoding"
    if path_field not in file:
        return None
    display = file[path_field]
    encoding = file[encoding_field]
    if encoding == "utf8":
        return display.encode("utf-8")
    require(
        encoding == "git_bytes_percent_encoded" and display.startswith("git-bytes:"),
        f"unsupported product path encoding: {encoding}",
    )
    encoded = display[len("git-bytes:") :]
    require(len(encoded) % 3 == 0, "malformed product percent encoding")
    chunks = [encoded[index : index + 3] for index in range(0, len(encoded), 3)]
    require(all(chunk[0] == "%" for chunk in chunks), "malformed product percent encoding")
    return bytes(int(chunk[1:], 16) for chunk in chunks)


def product_optional(file, field):
    return file[field] if field in file else None


def product_identity(file):
    before_path = decode_product_path(file, "before")
    after_path = decode_product_path(file, "after")
    return {
        "status": STATUS_NAMES[file["status"]],
        "similarity_percent": product_optional(file, "similarity_percent"),
        "before_path_base64": None
        if before_path is None
        else base64.b64encode(before_path).decode("ascii"),
        "after_path_base64": None
        if after_path is None
        else base64.b64encode(after_path).decode("ascii"),
        "before_mode": product_optional(file, "before_mode"),
        "after_mode": product_optional(file, "after_mode"),
        "before_object_id": product_optional(file, "before_blob"),
        "after_object_id": product_optional(file, "after_blob"),
    }


def read_build_info(binary):
    result = subprocess.run(
        [str(binary), "build-info"],
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    require(not result.stderr, "stratadiff build-info produced diagnostics")
    info = json.loads(result.stdout)
    require(info["schema"] == BUILD_INFO_SCHEMA, "unsupported build-info schema")
    validate_oid(info["git_revision"], "StrataDiff build revision")
    require(info["git_dirty"] is False, "benchmark requires a clean StrataDiff build")
    require(info["build_profile"] == "release", "benchmark requires a release build")
    require(info["rustc_version"].startswith("rustc "), "invalid rustc provenance")
    require(len(info["cargo_lock_sha256"]) == 64, "invalid Cargo.lock provenance")
    return info


def evaluate(repository, binary):
    oracle_result = verify_oracle(repository)
    manifest, oracle = load_bundle()
    snapshots = oracle["snapshots"]
    binary = Path(binary).resolve()
    build_info = read_build_info(binary)
    result = subprocess.run(
        [
            str(binary),
            "review",
            "--repo",
            str(Path(repository).resolve()),
            "--checkpoint",
            snapshots["B"],
            "--format",
            "json",
            "--",
            snapshots["C"],
            snapshots["D"],
        ],
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    require(not result.stderr, "stratadiff review produced diagnostics")
    report = json.loads(result.stdout)
    require(report["requested_base"] == snapshots["C"], "product requested base differs")
    require(report["base_commit"] == snapshots["C"], "product comparison base differs")
    require(report["requested_head"] == snapshots["D"], "product requested head differs")
    require(report["head_commit"] == snapshots["D"], "product head differs")
    require(report["checkpoint"]["commit"] == snapshots["B"], "product checkpoint differs")
    require(
        report["checkpoint"]["base_commit"] == snapshots["A"],
        "product checkpoint base differs",
    )
    require(
        report["checkpoint"]["match_basis"] == REPORT_MATCH_BASIS,
        "product global match basis differs",
    )

    current_by_path = identities_by_path(oracle["current_identities"])
    classifications = identities_by_classification_path(oracle["classification"])
    observed = {}
    for file in report["files"]:
        identity = product_identity(file)
        path = identity_path(identity)
        require(path not in observed, f"product emitted duplicate path: {path}")
        require(path in classifications, f"product emitted unexpected path: {path}")
        require(identity == current_by_path[path], f"product identity differs: {path}")
        expected = classifications[path]
        require(
            file["checkpoint_state"] == expected["checkpoint_state"],
            f"product checkpoint state differs: {path}",
        )
        if "checkpoint_match_basis" in expected:
            require(
                file["checkpoint_match_basis"] == expected["checkpoint_match_basis"],
                f"product checkpoint match basis differs: {path}",
            )
        else:
            require(
                "checkpoint_match_basis" not in file,
                f"needs-review product file unexpectedly has a basis: {path}",
            )
        observed[path] = {
            "path_utf8": expected["path_utf8"],
            "checkpoint_state": file["checkpoint_state"],
            "checkpoint_match_basis": file["checkpoint_match_basis"]
            if "checkpoint_match_basis" in file
            else None,
        }
    require(set(observed) == set(classifications), "product omitted expected paths")

    checkpoint_summary = report["summary"]["checkpoint"]
    require(
        report["summary"]["changed_files"] == oracle["summary"]["current_pr_files"],
        "product current file count differs",
    )
    require(
        checkpoint_summary["unchanged_since_checkpoint_files"] == oracle["summary"]["carried"],
        "product carried count differs",
    )
    require(
        checkpoint_summary["needs_review_now_files"] == oracle["summary"]["needs_review_now"],
        "product needs-review count differs",
    )
    require(
        checkpoint_summary["retired_change_count"]
        == oracle["summary"]["retired_checkpoint_changes"],
        "product retired count differs",
    )
    return {
        "schema": "stratadiff-resumebench-real-evaluation-v1",
        "dataset_version": manifest["dataset_version"],
        "evaluated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "benchmark_complete": True,
        "claim_boundary": (
            "One provider-labeled Gerrit rebase history; not a reviewer-time, defect-recall, "
            "semantic-safety, or prevalence estimate."
        ),
        "summary": oracle_result["summary"],
        "case": {
            "id": oracle["case_id"],
            "passed": True,
            "files": [observed[path] for path in sorted(observed)],
        },
        "provenance": {
            "manifest_sha256": sha256_bytes(MANIFEST_PATH.read_bytes()),
            "oracle_sha256": sha256_bytes(ORACLE_PATH.read_bytes()),
            "verifier_sha256": sha256_bytes(Path(__file__).read_bytes()),
            "stratadiff_binary_sha256": sha256_bytes(binary.read_bytes()),
            "engine": build_info,
        },
    }


def read_gerrit_json(url):
    request = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        require(response.status == 200, f"Gerrit returned HTTP {response.status}: {url}")
        payload = response.read()
    prefix = b")]}'\n"
    require(payload.startswith(prefix), f"Gerrit response lacks XSSI prefix: {url}")
    return json.loads(payload[len(prefix) :])


def verify_message(change_number, evidence, messages):
    expected_url = (
        f"https://gerrit-review.googlesource.com/changes/{change_number}/messages/"
        f"{evidence['message_id']}"
    )
    require(evidence["message_url"] == expected_url, "message URL is not bound to the change")
    matches = [message for message in messages if message["id"] == evidence["message_id"]]
    require(len(matches) == 1, f"message ID is not unique: {evidence['message_id']}")
    message = matches[0]
    if "patch_set" in evidence:
        require(message["_revision_number"] == evidence["patch_set"], "message patch set differs")
    require(message["date"] == evidence["recorded_at"], "message timestamp differs")
    for fragment in evidence["message_contains"]:
        require(fragment in message["message"], f"Gerrit message lacks fragment: {fragment}")
    return message


def verify_provenance():
    manifest, _ = load_bundle()
    case = manifest["case"]
    provenance = case["provenance"]
    change_number = case["change_number"]
    detail = read_gerrit_json(provenance["detail_url"])
    messages = read_gerrit_json(provenance["messages_url"])
    require(detail["_number"] == change_number, "Gerrit change number differs")
    require(detail["project"] == manifest["source_repository"]["id"], "Gerrit project differs")
    require(detail["change_id"] == case["change_id"], "Gerrit Change-Id differs")
    require(detail["subject"] == case["subject"], "Gerrit subject differs")
    require(detail["status"] == "MERGED", "Gerrit change is not merged")
    require(
        detail["current_revision"] == case["snapshots"]["D"]["commit"],
        "Gerrit submitted revision differs",
    )
    for label in ("B", "D"):
        snapshot = case["snapshots"][label]
        revision = detail["revisions"][snapshot["commit"]]
        require(revision["_number"] == snapshot["patch_set"], f"snapshot {label} patch set differs")
        require(revision["ref"] == snapshot["ref"], f"snapshot {label} ref differs")
        parents = revision["commit"]["parents"]
        require(
            len(parents) == 1 and parents[0]["commit"] == snapshot["parent"],
            f"snapshot {label} parent differs",
        )
    verify_message(change_number, provenance["checkpoint_event"], messages)
    verify_message(change_number, provenance["rebase_event"], messages)
    submission_message = verify_message(change_number, provenance["submission_event"], messages)
    file_prefix = "The name of the file: "
    observed_paths = sorted(
        line[len(file_prefix) :]
        for line in submission_message["message"].splitlines()
        if line.startswith(file_prefix)
    )
    expected_paths = sorted(provenance["submission_event"]["changed_since_approval_paths"])
    require(observed_paths == expected_paths, "Gerrit submission file list differs")
    return {
        "change_number": change_number,
        "checkpoint_message": provenance["checkpoint_event"]["message_id"],
        "rebase_message": provenance["rebase_event"]["message_id"],
        "submission_message": provenance["submission_event"]["message_id"],
        "verified": True,
    }


def local_ref(snapshot):
    return f"refs/resumebench/{snapshot['patch_set']}"


def write_json(path, value):
    encoded = json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    path.write_text(encoded, encoding="utf-8")


def materialize(output):
    manifest, _ = load_bundle()
    verify_provenance()
    output = Path(output).resolve()
    require(not output.exists(), f"materialization output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=f".{output.name}.tmp-", dir=output.parent))
    repository = stage / "repository.git"
    completed = False
    try:
        run_external_git(["init", "--bare", "-q", str(repository)])
        source = manifest["source_repository"]["git_url"]
        run_external_git(["-C", str(repository), "remote", "add", "origin", source])
        snapshots = manifest["case"]["snapshots"]
        selected = [snapshots["B"], snapshots["D"]]
        refspecs = [f"{snapshot['ref']}:{local_ref(snapshot)}" for snapshot in selected]
        run_external_git(
            ["-C", str(repository), "fetch", "--no-tags", "--depth=2", "origin", *refspecs]
        )
        run_external_git(["-C", str(repository), "config", "extensions.partialClone", "origin"])
        run_external_git(["-C", str(repository), "config", "remote.origin.promisor", "true"])
        run_external_git(
            ["-C", str(repository), "config", "remote.origin.partialclonefilter", "tree:0"]
        )
        run_external_git(
            [
                "-C",
                str(repository),
                "fetch",
                "--unshallow",
                "--filter=tree:0",
                "--no-tags",
                "origin",
                *refspecs,
            ]
        )
        run_external_git(
            [
                "-C",
                str(repository),
                "update-ref",
                "refs/heads/resumebench-real-v1",
                snapshots["D"]["commit"],
            ]
        )
        run_external_git(
            [
                "-C",
                str(repository),
                "symbolic-ref",
                "HEAD",
                "refs/heads/resumebench-real-v1",
            ]
        )
        require(
            git_bytes(repository, ["rev-parse", "--is-shallow-repository"]).decode("ascii").strip()
            == "false",
            "materialized repository remained shallow",
        )
        verify_oracle(repository)
        run_git(repository, ["fsck", "--full", "--no-dangling"])
        write_json(
            stage / "materialization.json",
            {
                "schema": "stratadiff-resumebench-real-materialization-v1",
                "dataset_version": manifest["dataset_version"],
                "manifest_sha256": sha256_bytes(MANIFEST_PATH.read_bytes()),
                "oracle_sha256": sha256_bytes(ORACLE_PATH.read_bytes()),
                "repository": "repository.git",
            },
        )
        stage.replace(output)
        completed = True
    finally:
        if not completed and stage.exists():
            shutil.rmtree(stage)
    return output / "repository.git"


def make_test_edit(source, start, end, replacement):
    return {
        "start": start,
        "end": end,
        "old_sha256": sha256_bytes(source[start:end]),
        "replacement": replacement,
    }


def expect_value_error(action):
    failed = False
    try:
        action()
    except ValueError:
        failed = True
    require(failed, "self-test expected ValueError")


def self_test():
    a_bytes = b"abcdef"
    checkpoint = [make_test_edit(a_bytes, 1, 1, b"XX")]
    upstream = [make_test_edit(a_bytes, 4, 5, b"")]
    b_bytes = apply_edits(a_bytes, checkpoint)
    c_bytes = apply_edits(a_bytes, upstream)
    d_bytes = apply_edits(a_bytes, [*checkpoint, *upstream])
    verify_replay_bytes(a_bytes, b_bytes, c_bytes, d_bytes, checkpoint, upstream)

    adjacent = [make_test_edit(a_bytes, 1, 2, b"Q")]
    touching = [make_test_edit(a_bytes, 2, 3, b"R")]
    expect_value_error(
        lambda: verify_replay_bytes(
            a_bytes,
            apply_edits(a_bytes, adjacent),
            apply_edits(a_bytes, touching),
            a_bytes,
            adjacent,
            touching,
        )
    )
    expect_value_error(
        lambda: verify_replay_bytes(a_bytes, b_bytes, c_bytes, b"tampered", checkpoint, upstream)
    )
    require(
        decode_product_path(
            {
                "before_path": "git-bytes:%66%6F%6F",
                "before_path_encoding": "git_bytes_percent_encoded",
            },
            "before",
        )
        == b"foo",
        "percent-decoding self-test failed",
    )
    return {"tests": 4, "passed": 4}


def parse_arguments():
    parser = argparse.ArgumentParser(description="Verify ResumeBench-Real v1")
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("self-test")
    subparsers.add_parser("verify-provenance")

    materialize_parser = subparsers.add_parser("materialize")
    materialize_parser.add_argument("--output", type=Path, required=True)

    oracle_parser = subparsers.add_parser("verify-oracle")
    oracle_parser.add_argument("--repository", type=Path, required=True)

    evaluate_parser = subparsers.add_parser("evaluate")
    evaluate_parser.add_argument("--repository", type=Path, required=True)
    evaluate_parser.add_argument("--stratadiff", type=Path, required=True)
    evaluate_parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main():
    arguments = parse_arguments()
    if arguments.command == "self-test":
        value = self_test()
    elif arguments.command == "verify-provenance":
        value = verify_provenance()
    elif arguments.command == "materialize":
        value = {"repository": str(materialize(arguments.output))}
    elif arguments.command == "verify-oracle":
        value = verify_oracle(arguments.repository)
    elif arguments.command == "evaluate":
        value = evaluate(arguments.repository, arguments.stratadiff)
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        write_json(arguments.output, value)
        value = {"benchmark_complete": value["benchmark_complete"], "output": str(arguments.output)}
    else:
        raise ValueError(f"unsupported command: {arguments.command}")
    print(json.dumps(value, ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main()
