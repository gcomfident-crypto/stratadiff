use std::{env, fs, path::Path, process::Command};

use anyhow::{Context, Result, ensure};
use stratadiff::review::{
    CheckpointCarryBasis, ReviewDeltaBaselineBasis, load_review_delta_file_sources,
    review_git_range_with_checkpoint, review_git_resume_delta,
};

use crate::viewer;

const EXACT_CARRY_FILES: usize = 21;
const FOUR_WAY_CARRY_FILES: usize = 4;
const CURRENT_FILES: usize = 26;
const NEEDS_REVIEW_FILES: usize = 1;

const EXACT_ORIGINAL_SOURCE: &[u8] = b"reviewed = 0\n";
const EXACT_REVIEWED_SOURCE: &[u8] = b"reviewed = 1\n";
const FOUR_WAY_ORIGINAL_SOURCE: &[u8] = b"upstream = 'old'\nanchor = 0\nreviewed = 0\n";
const FOUR_WAY_REVIEWED_SOURCE: &[u8] = b"upstream = 'old'\nanchor = 0\nreviewed = 1\n";
const FOUR_WAY_ADVANCED_BASE_SOURCE: &[u8] = b"upstream = 'new'\nanchor = 0\nreviewed = 0\n";
const FOUR_WAY_CURRENT_SOURCE: &[u8] = b"upstream = 'new'\nanchor = 0\nreviewed = 1\n";
const ORIGINAL_SOURCE: &[u8] = b"title = 'old'\nreviewed = 0\nfollowup = 0\n";
const REVIEWED_SOURCE: &[u8] = b"title = 'old'\nreviewed = 1\nfollowup = 0\n";
const ADVANCED_BASE_SOURCE: &[u8] = b"title = 'new'\nreviewed = 0\nfollowup = 0\n";
const CURRENT_SOURCE: &[u8] = b"title = 'new'\nreviewed = 1\nfollowup = 1\n";
const RECONSTRUCTED_SOURCE: &[u8] = b"title = 'new'\nreviewed = 1\nfollowup = 0\n";

struct DemoHistory {
    _workspace: tempfile::TempDir,
    repository: std::path::PathBuf,
    original_base: String,
    checkpoint: String,
    current_base: String,
    head: String,
}

#[derive(Debug, PartialEq, Eq)]
struct DemoOutcome {
    current_files: usize,
    needs_review_files: usize,
    carried_files: usize,
    exact_carries: usize,
    four_way_carries: usize,
}

impl DemoOutcome {
    fn value_line(&self) -> String {
        format!(
            "{} current files -> {} needs review; {} carried ({} exact-identity, {} strict four-way).",
            self.current_files,
            self.needs_review_files,
            self.carried_files,
            self.exact_carries,
            self.four_way_carries,
        )
    }
}

pub fn run(port: u16, no_open: bool) -> Result<()> {
    let history = build_history()?;
    let review = review_git_range_with_checkpoint(
        &history.repository,
        &history.current_base,
        &history.head,
        Some(&history.checkpoint),
    )?;
    let outcome = validate_demo(&history, &review)?;

    println!("{}", outcome.value_line());
    println!(
        "The base moved, yet the reconstructed residue is one follow-up line. A {} -> B {} (reviewed), C {} (new base) -> D {} (current head).",
        short_oid(&history.original_base),
        short_oid(&history.checkpoint),
        short_oid(&history.current_base),
        short_oid(&history.head),
    );
    viewer::serve_review(review, history.repository.clone(), port, !no_open)
}

fn build_history() -> Result<DemoHistory> {
    let workspace = tempfile::Builder::new()
        .prefix("stratadiff-demo-")
        .tempdir()
        .context("failed to create the isolated demo workspace")?;
    let repository = workspace.path().join("repository");
    let home = workspace.path().join("home");
    let hooks = workspace.path().join("empty-hooks");
    fs::create_dir(&repository).context("failed to create the demo repository")?;
    fs::create_dir(&home).context("failed to create the isolated demo home")?;
    fs::create_dir(&hooks).context("failed to create the empty demo hooks directory")?;

    git(&repository, &home, &hooks, &["init", "--quiet"])?;
    write_snapshot(
        &repository,
        EXACT_ORIGINAL_SOURCE,
        FOUR_WAY_ORIGINAL_SOURCE,
        ORIGINAL_SOURCE,
    )?;
    let original_base = commit(&repository, &home, &hooks, "A: original base")?;

    write_snapshot(
        &repository,
        EXACT_REVIEWED_SOURCE,
        FOUR_WAY_REVIEWED_SOURCE,
        REVIEWED_SOURCE,
    )?;
    let checkpoint = commit(&repository, &home, &hooks, "B: reviewed author change")?;

    git(
        &repository,
        &home,
        &hooks,
        &["checkout", "--quiet", "--detach", &original_base],
    )?;
    write_snapshot(
        &repository,
        EXACT_ORIGINAL_SOURCE,
        FOUR_WAY_ADVANCED_BASE_SOURCE,
        ADVANCED_BASE_SOURCE,
    )?;
    let current_base = commit(&repository, &home, &hooks, "C: upstream base change")?;

    write_snapshot(
        &repository,
        EXACT_REVIEWED_SOURCE,
        FOUR_WAY_CURRENT_SOURCE,
        CURRENT_SOURCE,
    )?;
    let head = commit(
        &repository,
        &home,
        &hooks,
        "D: rebased change plus one follow-up",
    )?;

    Ok(DemoHistory {
        _workspace: workspace,
        repository,
        original_base,
        checkpoint,
        current_base,
        head,
    })
}

fn write_snapshot(
    repository: &Path,
    exact_source: &[u8],
    four_way_source: &[u8],
    residue_source: &[u8],
) -> Result<()> {
    let exact_directory = repository.join("src/exact");
    let four_way_directory = repository.join("src/four-way");
    fs::create_dir_all(&exact_directory)
        .context("failed to create the exact-carry demo directory")?;
    fs::create_dir_all(&four_way_directory)
        .context("failed to create the four-way demo directory")?;
    for index in 1..=EXACT_CARRY_FILES {
        fs::write(
            exact_directory.join(format!("reviewed-{index:02}.py")),
            exact_source,
        )
        .context("failed to write an exact-carry demo source")?;
    }
    for index in 1..=FOUR_WAY_CARRY_FILES {
        fs::write(
            four_way_directory.join(format!("reviewed-{index:02}.py")),
            four_way_source,
        )
        .context("failed to write a four-way demo source")?;
    }
    fs::write(repository.join("shared.py"), residue_source)
        .context("failed to write the review-residue demo source")
}

fn commit(repository: &Path, home: &Path, hooks: &Path, message: &str) -> Result<String> {
    git(repository, home, hooks, &["add", "--all"])?;
    git(
        repository,
        home,
        hooks,
        &["commit", "--quiet", "-m", message],
    )?;
    git(repository, home, hooks, &["rev-parse", "HEAD"])
}

fn git(repository: &Path, home: &Path, hooks: &Path, arguments: &[&str]) -> Result<String> {
    let path = env::var_os("PATH").context("PATH is required to run the offline Git demo")?;
    let hooks_setting = format!("core.hooksPath={}", hooks.display());
    let template_setting = format!("init.templateDir={}", hooks.display());
    let mut command = Command::new("git");
    command
        .env_clear()
        .env("PATH", path)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/bin/false")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_AUTHOR_NAME", "StrataDiff Demo")
        .env("GIT_AUTHOR_EMAIL", "demo@stratadiff.invalid")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00+00:00")
        .env("GIT_COMMITTER_NAME", "StrataDiff Demo")
        .env("GIT_COMMITTER_EMAIL", "demo@stratadiff.invalid")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00+00:00")
        .arg("-C")
        .arg(repository)
        .arg("-c")
        .arg(&hooks_setting)
        .arg("-c")
        .arg(&template_setting)
        .arg("-c")
        .arg("commit.gpgSign=false")
        .arg("-c")
        .arg("core.autocrlf=false")
        .args(arguments);
    let output = command
        .output()
        .with_context(|| format!("failed to execute git {}", display_arguments(arguments)))?;
    ensure!(
        output.status.success(),
        "offline demo git {} failed: {}",
        display_arguments(arguments),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8(output.stdout)
        .context("offline demo Git output was not UTF-8")
        .map(|output| output.trim().to_owned())
}

fn display_arguments(arguments: &[&str]) -> String {
    arguments.join(" ")
}

fn validate_demo(
    history: &DemoHistory,
    review: &stratadiff::review::RepositoryReview,
) -> Result<DemoOutcome> {
    let review_checkpoint = review
        .checkpoint
        .as_ref()
        .context("demo invariant failed: checkpoint metadata is unavailable")?;
    ensure!(
        review_checkpoint.base_commit == history.original_base
            && review_checkpoint.commit == history.checkpoint
            && review.base_commit == history.current_base
            && review.head_commit == history.head
            && review_checkpoint.base_commit != review.base_commit,
        "demo invariant failed: expected a bound A/B/C/D history with base drift"
    );
    let checkpoint_summary = review
        .summary
        .checkpoint
        .as_ref()
        .context("demo invariant failed: checkpoint summary is unavailable")?;
    let exact_carries = review
        .files
        .iter()
        .filter(|file| {
            file.checkpoint_match_basis == Some(CheckpointCarryBasis::ExactGitChangeIdentity)
        })
        .count();
    let four_way_carries = review
        .files
        .iter()
        .filter(|file| {
            file.checkpoint_match_basis
                == Some(CheckpointCarryBasis::ExactNoninteractingFourWayByteReplay)
        })
        .count();
    let outcome = DemoOutcome {
        current_files: review.summary.changed_files,
        needs_review_files: checkpoint_summary.needs_review_now_files,
        carried_files: checkpoint_summary.unchanged_since_checkpoint_files,
        exact_carries,
        four_way_carries,
    };
    ensure!(
        outcome.current_files == CURRENT_FILES
            && outcome.exact_carries == EXACT_CARRY_FILES
            && outcome.four_way_carries == FOUR_WAY_CARRY_FILES
            && outcome.needs_review_files == NEEDS_REVIEW_FILES
            && outcome.carried_files == EXACT_CARRY_FILES + FOUR_WAY_CARRY_FILES,
        "demo invariant failed: expected 26/21/4/1 current/exact/four-way/review files, observed {}/{}/{}/{}",
        outcome.current_files,
        outcome.exact_carries,
        outcome.four_way_carries,
        outcome.needs_review_files,
    );
    let delta = review_git_resume_delta(&history.repository, review)?;
    ensure!(
        delta.entries.len() == NEEDS_REVIEW_FILES
            && delta.summary.needs_review_files == NEEDS_REVIEW_FILES,
        "demo invariant failed: expected exactly one file in the Resume queue"
    );
    let entry = &delta.entries[0];
    ensure!(
        entry.display_path() == "shared.py"
            && entry.baseline_basis == ReviewDeltaBaselineBasis::ReconstructedReviewBaseline,
        "demo invariant failed: expected a reconstructed shared.py review baseline"
    );
    let line_changes = entry
        .file
        .line_change_envelope
        .as_ref()
        .context("demo invariant failed: follow-up line change is unavailable")?;
    ensure!(
        line_changes.additions == 1 && line_changes.deletions == 1,
        "demo invariant failed: expected one changed line"
    );
    let sources = load_review_delta_file_sources(&history.repository, entry)?;
    ensure!(
        sources.before == RECONSTRUCTED_SOURCE && sources.after == CURRENT_SOURCE,
        "demo invariant failed: Resume sources differ from the deterministic scenario"
    );
    Ok(outcome)
}

fn short_oid(object_id: &str) -> &str {
    &object_id[..8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_and_single_line_resume_delta_are_deterministic() {
        let first = build_history().unwrap();
        let review = review_git_range_with_checkpoint(
            &first.repository,
            &first.current_base,
            &first.head,
            Some(&first.checkpoint),
        )
        .unwrap();
        let outcome = validate_demo(&first, &review).unwrap();
        assert_eq!(
            outcome.value_line(),
            "26 current files -> 1 needs review; 25 carried (21 exact-identity, 4 strict four-way)."
        );

        let second = build_history().unwrap();
        assert_eq!(first.original_base, second.original_base);
        assert_eq!(first.checkpoint, second.checkpoint);
        assert_eq!(first.current_base, second.current_base);
        assert_eq!(first.head, second.head);
    }
}
