#!/usr/bin/env python3
"""Verify and score the MergeForensicsBench v1 raw-provider replay corpus."""

from __future__ import annotations

import argparse
import base64
import copy
import hashlib
import json
from pathlib import Path
import re
from typing import Callable


BUNDLE = Path(__file__).resolve().parent
CASES_PATH = BUNDLE / "cases.json"
TRANSCRIPTS_PATH = BUNDLE / "transcripts.json"
ORACLE_PATH = BUNDLE / "oracle.json"
BASELINE_PATH = BUNDLE / "baseline-predictions.json"
MANIFEST_PATH = BUNDLE / "manifest.json"
CHECKSUMS_PATH = BUNDLE / "SHA256SUMS"

VERSION = "1.0.0"
CASES_SCHEMA = "stratadiff-merge-forensics-cases-v1"
TRANSCRIPTS_SCHEMA = "stratadiff-merge-forensics-transcripts-v1"
ORACLE_SCHEMA = "stratadiff-merge-forensics-oracle-v1"
PREDICTIONS_SCHEMA = "stratadiff-merge-forensics-predictions-v1"
MANIFEST_SCHEMA = "stratadiff-merge-forensics-manifest-v1"
EVALUATION_SCHEMA = "stratadiff-merge-forensics-evaluation-v1"

CASE_IDS = [f"c{index:03d}" for index in range(1, 13)]
QUERY_CONTRACT_SHA256 = "ed4b6e8e04a1a74c08bfb4e649a6ab721124beeb4d56a10c3df2790d08a0782d"
EXCHANGE_TRACE_SHA256 = {
    "c001": "544e0b74f12b38c99569d691f93301027ca53f59458f9013791ba23d9876b18d",
    "c002": "1dc111d0199f8af2d885bd0732cf88fa8fc4b482a22eb03382d6cd78af2ed423",
    "c003": "5a4a33ffb95d2db00fa4850cd9b3c2c77af902dadb9e0be12635788ab4baaf03",
    "c004": "c580cb70a0de64703977f936eb1a2a1d7fde22319f8d03ee3eb3914a12f5bf68",
    "c005": "888c47b6f770e620c7839b69a1d5fcad720abd861bd86daed6f22261a30f4ac0",
    "c006": "30876e4b5208693f54a496bf338a0105c55892bdc02349a69fd5fcbe522900ac",
    "c007": "608cd8f464bc0da389d65c3d60745ab2224d4b071bcae4e53e9b3301b25cccdb",
    "c008": "cc6f77dc39762665468f81f66bae3ff91b66095574e0ab720459df537a10092c",
    "c009": "fc70edde2b83cac1379376cdbd16ec3d27bf49b2d9530bfefe225eea48153683",
    "c010": "b8c888e2ad101b7db0ff24baf6a687960ec639a22c9eff6c55963de62afaa8fa",
    "c011": "234d2b8bb7f579a188c7970cce7b8bd46ff9812a66475926777f719d2fdb14c4",
    "c012": "55927280542f3b1f9c9ff51c657b21e2c6e1bc1104e9411d1f9be2e4825302eb",
}
PAYLOAD_SHA256 = {
    "p001": "584d8566fd7a20287a26d4c64141f4a68a0fceb576b8c6cf9cd889c0daab0026",
    "p002": "7a7ff59ce360c7c9b9eb0ec4fb772ac308e5257c7a1723173e5e0470d564e52a",
    "p003": "20bd7da8c098741153517e291163f44c5e7e4f60b7e7af96b39a8e7efeab3d63",
    "p004": "65b18b5654235e4b85fc7d2311c092bbae8d52aa7c3d07be46243f158ed8b0a1",
    "p005": "ed52d2c919d968b70c0890dde4eca9543203fd139a9a26db1524b902bf15c54a",
    "p006": "f20dc54f384d6c505008144b3014aa8c2e0d56e3e92eca85eee908c04ec1009d",
    "p007": "3824df8b49d195c95afee7faf6d14d9b1ee4d747b2d4dbeb9ed1ed6a41c872db",
    "p008": "52eee86998cce2fbb2226087de4a5117684d00dfa740957b2c070f4d5fbbbf8b",
    "p009": "3e15c231d907da12f04930795ab6ab6defdac6d81d56646044af675cde48eac8",
    "p010": "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
    "p011": "81e241914ffb014bde92943188707bd17e83c176c4c8744109112bbce6c00ebf",
    "p012": "302c1a7644de39d77d5416910fb0a4537dda94ae026c2e17801ba73ca4f55e9f",
    "p013": "1a53ffe2863abff1995fa35a9bbf635f4cc0d690184d0e816c6900ecc5a5e654",
    "p014": "48eac45ef0e9ab43efed4659e2a196a2c44ec22c188e0be522283b7498da35c1",
    "p015": "36f31a84f4b87d6be035e2cf257b217393201a74a4b0ac2c6b4ffbc0580ffb76",
    "p016": "9dcec08c640dc22553dabf4d3ef33f5ec50edccb99d65341dcdc7c893779a9f9",
    "p017": "c8cdf312b95dc493b03b8387605b3257b5c9c8b92643e4157a286236db5e5787",
    "p018": "4edd4a5570ac96fe40c5bb0d4413402ea2792bd2dc6df28f8789f4abf5508cfd",
    "p019": "ccd156a5f5cb4749aeab15fc4544e5c11fd0f1f76f3fb828fab962c51638716a",
    "p020": "f1eb788d64a3f1f77210ff053e10412465ea675d31c9f5a33a7d6145ddd5fa01",
    "p021": "a2790a384d7d281e7395679000c35d27768d89dbd7052f725b8f4688beb59915",
    "p022": "ebf14e95e45f90a9f5a5b7cf7a0462f5684b50e65ed6b0a01e0e55050d5d848e",
    "p023": "8979e4d2cec33cbfd20e9275dd308525af786d7f1dc6b1677f0d28d5a9e25077",
    "p024": "50b5152521305b0365eeb8ffa0ee74e34d0867e2558e657febdde92c65a6bfa1",
    "p025": "e096cfdec8593a0933dd6da8d8617479870839f055a5680fd04e59ac84be2556",
    "p026": "695025f3fd096debdaf480445d0758b99207c9f030803db63c0bb2f9cc2174b8",
    "p027": "e8eb3078d1fa0328c747345862050060d5e2999cd13ae337150e4f7a62526b76",
    "p028": "5ce5ff92dc349c513d8962da94d91127591e3a125bf418383e81827712a7af79",
    "p029": "a8752e994b9a405cc9bdd9288adc84b777aba26dc7a339d79f983e794a2fb1b7",
}
EXPECTED_COVERAGE = [
    "candidate_drift",
    "complete_unique_producer",
    "default_branch_only_definition",
    "dynamic_matrix",
    "exact_job_link_mismatch",
    "expected_app_mismatch",
    "inventory_cap",
    "merge_group_run_control",
    "producer_drift",
    "provider_unknown",
    "reusable_workflow",
    "second_static_producer",
]
EXPECTED_PROVENANCE_IDS = {
    "actions-setup-node-1107",
    "cargo-dist-1069",
    "comfy-desktop-1483",
    "danger-js-1427",
    "github-actions-merge-group-docs",
    "github-actions-runs-rest-docs",
    "github-checks-rest-docs",
    "github-contents-rest-docs",
    "github-merge-queue-docs",
    "github-required-checks-docs",
    "opamp-go-320",
    "openml-python-1573",
    "prow-915",
}
CASE_ID_PATTERN = re.compile(r"^c\d{3}$")
TRANSCRIPT_ID_PATTERN = re.compile(r"^t\d{3}$")
PAYLOAD_ID_PATTERN = re.compile(r"^p\d{3}$")
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
OID_PATTERN = re.compile(r"^[0-9a-f]{40}$")
REPOSITORY_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
TIMESTAMP_PATTERN = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
WORKFLOW_PATH_PATTERN = re.compile(r"^\.github/workflows/[^/]+\.ya?ml$")

EXPECTED_FILES = {
    "README.md",
    "baseline-predictions.json",
    "cases.json",
    "manifest.json",
    "oracle.json",
    "transcripts.json",
    "verify.py",
}
FORBIDDEN_INPUT_KEYS = {
    "authorization",
    "cause",
    "cause_code",
    "cookie",
    "diagnosis",
    "expected",
    "oracle",
    "password",
    "private_key",
    "secret",
    "token",
    "verdict",
}
IDENTIFIABILITY = {"identifiable", "not_identifiable", "retry_required"}
DISPOSITIONS = {"abstain", "diagnose", "no_cause", "retry"}
VERDICTS = {"checks_blocked", "checks_clear", "inconclusive"}
REQUIREMENT_STATUSES = {
    "failed",
    "missing",
    "pending",
    "satisfied",
    "source_mismatch",
    "source_unknown",
}
EXECUTION_STATUSES = {"error", "report", "retry"}
CONCRETE_CAUSES = {
    "duplicate_job_name_ambiguous",
    "fork_approval_required",
    "merge_group_trigger_missing",
    "provider_did_not_emit_merge_group_status",
    "pull_request_merge_conflict",
    "required_check_source_mismatch",
    "required_context_not_produced",
    "workflow_activity_excludes_synchronize",
    "workflow_branch_filter_excluded",
    "workflow_definition_invalid",
    "workflow_disabled",
    "workflow_path_filter_excluded",
}
ABSTAINING_CAUSES = {
    "fork_approval_possible",
    "provider_runtime_delivery_gap",
    "workflow_trigger_unknown",
}
KNOWN_ACTIONS = {
    "add_merge_group_trigger",
    "complete_collection",
    "enable_workflow",
    "inspect_failed_check",
    "none",
    "replace_or_ungate_unsupported_provider",
    "restore_required_check",
    "synchronize_required_context",
}


class BenchmarkError(RuntimeError):
    """A benchmark asset or prediction violates the frozen contract."""


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


def require_int(value: object, label: str, minimum: int = 0) -> int:
    require(type(value) is int and value >= minimum, f"{label} must be an integer >= {minimum}")
    assert isinstance(value, int)
    return value


def require_nullable_string(value: object, label: str) -> str | None:
    if value is None:
        return None
    return require_string(value, label)


def require_nullable_int(value: object, label: str) -> int | None:
    if value is None:
        return None
    return require_int(value, label, 1)


def unique_json_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        require(key not in value, f"duplicate JSON key: {key}")
        value[key] = item
    return value


def decode_json_bytes(payload: bytes, label: str) -> object:
    return json.loads(payload, object_pairs_hook=unique_json_object)


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode()


def read_json(path: Path) -> tuple[bytes, dict[str, object]]:
    payload = path.read_bytes()
    value = decode_json_bytes(payload, str(path))
    require(type(value) is dict, f"{path} must contain one JSON object")
    assert isinstance(value, dict)
    require(payload == canonical_json(value), f"{path} is not canonical JSON")
    return payload, value


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def reject_input_leak(value: object, label: str) -> None:
    if isinstance(value, dict):
        for key, item in value.items():
            require(key.casefold() not in FORBIDDEN_INPUT_KEYS, f"forbidden field at {label}.{key}")
            reject_input_leak(item, f"{label}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            reject_input_leak(item, f"{label}[{index}]")


def validate_string_array(value: object, label: str) -> list[str]:
    raw = require_array(value, label)
    result = [require_string(item, f"{label}[{index}]") for index, item in enumerate(raw)]
    require(len(result) == len(set(result)), f"{label} contains duplicates")
    return result


def validate_cases(value: dict[str, object]) -> list[dict[str, object]]:
    require_exact_keys(value, {"cases", "dataset_version", "schema"}, "cases")
    require(value["schema"] == CASES_SCHEMA, "cases schema changed")
    require(value["dataset_version"] == VERSION, "cases version changed")
    rows = require_array(value["cases"], "cases.cases")
    cases: list[dict[str, object]] = []
    ids: list[str] = []
    transcripts: set[str] = set()
    for index, item in enumerate(rows):
        case = require_object(item, f"cases.cases[{index}]")
        require_exact_keys(case, {"id", "transcript_id"}, f"case[{index}]")
        reject_input_leak(case, f"case[{index}]")
        case_id = require_string(case["id"], f"case[{index}].id")
        transcript_id = require_string(case["transcript_id"], f"case[{index}].transcript_id")
        require(bool(CASE_ID_PATTERN.fullmatch(case_id)), f"invalid case ID {case_id}")
        require(bool(TRANSCRIPT_ID_PATTERN.fullmatch(transcript_id)), f"invalid transcript ID {transcript_id}")
        require(transcript_id == f"t{case_id[1:]}", f"{case_id} transcript ID is not opaque-aligned")
        require(transcript_id not in transcripts, f"duplicate transcript ID {transcript_id}")
        ids.append(case_id)
        transcripts.add(transcript_id)
        cases.append(case)
    require(ids == CASE_IDS, f"case IDs/order must be exactly {CASE_IDS}")
    return cases


def validate_payload(value: object, index: int) -> tuple[str, bytes, object]:
    label = f"transcripts.payloads[{index}]"
    payload = require_object(value, label)
    require_exact_keys(payload, {"body_utf8", "byte_length", "id", "media_type", "sha256"}, label)
    payload_id = require_string(payload["id"], f"{label}.id")
    require(bool(PAYLOAD_ID_PATTERN.fullmatch(payload_id)), f"{label}.id is invalid")
    require(payload_id in PAYLOAD_SHA256, f"{label}.id is not in the frozen payload set")
    require(payload["media_type"] == "application/json", f"{label}.media_type changed")
    body = require_string(payload["body_utf8"], f"{label}.body_utf8").encode()
    require(len(body) == require_int(payload["byte_length"], f"{label}.byte_length", 1), f"{label} byte length mismatch")
    expected_hash = require_string(payload["sha256"], f"{label}.sha256")
    require(bool(SHA256_PATTERN.fullmatch(expected_hash)), f"{label}.sha256 is invalid")
    require(sha256_bytes(body) == expected_hash, f"{label} SHA-256 mismatch")
    require(expected_hash == PAYLOAD_SHA256[payload_id], f"{label} body differs from the frozen corpus")
    decoded = decode_json_bytes(body, label)
    reject_input_leak(decoded, label)
    return payload_id, body, decoded


def validate_transcripts(
    value: dict[str, object], cases: list[dict[str, object]]
) -> tuple[dict[str, dict[str, object]], dict[str, object]]:
    require_exact_keys(
        value,
        {"dataset_version", "payloads", "query_contracts", "schema", "transcripts"},
        "transcripts",
    )
    require(value["schema"] == TRANSCRIPTS_SCHEMA, "transcripts schema changed")
    require(value["dataset_version"] == VERSION, "transcripts version changed")

    payloads: dict[str, object] = {}
    payload_bodies: set[bytes] = set()
    for index, item in enumerate(require_array(value["payloads"], "transcripts.payloads")):
        payload_id, body, decoded = validate_payload(item, index)
        require(payload_id not in payloads, f"duplicate payload ID {payload_id}")
        require(body not in payload_bodies, f"payload {payload_id} duplicates raw body bytes")
        payloads[payload_id] = decoded
        payload_bodies.add(body)
    require(set(payloads) == set(PAYLOAD_SHA256), "payload IDs differ from the frozen corpus")

    contracts: dict[str, str] = {}
    for index, item in enumerate(require_array(value["query_contracts"], "query_contracts")):
        label = f"query_contracts[{index}]"
        contract = require_object(item, label)
        require_exact_keys(contract, {"id", "operation_name", "sha256"}, label)
        contract_id = require_string(contract["id"], f"{label}.id")
        operation = require_string(contract["operation_name"], f"{label}.operation_name")
        query_hash = require_string(contract["sha256"], f"{label}.sha256")
        require(operation == "StrataDiffPullRequestCandidate", "unexpected GraphQL operation")
        require(bool(SHA256_PATTERN.fullmatch(query_hash)), f"{label}.sha256 is invalid")
        require(query_hash == QUERY_CONTRACT_SHA256, "GraphQL query contract changed")
        require(contract_id not in contracts, f"duplicate query contract {contract_id}")
        contracts[contract_id] = query_hash
    require(set(contracts) == {"github-pr-candidate-v1"}, "query contracts changed")

    expected_transcripts = {str(case["transcript_id"]): str(case["id"]) for case in cases}
    transcripts: dict[str, dict[str, object]] = {}
    used_payloads: set[str] = set()
    for index, item in enumerate(require_array(value["transcripts"], "transcripts.transcripts")):
        label = f"transcripts.transcripts[{index}]"
        transcript = require_object(item, label)
        require_exact_keys(transcript, {"case_id", "exchanges", "id", "request"}, label)
        reject_input_leak(transcript, label)
        transcript_id = require_string(transcript["id"], f"{label}.id")
        case_id = require_string(transcript["case_id"], f"{label}.case_id")
        require(transcript_id in expected_transcripts, f"unknown transcript {transcript_id}")
        require(expected_transcripts[transcript_id] == case_id, f"{transcript_id} case mismatch")
        require(transcript_id not in transcripts, f"duplicate transcript {transcript_id}")

        request = require_object(transcript["request"], f"{label}.request")
        require_exact_keys(
            request,
            {"captured_at", "provider_url", "pull_request_number", "repository"},
            f"{label}.request",
        )
        require(request["provider_url"] == "https://github.com", f"{case_id} provider changed")
        repository = require_string(request["repository"], f"{label}.request.repository")
        require(bool(REPOSITORY_PATTERN.fullmatch(repository)), f"{case_id} repository is invalid")
        number = require_int(request["pull_request_number"], f"{label}.request.pull_request_number", 1)
        captured_at = require_string(request["captured_at"], f"{label}.request.captured_at")
        require(bool(TIMESTAMP_PATTERN.fullmatch(captured_at)), f"{case_id} captured_at is invalid")
        owner, name = repository.split("/")

        exchanges = require_array(transcript["exchanges"], f"{label}.exchanges")
        require(bool(exchanges), f"{case_id} transcript is empty")
        for exchange_index, item_value in enumerate(exchanges):
            exchange_label = f"{label}.exchanges[{exchange_index}]"
            exchange = require_object(item_value, exchange_label)
            require_exact_keys(exchange, {"protocol", "request", "response", "sequence"}, exchange_label)
            require(
                exchange["sequence"] == exchange_index + 1,
                f"{case_id} exchange sequence is not contiguous",
            )
            protocol = exchange["protocol"]
            request_data = require_object(exchange["request"], f"{exchange_label}.request")
            if protocol == "rest":
                require_exact_keys(request_data, {"endpoint", "method"}, f"{exchange_label}.request")
                require(request_data["method"] == "GET", f"{case_id} REST method changed")
                endpoint = require_string(request_data["endpoint"], f"{exchange_label}.request.endpoint")
                require(endpoint.startswith(f"repos/{repository}/" ) or endpoint == f"repos/{repository}", f"{case_id} REST endpoint escaped repository")
                require("//" not in endpoint and ".." not in endpoint, f"{case_id} REST endpoint is unsafe")
            elif protocol == "graphql":
                require_exact_keys(request_data, {"query_contract_id", "variables"}, f"{exchange_label}.request")
                contract_id = require_string(request_data["query_contract_id"], f"{exchange_label}.request.query_contract_id")
                require(contract_id in contracts, f"{case_id} uses unknown GraphQL contract")
                variables = require_object(request_data["variables"], f"{exchange_label}.request.variables")
                require_exact_keys(variables, {"name", "number", "owner"}, f"{exchange_label}.request.variables")
                require(
                    variables == {"name": name, "number": number, "owner": owner},
                    f"{case_id} GraphQL variables do not match request identity",
                )
            else:
                raise BenchmarkError(f"{case_id} exchange protocol is invalid")

            response = require_object(exchange["response"], f"{exchange_label}.response")
            require_exact_keys(response, {"link_header", "payload_id", "status"}, f"{exchange_label}.response")
            require(response["status"] == 200, f"{case_id} fixture response must be HTTP 200")
            link = response["link_header"]
            require(link is None or (type(link) is str and len(link) <= 16_384), f"{case_id} Link header is invalid")
            payload_id = require_string(response["payload_id"], f"{exchange_label}.response.payload_id")
            require(payload_id in payloads, f"{case_id} references unknown payload {payload_id}")
            used_payloads.add(payload_id)
        require(
            sha256_bytes(canonical_json(exchanges)) == EXCHANGE_TRACE_SHA256[case_id],
            f"{case_id} ordered exchange trace changed",
        )
        transcripts[case_id] = transcript
    require(set(transcripts) == set(CASE_IDS), "transcript case IDs differ from cases")
    require(used_payloads == set(payloads), "payload pool contains unused or unreferenced bodies")
    return transcripts, payloads


def exchange_payload(exchange: dict[str, object], payloads: dict[str, object]) -> object:
    response = require_object(exchange["response"], "exchange.response")
    require(response["status"] == 200, "fixture evidence requires an HTTP 200 response")
    return payloads[str(response["payload_id"])]


def rest_exchanges(transcript: dict[str, object], endpoint: str) -> list[dict[str, object]]:
    result: list[dict[str, object]] = []
    for item in require_array(transcript["exchanges"], "transcript.exchanges"):
        exchange = require_object(item, "exchange")
        if exchange["protocol"] != "rest":
            continue
        request = require_object(exchange["request"], "exchange.request")
        if request["endpoint"] == endpoint:
            result.append(exchange)
    return result


def matching_rest_exchanges(transcript: dict[str, object], pattern: re.Pattern[str]) -> list[dict[str, object]]:
    result: list[dict[str, object]] = []
    for item in require_array(transcript["exchanges"], "transcript.exchanges"):
        exchange = require_object(item, "exchange")
        if exchange["protocol"] != "rest":
            continue
        request = require_object(exchange["request"], "exchange.request")
        endpoint = str(request["endpoint"])
        if pattern.fullmatch(endpoint):
            result.append(exchange)
    return result


def graphql_exchanges(transcript: dict[str, object]) -> list[dict[str, object]]:
    return [
        require_object(item, "exchange")
        for item in require_array(transcript["exchanges"], "transcript.exchanges")
        if require_object(item, "exchange")["protocol"] == "graphql"
    ]


def pull_identity(
    transcript: dict[str, object], payloads: dict[str, object]
) -> dict[str, object]:
    request = require_object(transcript["request"], "transcript.request")
    repository_name = str(request["repository"])
    number = int(request["pull_request_number"])
    endpoint = f"repos/{repository_name}/pulls/{number}"
    exchanges = rest_exchanges(transcript, endpoint)
    require(bool(exchanges), "transcript has no pull-request observation")
    bodies = [require_object(exchange_payload(item, payloads), "pull request") for item in exchanges]
    require(all(item == bodies[0] for item in bodies), "pull-request identity changed between observations")
    pull = bodies[0]
    repository_url = f"https://github.com/{repository_name}"
    require(pull["number"] == number, "REST pull-request number changed")
    require(pull["html_url"] == f"{repository_url}/pull/{number}", "REST pull-request URL changed")
    require(pull["state"] == "open", "fixture pull request must be open")
    base = require_object(pull["base"], "REST pull-request base")
    head = require_object(pull["head"], "REST pull-request head")
    head_repository = require_object(head["repo"], "REST pull-request head repository")
    base_ref = require_string(base["ref"], "REST base ref")
    base_sha = require_string(base["sha"], "REST base SHA")
    head_sha = require_string(head["sha"], "REST head SHA")
    require(bool(OID_PATTERN.fullmatch(base_sha)), "REST base SHA is invalid")
    require(bool(OID_PATTERN.fullmatch(head_sha)), "REST head SHA is invalid")
    require(head_repository["full_name"] == repository_name, "REST head repository changed")
    return {
        "base_ref": base_ref,
        "base_sha": base_sha,
        "head_sha": head_sha,
        "number": number,
        "repository": repository_name,
        "repository_url": repository_url,
    }


def queue_candidate(
    exchange: dict[str, object],
    payloads: dict[str, object],
    pull_identity_value: dict[str, object],
) -> dict[str, object]:
    envelope = require_object(exchange_payload(exchange, payloads), "GraphQL payload")
    if "errors" in envelope:
        require(not require_array(envelope["errors"], "GraphQL errors"), "GraphQL response contains errors")
    data = require_object(envelope["data"], "GraphQL data")
    repository = require_object(data["repository"], "GraphQL repository")
    pull = require_object(repository["pullRequest"], "GraphQL pullRequest")
    entry = require_object(pull["mergeQueueEntry"], "GraphQL mergeQueueEntry")
    head = require_object(entry["headCommit"], "GraphQL headCommit")
    base = require_object(entry["baseCommit"], "GraphQL baseCommit")
    nested_pull = require_object(entry["pullRequest"], "GraphQL mergeQueueEntry pullRequest")
    potential_merge = require_object(pull["potentialMergeCommit"], "GraphQL potential merge")

    repository_name = require_string(repository["nameWithOwner"], "GraphQL repository name")
    repository_url = require_string(repository["url"], "GraphQL repository URL")
    pull_url = require_string(pull["url"], "GraphQL pull-request URL")
    pull_number = require_int(pull["number"], "GraphQL pull-request number", 1)
    base_ref = require_string(pull["baseRefName"], "GraphQL base ref")
    base_ref_sha = require_string(pull["baseRefOid"], "GraphQL base-ref SHA")
    head_ref_sha = require_string(pull["headRefOid"], "GraphQL head-ref SHA")
    potential_merge_sha = require_string(potential_merge["oid"], "GraphQL potential-merge SHA")
    queue_head_sha = require_string(head["oid"], "GraphQL queue head SHA")
    queue_base_sha = require_string(base["oid"], "GraphQL queue base SHA")
    queue_entry_id = require_string(entry["id"], "GraphQL queue entry ID")
    queue_state = require_string(entry["state"], "GraphQL queue state")
    queue_position = require_int(entry["position"], "GraphQL queue position", 1)
    mergeable = require_string(pull["mergeable"], "GraphQL mergeable state")
    merge_state_status = require_string(pull["mergeStateStatus"], "GraphQL merge state status")
    require(type(pull["isMergeQueueEnabled"]) is bool, "GraphQL merge-queue-enabled flag is invalid")
    require(type(pull["isInMergeQueue"]) is bool, "GraphQL in-merge-queue flag is invalid")
    require(pull["isMergeQueueEnabled"] and pull["isInMergeQueue"], "fixture pull request must be queued")
    require(pull["state"] == "OPEN", "GraphQL pull request must be open")
    require(repository_name == pull_identity_value["repository"], "GraphQL repository identity changed")
    require(repository_url == pull_identity_value["repository_url"], "GraphQL repository URL changed")
    require(pull_number == pull_identity_value["number"], "GraphQL pull-request number changed")
    require(pull_url == f"{repository_url}/pull/{pull_number}", "GraphQL pull-request URL changed")
    require(base_ref == pull_identity_value["base_ref"], "GraphQL base ref changed")
    require(base_ref_sha == pull_identity_value["base_sha"], "GraphQL base-ref SHA changed")
    require(head_ref_sha == pull_identity_value["head_sha"], "GraphQL head-ref SHA changed")
    require(nested_pull["number"] == pull_number, "GraphQL queue pull-request number changed")
    require(nested_pull["headRefOid"] == head_ref_sha, "GraphQL queue pull-request head changed")
    for label, sha in [
        ("base-ref", base_ref_sha),
        ("head-ref", head_ref_sha),
        ("potential-merge", potential_merge_sha),
        ("queue-base", queue_base_sha),
        ("queue-head", queue_head_sha),
    ]:
        require(bool(OID_PATTERN.fullmatch(sha)), f"GraphQL {label} SHA is invalid")
    return {
        "base_ref": base_ref,
        "base_ref_sha": base_ref_sha,
        "head_ref_sha": head_ref_sha,
        "merge_state_status": merge_state_status,
        "mergeable": mergeable,
        "potential_merge_sha": potential_merge_sha,
        "pull_number": pull_number,
        "pull_url": pull_url,
        "queue_base_sha": queue_base_sha,
        "queue_entry_id": queue_entry_id,
        "queue_head_sha": queue_head_sha,
        "queue_position": queue_position,
        "queue_state": queue_state,
        "repository": repository_name,
        "repository_url": repository_url,
    }


def policy_requirement(
    transcript: dict[str, object], payloads: dict[str, object]
) -> tuple[str, int, list[dict[str, object]]]:
    request = require_object(transcript["request"], "transcript.request")
    repository = str(request["repository"])
    endpoint = f"repos/{repository}/rules/branches/main?per_page=100&page=1"
    exchanges = rest_exchanges(transcript, endpoint)
    require(bool(exchanges), "transcript has no effective-rule observation")
    values: dict[tuple[str, int], list[dict[str, object]]] = {}
    payload_identities: set[str] = set()
    for exchange in exchanges:
        response = require_object(exchange["response"], "policy response")
        payload_identities.add(str(response["payload_id"]))
        rules = require_array(exchange_payload(exchange, payloads), "effective rules")
        for item in rules:
            rule = require_object(item, "effective rule")
            if rule["type"] != "required_status_checks":
                continue
            require(rule["ruleset_source"] == repository, "ruleset source does not match repository")
            require(rule["ruleset_source_type"] == "Repository", "ruleset source type changed")
            ruleset_id = require_int(rule["ruleset_id"], "ruleset ID", 1)
            policy = {
                "id": str(ruleset_id),
                "kind": "ruleset",
                "name": f"ruleset {ruleset_id}",
                "url": f"https://github.com/{repository}/rules/{ruleset_id}",
            }
            parameters = require_object(rule["parameters"], "required checks parameters")
            required = require_array(parameters["required_status_checks"], "required status checks")
            for check_value in required:
                check = require_object(check_value, "required status check")
                key = (
                    require_string(check["context"], "required check context"),
                    require_int(check["integration_id"], "required check App ID", 1),
                )
                policies = values.setdefault(key, [])
                if policy not in policies:
                    policies.append(policy)
    require(len(payload_identities) == 1, "policy changed between replay observations")
    require(len(values) == 1, "seed transcript must contain one pinned required check")
    (context, app_id), policies = next(iter(values.items()))
    return context, app_id, sorted(policies, key=lambda item: (str(item["kind"]), str(item["id"])))


def exact_requirement_status(
    transcript: dict[str, object],
    payloads: dict[str, object],
    target_sha: str,
    context: str,
    expected_app_id: int,
) -> tuple[str, list[dict[str, object]]]:
    request = require_object(transcript["request"], "transcript.request")
    repository = str(request["repository"])
    endpoint = (
        f"repos/{repository}/commits/{target_sha}/check-runs?filter=latest&per_page=100&page=1"
    )
    exchanges = rest_exchanges(transcript, endpoint)
    require(bool(exchanges), "transcript has no exact-target check-run observation")
    require(
        all(require_object(item["response"], "check-run response")["link_header"] is None for item in exchanges),
        "exact-target check-run observation is paginated",
    )
    payload_ids = {str(require_object(item["response"], "response")["payload_id"]) for item in exchanges}
    require(len(payload_ids) == 1, "exact-target checks changed between observations")
    body = require_object(exchange_payload(exchanges[0], payloads), "check-run list")
    checks = require_array(body["check_runs"], "check runs")
    require(body["total_count"] == len(checks), "exact-target check-run total_count mismatch")
    same_name: list[dict[str, object]] = []
    expected: list[dict[str, object]] = []
    for item in checks:
        check = require_object(item, "check run")
        require(check["head_sha"] == target_sha, "exact-target check run has the wrong SHA")
        if check["name"] != context:
            continue
        same_name.append(check)
        app = require_object(check["app"], "check run app")
        if app["id"] == expected_app_id:
            expected.append(check)
    if expected:
        successful = any(
            item["status"] == "completed" and item["conclusion"] in {"neutral", "skipped", "success"}
            for item in expected
        )
        status = "satisfied" if successful else "failed"
    elif same_name:
        status = "source_mismatch"
    else:
        status = "missing"
    evidence = [
        {
            "app_id": int(require_object(item["app"], "check app")["id"]),
            "context": str(item["name"]),
            "id": str(item["id"]),
            "kind": "check_run",
            "sha": str(item["head_sha"]),
        }
        for item in same_name
    ]
    return status, evidence


def parse_workflow_fixture(body: dict[str, object]) -> dict[str, object]:
    require(body["encoding"] == "base64", "workflow fixture encoding changed")
    encoded = require_string(body["content"], "workflow content")
    decoded = base64.b64decode(encoded, validate=True).decode()
    jobs: list[dict[str, object]] = []
    current: dict[str, object] | None = None
    in_jobs = False
    merge_group = False
    for line in decoded.splitlines():
        if line.startswith("on:"):
            merge_group = "merge_group" in line
        elif line == "  merge_group:" and not in_jobs:
            merge_group = True
        if line == "jobs:":
            in_jobs = True
            continue
        match = re.fullmatch(r"  ([A-Za-z0-9_-]+):", line)
        if in_jobs and match:
            if current is not None:
                jobs.append(current)
            current = {
                "id": match.group(1),
                "name": match.group(1),
                "name_static": True,
                "reusable": False,
            }
            continue
        if current is None:
            continue
        if line.startswith("    name: "):
            name = line.removeprefix("    name: ")
            current["name"] = name
            current["name_static"] = "${{" not in name
        elif line.startswith("    strategy:") or line.startswith("      matrix:"):
            current["name_static"] = False
        elif line.startswith("    uses:"):
            current["reusable"] = True
    if current is not None:
        jobs.append(current)
    require(bool(jobs), "workflow fixture has no jobs")
    return {"jobs": jobs, "merge_group": merge_group, "text": decoded}


def inventory_observation(
    transcript: dict[str, object], payloads: dict[str, object], target_sha: str
) -> tuple[bool, dict[str, dict[str, object]]]:
    request = require_object(transcript["request"], "transcript.request")
    repository = str(request["repository"])
    endpoint = f"repos/{repository}/contents/.github/workflows?ref={target_sha}"
    directories = rest_exchanges(transcript, endpoint)
    require(bool(directories), "workflow investigation has no exact-SHA inventory")
    observations: list[tuple[bool, list[object]]] = []
    for exchange in directories:
        response = require_object(exchange["response"], "directory response")
        entries = require_array(exchange_payload(exchange, payloads), "workflow directory")
        complete = response["link_header"] is None and len(entries) < 100
        observations.append((complete, entries))
    require(all(item == observations[0] for item in observations), "workflow inventory changed between passes")
    complete, entries = observations[0]
    if not complete:
        return False, {}
    definitions: dict[str, dict[str, object]] = {}
    for entry_value in entries:
        entry = require_object(entry_value, "workflow directory entry")
        path = require_string(entry["path"], "workflow path")
        if not path.endswith((".yml", ".yaml")):
            continue
        content_endpoint = f"repos/{repository}/contents/{path}?ref={target_sha}"
        contents = rest_exchanges(transcript, content_endpoint)
        require(bool(contents), f"workflow content is missing for {path}")
        bodies = [require_object(exchange_payload(item, payloads), "workflow response") for item in contents]
        require(all(item == bodies[0] for item in bodies), f"workflow {path} changed between passes")
        body = bodies[0]
        require(body["path"] == path and body["sha"] == entry["sha"], f"workflow {path} identity mismatch")
        definitions[path] = parse_workflow_fixture(body)
    return True, definitions


def producer_binding(
    transcript: dict[str, object],
    payloads: dict[str, object],
    context: str,
    app_id: int,
) -> dict[str, object] | None:
    request = require_object(transcript["request"], "transcript.request")
    repository = str(request["repository"])
    pattern = re.compile(
        rf"repos/{re.escape(repository)}/commits/([0-9a-f]{{40}})/check-runs\?check_name={re.escape(context)}&app_id={app_id}&filter=all&per_page=100&page=1"
    )
    checks_exchanges = matching_rest_exchanges(transcript, pattern)
    if not checks_exchanges:
        return None
    source_shas: set[str] = set()
    for exchange in checks_exchanges:
        request_data = require_object(exchange["request"], "producer checks request")
        match = pattern.fullmatch(str(request_data["endpoint"]))
        require(match is not None, "producer checks endpoint changed")
        assert match is not None
        source_shas.add(match.group(1))
        response = require_object(exchange["response"], "producer checks response")
        require(response["link_header"] is None, "producer check-run observation is paginated")
    require(len(source_shas) == 1, "producer source SHA changed between passes")
    source_sha = next(iter(source_shas))
    check_bodies = [require_object(exchange_payload(item, payloads), "producer checks") for item in checks_exchanges]
    require(all(item == check_bodies[0] for item in check_bodies), "producer checks changed between passes")
    checks = require_array(check_bodies[0]["check_runs"], "producer check runs")
    require(check_bodies[0]["total_count"] == len(checks), "producer check-run total_count mismatch")
    matching_checks: list[dict[str, object]] = []
    for item in checks:
        check = require_object(item, "producer check")
        app = require_object(check["app"], "producer app")
        suite = require_object(check["check_suite"], "producer check suite")
        require(check["head_sha"] == source_sha, "producer check SHA differs from its endpoint")
        require_int(check["id"], "producer check ID", 1)
        require_int(suite["id"], "producer check-suite ID", 1)
        require_int(app["id"], "producer App ID", 1)
        require_string(app["slug"], "producer App slug")
        if check["name"] == context and app["id"] == app_id and app["slug"] == "github-actions":
            matching_checks.append(check)
    if len(matching_checks) != 1:
        return None
    check = matching_checks[0]
    suite = require_object(check["check_suite"], "producer check suite")
    suite_id = int(suite["id"])
    runs_endpoint = f"repos/{repository}/actions/runs?check_suite_id={suite_id}&per_page=100&page=1"
    run_exchanges = rest_exchanges(transcript, runs_endpoint)
    if not run_exchanges:
        return None
    require(
        all(require_object(item["response"], "source runs response")["link_header"] is None for item in run_exchanges),
        "source workflow-run observation is paginated",
    )
    run_bodies = [require_object(exchange_payload(item, payloads), "source runs") for item in run_exchanges]
    require(all(item == run_bodies[0] for item in run_bodies), "source workflow runs changed between passes")
    runs = require_array(run_bodies[0]["workflow_runs"], "source workflow runs")
    require(run_bodies[0]["total_count"] == len(runs), "source workflow-run total_count mismatch")
    if len(runs) != 1:
        return None
    run = require_object(runs[0], "source workflow run")
    run_id = require_int(run["id"], "source workflow-run ID", 1)
    run_attempt = require_int(run["run_attempt"], "source workflow-run attempt", 1)
    workflow_id = require_int(run["workflow_id"], "source workflow ID", 1)
    workflow_run_path = require_string(run["path"], "source workflow-run path")
    require(run["head_sha"] == source_sha, "source workflow-run SHA differs from producer check")
    require(run["check_suite_id"] == suite_id, "source workflow-run check suite changed")
    jobs_endpoint = f"repos/{repository}/actions/runs/{run_id}/jobs?filter=all&per_page=100&page=1"
    job_exchanges = rest_exchanges(transcript, jobs_endpoint)
    if not job_exchanges:
        return None
    require(
        all(require_object(item["response"], "workflow jobs response")["link_header"] is None for item in job_exchanges),
        "workflow-job observation is paginated",
    )
    job_bodies = [require_object(exchange_payload(item, payloads), "workflow jobs") for item in job_exchanges]
    require(all(item == job_bodies[0] for item in job_bodies), "workflow jobs changed between passes")
    jobs = require_array(job_bodies[0]["jobs"], "workflow jobs")
    require(job_bodies[0]["total_count"] == len(jobs), "workflow-job total_count mismatch")
    matching_jobs = [
        require_object(item, "workflow job")
        for item in jobs
        if require_object(item, "workflow job")["name"] == context
        and require_object(item, "workflow job")["check_run_url"] == check["url"]
    ]
    if len(matching_jobs) != 1:
        return None
    job = matching_jobs[0]
    require(job["run_attempt"] == run_attempt, "workflow job attempt differs from its run")
    require_int(job["id"], "workflow job ID", 1)
    workflow_endpoint = f"repos/{repository}/actions/workflows/{workflow_id}"
    metadata_exchanges = rest_exchanges(transcript, workflow_endpoint)
    if not metadata_exchanges:
        return None
    require(
        all(require_object(item["response"], "workflow metadata response")["link_header"] is None for item in metadata_exchanges),
        "workflow metadata response is unexpectedly paginated",
    )
    metadata_bodies = [require_object(exchange_payload(item, payloads), "workflow metadata") for item in metadata_exchanges]
    require(all(item == metadata_bodies[0] for item in metadata_bodies), "producer metadata changed between passes")
    workflow = metadata_bodies[0]
    path = require_string(workflow["path"], "producer workflow path")
    require(workflow["id"] == workflow_id, "producer workflow metadata ID changed")
    require(workflow["state"] == "active", "producer workflow is not active")
    require(bool(WORKFLOW_PATH_PATTERN.fullmatch(path)), "producer workflow path is invalid")
    require(workflow_run_path.split("@", 1)[0] == path, "workflow run path differs from metadata")
    app = require_object(check["app"], "producer app")
    return {
        "app_id": int(app["id"]),
        "app_slug": str(app["slug"]),
        "check_name": str(check["name"]),
        "check_run_id": int(check["id"]),
        "check_suite_id": suite_id,
        "kind": "producer_binding",
        "source_sha": source_sha,
        "workflow_id": workflow_id,
        "workflow_job_id": int(job["id"]),
        "workflow_path": path,
        "workflow_run_id": run_id,
        "workflow_run_path": workflow_run_path,
    }


def diagnosis_claim(value: str) -> dict[str, object]:
    return {"kind": "diagnosis_evidence", "value": value}


def base_expected(
    disposition: str,
    cause_code: str | None,
    confidence: str | None,
    action_code: str | None,
    required_evidence: list[dict[str, object]],
) -> dict[str, object]:
    actions = ["complete_collection"]
    if action_code is not None:
        actions.append(action_code)
    return {
        "action_code": action_code,
        "allowed_action_codes": sorted(set(actions)),
        "cause_code": cause_code,
        "confidence": confidence,
        "disposition": disposition,
        "required_evidence": required_evidence,
        "verdict_allowlist": ["inconclusive"],
    }


def derive_observable_case(
    transcript: dict[str, object], payloads: dict[str, object]
) -> dict[str, object]:
    pull = pull_identity(transcript, payloads)
    graphql_calls = graphql_exchanges(transcript)
    require(len(graphql_calls) >= 2, "transcript must contain a GraphQL boundary pair")
    candidates = [queue_candidate(item, payloads, pull) for item in graphql_calls]
    context, app_id, policies = policy_requirement(transcript, payloads)
    status, requirement_evidence = exact_requirement_status(
        transcript, payloads, str(candidates[0]["queue_head_sha"]), context, app_id
    )
    target = {
        "base_sha": candidates[-1]["queue_base_sha"],
        "kind": "merge_group",
        "queue_entry_id": candidates[-1]["queue_entry_id"],
        "queue_state": str(candidates[-1]["queue_state"]).lower(),
        "resolution": "provisional",
        "sha": candidates[-1]["queue_head_sha"],
    }
    requirement = {
        "context": context,
        "expected_app_id": app_id,
        "policies": policies,
        "status": status,
    }
    if any(item != candidates[0] for item in candidates[1:]):
        return {
            "expected": {
                "action_code": None,
                "allowed_action_codes": [],
                "cause_code": None,
                "confidence": None,
                "disposition": "retry",
                "error_kind": "target_drift",
                "required_evidence": [],
                "verdict_allowlist": [],
            },
            "identifiability": {key: "retry_required" for key in ["action", "cause", "requirement", "target"]},
            "requirement": requirement,
            "target": target,
        }

    request = require_object(transcript["request"], "transcript.request")
    repository = str(request["repository"])
    metadata_pattern = re.compile(rf"repos/{re.escape(repository)}/actions/workflows/\d+")
    metadata = matching_rest_exchanges(transcript, metadata_pattern)
    metadata_payloads = [exchange_payload(item, payloads) for item in metadata]
    if metadata_payloads and any(item != metadata_payloads[0] for item in metadata_payloads):
        return {
            "expected": {
                "action_code": None,
                "allowed_action_codes": [],
                "cause_code": None,
                "confidence": None,
                "disposition": "retry",
                "error_kind": "producer_drift",
                "required_evidence": [],
                "verdict_allowlist": [],
            },
            "identifiability": {key: "retry_required" for key in ["action", "cause", "requirement", "target"]},
            "requirement": requirement,
            "target": target,
        }

    if status == "source_mismatch":
        evidence = [dict(item) for item in requirement_evidence]
        return {
            "expected": base_expected(
                "diagnose", "required_check_source_mismatch", None, None, evidence
            ),
            "identifiability": {
                "action": "identifiable",
                "cause": "identifiable",
                "requirement": "identifiable",
                "target": "identifiable",
            },
            "requirement": requirement,
            "target": target,
        }
    require(status == "missing", "seed workflow cases must expose a missing pinned requirement")

    producer = producer_binding(transcript, payloads, context, app_id)
    producer_evidence = [producer] if producer is not None else []
    target_sha = str(candidates[0]["queue_head_sha"])
    complete, workflows = inventory_observation(transcript, payloads, target_sha)
    if not complete or producer is None:
        return {
            "expected": base_expected("abstain", None, None, None, producer_evidence),
            "identifiability": {
                "action": "not_identifiable",
                "cause": "not_identifiable",
                "requirement": "identifiable",
                "target": "identifiable",
            },
            "requirement": requirement,
            "target": target,
        }
    if any(
        not bool(job["name_static"]) or bool(job["reusable"])
        for workflow in workflows.values()
        for job in require_array(workflow["jobs"], "workflow jobs")
    ):
        return {
            "expected": base_expected("abstain", None, None, None, producer_evidence),
            "identifiability": {
                "action": "not_identifiable",
                "cause": "not_identifiable",
                "requirement": "identifiable",
                "target": "identifiable",
            },
            "requirement": requirement,
            "target": target,
        }
    matching = [
        (path, job)
        for path, workflow in workflows.items()
        for job in require_array(workflow["jobs"], "workflow jobs")
        if job["name"] == context
    ]
    if len(matching) != 1 or matching[0][0] != producer["workflow_path"]:
        return {
            "expected": base_expected("abstain", None, None, None, producer_evidence),
            "identifiability": {
                "action": "not_identifiable",
                "cause": "not_identifiable",
                "requirement": "identifiable",
                "target": "identifiable",
            },
            "requirement": requirement,
            "target": target,
        }
    workflow = workflows[str(producer["workflow_path"])]
    if not bool(workflow["merge_group"]):
        evidence = producer_evidence + [
            diagnosis_claim("target:merge_group"),
            diagnosis_claim(f"workflow:{producer['workflow_path']}"),
            diagnosis_claim("trigger:merge_group_absent"),
        ]
        return {
            "expected": base_expected(
                "diagnose",
                "merge_group_trigger_missing",
                "certain",
                "add_merge_group_trigger",
                evidence,
            ),
            "identifiability": {
                "action": "identifiable",
                "cause": "identifiable",
                "requirement": "identifiable",
                "target": "identifiable",
            },
            "requirement": requirement,
            "target": target,
        }

    target_runs_endpoint = (
        f"repos/{repository}/actions/runs?head_sha={target_sha}&per_page=100&page=1"
    )
    target_runs = rest_exchanges(transcript, target_runs_endpoint)
    require(bool(target_runs), "workflow investigation has no exact-target run observation")
    require(
        all(require_object(item["response"], "target runs response")["link_header"] is None for item in target_runs),
        "exact-target workflow-run observation is paginated",
    )
    target_run_bodies = [
        require_object(exchange_payload(item, payloads), "target runs") for item in target_runs
    ]
    require(
        all(item == target_run_bodies[0] for item in target_run_bodies),
        "exact-target workflow runs changed between passes",
    )
    runs = require_array(target_run_bodies[0]["workflow_runs"], "target workflow runs")
    require(target_run_bodies[0]["total_count"] == len(runs), "target workflow-run total_count mismatch")
    matching_runs = []
    for item in runs:
        run = require_object(item, "target workflow run")
        require_int(run["id"], "target workflow-run ID", 1)
        require_int(run["run_attempt"], "target workflow-run attempt", 1)
        if (
            run["event"] == "merge_group"
            and run["head_sha"] == target_sha
            and run["workflow_id"] == producer["workflow_id"]
            and run["status"] == "queued"
            and str(run["path"]).split("@", 1)[0] == producer["workflow_path"]
        ):
            matching_runs.append(run)
    require(len(matching_runs) <= 1, "multiple exact-target producer runs matched")
    observed = len(matching_runs) == 1
    if observed:
        evidence = producer_evidence + [
            diagnosis_claim("target:merge_group"),
            diagnosis_claim("trigger:merge_group_checks_requested"),
            diagnosis_claim("run:queued"),
        ]
        return {
            "expected": base_expected("no_cause", "none", "certain", "none", evidence),
            "identifiability": {
                "action": "identifiable",
                "cause": "identifiable",
                "requirement": "identifiable",
                "target": "identifiable",
            },
            "requirement": requirement,
            "target": target,
        }
    return {
        "expected": base_expected("abstain", None, None, None, producer_evidence),
        "identifiability": {
            "action": "not_identifiable",
            "cause": "not_identifiable",
            "requirement": "identifiable",
            "target": "identifiable",
        },
        "requirement": requirement,
        "target": target,
    }


def validate_evidence_claim(value: object, label: str) -> dict[str, object]:
    claim = require_object(value, label)
    kind = require_string(claim["kind"], f"{label}.kind")
    if kind == "diagnosis_evidence":
        require_exact_keys(claim, {"kind", "value"}, label)
        require_string(claim["value"], f"{label}.value")
    elif kind == "producer_binding":
        require_exact_keys(
            claim,
            {
                "app_id",
                "app_slug",
                "check_name",
                "check_run_id",
                "check_suite_id",
                "kind",
                "source_sha",
                "workflow_id",
                "workflow_job_id",
                "workflow_path",
                "workflow_run_id",
                "workflow_run_path",
            },
            label,
        )
        for field in ["app_id", "check_run_id", "check_suite_id", "workflow_id", "workflow_job_id", "workflow_run_id"]:
            require_int(claim[field], f"{label}.{field}", 1)
        require_string(claim["app_slug"], f"{label}.app_slug")
        require_string(claim["check_name"], f"{label}.check_name")
        source_sha = require_string(claim["source_sha"], f"{label}.source_sha")
        require(bool(OID_PATTERN.fullmatch(source_sha)), f"{label}.source_sha is invalid")
        path = require_string(claim["workflow_path"], f"{label}.workflow_path")
        require(bool(WORKFLOW_PATH_PATTERN.fullmatch(path)), f"{label}.workflow_path is invalid")
        run_path = require_string(claim["workflow_run_path"], f"{label}.workflow_run_path")
        require(run_path.split("@", 1)[0] == path, f"{label}.workflow_run_path is invalid")
    elif kind == "check_run":
        require_exact_keys(claim, {"app_id", "context", "id", "kind", "sha"}, label)
        require_int(claim["app_id"], f"{label}.app_id", 1)
        require_string(claim["context"], f"{label}.context")
        require_string(claim["id"], f"{label}.id")
        sha = require_string(claim["sha"], f"{label}.sha")
        require(bool(OID_PATTERN.fullmatch(sha)), f"{label}.sha is invalid")
    else:
        raise BenchmarkError(f"{label}.kind is invalid")
    return claim


def validate_policy(value: object, label: str) -> dict[str, object]:
    policy = require_object(value, label)
    require_exact_keys(policy, {"id", "kind", "name", "url"}, label)
    require(policy["kind"] in {"branch_protection", "ruleset"}, f"{label}.kind is invalid")
    require_string(policy["id"], f"{label}.id")
    require_string(policy["name"], f"{label}.name")
    url = require_string(policy["url"], f"{label}.url")
    require(url.startswith("https://github.com/"), f"{label}.url is invalid")
    return policy


def validate_oracle(
    value: dict[str, object],
    transcripts: dict[str, dict[str, object]],
    payloads: dict[str, object],
) -> dict[str, dict[str, object]]:
    require_exact_keys(value, {"cases", "dataset_version", "schema"}, "oracle")
    require(value["schema"] == ORACLE_SCHEMA, "oracle schema changed")
    require(value["dataset_version"] == VERSION, "oracle version changed")
    rows = require_object(value["cases"], "oracle.cases")
    require(list(rows) == CASE_IDS, "oracle case IDs/order changed")
    result: dict[str, dict[str, object]] = {}
    for case_id in CASE_IDS:
        label = f"oracle.cases.{case_id}"
        case = require_object(rows[case_id], label)
        require_exact_keys(case, {"expected", "forbidden", "ground_truth", "identifiability"}, label)
        ground = require_object(case["ground_truth"], f"{label}.ground_truth")
        require_exact_keys(ground, {"cause_code", "requirement", "target"}, f"{label}.ground_truth")
        require_string(ground["cause_code"], f"{label}.ground_truth.cause_code")
        target = require_object(ground["target"], f"{label}.ground_truth.target")
        require_exact_keys(
            target,
            {"base_sha", "kind", "queue_entry_id", "queue_state", "resolution", "sha"},
            f"{label}.ground_truth.target",
        )
        require(target["kind"] == "merge_group", f"{case_id} target kind changed")
        require(target["resolution"] == "provisional", f"{case_id} target resolution changed")
        target_sha = require_string(target["sha"], f"{label}.ground_truth.target.sha")
        base_sha = require_string(target["base_sha"], f"{label}.ground_truth.target.base_sha")
        require(bool(OID_PATTERN.fullmatch(target_sha)), f"{case_id} target SHA is invalid")
        require(bool(OID_PATTERN.fullmatch(base_sha)), f"{case_id} target base SHA is invalid")
        require_string(target["queue_entry_id"], f"{label}.ground_truth.target.queue_entry_id")
        require_string(target["queue_state"], f"{label}.ground_truth.target.queue_state")
        requirement = require_object(ground["requirement"], f"{label}.ground_truth.requirement")
        require_exact_keys(
            requirement,
            {"context", "expected_app_id", "policies", "status"},
            f"{label}.ground_truth.requirement",
        )
        require_string(requirement["context"], f"{label}.ground_truth.requirement.context")
        require_int(requirement["expected_app_id"], f"{label}.ground_truth.requirement.expected_app_id", 1)
        policies = require_array(requirement["policies"], f"{label}.ground_truth.requirement.policies")
        require(bool(policies), f"{case_id} ground-truth requirement has no policy")
        policy_keys = [
            json.dumps(validate_policy(item, f"{label}.ground_truth.requirement.policies[{index}]"), sort_keys=True)
            for index, item in enumerate(policies)
        ]
        require(len(policy_keys) == len(set(policy_keys)), f"{case_id} repeats a ground-truth policy")
        require(requirement["status"] in REQUIREMENT_STATUSES, f"{case_id} requirement status is invalid")

        identifiability = require_object(case["identifiability"], f"{label}.identifiability")
        require_exact_keys(identifiability, {"action", "cause", "requirement", "target"}, f"{label}.identifiability")
        for field in ["action", "cause", "requirement", "target"]:
            require(identifiability[field] in IDENTIFIABILITY, f"{case_id} {field} identifiability is invalid")

        expected = require_object(case["expected"], f"{label}.expected")
        expected_keys = {
            "action_code",
            "allowed_action_codes",
            "cause_code",
            "confidence",
            "disposition",
            "required_evidence",
            "verdict_allowlist",
        }
        if expected["disposition"] == "retry":
            expected_keys.add("error_kind")
        require_exact_keys(expected, expected_keys, f"{label}.expected")
        require(expected["disposition"] in DISPOSITIONS, f"{case_id} disposition is invalid")
        require_nullable_string(expected["action_code"], f"{label}.expected.action_code")
        require_nullable_string(expected["cause_code"], f"{label}.expected.cause_code")
        require_nullable_string(expected["confidence"], f"{label}.expected.confidence")
        allowed_actions = validate_string_array(expected["allowed_action_codes"], f"{label}.expected.allowed_action_codes")
        require(set(allowed_actions) <= KNOWN_ACTIONS, f"{case_id} contains an unknown allowed action")
        verdicts = validate_string_array(expected["verdict_allowlist"], f"{label}.expected.verdict_allowlist")
        require(set(verdicts) <= VERDICTS, f"{case_id} contains an unknown verdict")
        evidence = require_array(expected["required_evidence"], f"{label}.expected.required_evidence")
        for evidence_index, claim in enumerate(evidence):
            validate_evidence_claim(claim, f"{label}.expected.required_evidence[{evidence_index}]")
        if expected["disposition"] == "retry":
            require(expected["error_kind"] in {"producer_drift", "target_drift"}, f"{case_id} error kind is invalid")

        forbidden = require_object(case["forbidden"], f"{label}.forbidden")
        require_exact_keys(forbidden, {"action_codes", "cause_codes", "verdicts"}, f"{label}.forbidden")
        forbidden_actions = validate_string_array(forbidden["action_codes"], f"{label}.forbidden.action_codes")
        require(set(forbidden_actions) <= KNOWN_ACTIONS, f"{case_id} has an unknown forbidden action")
        validate_string_array(forbidden["cause_codes"], f"{label}.forbidden.cause_codes")
        forbidden_verdicts = validate_string_array(forbidden["verdicts"], f"{label}.forbidden.verdicts")
        require(set(forbidden_verdicts) <= VERDICTS, f"{case_id} has an unknown forbidden verdict")
        require(not (set(allowed_actions) & set(forbidden_actions)), f"{case_id} action is both allowed and forbidden")

        derived = derive_observable_case(transcripts[case_id], payloads)
        require(ground["target"] == derived["target"], f"{case_id} ground-truth target disagrees with transcript")
        require(ground["requirement"] == derived["requirement"], f"{case_id} ground-truth requirement disagrees with transcript")
        require(identifiability == derived["identifiability"], f"{case_id} identifiability disagrees with transcript")
        require(expected == derived["expected"], f"{case_id} expected output disagrees with independent derivation")
        if identifiability["cause"] == "identifiable":
            require(ground["cause_code"] == expected["cause_code"], f"{case_id} identifiable cause disagrees with ground truth")
        result[case_id] = case
    return result


def validate_requirement_evidence(value: object, label: str) -> dict[str, object]:
    evidence = require_object(value, label)
    require_exact_keys(evidence, {"app_id", "context", "id", "kind", "sha"}, label)
    require(evidence["kind"] == "check_run", f"{label}.kind is invalid")
    require_int(evidence["app_id"], f"{label}.app_id", 1)
    require_string(evidence["context"], f"{label}.context")
    require_string(evidence["id"], f"{label}.id")
    sha = require_string(evidence["sha"], f"{label}.sha")
    require(bool(OID_PATTERN.fullmatch(sha)), f"{label}.sha is invalid")
    return evidence


def validate_producer(value: object, label: str) -> dict[str, object] | None:
    if value is None:
        return None
    producer = require_object(value, label)
    require_exact_keys(
        producer,
        {
            "app_id",
            "app_slug",
            "check_name",
            "check_run_id",
            "check_suite_id",
            "source_sha",
            "workflow_id",
            "workflow_job_id",
            "workflow_path",
            "workflow_run_id",
            "workflow_run_path",
        },
        label,
    )
    for field in ["app_id", "check_run_id", "check_suite_id", "workflow_id", "workflow_job_id", "workflow_run_id"]:
        require_int(producer[field], f"{label}.{field}", 1)
    require_string(producer["app_slug"], f"{label}.app_slug")
    require_string(producer["check_name"], f"{label}.check_name")
    source_sha = require_string(producer["source_sha"], f"{label}.source_sha")
    require(bool(OID_PATTERN.fullmatch(source_sha)), f"{label}.source_sha is invalid")
    path = require_string(producer["workflow_path"], f"{label}.workflow_path")
    require(bool(WORKFLOW_PATH_PATTERN.fullmatch(path)), f"{label}.workflow_path is invalid")
    run_path = require_string(producer["workflow_run_path"], f"{label}.workflow_run_path")
    require(run_path.split("@", 1)[0] == path, f"{label}.workflow_run_path is invalid")
    return producer


def validate_predictions(value: dict[str, object]) -> dict[str, dict[str, object]]:
    require_exact_keys(value, {"cases", "dataset_version", "schema"}, "predictions")
    require(value["schema"] == PREDICTIONS_SCHEMA, "predictions schema changed")
    require(value["dataset_version"] == VERSION, "predictions version changed")
    rows = require_array(value["cases"], "predictions.cases")
    result: dict[str, dict[str, object]] = {}
    for index, item in enumerate(rows):
        label = f"predictions.cases[{index}]"
        case = require_object(item, label)
        require_exact_keys(
            case,
            {"case_id", "execution", "next_action_codes", "requirements", "target", "verdict", "workflow_diagnoses"},
            label,
        )
        case_id = require_string(case["case_id"], f"{label}.case_id")
        require(case_id in CASE_IDS, f"prediction has unknown case {case_id}")
        require(case_id not in result, f"duplicate prediction for {case_id}")
        execution = require_object(case["execution"], f"{label}.execution")
        require_exact_keys(execution, {"error_kind", "status"}, f"{label}.execution")
        require(execution["status"] in EXECUTION_STATUSES, f"{case_id} execution status is invalid")
        error_kind = require_nullable_string(execution["error_kind"], f"{label}.execution.error_kind")
        actions = validate_string_array(case["next_action_codes"], f"{label}.next_action_codes")
        require(set(actions) <= KNOWN_ACTIONS, f"{case_id} has an unknown next action")
        requirements = require_array(case["requirements"], f"{label}.requirements")
        workflows = require_array(case["workflow_diagnoses"], f"{label}.workflow_diagnoses")
        if execution["status"] == "report":
            require(error_kind is None, f"{case_id} report cannot have an error kind")
            target = require_object(case["target"], f"{label}.target")
            require_exact_keys(
                target,
                {"base_sha", "kind", "queue_entry_id", "queue_state", "resolution", "sha"},
                f"{label}.target",
            )
            require(target["kind"] in {"merge_group", "pr_head", "test_merge"}, f"{case_id} target kind is invalid")
            require(target["resolution"] in {"provisional", "selected"}, f"{case_id} target resolution is invalid")
            sha = require_string(target["sha"], f"{label}.target.sha")
            base_sha = require_string(target["base_sha"], f"{label}.target.base_sha")
            require(bool(OID_PATTERN.fullmatch(sha)), f"{case_id} target SHA is invalid")
            require(bool(OID_PATTERN.fullmatch(base_sha)), f"{case_id} target base SHA is invalid")
            require_string(target["queue_entry_id"], f"{label}.target.queue_entry_id")
            require_string(target["queue_state"], f"{label}.target.queue_state")
            require(case["verdict"] in VERDICTS, f"{case_id} verdict is invalid")
        else:
            require(error_kind is not None, f"{case_id} non-report result needs an error kind")
            require(case["target"] is None and case["verdict"] is None, f"{case_id} non-report result leaked a report")
            require(not requirements and not workflows and not actions, f"{case_id} non-report result contains report fields")

        requirement_keys: set[tuple[str, int | None]] = set()
        for requirement_index, item_value in enumerate(requirements):
            requirement_label = f"{label}.requirements[{requirement_index}]"
            requirement = require_object(item_value, requirement_label)
            require_exact_keys(
                requirement,
                {"context", "evidence", "expected_app_id", "policies", "status"},
                requirement_label,
            )
            context = require_string(requirement["context"], f"{requirement_label}.context")
            app_id = require_nullable_int(requirement["expected_app_id"], f"{requirement_label}.expected_app_id")
            require(requirement["status"] in REQUIREMENT_STATUSES, f"{requirement_label}.status is invalid")
            policies = require_array(requirement["policies"], f"{requirement_label}.policies")
            require(bool(policies), f"{requirement_label}.policies must not be empty")
            policy_keys = [
                json.dumps(validate_policy(policy, f"{requirement_label}.policies[{policy_index}]"), sort_keys=True)
                for policy_index, policy in enumerate(policies)
            ]
            require(len(policy_keys) == len(set(policy_keys)), f"{case_id} repeats a policy")
            evidence = require_array(requirement["evidence"], f"{requirement_label}.evidence")
            evidence_keys = []
            for evidence_index, evidence_value in enumerate(evidence):
                validated = validate_requirement_evidence(
                    evidence_value, f"{requirement_label}.evidence[{evidence_index}]"
                )
                evidence_keys.append(json.dumps(validated, sort_keys=True))
            require(len(evidence_keys) == len(set(evidence_keys)), f"{case_id} repeats requirement evidence")
            require((context, app_id) not in requirement_keys, f"{case_id} repeats a requirement")
            requirement_keys.add((context, app_id))

        workflow_keys: set[tuple[str, int | None]] = set()
        for workflow_index, item_value in enumerate(workflows):
            workflow_label = f"{label}.workflow_diagnoses[{workflow_index}]"
            workflow = require_object(item_value, workflow_label)
            require_exact_keys(workflow, {"diagnosis", "producer", "requirement"}, workflow_label)
            requirement = require_object(workflow["requirement"], f"{workflow_label}.requirement")
            require_exact_keys(requirement, {"context", "expected_app_id"}, f"{workflow_label}.requirement")
            key = (
                require_string(requirement["context"], f"{workflow_label}.requirement.context"),
                require_nullable_int(requirement["expected_app_id"], f"{workflow_label}.requirement.expected_app_id"),
            )
            require(key in requirement_keys, f"{case_id} workflow diagnosis has no requirement")
            require(key not in workflow_keys, f"{case_id} repeats a workflow diagnosis")
            workflow_keys.add(key)
            producer = validate_producer(workflow["producer"], f"{workflow_label}.producer")
            if producer is not None:
                require(producer["check_name"] == key[0], f"{case_id} producer check name differs from requirement")
                require(producer["app_id"] == key[1], f"{case_id} producer App differs from requirement")
            if workflow["diagnosis"] is not None:
                diagnosis = require_object(workflow["diagnosis"], f"{workflow_label}.diagnosis")
                require_exact_keys(diagnosis, {"cause_code", "confidence", "evidence", "fix_action_code"}, f"{workflow_label}.diagnosis")
                cause = require_string(diagnosis["cause_code"], f"{workflow_label}.diagnosis.cause_code")
                require(cause in CONCRETE_CAUSES | ABSTAINING_CAUSES | {"none"}, f"{case_id} cause is invalid")
                require(diagnosis["confidence"] in {"certain", "high", "uncertain"}, f"{case_id} confidence is invalid")
                validate_string_array(diagnosis["evidence"], f"{workflow_label}.diagnosis.evidence")
                action = require_string(diagnosis["fix_action_code"], f"{workflow_label}.diagnosis.fix_action_code")
                require(action in KNOWN_ACTIONS, f"{case_id} workflow fix action is invalid")
        claims = evidence_claims(case)
        claim_keys = [json.dumps(claim, sort_keys=True) for claim in claims]
        require(len(claim_keys) == len(set(claim_keys)), f"{case_id} repeats an evidence claim")
        result[case_id] = case
    require(set(result) == set(CASE_IDS), "predictions must cover every case exactly once")
    return result


def prediction_decision(prediction: dict[str, object]) -> tuple[str, str | None, str | None, str | None]:
    execution = require_object(prediction["execution"], "prediction.execution")
    if execution["status"] == "retry":
        return "retry", None, None, None
    if execution["status"] == "error":
        return "error", None, None, None
    requirements = require_array(prediction["requirements"], "prediction.requirements")
    if any(require_object(item, "requirement")["status"] == "source_mismatch" for item in requirements):
        return "diagnose", "required_check_source_mismatch", None, None
    diagnoses = []
    for item in require_array(prediction["workflow_diagnoses"], "prediction.workflow_diagnoses"):
        workflow = require_object(item, "workflow diagnosis")
        if workflow["diagnosis"] is not None:
            diagnoses.append(require_object(workflow["diagnosis"], "diagnosis"))
    if not diagnoses:
        return "abstain", None, None, None
    require(len(diagnoses) == 1, "seed prediction supports one workflow diagnosis")
    diagnosis = diagnoses[0]
    cause = str(diagnosis["cause_code"])
    if cause in ABSTAINING_CAUSES:
        return "abstain", cause, str(diagnosis["confidence"]), str(diagnosis["fix_action_code"])
    if cause == "none":
        return "no_cause", cause, str(diagnosis["confidence"]), str(diagnosis["fix_action_code"])
    return "diagnose", cause, str(diagnosis["confidence"]), str(diagnosis["fix_action_code"])


def evidence_claims(prediction: dict[str, object]) -> list[dict[str, object]]:
    claims: list[dict[str, object]] = []
    for item in require_array(prediction["requirements"], "prediction.requirements"):
        requirement = require_object(item, "requirement")
        claims.extend(
            dict(require_object(evidence, "requirement evidence"))
            for evidence in require_array(requirement["evidence"], "requirement.evidence")
        )
    for item in require_array(prediction["workflow_diagnoses"], "prediction.workflow_diagnoses"):
        workflow = require_object(item, "workflow diagnosis")
        if workflow["producer"] is not None:
            producer = dict(require_object(workflow["producer"], "producer"))
            producer["kind"] = "producer_binding"
            claims.append(producer)
        if workflow["diagnosis"] is not None:
            diagnosis = require_object(workflow["diagnosis"], "diagnosis")
            claims.extend(
                diagnosis_claim(str(value))
                for value in require_array(diagnosis["evidence"], "diagnosis.evidence")
            )
    return claims


def metric(numerator: int, denominator: int) -> dict[str, object]:
    if denominator == 0:
        return {"basis_points": None, "denominator": 0, "numerator": numerator, "status": "undefined"}
    return {
        "basis_points": (numerator * 10_000 + denominator // 2) // denominator,
        "denominator": denominator,
        "numerator": numerator,
        "status": "defined",
    }


def score_predictions(
    predictions: dict[str, dict[str, object]],
    oracle: dict[str, dict[str, object]],
    derived: dict[str, dict[str, object]],
    enforce_gates: bool,
) -> dict[str, object]:
    decisive = 0
    decisive_correct = 0
    false_confident = 0
    unsafe_reports = 0
    false_clear = 0
    expected_abstentions = 0
    predicted_abstentions = 0
    correct_abstentions = 0
    expected_retries = 0
    predicted_retries = 0
    correct_retries = 0
    emitted_actions = 0
    forbidden_actions = 0
    emitted_evidence = 0
    entailed_evidence = 0
    required_evidence = 0
    recovered_required_evidence = 0
    target_total = 0
    target_correct = 0
    requirement_total = 0
    requirement_correct = 0
    exact_cases = 0

    case_results: list[dict[str, object]] = []
    for case_id in CASE_IDS:
        prediction = predictions[case_id]
        truth = oracle[case_id]
        expected = require_object(truth["expected"], "oracle expected")
        forbidden = require_object(truth["forbidden"], "oracle forbidden")
        identifiability = require_object(truth["identifiability"], "oracle identifiability")
        ground = require_object(truth["ground_truth"], "oracle ground truth")
        disposition, cause, confidence, action = prediction_decision(prediction)
        execution = require_object(prediction["execution"], "execution")

        expected_disposition = str(expected["disposition"])
        if disposition in {"diagnose", "no_cause"}:
            decisive += 1
            correct = (
                disposition == expected_disposition
                and cause == expected["cause_code"]
                and confidence == expected["confidence"]
                and action == expected["action_code"]
            )
            decisive_correct += int(correct)
            false_confident += int(
                identifiability["cause"] != "identifiable"
                or cause != ground["cause_code"]
            )

        expected_abstentions += int(expected_disposition == "abstain")
        predicted_abstentions += int(disposition == "abstain")
        correct_abstentions += int(disposition == "abstain" and expected_disposition == "abstain")
        expected_retries += int(expected_disposition == "retry")
        predicted_retries += int(disposition == "retry")
        correct_retries += int(
            disposition == "retry"
            and expected_disposition == "retry"
            and execution["error_kind"] == expected["error_kind"]
        )

        target_matches = False
        requirement_matches = False
        if execution["status"] == "report":
            unsafe_reports += int("checks_clear" in require_array(forbidden["verdicts"], "forbidden verdicts"))
            false_clear += int(prediction["verdict"] in require_array(forbidden["verdicts"], "forbidden verdicts"))
            target_matches = prediction["target"] == ground["target"]
            predicted_requirements = require_array(prediction["requirements"], "requirements")
            requirement_matches = (
                len(predicted_requirements) == 1
                and {
                    "context": require_object(predicted_requirements[0], "requirement")["context"],
                    "expected_app_id": require_object(predicted_requirements[0], "requirement")["expected_app_id"],
                    "policies": require_object(predicted_requirements[0], "requirement")["policies"],
                    "status": require_object(predicted_requirements[0], "requirement")["status"],
                }
                == ground["requirement"]
            )
        if identifiability["target"] == "identifiable":
            target_total += 1
            target_correct += int(target_matches)
        if identifiability["requirement"] == "identifiable":
            requirement_total += 1
            requirement_correct += int(requirement_matches)

        top_actions = validate_string_array(prediction["next_action_codes"], "next actions")
        expected_top_actions = {"complete_collection"} if expected_disposition != "retry" else set()
        top_actions_exact = set(top_actions) == expected_top_actions
        all_actions = list(top_actions)
        if action is not None:
            all_actions.append(action)
        allowed = set(validate_string_array(expected["allowed_action_codes"], "allowed actions"))
        explicitly_forbidden = set(validate_string_array(forbidden["action_codes"], "forbidden actions"))
        emitted_actions += len(all_actions)
        forbidden_actions += sum(item not in allowed or item in explicitly_forbidden for item in all_actions)

        actual_claims = evidence_claims(prediction)
        allowed_claims = derived[case_id]["expected"]["required_evidence"]
        expected_claims = require_array(expected["required_evidence"], "required evidence")
        actual_keys = {json.dumps(item, sort_keys=True) for item in actual_claims}
        allowed_keys = {json.dumps(item, sort_keys=True) for item in require_array(allowed_claims, "allowed evidence")}
        required_keys = {json.dumps(item, sort_keys=True) for item in expected_claims}
        emitted_evidence += len(actual_keys)
        entailed_evidence += len(actual_keys & allowed_keys)
        required_evidence += len(required_keys)
        recovered_required_evidence += len(actual_keys & required_keys)

        exact = (
            disposition == expected_disposition
            and cause == expected["cause_code"]
            and confidence == expected["confidence"]
            and action == expected["action_code"]
            and (
                expected_disposition != "retry"
                or execution["error_kind"] == expected["error_kind"]
            )
            and (prediction["verdict"] in expected["verdict_allowlist"] if expected["verdict_allowlist"] else prediction["verdict"] is None)
            and (identifiability["target"] != "identifiable" or target_matches)
            and (identifiability["requirement"] != "identifiable" or requirement_matches)
            and top_actions_exact
            and not any(item not in allowed or item in explicitly_forbidden for item in all_actions)
            and actual_keys <= allowed_keys
            and required_keys <= actual_keys
        )
        exact_cases += int(exact)
        case_results.append(
            {
                "case_id": case_id,
                "expected_disposition": expected_disposition,
                "observed_disposition": disposition,
                "passed": exact,
            }
        )

    metrics = {
        "abstention_precision": metric(correct_abstentions, predicted_abstentions),
        "abstention_recall": metric(correct_abstentions, expected_abstentions),
        "evidence_entailment": metric(entailed_evidence, emitted_evidence),
        "exact_case_rate": metric(exact_cases, len(CASE_IDS)),
        "exact_target_accuracy": metric(target_correct, target_total),
        "false_checks_clear_rate": metric(false_clear, unsafe_reports),
        "false_confident_rate": metric(false_confident, decisive),
        "forbidden_action_rate": metric(forbidden_actions, emitted_actions),
        "required_evidence_recall": metric(recovered_required_evidence, required_evidence),
        "requirement_accuracy": metric(requirement_correct, requirement_total),
        "retry_precision": metric(correct_retries, predicted_retries),
        "retry_recall": metric(correct_retries, expected_retries),
        "selective_accuracy": metric(decisive_correct, decisive),
        "selective_coverage": metric(decisive, len(CASE_IDS)),
    }
    gates = {
        "abstention_precision_100_percent": metrics["abstention_precision"]["basis_points"] == 10_000,
        "abstention_recall_100_percent": metrics["abstention_recall"]["basis_points"] == 10_000,
        "evidence_entailment_100_percent": metrics["evidence_entailment"]["basis_points"] == 10_000,
        "exact_target_accuracy_100_percent": metrics["exact_target_accuracy"]["basis_points"] == 10_000,
        "false_checks_clear_zero": metrics["false_checks_clear_rate"]["numerator"] == 0,
        "false_confident_zero": metrics["false_confident_rate"]["numerator"] == 0,
        "forbidden_action_zero": metrics["forbidden_action_rate"]["numerator"] == 0,
        "required_evidence_recall_100_percent": metrics["required_evidence_recall"]["basis_points"] == 10_000,
        "requirement_accuracy_100_percent": metrics["requirement_accuracy"]["basis_points"] == 10_000,
        "retry_precision_100_percent": metrics["retry_precision"]["basis_points"] == 10_000,
        "retry_recall_100_percent": metrics["retry_recall"]["basis_points"] == 10_000,
        "selective_accuracy_100_percent": metrics["selective_accuracy"]["basis_points"] == 10_000,
        "selective_coverage_at_least_2500bp": int(metrics["selective_coverage"]["basis_points"]) >= 2_500,
    }
    passed = all(gates.values()) and exact_cases == len(CASE_IDS)
    if enforce_gates:
        require(passed, f"prediction gates failed: {[name for name, ok in gates.items() if not ok]}; exact cases {exact_cases}/{len(CASE_IDS)}")
    return {
        "case_results": case_results,
        "dataset_version": VERSION,
        "gates": gates,
        "metrics": metrics,
        "passed": passed,
        "schema": EVALUATION_SCHEMA,
        "summary": {
            "cases": len(CASE_IDS),
            "decisive": decisive,
            "expected_abstentions": expected_abstentions,
            "expected_retries": expected_retries,
            "exact_cases": exact_cases,
        },
    }


def validate_manifest(
    value: dict[str, object],
    assets: dict[str, bytes],
    evaluation: dict[str, object],
) -> None:
    require_exact_keys(
        value,
        {
            "acceptance_gates",
            "assets",
            "claim_boundary",
            "coverage",
            "dataset_license",
            "dataset_version",
            "designation",
            "name",
            "provenance",
            "schema",
        },
        "manifest",
    )
    require(value["schema"] == MANIFEST_SCHEMA, "manifest schema changed")
    require(value["dataset_version"] == VERSION, "manifest version changed")
    require(value["name"] == "MergeForensicsBench v1", "manifest name changed")
    require(value["designation"] == "controlled_raw_provider_replay", "manifest designation changed")
    require(value["dataset_license"] == "MIT", "manifest license changed")
    manifest_assets = require_object(value["assets"], "manifest.assets")
    require(set(manifest_assets) == set(assets), "manifest asset set changed")
    for name, payload in assets.items():
        item = require_object(manifest_assets[name], f"manifest.assets.{name}")
        require_exact_keys(item, {"bytes", "path", "sha256"}, f"manifest.assets.{name}")
        require(item["path"] == name, f"manifest asset path changed for {name}")
        require(item["bytes"] == len(payload), f"manifest asset byte count changed for {name}")
        require(item["sha256"] == sha256_bytes(payload), f"manifest asset hash changed for {name}")

    provenance = require_array(value["provenance"], "manifest.provenance")
    provenance_ids: set[str] = set()
    for index, item in enumerate(provenance):
        label = f"manifest.provenance[{index}]"
        source = require_object(item, label)
        require_exact_keys(source, {"id", "kind", "url"}, label)
        source_id = require_string(source["id"], f"{label}.id")
        require(source_id not in provenance_ids, f"duplicate provenance {source_id}")
        require(source["kind"] in {"official_documentation", "public_incident"}, f"{source_id} kind is invalid")
        url = require_string(source["url"], f"{label}.url")
        require(url.startswith("https://"), f"{source_id} URL is invalid")
        provenance_ids.add(source_id)
    require(provenance_ids == EXPECTED_PROVENANCE_IDS, "manifest provenance set changed")

    coverage = require_object(value["coverage"], "manifest.coverage")
    require_exact_keys(coverage, {"requirement_statuses", "scenario_categories"}, "manifest.coverage")
    statuses = validate_string_array(coverage["requirement_statuses"], "manifest.coverage.requirement_statuses")
    require(statuses == ["missing", "source_mismatch"], "requirement-status coverage changed")
    categories = validate_string_array(coverage["scenario_categories"], "manifest.coverage.scenario_categories")
    require(categories == EXPECTED_COVERAGE, "manifest scenario coverage set or order changed")
    require(not any(CASE_ID_PATTERN.fullmatch(item) for item in categories), "coverage leaks case mappings")

    gates = require_object(value["acceptance_gates"], "manifest.acceptance_gates")
    require_exact_keys(gates, {"expected_metrics", "minimum_cases"}, "manifest.acceptance_gates")
    require(gates["minimum_cases"] == 12, "minimum case gate changed")
    require(gates["expected_metrics"] == evaluation["metrics"], "manifest expected metrics changed")

    boundary = require_object(value["claim_boundary"], "manifest.claim_boundary")
    require(
        boundary
        == {
            "controlled_collector_report_replay_supported": True,
            "end_to_end_gh_cli_transport_supported": False,
            "failure_prevalence_supported": False,
            "live_api_currentness_supported": False,
            "market_demand_supported": False,
            "merge_safety_supported": False,
            "production_accuracy_supported": False,
        },
        "manifest claim boundary changed",
    )


def validate_bundle_entries(paths: set[str]) -> None:
    require(paths == EXPECTED_FILES | {"SHA256SUMS"}, "bundle contains an unexpected entry")


def verify_checksums() -> None:
    lines = CHECKSUMS_PATH.read_text().splitlines()
    observed: dict[str, str] = {}
    for index, line in enumerate(lines):
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9_.-]+)", line)
        require(match is not None, f"invalid checksum line {index + 1}")
        assert match is not None
        file_hash, name = match.groups()
        require(name not in observed, f"duplicate checksum entry {name}")
        require(name != "SHA256SUMS", "checksum file cannot hash itself")
        observed[name] = file_hash
    require(set(observed) == EXPECTED_FILES, "checksum file set changed")
    entries = {path.relative_to(BUNDLE).as_posix(): path for path in BUNDLE.rglob("*")}
    validate_bundle_entries(set(entries))
    require(
        all(path.is_file() and not path.is_symlink() for path in entries.values()),
        "bundle entries must be regular files",
    )
    for name, expected in observed.items():
        require(sha256_file(BUNDLE / name) == expected, f"checksum mismatch for {name}")


def load_verified() -> tuple[
    list[dict[str, object]],
    dict[str, dict[str, object]],
    dict[str, object],
    dict[str, dict[str, object]],
    dict[str, dict[str, object]],
    dict[str, object],
]:
    cases_bytes, cases_value = read_json(CASES_PATH)
    transcripts_bytes, transcripts_value = read_json(TRANSCRIPTS_PATH)
    oracle_bytes, oracle_value = read_json(ORACLE_PATH)
    baseline_bytes, baseline_value = read_json(BASELINE_PATH)
    manifest_bytes, manifest_value = read_json(MANIFEST_PATH)
    cases = validate_cases(cases_value)
    transcripts, payloads = validate_transcripts(transcripts_value, cases)
    oracle = validate_oracle(oracle_value, transcripts, payloads)
    baseline = validate_predictions(baseline_value)
    derived = {case_id: derive_observable_case(transcripts[case_id], payloads) for case_id in CASE_IDS}
    evaluation = score_predictions(baseline, oracle, derived, True)
    validate_manifest(
        manifest_value,
        {
            "baseline-predictions.json": baseline_bytes,
            "cases.json": cases_bytes,
            "oracle.json": oracle_bytes,
            "transcripts.json": transcripts_bytes,
        },
        evaluation,
    )
    require(bool(manifest_bytes), "manifest must not be empty")
    verify_checksums()
    return cases, transcripts, payloads, oracle, baseline, evaluation


def command_verify() -> None:
    _, _, _, _, _, evaluation = load_verified()
    print(json.dumps(evaluation, indent=2, sort_keys=True))


def command_score(path: Path) -> None:
    _, transcripts, payloads, oracle, _, _ = load_verified()
    value = decode_json_bytes(path.read_bytes(), str(path))
    require(type(value) is dict, f"{path} must contain one JSON object")
    assert isinstance(value, dict)
    predictions = validate_predictions(value)
    derived = {case_id: derive_observable_case(transcripts[case_id], payloads) for case_id in CASE_IDS}
    evaluation = score_predictions(predictions, oracle, derived, True)
    print(json.dumps(evaluation, indent=2, sort_keys=True))


def expect_failure(label: str, action: Callable[[], object]) -> None:
    try:
        action()
    except (BenchmarkError, KeyError, ValueError, TypeError, UnicodeError):
        return
    raise BenchmarkError(f"self-test mutation survived: {label}")


def command_self_test() -> None:
    cases_bytes, cases_value = read_json(CASES_PATH)
    transcripts_bytes, transcripts_value = read_json(TRANSCRIPTS_PATH)
    oracle_bytes, oracle_value = read_json(ORACLE_PATH)
    baseline_bytes, baseline_value = read_json(BASELINE_PATH)
    _, manifest_value = read_json(MANIFEST_PATH)
    cases = validate_cases(cases_value)
    transcripts, payloads = validate_transcripts(transcripts_value, cases)
    oracle = validate_oracle(oracle_value, transcripts, payloads)
    baseline = validate_predictions(baseline_value)
    derived = {case_id: derive_observable_case(transcripts[case_id], payloads) for case_id in CASE_IDS}
    evaluation = score_predictions(baseline, oracle, derived, True)
    assets = {
        "baseline-predictions.json": baseline_bytes,
        "cases.json": cases_bytes,
        "oracle.json": oracle_bytes,
        "transcripts.json": transcripts_bytes,
    }
    validate_manifest(manifest_value, assets, evaluation)

    mutated = copy.deepcopy(transcripts_value)
    mutated["payloads"][0]["body_utf8"] += " "
    expect_failure("payload hash", lambda: validate_transcripts(mutated, cases))

    mutated = copy.deepcopy(transcripts_value)
    mutated["payloads"][0]["byte_length"] += 1
    expect_failure("payload byte length", lambda: validate_transcripts(mutated, cases))

    mutated = copy.deepcopy(transcripts_value)
    payload = require_object(mutated["payloads"][0], "mutated payload")
    body = require_object(decode_json_bytes(str(payload["body_utf8"]).encode(), "mutated body"), "mutated body")
    body["label"] = "innocuous"
    body_bytes = json.dumps(body, ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()
    payload["body_utf8"] = body_bytes.decode()
    payload["byte_length"] = len(body_bytes)
    payload["sha256"] = sha256_bytes(body_bytes)
    expect_failure("synchronized payload rewrite", lambda: validate_transcripts(mutated, cases))

    mutated = copy.deepcopy(transcripts_value)
    del mutated["transcripts"][0]["exchanges"][1]
    expect_failure("dropped ordered exchange", lambda: validate_transcripts(mutated, cases))

    mutated = copy.deepcopy(transcripts_value)
    mutated["transcripts"][0]["exchanges"][2]["request"]["variables"]["number"] = 18
    expect_failure("GraphQL variables", lambda: validate_transcripts(mutated, cases))

    mutated = copy.deepcopy(transcripts_value)
    mutated["query_contracts"][0]["sha256"] = "0" * 64
    expect_failure("query contract binding", lambda: validate_transcripts(mutated, cases))

    mutated = copy.deepcopy(transcripts_value)
    first = mutated["transcripts"][0]["exchanges"]
    first[0], first[1] = first[1], first[0]
    first[0]["sequence"], first[1]["sequence"] = 1, 2
    expect_failure("reordered exchange trace", lambda: validate_transcripts(mutated, cases))

    expect_failure(
        "duplicate JSON key",
        lambda: decode_json_bytes(b'{"schema":"first","schema":"second"}', "duplicate fixture"),
    )

    mutated_payloads = copy.deepcopy(payloads)
    graphql = graphql_exchanges(transcripts["c001"])[0]
    graphql_payload_id = str(require_object(graphql["response"], "GraphQL response")["payload_id"])
    graphql_body = require_object(mutated_payloads[graphql_payload_id], "GraphQL body")
    graphql_body["errors"] = [{"message": "partial response"}]
    expect_failure(
        "GraphQL data plus errors",
        lambda: derive_observable_case(transcripts["c001"], mutated_payloads),
    )

    mutated_transcript = copy.deepcopy(transcripts["c001"])
    first_exchange = require_object(require_array(mutated_transcript["exchanges"], "exchanges")[0], "exchange")
    require_object(first_exchange["response"], "response")["status"] = 500
    expect_failure(
        "non-success evidence response",
        lambda: derive_observable_case(mutated_transcript, payloads),
    )

    mutated_transcript = copy.deepcopy(transcripts["c002"])
    target_runs_endpoint = (
        "repos/acme/merge-lab/actions/runs?"
        "head_sha=eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee&per_page=100&page=1"
    )
    target_runs = rest_exchanges(mutated_transcript, target_runs_endpoint)
    require(len(target_runs) == 2, "c002 must contain two target-run observations")
    require_object(target_runs[1]["response"], "target-run response")["payload_id"] = "synthetic-target-run-drift"
    mutated_payloads = copy.deepcopy(payloads)
    mutated_payloads["synthetic-target-run-drift"] = {"total_count": 0, "workflow_runs": []}
    expect_failure(
        "target workflow-run pass drift",
        lambda: derive_observable_case(mutated_transcript, mutated_payloads),
    )

    producer_endpoint = (
        "repos/acme/merge-lab/commits/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/"
        "check-runs?check_name=ci&app_id=15368&filter=all&per_page=100&page=1"
    )
    producer_exchange = rest_exchanges(transcripts["c001"], producer_endpoint)[0]
    producer_payload_id = str(require_object(producer_exchange["response"], "producer response")["payload_id"])
    mutated_payloads = copy.deepcopy(payloads)
    producer_body = require_object(mutated_payloads[producer_payload_id], "producer body")
    producer_checks = require_array(producer_body["check_runs"], "producer checks")
    require_object(producer_checks[0], "producer check")["head_sha"] = "0" * 40
    expect_failure(
        "producer check source mismatch",
        lambda: derive_observable_case(transcripts["c001"], mutated_payloads),
    )

    mutated = copy.deepcopy(cases_value)
    mutated["cases"][0]["cause_code"] = "merge_group_trigger_missing"
    expect_failure("oracle leakage", lambda: validate_cases(mutated))

    mutated = copy.deepcopy(cases_value)
    mutated["cases"][0]["provenance_ids"] = ["cargo-dist-1069"]
    expect_failure("per-case provenance side channel", lambda: validate_cases(mutated))

    mutated = copy.deepcopy(oracle_value)
    mutated["cases"]["c001"]["expected"]["cause_code"] = "none"
    expect_failure("forged oracle", lambda: validate_oracle(mutated, transcripts, payloads))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"][0]["target"]["sha"] = "0" * 40
    changed = validate_predictions(mutated)
    expect_failure("wrong target", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"][0]["target"]["resolution"] = "selected"
    changed = validate_predictions(mutated)
    expect_failure("wrong target resolution", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"][0]["target"]["queue_entry_id"] = "MQE_wrong"
    changed = validate_predictions(mutated)
    expect_failure("wrong target queue identity", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"][0]["requirements"][0]["policies"][0]["id"] = "71"
    changed = validate_predictions(mutated)
    expect_failure("wrong requirement policy", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"][0]["workflow_diagnoses"][0]["producer"]["source_sha"] = "0" * 40
    changed = validate_predictions(mutated)
    expect_failure("wrong producer identity", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    for prediction in mutated["cases"]:
        if prediction["execution"]["status"] == "report":
            prediction["next_action_codes"] = []
    changed = validate_predictions(mutated)
    expect_failure("missing complete-collection actions", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    evidence = mutated["cases"][9]["requirements"][0]["evidence"]
    evidence.append(copy.deepcopy(evidence[0]))
    expect_failure("duplicate requirement evidence", lambda: validate_predictions(mutated))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"][2]["workflow_diagnoses"][0]["diagnosis"] = {
        "cause_code": "merge_group_trigger_missing",
        "confidence": "certain",
        "evidence": ["trigger:merge_group_absent"],
        "fix_action_code": "add_merge_group_trigger",
    }
    changed = validate_predictions(mutated)
    expect_failure("false confident diagnosis", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"][0]["verdict"] = "checks_clear"
    changed = validate_predictions(mutated)
    expect_failure("false checks-clear", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"][4]["next_action_codes"] = ["enable_workflow"]
    changed = validate_predictions(mutated)
    expect_failure("forbidden action", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"][0]["workflow_diagnoses"][0]["diagnosis"]["evidence"].append("run:success")
    changed = validate_predictions(mutated)
    expect_failure("fabricated evidence", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"][0]["workflow_diagnoses"][0]["diagnosis"]["evidence"].pop()
    changed = validate_predictions(mutated)
    expect_failure("missing required evidence", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"][7]["execution"]["error_kind"] = "producer_drift"
    changed = validate_predictions(mutated)
    expect_failure("wrong retry class", lambda: score_predictions(changed, oracle, derived, True))

    mutated = copy.deepcopy(baseline_value)
    mutated["cases"].pop()
    expect_failure("missing prediction", lambda: validate_predictions(mutated))

    mutated_manifest = copy.deepcopy(manifest_value)
    mutated_manifest["coverage"] = {"candidate_drift": "c008"}
    expect_failure(
        "coverage case mapping",
        lambda: validate_manifest(mutated_manifest, assets, evaluation),
    )

    expect_failure(
        "nested bundle entry",
        lambda: validate_bundle_entries(EXPECTED_FILES | {"SHA256SUMS", "nested/extra.txt"}),
    )

    print("MergeForensicsBench v1 self-test passed (31 mutations rejected)")


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("verify")
    subparsers.add_parser("self-test")
    score = subparsers.add_parser("score")
    score.add_argument("predictions", type=Path)
    arguments = parser.parse_args()
    if arguments.command == "verify":
        command_verify()
    elif arguments.command == "self-test":
        command_self_test()
    else:
        command_score(arguments.predictions)


if __name__ == "__main__":
    main()
