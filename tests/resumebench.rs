use std::{collections::BTreeSet, fs, path::Path, process::Command};

use serde::Deserialize;
use stratadiff::review::{CheckpointState, review_git_range_with_checkpoint};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    name: String,
    description: String,
    claim: String,
    known_limitations: Vec<String>,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    id: String,
    mutation_operator: String,
    base_files: Vec<FileSnapshot>,
    checkpoint_files: Vec<FileSnapshot>,
    current_files: Vec<FileSnapshot>,
    expected_needs_review_now: Vec<String>,
    expected_unchanged_since_checkpoint: Vec<String>,
    expected_retired_change_count: usize,
    rationale: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSnapshot {
    path: String,
    mode: String,
    content: String,
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!("../benchmarks/resumebench-seed-v1.json")).unwrap()
}

fn git(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn commit_snapshot(
    repository: &Path,
    all_paths: &BTreeSet<String>,
    files: &[FileSnapshot],
    message: &str,
) -> String {
    for relative in all_paths {
        let path = repository.join(relative);
        if path.is_file() {
            fs::remove_file(path).unwrap();
        }
    }
    for file in files {
        let path = repository.join(&file.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, &file.content).unwrap();
    }
    git(repository, &["add", "--all"]);
    for file in files {
        match file.mode.as_str() {
            "100644" => {}
            "100755" => {
                git(
                    repository,
                    &["update-index", "--chmod=+x", "--", &file.path],
                );
            }
            mode => panic!("unsupported fixture mode {mode} in {message}"),
        }
    }
    git(
        repository,
        &["commit", "-q", "--allow-empty", "-m", message],
    );
    git(repository, &["rev-parse", "HEAD"])
}

fn scenario_paths(scenario: &Scenario) -> BTreeSet<String> {
    scenario
        .base_files
        .iter()
        .chain(&scenario.checkpoint_files)
        .chain(&scenario.current_files)
        .map(|file| file.path.clone())
        .collect()
}

#[test]
fn seed_is_well_formed_and_every_mutation_is_invalidated() {
    let corpus = corpus();
    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.name, "resumebench-seed-v1");
    assert!(!corpus.description.trim().is_empty());
    assert!(!corpus.claim.trim().is_empty());
    assert!(corpus.known_limitations.len() >= 5);
    assert!(corpus.scenarios.len() >= 6);

    let mut ids = BTreeSet::new();
    let mut mutation_operators = BTreeSet::new();
    let mut expected_mutations = 0_usize;
    let mut observed_mutations = 0_usize;
    let mut current_changes = 0_usize;
    let mut carried_changes = 0_usize;

    for scenario in corpus.scenarios {
        assert!(
            ids.insert(scenario.id.clone()),
            "duplicate id: {}",
            scenario.id
        );
        assert!(!scenario.rationale.trim().is_empty(), "{}", scenario.id);
        assert!(
            mutation_operators.insert(scenario.mutation_operator.clone()),
            "duplicate mutation operator: {}",
            scenario.mutation_operator
        );
        let all_paths = scenario_paths(&scenario);
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.name", "StrataDiff ResumeBench"]);
        git(
            root,
            &["config", "user.email", "resumebench@stratadiff.test"],
        );

        let base = commit_snapshot(root, &all_paths, &scenario.base_files, "base");
        let checkpoint = commit_snapshot(
            root,
            &all_paths,
            &scenario.checkpoint_files,
            "reviewed checkpoint",
        );
        git(root, &["checkout", "-q", "-f", "--detach", &base]);
        let current = commit_snapshot(
            root,
            &all_paths,
            &scenario.current_files,
            "rewritten current head",
        );

        let report = review_git_range_with_checkpoint(root, &base, &current, Some(&checkpoint))
            .unwrap_or_else(|error| panic!("{} failed: {error:#}", scenario.id));
        let checkpoint_summary = report.summary.checkpoint.as_ref().unwrap();
        let actual_needs: BTreeSet<_> = report
            .files
            .iter()
            .filter(|file| file.checkpoint_state == Some(CheckpointState::NeedsReviewNow))
            .map(|file| file.display_path())
            .collect();
        let actual_unchanged: BTreeSet<_> = report
            .files
            .iter()
            .filter(|file| file.checkpoint_state == Some(CheckpointState::UnchangedSinceCheckpoint))
            .map(|file| file.display_path())
            .collect();
        let expected_needs: BTreeSet<_> = scenario.expected_needs_review_now.into_iter().collect();
        let expected_unchanged: BTreeSet<_> = scenario
            .expected_unchanged_since_checkpoint
            .into_iter()
            .collect();

        assert_eq!(actual_needs, expected_needs, "{}", scenario.id);
        assert_eq!(actual_unchanged, expected_unchanged, "{}", scenario.id);
        assert_eq!(
            checkpoint_summary.retired_change_count, scenario.expected_retired_change_count,
            "{}",
            scenario.id
        );
        assert_eq!(
            checkpoint_summary.needs_review_now_files
                + checkpoint_summary.unchanged_since_checkpoint_files,
            report.summary.changed_files,
            "{} did not account for every current change exactly once",
            scenario.id
        );

        expected_mutations += expected_needs.len();
        observed_mutations += actual_needs.len();
        current_changes += report.summary.changed_files;
        carried_changes += checkpoint_summary.unchanged_since_checkpoint_files;
    }

    assert_eq!(observed_mutations, expected_mutations);
    assert!(expected_mutations >= 3);
    assert!(carried_changes > 0);
    assert!(carried_changes < current_changes);
}

#[test]
fn two_hundred_file_push_focuses_the_five_changed_identities() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff ResumeBench"]);
    git(
        root,
        &["config", "user.email", "resumebench@stratadiff.test"],
    );

    let make_files = |revision: usize| {
        (0..200)
            .map(|index| {
                let value = if revision == 0 {
                    0
                } else if revision == 2 && index < 5 {
                    2
                } else {
                    1
                };
                FileSnapshot {
                    path: format!("src/generated_{index:03}.py"),
                    mode: "100644".to_owned(),
                    content: format!("value = {value}\n"),
                }
            })
            .collect::<Vec<_>>()
    };
    let base_files = make_files(0);
    let checkpoint_files = make_files(1);
    let current_files = make_files(2);
    let all_paths: BTreeSet<_> = base_files.iter().map(|file| file.path.clone()).collect();
    let base = commit_snapshot(root, &all_paths, &base_files, "base");
    let checkpoint = commit_snapshot(root, &all_paths, &checkpoint_files, "reviewed checkpoint");
    git(root, &["checkout", "-q", "-f", "--detach", &base]);
    let current = commit_snapshot(root, &all_paths, &current_files, "five-file follow-up");

    let report =
        review_git_range_with_checkpoint(root, &base, &current, Some(&checkpoint)).unwrap();
    let summary = report.summary.checkpoint.unwrap();
    assert_eq!(report.summary.changed_files, 200);
    assert_eq!(summary.needs_review_now_files, 5);
    assert_eq!(summary.unchanged_since_checkpoint_files, 195);
    assert_eq!(summary.retired_change_count, 5);
}
