use serde_json::json;

fn study_schema() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../benchmarks/reviewer-study-v1/study-data.schema.json"
    ))
    .unwrap()
}

fn schema_valid_observation() -> serde_json::Value {
    json!({
        "schema": "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/benchmarks/reviewer-study-v1/study-data.schema.json",
        "study_id": "study_synthetic_0000000000000001",
        "protocol_version": "1.0.0",
        "preregistration_sha256": "0".repeat(64),
        "synthetic": true,
        "collection_status": "open",
        "participants": [{
            "participant_id": "p_000000000001",
            "repeat_use": {
                "invited_again": false,
                "follow_up_complete": false,
                "used_within_28_days": false
            }
        }],
        "paired_observations": [{
            "pair_id": "pair_000000000001",
            "participant_id": "p_000000000001",
            "task_family_id": "task_000000000001",
            "assignment_order": "baseline_then_resume",
            "baseline_variant": "a",
            "resume_variant": "b",
            "baseline": {
                "completion_seconds": 100,
                "issues_found": 8,
                "seeded_issues": 10,
                "reopened_files": 100,
                "reopened_lines": 1000
            },
            "resume": {
                "completion_seconds": 80,
                "issues_found": 8,
                "seeded_issues": 10,
                "reopened_files": 60,
                "reopened_lines": 500
            },
            "false_carry_adjudication": {
                "unit": "carried_file_change",
                "carried_units": 1,
                "adjudicated_units": 1,
                "confirmed_false_carries": 0,
                "adjudicator_count": 2,
                "all_disagreements_resolved": true
            }
        }]
    })
}

#[test]
fn reviewer_study_schema_accepts_opposite_task_variants() {
    let schema = study_schema();
    let validator = jsonschema::draft202012::new(&schema).unwrap();

    assert!(validator.is_valid(&schema_valid_observation()));
}

#[test]
fn reviewer_study_schema_rejects_same_variant_and_unknown_measurement_fields() {
    let schema = study_schema();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let mut same_variant = schema_valid_observation();
    same_variant["paired_observations"][0]["resume_variant"] = json!("a");
    assert!(!validator.is_valid(&same_variant));

    let mut unknown_field = schema_valid_observation();
    unknown_field["paired_observations"][0]["baseline"]["free_text"] = json!("prohibited");
    assert!(!validator.is_valid(&unknown_field));
}

#[test]
fn reviewer_study_schema_rejects_study_id_mode_mismatch() {
    let schema = study_schema();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let mut data = schema_valid_observation();
    data["study_id"] = json!("study_0000000000000001");

    assert!(!validator.is_valid(&data));
}
