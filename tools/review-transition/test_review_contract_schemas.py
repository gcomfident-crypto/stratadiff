#!/usr/bin/env python3

import copy
import json
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parent
CONTEXT_SCHEMA = ROOT / "review-context-v1.schema.json"
RECEIPT_SCHEMA = ROOT / "review-receipt-v1.schema.json"
INPUT_SCHEMA = ROOT / "review-input-v1.schema.json"
CONTEXT_ID = (
    "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/"
    "tools/review-transition/review-context-v1.schema.json"
)
RECEIPT_ID = (
    "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/"
    "tools/review-transition/review-receipt-v1.schema.json"
)
INPUT_ID = (
    "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/"
    "tools/review-transition/review-input-v1.schema.json"
)


class ContractValidationError(ValueError):
    pass


class Draft202012SubsetValidator:
    """Validates the standard keyword subset intentionally used by these schemas."""

    def __init__(self, schema):
        self.schema = schema

    def validate(self, instance):
        self._validate(instance, self.schema, "$")

    def _resolve(self, reference):
        if not reference.startswith("#/"):
            raise ContractValidationError(f"unsupported non-local reference: {reference}")
        value = self.schema
        for component in reference[2:].split("/"):
            component = component.replace("~1", "/").replace("~0", "~")
            value = value[component]
        return value

    def _is_valid(self, instance, schema):
        try:
            self._validate(instance, schema, "$")
        except ContractValidationError:
            return False
        return True

    def _validate(self, instance, schema, path):
        if isinstance(schema, bool):
            if not schema:
                raise ContractValidationError(f"{path}: false schema")
            return

        if "$ref" in schema:
            self._validate(instance, self._resolve(schema["$ref"]), path)

        if "allOf" in schema:
            for child in schema["allOf"]:
                self._validate(instance, child, path)

        if "if" in schema:
            branch = None
            if self._is_valid(instance, schema["if"]):
                branch = schema["then"]
            elif "else" in schema:
                branch = schema["else"]
            if branch is not None:
                self._validate(instance, branch, path)

        if "oneOf" in schema:
            matches = sum(self._is_valid(instance, child) for child in schema["oneOf"])
            if matches != 1:
                raise ContractValidationError(f"{path}: expected one matching branch, found {matches}")

        expected_type = schema.get("type")
        if expected_type is not None:
            type_matches = {
                "object": isinstance(instance, dict),
                "array": isinstance(instance, list),
                "string": isinstance(instance, str),
                "integer": isinstance(instance, int) and not isinstance(instance, bool),
                "boolean": isinstance(instance, bool),
                "null": instance is None,
            }
            if not type_matches[expected_type]:
                raise ContractValidationError(f"{path}: expected {expected_type}")

        if "const" in schema and instance != schema["const"]:
            raise ContractValidationError(f"{path}: const differs")
        if "enum" in schema and instance not in schema["enum"]:
            raise ContractValidationError(f"{path}: value is outside enum")

        if isinstance(instance, dict):
            required = schema.get("required", [])
            missing = set(required) - set(instance)
            if missing:
                raise ContractValidationError(f"{path}: missing {sorted(missing)}")
            properties = schema.get("properties", {})
            if schema.get("additionalProperties") is False:
                extras = set(instance) - set(properties)
                if extras:
                    raise ContractValidationError(f"{path}: extra {sorted(extras)}")
            for name, value in instance.items():
                if name in properties:
                    self._validate(value, properties[name], f"{path}.{name}")

        if isinstance(instance, list):
            if len(instance) < schema.get("minItems", 0):
                raise ContractValidationError(f"{path}: too few items")
            if "maxItems" in schema and len(instance) > schema["maxItems"]:
                raise ContractValidationError(f"{path}: too many items")
            if schema.get("uniqueItems"):
                encoded = [json.dumps(value, sort_keys=True, separators=(",", ":")) for value in instance]
                if len(encoded) != len(set(encoded)):
                    raise ContractValidationError(f"{path}: duplicate items")
            if "items" in schema:
                for index, value in enumerate(instance):
                    self._validate(value, schema["items"], f"{path}[{index}]")

        if isinstance(instance, str):
            if len(instance) < schema.get("minLength", 0):
                raise ContractValidationError(f"{path}: string is too short")
            if "maxLength" in schema and len(instance) > schema["maxLength"]:
                raise ContractValidationError(f"{path}: string is too long")
            if "pattern" in schema and re.search(schema["pattern"], instance) is None:
                raise ContractValidationError(f"{path}: pattern differs")

        if isinstance(instance, int) and not isinstance(instance, bool):
            if "minimum" in schema and instance < schema["minimum"]:
                raise ContractValidationError(f"{path}: below minimum")
            if "maximum" in schema and instance > schema["maximum"]:
                raise ContractValidationError(f"{path}: above maximum")
            if "exclusiveMinimum" in schema and instance <= schema["exclusiveMinimum"]:
                raise ContractValidationError(f"{path}: below exclusive minimum")
            if "exclusiveMaximum" in schema and instance >= schema["exclusiveMaximum"]:
                raise ContractValidationError(f"{path}: above exclusive maximum")


def validator(schema):
    try:
        import jsonschema
    except ModuleNotFoundError:
        return Draft202012SubsetValidator(schema)
    jsonschema.Draft202012Validator.check_schema(schema)
    return jsonschema.Draft202012Validator(schema)


def digest(character):
    return character * 64


def oid(character):
    return character * 40


def repository():
    return {
        "provider": "github",
        "host": "github.com",
        "owner": "acme",
        "name": "widget",
        "repository_id": "R_kgDOExample",
        "object_format": "sha1",
    }


def context_fixture():
    return {
        "schema": CONTEXT_ID,
        "contract_version": "1.0.0",
        "canonicalization": "stratadiff-canonical-json-v1",
        "body": {
            "reuse_policy": "exact_git_change_identity_only_v1",
            "change_identity_schema": "stratadiff-exact-git-change-identity-v1",
            "review_input_scope": "selected_payload_only",
            "dependency_closure": {
                "status": "closed",
                "manifest_sha256": digest("0"),
            },
            "reviewer": {
                "implementation": {
                    "name": "reviewer",
                    "version": "1.2.3",
                    "entrypoint": "review",
                    "artifact_sha256": digest("1"),
                },
                "model": {
                    "kind": "model",
                    "provider": "model-provider",
                    "model_id": "review-model",
                    "immutable_revision": "2026-09-01",
                    "model_artifact_sha256": digest("2"),
                    "inference_parameters_sha256": digest("3"),
                },
                "prompt": {
                    "schema_uri": "urn:acme:prompt:v1",
                    "canonical_sha256": digest("4"),
                    "byte_length": 100,
                },
                "policy": {
                    "schema_uri": "urn:acme:policy:v1",
                    "canonical_sha256": digest("5"),
                    "byte_length": 50,
                },
                "configuration": {
                    "schema_uri": "urn:acme:config:v1",
                    "canonical_sha256": digest("6"),
                    "byte_length": 40,
                },
                "runtime": {
                    "execution_kind": "container",
                    "runtime_name": "python",
                    "runtime_version": "3.13.7",
                    "operating_system": "linux",
                    "architecture": "x86_64",
                    "runtime_manifest_sha256": digest("7"),
                    "environment_sha256": digest("8"),
                },
                "composition": {
                    "kind": "holistic",
                    "reviewer_input_projection": {
                        "schema_uri": "urn:stratadiff:reviewer-visible-input:v1",
                        "canonical_sha256": digest("d"),
                        "byte_length": 10,
                    },
                },
                "tools": [
                    {
                        "name": "git",
                        "version": "2.51.0",
                        "executable_sha256": digest("9"),
                        "configuration_sha256": digest("a"),
                    }
                ],
            },
            "repository": repository(),
            "repository_closure": {
                "kind": "declared_git_object_and_path_closure_v1",
                "complete": True,
                "root_commit_oids": [oid("1"), oid("2")],
                "root_tree_oids": [oid("3"), oid("4")],
                "object_count": 42,
                "path_count": 7,
                "object_manifest_sha256": digest("b"),
                "path_manifest_sha256": digest("c"),
                "canonical_closure_sha256": digest("d"),
            },
            "pull_request": {
                "node_id": "PR_kwDOExample",
                "number": 147,
                "base_ref": "main",
                "head_ref": "feature",
                "base_oid": oid("1"),
                "head_oid": oid("2"),
                "observed_at": "2026-09-06T00:00:00Z",
                "canonical_metadata_sha256": digest("e"),
            },
            "historical_dispositions": [
                {
                    "disposition_id": "disposition-exact",
                    "subject_identity_sha256": digest("1"),
                    "disposition": "approved",
                    "result_sha256": digest("2"),
                    "receipt_body_sha256": digest("3"),
                    "match_basis": "exact_git_change_identity",
                    "automatically_reusable": True,
                    "recorded_at": "2026-09-05T00:00:00Z",
                },
                {
                    "disposition_id": "disposition-replay",
                    "subject_identity_sha256": digest("4"),
                    "disposition": "commented",
                    "result_sha256": digest("5"),
                    "receipt_body_sha256": digest("6"),
                    "match_basis": "exact_noninteracting_four_way_byte_replay",
                    "automatically_reusable": False,
                    "recorded_at": "2026-09-05T01:00:00Z",
                },
            ],
        },
        "body_sha256": digest("f"),
        "compatibility_sha256": digest("e"),
    }


def receipt_fixture():
    return {
        "schema": RECEIPT_ID,
        "contract_version": "1.0.0",
        "canonicalization": "stratadiff-canonical-json-v1",
        "body": {
            "receipt_id": "receipt-147-v1",
            "issued_at": "2026-09-05T02:00:00Z",
            "issuer": {
                "source_kind": "signed_review_execution",
                "issuer_id": "review-service",
                "trust_domain": "acme-review-production",
                "trust_policy_sha256": digest("1"),
            },
            "repository": repository(),
            "pull_request": {
                "node_id": "PR_kwDOExample",
                "number": 147,
                "requested_base_oid": oid("1"),
                "reviewed_merge_base_oid": oid("1"),
                "reviewed_head_oid": oid("2"),
                "metadata_sha256": digest("2"),
            },
            "review_context_body_sha256": digest("f"),
            "review_context_compatibility_sha256": digest("e"),
            "review_repository_closure_sha256": digest("d"),
            "historical_dispositions_sha256": digest("c"),
            "review_composition": {
                "kind": "holistic",
                "reviewer_input_projection": {
                    "schema_uri": "urn:stratadiff:reviewer-visible-input:v1",
                    "canonical_sha256": digest("d"),
                    "byte_length": 10,
                },
            },
            "prior_input": {
                "schema_uri": "urn:acme:review-input:v1",
                "canonical_input_sha256": digest("3"),
                "selected_payload_sha256": digest("4"),
                "byte_length": 200,
            },
            "prior_result": {
                "schema_uri": "urn:acme:review-result:v1",
                "canonical_result_sha256": digest("5"),
                "selected_outcome": "passed",
                "retained_non_current_blocking_outcome": "passed",
                "effective_outcome": "passed",
                "verdict_scope": "reviewer_visible_input_projection",
                "reviewer_input_projection_sha256": digest("a"),
                "byte_length": 80,
            },
            "coverage": {
                "complete": True,
                "covered_input_sha256": digest("3"),
                "covered_identity_sha256": [digest("6"), digest("7")],
                "current_identity_results": [
                    {
                        "identity_sha256": digest("6"),
                        "outcome": "passed",
                        "lineage": {
                            "kind": "selected_execution",
                            "obligation_sha256": digest("6"),
                        },
                    },
                    {
                        "identity_sha256": digest("7"),
                        "outcome": "passed",
                        "lineage": {
                            "kind": "selected_execution",
                            "obligation_sha256": digest("7"),
                        },
                    },
                ],
                "omitted_identity_sha256": [],
                "unresolved_obligation_sha256": [],
            },
            "reuse_constraints": {
                "policy": "exact_git_change_identity_only_v1",
                "change_identity_schema": "stratadiff-exact-git-change-identity-v1",
                "future_reuse_requires_identical_context": True,
                "future_reuse_requires_exact_git_identity": True,
                "partial_reuse_requires_itemwise_closed_composition": True,
                "four_way_replay_authorizes_verdict_or_input_skip": False,
            },
        },
        "attestation": {
            "algorithm": "ed25519",
            "key_id": "acme-review-key-1",
            "signature_domain": "stratadiff.review-receipt",
            "signature_preimage_version": "1",
            "body_sha256": digest("8"),
            "signature": "9" * 128,
        },
    }


def snapshot(commit_character, tree_character):
    return {"commit_oid": oid(commit_character), "tree_oid": oid(tree_character)}


def receipt_reference():
    return {
        "status": "present",
        "schema_uri": RECEIPT_ID,
        "receipt_id": "receipt-147-v1",
        "body_sha256": digest("8"),
        "attestation_sha256": digest("9"),
        "prior_input_sha256": digest("3"),
        "prior_result_sha256": digest("5"),
        "review_context_body_sha256": digest("f"),
        "review_context_compatibility_sha256": digest("e"),
        "prior_outcome": "passed",
        "retained_non_current_blocking_outcome": "passed",
        "coverage_complete": True,
        "trusted_source_verified": True,
    }


def exact_context():
    return {
        "status": "exact",
        "closure_status": "closed",
        "current_body_sha256": digest("f"),
        "receipt_body_sha256": digest("f"),
        "current_compatibility_sha256": digest("e"),
        "receipt_compatibility_sha256": digest("e"),
    }


def input_base():
    return {
        "schema": INPUT_ID,
        "contract_version": "1.0.0",
        "canonicalization": "stratadiff-canonical-json-v1",
        "generated_at": "2026-09-06T03:00:00Z",
        "reuse_policy": "exact_git_change_identity_only_v1",
        "change_identity_schema": "stratadiff-exact-git-change-identity-v1",
        "repository": repository(),
        "pull_request": {
            "node_id": "PR_kwDOExample",
            "number": 147,
            "metadata_sha256": digest("2"),
        },
        "transition": {
            "route": "provider_attested_same_base",
            "Q": snapshot("1", "6"),
            "A": snapshot("1", "6"),
            "B": snapshot("2", "7"),
            "C": snapshot("1", "6"),
            "D": snapshot("3", "8"),
        },
        "current_context": {
            "schema_uri": CONTEXT_ID,
            "body_sha256": digest("f"),
            "compatibility_sha256": digest("e"),
            "closure_status": "closed",
        },
    }


def selected_payload(mode, identities):
    return {
        "mode": mode,
        "schema_uri": "urn:acme:selected-review-payload:v1",
        "canonical_payload_sha256": digest("a"),
        "reviewer_input_projection_sha256": digest("b"),
        "byte_length": 0 if not identities else 120,
        "selected_identity_sha256": identities,
    }


def accounting(coverage_complete, current, carried, selected, requirements, blocking):
    return {
        "coverage_complete": coverage_complete,
        "full_current_identity_sha256": current,
        "carried": carried,
        "selected_current_identity_sha256": selected,
        "review_required": requirements,
        "unresolved_retired_obligation_sha256": [],
        "base_drift_obligation_sha256": [],
        "retained_non_current_obligation_sha256": [],
        "blocking_reasons": blocking,
    }


def exact_carry(identity):
    return {
        "current_identity_sha256": identity,
        "receipt_identity_sha256": identity,
        "basis": "exact_git_change_identity",
        "prior_outcome": "passed",
    }


def review_requirement(identity, reason="new_or_changed_git_identity"):
    return {
        "obligation_sha256": identity,
        "reason": reason,
        "evidence_sha256": digest("b"),
    }


def skip_input():
    value = input_base()
    identities = [digest("6"), digest("7")]
    value["resolution"] = {
        "decision": "skip",
        "reason": "all_obligations_exactly_covered",
        "prior_receipt": receipt_reference(),
        "context_comparison": exact_context(),
        "selected_payload": selected_payload("empty", []),
        "accounting": accounting(
            True,
            identities,
            [exact_carry(identity) for identity in identities],
            [],
            [],
            [],
        ),
    }
    return value


def residue_input():
    value = input_base()
    carried = digest("6")
    selected = digest("7")
    value["resolution"] = {
        "decision": "residue",
        "reason": "exact_carries_removed_from_review_input",
        "prior_receipt": receipt_reference(),
        "context_comparison": exact_context(),
        "selected_payload": selected_payload("residue", [selected]),
        "accounting": accounting(
            True,
            [carried, selected],
            [exact_carry(carried)],
            [selected],
            [review_requirement(selected)],
            [],
        ),
    }
    return value


def full_input():
    value = input_base()
    identities = [digest("6"), digest("7")]
    value["resolution"] = {
        "decision": "full",
        "reason": "receipt_unavailable",
        "prior_receipt": {"status": "absent", "reason": "not_provided"},
        "context_comparison": {
            "status": "unavailable",
            "closure_status": "closed",
            "current_body_sha256": digest("f"),
            "current_compatibility_sha256": digest("e"),
            "reason": "receipt_absent",
        },
        "selected_payload": selected_payload("full", identities),
        "accounting": accounting(
            True,
            identities,
            [],
            identities,
            [review_requirement(identity) for identity in identities],
            [],
        ),
    }
    return value


def blocked_input():
    value = input_base()
    value["resolution"] = {
        "decision": "blocked",
        "reason": "source_closure_incomplete",
        "prior_receipt": {"status": "absent", "reason": "incomplete_coverage"},
        "context_comparison": {
            "status": "unavailable",
            "closure_status": "open",
            "current_body_sha256": digest("f"),
            "current_compatibility_sha256": digest("e"),
            "reason": "context_incomplete",
        },
        "selected_payload": selected_payload("blocked", []),
        "accounting": accounting(False, [], [], [], [], ["source closure is incomplete"]),
    }
    return value


class ReviewContractSchemaTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schemas = {
            path.name: json.loads(path.read_bytes())
            for path in (CONTEXT_SCHEMA, RECEIPT_SCHEMA, INPUT_SCHEMA)
        }
        cls.validators = {name: validator(schema) for name, schema in cls.schemas.items()}

    def assert_valid(self, schema_name, value):
        self.validators[schema_name].validate(value)

    def assert_invalid(self, schema_name, value):
        with self.assertRaises(Exception):
            self.validators[schema_name].validate(value)

    def test_schemas_are_strict_draft_2020_12(self):
        for name, schema in self.schemas.items():
            self.assertEqual(schema["$schema"], "https://json-schema.org/draft/2020-12/schema")
            self.assertTrue(schema["$id"].endswith(name))

            def visit(node, pointer="$"):
                if isinstance(node, dict):
                    if node.get("type") == "object":
                        self.assertIs(node.get("additionalProperties"), False, pointer)
                        self.assertEqual(
                            set(node.get("required", [])),
                            set(node.get("properties", {})),
                            pointer,
                        )
                    for key, child in node.items():
                        visit(child, f"{pointer}/{key}")
                elif isinstance(node, list):
                    for index, child in enumerate(node):
                        visit(child, f"{pointer}/{index}")

            visit(schema)

    def test_context_binds_complete_reviewer_repository_and_history(self):
        value = context_fixture()
        self.assert_valid(CONTEXT_SCHEMA.name, value)

        missing_configuration = copy.deepcopy(value)
        del missing_configuration["body"]["reviewer"]["configuration"]
        self.assert_invalid(CONTEXT_SCHEMA.name, missing_configuration)

        incomplete_closure = copy.deepcopy(value)
        incomplete_closure["body"]["repository_closure"]["complete"] = False
        self.assert_invalid(CONTEXT_SCHEMA.name, incomplete_closure)

        unknown_runtime_field = copy.deepcopy(value)
        unknown_runtime_field["body"]["reviewer"]["runtime"]["fallback"] = True
        self.assert_invalid(CONTEXT_SCHEMA.name, unknown_runtime_field)

        missing_compatibility = copy.deepcopy(value)
        del missing_compatibility["compatibility_sha256"]
        self.assert_invalid(CONTEXT_SCHEMA.name, missing_compatibility)

        missing_dependency_closure = copy.deepcopy(value)
        del missing_dependency_closure["body"]["dependency_closure"]
        self.assert_invalid(CONTEXT_SCHEMA.name, missing_dependency_closure)

    def test_four_way_history_never_becomes_automatically_reusable(self):
        value = context_fixture()
        value["body"]["historical_dispositions"][1]["automatically_reusable"] = True
        self.assert_invalid(CONTEXT_SCHEMA.name, value)

    def test_open_hosted_model_context_is_explicit_and_cannot_skip(self):
        context = context_fixture()
        context["body"]["dependency_closure"] = {
            "status": "open",
            "reason": "hosted_model_unpinned",
            "observed_inputs_sha256": digest("0"),
        }
        context["body"]["reviewer"]["model"] = {
            "kind": "hosted_model_unpinned",
            "provider": "model-provider",
            "model_id": "review-model",
            "provider_revision": "rolling-production",
            "inference_parameters_sha256": digest("3"),
            "automatically_reusable": False,
        }
        self.assert_valid(CONTEXT_SCHEMA.name, context)

        full = full_input()
        full["current_context"]["closure_status"] = "open"
        full["resolution"]["reason"] = "context_open"
        full["resolution"]["context_comparison"] = {
            "status": "unavailable",
            "closure_status": "open",
            "current_body_sha256": digest("f"),
            "current_compatibility_sha256": digest("e"),
            "reason": "hosted_model_unpinned",
        }
        self.assert_valid(INPUT_SCHEMA.name, full)

        unsafe_skip = skip_input()
        unsafe_skip["current_context"]["closure_status"] = "open"
        self.assert_invalid(INPUT_SCHEMA.name, unsafe_skip)

    def test_receipt_requires_complete_coverage_and_signed_source(self):
        value = receipt_fixture()
        self.assert_valid(RECEIPT_SCHEMA.name, value)

        incomplete = copy.deepcopy(value)
        incomplete["body"]["coverage"]["complete"] = False
        self.assert_invalid(RECEIPT_SCHEMA.name, incomplete)

        omitted = copy.deepcopy(value)
        omitted["body"]["coverage"]["omitted_identity_sha256"] = [digest("a")]
        self.assert_invalid(RECEIPT_SCHEMA.name, omitted)

        unsigned = copy.deepcopy(value)
        del unsigned["attestation"]["signature"]
        self.assert_invalid(RECEIPT_SCHEMA.name, unsigned)

        replay_reuse = copy.deepcopy(value)
        replay_reuse["body"]["reuse_constraints"][
            "four_way_replay_authorizes_verdict_or_input_skip"
        ] = True
        self.assert_invalid(RECEIPT_SCHEMA.name, replay_reuse)

    def test_all_four_input_states_validate(self):
        for value in (skip_input(), residue_input(), full_input(), blocked_input()):
            self.assert_valid(INPUT_SCHEMA.name, value)

    def test_skip_requires_exact_context_trusted_receipt_and_empty_payload(self):
        absent_receipt = skip_input()
        absent_receipt["resolution"]["prior_receipt"] = {
            "status": "absent",
            "reason": "not_provided",
        }
        self.assert_invalid(INPUT_SCHEMA.name, absent_receipt)

        mismatched_context = skip_input()
        mismatched_context["resolution"]["context_comparison"]["status"] = "mismatch"
        self.assert_invalid(INPUT_SCHEMA.name, mismatched_context)

        nonempty_payload = skip_input()
        nonempty_payload["resolution"]["selected_payload"]["selected_identity_sha256"] = [
            digest("6")
        ]
        self.assert_invalid(INPUT_SCHEMA.name, nonempty_payload)

    def test_four_way_cannot_authorize_skip_or_residue_carry(self):
        for value in (skip_input(), residue_input()):
            value["resolution"]["accounting"]["carried"][0]["basis"] = (
                "exact_noninteracting_four_way_byte_replay"
            )
            self.assert_invalid(INPUT_SCHEMA.name, value)

        evidence_only = residue_input()
        evidence_only["resolution"]["accounting"]["review_required"][0]["reason"] = (
            "four_way_replay_evidence_only"
        )
        self.assert_valid(INPUT_SCHEMA.name, evidence_only)

    def test_full_and_blocked_states_cannot_claim_carries(self):
        for value in (full_input(), blocked_input()):
            value["resolution"]["accounting"]["carried"] = [exact_carry(digest("6"))]
            self.assert_invalid(INPUT_SCHEMA.name, value)

    def test_input_requires_repository_snapshot_trees_and_payload_digest(self):
        missing_repository_identity = residue_input()
        del missing_repository_identity["repository"]["repository_id"]
        self.assert_invalid(INPUT_SCHEMA.name, missing_repository_identity)

        missing_tree = residue_input()
        del missing_tree["transition"]["C"]["tree_oid"]
        self.assert_invalid(INPUT_SCHEMA.name, missing_tree)

        missing_payload_digest = residue_input()
        del missing_payload_digest["resolution"]["selected_payload"]["canonical_payload_sha256"]
        self.assert_invalid(INPUT_SCHEMA.name, missing_payload_digest)


if __name__ == "__main__":
    unittest.main()
