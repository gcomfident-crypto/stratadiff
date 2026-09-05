#!/usr/bin/env python3

import argparse
import base64
import fcntl
import hashlib
import hmac
import json
import os
import re
import secrets
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PILOT_SOURCE = Path(__file__).resolve()
EVALUATOR = ROOT / "tools/reviewer-study-v1/reviewer_study_v1.py"
PREREGISTRATION = ROOT / "benchmarks/reviewer-study-v1/preregistration.json"

TASK_SPEC_SCHEMA = "stratadiff-reviewer-pilot-task-spec-v1"
TASK_CATALOG_SCHEMA = "stratadiff-reviewer-pilot-task-catalog-v1"
PLAN_SCHEMA = "stratadiff-reviewer-pilot-plan-v1"
PLAN_ATTESTATION_SCHEMA = "stratadiff-reviewer-pilot-plan-attestation-v1"
FINAL_ATTESTATION_SCHEMA = "stratadiff-reviewer-pilot-final-attestation-v1"
EVENT_SCHEMA = "stratadiff-reviewer-pilot-event-v1"
INVITE_SCHEMA = "stratadiff-reviewer-pilot-invite-v1"
ADJUDICATOR_SCHEMA = "stratadiff-reviewer-pilot-adjudicator-v1"
ASSIGNMENT_SCHEMA = "stratadiff-reviewer-pilot-adjudication-assignment-v1"
REVEAL_SCHEMA = "stratadiff-reviewer-pilot-adjudication-reveal-v1"
SESSION_RESULT_SCHEMA = "stratadiff-reviewer-pilot-session-result-v1"
FOLLOW_UP_SCHEMA = "stratadiff-reviewer-pilot-follow-up-v1"
DATA_SCHEMA = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/benchmarks/reviewer-study-v1/study-data.schema.json"

SIGNATURE_NAMESPACE = "stratadiff-reviewer-study-v1"
RANDOMIZATION_ALGORITHM = "hmac-sha256-rejection-fisher-yates-v1"
CLOCK_IMPLEMENTATION = "python-monotonic-ns-v1"
ZERO_SHA256 = "0" * 64
MAX_JSON_BYTES = 32 * 1024 * 1024
MAX_PUBLIC_KEY_BYTES = 16 * 1024
MAX_SIGNATURE_BYTES = 64 * 1024
MAX_BUNDLE_FILES = 10000
MAX_BUNDLE_BYTES = 1024 * 1024 * 1024
FOLLOW_UP_DAYS = 28
FOLLOW_UP_NS = FOLLOW_UP_DAYS * 24 * 60 * 60 * 1_000_000_000

STUDY_ID = re.compile(r"study_(?:[0-9a-f]{16}|synthetic_[0-9a-f]{16})")
REAL_STUDY_ID = re.compile(r"study_[0-9a-f]{16}")
SYNTHETIC_STUDY_ID = re.compile(r"study_synthetic_[0-9a-f]{16}")
PARTICIPANT_ID = re.compile(r"p_[0-9a-f]{12}")
PAIR_ID = re.compile(r"pair_[0-9a-f]{12}")
TASK_ID = re.compile(r"task_[0-9a-f]{12}")
ADJUDICATOR_SLOT_ID = re.compile(r"adjslot_[0-9a-f]{12}")
ADJUDICATOR_KEY_ID = re.compile(r"adj_[0-9a-f]{12}")
OPERATOR_KEY_ID = re.compile(r"operator_[0-9a-f]{12}")
OPAQUE_ID = re.compile(r"(?:issue|file|line|carry)_[0-9a-f]{16}")
ISSUE_ID = re.compile(r"issue_[0-9a-f]{16}")
FILE_ID = re.compile(r"file_[0-9a-f]{16}")
LINE_ID = re.compile(r"line_[0-9a-f]{16}")
CARRY_ID = re.compile(r"carry_[0-9a-f]{16}")
SHA256 = re.compile(r"[0-9a-f]{64}")

ASSIGNMENT_CELLS = (
    "baseline_then_resume:a",
    "baseline_then_resume:b",
    "resume_then_baseline:a",
    "resume_then_baseline:b",
)
DECISIONS = ("valid_carry", "false_carry")
PRE_START_REASONS = ("declined", "eligibility", "technical_pre_start")
POST_START_REASONS = ("participant_withdrew", "technical_post_start", "lost_contact")


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
    require(len(payload) <= MAX_JSON_BYTES, f"input exceeds {MAX_JSON_BYTES} bytes: {path}")
    return payload, json.loads(payload, object_pairs_hook=unique_json_object)


def read_limited_bytes(path, maximum, label):
    with path.open("rb") as handle:
        payload = handle.read(maximum + 1)
    require(len(payload) <= maximum, f"{label} exceeds {maximum} bytes")
    return payload


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")


def sha256(payload):
    return hashlib.sha256(payload).hexdigest()


def file_sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


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


def require_sha256(value, label):
    require_string(value, label)
    require(SHA256.fullmatch(value) is not None, f"{label} must be lowercase SHA-256")


def require_string_array(value, label, pattern=None, minimum=0):
    require(type(value) is list, f"{label} must be an array")
    require(len(value) >= minimum, f"{label} must contain at least {minimum} entries")
    require(len(value) == len(set(value)), f"{label} contains duplicates")
    for index, item in enumerate(value):
        require_string(item, f"{label}[{index}]")
        if pattern is not None:
            require(pattern.fullmatch(item) is not None, f"{label}[{index}] is not an opaque identifier")


def atomic_write(path, payload, mode=0o600):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = path.parent / f".{path.name}.{secrets.token_hex(8)}.tmp"
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
    with os.fdopen(descriptor, "wb") as handle:
        handle.write(payload)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)
    directory = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def write_new_private(path, payload):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as handle:
        handle.write(payload)
        handle.flush()
        os.fsync(handle.fileno())


def canonical_relative_path(path):
    relative = path.as_posix()
    require(relative != "", "bundle path must not be empty")
    require(not relative.startswith("/"), "bundle path must be relative")
    require(".." not in path.parts, "bundle path must not contain parent traversal")
    return relative


def bundle_sha256(root):
    require(root.is_dir(), f"task bundle is not a directory: {root}")
    entries = sorted(root.rglob("*"), key=lambda path: path.relative_to(root).as_posix())
    require(len(entries) <= MAX_BUNDLE_FILES, f"task bundle exceeds {MAX_BUNDLE_FILES} entries: {root}")
    digest = hashlib.sha256()
    total_bytes = 0
    file_count = 0
    for path in entries:
        relative = canonical_relative_path(path.relative_to(root))
        metadata = path.lstat()
        require(not stat.S_ISLNK(metadata.st_mode), f"task bundle contains a symlink: {relative}")
        if path.is_dir():
            digest.update(b"D\0")
            digest.update(relative.encode("utf-8"))
            digest.update(b"\0")
            continue
        require(path.is_file(), f"task bundle contains a non-regular entry: {relative}")
        file_count += 1
        total_bytes += metadata.st_size
        require(total_bytes <= MAX_BUNDLE_BYTES, f"task bundle exceeds {MAX_BUNDLE_BYTES} bytes: {root}")
        digest.update(b"F\0")
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(str(metadata.st_size).encode("ascii"))
        digest.update(b"\0")
        with path.open("rb") as handle:
            while chunk := handle.read(1024 * 1024):
                digest.update(chunk)
    require(file_count > 0, f"task bundle contains no files: {root}")
    return digest.hexdigest()


class CounterRandom:
    def __init__(self, seed, domain):
        self.seed = seed
        self.domain = domain.encode("utf-8")
        self.counter = 0

    def block(self):
        payload = self.domain + b"\0" + self.counter.to_bytes(8, "big")
        self.counter += 1
        return hmac.new(self.seed, payload, hashlib.sha256).digest()

    def randbelow(self, upper):
        require(upper > 0, "random upper bound must be positive")
        limit = (1 << 256) - ((1 << 256) % upper)
        while True:
            candidate = int.from_bytes(self.block(), "big")
            if candidate < limit:
                return candidate % upper

    def shuffle(self, values):
        result = list(values)
        for index in range(len(result) - 1, 0, -1):
            other = self.randbelow(index + 1)
            result[index], result[other] = result[other], result[index]
        return result


def derived_hex(seed, label, digits):
    return hmac.new(seed, label.encode("utf-8"), hashlib.sha256).hexdigest()[:digits]


def derived_study_id(seed, synthetic):
    prefix = "study_synthetic_" if synthetic else "study_"
    return f"{prefix}{derived_hex(seed, 'study-id-v1', 16)}"


def require_study_id(value, synthetic, label):
    pattern = SYNTHETIC_STUDY_ID if synthetic else REAL_STUDY_ID
    require_identifier(value, pattern, label)


def normalized_public_key(private_key):
    process = subprocess.run(
        ["ssh-keygen", "-y", "-f", str(private_key)],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    require(process.returncode == 0, f"ssh-keygen could not read signing key: {process.stderr.decode().strip()}")
    fields = process.stdout.decode("utf-8").strip().split()
    require(len(fields) == 2, "ssh-keygen returned an unexpected public key")
    require(fields[0] == "ssh-ed25519", "signing key must be Ed25519")
    return f"{fields[0]} {fields[1]}"


def public_key_id(public_key, prefix):
    digest = sha256(public_key.encode("utf-8"))[:12]
    return f"{prefix}_{digest}"


def require_canonical_signature(signature):
    require(len(signature) <= MAX_SIGNATURE_BYTES, "SSH signature exceeds size limit")
    try:
        text = signature.decode("ascii")
    except UnicodeDecodeError as error:
        raise ValueError("SSH signature is not ASCII armor") from error
    lines = text.splitlines()
    require(text.endswith("\n") and "\r" not in text, "SSH signature is not canonical armor")
    require(len(lines) >= 3, "SSH signature is malformed")
    require(lines[0] == "-----BEGIN SSH SIGNATURE-----", "SSH signature has an invalid header")
    require(lines[-1] == "-----END SSH SIGNATURE-----", "SSH signature has an invalid footer")
    body = "".join(lines[1:-1])
    try:
        decoded = base64.b64decode(body, validate=True)
    except ValueError as error:
        raise ValueError("SSH signature has invalid base64 armor") from error
    require(decoded.startswith(b"SSHSIG"), "SSH signature has an invalid payload")
    encoded_body = base64.b64encode(decoded).decode("ascii")
    wrapped_body = "\n".join(encoded_body[index : index + 70] for index in range(0, len(encoded_body), 70))
    canonical = f"-----BEGIN SSH SIGNATURE-----\n{wrapped_body}\n-----END SSH SIGNATURE-----\n"
    require(text == canonical, "SSH signature is not canonical armor")


def sign_bytes(private_key, payload):
    with tempfile.TemporaryDirectory(prefix="stratadiff-pilot-sign-") as temporary:
        message = Path(temporary) / "message"
        message.write_bytes(payload)
        process = subprocess.run(
            ["ssh-keygen", "-Y", "sign", "-f", str(private_key), "-n", SIGNATURE_NAMESPACE, str(message)],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        require(process.returncode == 0, f"ssh-keygen signing failed: {process.stderr.decode().strip()}")
        signature = message.with_suffix(".sig")
        require(signature.is_file(), "ssh-keygen did not write a signature")
        signature_bytes = read_limited_bytes(signature, MAX_SIGNATURE_BYTES, "SSH signature")
        require_canonical_signature(signature_bytes)
        return signature_bytes


def verify_signature(public_key, identity, payload, signature):
    require_canonical_signature(signature)
    with tempfile.TemporaryDirectory(prefix="stratadiff-pilot-verify-") as temporary:
        allowed = Path(temporary) / "allowed_signers"
        signature_path = Path(temporary) / "message.sig"
        allowed.write_text(f"{identity} {public_key}\n", encoding="utf-8")
        signature_path.write_bytes(signature)
        process = subprocess.run(
            [
                "ssh-keygen",
                "-Y",
                "verify",
                "-f",
                str(allowed),
                "-I",
                identity,
                "-n",
                SIGNATURE_NAMESPACE,
                "-s",
                str(signature_path),
            ],
            input=payload,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    require(process.returncode == 0, f"SSH signature verification failed: {process.stderr.decode().strip()}")


def boot_id_hash():
    linux_boot_id = Path("/proc/sys/kernel/random/boot_id")
    require(linux_boot_id.is_file(), "monotonic crash recovery requires Linux /proc boot identity")
    return sha256(linux_boot_id.read_bytes().strip())


def current_clock():
    return {
        "boot_id_hash": boot_id_hash(),
        "monotonic_ns": time.monotonic_ns(),
        "wall_ns": time.time_ns(),
    }


def workspace_paths(state_dir):
    return {
        "root": state_dir,
        "lock": state_dir / ".lock",
        "events": state_dir / "events",
        "plan": state_dir / "plan.json",
        "plan_attestation": state_dir / "plan-attestation.json",
        "plan_signature": state_dir / "plan-attestation.json.sig",
        "operator_public_key": state_dir / "operator.pub",
        "preregistration": state_dir / "preregistration.json",
        "task_spec": state_dir / "private-task-spec.json",
        "task_catalog": state_dir / "task-catalog.json",
        "preloaded": state_dir / "preloaded",
        "results": state_dir / "results",
    }


@contextmanager
def workspace_lock(state_dir):
    state_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(state_dir, 0o700)
    lock_path = state_dir / ".lock"
    descriptor = os.open(lock_path, os.O_RDWR | os.O_CREAT, 0o600)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield
    finally:
        fcntl.flock(descriptor, fcntl.LOCK_UN)
        os.close(descriptor)


def event_filename(sequence, digest):
    return f"{sequence:09d}.{digest}.json"


def validate_event(event, label):
    require_object(
        event,
        {"schema", "study_id", "seq", "event_id", "prev_sha256", "plan_sha256", "kind", "payload"},
        label,
    )
    require(event["schema"] == EVENT_SCHEMA, f"{label} has unsupported schema")
    require_identifier(event["study_id"], STUDY_ID, f"{label}.study_id")
    require_integer(event["seq"], 1, 1000000, f"{label}.seq")
    require_sha256(event["event_id"], f"{label}.event_id")
    require_sha256(event["prev_sha256"], f"{label}.prev_sha256")
    require_sha256(event["plan_sha256"], f"{label}.plan_sha256")
    require_string(event["kind"], f"{label}.kind")
    require(type(event["payload"]) is dict, f"{label}.payload must be an object")
    validate_event_payload(event["kind"], event["payload"], f"{label}.payload")


def require_nonnegative_integer(value, label):
    require(type(value) is int and value >= 0, f"{label} must be a non-negative integer")


def validate_event_payload(kind, payload, label):
    fields = {
        "participant_activated": {"participant_id", "generation", "credential_sha256"},
        "participant_invite_replaced": {
            "participant_id",
            "previous_generation",
            "generation",
            "credential_sha256",
            "reason",
        },
        "participant_withdrawn": {"participant_id", "reason"},
        "preflight_passed": {
            "participant_id",
            "pair_id",
            "arm",
            "variant",
            "source_bundle_sha256",
            "preloaded_sha256",
            "stratadiff_build_sha256",
        },
        "session_started": {
            "participant_id",
            "pair_id",
            "arm",
            "boot_id_hash",
            "monotonic_start_ns",
            "clock",
        },
        "session_interrupted": {"participant_id", "pair_id", "arm", "reason"},
        "session_completed": {
            "participant_id",
            "pair_id",
            "arm",
            "completion_seconds",
            "issues_found",
            "seeded_issues",
            "reopened_files",
            "reopened_lines",
            "result_sha256",
        },
        "adjudicator_registered": {"slot_id", "key_id", "public_key"},
        "adjudication_committed": {
            "study_id",
            "plan_sha256",
            "pair_id",
            "unit_id",
            "slot_id",
            "key_id",
            "role",
            "context_sha256",
            "commitment_sha256",
            "signature",
        },
        "adjudication_revealed": {
            "pair_id",
            "unit_id",
            "slot_id",
            "key_id",
            "role",
            "decision",
            "nonce",
        },
        "follow_up_invited": {
            "participant_id",
            "boot_id_hash",
            "monotonic_invited_ns",
            "wall_invited_ns",
            "wall_deadline_ns",
        },
        "follow_up_used": {"participant_id", "used", "boot_id_hash", "monotonic_used_ns", "wall_used_ns"},
        "follow_up_closed": {
            "participant_id",
            "used_within_28_days",
            "boot_id_hash",
            "monotonic_closed_ns",
            "wall_closed_ns",
        },
        "collection_locked": {"dataset_sha256", "aggregate_sha256"},
    }
    require(kind in fields, f"unsupported event kind: {kind}")
    require_object(payload, fields[kind], label)
    if "participant_id" in payload:
        require_identifier(payload["participant_id"], PARTICIPANT_ID, f"{label}.participant_id")
    if "pair_id" in payload:
        require_identifier(payload["pair_id"], PAIR_ID, f"{label}.pair_id")
    if "unit_id" in payload:
        require_identifier(payload["unit_id"], CARRY_ID, f"{label}.unit_id")
    if "slot_id" in payload:
        require_identifier(payload["slot_id"], ADJUDICATOR_SLOT_ID, f"{label}.slot_id")
    if "key_id" in payload:
        require_identifier(payload["key_id"], ADJUDICATOR_KEY_ID, f"{label}.key_id")
    for field in (
        "credential_sha256",
        "source_bundle_sha256",
        "preloaded_sha256",
        "stratadiff_build_sha256",
        "boot_id_hash",
        "result_sha256",
        "plan_sha256",
        "context_sha256",
        "commitment_sha256",
        "dataset_sha256",
        "aggregate_sha256",
    ):
        if field in payload:
            require_sha256(payload[field], f"{label}.{field}")
    for field in (
        "generation",
        "previous_generation",
        "monotonic_start_ns",
        "completion_seconds",
        "issues_found",
        "seeded_issues",
        "reopened_files",
        "reopened_lines",
        "monotonic_invited_ns",
        "monotonic_used_ns",
        "monotonic_closed_ns",
        "wall_invited_ns",
        "wall_deadline_ns",
        "wall_used_ns",
        "wall_closed_ns",
    ):
        if field in payload:
            require_nonnegative_integer(payload[field], f"{label}.{field}")
    if "arm" in payload:
        require(payload["arm"] in ("baseline", "resume"), f"{label}.arm is invalid")
    if "variant" in payload:
        require(payload["variant"] in ("a", "b"), f"{label}.variant is invalid")
    if "role" in payload:
        require(payload["role"] in ("initial", "resolver"), f"{label}.role is invalid")
    if "decision" in payload:
        require(payload["decision"] in DECISIONS, f"{label}.decision is invalid")
    if "used" in payload:
        require_boolean(payload["used"], f"{label}.used")
    if "used_within_28_days" in payload:
        require_boolean(payload["used_within_28_days"], f"{label}.used_within_28_days")
    if kind == "participant_invite_replaced":
        require(payload["reason"] in PRE_START_REASONS, f"{label}.reason is invalid")
        require(payload["generation"] == payload["previous_generation"] + 1, f"{label} generation is not contiguous")
    if kind == "participant_withdrawn":
        require(payload["reason"] in POST_START_REASONS, f"{label}.reason is invalid")
    if kind == "session_interrupted":
        require(
            payload["reason"] in ("monotonic_epoch_changed", "maximum_duration_exceeded"),
            f"{label}.reason is invalid",
        )
    if kind == "session_started":
        require(payload["clock"] == CLOCK_IMPLEMENTATION, f"{label}.clock is invalid")
    if kind == "session_completed":
        require_integer(payload["completion_seconds"], 1, 86400, f"{label}.completion_seconds")
        require(payload["issues_found"] <= payload["seeded_issues"], f"{label} issues_found exceeds seeded_issues")
    if kind == "adjudicator_registered":
        require_string(payload["public_key"], f"{label}.public_key")
    if kind == "adjudication_committed":
        require_identifier(payload["study_id"], STUDY_ID, f"{label}.study_id")
        require_string(payload["signature"], f"{label}.signature")
        base64.b64decode(payload["signature"], validate=True)
    if kind == "adjudication_revealed":
        require_sha256(payload["nonce"], f"{label}.nonce")


def read_events(state_dir, plan):
    paths = workspace_paths(state_dir)
    paths["events"].mkdir(parents=True, exist_ok=True, mode=0o700)
    observed = []
    event_ids = set()
    previous = ZERO_SHA256
    expected_sequence = 1
    for path in sorted(paths["events"].glob("*.json")):
        payload, event = read_json(path)
        validate_event(event, f"event {path.name}")
        require(payload == encoded(event), f"event JSON is not canonical: {path.name}")
        digest = sha256(payload)
        require(path.name == event_filename(expected_sequence, digest), f"event filename is not canonical: {path.name}")
        require(event["seq"] == expected_sequence, f"event sequence gap at {path.name}")
        require(event["prev_sha256"] == previous, f"event hash chain is broken at {path.name}")
        require(event["plan_sha256"] == sha256(encoded(plan)), f"event plan binding mismatch at {path.name}")
        require(event["study_id"] == plan["study_id"], f"event study binding mismatch at {path.name}")
        require(event["event_id"] not in event_ids, f"duplicate event business key at {path.name}")
        event_ids.add(event["event_id"])
        observed.append(event)
        previous = digest
        expected_sequence += 1
    return observed, previous


def append_event(state_dir, plan, events, chain_tip, kind, business_key, payload):
    event_id = sha256(f"{kind}\0{business_key}".encode("utf-8"))
    matches = [event for event in events if event["event_id"] == event_id]
    if matches:
        require(len(matches) == 1, f"duplicate event business key: {kind}:{business_key}")
        require(matches[0]["kind"] == kind and matches[0]["payload"] == payload, f"conflicting event retry: {kind}:{business_key}")
        return matches[0], chain_tip, False
    event = {
        "schema": EVENT_SCHEMA,
        "study_id": plan["study_id"],
        "seq": len(events) + 1,
        "event_id": event_id,
        "prev_sha256": chain_tip,
        "plan_sha256": sha256(encoded(plan)),
        "kind": kind,
        "payload": payload,
    }
    event_bytes = encoded(event)
    digest = sha256(event_bytes)
    path = workspace_paths(state_dir)["events"] / event_filename(event["seq"], digest)
    require(not path.exists(), f"event already exists: {path.name}")
    atomic_write(path, event_bytes)
    events.append(event)
    return event, digest, True


def events_of_kind(events, kind):
    return [event for event in events if event["kind"] == kind]


def latest_event(events, kind, predicate):
    matches = [event for event in events if event["kind"] == kind and predicate(event["payload"])]
    return matches[-1] if matches else None


def require_command(value, label, result_placeholder):
    require(type(value) is list and value, f"{label} must be a non-empty argument array")
    require(len(value) <= 1024, f"{label} exceeds 1024 arguments")
    for index, argument in enumerate(value):
        require_string(argument, f"{label}[{index}]")
    placeholder_count = value.count("{result}")
    if result_placeholder:
        require(placeholder_count == 1, f"{label} must contain one exact {{result}} argument")
    else:
        require(placeholder_count == 0, f"{label} must not contain {{result}}")


def normalize_task_spec(path, task_spec):
    require_object(task_spec, {"schema", "stratadiff_binary_path", "task_families"}, "task spec")
    require(task_spec["schema"] == TASK_SPEC_SCHEMA, "unsupported task spec schema")
    require_string(task_spec["stratadiff_binary_path"], "task spec.stratadiff_binary_path")
    binary = (path.parent / task_spec["stratadiff_binary_path"]).resolve()
    require(binary.is_file(), f"StrataDiff binary is not a file: {binary}")
    require(not binary.is_symlink(), "StrataDiff binary must not be a symlink")
    families = task_spec["task_families"]
    require(type(families) is list and families, "task spec.task_families must be a non-empty array")
    require(len(families) <= 100, "task spec exceeds 100 task families")
    family_ids = set()
    opaque_ids = set()
    normalized_families = []
    for family_index, family in enumerate(families):
        family_label = f"task spec.task_families[{family_index}]"
        require_object(family, {"task_family_id", "variants"}, family_label)
        require_identifier(family["task_family_id"], TASK_ID, f"{family_label}.task_family_id")
        require(family["task_family_id"] not in family_ids, "task spec has duplicate task_family_id")
        family_ids.add(family["task_family_id"])
        variants = family["variants"]
        require_object(variants, {"a", "b"}, f"{family_label}.variants")
        normalized_variants = {}
        seeded_counts = []
        for variant_name in ("a", "b"):
            variant = variants[variant_name]
            variant_label = f"{family_label}.variants.{variant_name}"
            require_object(
                variant,
                {"response_issue_ids", "seeded_issue_ids", "presentations"},
                variant_label,
            )
            require_string_array(variant["response_issue_ids"], f"{variant_label}.response_issue_ids", ISSUE_ID, 1)
            require_string_array(variant["seeded_issue_ids"], f"{variant_label}.seeded_issue_ids", ISSUE_ID, 1)
            require(len(variant["response_issue_ids"]) <= 10000, f"{variant_label}.response_issue_ids is too large")
            require(len(variant["seeded_issue_ids"]) <= 10000, f"{variant_label}.seeded_issue_ids is too large")
            require(
                set(variant["seeded_issue_ids"]).issubset(variant["response_issue_ids"]),
                f"{variant_label}.seeded_issue_ids must be a subset of response_issue_ids",
            )
            seeded_counts.append(len(variant["seeded_issue_ids"]))
            for identifier in variant["response_issue_ids"]:
                require(identifier not in opaque_ids, f"task spec reuses opaque identifier: {identifier}")
                opaque_ids.add(identifier)
            presentations = variant["presentations"]
            require_object(presentations, {"baseline", "resume"}, f"{variant_label}.presentations")
            normalized_presentations = {}
            for arm in ("baseline", "resume"):
                presentation = presentations[arm]
                presentation_label = f"{variant_label}.presentations.{arm}"
                require_object(
                    presentation,
                    {
                        "bundle_path",
                        "preflight_command",
                        "run_command",
                        "reopened_file_ids",
                        "reopened_line_ids",
                        "carried_unit_ids",
                    },
                    presentation_label,
                )
                require_string(presentation["bundle_path"], f"{presentation_label}.bundle_path")
                bundle = (path.parent / presentation["bundle_path"]).resolve()
                require(bundle.is_dir(), f"task bundle is not a directory: {bundle}")
                require_command(presentation["preflight_command"], f"{presentation_label}.preflight_command", False)
                require_command(presentation["run_command"], f"{presentation_label}.run_command", True)
                require_string_array(
                    presentation["reopened_file_ids"], f"{presentation_label}.reopened_file_ids", FILE_ID, 1
                )
                require_string_array(
                    presentation["reopened_line_ids"], f"{presentation_label}.reopened_line_ids", LINE_ID, 1
                )
                require_string_array(
                    presentation["carried_unit_ids"], f"{presentation_label}.carried_unit_ids", CARRY_ID
                )
                if arm == "baseline":
                    require(not presentation["carried_unit_ids"], f"{presentation_label} must not contain carry units")
                require(len(presentation["reopened_file_ids"]) <= 100000, f"{presentation_label}.reopened_file_ids is too large")
                require(len(presentation["reopened_line_ids"]) <= 100000000, f"{presentation_label}.reopened_line_ids is too large")
                require(len(presentation["carried_unit_ids"]) <= 100000, f"{presentation_label}.carried_unit_ids is too large")
                for field in ("reopened_file_ids", "reopened_line_ids", "carried_unit_ids"):
                    for identifier in presentation[field]:
                        require(identifier not in opaque_ids, f"task spec reuses opaque identifier: {identifier}")
                        opaque_ids.add(identifier)
                normalized_presentations[arm] = {
                    "bundle_path": str(bundle),
                    "preflight_command": presentation["preflight_command"],
                    "run_command": presentation["run_command"],
                    "reopened_file_ids": sorted(presentation["reopened_file_ids"]),
                    "reopened_line_ids": sorted(presentation["reopened_line_ids"]),
                    "carried_unit_ids": sorted(presentation["carried_unit_ids"]),
                }
            normalized_variants[variant_name] = {
                "response_issue_ids": sorted(variant["response_issue_ids"]),
                "seeded_issue_ids": sorted(variant["seeded_issue_ids"]),
                "presentations": normalized_presentations,
            }
        require(seeded_counts[0] == seeded_counts[1], f"{family_label} variants must contain equal seeded issue counts")
        normalized_families.append(
            {"task_family_id": family["task_family_id"], "variants": normalized_variants}
        )
    normalized_families.sort(key=lambda family: family["task_family_id"])
    return {
        "schema": TASK_SPEC_SCHEMA,
        "stratadiff_binary_path": str(binary),
        "task_families": normalized_families,
    }


def task_catalog_from_spec(task_spec):
    families = []
    for family in task_spec["task_families"]:
        variants = {}
        for variant_name in ("a", "b"):
            source_variant = family["variants"][variant_name]
            presentations = {}
            for arm in ("baseline", "resume"):
                source_presentation = source_variant["presentations"][arm]
                presentations[arm] = {
                    "bundle_sha256": bundle_sha256(Path(source_presentation["bundle_path"])),
                    "reopened_file_ids": source_presentation["reopened_file_ids"],
                    "reopened_line_ids": source_presentation["reopened_line_ids"],
                    "carried_unit_ids": source_presentation["carried_unit_ids"],
                }
            variants[variant_name] = {
                "response_issue_ids": source_variant["response_issue_ids"],
                "seeded_issue_ids": source_variant["seeded_issue_ids"],
                "presentations": presentations,
            }
        families.append({"task_family_id": family["task_family_id"], "variants": variants})
    catalog = {
        "schema": TASK_CATALOG_SCHEMA,
        "stratadiff_build_sha256": file_sha256(Path(task_spec["stratadiff_binary_path"])),
        "task_families": families,
    }
    validate_task_catalog(catalog)
    return catalog


def validate_task_catalog(task_catalog):
    require_object(task_catalog, {"schema", "stratadiff_build_sha256", "task_families"}, "task catalog")
    require(task_catalog["schema"] == TASK_CATALOG_SCHEMA, "unsupported task catalog schema")
    require_sha256(task_catalog["stratadiff_build_sha256"], "task catalog.stratadiff_build_sha256")
    families = task_catalog["task_families"]
    require(type(families) is list and families, "task catalog.task_families must be a non-empty array")
    require(len(families) <= 100, "task catalog exceeds 100 task families")
    family_ids = set()
    all_ids = set()
    for family_index, family in enumerate(families):
        family_label = f"task catalog.task_families[{family_index}]"
        require_object(family, {"task_family_id", "variants"}, family_label)
        require_identifier(family["task_family_id"], TASK_ID, f"{family_label}.task_family_id")
        require(family["task_family_id"] not in family_ids, "task catalog has duplicate task_family_id")
        family_ids.add(family["task_family_id"])
        require_object(family["variants"], {"a", "b"}, f"{family_label}.variants")
        seeded_counts = []
        for variant_name in ("a", "b"):
            variant = family["variants"][variant_name]
            variant_label = f"{family_label}.variants.{variant_name}"
            require_object(
                variant,
                {"response_issue_ids", "seeded_issue_ids", "presentations"},
                variant_label,
            )
            require_string_array(variant["response_issue_ids"], f"{variant_label}.response_issue_ids", ISSUE_ID, 1)
            require_string_array(variant["seeded_issue_ids"], f"{variant_label}.seeded_issue_ids", ISSUE_ID, 1)
            require(len(variant["response_issue_ids"]) <= 10000, f"{variant_label}.response_issue_ids is too large")
            require(len(variant["seeded_issue_ids"]) <= 10000, f"{variant_label}.seeded_issue_ids is too large")
            require(
                set(variant["seeded_issue_ids"]).issubset(variant["response_issue_ids"]),
                f"{variant_label}.seeded_issue_ids must be a subset of response_issue_ids",
            )
            seeded_counts.append(len(variant["seeded_issue_ids"]))
            for identifier in variant["response_issue_ids"]:
                require(identifier not in all_ids, f"task catalog reuses opaque identifier: {identifier}")
                all_ids.add(identifier)
            require_object(variant["presentations"], {"baseline", "resume"}, f"{variant_label}.presentations")
            for arm in ("baseline", "resume"):
                presentation = variant["presentations"][arm]
                presentation_label = f"{variant_label}.presentations.{arm}"
                require_object(
                    presentation,
                    {"bundle_sha256", "reopened_file_ids", "reopened_line_ids", "carried_unit_ids"},
                    presentation_label,
                )
                require_sha256(presentation["bundle_sha256"], f"{presentation_label}.bundle_sha256")
                require_string_array(
                    presentation["reopened_file_ids"], f"{presentation_label}.reopened_file_ids", FILE_ID, 1
                )
                require_string_array(
                    presentation["reopened_line_ids"], f"{presentation_label}.reopened_line_ids", LINE_ID, 1
                )
                require_string_array(
                    presentation["carried_unit_ids"], f"{presentation_label}.carried_unit_ids", CARRY_ID
                )
                if arm == "baseline":
                    require(not presentation["carried_unit_ids"], f"{presentation_label} must not contain carry units")
                require(len(presentation["reopened_file_ids"]) <= 100000, f"{presentation_label}.reopened_file_ids is too large")
                require(len(presentation["reopened_line_ids"]) <= 100000000, f"{presentation_label}.reopened_line_ids is too large")
                require(len(presentation["carried_unit_ids"]) <= 100000, f"{presentation_label}.carried_unit_ids is too large")
                for field in ("reopened_file_ids", "reopened_line_ids", "carried_unit_ids"):
                    for identifier in presentation[field]:
                        require(identifier not in all_ids, f"task catalog reuses opaque identifier: {identifier}")
                        all_ids.add(identifier)
        require(seeded_counts[0] == seeded_counts[1], f"{family_label} variants have unequal seeded issue counts")
    require(
        [family["task_family_id"] for family in families] == sorted(family_ids),
        "task catalog families must use canonical task_family_id order",
    )


def task_family_map(task_catalog):
    return {family["task_family_id"]: family for family in task_catalog["task_families"]}


def cell_counts(assignments):
    counts = {cell: 0 for cell in ASSIGNMENT_CELLS}
    for assignment in assignments:
        cell = f"{assignment['assignment_order']}:{assignment['baseline_variant']}"
        require(cell in counts, f"unsupported assignment cell: {cell}")
        counts[cell] += 1
    return counts


def cell_difference(assignments):
    counts = cell_counts(assignments)
    return max(counts.values()) - min(counts.values())


def validate_preregistration_mode(preregistration_bytes, preregistration, synthetic):
    canonical_bytes, canonical = read_json(PREREGISTRATION)
    if synthetic:
        require_object(preregistration["minimums"], canonical["minimums"], "synthetic preregistration.minimums")
        for key, value in preregistration["minimums"].items():
            require_integer(value, 1, 1000000, f"synthetic preregistration.minimums.{key}")
        expected = json.loads(json.dumps(canonical))
        expected["minimums"] = preregistration["minimums"]
        expected_bytes = canonical_bytes if preregistration == canonical else encoded(preregistration)
        require(
            preregistration == expected and preregistration_bytes == expected_bytes,
            "synthetic preregistration may change only canonical integer minimums",
        )
    else:
        require(
            preregistration_bytes == canonical_bytes,
            "real pilot must use the canonical Reviewer Study v1 preregistration",
        )


def validate_assignment_balance(assignments):
    require(cell_difference(assignments) <= 1, "global assignment cells are not counterbalanced")
    for field in ("participant_id", "task_family_id"):
        grouped = {}
        for assignment in assignments:
            key = assignment[field]
            if key not in grouped:
                grouped[key] = []
            grouped[key].append(assignment)
        for key, group in grouped.items():
            require(cell_difference(group) <= 1, f"{field} assignment cells are not counterbalanced: {key}")


def build_plan(
    study_id,
    synthetic,
    participant_slots,
    adjudicator_slots,
    seed,
    task_spec,
    task_catalog,
    preregistration_bytes,
    preregistration,
):
    require_study_id(study_id, synthetic, "study_id")
    require(study_id == derived_study_id(seed, synthetic), "study_id is not derived from the frozen seed")
    validate_preregistration_mode(preregistration_bytes, preregistration, synthetic)
    require_integer(participant_slots, 4, 500, "participant_slots")
    require(participant_slots % 4 == 0, "participant_slots must be a multiple of four")
    require_integer(adjudicator_slots, 3, 20, "adjudicator_slots")
    require(
        participant_slots * len(task_catalog["task_families"]) <= 10000,
        "participant slots multiplied by task families exceeds the 10,000-pair protocol limit",
    )
    participants = [f"p_{derived_hex(seed, f'participant:{index}', 12)}" for index in range(participant_slots)]
    require(len(participants) == len(set(participants)), "derived participant IDs collided")
    adjudicators = [f"adjslot_{derived_hex(seed, f'adjudicator:{index}', 12)}" for index in range(adjudicator_slots)]
    require(len(adjudicators) == len(set(adjudicators)), "derived adjudicator slot IDs collided")
    randomizer = CounterRandom(seed, "reviewer-pilot-assignment-v1")
    randomized_participants = randomizer.shuffle(participants)
    randomized_tasks = randomizer.shuffle([family["task_family_id"] for family in task_catalog["task_families"]])
    randomized_cells = randomizer.shuffle(ASSIGNMENT_CELLS)
    randomized_adjudicators = randomizer.shuffle(adjudicators)
    families = task_family_map(task_catalog)
    assignments = []
    for participant_index, participant_id in enumerate(randomized_participants):
        participant_tasks = CounterRandom(
            seed, f"reviewer-pilot-task-order-v1:{participant_id}"
        ).shuffle(randomized_tasks)
        task_rank = {task_id: index for index, task_id in enumerate(randomized_tasks)}
        for sequence, task_id in enumerate(participant_tasks):
            column = task_rank[task_id]
            cell = randomized_cells[(participant_index + column) % len(randomized_cells)]
            assignment_order, baseline_variant = cell.split(":")
            resume_variant = "b" if baseline_variant == "a" else "a"
            pair_id = f"pair_{derived_hex(seed, f'pair:{participant_id}:{task_id}', 12)}"
            family = families[task_id]
            resume_presentation = family["variants"][resume_variant]["presentations"]["resume"]
            carried_units = resume_presentation["carried_unit_ids"]
            if carried_units:
                units = [(unit_id, True) for unit_id in carried_units]
            else:
                units = [(f"carry_{derived_hex(seed, f'empty:{pair_id}', 16)}", False)]
            adjudication = []
            for unit_index, (unit_id, counts_as_carry) in enumerate(units):
                offset = (len(assignments) + unit_index) % len(randomized_adjudicators)
                rotated = randomized_adjudicators[offset:] + randomized_adjudicators[:offset]
                adjudication.append(
                    {
                        "unit_id": unit_id,
                        "counts_as_carry": counts_as_carry,
                        "initial_slots": rotated[:2],
                        "resolver_slot": rotated[2],
                    }
                )
            assignments.append(
                {
                    "pair_id": pair_id,
                    "participant_id": participant_id,
                    "task_family_id": task_id,
                    "sequence": sequence,
                    "assignment_order": assignment_order,
                    "baseline_variant": baseline_variant,
                    "resume_variant": resume_variant,
                    "arms": {
                        "baseline": {
                            "variant": baseline_variant,
                            "bundle_sha256": family["variants"][baseline_variant]["presentations"]["baseline"][
                                "bundle_sha256"
                            ],
                        },
                        "resume": {
                            "variant": resume_variant,
                            "bundle_sha256": resume_presentation["bundle_sha256"],
                        },
                    },
                    "adjudication": adjudication,
                }
            )
    validate_assignment_balance(assignments)
    return {
        "schema": PLAN_SCHEMA,
        "study_id": study_id,
        "protocol_version": preregistration["protocol_version"],
        "preregistration_sha256": sha256(preregistration_bytes),
        "synthetic": synthetic,
        "randomization": {
            "algorithm": RANDOMIZATION_ALGORITHM,
            "seed_hex": seed.hex(),
            "seed_commitment_sha256": sha256(seed),
        },
        "task_catalog_sha256": sha256(encoded(task_catalog)),
        "task_spec_sha256": sha256(encoded(task_spec)),
        "pilot_source_sha256": file_sha256(PILOT_SOURCE),
        "stratadiff_build_sha256": task_catalog["stratadiff_build_sha256"],
        "participants": randomized_participants,
        "adjudicator_slots": randomized_adjudicators,
        "assignments": assignments,
    }


def validate_plan(plan):
    require_object(
        plan,
        {
            "schema",
            "study_id",
            "protocol_version",
            "preregistration_sha256",
            "synthetic",
            "randomization",
            "task_catalog_sha256",
            "task_spec_sha256",
            "pilot_source_sha256",
            "stratadiff_build_sha256",
            "participants",
            "adjudicator_slots",
            "assignments",
        },
        "pilot plan",
    )
    require(plan["schema"] == PLAN_SCHEMA, "unsupported pilot plan schema")
    require_boolean(plan["synthetic"], "pilot plan.synthetic")
    require_study_id(plan["study_id"], plan["synthetic"], "pilot plan.study_id")
    require(plan["protocol_version"] == "1.0.0", "unsupported pilot protocol version")
    for field in (
        "preregistration_sha256",
        "task_catalog_sha256",
        "task_spec_sha256",
        "pilot_source_sha256",
        "stratadiff_build_sha256",
    ):
        require_sha256(plan[field], f"pilot plan.{field}")
    randomization = plan["randomization"]
    require_object(randomization, {"algorithm", "seed_hex", "seed_commitment_sha256"}, "pilot plan.randomization")
    require(randomization["algorithm"] == RANDOMIZATION_ALGORITHM, "unsupported randomization algorithm")
    require_sha256(randomization["seed_hex"], "pilot plan.randomization.seed_hex")
    require_sha256(randomization["seed_commitment_sha256"], "pilot plan.randomization.seed_commitment_sha256")
    require(sha256(bytes.fromhex(randomization["seed_hex"])) == randomization["seed_commitment_sha256"], "seed commitment mismatch")
    require_string_array(plan["participants"], "pilot plan.participants", PARTICIPANT_ID, 4)
    require(len(plan["participants"]) % 4 == 0, "pilot participant slots must be a multiple of four")
    require(len(plan["participants"]) <= 500, "pilot plan exceeds 500 participant slots")
    require_string_array(plan["adjudicator_slots"], "pilot plan.adjudicator_slots", ADJUDICATOR_SLOT_ID, 3)
    require(len(plan["adjudicator_slots"]) <= 20, "pilot plan exceeds 20 adjudicator slots")
    assignments = plan["assignments"]
    require(type(assignments) is list and assignments, "pilot plan.assignments must be a non-empty array")
    require(len(assignments) <= 10000, "pilot plan exceeds 10,000 assignments")
    pair_ids = set()
    participant_tasks = set()
    participant_ids = set(plan["participants"])
    slot_ids = set(plan["adjudicator_slots"])
    for index, assignment in enumerate(assignments):
        label = f"pilot plan.assignments[{index}]"
        require_object(
            assignment,
            {
                "pair_id",
                "participant_id",
                "task_family_id",
                "sequence",
                "assignment_order",
                "baseline_variant",
                "resume_variant",
                "arms",
                "adjudication",
            },
            label,
        )
        require_identifier(assignment["pair_id"], PAIR_ID, f"{label}.pair_id")
        require(assignment["pair_id"] not in pair_ids, "pilot plan contains duplicate pair_id")
        pair_ids.add(assignment["pair_id"])
        require(assignment["participant_id"] in participant_ids, f"{label} references unknown participant")
        require_identifier(assignment["task_family_id"], TASK_ID, f"{label}.task_family_id")
        participant_task = (assignment["participant_id"], assignment["task_family_id"])
        require(participant_task not in participant_tasks, f"{label} repeats a participant task family")
        participant_tasks.add(participant_task)
        require_integer(assignment["sequence"], 0, 99, f"{label}.sequence")
        require(assignment["assignment_order"] in ("baseline_then_resume", "resume_then_baseline"), f"{label} has invalid order")
        require(assignment["baseline_variant"] in ("a", "b"), f"{label} has invalid baseline variant")
        require(assignment["resume_variant"] in ("a", "b"), f"{label} has invalid resume variant")
        require(assignment["baseline_variant"] != assignment["resume_variant"], f"{label} reuses one variant")
        require_object(assignment["arms"], {"baseline", "resume"}, f"{label}.arms")
        for arm in ("baseline", "resume"):
            require_object(assignment["arms"][arm], {"variant", "bundle_sha256"}, f"{label}.arms.{arm}")
            require(assignment["arms"][arm]["variant"] in ("a", "b"), f"{label}.arms.{arm}.variant is invalid")
            require_sha256(assignment["arms"][arm]["bundle_sha256"], f"{label}.arms.{arm}.bundle_sha256")
        require(type(assignment["adjudication"]) is list and assignment["adjudication"], f"{label}.adjudication must be non-empty")
        unit_ids = set()
        for unit_index, unit in enumerate(assignment["adjudication"]):
            unit_label = f"{label}.adjudication[{unit_index}]"
            require_object(unit, {"unit_id", "counts_as_carry", "initial_slots", "resolver_slot"}, unit_label)
            require_identifier(unit["unit_id"], CARRY_ID, f"{unit_label}.unit_id")
            require(unit["unit_id"] not in unit_ids, f"{label} has duplicate adjudication unit")
            unit_ids.add(unit["unit_id"])
            require_boolean(unit["counts_as_carry"], f"{unit_label}.counts_as_carry")
            require_string_array(unit["initial_slots"], f"{unit_label}.initial_slots", ADJUDICATOR_SLOT_ID, 2)
            require(len(unit["initial_slots"]) == 2, f"{unit_label}.initial_slots must contain exactly two slots")
            require(set(unit["initial_slots"]).issubset(slot_ids), f"{unit_label} references unknown initial slot")
            require(unit["resolver_slot"] in slot_ids, f"{unit_label} references unknown resolver slot")
            require(unit["resolver_slot"] not in unit["initial_slots"], f"{unit_label} resolver must be independent")
    for participant_id in participant_ids:
        sequences = sorted(
            assignment["sequence"] for assignment in assignments if assignment["participant_id"] == participant_id
        )
        require(sequences == list(range(len(sequences))), f"participant task sequence is not contiguous: {participant_id}")
    validate_assignment_balance(assignments)


def validate_plan_randomization(plan, task_catalog, preregistration_bytes, preregistration):
    expected = build_plan(
        plan["study_id"],
        plan["synthetic"],
        len(plan["participants"]),
        len(plan["adjudicator_slots"]),
        bytes.fromhex(plan["randomization"]["seed_hex"]),
        {},
        task_catalog,
        preregistration_bytes,
        preregistration,
    )
    require(plan["participants"] == expected["participants"], "participant randomization does not replay")
    require(plan["adjudicator_slots"] == expected["adjudicator_slots"], "adjudicator randomization does not replay")
    require(plan["assignments"] == expected["assignments"], "assignment randomization does not replay")
    require(plan["protocol_version"] == preregistration["protocol_version"], "plan protocol version mismatch")
    require(plan["preregistration_sha256"] == sha256(preregistration_bytes), "plan preregistration hash mismatch")
    require(plan["task_catalog_sha256"] == sha256(encoded(task_catalog)), "plan task catalog hash mismatch")
    require(
        plan["stratadiff_build_sha256"] == task_catalog["stratadiff_build_sha256"],
        "plan StrataDiff build hash mismatch",
    )


def plan_create(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        paths = workspace_paths(state_dir)
        unexpected = [path for path in state_dir.iterdir() if path.name != ".lock"]
        require(not unexpected, f"state directory is not empty: {state_dir}")
        task_spec_path = arguments.task_spec.resolve()
        _, source_task_spec = read_json(task_spec_path)
        task_spec = normalize_task_spec(task_spec_path, source_task_spec)
        task_catalog = task_catalog_from_spec(task_spec)
        preregistration_bytes, preregistration = read_json(arguments.preregistration.resolve())
        if arguments.seed_file is None:
            seed = secrets.token_bytes(32)
        else:
            seed_text = arguments.seed_file.read_text(encoding="utf-8").strip()
            require(SHA256.fullmatch(seed_text) is not None, "seed file must contain exactly 64 lowercase hex characters")
            seed = bytes.fromhex(seed_text)
        study_id = derived_study_id(seed, arguments.synthetic)
        plan = build_plan(
            study_id,
            arguments.synthetic,
            arguments.participant_slots,
            arguments.adjudicator_slots,
            seed,
            task_spec,
            task_catalog,
            preregistration_bytes,
            preregistration,
        )
        validate_plan(plan)
        paths["events"].mkdir(mode=0o700)
        paths["preloaded"].mkdir(mode=0o700)
        paths["results"].mkdir(mode=0o700)
        write_new_private(paths["task_spec"], encoded(task_spec))
        write_new_private(paths["task_catalog"], encoded(task_catalog))
        write_new_private(paths["preregistration"], preregistration_bytes)
        write_new_private(paths["plan"], encoded(plan))
        os.chmod(paths["plan"], 0o400)
        print(f"created frozen pilot plan {sha256(encoded(plan))} with {len(plan['assignments'])} paired assignments")


def plan_attest(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        paths = workspace_paths(state_dir)
        _, plan = read_json(paths["plan"])
        validate_plan(plan)
        events, _ = read_events(state_dir, plan)
        require(not events, "plan must be attested before the first collection event")
        require_sha256(arguments.anchor_sha256, "plan external anchor SHA-256")
        public_key = normalized_public_key(arguments.operator_key.resolve())
        operator_id = public_key_id(public_key, "operator")
        require_identifier(operator_id, OPERATOR_KEY_ID, "operator key ID")
        attestation = {
            "schema": PLAN_ATTESTATION_SCHEMA,
            "kind": "plan",
            "study_id": plan["study_id"],
            "operator_key_id": operator_id,
            "preregistration_sha256": plan["preregistration_sha256"],
            "plan_sha256": sha256(encoded(plan)),
            "plan_anchor_sha256": arguments.anchor_sha256,
            "task_catalog_sha256": plan["task_catalog_sha256"],
            "task_spec_sha256": plan["task_spec_sha256"],
            "tool_source_sha256": plan["pilot_source_sha256"],
            "signature_algorithm": "openssh-ed25519-sshsig",
        }
        signature = sign_bytes(arguments.operator_key.resolve(), encoded(attestation))
        verify_signature(public_key, operator_id, encoded(attestation), signature)
        if paths["plan_attestation"].exists() or paths["plan_signature"].exists():
            _, observed = read_json(paths["plan_attestation"])
            require(observed == attestation, "existing plan attestation differs")
            require(
                read_limited_bytes(paths["plan_signature"], MAX_SIGNATURE_BYTES, "plan signature") == signature,
                "existing plan signature differs",
            )
        else:
            write_new_private(paths["plan_attestation"], encoded(attestation))
            write_new_private(paths["plan_signature"], signature)
            write_new_private(paths["operator_public_key"], f"{public_key}\n".encode("utf-8"))
            os.chmod(paths["operator_public_key"], 0o444)
        print(f"attested pilot plan as {operator_id}; collection may begin")


def validate_plan_attestation(attestation, plan):
    require_object(
        attestation,
        {
            "schema",
            "kind",
            "study_id",
            "operator_key_id",
            "preregistration_sha256",
            "plan_sha256",
            "plan_anchor_sha256",
            "task_catalog_sha256",
            "task_spec_sha256",
            "tool_source_sha256",
            "signature_algorithm",
        },
        "plan attestation",
    )
    require(attestation["schema"] == PLAN_ATTESTATION_SCHEMA, "unsupported plan attestation schema")
    require(attestation["kind"] == "plan", "plan attestation has wrong kind")
    require(attestation["study_id"] == plan["study_id"], "plan attestation study mismatch")
    require_identifier(attestation["operator_key_id"], OPERATOR_KEY_ID, "plan attestation.operator_key_id")
    for field in (
        "preregistration_sha256",
        "plan_sha256",
        "plan_anchor_sha256",
        "task_catalog_sha256",
        "task_spec_sha256",
        "tool_source_sha256",
    ):
        require_sha256(attestation[field], f"plan attestation.{field}")
    require(attestation["preregistration_sha256"] == plan["preregistration_sha256"], "plan preregistration binding mismatch")
    require(attestation["plan_sha256"] == sha256(encoded(plan)), "plan attestation hash mismatch")
    require(attestation["task_catalog_sha256"] == plan["task_catalog_sha256"], "plan catalog binding mismatch")
    require(attestation["task_spec_sha256"] == plan["task_spec_sha256"], "plan task-spec binding mismatch")
    require(attestation["tool_source_sha256"] == plan["pilot_source_sha256"], "plan tool binding mismatch")
    require(attestation["signature_algorithm"] == "openssh-ed25519-sshsig", "unsupported signature algorithm")


def load_context(state_dir, require_attested=True, verify_event_signatures=False):
    paths = workspace_paths(state_dir)
    _, plan = read_json(paths["plan"])
    validate_plan(plan)
    _, task_spec = read_json(paths["task_spec"])
    _, task_catalog = read_json(paths["task_catalog"])
    validate_task_catalog(task_catalog)
    preregistration_bytes, preregistration = read_json(paths["preregistration"])
    require(sha256(encoded(task_spec)) == plan["task_spec_sha256"], "private task spec changed after planning")
    require(sha256(encoded(task_catalog)) == plan["task_catalog_sha256"], "task catalog changed after planning")
    require(sha256(preregistration_bytes) == plan["preregistration_sha256"], "preregistration changed after planning")
    require(file_sha256(PILOT_SOURCE) == plan["pilot_source_sha256"], "pilot source changed after planning")
    require(file_sha256(Path(task_spec["stratadiff_binary_path"])) == plan["stratadiff_build_sha256"], "pinned StrataDiff binary changed")
    validate_plan_randomization(plan, task_catalog, preregistration_bytes, preregistration)
    attestation = None
    public_key = None
    if require_attested:
        _, attestation = read_json(paths["plan_attestation"])
        validate_plan_attestation(attestation, plan)
        public_key = normalized_public_key_file(paths["operator_public_key"], canonical=True)
        require(public_key_id(public_key, "operator") == attestation["operator_key_id"], "operator public key mismatch")
        verify_signature(
            public_key,
            attestation["operator_key_id"],
            encoded(attestation),
            read_limited_bytes(paths["plan_signature"], MAX_SIGNATURE_BYTES, "plan signature"),
        )
    events, chain_tip = read_events(state_dir, plan)
    validate_event_history(plan, task_catalog, events, verify_event_signatures)
    return paths, plan, task_spec, task_catalog, attestation, public_key, events, chain_tip


def require_operator_key(private_key, attestation):
    public_key = normalized_public_key(private_key.resolve())
    require(public_key_id(public_key, "operator") == attestation["operator_key_id"], "operator key does not match frozen plan")


def plan_verify(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, attestation, _, events, chain_tip = load_context(
            state_dir, verify_event_signatures=True
        )
        print(
            f"verified pilot plan {attestation['plan_sha256']}: {len(plan['assignments'])} pairs, "
            f"events={len(events)}, chain_tip={chain_tip}"
        )


def collection_locked(events):
    return bool(events_of_kind(events, "collection_locked"))


def participant_credential_event(events, participant_id):
    candidates = [
        event
        for event in events
        if event["kind"] in ("participant_activated", "participant_invite_replaced")
        and event["payload"]["participant_id"] == participant_id
    ]
    return candidates[-1] if candidates else None


def participant_started(events, participant_id, plan):
    pair_ids = {
        assignment["pair_id"]
        for assignment in plan["assignments"]
        if assignment["participant_id"] == participant_id
    }
    return any(
        event["kind"] == "session_started" and event["payload"]["pair_id"] in pair_ids for event in events
    )


def participant_withdrawal(events, participant_id):
    return latest_event(
        events,
        "participant_withdrawn",
        lambda payload: payload["participant_id"] == participant_id,
    )


def invite_payload(plan, participant_id, generation, token):
    return {
        "schema": INVITE_SCHEMA,
        "study_id": plan["study_id"],
        "participant_id": participant_id,
        "generation": generation,
        "token": token,
    }


def write_receipt(path, payload):
    require(not path.exists(), f"receipt already exists: {path}")
    write_new_private(path.resolve(), encoded(payload))


def read_invite(path, plan, events):
    _, invite = read_json(path.resolve())
    require_object(invite, {"schema", "study_id", "participant_id", "generation", "token"}, "participant invite")
    require(invite["schema"] == INVITE_SCHEMA, "unsupported participant invite schema")
    require(invite["study_id"] == plan["study_id"], "participant invite belongs to another study")
    require_identifier(invite["participant_id"], PARTICIPANT_ID, "participant invite.participant_id")
    require_integer(invite["generation"], 0, 1000, "participant invite.generation")
    require_sha256(invite["token"], "participant invite.token")
    credential = participant_credential_event(events, invite["participant_id"])
    require(credential is not None, "participant has not been activated")
    require(credential["payload"]["generation"] == invite["generation"], "participant invite has been superseded")
    require(
        hmac.compare_digest(credential["payload"]["credential_sha256"], sha256(bytes.fromhex(invite["token"]))),
        "participant invite credential is invalid",
    )
    require(participant_withdrawal(events, invite["participant_id"]) is None, "participant has withdrawn")
    return invite


def enroll(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, attestation, _, events, chain_tip = load_context(state_dir)
        require_operator_key(arguments.operator_key, attestation)
        require(not collection_locked(events), "collection is already locked")
        activated = {
            event["payload"]["participant_id"] for event in events_of_kind(events, "participant_activated")
        }
        available = [participant_id for participant_id in plan["participants"] if participant_id not in activated]
        require(available, "all frozen participant slots have been activated")
        participant_id = available[0]
        token = secrets.token_hex(32)
        payload = {
            "participant_id": participant_id,
            "generation": 0,
            "credential_sha256": sha256(bytes.fromhex(token)),
        }
        append_event(state_dir, plan, events, chain_tip, "participant_activated", participant_id, payload)
        write_receipt(arguments.receipt_out, invite_payload(plan, participant_id, 0, token))
        print(f"activated next frozen participant slot {participant_id}")


def attrition_replace(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, attestation, _, events, chain_tip = load_context(state_dir)
        require_operator_key(arguments.operator_key, attestation)
        invite = read_invite(arguments.invite, plan, events)
        participant_id = invite["participant_id"]
        require(not participant_started(events, participant_id, plan), "an exposed participant slot cannot be replaced")
        require(arguments.reason in PRE_START_REASONS, "unsupported pre-start attrition reason")
        generation = invite["generation"] + 1
        token = secrets.token_hex(32)
        payload = {
            "participant_id": participant_id,
            "previous_generation": invite["generation"],
            "generation": generation,
            "credential_sha256": sha256(bytes.fromhex(token)),
            "reason": arguments.reason,
        }
        append_event(
            state_dir,
            plan,
            events,
            chain_tip,
            "participant_invite_replaced",
            f"{participant_id}:{generation}",
            payload,
        )
        write_receipt(arguments.receipt_out, invite_payload(plan, participant_id, generation, token))
        print(f"rotated pre-start invite for frozen participant slot {participant_id}")


def attrition_withdraw(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, _, _, events, chain_tip = load_context(state_dir)
        invite = read_invite(arguments.invite, plan, events)
        require(arguments.reason in POST_START_REASONS, "unsupported post-start attrition reason")
        participant_id = invite["participant_id"]
        require(participant_started(events, participant_id, plan), "use invite replacement before a session starts")
        payload = {"participant_id": participant_id, "reason": arguments.reason}
        _, _, created = append_event(
            state_dir, plan, events, chain_tip, "participant_withdrawn", participant_id, payload
        )
        require(created, "participant withdrawal was already recorded")
        print(f"recorded post-start attrition for {participant_id}; collection lock will fail closed")


def assignments_for_participant(plan, participant_id):
    assignments = [
        assignment for assignment in plan["assignments"] if assignment["participant_id"] == participant_id
    ]
    return sorted(assignments, key=lambda assignment: assignment["sequence"])


def arm_sequence(assignment):
    if assignment["assignment_order"] == "baseline_then_resume":
        return ("baseline", "resume")
    return ("resume", "baseline")


def session_event(events, kind, pair_id, arm):
    return latest_event(
        events,
        kind,
        lambda payload: payload["pair_id"] == pair_id and payload["arm"] == arm,
    )


def current_session(plan, events, participant_id):
    for assignment in assignments_for_participant(plan, participant_id):
        for arm in arm_sequence(assignment):
            completed = session_event(events, "session_completed", assignment["pair_id"], arm)
            if completed is not None:
                continue
            interrupted = session_event(events, "session_interrupted", assignment["pair_id"], arm)
            require(interrupted is None, f"current session was interrupted and cannot be reopened: {assignment['pair_id']}:{arm}")
            return assignment, arm
    return None, None


def private_presentation(task_spec, task_family_id, variant, arm):
    matches = [family for family in task_spec["task_families"] if family["task_family_id"] == task_family_id]
    require(len(matches) == 1, f"private task spec is missing family {task_family_id}")
    return matches[0]["variants"][variant]["presentations"][arm]


def public_variant(task_catalog, task_family_id, variant):
    matches = [family for family in task_catalog["task_families"] if family["task_family_id"] == task_family_id]
    require(len(matches) == 1, f"task catalog is missing family {task_family_id}")
    return matches[0]["variants"][variant]


def preloaded_path(paths, pair_id, arm):
    return paths["preloaded"] / pair_id / arm


def session_preflight(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        paths, plan, task_spec, task_catalog, _, _, events, chain_tip = load_context(state_dir)
        require(not collection_locked(events), "collection is already locked")
        invite = read_invite(arguments.invite, plan, events)
        assignment, arm = current_session(plan, events, invite["participant_id"])
        require(assignment is not None, "participant has completed every frozen assignment")
        pair_id = assignment["pair_id"]
        existing = session_event(events, "preflight_passed", pair_id, arm)
        destination = preloaded_path(paths, pair_id, arm)
        if existing is not None:
            require(destination.is_dir(), "preflight receipt exists but preloaded bundle is missing")
            require(bundle_sha256(destination) == existing["payload"]["preloaded_sha256"], "preloaded bundle changed")
            print(f"preflight already passed for {pair_id}:{arm}")
            return
        variant = assignment["arms"][arm]["variant"]
        presentation = private_presentation(task_spec, assignment["task_family_id"], variant, arm)
        source = Path(presentation["bundle_path"])
        source_digest = bundle_sha256(source)
        require(source_digest == assignment["arms"][arm]["bundle_sha256"], "task bundle differs from frozen plan")
        destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        if destination.exists():
            ready_digest = bundle_sha256(destination)
        else:
            temporary = destination.parent / f".{arm}.{secrets.token_hex(8)}.preflight"
            shutil.copytree(source, temporary)
            process = subprocess.run(
                presentation["preflight_command"],
                cwd=temporary,
                env={
                    **os.environ,
                    "STRATADIFF_BINARY": task_spec["stratadiff_binary_path"],
                    "STRATADIFF_PILOT_OFFLINE": "1",
                },
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            require(
                process.returncode == 0,
                f"task preflight failed with {process.returncode}: {process.stderr.decode('utf-8', errors='replace').strip()}",
            )
            ready_digest = bundle_sha256(temporary)
            os.replace(temporary, destination)
            directory = os.open(destination.parent, os.O_RDONLY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        payload = {
            "participant_id": invite["participant_id"],
            "pair_id": pair_id,
            "arm": arm,
            "variant": variant,
            "source_bundle_sha256": source_digest,
            "preloaded_sha256": ready_digest,
            "stratadiff_build_sha256": plan["stratadiff_build_sha256"],
        }
        append_event(state_dir, plan, events, chain_tip, "preflight_passed", f"{pair_id}:{arm}", payload)
        print(f"preflight passed before timing for {pair_id}:{arm} ({variant})")


def validate_session_result(result, variant, presentation):
    require_object(
        result,
        {"schema", "submitted_issue_ids", "reopened_file_ids", "reopened_line_ids"},
        "session result",
    )
    require(result["schema"] == SESSION_RESULT_SCHEMA, "unsupported session result schema")
    require_string_array(result["submitted_issue_ids"], "session result.submitted_issue_ids", ISSUE_ID)
    require_string_array(result["reopened_file_ids"], "session result.reopened_file_ids", FILE_ID)
    require_string_array(result["reopened_line_ids"], "session result.reopened_line_ids", LINE_ID)
    require(set(result["submitted_issue_ids"]).issubset(variant["response_issue_ids"]), "session submitted an unknown issue token")
    require(set(result["reopened_file_ids"]).issubset(presentation["reopened_file_ids"]), "session reopened an unknown file token")
    require(set(result["reopened_line_ids"]).issubset(presentation["reopened_line_ids"]), "session reopened an unknown line token")


def session_run(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        paths, plan, task_spec, task_catalog, _, _, events, chain_tip = load_context(state_dir)
        require(not collection_locked(events), "collection is already locked")
        invite = read_invite(arguments.invite, plan, events)
        assignment, arm = current_session(plan, events, invite["participant_id"])
        require(assignment is not None, "participant has completed every frozen assignment")
        pair_id = assignment["pair_id"]
        preflight = session_event(events, "preflight_passed", pair_id, arm)
        require(preflight is not None, "session preflight must pass before timing starts")
        destination = preloaded_path(paths, pair_id, arm)
        require(destination.is_dir(), "preloaded task bundle is missing")
        require(bundle_sha256(destination) == preflight["payload"]["preloaded_sha256"], "preloaded task bundle changed")
        started = session_event(events, "session_started", pair_id, arm)
        result_path = paths["results"] / f"{pair_id}.{arm}.json"
        if started is None:
            require(not result_path.exists(), "session result exists before the monotonic start event")
            if not arguments.yes:
                confirmation = input(f"Type START to begin timed {arm} arm {pair_id}: ")
                require(confirmation == "START", "timed session was not started")
            clock = current_clock()
            start_payload = {
                "participant_id": invite["participant_id"],
                "pair_id": pair_id,
                "arm": arm,
                "boot_id_hash": clock["boot_id_hash"],
                "monotonic_start_ns": clock["monotonic_ns"],
                "clock": CLOCK_IMPLEMENTATION,
            }
            started, chain_tip, _ = append_event(
                state_dir, plan, events, chain_tip, "session_started", f"{pair_id}:{arm}", start_payload
            )
        clock = current_clock()
        if clock["boot_id_hash"] != started["payload"]["boot_id_hash"]:
            interrupted_payload = {
                "participant_id": invite["participant_id"],
                "pair_id": pair_id,
                "arm": arm,
                "reason": "monotonic_epoch_changed",
            }
            append_event(
                state_dir,
                plan,
                events,
                chain_tip,
                "session_interrupted",
                f"{pair_id}:{arm}",
                interrupted_payload,
            )
            raise ValueError("system boot changed during a timed session; the arm is interrupted and cannot restart")
        variant_name = assignment["arms"][arm]["variant"]
        private = private_presentation(task_spec, assignment["task_family_id"], variant_name, arm)
        public = public_variant(task_catalog, assignment["task_family_id"], variant_name)
        presentation = public["presentations"][arm]
        if not result_path.exists():
            command = [str(result_path) if argument == "{result}" else argument for argument in private["run_command"]]
            process = subprocess.run(
                command,
                cwd=destination,
                env={
                    **os.environ,
                    "STRATADIFF_BINARY": task_spec["stratadiff_binary_path"],
                    "STRATADIFF_PILOT_OFFLINE": "1",
                    "STRATADIFF_PILOT_NO_OPEN": "1" if arguments.no_open else "0",
                },
                check=False,
            )
            require(process.returncode == 0, f"task runner exited with {process.returncode}; rerun continues the same timer")
        result_bytes, result = read_json(result_path)
        validate_session_result(result, public, presentation)
        finish = current_clock()
        require(finish["boot_id_hash"] == started["payload"]["boot_id_hash"], "monotonic epoch changed before submission")
        elapsed_ns = finish["monotonic_ns"] - started["payload"]["monotonic_start_ns"]
        require(elapsed_ns > 0, "monotonic session duration must be positive")
        completion_seconds = (elapsed_ns + 999_999_999) // 1_000_000_000
        if completion_seconds > 86400:
            interrupted_payload = {
                "participant_id": invite["participant_id"],
                "pair_id": pair_id,
                "arm": arm,
                "reason": "maximum_duration_exceeded",
            }
            append_event(
                state_dir,
                plan,
                events,
                chain_tip,
                "session_interrupted",
                f"{pair_id}:{arm}",
                interrupted_payload,
            )
            raise ValueError("timed session exceeded 86400 seconds and is permanently interrupted")
        reopened_files = len(result["reopened_file_ids"])
        reopened_lines = len(result["reopened_line_ids"])
        if arm == "baseline":
            require(reopened_files >= 1, "baseline must record at least one explicit reopened file")
            require(reopened_lines >= 1, "baseline must record at least one explicit reopened line")
        complete_payload = {
            "participant_id": invite["participant_id"],
            "pair_id": pair_id,
            "arm": arm,
            "completion_seconds": completion_seconds,
            "issues_found": len(set(result["submitted_issue_ids"]) & set(public["seeded_issue_ids"])),
            "seeded_issues": len(public["seeded_issue_ids"]),
            "reopened_files": reopened_files,
            "reopened_lines": reopened_lines,
            "result_sha256": sha256(result_bytes),
        }
        append_event(state_dir, plan, events, chain_tip, "session_completed", f"{pair_id}:{arm}", complete_payload)
        print(
            f"completed {pair_id}:{arm} in {completion_seconds}s; "
            f"issues={complete_payload['issues_found']}/{complete_payload['seeded_issues']}, "
            f"reopened={reopened_files} files/{reopened_lines} lines"
        )


def session_status(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, _, _, events, _ = load_context(state_dir)
        invite = read_invite(arguments.invite, plan, events)
        assignments = assignments_for_participant(plan, invite["participant_id"])
        completed = 0
        for assignment in assignments:
            for arm in arm_sequence(assignment):
                if session_event(events, "session_completed", assignment["pair_id"], arm) is not None:
                    completed += 1
        assignment, arm = current_session(plan, events, invite["participant_id"])
        current = "complete" if assignment is None else f"{assignment['pair_id']}:{arm}"
        print(f"participant {invite['participant_id']}: {completed}/{len(assignments) * 2} arms complete; current={current}")


def normalized_public_key_file(path, canonical=False):
    payload = read_limited_bytes(path, MAX_PUBLIC_KEY_BYTES, "public key")
    fields = payload.decode("ascii").split()
    require(len(fields) >= 2, "public key file is malformed")
    require(fields[0] == "ssh-ed25519", "public key must be Ed25519")
    base64.b64decode(fields[1], validate=True)
    public_key = f"{fields[0]} {fields[1]}"
    if canonical:
        require(payload == f"{public_key}\n".encode("ascii"), "public key file is not canonical")
    return public_key


def adjudicator_registration(events, key_id=None, slot_id=None):
    matches = []
    for event in events_of_kind(events, "adjudicator_registered"):
        if key_id is not None and event["payload"]["key_id"] != key_id:
            continue
        if slot_id is not None and event["payload"]["slot_id"] != slot_id:
            continue
        matches.append(event)
    require(len(matches) <= 1, "adjudicator registration is ambiguous")
    return matches[0] if matches else None


def adjudicator_register(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, attestation, _, events, chain_tip = load_context(state_dir)
        require_operator_key(arguments.operator_key, attestation)
        require(not collection_locked(events), "collection is already locked")
        public_key = normalized_public_key_file(arguments.public_key.resolve())
        key_id = public_key_id(public_key, "adj")
        require_identifier(key_id, ADJUDICATOR_KEY_ID, "adjudicator key ID")
        require(adjudicator_registration(events, key_id=key_id) is None, "adjudicator key is already registered")
        registered_slots = {
            event["payload"]["slot_id"] for event in events_of_kind(events, "adjudicator_registered")
        }
        available = [slot_id for slot_id in plan["adjudicator_slots"] if slot_id not in registered_slots]
        require(available, "all frozen adjudicator slots are registered")
        slot_id = available[0]
        payload = {"slot_id": slot_id, "key_id": key_id, "public_key": public_key}
        append_event(state_dir, plan, events, chain_tip, "adjudicator_registered", slot_id, payload)
        receipt = {
            "schema": ADJUDICATOR_SCHEMA,
            "study_id": plan["study_id"],
            "slot_id": slot_id,
            "key_id": key_id,
            "public_key": public_key,
        }
        write_receipt(arguments.receipt_out, receipt)
        print(f"bound next frozen adjudicator slot {slot_id} to {key_id}")


def planned_units(plan):
    units = []
    for assignment in plan["assignments"]:
        for unit in assignment["adjudication"]:
            units.append(
                {
                    "pair_id": assignment["pair_id"],
                    "participant_id": assignment["participant_id"],
                    "task_family_id": assignment["task_family_id"],
                    "resume_variant": assignment["resume_variant"],
                    **unit,
                }
            )
    return units


def planned_unit(plan, pair_id, unit_id):
    matches = [unit for unit in planned_units(plan) if unit["pair_id"] == pair_id and unit["unit_id"] == unit_id]
    require(len(matches) == 1, "adjudication unit is not in the frozen plan")
    return matches[0]


def adjudication_commit_event(events, pair_id, unit_id, slot_id):
    return latest_event(
        events,
        "adjudication_committed",
        lambda payload: payload["pair_id"] == pair_id
        and payload["unit_id"] == unit_id
        and payload["slot_id"] == slot_id,
    )


def adjudication_reveal_event(events, pair_id, unit_id, slot_id):
    return latest_event(
        events,
        "adjudication_revealed",
        lambda payload: payload["pair_id"] == pair_id
        and payload["unit_id"] == unit_id
        and payload["slot_id"] == slot_id,
    )


def initial_reveals(events, unit):
    return [
        adjudication_reveal_event(events, unit["pair_id"], unit["unit_id"], slot_id)
        for slot_id in unit["initial_slots"]
    ]


def unit_outcome(events, unit):
    reveals = initial_reveals(events, unit)
    if any(reveal is None for reveal in reveals):
        return None
    decisions = [reveal["payload"]["decision"] for reveal in reveals]
    if decisions[0] == decisions[1]:
        return decisions[0]
    resolver = adjudication_reveal_event(events, unit["pair_id"], unit["unit_id"], unit["resolver_slot"])
    return None if resolver is None else resolver["payload"]["decision"]


def adjudicator_identity(private_key, events):
    public_key = normalized_public_key(private_key.resolve())
    key_id = public_key_id(public_key, "adj")
    registration = adjudicator_registration(events, key_id=key_id)
    require(registration is not None, "adjudicator key is not registered")
    require(registration["payload"]["public_key"] == public_key, "adjudicator public key registration mismatch")
    return registration["payload"]["slot_id"], key_id, public_key


def unit_role(events, unit, slot_id):
    if slot_id in unit["initial_slots"]:
        return "initial"
    if slot_id == unit["resolver_slot"]:
        reveals = initial_reveals(events, unit)
        require(all(reveal is not None for reveal in reveals), "resolver is not assigned before both initial reveals")
        require(
            reveals[0]["payload"]["decision"] != reveals[1]["payload"]["decision"],
            "resolver is not assigned when initial decisions agree",
        )
        return "resolver"
    raise ValueError("adjudicator slot is not assigned to this unit")


def assignment_context(plan, unit, slot_id, key_id, role):
    return {
        "study_id": plan["study_id"],
        "plan_sha256": sha256(encoded(plan)),
        "pair_id": unit["pair_id"],
        "unit_id": unit["unit_id"],
        "slot_id": slot_id,
        "key_id": key_id,
        "role": role,
        "counts_as_carry": unit["counts_as_carry"],
    }


def assignment_receipt(plan, unit, slot_id, key_id, role):
    context = assignment_context(plan, unit, slot_id, key_id, role)
    return {"schema": ASSIGNMENT_SCHEMA, **context, "context_sha256": sha256(encoded(context))}


def adjudication_assign(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        paths, plan, _, _, _, _, events, _ = load_context(state_dir)
        slot_id, key_id, _ = adjudicator_identity(arguments.adjudicator_key, events)
        selected = None
        selected_role = None
        for unit in planned_units(plan):
            resume = session_event(events, "session_completed", unit["pair_id"], "resume")
            if resume is None:
                continue
            if slot_id in unit["initial_slots"]:
                if adjudication_commit_event(events, unit["pair_id"], unit["unit_id"], slot_id) is None:
                    selected = unit
                    selected_role = "initial"
                    break
            elif slot_id == unit["resolver_slot"]:
                reveals = initial_reveals(events, unit)
                if all(reveal is not None for reveal in reveals):
                    disagreement = reveals[0]["payload"]["decision"] != reveals[1]["payload"]["decision"]
                    missing = adjudication_commit_event(events, unit["pair_id"], unit["unit_id"], slot_id) is None
                    if disagreement and missing:
                        selected = unit
                        selected_role = "resolver"
                        break
        require(selected is not None, "no adjudication commitment is currently assigned to this key")
        receipt = assignment_receipt(plan, selected, slot_id, key_id, selected_role)
        write_receipt(arguments.receipt_out, receipt)
        bundle = preloaded_path(paths, selected["pair_id"], "resume")
        require(bundle.is_dir(), "resume adjudication bundle is missing")
        print(
            f"assigned {selected_role} adjudication {selected['pair_id']}:{selected['unit_id']} "
            f"from local bundle {bundle}"
        )


def read_assignment_receipt(path, plan, events, private_key):
    _, receipt = read_json(path.resolve())
    require_object(
        receipt,
        {
            "schema",
            "study_id",
            "plan_sha256",
            "pair_id",
            "unit_id",
            "slot_id",
            "key_id",
            "role",
            "counts_as_carry",
            "context_sha256",
        },
        "adjudication assignment",
    )
    require(receipt["schema"] == ASSIGNMENT_SCHEMA, "unsupported adjudication assignment schema")
    unit = planned_unit(plan, receipt["pair_id"], receipt["unit_id"])
    slot_id, key_id, public_key = adjudicator_identity(private_key, events)
    require(receipt["study_id"] == plan["study_id"], "adjudication assignment study mismatch")
    require(receipt["plan_sha256"] == sha256(encoded(plan)), "adjudication assignment plan mismatch")
    require(receipt["slot_id"] == slot_id and receipt["key_id"] == key_id, "adjudication assignment belongs to another key")
    require(receipt["counts_as_carry"] == unit["counts_as_carry"], "adjudication assignment carry kind mismatch")
    require(receipt["role"] == unit_role(events, unit, slot_id), "adjudication assignment role mismatch")
    context = {key: receipt[key] for key in receipt if key not in ("schema", "context_sha256")}
    require(receipt["context_sha256"] == sha256(encoded(context)), "adjudication assignment context changed")
    return receipt, unit, public_key


def commitment_payload(receipt, decision, nonce):
    return encoded(
        {
            "context_sha256": receipt["context_sha256"],
            "decision": decision,
            "nonce": nonce,
        }
    )


def adjudication_commit(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, _, _, events, chain_tip = load_context(state_dir)
        receipt, unit, public_key = read_assignment_receipt(
            arguments.assignment, plan, events, arguments.adjudicator_key
        )
        require(arguments.decision in DECISIONS, "unsupported adjudication decision")
        if not unit["counts_as_carry"]:
            require(arguments.decision == "valid_carry", "empty-manifest confirmation cannot report a false carry")
        if arguments.reveal_out.exists():
            _, reveal = read_json(arguments.reveal_out.resolve())
            require(reveal["decision"] == arguments.decision, "existing reveal receipt has another decision")
        else:
            nonce = secrets.token_hex(32)
            commitment = sha256(commitment_payload(receipt, arguments.decision, nonce))
            statement = {
                "study_id": plan["study_id"],
                "plan_sha256": receipt["plan_sha256"],
                "pair_id": receipt["pair_id"],
                "unit_id": receipt["unit_id"],
                "slot_id": receipt["slot_id"],
                "key_id": receipt["key_id"],
                "role": receipt["role"],
                "context_sha256": receipt["context_sha256"],
                "commitment_sha256": commitment,
            }
            signature = sign_bytes(arguments.adjudicator_key.resolve(), encoded(statement))
            verify_signature(public_key, receipt["key_id"], encoded(statement), signature)
            reveal = {
                "schema": REVEAL_SCHEMA,
                **statement,
                "decision": arguments.decision,
                "nonce": nonce,
                "signature": base64.b64encode(signature).decode("ascii"),
            }
            write_receipt(arguments.reveal_out, reveal)
        require_object(
            reveal,
            {
                "schema",
                "study_id",
                "plan_sha256",
                "pair_id",
                "unit_id",
                "slot_id",
                "key_id",
                "role",
                "context_sha256",
                "commitment_sha256",
                "decision",
                "nonce",
                "signature",
            },
            "adjudication reveal receipt",
        )
        require(reveal["schema"] == REVEAL_SCHEMA, "unsupported reveal receipt schema")
        require(reveal["decision"] == arguments.decision, "reveal receipt decision mismatch")
        require(SHA256.fullmatch(reveal["nonce"]) is not None, "reveal nonce must be 256-bit lowercase hex")
        require(
            reveal["commitment_sha256"]
            == sha256(commitment_payload(receipt, reveal["decision"], reveal["nonce"])),
            "adjudication commitment mismatch",
        )
        statement = {key: reveal[key] for key in reveal if key not in ("schema", "decision", "nonce", "signature")}
        signature = base64.b64decode(reveal["signature"], validate=True)
        verify_signature(public_key, reveal["key_id"], encoded(statement), signature)
        payload = {**statement, "signature": reveal["signature"]}
        append_event(
            state_dir,
            plan,
            events,
            chain_tip,
            "adjudication_committed",
            f"{receipt['pair_id']}:{receipt['unit_id']}:{receipt['slot_id']}",
            payload,
        )
        print(f"committed blind {receipt['role']} adjudication; keep reveal receipt private")


def validate_reveal_receipt(reveal, plan, events):
    require_object(
        reveal,
        {
            "schema",
            "study_id",
            "plan_sha256",
            "pair_id",
            "unit_id",
            "slot_id",
            "key_id",
            "role",
            "context_sha256",
            "commitment_sha256",
            "decision",
            "nonce",
            "signature",
        },
        "adjudication reveal receipt",
    )
    require(reveal["schema"] == REVEAL_SCHEMA, "unsupported reveal receipt schema")
    unit = planned_unit(plan, reveal["pair_id"], reveal["unit_id"])
    require(reveal["study_id"] == plan["study_id"], "reveal study mismatch")
    require(reveal["plan_sha256"] == sha256(encoded(plan)), "reveal plan mismatch")
    require(reveal["decision"] in DECISIONS, "reveal decision is invalid")
    require(SHA256.fullmatch(reveal["nonce"]) is not None, "reveal nonce must be 256-bit lowercase hex")
    registration = adjudicator_registration(events, key_id=reveal["key_id"])
    require(registration is not None, "reveal adjudicator is not registered")
    require(registration["payload"]["slot_id"] == reveal["slot_id"], "reveal adjudicator slot mismatch")
    require(reveal["role"] == unit_role(events, unit, reveal["slot_id"]), "reveal role mismatch")
    context = assignment_context(plan, unit, reveal["slot_id"], reveal["key_id"], reveal["role"])
    require(reveal["context_sha256"] == sha256(encoded(context)), "reveal context mismatch")
    require(
        reveal["commitment_sha256"] == sha256(commitment_payload({"context_sha256": reveal["context_sha256"]}, reveal["decision"], reveal["nonce"])),
        "reveal does not open its commitment",
    )
    statement = {key: reveal[key] for key in reveal if key not in ("schema", "decision", "nonce", "signature")}
    signature = base64.b64decode(reveal["signature"], validate=True)
    verify_signature(registration["payload"]["public_key"], reveal["key_id"], encoded(statement), signature)
    committed = adjudication_commit_event(events, reveal["pair_id"], reveal["unit_id"], reveal["slot_id"])
    require(committed is not None, "reveal has no prior commitment")
    require(committed["payload"] == {**statement, "signature": reveal["signature"]}, "reveal differs from committed statement")
    return unit


def adjudication_reveal(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, _, _, events, chain_tip = load_context(state_dir)
        _, reveal = read_json(arguments.reveal.resolve())
        unit = planned_unit(plan, reveal["pair_id"], reveal["unit_id"])
        if reveal["role"] == "initial":
            require(
                all(
                    adjudication_commit_event(events, unit["pair_id"], unit["unit_id"], slot_id) is not None
                    for slot_id in unit["initial_slots"]
                ),
                "both independent initial commitments are required before either reveal",
            )
        validate_reveal_receipt(reveal, plan, events)
        payload = {
            "pair_id": reveal["pair_id"],
            "unit_id": reveal["unit_id"],
            "slot_id": reveal["slot_id"],
            "key_id": reveal["key_id"],
            "role": reveal["role"],
            "decision": reveal["decision"],
            "nonce": reveal["nonce"],
        }
        append_event(
            state_dir,
            plan,
            events,
            chain_tip,
            "adjudication_revealed",
            f"{reveal['pair_id']}:{reveal['unit_id']}:{reveal['slot_id']}",
            payload,
        )
        outcome = unit_outcome(events, unit)
        status = "resolved" if outcome is not None else "awaiting independent reveal or resolver"
        print(f"revealed {reveal['role']} adjudication; unit is {status}")


def adjudication_status(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, _, _, events, _ = load_context(state_dir)
        completed_resume_pairs = {
            event["payload"]["pair_id"] for event in events_of_kind(events, "session_completed") if event["payload"]["arm"] == "resume"
        }
        eligible = [unit for unit in planned_units(plan) if unit["pair_id"] in completed_resume_pairs]
        resolved = [unit for unit in eligible if unit_outcome(events, unit) is not None]
        conflicts = 0
        for unit in eligible:
            reveals = initial_reveals(events, unit)
            if all(reveal is not None for reveal in reveals) and reveals[0]["payload"]["decision"] != reveals[1]["payload"]["decision"]:
                if unit_outcome(events, unit) is None:
                    conflicts += 1
        print(f"adjudication: {len(resolved)}/{len(eligible)} units resolved; unresolved_conflicts={conflicts}")


def require_event_key(event, business_key):
    expected = sha256(f"{event['kind']}\0{business_key}".encode("utf-8"))
    require(event["event_id"] == expected, f"event business key mismatch at sequence {event['seq']}")


def validate_event_history(plan, task_catalog, events, verify_signatures):
    prefix = []
    activated = []
    registered_slots = []
    registered_keys = set()
    locked = False
    assignments = {assignment["pair_id"]: assignment for assignment in plan["assignments"]}
    for event in events:
        kind = event["kind"]
        payload = event["payload"]
        require(not locked, "event appears after collection lock")
        if kind == "participant_activated":
            participant_id = payload["participant_id"]
            require(len(activated) < len(plan["participants"]), "too many participants were activated")
            require(participant_id == plan["participants"][len(activated)], "participant activation bypassed frozen order")
            require(payload["generation"] == 0, "initial participant generation must be zero")
            require_event_key(event, participant_id)
            activated.append(participant_id)
        elif kind == "participant_invite_replaced":
            participant_id = payload["participant_id"]
            credential = participant_credential_event(prefix, participant_id)
            require(credential is not None, "invite replacement precedes activation")
            require(not participant_started(prefix, participant_id, plan), "invite replacement follows task exposure")
            require(credential["payload"]["generation"] == payload["previous_generation"], "invite generation skipped")
            require_event_key(event, f"{participant_id}:{payload['generation']}")
        elif kind == "participant_withdrawn":
            participant_id = payload["participant_id"]
            require(participant_id in activated, "withdrawal references inactive participant")
            require(participant_started(prefix, participant_id, plan), "post-start withdrawal precedes task exposure")
            require(participant_withdrawal(prefix, participant_id) is None, "duplicate participant withdrawal")
            require_event_key(event, participant_id)
        elif kind in ("preflight_passed", "session_started", "session_completed", "session_interrupted"):
            participant_id = payload["participant_id"]
            pair_id = payload["pair_id"]
            arm = payload["arm"]
            require(participant_id in activated, f"{kind} references inactive participant")
            require(pair_id in assignments, f"{kind} references unknown pair")
            assignment = assignments[pair_id]
            require(assignment["participant_id"] == participant_id, f"{kind} crosses participant assignments")
            current_assignment, current_arm = current_session(plan, prefix, participant_id)
            require(
                current_assignment is not None and current_assignment["pair_id"] == pair_id and current_arm == arm,
                f"{kind} violates frozen pair or arm order",
            )
            require_event_key(event, f"{pair_id}:{arm}")
            if kind == "preflight_passed":
                require(session_event(prefix, "preflight_passed", pair_id, arm) is None, "duplicate preflight event")
                require(payload["variant"] == assignment["arms"][arm]["variant"], "preflight variant mismatch")
                require(
                    payload["source_bundle_sha256"] == assignment["arms"][arm]["bundle_sha256"],
                    "preflight bundle mismatch",
                )
                require(payload["stratadiff_build_sha256"] == plan["stratadiff_build_sha256"], "preflight build mismatch")
            elif kind == "session_started":
                require(session_event(prefix, "preflight_passed", pair_id, arm) is not None, "session started before preflight")
                require(session_event(prefix, "session_started", pair_id, arm) is None, "duplicate session start")
                require(payload["monotonic_start_ns"] > 0, "session monotonic start must be positive")
            elif kind == "session_completed":
                require(session_event(prefix, "session_started", pair_id, arm) is not None, "session completed before start")
                require(session_event(prefix, "session_completed", pair_id, arm) is None, "duplicate session completion")
                variant_name = assignment["arms"][arm]["variant"]
                variant = public_variant(task_catalog, assignment["task_family_id"], variant_name)
                require(payload["seeded_issues"] == len(variant["seeded_issue_ids"]), "session seeded issue count changed")
                if arm == "baseline":
                    require(payload["reopened_files"] >= 1, "baseline completed without an explicit reopened file")
                    require(payload["reopened_lines"] >= 1, "baseline completed without an explicit reopened line")
            else:
                require(session_event(prefix, "session_started", pair_id, arm) is not None, "session interrupted before start")
                require(session_event(prefix, "session_completed", pair_id, arm) is None, "completed session was interrupted")
                require(session_event(prefix, "session_interrupted", pair_id, arm) is None, "duplicate session interruption")
        elif kind == "adjudicator_registered":
            slot_id = payload["slot_id"]
            require(len(registered_slots) < len(plan["adjudicator_slots"]), "too many adjudicators were registered")
            require(slot_id == plan["adjudicator_slots"][len(registered_slots)], "adjudicator registration bypassed frozen order")
            require(payload["key_id"] not in registered_keys, "one adjudicator key occupies multiple slots")
            require(public_key_id(payload["public_key"], "adj") == payload["key_id"], "adjudicator key ID mismatch")
            require_event_key(event, slot_id)
            registered_slots.append(slot_id)
            registered_keys.add(payload["key_id"])
        elif kind == "adjudication_committed":
            unit = planned_unit(plan, payload["pair_id"], payload["unit_id"])
            require(participant_pair_complete(prefix, payload["pair_id"]), "adjudication commitment precedes pair completion")
            registration = adjudicator_registration(prefix, slot_id=payload["slot_id"])
            require(registration is not None, "adjudication commitment uses an unregistered slot")
            require(registration["payload"]["key_id"] == payload["key_id"], "adjudication commitment key mismatch")
            require(payload["role"] == unit_role(prefix, unit, payload["slot_id"]), "adjudication commitment role mismatch")
            context = assignment_context(plan, unit, payload["slot_id"], payload["key_id"], payload["role"])
            for field in ("study_id", "plan_sha256", "pair_id", "unit_id", "slot_id", "key_id", "role"):
                require(payload[field] == context[field], f"adjudication commitment {field} mismatch")
            require(
                payload["context_sha256"] == sha256(encoded(context)),
                "adjudication commitment context mismatch",
            )
            require(adjudication_commit_event(prefix, payload["pair_id"], payload["unit_id"], payload["slot_id"]) is None, "duplicate adjudication commitment")
            if verify_signatures:
                statement = {key: payload[key] for key in payload if key != "signature"}
                verify_signature(
                    registration["payload"]["public_key"],
                    payload["key_id"],
                    encoded(statement),
                    base64.b64decode(payload["signature"], validate=True),
                )
            require_event_key(event, f"{payload['pair_id']}:{payload['unit_id']}:{payload['slot_id']}")
        elif kind == "adjudication_revealed":
            unit = planned_unit(plan, payload["pair_id"], payload["unit_id"])
            commit = adjudication_commit_event(prefix, payload["pair_id"], payload["unit_id"], payload["slot_id"])
            require(commit is not None, "adjudication reveal precedes commitment")
            if payload["role"] == "initial":
                require(
                    all(
                        adjudication_commit_event(prefix, unit["pair_id"], unit["unit_id"], slot_id) is not None
                        for slot_id in unit["initial_slots"]
                    ),
                    "initial adjudication reveal precedes two commitments",
                )
            require(payload["role"] == unit_role(prefix, unit, payload["slot_id"]), "adjudication reveal role mismatch")
            require(payload["key_id"] == commit["payload"]["key_id"], "adjudication reveal key differs from commitment")
            require(payload["role"] == commit["payload"]["role"], "adjudication reveal role differs from commitment")
            require(adjudication_reveal_event(prefix, payload["pair_id"], payload["unit_id"], payload["slot_id"]) is None, "duplicate adjudication reveal")
            require(
                commit["payload"]["commitment_sha256"]
                == sha256(
                    commitment_payload(
                        {"context_sha256": commit["payload"]["context_sha256"]},
                        payload["decision"],
                        payload["nonce"],
                    )
                ),
                "adjudication reveal does not open commitment",
            )
            require_event_key(event, f"{payload['pair_id']}:{payload['unit_id']}:{payload['slot_id']}")
        elif kind == "follow_up_invited":
            participant_id = payload["participant_id"]
            require(participant_id in activated, "follow-up invitation references inactive participant")
            require(
                any(
                    participant_pair_complete(prefix, assignment["pair_id"])
                    for assignment in assignments_for_participant(plan, participant_id)
                ),
                "follow-up invitation precedes every completed pair",
            )
            require(follow_up_event(prefix, "follow_up_invited", participant_id) is None, "duplicate follow-up invitation")
            require(payload["wall_deadline_ns"] - payload["wall_invited_ns"] == FOLLOW_UP_NS, "follow-up window is not 28 days")
            require_event_key(event, participant_id)
        elif kind == "follow_up_used":
            participant_id = payload["participant_id"]
            require(payload["used"] is True, "follow-up use event must record true")
            require(follow_up_event(prefix, "follow_up_invited", participant_id) is not None, "follow-up use precedes invitation")
            require(follow_up_event(prefix, "follow_up_used", participant_id) is None, "duplicate follow-up use")
            require(follow_up_event(prefix, "follow_up_closed", participant_id) is None, "follow-up use follows close")
            invited = follow_up_event(prefix, "follow_up_invited", participant_id)["payload"]
            require(invited["wall_invited_ns"] <= payload["wall_used_ns"] <= invited["wall_deadline_ns"], "follow-up use is outside the window")
            if payload["boot_id_hash"] == invited["boot_id_hash"]:
                require(payload["monotonic_used_ns"] >= invited["monotonic_invited_ns"], "follow-up monotonic use precedes invitation")
            require_event_key(event, participant_id)
        elif kind == "follow_up_closed":
            participant_id = payload["participant_id"]
            require(follow_up_event(prefix, "follow_up_invited", participant_id) is not None, "follow-up close precedes invitation")
            require(follow_up_event(prefix, "follow_up_closed", participant_id) is None, "duplicate follow-up close")
            observed_use = follow_up_event(prefix, "follow_up_used", participant_id) is not None
            require(payload["used_within_28_days"] == observed_use, "follow-up close use flag mismatch")
            invited = follow_up_event(prefix, "follow_up_invited", participant_id)["payload"]
            require(payload["wall_closed_ns"] >= invited["wall_deadline_ns"], "follow-up closed before 28 days")
            if payload["boot_id_hash"] == invited["boot_id_hash"]:
                require(
                    payload["monotonic_closed_ns"] - invited["monotonic_invited_ns"] >= FOLLOW_UP_NS,
                    "follow-up monotonic close is shorter than 28 days",
                )
            require_event_key(event, participant_id)
        else:
            require(kind == "collection_locked", f"unhandled event kind: {kind}")
            require_event_key(event, plan["study_id"])
            locked = True
        prefix.append(event)


def participant_pair_complete(events, pair_id):
    return all(session_event(events, "session_completed", pair_id, arm) is not None for arm in ("baseline", "resume"))


def follow_up_event(events, kind, participant_id):
    return latest_event(events, kind, lambda payload: payload["participant_id"] == participant_id)


def follow_up_receipt(plan, invite):
    return {
        "schema": FOLLOW_UP_SCHEMA,
        "study_id": plan["study_id"],
        "participant_id": invite["participant_id"],
        "generation": invite["generation"],
        "token": invite["token"],
    }


def read_follow_up(path, plan, events):
    _, receipt = read_json(path.resolve())
    require_object(receipt, {"schema", "study_id", "participant_id", "generation", "token"}, "follow-up receipt")
    require(receipt["schema"] == FOLLOW_UP_SCHEMA, "unsupported follow-up receipt schema")
    invite = {
        "schema": INVITE_SCHEMA,
        "study_id": receipt["study_id"],
        "participant_id": receipt["participant_id"],
        "generation": receipt["generation"],
        "token": receipt["token"],
    }
    require(invite["study_id"] == plan["study_id"], "follow-up receipt belongs to another study")
    credential = participant_credential_event(events, invite["participant_id"])
    require(credential is not None, "follow-up participant has not been activated")
    require(credential["payload"]["generation"] == invite["generation"], "follow-up credential was superseded")
    require(
        hmac.compare_digest(credential["payload"]["credential_sha256"], sha256(bytes.fromhex(invite["token"]))),
        "follow-up credential is invalid",
    )
    return receipt


def follow_up_invite(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, attestation, _, events, chain_tip = load_context(state_dir)
        require_operator_key(arguments.operator_key, attestation)
        require(not collection_locked(events), "collection is already locked")
        invite = read_invite(arguments.invite, plan, events)
        participant_id = invite["participant_id"]
        complete_pairs = [
            assignment
            for assignment in assignments_for_participant(plan, participant_id)
            if participant_pair_complete(events, assignment["pair_id"])
        ]
        require(complete_pairs, "follow-up requires at least one completed pair")
        require(follow_up_event(events, "follow_up_invited", participant_id) is None, "follow-up was already invited")
        clock = current_clock()
        payload = {
            "participant_id": participant_id,
            "boot_id_hash": clock["boot_id_hash"],
            "monotonic_invited_ns": clock["monotonic_ns"],
            "wall_invited_ns": clock["wall_ns"],
            "wall_deadline_ns": clock["wall_ns"] + FOLLOW_UP_NS,
        }
        append_event(state_dir, plan, events, chain_tip, "follow_up_invited", participant_id, payload)
        write_receipt(arguments.receipt_out, follow_up_receipt(plan, invite))
        print(f"opened one {FOLLOW_UP_DAYS}-day follow-up window for {participant_id}")


def follow_up_window(events, participant_id, clock):
    invited = follow_up_event(events, "follow_up_invited", participant_id)
    require(invited is not None, "participant has no follow-up invitation")
    payload = invited["payload"]
    require(clock["wall_ns"] >= payload["wall_invited_ns"], "system wall clock moved before follow-up invitation")
    if clock["boot_id_hash"] == payload["boot_id_hash"]:
        require(clock["monotonic_ns"] >= payload["monotonic_invited_ns"], "monotonic follow-up clock moved backward")
    return invited


def follow_up_run(arguments):
    state_dir = arguments.state_dir.resolve()
    process = None
    ready_line = None
    with workspace_lock(state_dir):
        _, plan, task_spec, _, _, _, events, chain_tip = load_context(state_dir)
        receipt = read_follow_up(arguments.follow_up, plan, events)
        participant_id = receipt["participant_id"]
        require(follow_up_event(events, "follow_up_closed", participant_id) is None, "follow-up window is closed")
        invited = follow_up_window(events, participant_id, current_clock())
        require(time.time_ns() <= invited["payload"]["wall_deadline_ns"], "follow-up use occurred after the frozen window")
        existing = follow_up_event(events, "follow_up_used", participant_id)
        if existing is not None:
            print(f"follow-up Resume use already recorded for {participant_id}")
            return
        command = [task_spec["stratadiff_binary_path"], "resume", *arguments.resume_arguments]
        process = subprocess.Popen(
            command,
            stdin=None,
            stdout=subprocess.PIPE,
            stderr=None,
            text=True,
            bufsize=1,
        )
        require(process.stdout is not None, "follow-up Resume stdout pipe is unavailable")
        while True:
            line = process.stdout.readline()
            if line == "":
                status = process.poll()
                require(status is None, f"native Resume exited before Workbench readiness with {status}")
                continue
            print(line, end="", flush=True)
            if line.startswith("StrataDiff Review Resume Workbench: http://"):
                ready_line = line
                break
        used_clock = current_clock()
        payload = {
            "participant_id": participant_id,
            "used": True,
            "boot_id_hash": used_clock["boot_id_hash"],
            "monotonic_used_ns": used_clock["monotonic_ns"],
            "wall_used_ns": used_clock["wall_ns"],
        }
        append_event(state_dir, plan, events, chain_tip, "follow_up_used", participant_id, payload)
        print(f"recorded authenticated native Resume reuse for {participant_id}")
    require(process is not None and ready_line is not None, "follow-up Resume did not become ready")
    require(process.stdout is not None, "follow-up Resume stdout pipe is unavailable")
    try:
        for line in process.stdout:
            print(line, end="", flush=True)
        status = process.wait()
        require(status == 0, f"native Resume exited with {status}")
    except KeyboardInterrupt:
        process.send_signal(2)
        process.wait(timeout=10)


def follow_up_close_at(state_dir, plan, events, chain_tip, participant_id, clock):
    invited = follow_up_window(events, participant_id, clock)
    payload = invited["payload"]
    require(clock["wall_ns"] >= payload["wall_deadline_ns"], f"{FOLLOW_UP_DAYS}-day follow-up window is not complete")
    if clock["boot_id_hash"] == payload["boot_id_hash"]:
        require(
            clock["monotonic_ns"] - payload["monotonic_invited_ns"] >= FOLLOW_UP_NS,
            "monotonic follow-up duration is shorter than 28 days",
        )
    used = follow_up_event(events, "follow_up_used", participant_id) is not None
    close_payload = {
        "participant_id": participant_id,
        "used_within_28_days": used,
        "boot_id_hash": clock["boot_id_hash"],
        "monotonic_closed_ns": clock["monotonic_ns"],
        "wall_closed_ns": clock["wall_ns"],
    }
    return append_event(state_dir, plan, events, chain_tip, "follow_up_closed", participant_id, close_payload)


def follow_up_close(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, attestation, _, events, chain_tip = load_context(state_dir)
        require_operator_key(arguments.operator_key, attestation)
        receipt = read_follow_up(arguments.follow_up, plan, events)
        participant_id = receipt["participant_id"]
        follow_up_close_at(state_dir, plan, events, chain_tip, participant_id, current_clock())
        print(f"closed completed follow-up window for {participant_id}")


def activated_participants(events):
    return [event["payload"]["participant_id"] for event in events_of_kind(events, "participant_activated")]


def completed_pair_observation(plan, events, assignment):
    baseline = session_event(events, "session_completed", assignment["pair_id"], "baseline")
    resume = session_event(events, "session_completed", assignment["pair_id"], "resume")
    require(baseline is not None and resume is not None, f"pair is incomplete: {assignment['pair_id']}")
    units = [
        unit
        for unit in planned_units(plan)
        if unit["pair_id"] == assignment["pair_id"]
    ]
    outcomes = [unit_outcome(events, unit) for unit in units]
    require(all(outcome is not None for outcome in outcomes), f"pair adjudication is incomplete: {assignment['pair_id']}")
    carried_units = [unit for unit in units if unit["counts_as_carry"]]
    false_carries = sum(
        1 for unit, outcome in zip(units, outcomes, strict=True) if unit["counts_as_carry"] and outcome == "false_carry"
    )
    reviewer_counts = []
    for unit in units:
        reveals = [
            event
            for event in events_of_kind(events, "adjudication_revealed")
            if event["payload"]["pair_id"] == unit["pair_id"]
            and event["payload"]["unit_id"] == unit["unit_id"]
        ]
        reviewer_counts.append(len({event["payload"]["key_id"] for event in reveals}))
    require(min(reviewer_counts) >= 2, f"pair lacks two independent adjudicators: {assignment['pair_id']}")
    return {
        "pair_id": assignment["pair_id"],
        "participant_id": assignment["participant_id"],
        "task_family_id": assignment["task_family_id"],
        "assignment_order": assignment["assignment_order"],
        "baseline_variant": assignment["baseline_variant"],
        "resume_variant": assignment["resume_variant"],
        "baseline": {
            key: baseline["payload"][key]
            for key in ("completion_seconds", "issues_found", "seeded_issues", "reopened_files", "reopened_lines")
        },
        "resume": {
            key: resume["payload"][key]
            for key in ("completion_seconds", "issues_found", "seeded_issues", "reopened_files", "reopened_lines")
        },
        "false_carry_adjudication": {
            "unit": "carried_file_change",
            "carried_units": len(carried_units),
            "adjudicated_units": len(carried_units),
            "confirmed_false_carries": false_carries,
            "adjudicator_count": min(reviewer_counts),
            "all_disagreements_resolved": True,
        },
    }


def build_dataset(plan, events):
    participants = activated_participants(events)
    require(participants, "no participant slots were activated")
    require(len(participants) % 4 == 0, "activated participant slots must close in cohorts of four")
    require(not events_of_kind(events, "participant_withdrawn"), "post-start attrition prevents collection lock")
    observations = []
    participant_records = []
    for participant_id in participants:
        assignments = assignments_for_participant(plan, participant_id)
        require(assignments, f"activated participant has no assignment: {participant_id}")
        for assignment in assignments:
            observations.append(completed_pair_observation(plan, events, assignment))
        invited = follow_up_event(events, "follow_up_invited", participant_id)
        closed = follow_up_event(events, "follow_up_closed", participant_id)
        require(invited is not None, f"participant follow-up was not invited: {participant_id}")
        require(closed is not None, f"participant follow-up is not complete: {participant_id}")
        participant_records.append(
            {
                "participant_id": participant_id,
                "repeat_use": {
                    "invited_again": True,
                    "follow_up_complete": True,
                    "used_within_28_days": closed["payload"]["used_within_28_days"],
                },
            }
        )
    return {
        "schema": DATA_SCHEMA,
        "study_id": plan["study_id"],
        "protocol_version": plan["protocol_version"],
        "preregistration_sha256": plan["preregistration_sha256"],
        "synthetic": plan["synthetic"],
        "collection_status": "locked",
        "participants": participant_records,
        "paired_observations": observations,
    }


def evaluator_aggregate(dataset_path, aggregate_path, preregistration_path, synthetic):
    command = [
        sys.executable,
        str(EVALUATOR),
        "--preregistration",
        str(preregistration_path),
        "aggregate",
        "--input",
        str(dataset_path),
        "--output",
        str(aggregate_path),
    ]
    if synthetic:
        command.append("--allow-synthetic")
    process = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    require(
        process.returncode == 0,
        f"frozen evaluator rejected collection lock: {process.stderr.decode('utf-8', errors='replace').strip()}",
    )


def require_external_output(path, state_dir, label):
    resolved = path.resolve()
    require(not resolved.is_relative_to(state_dir.resolve()), f"{label} must be outside the private state directory")
    return resolved


def lock_collection(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        paths, plan, _, _, attestation, _, events, chain_tip = load_context(
            state_dir, verify_event_signatures=True
        )
        require_operator_key(arguments.operator_key, attestation)
        output = require_external_output(arguments.output, state_dir, "study dataset output")
        aggregate_output = require_external_output(arguments.aggregate_output, state_dir, "aggregate output")
        require(output != aggregate_output, "study dataset and aggregate outputs must differ")
        dataset = build_dataset(plan, events)
        dataset_bytes = encoded(dataset)
        output.parent.mkdir(parents=True, exist_ok=True)
        aggregate_output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="stratadiff-pilot-lock-") as temporary:
            dataset_path = Path(temporary) / "study-data.json"
            aggregate_path = Path(temporary) / "aggregate.json"
            dataset_path.write_bytes(dataset_bytes)
            evaluator_aggregate(dataset_path, aggregate_path, paths["preregistration"], plan["synthetic"])
            aggregate_bytes = aggregate_path.read_bytes()
        locked = events_of_kind(events, "collection_locked")
        lock_payload = {
            "dataset_sha256": sha256(dataset_bytes),
            "aggregate_sha256": sha256(aggregate_bytes),
        }
        if locked:
            require(len(locked) == 1 and locked[0]["payload"] == lock_payload, "collection lock differs from recomputation")
        else:
            append_event(state_dir, plan, events, chain_tip, "collection_locked", plan["study_id"], lock_payload)
        atomic_write(output, dataset_bytes)
        atomic_write(aggregate_output, aggregate_bytes)
        print(f"locked {len(dataset['paired_observations'])} paired observations into privacy-minimized output")


def flow_counts(plan, events):
    activated = activated_participants(events)
    complete_pairs = sum(
        participant_pair_complete(events, assignment["pair_id"])
        for assignment in plan["assignments"]
        if assignment["participant_id"] in activated
    )
    planned_for_activated = sum(
        assignment["participant_id"] in activated for assignment in plan["assignments"]
    )
    return {
        "planned_participant_slots": len(plan["participants"]),
        "activated_participants": len(activated),
        "planned_pairs_for_activated": planned_for_activated,
        "complete_pairs": complete_pairs,
        "incomplete_pairs": planned_for_activated - complete_pairs,
        "interrupted_arms": len(events_of_kind(events, "session_interrupted")),
        "pre_start_invite_replacements": len(events_of_kind(events, "participant_invite_replaced")),
        "post_start_withdrawals": len(events_of_kind(events, "participant_withdrawn")),
    }


def attest_final(arguments):
    state_dir = arguments.state_dir.resolve()
    with workspace_lock(state_dir):
        _, plan, _, _, plan_attestation, _, events, chain_tip = load_context(
            state_dir, verify_event_signatures=True
        )
        require_operator_key(arguments.operator_key, plan_attestation)
        output = require_external_output(arguments.output, state_dir, "final attestation output")
        require(arguments.consent_obtained, "--consent-obtained is required for operator attestation")
        require(arguments.provider_authorized, "--provider-authorized is required for operator attestation")
        require(arguments.linkage_key_not_exported, "--linkage-key-not-exported is required for operator attestation")
        dataset_bytes, dataset = read_json(arguments.dataset.resolve())
        aggregate_bytes, aggregate = read_json(arguments.aggregate.resolve())
        locked = events_of_kind(events, "collection_locked")
        require(len(locked) == 1, "collection must be locked before final attestation")
        require(locked[0]["payload"]["dataset_sha256"] == sha256(dataset_bytes), "dataset differs from locked bytes")
        require(locked[0]["payload"]["aggregate_sha256"] == sha256(aggregate_bytes), "aggregate differs from locked bytes")
        require(dataset["study_id"] == plan["study_id"] and aggregate["study_id"] == plan["study_id"], "final artifacts study mismatch")
        public_key = normalized_public_key(arguments.operator_key.resolve())
        attestation = {
            "schema": FINAL_ATTESTATION_SCHEMA,
            "kind": "final",
            "study_id": plan["study_id"],
            "operator_key_id": plan_attestation["operator_key_id"],
            "preregistration_sha256": plan["preregistration_sha256"],
            "plan_sha256": sha256(encoded(plan)),
            "plan_anchor_sha256": plan_attestation["plan_anchor_sha256"],
            "task_catalog_sha256": plan["task_catalog_sha256"],
            "event_chain_tip_sha256": chain_tip,
            "dataset_sha256": sha256(dataset_bytes),
            "aggregate_sha256": sha256(aggregate_bytes),
            "tool_source_sha256": file_sha256(PILOT_SOURCE),
            "flow_counts": flow_counts(plan, events),
            "claims": {
                "assignment_frozen_before_collection": True,
                "all_complete_pairs_included": True,
                "consent_obtained": True,
                "provider_authorized": True,
                "linkage_key_not_exported": True,
                "all_carries_blindly_adjudicated": True,
                "follow_up_complete": True,
            },
            "signature_algorithm": "openssh-ed25519-sshsig",
        }
        signature = sign_bytes(arguments.operator_key.resolve(), encoded(attestation))
        validate_final_attestation(attestation)
        verify_public_privacy(attestation, "final attestation")
        verify_signature(public_key, attestation["operator_key_id"], encoded(attestation), signature)
        atomic_write(output, encoded(attestation))
        signature_output = Path(f"{output}.sig")
        atomic_write(signature_output, signature)
        print(f"wrote signed final operator attestation to {output}")


def validate_final_attestation(attestation):
    require_object(
        attestation,
        {
            "schema",
            "kind",
            "study_id",
            "operator_key_id",
            "preregistration_sha256",
            "plan_sha256",
            "plan_anchor_sha256",
            "task_catalog_sha256",
            "event_chain_tip_sha256",
            "dataset_sha256",
            "aggregate_sha256",
            "tool_source_sha256",
            "flow_counts",
            "claims",
            "signature_algorithm",
        },
        "final attestation",
    )
    require(attestation["schema"] == FINAL_ATTESTATION_SCHEMA, "unsupported final attestation schema")
    require(attestation["kind"] == "final", "final attestation has wrong kind")
    require_identifier(attestation["study_id"], STUDY_ID, "final attestation.study_id")
    require_identifier(attestation["operator_key_id"], OPERATOR_KEY_ID, "final attestation.operator_key_id")
    for field in (
        "preregistration_sha256",
        "plan_sha256",
        "plan_anchor_sha256",
        "task_catalog_sha256",
        "event_chain_tip_sha256",
        "dataset_sha256",
        "aggregate_sha256",
        "tool_source_sha256",
    ):
        require_sha256(attestation[field], f"final attestation.{field}")
    require_object(
        attestation["flow_counts"],
        {
            "planned_participant_slots",
            "activated_participants",
            "planned_pairs_for_activated",
            "complete_pairs",
            "incomplete_pairs",
            "interrupted_arms",
            "pre_start_invite_replacements",
            "post_start_withdrawals",
        },
        "final attestation.flow_counts",
    )
    for key, value in attestation["flow_counts"].items():
        require_integer(value, 0, 1000000, f"final attestation.flow_counts.{key}")
    require_object(
        attestation["claims"],
        {
            "assignment_frozen_before_collection",
            "all_complete_pairs_included",
            "consent_obtained",
            "provider_authorized",
            "linkage_key_not_exported",
            "all_carries_blindly_adjudicated",
            "follow_up_complete",
        },
        "final attestation.claims",
    )
    for key, value in attestation["claims"].items():
        require(value is True, f"final attestation claim is not true: {key}")
    require(attestation["signature_algorithm"] == "openssh-ed25519-sshsig", "unsupported final signature algorithm")


def verify_public_privacy(value, label):
    prohibited_keys = ("path", "code", "comment", "login", "url", "timestamp", "token", "free_text")
    if type(value) is dict:
        for key, item in value.items():
            lowered = key.lower()
            require(not any(fragment in lowered for fragment in prohibited_keys), f"{label} has prohibited field: {key}")
            verify_public_privacy(item, f"{label}.{key}")
    elif type(value) is list:
        for index, item in enumerate(value):
            verify_public_privacy(item, f"{label}[{index}]")
    elif type(value) is str:
        if value == DATA_SCHEMA:
            return
        require(re.search(r"[A-Za-z][A-Za-z0-9+.-]*://", value) is None, f"{label} contains a prohibited URI")
        require(not value.startswith("/"), f"{label} contains an absolute path")


def validate_public_dataset_bindings(plan, task_catalog, dataset, final_attestation):
    require(dataset["study_id"] == plan["study_id"], "dataset study mismatch")
    require(dataset["protocol_version"] == plan["protocol_version"], "dataset protocol mismatch")
    require(
        dataset["preregistration_sha256"] == plan["preregistration_sha256"],
        "dataset preregistration binding mismatch",
    )
    require(dataset["synthetic"] is plan["synthetic"], "dataset synthetic status differs from plan")

    participant_ids = [participant["participant_id"] for participant in dataset["participants"]]
    require(
        participant_ids == plan["participants"][: len(participant_ids)],
        "dataset participants are not the frozen activation prefix",
    )
    expected_assignments = []
    for participant_id in participant_ids:
        expected_assignments.extend(assignments_for_participant(plan, participant_id))
    observations = dataset["paired_observations"]
    require(len(observations) == len(expected_assignments), "dataset does not cover every frozen assignment")
    families = task_family_map(task_catalog)
    identity_fields = (
        "pair_id",
        "participant_id",
        "task_family_id",
        "assignment_order",
        "baseline_variant",
        "resume_variant",
    )
    for index, (observation, assignment) in enumerate(zip(observations, expected_assignments, strict=True)):
        label = f"study dataset.paired_observations[{index}]"
        for field in identity_fields:
            require(observation[field] == assignment[field], f"{label}.{field} differs from frozen plan")
        family = families[assignment["task_family_id"]]
        for arm in ("baseline", "resume"):
            variant_name = assignment[f"{arm}_variant"]
            variant = family["variants"][variant_name]
            presentation = variant["presentations"][arm]
            measurement = observation[arm]
            require(
                measurement["seeded_issues"] == len(variant["seeded_issue_ids"]),
                f"{label}.{arm}.seeded_issues differs from task catalog",
            )
            require(
                measurement["reopened_files"] <= len(presentation["reopened_file_ids"]),
                f"{label}.{arm}.reopened_files exceeds task catalog",
            )
            require(
                measurement["reopened_lines"] <= len(presentation["reopened_line_ids"]),
                f"{label}.{arm}.reopened_lines exceeds task catalog",
            )
        carried_units = sum(unit["counts_as_carry"] for unit in assignment["adjudication"])
        adjudication = observation["false_carry_adjudication"]
        require(adjudication["carried_units"] == carried_units, f"{label} carry count differs from frozen plan")
        require(
            adjudication["adjudicated_units"] == carried_units,
            f"{label} adjudicated carry count differs from frozen plan",
        )

    counts = final_attestation["flow_counts"]
    expected_counts = {
        "planned_participant_slots": len(plan["participants"]),
        "activated_participants": len(participant_ids),
        "planned_pairs_for_activated": len(expected_assignments),
        "complete_pairs": len(observations),
        "incomplete_pairs": 0,
        "interrupted_arms": 0,
        "post_start_withdrawals": 0,
    }
    for field, expected in expected_counts.items():
        require(counts[field] == expected, f"final attestation {field} differs from public artifacts")


def verify_export(arguments):
    plan_bytes, plan = read_json(arguments.plan.resolve())
    preregistration_bytes, preregistration = read_json(arguments.preregistration.resolve())
    catalog_bytes, task_catalog = read_json(arguments.task_catalog.resolve())
    plan_attestation_bytes, plan_attestation = read_json(arguments.plan_attestation.resolve())
    final_attestation_bytes, final_attestation = read_json(arguments.final_attestation.resolve())
    dataset_bytes, dataset = read_json(arguments.dataset.resolve())
    aggregate_bytes, aggregate = read_json(arguments.aggregate.resolve())
    validate_plan(plan)
    validate_task_catalog(task_catalog)
    validate_plan_randomization(plan, task_catalog, preregistration_bytes, preregistration)
    validate_plan_attestation(plan_attestation, plan)
    validate_final_attestation(final_attestation)
    public_key = normalized_public_key_file(arguments.operator_public_key.resolve(), canonical=True)
    require(public_key_id(public_key, "operator") == plan_attestation["operator_key_id"], "operator public key ID mismatch")
    verify_signature(
        public_key,
        plan_attestation["operator_key_id"],
        plan_attestation_bytes,
        read_limited_bytes(arguments.plan_signature, MAX_SIGNATURE_BYTES, "plan signature"),
    )
    verify_signature(
        public_key,
        final_attestation["operator_key_id"],
        final_attestation_bytes,
        read_limited_bytes(arguments.final_signature, MAX_SIGNATURE_BYTES, "final signature"),
    )
    require(plan["task_catalog_sha256"] == sha256(catalog_bytes), "published task catalog hash mismatch")
    require(plan["preregistration_sha256"] == sha256(preregistration_bytes), "published preregistration hash mismatch")
    require(final_attestation["study_id"] == plan["study_id"], "final attestation study mismatch")
    require(final_attestation["operator_key_id"] == plan_attestation["operator_key_id"], "operator key changed")
    require(
        final_attestation["preregistration_sha256"] == plan["preregistration_sha256"],
        "final preregistration binding mismatch",
    )
    require(final_attestation["plan_sha256"] == sha256(plan_bytes), "final plan hash mismatch")
    require(final_attestation["plan_anchor_sha256"] == plan_attestation["plan_anchor_sha256"], "plan anchor changed")
    require(final_attestation["task_catalog_sha256"] == sha256(catalog_bytes), "final catalog hash mismatch")
    require(final_attestation["dataset_sha256"] == sha256(dataset_bytes), "final dataset hash mismatch")
    require(final_attestation["aggregate_sha256"] == sha256(aggregate_bytes), "final aggregate hash mismatch")
    require(final_attestation["tool_source_sha256"] == plan["pilot_source_sha256"], "final tool binding mismatch")
    require(dataset["study_id"] == plan["study_id"] and aggregate["study_id"] == plan["study_id"], "artifact study mismatch")
    require(plan_bytes == encoded(plan), "pilot plan JSON is not canonical")
    require(catalog_bytes == encoded(task_catalog), "task catalog JSON is not canonical")
    require(plan_attestation_bytes == encoded(plan_attestation), "plan attestation JSON is not canonical")
    require(final_attestation_bytes == encoded(final_attestation), "final attestation JSON is not canonical")
    require(dataset_bytes == encoded(dataset), "study dataset JSON is not canonical")
    validate_public_dataset_bindings(plan, task_catalog, dataset, final_attestation)
    for label, value in (
        ("pilot plan", plan),
        ("preregistration", preregistration),
        ("task catalog", task_catalog),
        ("plan attestation", plan_attestation),
        ("study dataset", dataset),
        ("aggregate", aggregate),
        ("final attestation", final_attestation),
    ):
        verify_public_privacy(value, label)
    command = [
        sys.executable,
        str(EVALUATOR),
        "--preregistration",
        str(arguments.preregistration.resolve()),
        "verify",
        "--input",
        str(arguments.dataset.resolve()),
        "--aggregate",
        str(arguments.aggregate.resolve()),
    ]
    if dataset["synthetic"]:
        command.append("--allow-synthetic")
    process = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    require(process.returncode == 0, f"frozen evaluator verification failed: {process.stderr.decode().strip()}")
    print(
        f"verified signed reviewer-pilot export: {len(dataset['paired_observations'])} pairs, "
        f"synthetic={str(dataset['synthetic']).lower()}"
    )


def require_rejected(action, message_fragment):
    try:
        action()
    except (ValueError, FileNotFoundError, json.JSONDecodeError) as error:
        require(
            message_fragment in str(error),
            f"self-test rejection mismatch: expected {message_fragment!r}, observed {str(error)!r}",
        )
        return
    raise ValueError(f"self-test expected rejection containing: {message_fragment}")


def generate_test_key(path):
    process = subprocess.run(
        ["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "", "-f", str(path)],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    require(process.returncode == 0, f"self-test key generation failed: {process.stderr.decode().strip()}")


def self_test_task_spec(root):
    families = []
    family_id = "task_000000000001"
    variants = {}
    for variant_index, variant_name in enumerate(("a", "b"), start=1):
        issue_correct = f"issue_{variant_index:016x}"
        issue_decoy = f"issue_{variant_index + 10:016x}"
        presentations = {}
        for arm_index, arm in enumerate(("baseline", "resume"), start=1):
            bundle = root / f"bundle-{variant_name}-{arm}"
            bundle.mkdir()
            file_id = f"file_{variant_index * 10 + arm_index:016x}"
            line_id = f"line_{variant_index * 10 + arm_index:016x}"
            carry_ids = [f"carry_{variant_index:016x}"] if arm == "resume" else []
            result = {
                "schema": SESSION_RESULT_SCHEMA,
                "submitted_issue_ids": [issue_correct],
                "reopened_file_ids": [file_id] if arm == "baseline" else [],
                "reopened_line_ids": [line_id] if arm == "baseline" else [],
            }
            (bundle / "result-template.json").write_bytes(encoded(result))
            (bundle / "preflight.py").write_text(
                "from pathlib import Path\nassert Path('result-template.json').is_file()\n",
                encoding="utf-8",
            )
            (bundle / "run.py").write_text(
                "import shutil, sys\n"
                "from pathlib import Path\n"
                "result = Path(sys.argv[1])\n"
                "request = Path(f'{result}.fail-once-request')\n"
                "attempted = Path(f'{result}.attempted')\n"
                "if request.exists() and not attempted.exists():\n"
                "    attempted.write_text('attempted')\n"
                "    raise SystemExit(9)\n"
                "shutil.copyfile('result-template.json', result)\n",
                encoding="utf-8",
            )
            presentations[arm] = {
                "bundle_path": str(bundle),
                "preflight_command": [sys.executable, "preflight.py"],
                "run_command": [sys.executable, "run.py", "{result}"],
                "reopened_file_ids": [file_id],
                "reopened_line_ids": [line_id],
                "carried_unit_ids": carry_ids,
            }
        variants[variant_name] = {
            "response_issue_ids": [issue_correct, issue_decoy],
            "seeded_issue_ids": [issue_correct],
            "presentations": presentations,
        }
    families.append({"task_family_id": family_id, "variants": variants})
    task_spec = {
        "schema": TASK_SPEC_SCHEMA,
        "stratadiff_binary_path": str(Path("/bin/true").resolve()),
        "task_families": families,
    }
    path = root / "task-spec.json"
    path.write_bytes(encoded(task_spec))
    return path


def run_self_test():
    with tempfile.TemporaryDirectory(prefix="stratadiff-reviewer-pilot-") as temporary:
        root = Path(temporary)
        task_spec = self_test_task_spec(root)
        _, preregistration = read_json(PREREGISTRATION)
        preregistration["minimums"]["eligible_pairs"] = 4
        preregistration["minimums"]["unique_participants"] = 4
        test_preregistration = root / "preregistration.json"
        test_preregistration.write_bytes(encoded(preregistration))
        seed = root / "seed"
        seed.write_text("42" * 32 + "\n", encoding="utf-8")
        operator_key = root / "operator"
        generate_test_key(operator_key)
        state = root / "state"
        replay_state = root / "replay-state"
        create_arguments = argparse.Namespace(
            state_dir=state,
            task_spec=task_spec,
            participant_slots=4,
            adjudicator_slots=3,
            seed_file=seed,
            synthetic=True,
            preregistration=test_preregistration,
        )
        plan_create(create_arguments)
        plan_create(
            argparse.Namespace(
                state_dir=replay_state,
                task_spec=task_spec,
                participant_slots=create_arguments.participant_slots,
                adjudicator_slots=create_arguments.adjudicator_slots,
                seed_file=seed,
                synthetic=True,
                preregistration=test_preregistration,
            )
        )
        require(
            workspace_paths(state)["plan"].read_bytes() == workspace_paths(replay_state)["plan"].read_bytes(),
            "same randomization seed did not reproduce identical plan bytes",
        )
        plan_attest(
            argparse.Namespace(
                state_dir=state,
                operator_key=operator_key,
                anchor_sha256="a" * 64,
            )
        )
        invite_paths = []
        for participant_index in range(4):
            invite_path = root / f"invite-{participant_index}.json"
            enroll(
                argparse.Namespace(
                    state_dir=state,
                    operator_key=operator_key,
                    receipt_out=invite_path,
                )
            )
            invite_paths.append(invite_path)
            for arm_index in range(2):
                session_preflight(argparse.Namespace(state_dir=state, invite=invite_path))
                if participant_index == 0 and arm_index == 0:
                    with workspace_lock(state):
                        paths, plan, _, _, _, _, events, _ = load_context(state)
                        invite = read_invite(invite_path, plan, events)
                        assignment, arm = current_session(plan, events, invite["participant_id"])
                        result_path = paths["results"] / f"{assignment['pair_id']}.{arm}.json"
                        Path(f"{result_path}.fail-once-request").write_text("fail once", encoding="utf-8")
                    require_rejected(
                        lambda: session_run(
                            argparse.Namespace(
                                state_dir=state,
                                invite=invite_path,
                                yes=True,
                                no_open=True,
                            )
                        ),
                        "runner exited with 9",
                    )
                    with workspace_lock(state):
                        _, plan, _, _, _, _, events, _ = load_context(state)
                        starts = events_of_kind(events, "session_started")
                        require(len(starts) == 1, "failed session must preserve exactly one monotonic start")
                session_run(
                    argparse.Namespace(
                        state_dir=state,
                        invite=invite_path,
                        yes=True,
                        no_open=True,
                    )
                )
                if participant_index == 0 and arm_index == 0:
                    with workspace_lock(state):
                        _, _, _, _, _, _, events, _ = load_context(state)
                        require(
                            len(events_of_kind(events, "session_started")) == 1,
                            "resumed session created a duplicate start event",
                        )
        slot_to_key = {}
        for index in range(3):
            key = root / f"adjudicator-{index}"
            receipt_path = root / f"adjudicator-{index}.json"
            generate_test_key(key)
            adjudicator_register(
                argparse.Namespace(
                    state_dir=state,
                    operator_key=operator_key,
                    public_key=Path(f"{key}.pub"),
                    receipt_out=receipt_path,
                )
            )
            _, registration = read_json(receipt_path)
            slot_to_key[registration["slot_id"]] = key
        _, plan = read_json(workspace_paths(state)["plan"])
        units = planned_units(plan)
        for unit_index, unit in enumerate(units):
            reveal_paths = []
            for reviewer_index, slot_id in enumerate(unit["initial_slots"]):
                key = slot_to_key[slot_id]
                assignment_path = root / f"assignment-{unit_index}-{reviewer_index}.json"
                reveal_path = root / f"reveal-{unit_index}-{reviewer_index}.json"
                adjudication_assign(
                    argparse.Namespace(state_dir=state, adjudicator_key=key, receipt_out=assignment_path)
                )
                decision = "false_carry" if unit_index == 0 and reviewer_index == 1 else "valid_carry"
                adjudication_commit(
                    argparse.Namespace(
                        state_dir=state,
                        assignment=assignment_path,
                        adjudicator_key=key,
                        decision=decision,
                        reveal_out=reveal_path,
                    )
                )
                reveal_paths.append(reveal_path)
                if reviewer_index == 0:
                    require_rejected(
                        lambda: adjudication_reveal(argparse.Namespace(state_dir=state, reveal=reveal_path)),
                        "both independent initial commitments",
                    )
            for reveal_path in reveal_paths:
                adjudication_reveal(argparse.Namespace(state_dir=state, reveal=reveal_path))
            if unit_index == 0:
                resolver_key = slot_to_key[unit["resolver_slot"]]
                resolver_assignment = root / "resolver-assignment.json"
                resolver_reveal = root / "resolver-reveal.json"
                adjudication_assign(
                    argparse.Namespace(
                        state_dir=state,
                        adjudicator_key=resolver_key,
                        receipt_out=resolver_assignment,
                    )
                )
                adjudication_commit(
                    argparse.Namespace(
                        state_dir=state,
                        assignment=resolver_assignment,
                        adjudicator_key=resolver_key,
                        decision="valid_carry",
                        reveal_out=resolver_reveal,
                    )
                )
                adjudication_reveal(argparse.Namespace(state_dir=state, reveal=resolver_reveal))
        for participant_index, invite_path in enumerate(invite_paths):
            follow_up_path = root / f"follow-up-{participant_index}.json"
            follow_up_invite(
                argparse.Namespace(
                    state_dir=state,
                    operator_key=operator_key,
                    invite=invite_path,
                    receipt_out=follow_up_path,
                )
            )
            with workspace_lock(state):
                _, plan, _, _, _, _, events, chain_tip = load_context(state)
                _, follow_up = read_json(follow_up_path)
                invited = follow_up_event(events, "follow_up_invited", follow_up["participant_id"])
                if participant_index == 0:
                    require_rejected(
                        lambda: follow_up_close_at(
                            state,
                            plan,
                            events,
                            chain_tip,
                            follow_up["participant_id"],
                            {
                                "boot_id_hash": invited["payload"]["boot_id_hash"],
                                "monotonic_ns": invited["payload"]["monotonic_invited_ns"] + FOLLOW_UP_NS - 1,
                                "wall_ns": invited["payload"]["wall_deadline_ns"] - 1,
                            },
                        ),
                        "window is not complete",
                    )
                follow_up_close_at(
                    state,
                    plan,
                    events,
                    chain_tip,
                    follow_up["participant_id"],
                    {
                        "boot_id_hash": invited["payload"]["boot_id_hash"],
                        "monotonic_ns": invited["payload"]["monotonic_invited_ns"] + FOLLOW_UP_NS,
                        "wall_ns": invited["payload"]["wall_deadline_ns"],
                    },
                )
        dataset_path = root / "study-data.json"
        aggregate_path = root / "aggregate.json"
        lock_collection(
            argparse.Namespace(
                state_dir=state,
                operator_key=operator_key,
                output=dataset_path,
                aggregate_output=aggregate_path,
            )
        )
        final_attestation_path = root / "final-attestation.json"
        attest_final(
            argparse.Namespace(
                state_dir=state,
                operator_key=operator_key,
                dataset=dataset_path,
                aggregate=aggregate_path,
                output=final_attestation_path,
                consent_obtained=True,
                provider_authorized=True,
                linkage_key_not_exported=True,
            )
        )
        verify_arguments = argparse.Namespace(
            plan=workspace_paths(state)["plan"],
            preregistration=workspace_paths(state)["preregistration"],
            task_catalog=workspace_paths(state)["task_catalog"],
            plan_attestation=workspace_paths(state)["plan_attestation"],
            plan_signature=workspace_paths(state)["plan_signature"],
            operator_public_key=workspace_paths(state)["operator_public_key"],
            dataset=dataset_path,
            aggregate=aggregate_path,
            final_attestation=final_attestation_path,
            final_signature=Path(f"{final_attestation_path}.sig"),
        )
        verify_export(verify_arguments)
        _, plan = read_json(workspace_paths(state)["plan"])
        _, preregistration_value = read_json(workspace_paths(state)["preregistration"])
        _, catalog = read_json(workspace_paths(state)["task_catalog"])
        _, plan_attestation_value = read_json(workspace_paths(state)["plan_attestation"])
        _, dataset_value = read_json(dataset_path)
        _, final_attestation_value = read_json(final_attestation_path)
        verify_public_privacy(plan, "pilot plan")
        verify_public_privacy(preregistration_value, "preregistration")
        verify_public_privacy(catalog, "task catalog")
        verify_public_privacy(plan_attestation_value, "plan attestation")
        validate_public_dataset_bindings(plan, catalog, dataset_value, final_attestation_value)
        _, _, _, _, _, _, stable_events, _ = load_context(state, verify_event_signatures=True)
        for field, value, message in (
            ("study_id", "study_synthetic_ffffffffffffffff", "study_id mismatch"),
            ("context_sha256", "f" * 64, "context mismatch"),
        ):
            tampered_events = json.loads(json.dumps(stable_events))
            tampered_commit = next(
                event for event in tampered_events if event["kind"] == "adjudication_committed"
            )
            tampered_commit["payload"][field] = value
            tampered_statement = {
                key: item for key, item in tampered_commit["payload"].items() if key != "signature"
            }
            tampered_commit["payload"]["signature"] = base64.b64encode(
                sign_bytes(slot_to_key[tampered_commit["payload"]["slot_id"]], encoded(tampered_statement))
            ).decode("ascii")
            require_rejected(
                lambda events=tampered_events: validate_event_history(plan, catalog, events, True),
                message,
            )
        first_reveal = next(event for event in stable_events if event["kind"] == "adjudication_revealed")
        other_key_id = next(
            event["payload"]["key_id"]
            for event in stable_events
            if event["kind"] == "adjudicator_registered"
            and event["payload"]["key_id"] != first_reveal["payload"]["key_id"]
        )
        for field, value, message in (
            ("key_id", other_key_id, "key differs from commitment"),
            ("role", "resolver", "reveal role mismatch"),
        ):
            tampered_events = json.loads(json.dumps(stable_events))
            tampered_reveal = next(
                event for event in tampered_events if event["kind"] == "adjudication_revealed"
            )
            tampered_reveal["payload"][field] = value
            require_rejected(
                lambda events=tampered_events: validate_event_history(plan, catalog, events, True),
                message,
            )
        tampered_attestation = dict(plan_attestation_value)
        tampered_attestation["plan_anchor_sha256"] = "b" * 64
        public_key = workspace_paths(state)["operator_public_key"].read_text(encoding="utf-8").strip()
        require_rejected(
            lambda: verify_signature(
                public_key,
                plan_attestation_value["operator_key_id"],
                encoded(tampered_attestation),
                workspace_paths(state)["plan_signature"].read_bytes(),
            ),
            "signature verification failed",
        )
        require_rejected(
            lambda: verify_signature(
                public_key,
                plan_attestation_value["operator_key_id"],
                workspace_paths(state)["plan_attestation"].read_bytes(),
                workspace_paths(state)["plan_signature"].read_bytes() + b"alice@example.com\n",
            ),
            "invalid footer",
        )
        commented_public_key = root / "commented-operator.pub"
        commented_public_key.write_text(f"{public_key} alice@example.com\n", encoding="ascii")
        require_rejected(
            lambda: normalized_public_key_file(commented_public_key, canonical=True),
            "not canonical",
        )
        require_rejected(
            lambda: validate_preregistration_mode(test_preregistration.read_bytes(), preregistration_value, False),
            "canonical Reviewer Study v1 preregistration",
        )
        unsafe_preregistration = json.loads(json.dumps(preregistration_value))
        unsafe_preregistration["privacy"]["identifier_policy"] = "alice@example.com"
        require_rejected(
            lambda: validate_preregistration_mode(encoded(unsafe_preregistration), unsafe_preregistration, True),
            "may change only canonical integer minimums",
        )
        dataset_value["synthetic"] = False
        require_rejected(
            lambda: validate_public_dataset_bindings(plan, catalog, dataset_value, final_attestation_value),
            "synthetic status differs",
        )
        dataset_value["synthetic"] = True
        original_pair_id = dataset_value["paired_observations"][0]["pair_id"]
        dataset_value["paired_observations"][0]["pair_id"] = "pair_ffffffffffff"
        require_rejected(
            lambda: validate_public_dataset_bindings(plan, catalog, dataset_value, final_attestation_value),
            "differs from frozen plan",
        )
        dataset_value["paired_observations"][0]["pair_id"] = original_pair_id
        require_rejected(
            lambda: verify_public_privacy({"repository_path": "/private/source"}, "negative fixture"),
            "prohibited field",
        )
        require_rejected(
            lambda: verify_public_privacy({"policy": "/private/source"}, "negative fixture"),
            "absolute path",
        )
        tampered_dataset = root / "tampered-study-data.json"
        _, tampered_value = read_json(dataset_path)
        tampered_value["paired_observations"][0]["baseline"]["completion_seconds"] += 1
        tampered_dataset.write_bytes(encoded(tampered_value))
        tampered_arguments = argparse.Namespace(**vars(verify_arguments))
        tampered_arguments.dataset = tampered_dataset
        require_rejected(lambda: verify_export(tampered_arguments), "final dataset hash mismatch")
        first_event = sorted(workspace_paths(state)["events"].glob("*.json"))[0]
        original_event_bytes, tampered_event = read_json(first_event)
        tampered_event["payload"]["free_text"] = "forbidden"
        atomic_write(first_event, encoded(tampered_event))
        require_rejected(lambda: load_context(state), "unknown fields")
        atomic_write(first_event, original_event_bytes)
        load_context(state)
        evaluator = subprocess.run(
            [sys.executable, str(EVALUATOR), "self-test"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        require(evaluator.returncode == 0, f"frozen evaluator self-test failed: {evaluator.stderr.decode().strip()}")
    print("reviewer-pilot self-test passed")


def add_state_dir(parser):
    parser.add_argument("--state-dir", type=Path, required=True)


def parse_arguments():
    parser = argparse.ArgumentParser(
        description="Run a privacy-minimized, preregistered StrataDiff reviewer pilot"
    )
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("self-test", help="exercise the pilot trust and collection state machine")

    plan = commands.add_parser("plan", help="create, attest, or verify a frozen assignment plan")
    plan_commands = plan.add_subparsers(dest="plan_command", required=True)
    create = plan_commands.add_parser("create", help="create a deterministic counterbalanced plan")
    add_state_dir(create)
    create.add_argument("--task-spec", type=Path, required=True)
    create.add_argument("--preregistration", type=Path, default=PREREGISTRATION)
    create.add_argument("--participant-slots", type=int, default=20)
    create.add_argument("--adjudicator-slots", type=int, default=3)
    create.add_argument("--seed-file", type=Path)
    create.add_argument("--synthetic", action="store_true")
    attest = plan_commands.add_parser("attest", help="sign an externally anchored plan before enrollment")
    add_state_dir(attest)
    attest.add_argument("--operator-key", type=Path, required=True)
    attest.add_argument("--anchor-sha256", required=True)
    plan_check = plan_commands.add_parser("verify", help="verify plan signature and event-chain binding")
    add_state_dir(plan_check)

    enroll_parser = commands.add_parser("enroll", help="activate the next frozen participant slot")
    add_state_dir(enroll_parser)
    enroll_parser.add_argument("--operator-key", type=Path, required=True)
    enroll_parser.add_argument("--receipt-out", type=Path, required=True)

    attrition = commands.add_parser("attrition", help="handle pre-start replacement or post-start withdrawal")
    attrition_commands = attrition.add_subparsers(dest="attrition_command", required=True)
    replace = attrition_commands.add_parser("replace", help="rotate an unexposed participant invite")
    add_state_dir(replace)
    replace.add_argument("--operator-key", type=Path, required=True)
    replace.add_argument("--invite", type=Path, required=True)
    replace.add_argument("--reason", choices=PRE_START_REASONS, required=True)
    replace.add_argument("--receipt-out", type=Path, required=True)
    withdraw = attrition_commands.add_parser("withdraw", help="record irreversible post-start attrition")
    add_state_dir(withdraw)
    withdraw.add_argument("--invite", type=Path, required=True)
    withdraw.add_argument("--reason", choices=POST_START_REASONS, required=True)

    session = commands.add_parser("session", help="preflight, run, or inspect an authenticated session")
    session_commands = session.add_subparsers(dest="session_command", required=True)
    preflight = session_commands.add_parser("preflight", help="materialize and verify content before timing")
    add_state_dir(preflight)
    preflight.add_argument("--invite", type=Path, required=True)
    run = session_commands.add_parser("run", help="start or resume the one monotonic timed arm")
    add_state_dir(run)
    run.add_argument("--invite", type=Path, required=True)
    run.add_argument("--yes", action="store_true", help="confirm START non-interactively")
    run.add_argument("--no-open", action="store_true")
    status = session_commands.add_parser("status", help="show only the authenticated participant's progress")
    add_state_dir(status)
    status.add_argument("--invite", type=Path, required=True)

    adjudicator = commands.add_parser("adjudicator", help="bind independent public keys to frozen slots")
    adjudicator_commands = adjudicator.add_subparsers(dest="adjudicator_command", required=True)
    register = adjudicator_commands.add_parser("register", help="register the next adjudicator key")
    add_state_dir(register)
    register.add_argument("--operator-key", type=Path, required=True)
    register.add_argument("--public-key", type=Path, required=True)
    register.add_argument("--receipt-out", type=Path, required=True)

    adjudication = commands.add_parser("adjudication", help="run blind commit-reveal carry adjudication")
    adjudication_commands = adjudication.add_subparsers(dest="adjudication_command", required=True)
    assign = adjudication_commands.add_parser("assign", help="claim the next frozen unit for this key")
    add_state_dir(assign)
    assign.add_argument("--adjudicator-key", type=Path, required=True)
    assign.add_argument("--receipt-out", type=Path, required=True)
    commit = adjudication_commands.add_parser("commit", help="commit a hidden signed decision")
    add_state_dir(commit)
    commit.add_argument("--assignment", type=Path, required=True)
    commit.add_argument("--adjudicator-key", type=Path, required=True)
    commit.add_argument("--decision", choices=DECISIONS, required=True)
    commit.add_argument("--reveal-out", type=Path, required=True)
    reveal = adjudication_commands.add_parser("reveal", help="open a decision after independent commitments")
    add_state_dir(reveal)
    reveal.add_argument("--reveal", type=Path, required=True)
    adjudication_status_parser = adjudication_commands.add_parser("status", help="show aggregate adjudication progress")
    add_state_dir(adjudication_status_parser)

    follow_up = commands.add_parser("follow-up", help="measure authenticated 28-day native Resume reuse")
    follow_up_commands = follow_up.add_subparsers(dest="follow_up_command", required=True)
    invite = follow_up_commands.add_parser("invite", help="open one follow-up window")
    add_state_dir(invite)
    invite.add_argument("--operator-key", type=Path, required=True)
    invite.add_argument("--invite", type=Path, required=True)
    invite.add_argument("--receipt-out", type=Path, required=True)
    follow_run = follow_up_commands.add_parser("run", help="run native Resume and record Workbench readiness")
    add_state_dir(follow_run)
    follow_run.add_argument("--follow-up", type=Path, required=True)
    follow_run.add_argument("resume_arguments", nargs=argparse.REMAINDER)
    close = follow_up_commands.add_parser("close", help="close a fully elapsed 28-day window")
    add_state_dir(close)
    close.add_argument("--operator-key", type=Path, required=True)
    close.add_argument("--follow-up", type=Path, required=True)

    lock = commands.add_parser("lock", help="build counts-only data and invoke the frozen evaluator")
    add_state_dir(lock)
    lock.add_argument("--operator-key", type=Path, required=True)
    lock.add_argument("--output", type=Path, required=True)
    lock.add_argument("--aggregate-output", type=Path, required=True)

    final = commands.add_parser("attest-final", help="sign hashes, flow counts, and operator claims")
    add_state_dir(final)
    final.add_argument("--operator-key", type=Path, required=True)
    final.add_argument("--dataset", type=Path, required=True)
    final.add_argument("--aggregate", type=Path, required=True)
    final.add_argument("--output", type=Path, required=True)
    final.add_argument("--consent-obtained", action="store_true")
    final.add_argument("--provider-authorized", action="store_true")
    final.add_argument("--linkage-key-not-exported", action="store_true")

    verify = commands.add_parser("verify", help="independently verify signatures, hashes, privacy, and aggregate")
    verify.add_argument("--plan", type=Path, required=True)
    verify.add_argument("--preregistration", type=Path, required=True)
    verify.add_argument("--task-catalog", type=Path, required=True)
    verify.add_argument("--plan-attestation", type=Path, required=True)
    verify.add_argument("--plan-signature", type=Path, required=True)
    verify.add_argument("--operator-public-key", type=Path, required=True)
    verify.add_argument("--dataset", type=Path, required=True)
    verify.add_argument("--aggregate", type=Path, required=True)
    verify.add_argument("--final-attestation", type=Path, required=True)
    verify.add_argument("--final-signature", type=Path, required=True)
    return parser.parse_args()


def main():
    arguments = parse_arguments()
    if arguments.command == "self-test":
        run_self_test()
    elif arguments.command == "plan":
        if arguments.plan_command == "create":
            plan_create(arguments)
        elif arguments.plan_command == "attest":
            plan_attest(arguments)
        else:
            plan_verify(arguments)
    elif arguments.command == "enroll":
        enroll(arguments)
    elif arguments.command == "attrition":
        if arguments.attrition_command == "replace":
            attrition_replace(arguments)
        else:
            attrition_withdraw(arguments)
    elif arguments.command == "session":
        if arguments.session_command == "preflight":
            session_preflight(arguments)
        elif arguments.session_command == "run":
            session_run(arguments)
        else:
            session_status(arguments)
    elif arguments.command == "adjudicator":
        adjudicator_register(arguments)
    elif arguments.command == "adjudication":
        if arguments.adjudication_command == "assign":
            adjudication_assign(arguments)
        elif arguments.adjudication_command == "commit":
            adjudication_commit(arguments)
        elif arguments.adjudication_command == "reveal":
            adjudication_reveal(arguments)
        else:
            adjudication_status(arguments)
    elif arguments.command == "follow-up":
        if arguments.follow_up_command == "invite":
            follow_up_invite(arguments)
        elif arguments.follow_up_command == "run":
            follow_up_run(arguments)
        else:
            follow_up_close(arguments)
    elif arguments.command == "lock":
        lock_collection(arguments)
    elif arguments.command == "attest-final":
        attest_final(arguments)
    else:
        verify_export(arguments)


if __name__ == "__main__":
    main()
