#!/usr/bin/env python3

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from datetime import datetime, timezone


MANIFEST_SCHEMA = "stratadiff-resumebench-real-manifest-v0"
ORACLE_SCHEMA = "stratadiff-resumebench-real-oracle-v0"
ZERO_OID = "0" * 40
STATUS_NAMES = {
    "added": "A",
    "deleted": "D",
    "modified": "M",
    "renamed": "R",
    "type_changed": "T",
}


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


def run_external_git(arguments):
    subprocess.run(["git", *arguments], env=isolated_environment(), check=True)


def git_stdout(repository, arguments):
    result = run_git(repository, arguments)
    allowed_warning = b"warning: lazy fetching disabled; some objects may not be available\n"
    if result.stderr not in (b"", allowed_warning):
        raise RuntimeError(
            f"git {' '.join(arguments)} produced diagnostics: "
            f"{result.stderr.decode('utf-8', errors='replace').strip()}"
        )
    return result.stdout


def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    with tempfile.NamedTemporaryFile(
        mode="w",
        encoding="utf-8",
        dir=path.parent,
        prefix=f".{path.name}.",
        suffix=".tmp",
        delete=False,
    ) as temporary:
        temporary.write(encoded)
        temporary.flush()
        os.fsync(temporary.fileno())
    Path(temporary.name).replace(path)


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def validate_oid(value, label):
    if len(value) != 40 or any(character not in "0123456789abcdef" for character in value):
        raise ValueError(f"{label} is not a full lowercase SHA-1 object ID: {value}")


def load_manifest(path):
    manifest = load_json(path)
    if manifest["schema"] != MANIFEST_SCHEMA:
        raise ValueError(f"unsupported manifest schema: {manifest['schema']}")
    if not manifest["cases"]:
        raise ValueError("manifest has no cases")
    ids = set()
    for case in manifest["cases"]:
        if case["id"] in ids:
            raise ValueError(f"duplicate case ID: {case['id']}")
        ids.add(case["id"])
        revisions = case["revisions"]
        validate_oid(revisions["requested_base_commit"], f"{case['id']} requested base")
        for side in ("checkpoint", "current"):
            validate_oid(revisions[side]["commit"], f"{case['id']} {side} commit")
            validate_oid(revisions[side]["parent"], f"{case['id']} {side} parent")
            validate_gerrit_ref(
                revisions[side]["ref"],
                case["change_number"],
                revisions[side]["patch_set"],
            )
        if case["checkpoint_evidence"]["patch_set"] != revisions["checkpoint"]["patch_set"]:
            raise ValueError(f"{case['id']} checkpoint evidence targets a different patch set")
    return manifest


def validate_gerrit_ref(reference, change_number, patch_set):
    parts = reference.split("/")
    expected_suffix = f"{change_number % 100:02d}"
    if (
        len(parts) != 5
        or parts[0:2] != ["refs", "changes"]
        or parts[2] != expected_suffix
        or parts[3] != str(change_number)
        or parts[4] != str(patch_set)
    ):
        raise ValueError(
            f"Gerrit ref {reference} does not encode change {change_number} patch set {patch_set}"
        )


def optional(value):
    return None if value == ZERO_OID or set(value) == {"0"} else value


def raw_change(status, before_path, after_path, before_mode, after_mode, before_oid, after_oid):
    return {
        "status": status,
        "similarity_percent": None,
        "before_path": before_path,
        "after_path": after_path,
        "before_mode": optional(before_mode),
        "after_mode": optional(after_mode),
        "before_object_id": optional(before_oid),
        "after_object_id": optional(after_oid),
    }


def parse_raw_diff(raw):
    fields = raw.split(b"\0")
    if fields[-1] != b"":
        raise ValueError("raw Git diff is not NUL terminated")
    fields.pop()
    changes = []
    index = 0
    while index < len(fields):
        columns = fields[index].decode("ascii").split()
        index += 1
        if len(columns) != 5 or not columns[0].startswith(":"):
            raise ValueError(f"unexpected raw Git header: {columns}")
        status = columns[4]
        if status not in ("A", "D", "M", "T"):
            raise ValueError(f"--no-renames emitted unsupported status: {status}")
        if index >= len(fields) or not fields[index]:
            raise ValueError("raw Git record is missing its path")
        path = fields[index]
        index += 1
        before_path = None if status == "A" else path
        after_path = None if status == "D" else path
        changes.append(
            raw_change(
                status,
                before_path,
                after_path,
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

    pairs = []
    ambiguous = []
    for key, indexes in candidates.items():
        deleted = indexes["deleted"]
        added = indexes["added"]
        if len(deleted) == 1 and len(added) == 1:
            pairs.append((deleted[0], added[0]))
        elif deleted and added:
            ambiguous.append(
                {
                    "object_id": key[0],
                    "mode": key[1],
                    "deletion_count": len(deleted),
                    "addition_count": len(added),
                }
            )
    if ambiguous:
        raise ValueError(f"ambiguous exact relocation candidates: {ambiguous}")

    normalized = list(changes)
    removed = set()
    replacements = {}
    for deleted_index, added_index in pairs:
        deleted = changes[deleted_index]
        added = changes[added_index]
        replacement_index = min(deleted_index, added_index)
        replacements[replacement_index] = {
            "status": "R",
            "similarity_percent": 100,
            "before_path": deleted["before_path"],
            "after_path": added["after_path"],
            "before_mode": deleted["before_mode"],
            "after_mode": added["after_mode"],
            "before_object_id": deleted["before_object_id"],
            "after_object_id": added["after_object_id"],
        }
        removed.add(deleted_index)
        removed.add(added_index)

    output = []
    for index, change in enumerate(normalized):
        if index in replacements:
            output.append(replacements[index])
        elif index not in removed:
            output.append(change)
    return output


def path_base64(path):
    return None if path is None else base64.b64encode(path).decode("ascii")


def identity(change):
    payload = {
        "status": change["status"],
        "similarity_percent": change["similarity_percent"],
        "before_path_base64": path_base64(change["before_path"]),
        "after_path_base64": path_base64(change["after_path"]),
        "before_mode": change["before_mode"],
        "after_mode": change["after_mode"],
        "before_object_id": change["before_object_id"],
        "after_object_id": change["after_object_id"],
    }
    canonical = json.dumps(payload, ensure_ascii=True, sort_keys=True, separators=(",", ":"))
    return {"identity_sha256": sha256_bytes(canonical.encode("ascii")), **payload}


def diff_identities(repository, left, right):
    raw = git_stdout(
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
    changes = normalize_exact_relocations(parse_raw_diff(raw))
    identities = sorted((identity(change) for change in changes), key=lambda item: item["identity_sha256"])
    if len({item["identity_sha256"] for item in identities}) != len(identities):
        raise ValueError(f"duplicate exact identities in {left}..{right}")
    return {"raw_sha256": sha256_bytes(raw), "identities": identities}


def resolve_commit(repository, revision):
    resolved = git_stdout(repository, ["rev-parse", "--verify", f"{revision}^{{commit}}"]).decode("ascii").strip()
    validate_oid(resolved, "resolved commit")
    return resolved


def commit_parent(repository, commit):
    parents = git_stdout(repository, ["rev-list", "--parents", "-n", "1", commit]).decode("ascii").split()
    if len(parents) != 2 or parents[0] != commit:
        raise ValueError(f"expected one parent for {commit}, found {len(parents) - 1}")
    validate_oid(parents[1], "commit parent")
    return parents[1]


def unique_merge_base(repository, left, right):
    merge_bases = git_stdout(repository, ["merge-base", "--all", left, right]).decode("ascii").splitlines()
    if len(merge_bases) != 1:
        raise ValueError(f"expected one merge base for {left} and {right}, found {len(merge_bases)}")
    validate_oid(merge_bases[0], "merge base")
    return merge_bases[0]


def blob_evidence(repository, identities):
    object_ids = set()
    for item in identities:
        for field in ("before_object_id", "after_object_id"):
            if item[field] is not None:
                object_ids.add(item[field])
    evidence = []
    for object_id in sorted(object_ids):
        content = git_stdout(repository, ["cat-file", "blob", object_id])
        size = int(git_stdout(repository, ["cat-file", "-s", object_id]).decode("ascii").strip())
        if len(content) != size:
            raise ValueError(f"blob size mismatch for {object_id}")
        evidence.append(
            {
                "object_id": object_id,
                "byte_len": size,
                "content_sha256": sha256_bytes(content),
            }
        )
    return evidence


def verify_source_license(manifest, repository, commits):
    license_metadata = manifest["source_repository"]["license"]
    for commit in commits:
        specifier = f"{commit}:{license_metadata['path']}"
        object_id = git_stdout(repository, ["rev-parse", specifier]).decode("ascii").strip()
        if object_id != license_metadata["blob_oid"]:
            raise ValueError(f"source license blob changed at {commit}: {object_id}")
        content = git_stdout(repository, ["show", specifier])
        if sha256_bytes(content) != license_metadata["content_sha256"]:
            raise ValueError(f"source license digest changed at {commit}")


def generate_case_oracle(manifest, case, repository):
    revisions = case["revisions"]
    requested_base = resolve_commit(repository, revisions["requested_base_commit"])
    checkpoint = resolve_commit(repository, revisions["checkpoint"]["commit"])
    current = resolve_commit(repository, revisions["current"]["commit"])
    if commit_parent(repository, checkpoint) != revisions["checkpoint"]["parent"]:
        raise ValueError(f"{case['id']} checkpoint parent does not match the manifest")
    if commit_parent(repository, current) != revisions["current"]["parent"]:
        raise ValueError(f"{case['id']} current parent does not match the manifest")
    checkpoint_merge_base = unique_merge_base(repository, requested_base, checkpoint)
    current_merge_base = unique_merge_base(repository, requested_base, current)
    expectation = case["expectation"]
    common = {
        "schema": ORACLE_SCHEMA,
        "case_id": case["id"],
        "source_repository": manifest["source_repository"]["git_url"],
        "requested_base_commit": requested_base,
        "checkpoint_commit": checkpoint,
        "current_commit": current,
        "checkpoint_merge_base": checkpoint_merge_base,
        "current_merge_base": current_merge_base,
    }

    verify_source_license(
        manifest,
        repository,
        sorted(
            {
                requested_base,
                checkpoint,
                current,
                checkpoint_merge_base,
                current_merge_base,
            }
        ),
    )
    if expectation["kind"] == "base_mismatch_rejected":
        if current_merge_base != requested_base:
            raise ValueError(f"{case['id']} current range does not resolve to its requested base")
        if checkpoint_merge_base == current_merge_base:
            raise ValueError(f"{case['id']} no longer demonstrates base drift")
        return {
            **common,
            "expectation": "base_mismatch_rejected",
            "error_contains": expectation["error_contains"],
        }
    if expectation["kind"] != "exact_identity_partition":
        raise ValueError(f"unsupported expectation kind: {expectation['kind']}")
    if checkpoint_merge_base != requested_base or current_merge_base != requested_base:
        raise ValueError(f"{case['id']} does not share the requested merge base")

    checkpoint_diff = diff_identities(repository, requested_base, checkpoint)
    head_diff = diff_identities(repository, requested_base, current)
    resume_diff = diff_identities(repository, checkpoint, current)
    checkpoint_by_id = {item["identity_sha256"]: item for item in checkpoint_diff["identities"]}
    head_by_id = {item["identity_sha256"]: item for item in head_diff["identities"]}
    checkpoint_ids = set(checkpoint_by_id)
    head_ids = set(head_by_id)
    carried = sorted(checkpoint_ids & head_ids)
    needs = sorted(head_ids - checkpoint_ids)
    retired = sorted(checkpoint_ids - head_ids)
    expected_head = [
        {
            "identity_sha256": identity_id,
            "checkpoint_state": "unchanged_since_checkpoint" if identity_id in checkpoint_ids else "needs_review_now",
        }
        for identity_id in sorted(head_ids)
    ]
    all_identities = [
        *checkpoint_diff["identities"],
        *head_diff["identities"],
        *resume_diff["identities"],
    ]
    return {
        **common,
        "expectation": "exact_identity_partition",
        "raw_diff_sha256": {
            "checkpoint": checkpoint_diff["raw_sha256"],
            "head": head_diff["raw_sha256"],
            "resume_delta": resume_diff["raw_sha256"],
        },
        "summary": {
            "checkpoint_identities": len(checkpoint_ids),
            "current_identities": len(head_ids),
            "resume_delta_identities": len(resume_diff["identities"]),
            "unchanged_since_checkpoint": len(carried),
            "needs_review_now": len(needs),
            "retired": len(retired),
        },
        "identities": {
            "checkpoint": checkpoint_diff["identities"],
            "head": head_diff["identities"],
            "resume_delta": resume_diff["identities"],
        },
        "expected_head": expected_head,
        "retired_identity_sha256": retired,
        "blob_evidence": blob_evidence(repository, all_identities),
    }


def oracle_path(manifest_path, case):
    return manifest_path.parent / case["expectation"]["oracle"]


def generate_all(manifest_path, repository):
    manifest = load_manifest(manifest_path)
    generated = []
    for case in manifest["cases"]:
        value = generate_case_oracle(manifest, case, repository)
        path = oracle_path(manifest_path, case)
        write_json(path, value)
        generated.append(path)
    return generated


def verify_all(manifest_path, repository):
    manifest = load_manifest(manifest_path)
    for case in manifest["cases"]:
        expected = load_json(oracle_path(manifest_path, case))
        actual = generate_case_oracle(manifest, case, repository)
        if actual != expected:
            raise ValueError(f"oracle drift for {case['id']}")
    return len(manifest["cases"])


def decode_product_path(display, encoding):
    if encoding == "utf8":
        return display.encode("utf-8")
    if encoding != "git_bytes_percent_encoded" or not display.startswith("git-bytes:"):
        raise ValueError(f"unsupported product path encoding: {encoding}")
    encoded = display[len("git-bytes:"):]
    if len(encoded) % 3 != 0:
        raise ValueError(f"malformed percent-encoded Git path: {display}")
    chunks = [encoded[index:index + 3] for index in range(0, len(encoded), 3)]
    if any(len(chunk) != 3 or chunk[0] != "%" for chunk in chunks):
        raise ValueError(f"malformed percent-encoded Git path: {display}")
    return bytes(int(chunk[1:], 16) for chunk in chunks)


def product_optional(file, field):
    return file[field] if field in file else None


def product_path(file, side):
    path_field = f"{side}_path"
    encoding_field = f"{side}_path_encoding"
    if path_field not in file:
        return None
    return decode_product_path(file[path_field], file[encoding_field])


def product_identity(file):
    change = {
        "status": STATUS_NAMES[file["status"]],
        "similarity_percent": product_optional(file, "similarity_percent"),
        "before_path": product_path(file, "before"),
        "after_path": product_path(file, "after"),
        "before_mode": product_optional(file, "before_mode"),
        "after_mode": product_optional(file, "after_mode"),
        "before_object_id": product_optional(file, "before_blob"),
        "after_object_id": product_optional(file, "after_blob"),
    }
    return identity(change)


def evaluate_partition_case(case, oracle, repository, binary):
    revisions = case["revisions"]
    command = [
        str(binary),
        "review",
        "--repo",
        str(repository),
        "--checkpoint",
        revisions["checkpoint"]["commit"],
        "--format",
        "json",
        "--",
        revisions["requested_base_commit"],
        revisions["current"]["commit"],
    ]
    result = subprocess.run(
        command,
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        return {
            "id": case["id"],
            "passed": False,
            "error": result.stderr.decode("utf-8", errors="replace").strip(),
        }
    report = json.loads(result.stdout)
    observed = {}
    duplicate_product_identities = []
    for file in report["files"]:
        item = product_identity(file)
        if item["identity_sha256"] in observed:
            duplicate_product_identities.append(item["identity_sha256"])
        observed[item["identity_sha256"]] = file["checkpoint_state"]
    expected = {
        item["identity_sha256"]: item["checkpoint_state"]
        for item in oracle["expected_head"]
    }
    observed_ids = set(observed)
    expected_ids = set(expected)
    false_carry = sorted(
        identity_id
        for identity_id in observed_ids & expected_ids
        if observed[identity_id] == "unchanged_since_checkpoint" and expected[identity_id] == "needs_review_now"
    )
    false_invalidation = sorted(
        identity_id
        for identity_id in observed_ids & expected_ids
        if observed[identity_id] == "needs_review_now" and expected[identity_id] == "unchanged_since_checkpoint"
    )
    state_mismatches = sorted(
        identity_id
        for identity_id in observed_ids & expected_ids
        if observed[identity_id] != expected[identity_id]
    )
    omissions = sorted(expected_ids - observed_ids)
    extras = sorted(observed_ids - expected_ids)
    product_summary = report["summary"]["checkpoint"]
    retired_mismatch = product_summary["retired_change_count"] != oracle["summary"]["retired"]
    summary_mismatch = (
        report["summary"]["changed_files"] != oracle["summary"]["current_identities"]
        or product_summary["needs_review_now_files"] != oracle["summary"]["needs_review_now"]
        or product_summary["unchanged_since_checkpoint_files"]
        != oracle["summary"]["unchanged_since_checkpoint"]
    )
    passed = (
        not state_mismatches
        and not omissions
        and not extras
        and not duplicate_product_identities
        and not retired_mismatch
        and not summary_mismatch
    )
    return {
        "id": case["id"],
        "passed": passed,
        "current_identities": oracle["summary"]["current_identities"],
        "expected_needs_review_now": oracle["summary"]["needs_review_now"],
        "expected_unchanged_since_checkpoint": oracle["summary"]["unchanged_since_checkpoint"],
        "expected_retired": oracle["summary"]["retired"],
        "false_carry": false_carry,
        "false_invalidation": false_invalidation,
        "state_mismatches": state_mismatches,
        "duplicate_product_identities": sorted(duplicate_product_identities),
        "identity_omissions": omissions,
        "identity_extras": extras,
        "retired_mismatch": retired_mismatch,
        "summary_mismatch": summary_mismatch,
        "engine_version": report["engine_version"],
    }


def evaluate_rejection_case(case, oracle, repository, binary):
    revisions = case["revisions"]
    result = subprocess.run(
        [
            str(binary),
            "review",
            "--repo",
            str(repository),
            "--checkpoint",
            revisions["checkpoint"]["commit"],
            "--format",
            "json",
            "--",
            revisions["requested_base_commit"],
            revisions["current"]["commit"],
        ],
        env=isolated_environment(),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    stderr = result.stderr.decode("utf-8", errors="replace")
    passed = result.returncode != 0 and oracle["error_contains"] in stderr and not result.stdout
    return {
        "id": case["id"],
        "passed": passed,
        "expected_rejection": "base_mismatch_rejected",
        "exit_code": result.returncode,
        "matched_error": oracle["error_contains"] in stderr,
    }


def evaluate_all(manifest_path, repository, binary, output):
    manifest = load_manifest(manifest_path)
    results = []
    current_identities = 0
    needs_review_now = 0
    unchanged_since_checkpoint = 0
    retired = 0
    false_carry = 0
    false_invalidation = 0
    for case in manifest["cases"]:
        oracle = load_json(oracle_path(manifest_path, case))
        if oracle["expectation"] == "exact_identity_partition":
            result = evaluate_partition_case(case, oracle, repository, binary)
            current_identities += oracle["summary"]["current_identities"]
            needs_review_now += oracle["summary"]["needs_review_now"]
            unchanged_since_checkpoint += oracle["summary"]["unchanged_since_checkpoint"]
            retired += oracle["summary"]["retired"]
            if "false_carry" in result:
                false_carry += len(result["false_carry"])
                false_invalidation += len(result["false_invalidation"])
        elif oracle["expectation"] == "base_mismatch_rejected":
            result = evaluate_rejection_case(case, oracle, repository, binary)
        else:
            raise ValueError(f"unsupported oracle expectation: {oracle['expectation']}")
        results.append(result)
    benchmark_complete = len(results) == len(manifest["cases"]) and all(result["passed"] for result in results)
    evaluation = {
        "schema": "stratadiff-resumebench-real-evaluation-v0",
        "dataset_version": manifest["dataset_version"],
        "evaluated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "benchmark_complete": benchmark_complete,
        "claim_boundary": "Diagnostic exact-identity correctness on five selected Gerrit histories; not a reviewer-time or population-frequency estimate.",
        "provenance": {
            "manifest_sha256": sha256_bytes(manifest_path.read_bytes()),
            "stratadiff_binary_sha256": sha256_bytes(binary.read_bytes()),
            "oracle_sha256": {
                case["id"]: sha256_bytes(oracle_path(manifest_path, case).read_bytes())
                for case in manifest["cases"]
            },
        },
        "summary": {
            "cases": len(results),
            "passed_cases": sum(1 for result in results if result["passed"]),
            "current_identities": current_identities,
            "needs_review_now": needs_review_now,
            "unchanged_since_checkpoint": unchanged_since_checkpoint,
            "retired": retired,
            "observed_focus_share": None if current_identities == 0 else needs_review_now / current_identities,
            "false_carry": false_carry,
            "false_invalidation": false_invalidation,
        },
        "cases": results,
    }
    write_json(output, evaluation)
    return benchmark_complete


def local_ref(upstream_ref):
    parts = upstream_ref.split("/")
    if len(parts) != 5 or parts[0:2] != ["refs", "changes"]:
        raise ValueError(f"unsupported Gerrit ref: {upstream_ref}")
    return f"refs/resumebench/{parts[3]}/{parts[4]}"


def read_gerrit_json(url):
    request = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        if response.status != 200:
            raise RuntimeError(f"Gerrit returned HTTP {response.status} for {url}")
        payload = response.read()
    prefix = b")]}'\n"
    if not payload.startswith(prefix):
        raise ValueError(f"Gerrit response is missing its XSSI prefix: {url}")
    return json.loads(payload[len(prefix):])


def verify_message_evidence(change_number, evidence, expected_fragments):
    expected_url = (
        f"https://gerrit-review.googlesource.com/changes/{change_number}/messages/"
        f"{evidence['message_id']}"
    )
    if evidence["message_url"] != expected_url:
        raise ValueError(f"Gerrit message URL is not bound to change {change_number}")
    message = read_gerrit_json(evidence["message_url"])
    if message["id"] != evidence["message_id"]:
        raise ValueError(f"Gerrit message ID mismatch at {evidence['message_url']}")
    if "patch_set" in evidence and message["_revision_number"] != evidence["patch_set"]:
        raise ValueError(f"Gerrit patch-set mismatch at {evidence['message_url']}")
    if "recorded_at" in evidence and message["date"] != evidence["recorded_at"]:
        raise ValueError(f"Gerrit message timestamp mismatch at {evidence['message_url']}")
    for fragment in expected_fragments:
        if fragment not in message["message"]:
            raise ValueError(f"Gerrit message is missing expected evidence at {evidence['message_url']}")


def verify_case_provenance(case):
    change_number = case["change_number"]
    for side in ("checkpoint", "current"):
        revision = case["revisions"][side]
        commit_url = (
            f"https://gerrit-review.googlesource.com/changes/{change_number}/revisions/"
            f"{revision['patch_set']}/commit"
        )
        commit = read_gerrit_json(commit_url)
        if commit["commit"] != revision["commit"]:
            raise ValueError(f"Gerrit commit mismatch for change {change_number} {side}")
        parents = commit["parents"]
        if len(parents) != 1 or parents[0]["commit"] != revision["parent"]:
            raise ValueError(f"Gerrit parent mismatch for change {change_number} {side}")
    checkpoint = case["checkpoint_evidence"]
    verify_message_evidence(
        change_number,
        checkpoint,
        [f"Patch Set {checkpoint['patch_set']}: {checkpoint['label']}+{checkpoint['value']}"],
    )
    for field in ("merge_evidence", "rebase_evidence"):
        if field in case:
            verify_message_evidence(change_number, case[field], case[field]["message_contains"])


def materialize(manifest_path, output):
    manifest = load_manifest(manifest_path)
    if output.exists():
        raise FileExistsError(f"materialization output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=f".{output.name}.tmp-", dir=output.parent))
    repository = stage / "repository.git"
    completed = False
    try:
        for case in manifest["cases"]:
            verify_case_provenance(case)
        run_external_git(["init", "--bare", "-q", str(repository)])
        run_external_git(
            ["-C", str(repository), "remote", "add", "origin", manifest["source_repository"]["git_url"]],
        )
        refspecs = {}
        for case in manifest["cases"]:
            for side in ("checkpoint", "current"):
                upstream = case["revisions"][side]["ref"]
                refspecs[upstream] = local_ref(upstream)
        first_fetch = ["-C", str(repository), "fetch", "--no-tags", "--depth=2", "origin"]
        first_fetch.extend(f"{upstream}:{local}" for upstream, local in sorted(refspecs.items()))
        run_external_git(first_fetch)
        run_external_git(["-C", str(repository), "config", "extensions.partialClone", "origin"])
        run_external_git(["-C", str(repository), "config", "remote.origin.promisor", "true"])
        run_external_git(["-C", str(repository), "config", "remote.origin.partialclonefilter", "tree:0"])
        ancestry_fetch = [
            "-C",
            str(repository),
            "fetch",
            "--unshallow",
            "--filter=tree:0",
            "--no-tags",
            "origin",
        ]
        ancestry_fetch.extend(f"{upstream}:{local}" for upstream, local in sorted(refspecs.items()))
        run_external_git(
            ancestry_fetch,
        )
        head_commit = manifest["cases"][0]["revisions"]["current"]["commit"]
        run_external_git(["-C", str(repository), "update-ref", "refs/heads/resumebench", head_commit])
        run_external_git(
            ["-C", str(repository), "symbolic-ref", "HEAD", "refs/heads/resumebench"],
        )
        if git_stdout(repository, ["rev-parse", "--is-shallow-repository"]).decode("ascii").strip() != "false":
            raise ValueError("materialized repository remained shallow")
        for case in manifest["cases"]:
            revisions = case["revisions"]
            for side in ("requested_base_commit",):
                resolve_commit(repository, revisions[side])
            for side in ("checkpoint", "current"):
                commit = resolve_commit(repository, local_ref(revisions[side]["ref"]))
                if commit != revisions[side]["commit"]:
                    raise ValueError(f"fetched ref mismatch for {case['id']} {side}")
            generate_case_oracle(manifest, case, repository)
        git_stdout(repository, ["fsck", "--full", "--no-dangling"])
        marker = {
            "schema": "stratadiff-resumebench-real-materialization-v0",
            "dataset_version": manifest["dataset_version"],
            "manifest_sha256": sha256_bytes(manifest_path.read_bytes()),
            "repository": "repository.git",
        }
        write_json(stage / "materialization.json", marker)
        stage.replace(output)
        completed = True
    finally:
        if not completed and stage.exists():
            shutil.rmtree(stage)
    return output / "repository.git"


def self_test():
    before = "1" * 40
    after = "2" * 40
    moved = "3" * 40
    raw = (
        f":100644 100644 {before} {after} M\0same.py\0"
        f":100644 000000 {moved} {ZERO_OID} D\0old.py\0"
        f":000000 100644 {ZERO_OID} {moved} A\0new.py\0"
    ).encode("ascii")
    parsed = normalize_exact_relocations(parse_raw_diff(raw))
    if [change["status"] for change in parsed] != ["M", "R"]:
        raise AssertionError("exact relocation self-test failed")
    if parsed[1]["before_path"] != b"old.py" or parsed[1]["after_path"] != b"new.py":
        raise AssertionError("raw path self-test failed")
    if parsed[1]["similarity_percent"] != 100:
        raise AssertionError("similarity self-test failed")
    first = identity(parsed[0])
    changed_mode = dict(parsed[0])
    changed_mode["after_mode"] = "100755"
    if first["identity_sha256"] == identity(changed_mode)["identity_sha256"]:
        raise AssertionError("identity digest omitted the target mode")
    ambiguous = parse_raw_diff(
        (
            f":100644 000000 {moved} {ZERO_OID} D\0one.py\0"
            f":100644 000000 {moved} {ZERO_OID} D\0two.py\0"
            f":000000 100644 {ZERO_OID} {moved} A\0three.py\0"
        ).encode("ascii")
    )
    try:
        normalize_exact_relocations(ambiguous)
    except ValueError:
        return
    raise AssertionError("ambiguous relocation self-test did not fail")


def parse_arguments():
    parser = argparse.ArgumentParser(description="Materialize and evaluate ResumeBench-Real v0.")
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("self-test")
    provenance_command = subparsers.add_parser("verify-provenance")
    provenance_command.add_argument("--manifest", type=Path, required=True)
    for name in ("generate", "verify"):
        command = subparsers.add_parser(name)
        command.add_argument("--manifest", type=Path, required=True)
        command.add_argument("--repository", type=Path, required=True)
    materialize_command = subparsers.add_parser("materialize")
    materialize_command.add_argument("--manifest", type=Path, required=True)
    materialize_command.add_argument("--output", type=Path, required=True)
    evaluate_command = subparsers.add_parser("evaluate")
    evaluate_command.add_argument("--manifest", type=Path, required=True)
    evaluate_command.add_argument("--repository", type=Path, required=True)
    evaluate_command.add_argument("--stratadiff", type=Path, required=True)
    evaluate_command.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main():
    args = parse_arguments()
    if args.command == "self-test":
        self_test()
        print("self-test passed")
        return
    manifest_path = args.manifest.resolve()
    if args.command == "verify-provenance":
        manifest = load_manifest(manifest_path)
        for case in manifest["cases"]:
            verify_case_provenance(case)
        print(f"verified provenance for {len(manifest['cases'])} cases")
    elif args.command == "materialize":
        repository = materialize(manifest_path, args.output.resolve())
        print(f"materialized repository: {repository}")
    elif args.command == "generate":
        paths = generate_all(manifest_path, args.repository.resolve())
        print(f"generated {len(paths)} independent oracle files")
    elif args.command == "verify":
        count = verify_all(manifest_path, args.repository.resolve())
        print(f"verified {count} independent oracle files")
    elif args.command == "evaluate":
        complete = evaluate_all(
            manifest_path,
            args.repository.resolve(),
            args.stratadiff.resolve(),
            args.output.resolve(),
        )
        print(f"benchmark_complete={str(complete).lower()}")
        if not complete:
            sys.exit(1)


if __name__ == "__main__":
    main()
