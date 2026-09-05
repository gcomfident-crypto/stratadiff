use serde_json::{Value, json};

const SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn schema(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

fn assert_valid(schema: &Value, instance: &Value, label: &str) {
    let validator = jsonschema::draft202012::new(schema).unwrap();
    let errors = validator
        .iter_errors(instance)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{label} schema errors: {errors:#?}");
}

fn assert_invalid(schema: &Value, instance: &Value, label: &str) {
    assert!(
        !jsonschema::draft202012::new(schema)
            .unwrap()
            .is_valid(instance),
        "{label} unexpectedly passed schema validation"
    );
}

fn task_spec() -> Value {
    let variant = json!({
        "response_issue_ids": ["issue_0000000000000001"],
        "seeded_issue_ids": ["issue_0000000000000001"],
        "presentations": {
            "baseline": {
                "bundle_path": "/private/baseline",
                "preflight_command": ["python3", "preflight.py"],
                "run_command": ["python3", "run.py", "{result}"],
                "reopened_file_ids": ["file_0000000000000001"],
                "reopened_line_ids": ["line_0000000000000001"],
                "carried_unit_ids": []
            },
            "resume": {
                "bundle_path": "/private/resume",
                "preflight_command": ["python3", "preflight.py"],
                "run_command": ["python3", "run.py", "{result}"],
                "reopened_file_ids": ["file_0000000000000002"],
                "reopened_line_ids": ["line_0000000000000002"],
                "carried_unit_ids": ["carry_0000000000000001"]
            }
        }
    });
    json!({
        "schema": "stratadiff-reviewer-pilot-task-spec-v1",
        "stratadiff_binary_path": "/usr/bin/stratadiff",
        "task_families": [{
            "task_family_id": "task_000000000001",
            "variants": {"a": variant.clone(), "b": variant}
        }]
    })
}

fn task_catalog() -> Value {
    let variant = json!({
        "response_issue_ids": ["issue_0000000000000001"],
        "seeded_issue_ids": ["issue_0000000000000001"],
        "presentations": {
            "baseline": {
                "bundle_sha256": SHA256,
                "reopened_file_ids": ["file_0000000000000001"],
                "reopened_line_ids": ["line_0000000000000001"],
                "carried_unit_ids": []
            },
            "resume": {
                "bundle_sha256": SHA256,
                "reopened_file_ids": ["file_0000000000000002"],
                "reopened_line_ids": ["line_0000000000000002"],
                "carried_unit_ids": ["carry_0000000000000001"]
            }
        }
    });
    json!({
        "schema": "stratadiff-reviewer-pilot-task-catalog-v1",
        "stratadiff_build_sha256": SHA256,
        "task_families": [{
            "task_family_id": "task_000000000001",
            "variants": {"a": variant.clone(), "b": variant}
        }]
    })
}

fn plan() -> Value {
    json!({
        "schema": "stratadiff-reviewer-pilot-plan-v1",
        "study_id": "study_synthetic_0000000000000001",
        "protocol_version": "1.0.0",
        "preregistration_sha256": SHA256,
        "synthetic": true,
        "randomization": {
            "algorithm": "hmac-sha256-rejection-fisher-yates-v1",
            "seed_hex": SHA256,
            "seed_commitment_sha256": SHA256
        },
        "task_catalog_sha256": SHA256,
        "task_spec_sha256": SHA256,
        "pilot_source_sha256": SHA256,
        "stratadiff_build_sha256": SHA256,
        "participants": [
            "p_000000000001",
            "p_000000000002",
            "p_000000000003",
            "p_000000000004"
        ],
        "adjudicator_slots": [
            "adjslot_000000000001",
            "adjslot_000000000002",
            "adjslot_000000000003"
        ],
        "assignments": [{
            "pair_id": "pair_000000000001",
            "participant_id": "p_000000000001",
            "task_family_id": "task_000000000001",
            "sequence": 0,
            "assignment_order": "baseline_then_resume",
            "baseline_variant": "a",
            "resume_variant": "b",
            "arms": {
                "baseline": {"variant": "a", "bundle_sha256": SHA256},
                "resume": {"variant": "b", "bundle_sha256": SHA256}
            },
            "adjudication": [{
                "unit_id": "carry_0000000000000001",
                "counts_as_carry": true,
                "initial_slots": ["adjslot_000000000001", "adjslot_000000000002"],
                "resolver_slot": "adjslot_000000000003"
            }]
        }]
    })
}

fn plan_attestation() -> Value {
    json!({
        "schema": "stratadiff-reviewer-pilot-plan-attestation-v1",
        "kind": "plan",
        "study_id": "study_synthetic_0000000000000001",
        "operator_key_id": "operator_000000000001",
        "preregistration_sha256": SHA256,
        "plan_sha256": SHA256,
        "plan_anchor_sha256": SHA256,
        "task_catalog_sha256": SHA256,
        "task_spec_sha256": SHA256,
        "tool_source_sha256": SHA256,
        "signature_algorithm": "openssh-ed25519-sshsig"
    })
}

#[test]
fn pilot_schemas_are_closed_world_and_accept_minimal_instances() {
    let fixtures = [
        (
            "task spec",
            schema(include_str!(
                "../benchmarks/reviewer-study-v1/pilot-task-spec.schema.json"
            )),
            task_spec(),
        ),
        (
            "task catalog",
            schema(include_str!(
                "../benchmarks/reviewer-study-v1/pilot-task-catalog.schema.json"
            )),
            task_catalog(),
        ),
        (
            "plan",
            schema(include_str!(
                "../benchmarks/reviewer-study-v1/pilot-plan.schema.json"
            )),
            plan(),
        ),
        (
            "attestation",
            schema(include_str!(
                "../benchmarks/reviewer-study-v1/pilot-attestation.schema.json"
            )),
            plan_attestation(),
        ),
    ];

    for (label, schema, mut fixture) in fixtures {
        assert_valid(&schema, &fixture, label);
        fixture["unexpected"] = json!(true);
        assert_invalid(&schema, &fixture, label);
    }
}

#[test]
fn public_task_catalog_rejects_private_paths_and_commands() {
    let schema = schema(include_str!(
        "../benchmarks/reviewer-study-v1/pilot-task-catalog.schema.json"
    ));

    for field in ["bundle_path", "run_command"] {
        let mut fixture = task_catalog();
        fixture["task_families"][0]["variants"]["a"]["presentations"]["resume"][field] =
            json!("private");
        assert_invalid(&schema, &fixture, field);
    }
}

#[test]
fn pilot_attestation_rejects_an_embedded_signature() {
    let schema = schema(include_str!(
        "../benchmarks/reviewer-study-v1/pilot-attestation.schema.json"
    ));
    let mut fixture = plan_attestation();
    fixture["signature"] = json!("must remain in the detached sidecar");

    assert_invalid(&schema, &fixture, "embedded signature");
}

#[test]
fn pilot_plan_rejects_arm_variant_mismatches() {
    let schema = schema(include_str!(
        "../benchmarks/reviewer-study-v1/pilot-plan.schema.json"
    ));

    for arm in ["baseline", "resume"] {
        let mut fixture = plan();
        fixture["assignments"][0]["arms"][arm]["variant"] = match arm {
            "baseline" => json!("b"),
            "resume" => json!("a"),
            _ => unreachable!(),
        };
        assert_invalid(&schema, &fixture, arm);
    }
}

#[test]
fn pilot_plan_rejects_study_id_mode_mismatch() {
    let schema = schema(include_str!(
        "../benchmarks/reviewer-study-v1/pilot-plan.schema.json"
    ));
    let mut fixture = plan();
    fixture["study_id"] = json!("study_0000000000000001");

    assert_invalid(&schema, &fixture, "synthetic study ID mode");
}
