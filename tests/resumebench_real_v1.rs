use std::{collections::BTreeMap, fs, path::PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn dataset() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benchmarks/resumebench-real-v1")
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn rebase_oracle_freezes_the_complete_five_two_partition() {
    let dataset = dataset();
    for line in fs::read_to_string(dataset.join("SHA256SUMS"))
        .unwrap()
        .lines()
    {
        let (expected, relative) = line.split_once("  ").unwrap();
        assert_eq!(expected, sha256(&fs::read(dataset.join(relative)).unwrap()));
    }

    let manifest: Value =
        serde_json::from_slice(&fs::read(dataset.join("manifest.json")).unwrap()).unwrap();
    let oracle: Value =
        serde_json::from_slice(&fs::read(dataset.join("oracle.json")).unwrap()).unwrap();
    let evaluation: Value =
        serde_json::from_slice(&fs::read(dataset.join("evaluation-v1.0.0.json")).unwrap()).unwrap();

    assert_eq!(
        manifest["schema"],
        "stratadiff-resumebench-real-manifest-v1"
    );
    assert_eq!(oracle["schema"], "stratadiff-resumebench-real-oracle-v1");
    assert_eq!(manifest["dataset_version"], "1.0.0");
    assert_eq!(manifest["case"]["expected_summary"], oracle["summary"]);
    assert_eq!(oracle["summary"]["current_pr_files"], 7);
    assert_eq!(oracle["summary"]["carried"], 5);
    assert_eq!(oracle["summary"]["exactly_carried"], 4);
    assert_eq!(oracle["summary"]["replay_carried"], 1);
    assert_eq!(oracle["summary"]["needs_review_now"], 2);
    assert_eq!(oracle["summary"]["retired_checkpoint_changes"], 2);

    let classification: BTreeMap<_, _> = oracle["classification"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| (file["path_utf8"].as_str().unwrap(), file))
        .collect();
    assert_eq!(classification.len(), 7);
    assert_eq!(
        classification["Documentation/user-search.txt"]["checkpoint_match_basis"],
        "exact_noninteracting_four_way_byte_replay"
    );
    for path in [
        "java/com/google/gerrit/server/query/change/ChangeQueryBuilder.java",
        "java/com/google/gerrit/server/query/change/RegexOnlyPathsPredicate.java",
    ] {
        assert_eq!(classification[path]["checkpoint_state"], "needs_review_now");
        assert!(classification[path]["checkpoint_match_basis"].is_null());
    }

    assert_eq!(evaluation["benchmark_complete"], true);
    assert_eq!(evaluation["summary"], oracle["summary"]);
    assert_eq!(evaluation["provenance"]["engine"]["git_dirty"], false);
    assert_eq!(
        evaluation["provenance"]["manifest_sha256"],
        sha256(&fs::read(dataset.join("manifest.json")).unwrap())
    );
    assert_eq!(
        evaluation["provenance"]["oracle_sha256"],
        sha256(&fs::read(dataset.join("oracle.json")).unwrap())
    );
}
