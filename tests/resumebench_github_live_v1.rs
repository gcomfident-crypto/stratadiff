use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn dataset() -> PathBuf {
    repository_root().join("benchmarks/resumebench-github-live-v1")
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn files_under(root: &Path) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                );
            }
        }
    }
    files
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_tree(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).unwrap();
        }
    }
}

fn as_u64(value: &Value) -> u64 {
    value.as_u64().unwrap()
}

#[test]
fn github_live_v1_freezes_the_complete_evidence_and_claim_boundary() {
    let dataset = dataset();
    let manifest_path = dataset.join("manifest.json");
    let manifest = read_json(&manifest_path);
    let cases = manifest["cases"].as_array().unwrap();

    assert_eq!(
        manifest["schema"],
        "stratadiff-resumebench-github-live-manifest-v1"
    );
    assert_eq!(manifest["dataset_version"], "1.0.0");
    assert_eq!(
        manifest["dataset_kind"],
        "purposefully_selected_diagnostic_cases"
    );
    assert_eq!(manifest["claim_boundary"]["diagnostic_sample"], true);
    assert_eq!(
        manifest["claim_boundary"]["random_or_representative_sample"],
        false
    );
    assert_eq!(
        manifest["claim_boundary"]["population_estimates_supported"],
        false
    );
    assert_eq!(
        manifest["claim_boundary"]["human_priority_ground_truth"],
        "absent"
    );
    assert_eq!(
        manifest["claim_boundary"]["policy_ground_truth"],
        "frozen_independent_oracles"
    );
    assert_eq!(cases.len(), 5);

    let readme = fs::read_to_string(dataset.join("README.md")).unwrap();
    for boundary in [
        "purposefully selected diagnostic set, not a random or representative sample",
        "There is **no human-priority ground truth**",
        "estimate how often force-pushes occur",
        "does not restore or grant a GitHub approval",
    ] {
        assert!(
            readme.contains(boundary),
            "README lost claim boundary: {boundary}"
        );
    }

    let expected_paths: BTreeSet<_> = std::iter::once("README.md".to_owned())
        .chain(std::iter::once("manifest.json".to_owned()))
        .chain(std::iter::once("evaluation-v1.0.0.json".to_owned()))
        .chain(
            cases
                .iter()
                .map(|case| case["expectation"]["oracle"].as_str().unwrap().to_owned()),
        )
        .collect();
    let checksum_lines = fs::read_to_string(dataset.join("SHA256SUMS")).unwrap();
    let mut checksums = BTreeMap::new();
    for line in checksum_lines.lines() {
        let (digest, relative) = line.split_once("  ").unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(
            checksums
                .insert(relative.to_owned(), digest.to_owned())
                .is_none()
        );
    }
    assert_eq!(
        checksums.keys().cloned().collect::<BTreeSet<_>>(),
        expected_paths
    );
    for (relative, digest) in &checksums {
        assert_eq!(digest, &sha256(&fs::read(dataset.join(relative)).unwrap()));
    }
    let mut all_files = expected_paths.clone();
    all_files.insert("SHA256SUMS".to_owned());
    assert_eq!(files_under(&dataset), all_files);

    let snapshot_roles = [
        ("Q", "requested_base"),
        ("A", "checkpoint_merge_base"),
        ("B", "reviewed_checkpoint"),
        ("C", "current_merge_base"),
        ("D", "captured_final_head"),
    ];
    let mut ids = BTreeSet::new();
    let mut oracle_summaries = BTreeMap::new();
    let mut oracle_hashes = BTreeMap::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        assert!(ids.insert(id));
        assert_eq!(
            case["checkpoint_review"]["commit"],
            case["snapshots"]["B"]["commit"]
        );
        assert_eq!(
            case["pull_request"]["requested_base"],
            case["snapshots"]["Q"]["commit"]
        );
        assert_eq!(
            case["pull_request"]["captured_head"],
            case["snapshots"]["D"]["commit"]
        );
        assert_ne!(
            case["snapshots"]["B"]["commit"],
            case["snapshots"]["D"]["commit"]
        );
        assert_eq!(case["checkpoint_review"]["account_type"], "User");
        assert!(matches!(
            case["checkpoint_review"]["state"].as_str(),
            Some("APPROVED" | "CHANGES_REQUESTED")
        ));
        assert!(!case["force_push_chain"].as_array().unwrap().is_empty());
        for (label, role) in snapshot_roles {
            assert_eq!(case["snapshots"][label]["role"], role);
            let commit = case["snapshots"][label]["commit"].as_str().unwrap();
            assert_eq!(commit.len(), 40);
            assert!(
                commit
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            );
        }

        let observed = &case["observed"];
        assert_eq!(
            as_u64(&observed["current_pr_files"]),
            as_u64(&observed["exact_carries"])
                + as_u64(&observed["four_way_carries"])
                + as_u64(&observed["needs_review_now"])
        );
        assert_eq!(
            case["expectation"]["oracle_kind"],
            "exact_policy_conformance"
        );
        assert_eq!(case["expectation"]["human_priority_ground_truth"], "absent");
        let oracle_relative = case["expectation"]["oracle"].as_str().unwrap();
        assert!(oracle_relative.starts_with("oracles/"));
        assert!(!oracle_relative.contains(".."));
        let oracle_path = dataset.join(oracle_relative);
        let oracle = read_json(&oracle_path);
        assert_eq!(
            oracle["schema"],
            "stratadiff-resumebench-github-live-oracle-v1"
        );
        assert_eq!(oracle["dataset_version"], manifest["dataset_version"]);
        assert_eq!(oracle["case_id"], id);
        assert_eq!(oracle["oracle_kind"], "exact_policy_conformance");
        assert_eq!(oracle["human_priority_ground_truth"], "absent");
        for (label, _) in snapshot_roles {
            assert_eq!(
                oracle["snapshots"][label],
                case["snapshots"][label]["commit"]
            );
        }
        let expected_summary = json!({
            "current_pr_files": observed["current_pr_files"],
            "carried": as_u64(&observed["exact_carries"]) + as_u64(&observed["four_way_carries"]),
            "exactly_carried": observed["exact_carries"],
            "replay_carried": observed["four_way_carries"],
            "needs_review_now": observed["needs_review_now"],
            "retired_checkpoint_changes": observed["retired_checkpoint_changes"],
            "naive_snapshot_paths": observed["naive_snapshot_paths"],
            "naive_extra_paths": observed["naive_extra_paths"],
            "naive_missing_current_paths": observed["naive_missing_current_paths"],
        });
        assert_eq!(oracle["summary"], expected_summary);

        let classification = oracle["classification"].as_array().unwrap();
        assert_eq!(
            classification.len() as u64,
            as_u64(&oracle["summary"]["current_pr_files"])
        );
        let current_identities = oracle["current_identities"].as_array().unwrap();
        let current_identity_ids: BTreeSet<_> = current_identities
            .iter()
            .map(|identity| identity["identity_sha256"].as_str().unwrap())
            .collect();
        assert_eq!(current_identity_ids.len(), current_identities.len());
        let mut paths = BTreeSet::new();
        let mut exact = 0;
        let mut replay = 0;
        let mut residue = 0;
        for file in classification {
            assert!(paths.insert(file["path_base64"].as_str().unwrap()));
            assert!(
                current_identity_ids.contains(file["current_identity_sha256"].as_str().unwrap())
            );
            match file["checkpoint_state"].as_str().unwrap() {
                "unchanged_since_checkpoint" => {
                    match file["checkpoint_match_basis"].as_str().unwrap() {
                        "exact_git_change_identity" => exact += 1,
                        "exact_noninteracting_four_way_byte_replay" => replay += 1,
                        other => panic!("unexpected carry basis: {other}"),
                    }
                }
                "needs_review_now" => {
                    assert!(file.get("checkpoint_match_basis").is_none());
                    residue += 1;
                }
                other => panic!("unexpected checkpoint state: {other}"),
            }
        }
        assert_eq!(exact, as_u64(&oracle["summary"]["exactly_carried"]));
        assert_eq!(replay, as_u64(&oracle["summary"]["replay_carried"]));
        assert_eq!(residue, as_u64(&oracle["summary"]["needs_review_now"]));
        assert_eq!(
            oracle["checkpoint_identities"].as_array().unwrap().len(),
            (exact + replay + as_u64(&oracle["summary"]["retired_checkpoint_changes"])) as usize
        );
        assert_eq!(
            oracle["replay_witnesses"].as_array().unwrap().len() as u64,
            replay
        );
        assert_eq!(
            oracle["retired_checkpoint_identities"]
                .as_array()
                .unwrap()
                .len() as u64,
            as_u64(&oracle["summary"]["retired_checkpoint_changes"])
        );
        assert_eq!(
            oracle["naive_path_set"]["paths"],
            oracle["summary"]["naive_snapshot_paths"]
        );
        assert_eq!(
            oracle["naive_path_set"]["extra_paths"],
            oracle["summary"]["naive_extra_paths"]
        );
        assert_eq!(
            oracle["naive_path_set"]["missing_current_paths"],
            oracle["summary"]["naive_missing_current_paths"]
        );

        oracle_summaries.insert(id, oracle["summary"].clone());
        oracle_hashes.insert(id.to_owned(), sha256(&fs::read(oracle_path).unwrap()));
    }

    let totals = &manifest["observed_totals"];
    assert_eq!(totals["case_count"], 5);
    assert_eq!(totals["current_pr_files"], 47);
    assert_eq!(totals["exact_carries"], 23);
    assert_eq!(totals["four_way_carries"], 6);
    assert_eq!(totals["carried_files"], 29);
    assert_eq!(totals["needs_review_now"], 18);
    assert_eq!(totals["retired_checkpoint_changes"], 67);
    assert_eq!(totals["naive_snapshot_paths"], 1838);
    assert_eq!(totals["naive_extra_paths"], 1815);
    assert_eq!(totals["naive_missing_current_paths"], 24);

    let evaluation = read_json(&dataset.join("evaluation-v1.0.0.json"));
    assert_eq!(
        evaluation["schema"],
        "stratadiff-resumebench-github-live-evaluation-v1"
    );
    assert_eq!(evaluation["dataset_version"], manifest["dataset_version"]);
    assert_eq!(evaluation["benchmark_complete"], true);
    for boundary in [
        "purposefully selected",
        "no human-priority",
        "prevalence ground truth",
    ] {
        assert!(
            evaluation["claim_boundary"]
                .as_str()
                .unwrap()
                .contains(boundary)
        );
    }
    assert_eq!(
        evaluation["provenance"]["manifest_sha256"],
        sha256(&fs::read(&manifest_path).unwrap())
    );
    assert_eq!(
        evaluation["provenance"]["oracle_sha256"],
        json!(oracle_hashes)
    );
    assert_eq!(
        evaluation["provenance"]["verifier_sha256"],
        sha256(
            &fs::read(
                repository_root().join("tools/resumebench-github-live/resumebench_github_live.py"),
            )
            .unwrap()
        )
    );
    let summary = &evaluation["summary"];
    assert_eq!(summary["cases"], 5);
    assert_eq!(summary["passed_cases"], 5);
    assert_eq!(summary["false_carry"], 0);
    assert_eq!(summary["false_invalidation"], 0);
    assert_eq!(summary["basis_mismatches"], 0);
    assert_eq!(summary["identity_omissions"], 0);
    assert_eq!(summary["identity_extras"], 0);
    for (evaluation_key, oracle_key) in [
        ("current_pr_files", "current_pr_files"),
        ("carried", "carried"),
        ("exactly_carried", "exactly_carried"),
        ("replay_carried", "replay_carried"),
        ("needs_review_now", "needs_review_now"),
        ("retired_checkpoint_changes", "retired_checkpoint_changes"),
    ] {
        let total: u64 = oracle_summaries
            .values()
            .map(|item| as_u64(&item[oracle_key]))
            .sum();
        assert_eq!(as_u64(&summary[evaluation_key]), total);
    }
    let evaluation_cases = evaluation["cases"].as_array().unwrap();
    assert_eq!(evaluation_cases.len(), cases.len());
    for result in evaluation_cases {
        let id = result["id"].as_str().unwrap();
        assert_eq!(result["passed"], true);
        assert_eq!(result["summary"], oracle_summaries[id]);
        for field in [
            "false_carry",
            "false_invalidation",
            "basis_mismatches",
            "identity_omissions",
            "identity_extras",
            "duplicate_product_identities",
        ] {
            assert!(result[field].as_array().unwrap().is_empty());
        }
        assert_eq!(result["summary_mismatch"], false);
    }
}

#[test]
fn github_live_v1_bundle_verifier_rejects_a_tampered_oracle() {
    let source_dataset = dataset();
    let temporary = tempfile::tempdir().unwrap();
    let copied_dataset = temporary.path().join("resumebench-github-live-v1");
    copy_tree(&source_dataset, &copied_dataset);

    let manifest = read_json(&copied_dataset.join("manifest.json"));
    let oracle = copied_dataset.join(
        manifest["cases"][0]["expectation"]["oracle"]
            .as_str()
            .unwrap(),
    );
    let mut bytes = fs::read(&oracle).unwrap();
    let index = bytes.len() / 2;
    bytes[index] ^= 1;
    fs::write(oracle, bytes).unwrap();

    let output = Command::new("python3")
        .arg(repository_root().join("tools/resumebench-github-live/resumebench_github_live.py"))
        .arg("verify-bundle")
        .arg("--manifest")
        .arg(copied_dataset.join("manifest.json"))
        .current_dir(repository_root())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("checksum mismatch")
    );
}
