use serde::Serialize;
use stratadiff::{
    AmbiguityAbstentionCause, AmbiguityConstraint, ChangeKind, Correspondence, DiffReport,
    Language, Predicate, analyze_bytes,
    codeowners::MAX_OWNERS_PER_RULE,
    coverage::{
        MAX_REVIEW_COVERAGE_CHECKPOINTS, MAX_REVIEW_COVERAGE_OWNER_RESULT_ITEMS,
        MAX_REVIEW_COVERAGE_REQUIREMENTS,
    },
    ledger::GITHUB_REVIEW_LEDGER_SCHEMA,
    ownership::GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA,
    review::{
        CheckpointCarryBasis, CheckpointMatchBasis, CheckpointState, REVIEW_DELTA_SCHEMA,
        ReviewDeltaBaselineBasis, ReviewDeltaComparison, ReviewDeltaFallbackReason,
        ReviewDeltaUnresolvedReason,
    },
};

#[test]
fn review_coverage_schema_uses_the_global_requirement_limit() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-coverage-v1.schema.json")).unwrap();
    let expected = serde_json::json!(MAX_REVIEW_COVERAGE_REQUIREMENTS);

    for pointer in [
        "/$defs/body/properties/files/maxItems",
        "/$defs/body/properties/unresolved_residue/maxItems",
        "/$defs/summary/properties/current_files/maximum",
        "/$defs/summary/properties/retired_residue_files/maximum",
        "/$defs/summary/properties/unresolved_residue/maximum",
        "/$defs/summary/properties/total_requirements/maximum",
        "/$defs/summary/properties/covered_files/maximum",
        "/$defs/summary/properties/needs_review_files/maximum",
        "/$defs/summary/properties/blocked_files/maximum",
    ] {
        assert_eq!(schema.pointer(pointer).unwrap(), &expected, "{pointer}");
    }

    let checkpoint_limit = serde_json::json!(MAX_REVIEW_COVERAGE_CHECKPOINTS);
    for pointer in [
        "/$defs/body/properties/checkpoint_proofs/maxItems",
        "/$defs/summary/properties/unique_checkpoint_proofs/maximum",
    ] {
        assert_eq!(
            schema.pointer(pointer).unwrap(),
            &checkpoint_limit,
            "{pointer}"
        );
    }
}

#[test]
fn review_coverage_schema_bounds_owner_cells_per_rule() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-coverage-v1.schema.json")).unwrap();
    let expected = serde_json::json!(MAX_OWNERS_PER_RULE);

    for pointer in [
        "/$defs/rule/properties/owner_alternatives/maxItems",
        "/$defs/file_coverage/properties/owner_alternatives/maxItems",
    ] {
        assert_eq!(schema.pointer(pointer).unwrap(), &expected, "{pointer}");
    }
}

#[test]
fn review_coverage_schema_bounds_expanded_owner_results() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-coverage-v1.schema.json")).unwrap();
    let expected = serde_json::json!(MAX_REVIEW_COVERAGE_OWNER_RESULT_ITEMS);

    for pointer in [
        "/$defs/owner_coverage/properties/eligible_reviewer_ids/maxItems",
        "/$defs/owner_coverage/properties/active_review_ids/maxItems",
        "/$defs/owner_coverage/properties/covering_review_ids/maxItems",
        "/$defs/owner_coverage/properties/blockers/maxItems",
    ] {
        assert_eq!(schema.pointer(pointer).unwrap(), &expected, "{pointer}");
    }
}

#[test]
fn every_published_schema_is_valid_draft_2020_12() {
    for source in [
        include_str!("../schema/report-v1.schema.json"),
        include_str!("../schema/report-v2.schema.json"),
        include_str!("../schema/report-v3.schema.json"),
        include_str!("../schema/review-v1.schema.json"),
        include_str!("../schema/review-delta-v1.schema.json"),
        include_str!("../schema/github-review-ledger-v1.schema.json"),
        include_str!("../schema/github-ownership-snapshot-v1.schema.json"),
        include_str!("../schema/review-memory-audit-v1.schema.json"),
        include_str!("../schema/review-memory-audit-v2.schema.json"),
        include_str!("../schema/review-inbox-v1.schema.json"),
    ] {
        let schema: serde_json::Value = serde_json::from_str(source).unwrap();
        jsonschema::draft202012::new(&schema).unwrap();
    }

    let coverage_schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-coverage-v1.schema.json")).unwrap();
    let ledger_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../schema/github-review-ledger-v1.schema.json"
    ))
    .unwrap();
    let ownership_schema: serde_json::Value = serde_json::from_str(include_str!(
        "../schema/github-ownership-snapshot-v1.schema.json"
    ))
    .unwrap();
    let review_delta_schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-delta-v1.schema.json")).unwrap();
    let registry = jsonschema::Registry::new()
        .add(GITHUB_REVIEW_LEDGER_SCHEMA, ledger_schema)
        .unwrap()
        .add(GITHUB_OWNERSHIP_SNAPSHOT_SCHEMA, ownership_schema)
        .unwrap()
        .add(REVIEW_DELTA_SCHEMA, review_delta_schema)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::draft202012::options()
        .with_registry(&registry)
        .offline()
        .build(&coverage_schema)
        .unwrap();
}

#[test]
fn review_memory_audit_schema_accepts_the_v1_contract() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-memory-audit-v1.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let metric = |id: &str, numerator: u64, denominator: u64| {
        let basis_points = (numerator * 10_000 + denominator / 2).checked_div(denominator);
        let (status, basis_points) = match basis_points {
            Some(value) => ("defined", serde_json::json!(value)),
            None => ("undefined", serde_json::Value::Null),
        };
        serde_json::json!({
            "id": id,
            "numerator": numerator,
            "denominator": denominator,
            "status": status,
            "basis_points": basis_points
        })
    };
    let instance = serde_json::json!({
        "schema": "stratadiff-review-memory-audit-v1",
        "tool_version": "1.0.0",
        "generated_at": "2026-09-05T13:00:00Z",
        "scope": {
            "provider_url": "https://github.com",
            "repository": "acme/widget",
            "window": {
                "start": "2026-06-07T13:00:00Z",
                "end_exclusive": "2026-09-05T13:00:00Z"
            },
            "selection": {
                "method": "latest_merged_at_desc_v1",
                "requested_limit": 50,
                "candidate_count": 2,
                "selected_count": 2,
                "shortfall": 48
            }
        },
        "collection": {
            "status": "complete",
            "graphql_calls": 3,
            "minimum_rate_limit_remaining": 4997,
            "last_rate_limit_reset_at": "2026-09-05T14:00:00Z"
        },
        "privacy": {
            "source_collected": false,
            "pr_text_collected": false,
            "review_text_collected": false,
            "commit_messages_collected": false,
            "logins_persisted": false,
            "actor_identity": "pr_local_opaque_key"
        },
        "claim_boundary": {
            "repository_window_description_supported": true,
            "github_population_estimate_supported": false,
            "reviewer_time_savings_supported": false,
            "issue_recall_or_safety_supported": false,
            "willingness_to_pay_supported": false,
            "checkpoint_materialization_supported": false
        },
        "summary": {
            "status": "affected",
            "selected_pull_requests": 2,
            "formal_peer_reviewed_pull_requests": 1,
            "completed_reviewed_pull_requests": 1,
            "completed_reviewer_pairs": 1,
            "comparable_reviewer_pairs": 1,
            "unobservable_reviewer_pairs": 0,
            "drifted_reviewer_pairs": 1,
            "affected_pull_requests": 1
        },
        "descriptive_metrics": [
            metric("formal_peer_reviewed_pr_rate", 1, 2),
            metric("completed_review_pr_rate", 1, 2),
            metric("checkpoint_oid_observability_rate", 1, 1),
            metric("checkpoint_pair_head_drift_rate", 1, 1),
            metric("completed_review_pair_post_force_push_rate", 1, 1),
            metric("checkpoint_pair_drift_without_observed_force_push_rate", 0, 1),
            metric("stranded_reviewer_pr_rate", 1, 1)
        ],
        "findings": [{
            "number": 17,
            "url": "https://github.com/acme/widget/pull/17",
            "merged_at": "2026-09-04T12:00:00Z",
            "final_head_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "completed_pair_count": 1,
            "comparable_pair_count": 1,
            "unobservable_pair_count": 0,
            "drifted_reviewers": [{
                "reviewer_key": "actor-0123456789abcdef01234567",
                "checkpoint_oid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "checkpoint_submitted_at": "2026-09-04T10:00:00Z",
                "checkpoint_state": "APPROVED",
                "dismissed": false,
                "post_completed_review_force_push": true,
                "post_latest_checkpoint_force_push": true
            }]
        }]
    });
    let errors: Vec<_> = validator
        .iter_errors(&instance)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "schema errors: {errors:#?}");

    let mut unknown_field = instance;
    unknown_field["summary"]["market_claim"] = serde_json::json!(true);
    assert!(!validator.is_valid(&unknown_field));
}

#[test]
fn review_memory_audit_schema_accepts_actionable_v2_identity() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-memory-audit-v2.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let metric = |id: &str, numerator: u64, denominator: u64| {
        let basis_points = (numerator * 10_000 + denominator / 2).checked_div(denominator);
        let (status, basis_points) = match basis_points {
            Some(value) => ("defined", serde_json::json!(value)),
            None => ("undefined", serde_json::Value::Null),
        };
        serde_json::json!({
            "id": id,
            "numerator": numerator,
            "denominator": denominator,
            "status": status,
            "basis_points": basis_points
        })
    };
    let instance = serde_json::json!({
        "schema": "stratadiff-review-memory-audit-v2",
        "tool_version": "1.1.0",
        "generated_at": "2026-09-05T13:00:00Z",
        "scope": {
            "provider_url": "https://github.com",
            "repository": "acme/widget",
            "window": {
                "start": "2026-06-07T13:00:00Z",
                "end_exclusive": "2026-09-05T13:00:00Z"
            },
            "selection": {
                "method": "latest_merged_at_desc_v1",
                "requested_limit": 50,
                "candidate_count": 2,
                "selected_count": 2,
                "shortfall": 48
            }
        },
        "collection": {
            "status": "complete",
            "graphql_calls": 3,
            "minimum_rate_limit_remaining": 4997,
            "last_rate_limit_reset_at": "2026-09-05T14:00:00Z"
        },
        "privacy": {
            "source_collected": false,
            "pr_text_collected": false,
            "review_text_collected": false,
            "commit_messages_collected": false,
            "logins_persisted": true,
            "actor_identity": "github_user_node_id_and_login"
        },
        "claim_boundary": {
            "repository_window_description_supported": true,
            "github_population_estimate_supported": false,
            "reviewer_time_savings_supported": false,
            "issue_recall_or_safety_supported": false,
            "willingness_to_pay_supported": false,
            "checkpoint_materialization_supported": false
        },
        "summary": {
            "status": "affected",
            "selected_pull_requests": 2,
            "formal_peer_reviewed_pull_requests": 1,
            "completed_reviewed_pull_requests": 1,
            "completed_reviewer_pairs": 2,
            "comparable_reviewer_pairs": 1,
            "unobservable_reviewer_pairs": 1,
            "drifted_reviewer_pairs": 1,
            "affected_pull_requests": 1
        },
        "descriptive_metrics": [
            metric("formal_peer_reviewed_pr_rate", 1, 2),
            metric("completed_review_pr_rate", 1, 2),
            metric("checkpoint_oid_observability_rate", 1, 2),
            metric("checkpoint_pair_head_drift_rate", 1, 1),
            metric("completed_review_pair_post_force_push_rate", 1, 2),
            metric("checkpoint_pair_drift_without_observed_force_push_rate", 0, 1),
            metric("stranded_reviewer_pr_rate", 1, 1)
        ],
        "findings": [{
            "number": 17,
            "url": "https://github.com/acme/widget/pull/17",
            "merged_at": "2026-09-04T12:00:00Z",
            "final_head_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "completed_pair_count": 2,
            "comparable_pair_count": 1,
            "unobservable_pair_count": 1,
            "drifted_reviewers": [{
                "login": "bob",
                "node_id": "U_kgDOBexample",
                "checkpoint_oid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "checkpoint_submitted_at": "2026-09-04T10:00:00Z",
                "checkpoint_state": "APPROVED",
                "dismissed": false,
                "post_completed_review_force_push": true,
                "post_latest_checkpoint_force_push": true
            }]
        }]
    });
    let errors: Vec<_> = validator
        .iter_errors(&instance)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "schema errors: {errors:#?}");

    let mut wrong_status = instance.clone();
    wrong_status["summary"]["status"] = serde_json::json!("insufficient_evidence");
    assert!(!validator.is_valid(&wrong_status));

    let mut no_actionable_finding = instance.clone();
    no_actionable_finding["findings"][0]["drifted_reviewers"] = serde_json::json!([]);
    assert!(!validator.is_valid(&no_actionable_finding));

    let mut no_findings = instance.clone();
    no_findings["findings"] = serde_json::json!([]);
    assert!(!validator.is_valid(&no_findings));

    let mut non_affected_with_actionable_finding = instance.clone();
    non_affected_with_actionable_finding["summary"]["status"] =
        serde_json::json!("no_observed_drift");
    non_affected_with_actionable_finding["summary"]["drifted_reviewer_pairs"] =
        serde_json::json!(0);
    assert!(!validator.is_valid(&non_affected_with_actionable_finding));

    let mut legacy_identity = instance;
    let reviewer = legacy_identity["findings"][0]["drifted_reviewers"][0]
        .as_object_mut()
        .unwrap();
    reviewer.remove("login");
    reviewer.remove("node_id");
    reviewer.insert(
        "reviewer_key".to_owned(),
        serde_json::json!("actor-0123456789abcdef01234567"),
    );
    assert!(!validator.is_valid(&legacy_identity));
}

#[test]
fn review_inbox_schema_accepts_actions_and_fails_closed_on_unknown_evidence() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-inbox-v1.schema.json")).unwrap();
    assert_eq!(
        schema
            .pointer(
                "/$defs/collection/properties/resource_budget/properties/resume_review_limit/const"
            )
            .and_then(serde_json::Value::as_u64),
        Some(stratadiff::github::MAX_GITHUB_REVIEWS as u64)
    );
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let instance = serde_json::json!({
        "schema": "stratadiff-review-inbox-v1",
        "tool_version": "1.0.0",
        "generated_at": "2026-09-05T14:20:00Z",
        "scope": {
            "provider_url": "https://github.com",
            "repository": "PostHog/posthog",
            "reviewer": {
                "login": "andrewm4894",
                "node_id": "U_kgDOBexample",
                "source": "authenticated_viewer"
            }
        },
        "collection": {
            "status": "complete",
            "pagination_complete": true,
            "temporal_consistency": "eligible_candidates_revalidated_non_atomic",
            "observation_started_at": "2026-09-05T14:19:00Z",
            "observation_completed_at": "2026-09-05T14:20:00Z",
            "open_pull_request_pages": 1,
            "review_pages": 6,
            "revalidated_review_prs": 3,
            "graphql_calls": 7,
            "minimum_rate_limit_remaining": 4800,
            "last_rate_limit_reset_at": "2026-09-05T15:00:00Z",
            "resource_budget": {
                "graphql_call_limit": 1000,
                "captured_node_limit": 100000,
                "response_byte_limit": 268435456,
                "wall_time_seconds_limit": 600,
                "resume_review_limit": 10000,
                "captured_nodes": 9,
                "response_bytes": 12000
            }
        },
        "privacy": {
            "source_collected": false,
            "pr_text_collected": false,
            "review_text_collected": false,
            "commit_messages_collected": false,
            "logins_persisted": true,
            "actor_identity": "authenticated_viewer_node_id_and_login"
        },
        "summary": {
            "status": "actionable",
            "open_prs": 3,
            "eligible_review_prs": 3,
            "comparable_review_prs": 2,
            "up_to_date_prs": 1,
            "resume_available_prs": 1,
            "unobservable_review_prs": 1
        },
        "actionable": [{
            "number": 95462,
            "url": "https://github.com/PostHog/posthog/pull/95462",
            "is_draft": false,
            "updated_at": "2026-09-05T13:12:02Z",
            "head_oid": "3e39bb0b87a36b7e000393f446c3042690c943c7",
            "total_review_count": 75,
            "checkpoint": {
                "oid": "3c2ad3361a98cd34bcea07a8acba71144b9c86ca",
                "submitted_at": "2026-09-04T22:18:42Z",
                "state": "APPROVED"
            },
            "resume_argv": [
                "gh", "stratadiff", "resume", "95462", "-R",
                "github.com/PostHog/posthog", "--reviewer", "andrewm4894"
            ]
        }],
        "unobservable": [{
            "number": 95500,
            "url": "https://github.com/PostHog/posthog/pull/95500",
            "is_draft": true,
            "updated_at": "2026-09-05T13:20:00Z",
            "head_oid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "total_review_count": 2,
            "checkpoint": {
                "oid": null,
                "submitted_at": "2026-09-05T13:00:00Z",
                "state": "CHANGES_REQUESTED"
            },
            "reason": "checkpoint_oid_unavailable"
        }]
    });
    let errors: Vec<_> = validator
        .iter_errors(&instance)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "schema errors: {errors:#?}");

    let mut unsafe_action = instance.clone();
    unsafe_action["actionable"][0]["checkpoint"]["oid"] = serde_json::Value::Null;
    assert!(!validator.is_valid(&unsafe_action));

    let mut partial_pagination = instance.clone();
    partial_pagination["collection"]["pagination_complete"] = serde_json::json!(false);
    assert!(!validator.is_valid(&partial_pagination));

    let mut oversized_drift = instance.clone();
    oversized_drift["unobservable"][0]["total_review_count"] = serde_json::json!(10001);
    oversized_drift["unobservable"][0]["checkpoint"]["oid"] =
        serde_json::json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    oversized_drift["unobservable"][0]["reason"] =
        serde_json::json!("resume_review_limit_exceeded");
    assert!(validator.is_valid(&oversized_drift));

    let mut false_review_limit = oversized_drift.clone();
    false_review_limit["unobservable"][0]["total_review_count"] = serde_json::json!(10000);
    assert!(!validator.is_valid(&false_review_limit));

    let mut missing_limit_head = oversized_drift.clone();
    missing_limit_head["unobservable"][0]["head_oid"] = serde_json::Value::Null;
    assert!(!validator.is_valid(&missing_limit_head));

    let mut missing_limit_checkpoint = oversized_drift;
    missing_limit_checkpoint["unobservable"][0]["checkpoint"]["oid"] = serde_json::Value::Null;
    assert!(!validator.is_valid(&missing_limit_checkpoint));

    let mut unknown_field = instance;
    unknown_field["actionable"][0]["review_is_safe"] = serde_json::json!(true);
    assert!(!validator.is_valid(&unknown_field));
}

#[test]
fn emitted_report_conforms_to_the_published_schema() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/report-v3.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let reports = [
        analyze_bytes(
            b"def value():\n    return 1\n".to_vec(),
            b"def value():\n    return 2\n".to_vec(),
            "before.py".to_owned(),
            "after.py".to_owned(),
            Language::Python,
        )
        .unwrap(),
        analyze_bytes(
            b"def add_old(value):\n    return value + 1\n\ndef multiply_old(value):\n    return value * 2\n".to_vec(),
            b"def multiply_new(item):\n    return item * 3\n\ndef add_new(item):\n    return item + 4\n".to_vec(),
            "before.py".to_owned(),
            "after.py".to_owned(),
            Language::Python,
        )
        .unwrap(),
        analyze_bytes(
            b"def same():\n    return 1\n\ndef same():\n    return 1\n".to_vec(),
            b"def same():\n    return 1\n\ndef same():\n    return 1\n".to_vec(),
            "before.py".to_owned(),
            "after.py".to_owned(),
            Language::Python,
        )
        .unwrap(),
        analyze_bytes(
            vec![0xff, 0x00, b'a', b'\n'],
            vec![0xff, 0x01, b'b', b'\n'],
            "before.unknown".to_owned(),
            "after.unknown".to_owned(),
            Language::Universal,
        )
        .unwrap(),
    ];
    for report in reports {
        let instance = serde_json::to_value(report).unwrap();
        let errors: Vec<_> = validator
            .iter_errors(&instance)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "schema errors: {errors:#?}");
    }
}

#[test]
fn schema_enums_track_every_serialized_public_variant() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/report-v3.schema.json")).unwrap();
    assert_enum(
        &schema["$defs"]["predicate"]["enum"],
        &[
            Predicate::InputPair,
            Predicate::ByteEqual,
            Predicate::SyntaxEqual,
            Predicate::ShapeEqual,
        ],
    );
    assert_enum(
        &schema["$defs"]["relation"]["properties"]["correspondence"]["enum"],
        &[
            Correspondence::InputPair,
            Correspondence::ModelForced,
            Correspondence::Suggested,
        ],
    );
    assert_enum(
        &schema["$defs"]["change"]["properties"]["kind"]["enum"],
        &[
            ChangeKind::Insert,
            ChangeKind::Delete,
            ChangeKind::EquivalentRelocation,
            ChangeKind::ChildOrderChanged,
            ChangeKind::ModelForcedUpdate,
            ChangeKind::SuggestedUpdate,
            ChangeKind::FormattingOnly,
        ],
    );
    assert_enum(
        &schema["$defs"]["parser"]["properties"]["language"]["enum"],
        &[
            Language::Universal,
            Language::Python,
            Language::Javascript,
            Language::Typescript,
            Language::Tsx,
            Language::Rust,
            Language::Java,
            Language::Json,
            Language::C,
            Language::Cpp,
            Language::CSharp,
            Language::Go,
            Language::Ruby,
            Language::Bash,
            Language::Php,
            Language::Html,
            Language::Css,
            Language::Yaml,
            Language::Toml,
            Language::Markdown,
            Language::Kotlin,
            Language::Swift,
            Language::Lua,
            Language::Scala,
            Language::R,
            Language::Elixir,
            Language::Haskell,
            Language::Ocaml,
            Language::OcamlInterface,
            Language::Zig,
        ],
    );
    assert_enum(
        &schema["$defs"]["symbolic_abstention"]["properties"]["cause"]["enum"],
        &[
            AmbiguityAbstentionCause::DuplicateSymmetry,
            AmbiguityAbstentionCause::ComponentLimit,
            AmbiguityAbstentionCause::CandidateScanLimit,
        ],
    );
}

#[test]
fn review_schema_tracks_checkpoint_variants() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-v1.schema.json")).unwrap();
    assert_enum(
        &schema["$defs"]["checkpoint_state"]["enum"],
        &[
            CheckpointState::NeedsReviewNow,
            CheckpointState::UnchangedSinceCheckpoint,
        ],
    );
    assert_enum(
        &schema["$defs"]["checkpoint_match_basis"]["enum"],
        &[
            CheckpointCarryBasis::ExactGitChangeIdentity,
            CheckpointCarryBasis::ExactNoninteractingFourWayByteReplay,
        ],
    );
    assert_enum(
        &schema["$defs"]["review_checkpoint"]["properties"]["match_basis"]["enum"],
        &[
            CheckpointMatchBasis::ExactGitChangeIdentity,
            CheckpointMatchBasis::ExactGitChangeIdentityOrNoninteractingFourWayByteReplay,
        ],
    );
}

#[test]
fn review_delta_schema_tracks_public_variants() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-delta-v1.schema.json")).unwrap();
    assert_enum(
        &schema["properties"]["comparison"]["enum"],
        &[
            ReviewDeltaComparison::CheckpointToHead,
            ReviewDeltaComparison::PerFileReviewBaselineToHead,
        ],
    );
    assert_enum(
        &schema["$defs"]["entry"]["properties"]["baseline_basis"]["enum"],
        &[
            ReviewDeltaBaselineBasis::CheckpointSnapshot,
            ReviewDeltaBaselineBasis::CurrentBaseNoCheckpointChange,
            ReviewDeltaBaselineBasis::ReconstructedReviewBaseline,
            ReviewDeltaBaselineBasis::CurrentBaseFallback,
            ReviewDeltaBaselineBasis::CheckpointHeadFallback,
        ],
    );
    assert_enum(
        &schema["$defs"]["entry"]["properties"]["fallback_reason"]["enum"],
        &[
            ReviewDeltaFallbackReason::OverlapOrAdjacent,
            ReviewDeltaFallbackReason::BinaryNul,
            ReviewDeltaFallbackReason::SourceUnavailable,
            ReviewDeltaFallbackReason::UnsupportedChange,
            ReviewDeltaFallbackReason::TranslationFailed,
            ReviewDeltaFallbackReason::ReplayOrdersMismatch,
        ],
    );
    assert_enum(
        &schema["$defs"]["unresolved_change"]["properties"]["reason"]["enum"],
        &[ReviewDeltaUnresolvedReason::NonUtf8GitPath],
    );
}

#[test]
fn review_delta_schema_rejects_fallback_evidence_on_an_exact_basis() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-delta-v1.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let object_id = "0".repeat(40);
    let mut exact = serde_json::json!({
        "schema": "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-delta-v1.schema.json",
        "engine_version": "0.3.0",
        "comparison": "per_file_review_baseline_to_head",
        "old_base_commit": object_id,
        "checkpoint_commit": object_id,
        "current_base_commit": object_id,
        "head_commit": object_id,
        "summary": {
            "displayable_files": 1,
            "unresolved_retired_changes": 0,
            "needs_review_files": 1,
            "gate_passed": false
        },
        "entries": [{
            "file": {
                "status": "added",
                "after_path": "new.py",
                "after_path_encoding": "utf8",
                "after_mode": "100644",
                "after_blob": object_id,
                "after_bytes": 8,
                "priority": "review_first",
                "lane": "unverified",
                "checkpoint_state": "needs_review_now",
                "reason": "new current change"
            },
            "baseline_basis": "current_base_no_checkpoint_change",
            "before_source": {"kind": "empty"},
            "after_source": {
                "kind": "git_object",
                "commit": object_id,
                "object_id": object_id,
                "byte_len": 8
            }
        }],
        "unresolved_retired_changes": []
    });
    assert!(validator.is_valid(&exact));

    exact["entries"][0]["fallback_reason"] = serde_json::json!("unsupported_change");
    assert!(
        !validator.is_valid(&exact),
        "schema accepted fallback evidence on an exact baseline basis"
    );
}

#[test]
fn schema_binds_universal_identity_and_native_engine() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/report-v3.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let universal = analyze_bytes(
        b"old\n".to_vec(),
        b"new\n".to_vec(),
        "before.data".to_owned(),
        "after.data".to_owned(),
        Language::Universal,
    )
    .unwrap();

    for (field, value) in [
        ("engine", serde_json::json!("tree-sitter")),
        ("runtime_version", serde_json::json!("0.27.0")),
        ("grammar_name", serde_json::json!("tree-sitter-python")),
        ("grammar_version", serde_json::json!("0.0.0")),
        ("grammar_abi", serde_json::json!(2)),
        (
            "node_types_blake3",
            serde_json::json!("0000000000000000000000000000000000000000000000000000000000000000"),
        ),
        (
            "coordinate_unit",
            serde_json::json!("zero_based_row_utf8_byte_column"),
        ),
        ("root_kind", serde_json::json!("module")),
    ] {
        let mut candidate = serde_json::to_value(&universal).unwrap();
        candidate["parser"][field] = value;
        assert!(
            !validator.is_valid(&candidate),
            "schema accepted a Universal manifest with tampered {field}"
        );
    }

    let mut wrong_tree_sitter_engine = serde_json::to_value(
        analyze_bytes(
            b"value = 1\n".to_vec(),
            b"value = 2\n".to_vec(),
            "before.py".to_owned(),
            "after.py".to_owned(),
            Language::Python,
        )
        .unwrap(),
    )
    .unwrap();
    wrong_tree_sitter_engine["parser"]["engine"] = "stratadiff-universal".into();
    assert!(!validator.is_valid(&wrong_tree_sitter_engine));
}

#[test]
fn readme_report_excerpt_uses_current_certificate_vocabulary() {
    let readme = include_str!("../README.md");
    let stable = analyze_bytes(
        b"def old_name(value):\n    return value + 1\n".to_vec(),
        b"def new_name(item):\n    return item + 2\n".to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    )
    .unwrap();
    let evidence = &stable
        .relations
        .iter()
        .find(|relation| relation.predicate == Predicate::ShapeEqual)
        .unwrap()
        .evidence;
    for item in evidence {
        assert!(
            readme.contains(&format!("\"{item}\"")),
            "README report excerpt is missing evidence {item}"
        );
    }

    let duplicate_source = "def same():\n    return 1\n\ndef same():\n    return 1\n";
    let duplicate = analyze_bytes(
        duplicate_source.as_bytes().to_vec(),
        duplicate_source.as_bytes().to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    )
    .unwrap();
    let reason = &duplicate
        .ambiguities
        .iter()
        .find(|group| {
            group.before.len() == 2
                && group.after.len() == 2
                && group.before[0].kind == "function_definition"
        })
        .unwrap()
        .reason;
    assert!(
        readme.contains(&format!("\"{reason}\"")),
        "README report excerpt uses a stale ambiguity reason"
    );
}

#[test]
fn every_report_object_rejects_unknown_fields_during_deserialization() {
    let before = concat!(
        "def same():\n    return 1\n\n",
        "def same():\n    return 1\n",
    );
    let after = concat!(
        "def same():\n    return 1\n\n",
        "def same():\n    return 1\n\n",
        "added = 2\n",
    );
    let report = analyze_bytes(
        before.as_bytes().to_vec(),
        after.as_bytes().to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    )
    .unwrap();
    assert!(!report.relations.is_empty());
    assert!(!report.ambiguities.is_empty());
    assert!(!report.changes.is_empty());
    assert!(!report.patch.edits.is_empty());

    let encoded = serde_json::to_value(report).unwrap();
    for pointer in [
        "",
        "/before",
        "/parser",
        "/relations/0",
        "/relations/0/before",
        "/relations/0/before/span",
        "/relations/0/before/span/start",
        "/ambiguities/0",
        "/ambiguities/0/constraint",
        "/changes/0",
        "/patch",
        "/patch/edits/0",
        "/certificate",
        "/summary",
    ] {
        let mut candidate = encoded.clone();
        let object = if pointer.is_empty() {
            candidate.as_object_mut().unwrap()
        } else {
            candidate
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
        };
        object.insert("unexpected".to_owned(), serde_json::Value::Bool(true));
        assert!(
            serde_json::from_value::<DiffReport>(candidate).is_err(),
            "unknown field was accepted at {pointer}"
        );
    }

    let exact = analyze_bytes(
        b"def add_old(value):\n    return value + 1\n\ndef multiply_old(value):\n    return value * 2\n".to_vec(),
        b"def multiply_new(item):\n    return item * 3\n\ndef add_new(item):\n    return item + 4\n".to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    )
    .unwrap();
    let exact_index = exact
        .ambiguities
        .iter()
        .position(|group| {
            matches!(
                group.constraint,
                AmbiguityConstraint::ExactOrderedAlignment { .. }
            )
        })
        .unwrap();
    let mut encoded = serde_json::to_value(exact).unwrap();
    encoded["ambiguities"][exact_index]["constraint"]["possible_pairs"][0]["unexpected"] =
        serde_json::Value::Bool(true);
    assert!(serde_json::from_value::<DiffReport>(encoded).is_err());
}

#[test]
fn legacy_v1_ambiguity_sets_fail_closed_instead_of_inventing_pairs() {
    let source = b"def same():\n    return 1\n\ndef same():\n    return 1\n";
    let report = analyze_bytes(
        source.to_vec(),
        source.to_vec(),
        "before.py".to_owned(),
        "after.py".to_owned(),
        Language::Python,
    )
    .unwrap();
    let mut encoded = serde_json::to_value(report).unwrap();
    encoded["schema"] = serde_json::Value::String(
        "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v1.schema.json"
            .to_owned(),
    );
    encoded["ambiguities"][0]
        .as_object_mut()
        .unwrap()
        .remove("constraint");

    assert!(serde_json::from_value::<DiffReport>(encoded).is_err());
}

fn assert_enum<T: Serialize>(schema_values: &serde_json::Value, expected: &[T]) {
    let actual = schema_values.as_array().unwrap();
    let expected: Vec<_> = expected
        .iter()
        .map(|value| serde_json::to_value(value).unwrap())
        .collect();
    assert_eq!(actual, &expected);
}
