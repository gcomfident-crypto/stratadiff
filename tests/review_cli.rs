use std::{fs, path::Path, process::Command};

use stratadiff::{
    VerificationLimits,
    review::{
        CheckpointCarryBasis, CheckpointMatchBasis, CheckpointState, MAX_REVIEW_TOTAL_SOURCE_BYTES,
        RepositoryReview, ReviewLane, ReviewPriority, load_review_file_sources, markdown_report,
        regenerate_review_file_report, review_git_range_with_checkpoint, review_git_resume_delta,
        review_git_snapshot_delta,
    },
};

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

fn commit(repository: &Path, message: &str) -> String {
    git(repository, &["add", "--all"]);
    git(repository, &["commit", "-q", "-m", message]);
    git(repository, &["rev-parse", "HEAD"])
}

fn review_fixture() -> (tempfile::TempDir, String, String) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);

    fs::write(
        root.join("format.py"),
        "def total(values):\n    return sum(values)\n",
    )
    .unwrap();
    fs::write(root.join("logic.py"), "def answer():\n    return 1\n").unwrap();
    fs::write(
        root.join("moving.py"),
        "def preserved_name():\n    return 'same bytes'\n",
    )
    .unwrap();
    fs::write(
        root.join("removed.py"),
        "def obsolete_feature():\n    return 'remove me entirely'\n",
    )
    .unwrap();
    fs::write(root.join("notes.txt"), "old unsupported content\n").unwrap();
    let base = commit(root, "base");

    fs::write(
        root.join("format.py"),
        "def total( values ):\n    return sum( values )\n",
    )
    .unwrap();
    fs::write(root.join("logic.py"), "def answer():\n    return 2\n").unwrap();
    git(root, &["mv", "moving.py", "moved.py"]);
    fs::remove_file(root.join("removed.py")).unwrap();
    fs::write(
        root.join("created.py"),
        "def brand_new():\n    return 'created content'\n",
    )
    .unwrap();
    fs::write(root.join("notes.txt"), "new unsupported content\n").unwrap();
    let head = commit(root, "mixed change");
    (directory, base, head)
}

fn checkpoint_fixture() -> (tempfile::TempDir, String, String, String) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);

    fs::write(root.join("stable.py"), "value = 1\n").unwrap();
    fs::write(root.join("changing.py"), "value = 1\n").unwrap();
    fs::write(root.join("moving.py"), "moving_value = 1\n").unwrap();
    fs::write(root.join("removed.py"), "removed_value = 1\n").unwrap();
    fs::write(root.join("notes.txt"), "old\n").unwrap();
    let base = commit(root, "base");

    fs::write(root.join("stable.py"), "value = 2\n").unwrap();
    fs::write(root.join("changing.py"), "value = 2\n").unwrap();
    git(root, &["mv", "moving.py", "moved.py"]);
    fs::remove_file(root.join("removed.py")).unwrap();
    fs::write(root.join("created.py"), "created_value = 1\n").unwrap();
    fs::write(root.join("notes.txt"), "new\n").unwrap();
    let checkpoint = commit(root, "review checkpoint");

    git(root, &["checkout", "-q", &base]);
    fs::write(root.join("stable.py"), "value = 2\n").unwrap();
    fs::write(root.join("changing.py"), "value = 3\n").unwrap();
    git(root, &["mv", "moving.py", "relocated.py"]);
    fs::remove_file(root.join("removed.py")).unwrap();
    fs::write(root.join("created.py"), "created_value = 1\n").unwrap();
    fs::write(root.join("notes.txt"), "new\n").unwrap();
    let head = commit(root, "force-pushed successor");
    (directory, base, checkpoint, head)
}

#[test]
fn checkpoint_delta_is_the_direct_reviewed_snapshot_to_current_head_diff() {
    let (directory, _base, checkpoint, head) = checkpoint_fixture();
    let delta = review_git_snapshot_delta(directory.path(), &checkpoint, &head).unwrap();

    assert_eq!(delta.from_commit, checkpoint);
    assert_eq!(delta.source_base_commit, checkpoint);
    assert_eq!(delta.to_commit, head);
    assert_eq!(delta.comparison, "snapshot_to_snapshot");
    assert_eq!(delta.summary.changed_files, 2);
    assert_eq!(
        delta
            .files
            .iter()
            .map(|file| file.display_path())
            .collect::<Vec<_>>(),
        ["changing.py", "moved.py -> relocated.py"]
    );

    let changed = &delta.files[0];
    let sources = load_review_file_sources(directory.path(), changed).unwrap();
    assert_eq!(sources.before, b"value = 2\n");
    assert_eq!(sources.after, b"value = 3\n");
    let report = regenerate_review_file_report(changed, &sources).unwrap();
    assert!(report.certificate.patch_verified);
}

#[test]
fn review_command_triages_a_real_git_range_without_hiding_unverified_files() {
    let (directory, base, head) = review_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg(&base)
        .arg(&head)
        .arg("--repo")
        .arg(directory.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: RepositoryReview = serde_json::from_slice(&output.stdout).unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-v1.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let instance = serde_json::to_value(&report).unwrap();
    let errors: Vec<_> = validator
        .iter_errors(&instance)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "schema errors: {errors:#?}");

    assert_eq!(report.base_commit, base);
    assert_eq!(report.head_commit, head);
    assert_eq!(report.summary.changed_files, 6);
    assert_eq!(report.summary.first_pass_files, 6);
    assert_eq!(report.summary.review_first_files, 3);
    assert_eq!(report.summary.syntax_preserved_files, 1);
    assert_eq!(report.summary.content_preserved_files, 1);
    assert_eq!(report.summary.unverified_files, 1);
    assert_eq!(report.summary.replay_check_passed_files, 2);
    assert_eq!(report.summary.replay_check_not_run_files, 4);
    assert!(report.summary.line_envelope_complete);
    assert!(report.checkpoint.is_none());
    assert!(report.summary.checkpoint.is_none());
    assert!(
        report
            .files
            .iter()
            .all(|file| file.checkpoint_state.is_none())
    );

    let lane = |path: &str| {
        report
            .files
            .iter()
            .find(|file| file.display_path().contains(path))
            .unwrap()
            .lane
    };
    assert_eq!(lane("logic.py"), ReviewLane::ReviewFirst);
    assert_eq!(lane("created.py"), ReviewLane::ReviewFirst);
    assert_eq!(lane("removed.py"), ReviewLane::ReviewFirst);
    assert_eq!(lane("format.py"), ReviewLane::SyntaxPreserved);
    assert_eq!(lane("moving.py"), ReviewLane::ContentPreserved);
    assert_eq!(lane("notes.txt"), ReviewLane::Unverified);
    assert!(
        report
            .files
            .iter()
            .all(|file| file.priority == ReviewPriority::ReviewFirst)
    );
}

#[test]
fn checkpoint_resume_matches_only_complete_git_change_identities() {
    let (directory, base, checkpoint, head) = checkpoint_fixture();
    let report =
        review_git_range_with_checkpoint(directory.path(), &base, &head, Some(&checkpoint))
            .unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/review-v1.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let instance = serde_json::to_value(&report).unwrap();
    let errors: Vec<_> = validator
        .iter_errors(&instance)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "schema errors: {errors:#?}");
    let mut missing_file_state = instance.clone();
    missing_file_state["files"][0]
        .as_object_mut()
        .unwrap()
        .remove("checkpoint_state");
    assert!(!validator.is_valid(&missing_file_state));
    let mut missing_summary = instance.clone();
    missing_summary["summary"]
        .as_object_mut()
        .unwrap()
        .remove("checkpoint");
    assert!(!validator.is_valid(&missing_summary));

    let metadata = report.checkpoint.as_ref().unwrap();
    assert_eq!(metadata.requested_revision, checkpoint);
    assert_eq!(metadata.commit, checkpoint);
    assert_eq!(metadata.base_commit, base);
    assert_eq!(
        metadata.match_basis,
        CheckpointMatchBasis::ExactGitChangeIdentity
    );
    let summary = report.summary.checkpoint.as_ref().unwrap();
    assert_eq!(report.summary.changed_files, 6);
    assert_eq!(summary.needs_review_now_files, 2);
    assert_eq!(summary.unchanged_since_checkpoint_files, 4);
    assert_eq!(summary.retired_change_count, 2);
    assert_eq!(
        summary.needs_review_now_files + summary.unchanged_since_checkpoint_files,
        report.summary.changed_files
    );

    let state = |path: &str| {
        report
            .files
            .iter()
            .find(|file| file.display_path().contains(path))
            .unwrap()
            .checkpoint_state
    };
    assert_eq!(state("changing.py"), Some(CheckpointState::NeedsReviewNow));
    assert_eq!(state("relocated.py"), Some(CheckpointState::NeedsReviewNow));
    for path in ["stable.py", "created.py", "removed.py", "notes.txt"] {
        assert_eq!(
            state(path),
            Some(CheckpointState::UnchangedSinceCheckpoint),
            "unexpected checkpoint state for {path}"
        );
    }
    assert_eq!(
        report
            .files
            .iter()
            .find(|file| file.display_path() == "notes.txt")
            .unwrap()
            .lane,
        ReviewLane::Unverified
    );
    assert!(
        report
            .files
            .iter()
            .all(|file| file.priority == ReviewPriority::ReviewFirst)
    );

    let markdown = markdown_report(&report);
    assert!(markdown.contains("Review coverage: **2** of 6 current files need review"));
    assert!(markdown.contains("**4** carried (**4** exact-identity, **0** four-way)"));
    assert!(markdown.contains("| exact-identity carry |"));
    assert!(markdown.contains("Per-file diff reconstruction"));
    assert!(markdown.contains("<details>"));
    assert!(markdown.contains("Unchanged since checkpoint: <strong>4</strong>"));
    assert!(markdown.contains("**2** checkpoint changes retired"));
    assert!(markdown.contains("Cross-file effects were not checked"));
    assert!(markdown.contains(
        "Intrinsic priority before checkpoint carry-forward: **6** of 6 files are review first"
    ));
    assert!(!markdown.contains("Review first: **6** of 6 files"));
    assert!(
        markdown.find("## Needs review now").unwrap()
            < markdown.find("Unchanged since checkpoint:").unwrap()
    );

    let repeated =
        review_git_range_with_checkpoint(directory.path(), &base, &head, Some(&checkpoint))
            .unwrap();
    assert_eq!(
        serde_json::to_vec(&report).unwrap(),
        serde_json::to_vec(&repeated).unwrap()
    );
}

#[test]
fn review_command_accepts_an_explicit_checkpoint() {
    let (directory, base, checkpoint, head) = checkpoint_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg(&base)
        .arg(&head)
        .arg("--checkpoint")
        .arg(&checkpoint)
        .arg("--repo")
        .arg(directory.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: RepositoryReview = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.checkpoint.unwrap().commit, checkpoint);
    let summary = report.summary.checkpoint.unwrap();
    assert_eq!(summary.needs_review_now_files, 2);
    assert_eq!(summary.unchanged_since_checkpoint_files, 4);
    assert_eq!(summary.retired_change_count, 2);
}

#[test]
fn review_residue_gate_requires_a_checkpoint_and_zero_unreviewed_files() {
    let (directory, base, checkpoint, head) = checkpoint_fixture();
    let residue_report = directory.path().join("residue.json");
    let residue = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg("--repo")
        .arg(directory.path())
        .arg("--checkpoint")
        .arg(&checkpoint)
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&residue_report)
        .arg("--fail-on-review-residue")
        .arg("--")
        .arg(&base)
        .arg(&head)
        .output()
        .unwrap();
    assert!(!residue.status.success());
    assert!(
        String::from_utf8_lossy(&residue.stderr)
            .contains("review residue gate is open: 2 current PR files need review")
    );
    assert!(!fs::read(&residue_report).unwrap().is_empty());

    let no_checkpoint_report = directory.path().join("no-checkpoint.json");
    let no_checkpoint = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg("--repo")
        .arg(directory.path())
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&no_checkpoint_report)
        .arg("--fail-on-review-residue")
        .arg("--")
        .arg(&base)
        .arg(&head)
        .output()
        .unwrap();
    assert!(!no_checkpoint.status.success());
    assert!(
        String::from_utf8_lossy(&no_checkpoint.stderr)
            .contains("review residue gate requires a resolved checkpoint")
    );
    assert!(!fs::read(&no_checkpoint_report).unwrap().is_empty());

    let clean_report = directory.path().join("clean.json");
    let clean = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg("--repo")
        .arg(directory.path())
        .arg("--checkpoint")
        .arg(&head)
        .arg("--format")
        .arg("json")
        .arg("--output")
        .arg(&clean_report)
        .arg("--fail-on-review-residue")
        .arg("--")
        .arg(&base)
        .arg(&head)
        .output()
        .unwrap();
    assert!(
        clean.status.success(),
        "{}",
        String::from_utf8_lossy(&clean.stderr)
    );
    let report: RepositoryReview =
        serde_json::from_slice(&fs::read(clean_report).unwrap()).unwrap();
    assert_eq!(
        report
            .summary
            .checkpoint
            .as_ref()
            .unwrap()
            .needs_review_now_files,
        0
    );
}

#[test]
fn checkpoint_resume_accounts_for_new_and_retired_changes_separately() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);
    fs::write(root.join("kept.py"), "value = 0\n").unwrap();
    fs::write(root.join("retired.py"), "value = 0\n").unwrap();
    fs::write(root.join("new.py"), "value = 0\n").unwrap();
    let base = commit(root, "base");

    fs::write(root.join("kept.py"), "value = 1\n").unwrap();
    fs::write(root.join("retired.py"), "value = 1\n").unwrap();
    let checkpoint = commit(root, "checkpoint");

    git(root, &["checkout", "-q", &base]);
    fs::write(root.join("kept.py"), "value = 1\n").unwrap();
    fs::write(root.join("new.py"), "value = 1\n").unwrap();
    let head = commit(root, "current");

    let report = review_git_range_with_checkpoint(root, &base, &head, Some(&checkpoint)).unwrap();
    let summary = report.summary.checkpoint.as_ref().unwrap();
    assert_eq!(summary.needs_review_now_files, 1);
    assert_eq!(summary.unchanged_since_checkpoint_files, 1);
    assert_eq!(summary.retired_change_count, 1);
    assert_eq!(report.files[0].display_path(), "new.py");
    assert_eq!(
        report.files[0].checkpoint_state,
        Some(CheckpointState::NeedsReviewNow)
    );
    assert_eq!(report.files[1].display_path(), "kept.py");
    assert_eq!(
        report.files[1].checkpoint_state,
        Some(CheckpointState::UnchangedSinceCheckpoint)
    );
}

#[test]
fn checkpoint_resume_carries_noninteracting_rebase_changes_in_the_same_file() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);
    fs::write(
        root.join("shared.py"),
        "title = 'old'\nstable = 0\nreviewed = 0\n",
    )
    .unwrap();
    let original_base = commit(root, "original base");

    fs::write(
        root.join("shared.py"),
        "title = 'old'\nstable = 0\nreviewed = 1\n",
    )
    .unwrap();
    let checkpoint = commit(root, "reviewed checkpoint");

    git(root, &["checkout", "-q", &original_base]);
    fs::write(
        root.join("shared.py"),
        "title = 'new'\nstable = 0\nreviewed = 0\n",
    )
    .unwrap();
    let current_base = commit(root, "advanced base");
    fs::write(
        root.join("shared.py"),
        "title = 'new'\nstable = 0\nreviewed = 1\n",
    )
    .unwrap();
    let head = commit(root, "rebased current head");

    let report =
        review_git_range_with_checkpoint(root, &current_base, &head, Some(&checkpoint)).unwrap();
    assert_eq!(
        report.checkpoint.as_ref().unwrap().match_basis,
        CheckpointMatchBasis::ExactGitChangeIdentityOrNoninteractingFourWayByteReplay
    );
    assert_eq!(report.files.len(), 1);
    assert_eq!(
        report.files[0].checkpoint_state,
        Some(CheckpointState::UnchangedSinceCheckpoint)
    );
    assert_eq!(
        report.files[0].checkpoint_match_basis,
        Some(CheckpointCarryBasis::ExactNoninteractingFourWayByteReplay)
    );
    assert!(report.files[0].reason.contains("four-way byte replay"));
    let summary = report.summary.checkpoint.as_ref().unwrap();
    assert_eq!(summary.needs_review_now_files, 0);
    assert_eq!(summary.unchanged_since_checkpoint_files, 1);
    assert_eq!(summary.retired_change_count, 0);

    let residue = review_git_resume_delta(root, &report).unwrap();
    assert_eq!(residue.summary.changed_files, 0);
    let markdown = markdown_report(&report);
    assert!(markdown.contains("non-interacting four-way byte replay"));
    assert!(markdown.contains("**1** carried (**0** exact-identity, **1** four-way)"));
    assert!(markdown.contains("four-way carry"));
}

#[test]
fn checkpoint_resume_rejects_overlapping_changes_during_base_drift() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);
    fs::write(root.join("shared.py"), "value = 0000\n").unwrap();
    let original_base = commit(root, "original base");

    fs::write(root.join("shared.py"), "value = 1111\n").unwrap();
    let checkpoint = commit(root, "reviewed checkpoint");

    git(root, &["checkout", "-q", &original_base]);
    fs::write(root.join("shared.py"), "value = 2222\n").unwrap();
    let current_base = commit(root, "overlapping base update");
    fs::write(root.join("shared.py"), "value = 1111\n").unwrap();
    let head = commit(root, "resolved current head");

    let report =
        review_git_range_with_checkpoint(root, &current_base, &head, Some(&checkpoint)).unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(
        report.files[0].checkpoint_state,
        Some(CheckpointState::NeedsReviewNow)
    );
    assert_eq!(report.files[0].checkpoint_match_basis, None);
    let summary = report.summary.checkpoint.as_ref().unwrap();
    assert_eq!(summary.needs_review_now_files, 1);
    assert_eq!(summary.unchanged_since_checkpoint_files, 0);
    assert_eq!(summary.retired_change_count, 1);
}

#[test]
fn checkpoint_resume_uses_pr_relative_ranges_when_the_base_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);
    fs::write(root.join("seed.py"), "value = 1\n").unwrap();
    fs::write(root.join("shared.py"), "value = 0\n").unwrap();
    fs::write(root.join("divergent.py"), "value = 0\n").unwrap();
    let original_base = commit(root, "original base");

    fs::write(root.join("checkpoint.py"), "value = 1\n").unwrap();
    fs::write(root.join("shared.py"), "value = 1\n").unwrap();
    fs::write(root.join("divergent.py"), "value = 1\n").unwrap();
    let checkpoint = commit(root, "checkpoint branch");

    git(root, &["checkout", "-q", &original_base]);
    fs::write(root.join("base-update.py"), "value = 1\n").unwrap();
    let current_base = commit(root, "advanced base");
    fs::write(root.join("current.py"), "value = 1\n").unwrap();
    fs::write(root.join("shared.py"), "value = 1\n").unwrap();
    fs::write(root.join("divergent.py"), "value = 2\n").unwrap();
    let head = commit(root, "current head");

    let report =
        review_git_range_with_checkpoint(root, &current_base, &head, Some(&checkpoint)).unwrap();
    let checkpoint_metadata = report.checkpoint.as_ref().unwrap();
    assert_eq!(checkpoint_metadata.base_commit, original_base);
    assert_eq!(
        checkpoint_metadata.match_basis,
        CheckpointMatchBasis::ExactGitChangeIdentityOrNoninteractingFourWayByteReplay
    );
    assert_eq!(report.base_commit, current_base);
    assert_ne!(checkpoint_metadata.base_commit, report.base_commit);
    assert_eq!(report.summary.changed_files, 3);
    assert_eq!(report.files[0].display_path(), "current.py");
    assert_eq!(
        report.files[0].checkpoint_state,
        Some(CheckpointState::NeedsReviewNow)
    );
    assert_eq!(report.files[1].display_path(), "divergent.py");
    assert_eq!(
        report.files[1].checkpoint_state,
        Some(CheckpointState::NeedsReviewNow)
    );
    assert_eq!(report.files[2].display_path(), "shared.py");
    assert_eq!(
        report.files[2].checkpoint_state,
        Some(CheckpointState::UnchangedSinceCheckpoint)
    );
    assert_eq!(
        report.files[2].checkpoint_match_basis,
        Some(CheckpointCarryBasis::ExactGitChangeIdentity)
    );
    let summary = report.summary.checkpoint.as_ref().unwrap();
    assert_eq!(summary.needs_review_now_files, 2);
    assert_eq!(summary.unchanged_since_checkpoint_files, 1);
    assert_eq!(summary.retired_change_count, 2);

    let residue = review_git_resume_delta(root, &report).unwrap();
    assert_eq!(residue.comparison, "current_pr_unmatched_identities");
    assert_eq!(residue.from_commit, checkpoint);
    assert_eq!(residue.source_base_commit, current_base);
    assert_eq!(residue.to_commit, head);
    assert_eq!(residue.summary.changed_files, 2);
    assert_eq!(residue.files[0].display_path(), "current.py");
    assert_eq!(residue.files[1].display_path(), "divergent.py");

    let markdown = markdown_report(&report);
    assert!(markdown.contains("base changed"));
    assert!(markdown.contains(&original_base));
    assert!(markdown.contains(&current_base));
}

#[test]
fn markdown_output_is_ready_for_a_github_step_summary() {
    let (directory, base, head) = review_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg(&base)
        .arg(&head)
        .arg("--repo")
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let markdown = String::from_utf8(output.stdout).unwrap();

    assert!(markdown.starts_with("# StrataDiff review focus\n"));
    assert!(markdown.contains("Review first: **6** of 6 files"));
    assert!(markdown.contains("parser model matched (non-semantic)"));
    assert!(markdown.contains("same Git object"));
    assert!(markdown.contains("unverified"));
    assert!(markdown.contains("logic.py"));
}

#[test]
fn github_summary_can_be_written_alongside_the_json_artifact() {
    let (directory, base, head) = review_fixture();
    let summary_path = directory.path().join("step-summary.md");
    let report_path = directory.path().join("review.json");
    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg("--repo=.")
        .arg("--format=json")
        .arg(format!("--output={}", report_path.display()))
        .arg("--github-summary")
        .arg("--")
        .arg(&base)
        .arg(&head)
        .current_dir(directory.path())
        .env("GITHUB_STEP_SUMMARY", &summary_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: RepositoryReview = serde_json::from_slice(&fs::read(report_path).unwrap()).unwrap();
    assert_eq!(report.summary.changed_files, 6);
    let markdown = fs::read_to_string(summary_path).unwrap();
    assert!(markdown.starts_with("# StrataDiff review focus\n"));
    assert!(markdown.contains("Review first: **6** of 6 files"));
}

#[test]
fn review_command_rejects_an_unknown_revision() {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "-q"]);
    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg("missing-base")
        .arg("HEAD")
        .arg("--repo")
        .arg(directory.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("git rev-parse"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn review_command_treats_hyphen_prefixed_inputs_as_revisions_after_the_option_separator() {
    let (directory, base, head) = review_fixture();
    for (base_argument, head_argument) in [("--help", head.as_str()), (base.as_str(), "--help")] {
        let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
            .arg("review")
            .arg("--repo")
            .arg(directory.path())
            .arg("--format")
            .arg("json")
            .arg("--")
            .arg(base_argument)
            .arg(head_argument)
            .output()
            .unwrap();

        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("git rev-parse"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn composite_action_separates_revision_inputs_and_rejects_an_empty_report() {
    let action = include_str!("../action.yml");
    let script = include_str!("../scripts/github_action_review.sh");
    assert!(action.contains(
        "bash --noprofile --norc \"${GITHUB_ACTION_PATH}/scripts/github_action_review.sh\""
    ));
    assert!(script.contains("cd -- \"${GITHUB_ACTION_PATH}\""));
    assert!(script.contains("\"--repo=${STRATADIFF_REPOSITORY}\""));
    assert!(script.contains("github-checkpoint \"${reviews_path}\" --reviewer"));
    assert!(script.contains("! \"${resolved_checkpoint}\" =~ ^[0-9a-f]{40}$"));
    assert!(script.contains("review_args+=(\"--checkpoint=${resolved_checkpoint}\")"));
    assert!(script.contains("true) review_args+=(--fail-on-review-residue)"));
    assert!(script.contains("review_args+=(-- \"${STRATADIFF_BASE}\" \"${STRATADIFF_HEAD}\")"));
    assert!(script.contains("[[ ! -s \"${report_path}\" ]]"));
    assert!(script.contains("review_status=$?"));
    assert!(script.contains("exit \"${review_status}\""));
    assert!(script.contains("grep -qi 'rel=\"next\"'"));
    assert_eq!(script.matches("curl --disable --config").count(), 2);
    assert!(!script.contains("curl --config"));
    assert!(!script.contains("Authorization: Bearer ${STRATADIFF_GITHUB_TOKEN}"));
    assert!(script.contains("--max-filesize 8388608"));
    assert!(script.contains("--max-filesize 1048576"));
    assert!(script.contains("proto = \"=https\""));
    assert!(script.contains("/git/commits/${resolved_checkpoint}"));
    assert!(script.contains("github-commit-object \"${commit_object_path}\""));
    assert!(script.contains("git clone --bare --shared --quiet"));
    assert!(script.contains("git --git-dir=\"${provider_repository}\" fetch"));
    assert!(script.contains("\"${resolved_checkpoint}:${provider_ref}\""));
    assert!(script.contains("env -u STRATADIFF_GITHUB_TOKEN"));
    assert!(script.contains("\"${provider_ref}:${checkpoint_ref}\""));
    assert!(script.contains("rev-parse --verify \"${checkpoint_ref}^{commit}\""));
    assert!(script.contains("update-ref -d \"${checkpoint_ref}\" \"${resolved_checkpoint}\""));
}

#[test]
fn curl_disable_first_argument_ignores_a_malicious_user_curlrc() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::write(
        home.join(".curlrc"),
        "write-out = \"malicious-curlrc-loaded\"\n",
    )
    .unwrap();
    let source = directory.path().join("source.txt");
    let destination = directory.path().join("destination.txt");
    fs::write(&source, "provider response\n").unwrap();
    let source_url = format!("file://{}", source.display());

    let control = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--output")
        .arg(&destination)
        .arg(&source_url)
        .env("HOME", &home)
        .env("CURL_HOME", &home)
        .output()
        .unwrap();
    assert!(control.status.success());
    assert_eq!(control.stdout, b"malicious-curlrc-loaded");

    let protected = Command::new("curl")
        .arg("--disable")
        .arg("--silent")
        .arg("--show-error")
        .arg("--output")
        .arg(&destination)
        .arg(&source_url)
        .env("HOME", &home)
        .env("CURL_HOME", &home)
        .output()
        .unwrap();
    assert!(protected.status.success());
    assert!(protected.stdout.is_empty());
    assert_eq!(fs::read(&destination).unwrap(), b"provider response\n");
}

#[test]
fn isolated_provider_git_ignores_a_malicious_user_gitconfig() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let repository = directory.path().join("provider.git");
    fs::create_dir(&home).unwrap();
    fs::write(
        home.join(".gitconfig"),
        "[url \"https://attacker.invalid/\"]\n\tinsteadOf = https://github.com/\n",
    )
    .unwrap();
    let initialized = Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg("--quiet")
        .arg(&repository)
        .output()
        .unwrap();
    assert!(initialized.status.success());
    let provider_url = "https://github.com/example/project.git";

    let control = Command::new("git")
        .arg(format!("--git-dir={}", repository.display()))
        .arg("ls-remote")
        .arg("--get-url")
        .arg(provider_url)
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(control.status.success());
    assert_eq!(
        String::from_utf8(control.stdout).unwrap().trim(),
        "https://attacker.invalid/example/project.git"
    );

    let isolated_home = directory.path().join("isolated-home");
    fs::create_dir(&isolated_home).unwrap();
    let protected = Command::new("git")
        .arg(format!("--git-dir={}", repository.display()))
        .arg("ls-remote")
        .arg("--get-url")
        .arg(provider_url)
        .env("HOME", &isolated_home)
        .env("XDG_CONFIG_HOME", &isolated_home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(protected.status.success());
    assert_eq!(
        String::from_utf8(protected.stdout).unwrap().trim(),
        provider_url
    );
}

#[test]
fn review_is_bound_to_repo_despite_git_environment_and_local_order_config() {
    let (directory, base, head) = review_fixture();
    git(
        directory.path(),
        &["config", "diff.orderFile", "/definitely/missing/order-file"],
    );
    let other = tempfile::tempdir().unwrap();
    git(other.path(), &["init", "-q"]);

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg(&base)
        .arg(&head)
        .arg("--repo")
        .arg(directory.path())
        .arg("--format")
        .arg("json")
        .env("GIT_DIR", other.path().join(".git"))
        .env("GIT_WORK_TREE", other.path())
        .env("GIT_TRACE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: RepositoryReview = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.base_commit, base);
    assert_eq!(report.head_commit, head);
    assert_eq!(report.summary.changed_files, 6);
}

#[test]
fn replacement_refs_cannot_hide_the_requested_change() {
    let (directory, base, head) = review_fixture();
    git(
        directory.path(),
        &["checkout", "-q", "--orphan", "replacement"],
    );
    git(
        directory.path(),
        &["commit", "-q", "-m", "replacement tree"],
    );
    let replacement = git(directory.path(), &["rev-parse", "HEAD"]);
    git(directory.path(), &["replace", &base, &replacement]);

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg(&base)
        .arg(&head)
        .arg("--repo")
        .arg(directory.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: RepositoryReview = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.base_commit, base);
    assert_eq!(report.head_commit, head);
    assert_eq!(report.summary.changed_files, 6);
}

#[test]
fn shallow_repository_fails_closed_before_comparison() {
    let (directory, base, head) = review_fixture();
    fs::write(directory.path().join(".git/shallow"), format!("{base}\n")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg(&base)
        .arg(&head)
        .arg("--repo")
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("shallow repositories are not supported"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn changed_submodule_is_retained_as_unverified_instead_of_aborting_review() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);
    fs::write(root.join("seed.txt"), "one\n").unwrap();
    let first_target = commit(root, "first target");
    fs::write(root.join("seed.txt"), "two\n").unwrap();
    let second_target = commit(root, "second target");
    let first_cacheinfo = format!("160000,{first_target},vendor/example");
    git(
        root,
        &["update-index", "--add", "--cacheinfo", &first_cacheinfo],
    );
    git(root, &["commit", "-q", "-m", "add gitlink"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    let second_cacheinfo = format!("160000,{second_target},vendor/example");
    git(root, &["update-index", "--cacheinfo", &second_cacheinfo]);
    git(root, &["commit", "-q", "-m", "update gitlink"]);
    let head = git(root, &["rev-parse", "HEAD"]);
    git(root, &["config", "diff.ignoreSubmodules", "all"]);

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg(&base)
        .arg(&head)
        .arg("--repo")
        .arg(root)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: RepositoryReview = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.summary.changed_files, 1);
    assert_eq!(report.summary.unverified_files, 1);
    assert_eq!(report.files[0].lane, ReviewLane::Unverified);
    assert!(report.files[0].reason.contains("gitlink/submodule"));
    assert!(!report.summary.line_envelope_complete);
}

#[cfg(unix)]
#[test]
fn executable_bit_change_keeps_same_object_evidence_in_the_first_pass() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);
    let script = root.join("run.sh");
    fs::write(&script, "#!/bin/sh\necho ready\n").unwrap();
    let base = commit(root, "non-executable script");
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    let head = commit(root, "make script executable");

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg(&base)
        .arg(&head)
        .arg("--repo")
        .arg(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let markdown = String::from_utf8(output.stdout).unwrap();
    assert!(markdown.contains("same Git object"), "{markdown}");
    assert!(markdown.contains("mode 100644 -&gt; 100755"), "{markdown}");
    assert!(
        markdown.contains("file-mode effects remain in the first pass"),
        "{markdown}"
    );
}

#[cfg(unix)]
#[test]
fn non_structural_files_do_not_consume_the_structural_analysis_budget() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt, os::unix::fs::PermissionsExt};

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);

    let large_size = MAX_REVIEW_TOTAL_SOURCE_BYTES / 8;
    assert_eq!(large_size, VerificationLimits::default().max_source_bytes);
    fs::write(root.join("b-deleted-0.txt"), vec![b'd'; large_size]).unwrap();
    fs::write(root.join("b-deleted-1.txt"), vec![b'e'; large_size]).unwrap();
    let non_utf8_path = root.join(OsString::from_vec(b"c-non-utf8-\xff.py".to_vec()));
    fs::write(&non_utf8_path, vec![b'n'; large_size]).unwrap();
    let first_mode_path = root.join("d-mode-0.py");
    let second_mode_path = root.join("d-mode-1.py");
    fs::write(&first_mode_path, vec![b'm'; large_size]).unwrap();
    fs::write(&second_mode_path, vec![b'q'; large_size]).unwrap();
    fs::write(root.join("z-target.py"), "def target():\n    return 1\n").unwrap();
    let base = commit(root, "large non-structural base");

    fs::write(root.join("a-added-0.txt"), vec![b'a'; large_size]).unwrap();
    fs::write(root.join("a-added-1.txt"), vec![b'b'; large_size]).unwrap();
    fs::remove_file(root.join("b-deleted-0.txt")).unwrap();
    fs::remove_file(root.join("b-deleted-1.txt")).unwrap();
    fs::write(&non_utf8_path, vec![b'N'; large_size]).unwrap();
    for mode_path in [&first_mode_path, &second_mode_path] {
        let mut permissions = fs::metadata(mode_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(mode_path, permissions).unwrap();
    }
    fs::write(root.join("z-target.py"), "def target( ):\n    return 1\n").unwrap();
    let head = commit(root, "large mixed change");

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg(&base)
        .arg(&head)
        .arg("--repo")
        .arg(root)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: RepositoryReview = serde_json::from_slice(&output.stdout).unwrap();
    let target = report
        .files
        .iter()
        .find(|file| file.display_path() == "z-target.py")
        .unwrap();
    assert_eq!(target.lane, ReviewLane::SyntaxPreserved);
    assert!(target.evidence.is_some());
    assert!(target.line_change_envelope.is_none());
    assert!(
        report
            .files
            .iter()
            .all(|file| !file.reason.contains("aggregate analysis limit"))
    );
}

#[cfg(unix)]
#[test]
fn structural_analysis_reads_each_shared_blob_oid_once() {
    use std::{ffi::OsString, os::unix::fs::PermissionsExt};

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);
    for index in 0..3 {
        fs::write(
            root.join(format!("shared-{index}.py")),
            "def shared():\n    return 1\n",
        )
        .unwrap();
    }
    let base = commit(root, "shared base blobs");
    for index in 0..3 {
        fs::write(
            root.join(format!("shared-{index}.py")),
            "def shared():\n    return 2\n",
        )
        .unwrap();
    }
    let head = commit(root, "shared head blobs");
    let before_blob = git(root, &["rev-parse", &format!("{base}:shared-0.py")]);
    let after_blob = git(root, &["rev-parse", &format!("{head}:shared-0.py")]);

    let wrapper_directory = tempfile::tempdir().unwrap();
    let wrapper = wrapper_directory.path().join("git");
    let log = wrapper_directory.path().join("blob-reads.log");
    fs::write(
        &wrapper,
        "#!/bin/sh\nif [ \"$6\" = cat-file ] && [ \"$7\" = blob ]; then\n  printf '%s\\n' \"$8\" >> \"$STRATADIFF_GIT_LOG\"\nfi\nPATH=\"$STRATADIFF_REAL_PATH\"\nexport PATH\nexec git \"$@\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&wrapper).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper, permissions).unwrap();
    let real_path = std::env::var_os("PATH").unwrap();
    let mut wrapped_path = OsString::from(wrapper_directory.path().as_os_str());
    wrapped_path.push(":");
    wrapped_path.push(&real_path);

    let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg(&base)
        .arg(&head)
        .arg("--repo")
        .arg(root)
        .arg("--format")
        .arg("json")
        .env("PATH", wrapped_path)
        .env("STRATADIFF_REAL_PATH", real_path)
        .env("STRATADIFF_GIT_LOG", &log)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: RepositoryReview = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.summary.replay_check_passed_files, 3);
    let reads = fs::read_to_string(log).unwrap();
    assert_eq!(reads.lines().filter(|oid| *oid == before_blob).count(), 1);
    assert_eq!(reads.lines().filter(|oid| *oid == after_blob).count(), 1);
    assert_eq!(reads.lines().count(), 2);
}

#[test]
fn review_workbench_rejects_missing_checkpoint_and_output_modes() {
    let cases: &[(&[&str], &str)] = &[
        (
            &["review", "HEAD", "--workbench"],
            "--checkpoint <CHECKPOINT>",
        ),
        (
            &[
                "review",
                "HEAD",
                "--checkpoint",
                "HEAD",
                "--workbench",
                "--format",
                "json",
            ],
            "cannot be used with '--format <FORMAT>'",
        ),
        (
            &[
                "review",
                "HEAD",
                "--checkpoint",
                "HEAD",
                "--workbench",
                "--output",
                "review.json",
            ],
            "cannot be used with '--output <OUTPUT>'",
        ),
        (
            &[
                "review",
                "HEAD",
                "--checkpoint",
                "HEAD",
                "--workbench",
                "--github-summary",
            ],
            "cannot be used with '--github-summary'",
        ),
    ];

    for (arguments, expected_error) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
            .args(*arguments)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(expected_error), "{arguments:?}: {stderr}");
    }
}
