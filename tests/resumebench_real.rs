use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;
use sha2::{Digest, Sha256};

const MANIFEST_SCHEMA: &str = "stratadiff-resumebench-real-manifest-v0";
const ORACLE_SCHEMA: &str = "stratadiff-resumebench-real-oracle-v0";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn string_set(array: &Value, field: &str) -> BTreeSet<String> {
    array
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item[field].as_str().unwrap().to_owned())
        .collect()
}

fn assert_identity_digest(identity: &Value) {
    let mut canonical = BTreeMap::new();
    for field in [
        "status",
        "similarity_percent",
        "before_path_base64",
        "after_path_base64",
        "before_mode",
        "after_mode",
        "before_object_id",
        "after_object_id",
    ] {
        canonical.insert(field, identity[field].clone());
    }
    assert_eq!(
        identity["identity_sha256"].as_str().unwrap(),
        sha256(&serde_json::to_vec(&canonical).unwrap())
    );
}

#[test]
fn real_manifest_oracles_and_evaluation_are_internally_complete() {
    let dataset = root().join("benchmarks/resumebench-real-v0");
    for line in fs::read_to_string(dataset.join("SHA256SUMS"))
        .unwrap()
        .lines()
    {
        let (expected, relative) = line.split_once("  ").unwrap();
        assert_eq!(expected, sha256(&fs::read(dataset.join(relative)).unwrap()));
    }
    let manifest_bytes = fs::read(dataset.join("manifest.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest["schema"], MANIFEST_SCHEMA);
    assert_eq!(manifest["dataset_version"], "0.1.0");
    assert_eq!(manifest["selection_protocol"]["prevalence_claim"], false);
    assert_eq!(
        manifest["source_repository"]["license"]["spdx_expression"],
        "Apache-2.0"
    );
    assert!(manifest["known_limitations"].as_array().unwrap().len() >= 5);

    let generator = root().join(manifest["oracle_contract"]["generator"].as_str().unwrap());
    assert_eq!(
        manifest["oracle_contract"]["generator_sha256"]
            .as_str()
            .unwrap(),
        sha256(&fs::read(generator).unwrap())
    );

    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    let mut case_ids = BTreeSet::new();
    let mut current_total = 0_u64;
    let mut needs_total = 0_u64;
    let mut carried_total = 0_u64;
    let mut retired_total = 0_u64;
    let mut rejection_cases = 0_usize;

    for case in cases {
        let case_id = case["id"].as_str().unwrap();
        assert!(case_ids.insert(case_id));
        assert_eq!(
            case["checkpoint_evidence"]["kind"],
            "public_gerrit_code_review"
        );
        assert_eq!(case["checkpoint_evidence"]["label"], "Code-Review");
        assert_eq!(case["checkpoint_evidence"]["value"], 2);
        assert_eq!(
            case["checkpoint_evidence"]["patch_set"],
            case["revisions"]["checkpoint"]["patch_set"]
        );
        assert!(
            case["api_evidence"]["detail_url"]
                .as_str()
                .unwrap()
                .contains(case_id.split('-').nth(1).unwrap())
        );
        assert!(
            case["api_evidence"]["messages_url"]
                .as_str()
                .unwrap()
                .contains(case_id.split('-').nth(1).unwrap())
        );

        let oracle = read_json(&dataset.join(case["expectation"]["oracle"].as_str().unwrap()));
        assert_eq!(oracle["schema"], ORACLE_SCHEMA);
        assert_eq!(oracle["case_id"], case_id);
        assert_eq!(
            oracle["requested_base_commit"],
            case["revisions"]["requested_base_commit"]
        );
        assert_eq!(
            oracle["checkpoint_commit"],
            case["revisions"]["checkpoint"]["commit"]
        );
        assert_eq!(
            oracle["current_commit"],
            case["revisions"]["current"]["commit"]
        );

        match oracle["expectation"].as_str().unwrap() {
            "exact_identity_partition" => {
                assert_eq!(
                    oracle["checkpoint_merge_base"],
                    oracle["requested_base_commit"]
                );
                assert_eq!(
                    oracle["current_merge_base"],
                    oracle["requested_base_commit"]
                );
                let checkpoint = string_set(&oracle["identities"]["checkpoint"], "identity_sha256");
                let head = string_set(&oracle["identities"]["head"], "identity_sha256");
                let delta = string_set(&oracle["identities"]["resume_delta"], "identity_sha256");
                for identity in oracle["identities"]["checkpoint"].as_array().unwrap() {
                    assert_identity_digest(identity);
                }
                for identity in oracle["identities"]["head"].as_array().unwrap() {
                    assert_identity_digest(identity);
                }
                for identity in oracle["identities"]["resume_delta"].as_array().unwrap() {
                    assert_identity_digest(identity);
                }
                let carried: BTreeSet<_> = checkpoint.intersection(&head).cloned().collect();
                let needs: BTreeSet<_> = head.difference(&checkpoint).cloned().collect();
                let retired: BTreeSet<_> = checkpoint.difference(&head).cloned().collect();
                let expected_head: BTreeMap<_, _> = oracle["expected_head"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| {
                        (
                            item["identity_sha256"].as_str().unwrap(),
                            item["checkpoint_state"].as_str().unwrap(),
                        )
                    })
                    .collect();
                assert_eq!(expected_head.len(), head.len());
                for identity in &head {
                    let expected = if checkpoint.contains(identity) {
                        "unchanged_since_checkpoint"
                    } else {
                        "needs_review_now"
                    };
                    assert_eq!(expected_head[identity.as_str()], expected);
                }
                let serialized_retired: BTreeSet<_> = oracle["retired_identity_sha256"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|identity| identity.as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(retired, serialized_retired);
                assert_eq!(oracle["summary"]["checkpoint_identities"], checkpoint.len());
                assert_eq!(oracle["summary"]["current_identities"], head.len());
                assert_eq!(oracle["summary"]["resume_delta_identities"], delta.len());
                assert_eq!(
                    oracle["summary"]["unchanged_since_checkpoint"],
                    carried.len()
                );
                assert_eq!(oracle["summary"]["needs_review_now"], needs.len());
                assert_eq!(oracle["summary"]["retired"], retired.len());
                current_total += head.len() as u64;
                needs_total += needs.len() as u64;
                carried_total += carried.len() as u64;
                retired_total += retired.len() as u64;
            }
            "base_mismatch_rejected" => {
                rejection_cases += 1;
                assert_ne!(
                    oracle["checkpoint_merge_base"],
                    oracle["current_merge_base"]
                );
                assert_eq!(
                    oracle["current_merge_base"],
                    oracle["requested_base_commit"]
                );
                assert_eq!(
                    oracle["error_contains"],
                    "checkpoint and current review must have the same merge base"
                );
            }
            expectation => panic!("unexpected oracle expectation {expectation}"),
        }
    }

    assert_eq!(
        (current_total, needs_total, carried_total, retired_total),
        (24, 4, 20, 3)
    );
    assert_eq!(rejection_cases, 1);

    let evaluation = read_json(&dataset.join("evaluation-v0.1.0.json"));
    assert_eq!(evaluation["benchmark_complete"], true);
    assert_eq!(
        evaluation["provenance"]["manifest_sha256"],
        sha256(&manifest_bytes)
    );
    assert!(is_sha256(
        evaluation["provenance"]["stratadiff_binary_sha256"]
            .as_str()
            .unwrap()
    ));
    assert_eq!(evaluation["provenance"]["engine_provenance_complete"], true);
    let engine = &evaluation["provenance"]["engine"];
    assert_eq!(engine["schema"], "stratadiff-build-info-v1");
    assert_eq!(engine["engine_version"], "0.3.0");
    assert_eq!(engine["git_dirty"], false);
    assert_eq!(engine["build_profile"], "release");
    assert!(is_sha256(engine["cargo_lock_sha256"].as_str().unwrap()));
    let git_revision = engine["git_revision"].as_str().unwrap();
    assert!(
        git_revision.len() == 40
            && git_revision
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert!(
        engine["rustc_version"]
            .as_str()
            .unwrap()
            .starts_with("rustc ")
    );
    let evaluated_cases: BTreeMap<_, _> = evaluation["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| (case["id"].as_str().unwrap(), case))
        .collect();
    assert_eq!(evaluated_cases.len(), cases.len());
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let result = evaluated_cases[id];
        assert_eq!(result["passed"], true, "{id}");
        let oracle_path = case["expectation"]["oracle"].as_str().unwrap();
        assert_eq!(
            evaluation["provenance"]["oracle_sha256"][id],
            sha256(&fs::read(dataset.join(oracle_path)).unwrap())
        );
        if case["expectation"]["kind"] == "exact_identity_partition" {
            assert_eq!(result["engine_version"], engine["engine_version"], "{id}");
            for field in [
                "false_carry",
                "false_invalidation",
                "state_mismatches",
                "duplicate_product_identities",
                "identity_omissions",
                "identity_extras",
            ] {
                assert!(
                    result[field].as_array().unwrap().is_empty(),
                    "{id}: {field}"
                );
            }
            assert_eq!(result["retired_mismatch"], false, "{id}");
            assert_eq!(result["summary_mismatch"], false, "{id}");
        } else {
            assert_eq!(result["expected_rejection"], "base_mismatch_rejected");
            assert_eq!(result["matched_error"], true);
        }
    }
    assert_eq!(evaluation["summary"]["cases"], 5);
    assert_eq!(evaluation["summary"]["passed_cases"], 5);
    assert_eq!(evaluation["summary"]["current_identities"], current_total);
    assert_eq!(evaluation["summary"]["needs_review_now"], needs_total);
    assert_eq!(
        evaluation["summary"]["unchanged_since_checkpoint"],
        carried_total
    );
    assert_eq!(evaluation["summary"]["retired"], retired_total);
    assert_eq!(evaluation["summary"]["false_carry"], 0);
    assert_eq!(evaluation["summary"]["false_invalidation"], 0);
}
