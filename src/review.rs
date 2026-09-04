use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsStr,
    io::{self, Read},
    path::Path,
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use crate::{
    ByteEdit, ChangeKind, DiffReport, Language, LosslessPatch, VerificationLimits,
    analyze_bytes_with_limits, apply_patch, patch::create_patch,
};

pub const REVIEW_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-v1.schema.json";
pub const MAX_REVIEW_FILES: usize = 1_000;
pub const MAX_REVIEW_TOTAL_SOURCE_BYTES: usize = 128 * 1024 * 1024;
const MAX_REVIEW_TOTAL_LINE_STAT_BYTES: usize = 128 * 1024 * 1024;
const MAX_REVIEW_MARKDOWN_BYTES: usize = 900 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Copied,
    Deleted,
    Modified,
    Renamed,
    TypeChanged,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewLane {
    ReviewFirst,
    SyntaxPreserved,
    ContentPreserved,
    Unverified,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PathEncoding {
    Utf8,
    GitBytesPercentEncoded,
}

impl ReviewLane {
    pub fn label(self) -> &'static str {
        match self {
            Self::ReviewFirst => "structural delta",
            Self::SyntaxPreserved => "parser model matched (non-semantic)",
            Self::ContentPreserved => "same Git object",
            Self::Unverified => "unverified",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPriority {
    ReviewFirst,
    EvidenceFollowUp,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointState {
    NeedsReviewNow,
    UnchangedSinceCheckpoint,
}

impl CheckpointState {
    fn label(self) -> &'static str {
        match self {
            Self::NeedsReviewNow => "needs review now",
            Self::UnchangedSinceCheckpoint => "unchanged since checkpoint",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointMatchBasis {
    ExactGitChangeIdentity,
    ExactGitChangeIdentityOrNoninteractingFourWayByteReplay,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointCarryBasis {
    ExactGitChangeIdentity,
    ExactNoninteractingFourWayByteReplay,
}

impl CheckpointCarryBasis {
    fn label(self) -> &'static str {
        match self {
            Self::ExactGitChangeIdentity => "exact-identity carry",
            Self::ExactNoninteractingFourWayByteReplay => "four-way carry",
        }
    }
}

impl ReviewPriority {
    fn label(self) -> &'static str {
        match self {
            Self::ReviewFirst => "review first",
            Self::EvidenceFollowUp => "evidence follow-up",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LineChangeEnvelope {
    pub additions: usize,
    pub deletions: usize,
}

impl LineChangeEnvelope {
    fn total(&self) -> usize {
        self.additions + self.deletions
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChangeCounts {
    pub insertions: usize,
    pub deletions: usize,
    pub equivalent_relocations: usize,
    pub child_order_changes: usize,
    pub model_forced_updates: usize,
    pub suggested_updates: usize,
    pub formatting_only: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileEvidence {
    pub report_blake3: String,
    pub replay_check_passed_during_analysis: bool,
    pub model_forced_relations: usize,
    pub suggested_relations: usize,
    pub ambiguity_groups: usize,
    pub byte_edits: usize,
    pub changes: ChangeCounts,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewFile {
    pub status: FileStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_path_encoding: Option<PathEncoding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_path_encoding: Option<PathEncoding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_blob: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_blob: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_change_envelope: Option<LineChangeEnvelope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
    pub priority: ReviewPriority,
    pub lane: ReviewLane,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_state: Option<CheckpointState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_match_basis: Option<CheckpointCarryBasis>,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<FileEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewCheckpoint {
    pub requested_revision: String,
    pub commit: String,
    pub base_commit: String,
    pub match_basis: CheckpointMatchBasis,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CheckpointSummary {
    pub needs_review_now_files: usize,
    pub unchanged_since_checkpoint_files: usize,
    pub retired_change_count: usize,
}

impl ReviewFile {
    pub fn display_path(&self) -> String {
        match (&self.before_path, &self.after_path) {
            (Some(before), Some(after)) if before != after => format!("{before} -> {after}"),
            (_, Some(after)) => after.clone(),
            (Some(before), None) => before.clone(),
            (None, None) => "<unknown>".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewSummary {
    pub changed_files: usize,
    pub first_pass_files: usize,
    pub review_first_files: usize,
    pub syntax_preserved_files: usize,
    pub content_preserved_files: usize,
    pub unverified_files: usize,
    pub replay_check_passed_files: usize,
    pub replay_check_not_run_files: usize,
    pub line_envelope_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_line_envelope: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_pass_line_envelope: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<CheckpointSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RepositoryReview {
    pub schema: String,
    pub engine_version: String,
    pub requested_base: String,
    pub requested_head: String,
    pub base_commit: String,
    pub head_commit: String,
    pub comparison: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<ReviewCheckpoint>,
    pub summary: ReviewSummary,
    pub files: Vec<ReviewFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewDelta {
    pub comparison: String,
    pub from_commit: String,
    pub source_base_commit: String,
    pub to_commit: String,
    pub summary: ReviewSummary,
    pub files: Vec<ReviewFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewFileSources {
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GitChange {
    status: FileStatus,
    similarity_percent: Option<u8>,
    before_path: Option<GitPath>,
    after_path: Option<GitPath>,
    before_mode: Option<String>,
    after_mode: Option<String>,
    before_blob: Option<String>,
    after_blob: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GitPath {
    display: String,
    encoding: PathEncoding,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GitChangeIdentity {
    status: FileStatus,
    similarity_percent: Option<u8>,
    before_path: Option<GitPath>,
    after_path: Option<GitPath>,
    before_mode: Option<String>,
    after_mode: Option<String>,
    before_blob: Option<String>,
    after_blob: Option<String>,
}

impl From<&GitChange> for GitChangeIdentity {
    fn from(change: &GitChange) -> Self {
        Self {
            status: change.status,
            similarity_percent: change.similarity_percent,
            before_path: change.before_path.clone(),
            after_path: change.after_path.clone(),
            before_mode: change.before_mode.clone(),
            after_mode: change.after_mode.clone(),
            before_blob: change.before_blob.clone(),
            after_blob: change.after_blob.clone(),
        }
    }
}

struct Blob {
    size: Option<usize>,
    unavailable_reason: Option<String>,
}

#[derive(Default)]
struct BlobLoader {
    sizes: HashMap<String, usize>,
    bytes: HashMap<String, Vec<u8>>,
    errors: HashMap<String, String>,
    structural_oids: HashSet<String>,
    structural_source_bytes: usize,
    line_stat_source_bytes: usize,
    line_changes: HashMap<(Option<String>, Option<String>), Option<LineChangeEnvelope>>,
}

enum StructuralBlobPair {
    Available { before: Vec<u8>, after: Vec<u8> },
    Unavailable(String),
}

enum BlobLoad {
    Available,
    Unavailable(String),
}

pub fn review_git_range(repository: &Path, base: &str, head: &str) -> Result<RepositoryReview> {
    review_git_range_with_checkpoint(repository, base, head, None)
}

pub fn review_git_range_with_checkpoint(
    repository: &Path,
    base: &str,
    head: &str,
    checkpoint: Option<&str>,
) -> Result<RepositoryReview> {
    let shallow = git_text(repository, &["rev-parse", "--is-shallow-repository"])?;
    ensure!(
        trim_line_ending(&shallow) == "false",
        "repository review requires complete ancestry; shallow repositories are not supported"
    );
    let requested_base_commit = resolve_commit(repository, base)?;
    let head_commit = resolve_commit(repository, head)?;
    let base_commit = unique_merge_base(
        repository,
        &requested_base_commit,
        &head_commit,
        "Git comparison",
    )?;

    let checkpoint = checkpoint
        .map(|requested_revision| -> Result<ReviewCheckpoint> {
            let commit = resolve_commit(repository, requested_revision)?;
            let checkpoint_base = unique_merge_base(
                repository,
                &requested_base_commit,
                &commit,
                "checkpoint comparison",
            )?;
            let match_basis = if checkpoint_base == base_commit {
                CheckpointMatchBasis::ExactGitChangeIdentity
            } else {
                CheckpointMatchBasis::ExactGitChangeIdentityOrNoninteractingFourWayByteReplay
            };
            Ok(ReviewCheckpoint {
                requested_revision: requested_revision.to_owned(),
                commit,
                base_commit: checkpoint_base,
                match_basis,
            })
        })
        .transpose()?;
    let checkpoint_changes = checkpoint
        .as_ref()
        .map(|checkpoint| {
            discover_git_changes(repository, &checkpoint.base_commit, &checkpoint.commit)
        })
        .transpose()?;
    let changes = discover_git_changes(repository, &base_commit, &head_commit)?;
    let base_changed = checkpoint
        .as_ref()
        .is_some_and(|checkpoint| checkpoint.base_commit != base_commit);

    let limits = VerificationLimits::default();
    let mut files = Vec::with_capacity(changes.len());
    let mut blob_loader = BlobLoader::default();
    let mut matched_checkpoint_indices = HashSet::new();
    for change in changes {
        let mut carried_by_replay = false;
        let mut checkpoint_match_basis = None;
        let checkpoint_state = checkpoint_changes.as_ref().map(|checkpoint_changes| {
            let identity = GitChangeIdentity::from(&change);
            let mut exact_match = false;
            for (index, checkpoint_change) in checkpoint_changes.iter().enumerate() {
                if GitChangeIdentity::from(checkpoint_change) == identity {
                    matched_checkpoint_indices.insert(index);
                    exact_match = true;
                }
            }
            if exact_match {
                checkpoint_match_basis = Some(CheckpointCarryBasis::ExactGitChangeIdentity);
                return CheckpointState::UnchangedSinceCheckpoint;
            }

            let replay_match = base_changed
                && unique_replay_candidate(checkpoint_changes, &change).is_some_and(
                    |(index, checkpoint_change)| {
                        if matched_checkpoint_indices.contains(&index) {
                            return false;
                        }
                        match independent_four_way_replay_matches(
                            repository,
                            checkpoint_change,
                            &change,
                            &limits,
                            &mut blob_loader,
                        ) {
                            Ok(true) => {
                                matched_checkpoint_indices.insert(index);
                                true
                            }
                            Ok(false) | Err(_) => false,
                        }
                    },
                );
            if replay_match {
                carried_by_replay = true;
                checkpoint_match_basis =
                    Some(CheckpointCarryBasis::ExactNoninteractingFourWayByteReplay);
                CheckpointState::UnchangedSinceCheckpoint
            } else {
                CheckpointState::NeedsReviewNow
            }
        });
        let mut file = analyze_change(repository, change, &limits, &mut blob_loader)?;
        file.checkpoint_state = checkpoint_state;
        file.checkpoint_match_basis = checkpoint_match_basis;
        if carried_by_replay {
            file.reason.push_str(
                "; checkpoint carry-forward was proven by exact non-interacting four-way byte replay across the base change",
            );
        }
        files.push(file);
    }
    let retired_change_count = checkpoint_changes
        .as_ref()
        .map(|changes| changes.len() - matched_checkpoint_indices.len());
    files.sort_by_key(|file| {
        (
            checkpoint_state_rank(file.checkpoint_state),
            priority_rank(file.priority),
            lane_rank(file.lane),
            file.display_path(),
        )
    });
    let summary = summarize(&files, retired_change_count);

    Ok(RepositoryReview {
        schema: REVIEW_SCHEMA.to_owned(),
        engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        requested_base: base.to_owned(),
        requested_head: head.to_owned(),
        base_commit,
        head_commit,
        comparison: "merge_base_to_head".to_owned(),
        checkpoint,
        summary,
        files,
    })
}

pub fn review_git_snapshot_delta(repository: &Path, from: &str, to: &str) -> Result<ReviewDelta> {
    let from_commit = resolve_commit(repository, from)?;
    let to_commit = resolve_commit(repository, to)?;
    let changes = discover_git_changes(repository, &from_commit, &to_commit)?;
    let limits = VerificationLimits::default();
    let mut blob_loader = BlobLoader::default();
    let mut files = Vec::with_capacity(changes.len());
    for change in changes {
        files.push(analyze_change(
            repository,
            change,
            &limits,
            &mut blob_loader,
        )?);
    }
    files.sort_by_key(|file| {
        (
            priority_rank(file.priority),
            lane_rank(file.lane),
            file.display_path(),
        )
    });
    let summary = summarize(&files, None);
    Ok(ReviewDelta {
        comparison: "snapshot_to_snapshot".to_owned(),
        source_base_commit: from_commit.clone(),
        from_commit,
        to_commit,
        summary,
        files,
    })
}

pub fn review_git_resume_delta(
    repository: &Path,
    review: &RepositoryReview,
) -> Result<ReviewDelta> {
    let checkpoint = review
        .checkpoint
        .as_ref()
        .context("review residue requires a checkpoint")?;
    if checkpoint.base_commit == review.base_commit {
        return review_git_snapshot_delta(repository, &checkpoint.commit, &review.head_commit);
    }

    let files = review
        .files
        .iter()
        .filter(|file| file.checkpoint_state == Some(CheckpointState::NeedsReviewNow))
        .cloned()
        .collect::<Vec<_>>();
    let summary = summarize(&files, None);
    Ok(ReviewDelta {
        comparison: "current_pr_unmatched_identities".to_owned(),
        from_commit: checkpoint.commit.clone(),
        source_base_commit: review.base_commit.clone(),
        to_commit: review.head_commit.clone(),
        summary,
        files,
    })
}

pub fn load_review_file_sources(repository: &Path, file: &ReviewFile) -> Result<ReviewFileSources> {
    let limits = VerificationLimits::default();
    Ok(ReviewFileSources {
        before: load_review_source(
            repository,
            file.before_blob.as_deref(),
            file.before_mode.as_deref(),
            file.before_bytes,
            limits.max_source_bytes,
        )?,
        after: load_review_source(
            repository,
            file.after_blob.as_deref(),
            file.after_mode.as_deref(),
            file.after_bytes,
            limits.max_source_bytes,
        )?,
    })
}

pub fn regenerate_review_file_report(
    file: &ReviewFile,
    sources: &ReviewFileSources,
) -> Result<DiffReport> {
    let evidence = file
        .evidence
        .as_ref()
        .context("selected file has no structural evidence report")?;
    let language = file
        .language
        .context("selected file has no structural analysis language")?;
    let before_path = file
        .before_path
        .clone()
        .context("selected evidence file has no before path")?;
    let after_path = file
        .after_path
        .clone()
        .context("selected evidence file has no after path")?;
    let limits = VerificationLimits::default();
    let report = analyze_bytes_with_limits(
        sources.before.clone(),
        sources.after.clone(),
        before_path,
        after_path,
        language,
        &limits,
    )?;
    let encoded = serde_json::to_vec(&report).context("failed to encode regenerated evidence")?;
    ensure!(
        blake3::hash(&encoded).to_hex().as_str() == evidence.report_blake3,
        "regenerated evidence digest does not match the repository review report"
    );
    Ok(report)
}

fn unique_merge_base(
    repository: &Path,
    left: &str,
    right: &str,
    comparison: &str,
) -> Result<String> {
    let output = git_text(repository, &["merge-base", "--all", left, right])?;
    let merge_bases: Vec<_> = output.lines().collect();
    ensure!(
        merge_bases.len() == 1,
        "{comparison} requires exactly one merge base, found {}",
        merge_bases.len()
    );
    let merge_base = merge_bases[0].to_owned();
    ensure!(
        is_object_id(&merge_base),
        "git merge-base returned an invalid object id"
    );
    Ok(merge_base)
}

fn discover_git_changes(
    repository: &Path,
    base_commit: &str,
    head_commit: &str,
) -> Result<Vec<GitChange>> {
    let diff = git_output_bounded(
        repository,
        &[
            "diff",
            "--raw",
            "-z",
            "--no-abbrev",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--ignore-submodules=none",
            base_commit,
            head_commit,
            "--",
        ],
        16 * 1024 * 1024,
    )?;
    ensure!(
        allowed_raw_diff_diagnostics(&diff.stderr),
        "git diff produced diagnostics: {}",
        String::from_utf8_lossy(&diff.stderr).trim()
    );
    let changes = parse_raw_diff(&diff.stdout, MAX_REVIEW_FILES * 2)?;
    let mut changes = pair_unique_exact_relocations(changes);
    ensure!(
        changes.len() <= MAX_REVIEW_FILES,
        "changed file limit exceeded: observed {}, limit {MAX_REVIEW_FILES}",
        changes.len()
    );
    changes.sort_by_key(git_change_sort_key);
    Ok(changes)
}

fn unique_replay_candidate<'a>(
    checkpoint_changes: &'a [GitChange],
    current: &GitChange,
) -> Option<(usize, &'a GitChange)> {
    let mut candidates = checkpoint_changes
        .iter()
        .enumerate()
        .filter(|(_, checkpoint)| replay_candidate_metadata_matches(checkpoint, current));
    let candidate = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    Some(candidate)
}

fn replay_candidate_metadata_matches(checkpoint: &GitChange, current: &GitChange) -> bool {
    if checkpoint.status != FileStatus::Modified || current.status != FileStatus::Modified {
        return false;
    }
    let Some(path) = checkpoint.before_path.as_ref() else {
        return false;
    };
    if checkpoint.after_path.as_ref() != Some(path)
        || current.before_path.as_ref() != Some(path)
        || current.after_path.as_ref() != Some(path)
    {
        return false;
    }
    let Some(mode) = checkpoint.before_mode.as_deref() else {
        return false;
    };
    matches!(mode, "100644" | "100755")
        && checkpoint.after_mode.as_deref() == Some(mode)
        && current.before_mode.as_deref() == Some(mode)
        && current.after_mode.as_deref() == Some(mode)
}

fn independent_four_way_replay_matches(
    repository: &Path,
    checkpoint: &GitChange,
    current: &GitChange,
    limits: &VerificationLimits,
    blob_loader: &mut BlobLoader,
) -> Result<bool> {
    let Some((checkpoint_before, checkpoint_after)) =
        load_replay_blob_pair(repository, checkpoint, limits, blob_loader)?
    else {
        return Ok(false);
    };
    let Some((current_before, current_after)) =
        load_replay_blob_pair(repository, current, limits, blob_loader)?
    else {
        return Ok(false);
    };
    if [
        checkpoint_before.as_slice(),
        checkpoint_after.as_slice(),
        current_before.as_slice(),
        current_after.as_slice(),
    ]
    .iter()
    .any(|bytes| bytes.contains(&0))
    {
        return Ok(false);
    }

    let reviewed_patch = create_patch(&checkpoint_before, &checkpoint_after);
    let upstream_patch = create_patch(&checkpoint_before, &current_before);
    if patches_interact(&reviewed_patch, &upstream_patch) {
        return Ok(false);
    }
    let Some(reviewed_on_current) = translate_patch(&reviewed_patch, &upstream_patch) else {
        return Ok(false);
    };
    let Some(upstream_on_reviewed) = translate_patch(&upstream_patch, &reviewed_patch) else {
        return Ok(false);
    };

    let reviewed_result = apply_patch(&current_before, &reviewed_on_current)?;
    if reviewed_result != current_after {
        return Ok(false);
    }
    let upstream_result = apply_patch(&checkpoint_after, &upstream_on_reviewed)?;
    Ok(upstream_result == current_after)
}

fn load_replay_blob_pair(
    repository: &Path,
    change: &GitChange,
    limits: &VerificationLimits,
    blob_loader: &mut BlobLoader,
) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    let before_id = change
        .before_blob
        .as_deref()
        .context("replay candidate is missing its before blob")?;
    let after_id = change
        .after_blob
        .as_deref()
        .context("replay candidate is missing its after blob")?;
    let before = inspect_optional_blob(
        repository,
        Some(before_id),
        change.before_mode.as_deref(),
        limits,
        blob_loader,
    )?
    .context("replay candidate is missing before blob metadata")?;
    let after = inspect_optional_blob(
        repository,
        Some(after_id),
        change.after_mode.as_deref(),
        limits,
        blob_loader,
    )?
    .context("replay candidate is missing after blob metadata")?;
    if before.unavailable_reason.is_some() || after.unavailable_reason.is_some() {
        return Ok(None);
    }
    let before_size = before
        .size
        .context("replay before blob size is unavailable")?;
    let after_size = after
        .size
        .context("replay after blob size is unavailable")?;
    match load_structural_blob_pair(
        repository,
        before_id,
        before_size,
        after_id,
        after_size,
        blob_loader,
    )? {
        StructuralBlobPair::Available { before, after } => Ok(Some((before, after))),
        StructuralBlobPair::Unavailable(_) => Ok(None),
    }
}

fn patches_interact(left: &LosslessPatch, right: &LosslessPatch) -> bool {
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left.edits.len() && right_index < right.edits.len() {
        let left_edit = &left.edits[left_index];
        let right_edit = &right.edits[right_index];
        if left_edit.old_end < right_edit.old_start {
            left_index += 1;
        } else if right_edit.old_end < left_edit.old_start {
            right_index += 1;
        } else {
            return true;
        }
    }
    false
}

fn translate_patch(patch: &LosslessPatch, preceding: &LosslessPatch) -> Option<LosslessPatch> {
    let mut preceding_index = 0;
    let mut offset_delta = 0_i128;
    let mut edits = Vec::with_capacity(patch.edits.len());
    for edit in &patch.edits {
        while preceding_index < preceding.edits.len()
            && preceding.edits[preceding_index].old_end < edit.old_start
        {
            offset_delta =
                offset_delta.checked_add(edit_byte_delta(&preceding.edits[preceding_index])?)?;
            preceding_index += 1;
        }
        edits.push(ByteEdit {
            old_start: translate_offset(edit.old_start, offset_delta)?,
            old_end: translate_offset(edit.old_end, offset_delta)?,
            replacement_base64: edit.replacement_base64.clone(),
        });
    }
    Some(LosslessPatch {
        algorithm: patch.algorithm.clone(),
        edits,
    })
}

fn edit_byte_delta(edit: &ByteEdit) -> Option<i128> {
    let removed = edit.old_end.checked_sub(edit.old_start)?;
    let replacement = STANDARD.decode(&edit.replacement_base64).ok()?;
    i128::try_from(replacement.len())
        .ok()?
        .checked_sub(i128::try_from(removed).ok()?)
}

fn translate_offset(offset: usize, delta: i128) -> Option<usize> {
    let offset = i128::try_from(offset).ok()?;
    usize::try_from(offset.checked_add(delta)?).ok()
}

pub fn classify_report(report: &DiffReport) -> ReviewLane {
    let cst_preserved = report
        .changes
        .iter()
        .all(|change| change.kind == ChangeKind::FormattingOnly)
        && report
            .changes
            .iter()
            .any(|change| change.kind == ChangeKind::FormattingOnly);
    if cst_preserved && report.ambiguities.is_empty() && report.summary.suggested_relations == 0 {
        ReviewLane::SyntaxPreserved
    } else {
        ReviewLane::ReviewFirst
    }
}

pub fn classify_priority(_report: &DiffReport) -> ReviewPriority {
    // Parser/CST equality is factual evidence, not evidence of unchanged behavior. Source-reflecting
    // constructs such as Rust stringify! and Python debug f-strings make that distinction observable.
    ReviewPriority::ReviewFirst
}

pub fn markdown_report(review: &RepositoryReview) -> String {
    let mut output = String::new();
    output.push_str("# StrataDiff review focus\n\n");
    output.push_str(
        "> Evidence-based triage, not a claim of semantic equivalence or approval. Unverified files stay in the human-review lane.\n\n",
    );
    output.push_str(&format!(
        "- Range: {} → {} (merge base)\n",
        markdown_code(&review.base_commit),
        markdown_code(&review.head_commit)
    ));
    if let Some(checkpoint) = &review.checkpoint {
        let checkpoint_summary = review
            .summary
            .checkpoint
            .as_ref()
            .expect("checkpoint metadata has a checkpoint summary");
        let exact_identity_carries = review
            .files
            .iter()
            .filter(|file| {
                file.checkpoint_match_basis == Some(CheckpointCarryBasis::ExactGitChangeIdentity)
            })
            .count();
        let four_way_replay_carries = review
            .files
            .iter()
            .filter(|file| {
                file.checkpoint_match_basis
                    == Some(CheckpointCarryBasis::ExactNoninteractingFourWayByteReplay)
            })
            .count();
        if checkpoint.base_commit == review.base_commit {
            output.push_str(&format!(
                "- Checkpoint: {} (same merge base; exact Git change identity only)\n",
                markdown_code(&checkpoint.commit)
            ));
        } else {
            output.push_str(&format!(
                "- Checkpoint: {} (base changed {} → {}; exact Git identities are compared first, then unique same-path modifications may carry only through exact non-interacting four-way byte replay)\n",
                markdown_code(&checkpoint.commit),
                markdown_code(&checkpoint.base_commit),
                markdown_code(&review.base_commit)
            ));
        }
        output.push_str(&format!(
            "- Review coverage: **{}** of {} current {} need review; **{}** carried (**{}** exact-identity, **{}** four-way); **{}** checkpoint changes retired\n",
            checkpoint_summary.needs_review_now_files,
            review.summary.changed_files,
            file_word(review.summary.changed_files),
            checkpoint_summary.unchanged_since_checkpoint_files,
            exact_identity_carries,
            four_way_replay_carries,
            checkpoint_summary.retired_change_count
        ));
        output.push_str(
            "- Checkpoint matching does not establish semantic safety or account for cross-file effects\n",
        );
    }
    if review.checkpoint.is_some() {
        output.push_str(&format!(
            "- Intrinsic priority before checkpoint carry-forward: **{}** of {} files are review first\n",
            review.summary.first_pass_files, review.summary.changed_files
        ));
    } else {
        output.push_str(&format!(
            "- Review first: **{}** of {} files (conservative alpha policy)\n",
            review.summary.first_pass_files, review.summary.changed_files
        ));
    }
    output.push_str(&format!(
        "- Parser model matched (non-semantic): **{}** {}; same Git object: **{}** {}\n",
        review.summary.syntax_preserved_files,
        file_word(review.summary.syntax_preserved_files),
        review.summary.content_preserved_files,
        file_word(review.summary.content_preserved_files)
    ));
    output.push_str(&format!(
        "- Per-file diff reconstruction: passed during analysis for **{}** paired {}; not run/not applicable for **{}** {}\n",
        review.summary.replay_check_passed_files,
        file_word(review.summary.replay_check_passed_files),
        review.summary.replay_check_not_run_files,
        file_word(review.summary.replay_check_not_run_files)
    ));
    match (
        review.summary.changed_line_envelope,
        review.summary.first_pass_line_envelope,
    ) {
        (Some(changed), Some(priority)) if review.checkpoint.is_some() => {
            output.push_str(&format!(
                "- Intrinsic review-first line envelope before checkpoint carry-forward: **{priority} / {changed}**\n"
            ));
        }
        (Some(changed), Some(priority)) => output.push_str(&format!(
            "- Conservative changed-line envelope: **{priority} / {changed}** lines remain first pass (no automatic deprioritization)\n"
        )),
        _ => output.push_str(
            "- Conservative changed-line envelope: incomplete because at least one file is non-text or exceeded the line-stat budget\n",
        ),
    }

    if review.checkpoint.is_some() {
        let needs_review: Vec<_> = review
            .files
            .iter()
            .filter(|file| file.checkpoint_state == Some(CheckpointState::NeedsReviewNow))
            .collect();
        let unchanged: Vec<_> = review
            .files
            .iter()
            .filter(|file| file.checkpoint_state == Some(CheckpointState::UnchangedSinceCheckpoint))
            .collect();

        output.push_str("\n## Needs review now\n\n");
        let shown_needs = append_markdown_file_table(&mut output, &needs_review, true, 2_048);
        append_markdown_omission(&mut output, needs_review.len() - shown_needs);

        let details = if review
            .checkpoint
            .as_ref()
            .is_some_and(|checkpoint| checkpoint.base_commit == review.base_commit)
        {
            format!(
                "\n<details>\n<summary>Unchanged since checkpoint: <strong>{}</strong> exact change identities</summary>\n\n> These entries have the same complete Git change identity as at the checkpoint. Cross-file effects were not checked.\n\n",
                unchanged.len()
            )
        } else {
            let exact_identity_carries = unchanged
                .iter()
                .filter(|file| {
                    file.checkpoint_match_basis
                        == Some(CheckpointCarryBasis::ExactGitChangeIdentity)
                })
                .count();
            let four_way_replay_carries = unchanged.len() - exact_identity_carries;
            format!(
                "\n<details>\n<summary>Carried from checkpoint: <strong>{}</strong> changes ({} exact-identity; {} four-way)</summary>\n\n> Each entry either has the same complete Git change identity or passed exact non-interacting four-way byte replay against the changed base. Cross-file effects were not checked.\n\n",
                unchanged.len(),
                exact_identity_carries,
                four_way_replay_carries
            )
        };
        if output.len() + details.len() + 32 <= MAX_REVIEW_MARKDOWN_BYTES {
            output.push_str(&details);
            let shown_unchanged = append_markdown_file_table(&mut output, &unchanged, true, 256);
            append_markdown_omission(&mut output, unchanged.len() - shown_unchanged);
            output.push_str("\n</details>\n");
        }
    } else {
        let files: Vec<_> = review.files.iter().collect();
        let shown_files = append_markdown_file_table(&mut output, &files, false, 256);
        append_markdown_omission(&mut output, review.files.len() - shown_files);
    }
    debug_assert!(output.len() <= MAX_REVIEW_MARKDOWN_BYTES);
    output
}

fn append_markdown_file_table(
    output: &mut String,
    files: &[&ReviewFile],
    include_checkpoint_state: bool,
    reserved_bytes: usize,
) -> usize {
    let header = if include_checkpoint_state {
        "| Checkpoint | Priority | Evidence class | File | Git change | Lines | Why |\n|---|---|---|---|---:|---:|---|\n"
    } else {
        "\n| Priority | Evidence class | File | Git change | Lines | Why |\n|---|---|---|---:|---:|---|\n"
    };
    if output.len() + header.len() + reserved_bytes > MAX_REVIEW_MARKDOWN_BYTES {
        return 0;
    }
    output.push_str(header);

    let mut shown = 0;
    for file in files {
        let lines = file.line_change_envelope.as_ref().map_or_else(
            || "unknown".to_owned(),
            |lines| format!("+{} / -{}", lines.additions, lines.deletions),
        );
        let git_change = match file.similarity_percent {
            Some(similarity) => format!("{:?} {similarity}%", file.status),
            None => format!("{:?}", file.status),
        };
        let row = if include_checkpoint_state {
            format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                file.checkpoint_match_basis.map_or_else(
                    || {
                        file.checkpoint_state
                            .expect("checkpoint report files have checkpoint state")
                            .label()
                    },
                    CheckpointCarryBasis::label,
                ),
                file.priority.label(),
                file.lane.label(),
                markdown_code(&file.display_path()),
                markdown_cell(&format_git_change(file, &git_change)),
                lines,
                markdown_cell(&file.reason)
            )
        } else {
            format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                file.priority.label(),
                file.lane.label(),
                markdown_code(&file.display_path()),
                markdown_cell(&format_git_change(file, &git_change)),
                lines,
                markdown_cell(&file.reason)
            )
        };
        if output.len() + row.len() + reserved_bytes > MAX_REVIEW_MARKDOWN_BYTES {
            break;
        }
        output.push_str(&row);
        shown += 1;
    }
    shown
}

fn append_markdown_omission(output: &mut String, omitted: usize) {
    if omitted == 0 {
        return;
    }
    let note = format!(
        "\n_{omitted} additional files omitted to keep the step summary below {MAX_REVIEW_MARKDOWN_BYTES} bytes. Use the JSON artifact for the complete list._\n"
    );
    if output.len() + note.len() <= MAX_REVIEW_MARKDOWN_BYTES {
        output.push_str(&note);
    }
}

fn resolve_commit(repository: &Path, revision: &str) -> Result<String> {
    let revision = format!("{revision}^{{commit}}");
    let value = git_text(
        repository,
        &["rev-parse", "--verify", "--end-of-options", &revision],
    )?;
    let value = trim_line_ending(&value).to_owned();
    ensure!(
        is_object_id(&value),
        "git rev-parse returned an invalid object id"
    );
    Ok(value)
}

fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn trim_line_ending(value: &str) -> &str {
    value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(value)
}

fn parse_raw_diff(bytes: &[u8], max_files: usize) -> Result<Vec<GitChange>> {
    let mut fields = bytes.split(|byte| *byte == 0).peekable();
    let mut changes = Vec::new();
    while let Some(header) = fields.next() {
        if header.is_empty() {
            ensure!(fields.peek().is_none(), "unexpected empty git diff record");
            break;
        }
        let header = std::str::from_utf8(header).context("git diff header is not UTF-8")?;
        let columns: Vec<_> = header.split_ascii_whitespace().collect();
        ensure!(
            columns.len() == 5,
            "unexpected git raw diff header: {header}"
        );
        let before_mode = nonzero(
            columns[0]
                .strip_prefix(':')
                .context("missing git mode prefix")?,
        );
        let after_mode = nonzero(columns[1]);
        let before_blob = nonzero(columns[2]);
        let after_blob = nonzero(columns[3]);
        let status_text = columns[4];
        let status_code = status_text
            .bytes()
            .next()
            .context("missing git diff status")?;
        let (status, similarity_percent) = match status_code {
            b'A' => (FileStatus::Added, None),
            b'C' => (FileStatus::Copied, Some(parse_similarity(status_text)?)),
            b'D' => (FileStatus::Deleted, None),
            b'M' => (FileStatus::Modified, None),
            b'R' => (FileStatus::Renamed, Some(parse_similarity(status_text)?)),
            b'T' => (FileStatus::TypeChanged, None),
            _ => bail!("unsupported git diff status: {status_text}"),
        };
        let first_path = next_path(&mut fields)?;
        let (before_path, after_path) = match status {
            FileStatus::Added => (None, Some(first_path)),
            FileStatus::Deleted => (Some(first_path), None),
            FileStatus::Copied | FileStatus::Renamed => {
                (Some(first_path), Some(next_path(&mut fields)?))
            }
            FileStatus::Modified | FileStatus::TypeChanged => {
                (Some(first_path.clone()), Some(first_path))
            }
        };
        match status {
            FileStatus::Added => ensure!(
                before_mode.is_none()
                    && before_blob.is_none()
                    && after_mode.is_some()
                    && after_blob.is_some(),
                "added git diff record has inconsistent object metadata"
            ),
            FileStatus::Deleted => ensure!(
                before_mode.is_some()
                    && before_blob.is_some()
                    && after_mode.is_none()
                    && after_blob.is_none(),
                "deleted git diff record has inconsistent object metadata"
            ),
            FileStatus::Copied
            | FileStatus::Modified
            | FileStatus::Renamed
            | FileStatus::TypeChanged => ensure!(
                before_mode.is_some()
                    && before_blob.is_some()
                    && after_mode.is_some()
                    && after_blob.is_some(),
                "paired git diff record has inconsistent object metadata"
            ),
        }
        changes.push(GitChange {
            status,
            similarity_percent,
            before_path,
            after_path,
            before_mode,
            after_mode,
            before_blob,
            after_blob,
        });
        if changes.len() > max_files {
            bail!(
                "changed file limit exceeded: observed at least {}, limit {max_files}",
                changes.len()
            );
        }
    }
    Ok(changes)
}

fn pair_unique_exact_relocations(changes: Vec<GitChange>) -> Vec<GitChange> {
    let mut candidates = HashMap::<(String, String), (Vec<usize>, Vec<usize>)>::new();
    for (index, change) in changes.iter().enumerate() {
        match change.status {
            FileStatus::Deleted => {
                if let (Some(object_id), Some(mode)) = (&change.before_blob, &change.before_mode) {
                    candidates
                        .entry((object_id.clone(), mode.clone()))
                        .or_default()
                        .0
                        .push(index);
                }
            }
            FileStatus::Added => {
                if let (Some(object_id), Some(mode)) = (&change.after_blob, &change.after_mode) {
                    candidates
                        .entry((object_id.clone(), mode.clone()))
                        .or_default()
                        .1
                        .push(index);
                }
            }
            FileStatus::Copied
            | FileStatus::Modified
            | FileStatus::Renamed
            | FileStatus::TypeChanged => {}
        }
    }
    let pairs: Vec<_> = candidates
        .into_values()
        .filter_map(|(deleted, added)| {
            (deleted.len() == 1 && added.len() == 1).then(|| (deleted[0], added[0]))
        })
        .collect();
    let mut changes: Vec<_> = changes.into_iter().map(Some).collect();
    for (deleted_index, added_index) in pairs {
        let deleted = changes[deleted_index]
            .take()
            .expect("unique deleted relocation candidate is present");
        let added = changes[added_index]
            .take()
            .expect("unique added relocation candidate is present");
        let relocation = GitChange {
            status: FileStatus::Renamed,
            similarity_percent: Some(100),
            before_path: deleted.before_path,
            after_path: added.after_path,
            before_mode: deleted.before_mode,
            after_mode: added.after_mode,
            before_blob: deleted.before_blob,
            after_blob: added.after_blob,
        };
        changes[deleted_index.min(added_index)] = Some(relocation);
    }
    changes.into_iter().flatten().collect()
}

fn git_change_sort_key(change: &GitChange) -> (String, String, u8, String, String) {
    (
        change
            .before_path
            .as_ref()
            .map_or_else(String::new, |path| path.display.clone()),
        change
            .after_path
            .as_ref()
            .map_or_else(String::new, |path| path.display.clone()),
        file_status_rank(change.status),
        change.before_blob.clone().unwrap_or_default(),
        change.after_blob.clone().unwrap_or_default(),
    )
}

fn file_status_rank(status: FileStatus) -> u8 {
    match status {
        FileStatus::Added => 0,
        FileStatus::Copied => 1,
        FileStatus::Deleted => 2,
        FileStatus::Modified => 3,
        FileStatus::Renamed => 4,
        FileStatus::TypeChanged => 5,
    }
}

fn nonzero(value: &str) -> Option<String> {
    if value.bytes().all(|byte| byte == b'0') {
        None
    } else {
        Some(value.to_owned())
    }
}

fn parse_similarity(status: &str) -> Result<u8> {
    let value = status
        .get(1..)
        .context("missing rename or copy similarity")?
        .parse::<u8>()
        .context("invalid rename or copy similarity")?;
    ensure!(value <= 100, "git similarity exceeds 100%: {value}");
    Ok(value)
}

fn next_path<'a>(fields: &mut impl Iterator<Item = &'a [u8]>) -> Result<GitPath> {
    let bytes = fields.next().context("git diff record is missing a path")?;
    ensure!(!bytes.is_empty(), "git diff path is empty");
    Ok(match std::str::from_utf8(bytes) {
        Ok(path) => GitPath {
            display: path.to_owned(),
            encoding: PathEncoding::Utf8,
        },
        Err(_) => GitPath {
            display: percent_encode_git_path(bytes),
            encoding: PathEncoding::GitBytesPercentEncoded,
        },
    })
}

fn analyze_change(
    repository: &Path,
    change: GitChange,
    limits: &VerificationLimits,
    blob_loader: &mut BlobLoader,
) -> Result<ReviewFile> {
    let before = inspect_optional_blob(
        repository,
        change.before_blob.as_deref(),
        change.before_mode.as_deref(),
        limits,
        blob_loader,
    )?;
    let after = inspect_optional_blob(
        repository,
        change.after_blob.as_deref(),
        change.after_mode.as_deref(),
        limits,
        blob_loader,
    )?;
    let line_change_envelope = load_line_changes(
        repository,
        change.status,
        change.before_blob.as_deref(),
        before.as_ref(),
        change.after_blob.as_deref(),
        after.as_ref(),
        blob_loader,
    )?;

    let mut file = ReviewFile {
        status: change.status,
        similarity_percent: change.similarity_percent,
        before_path: change.before_path.as_ref().map(|path| path.display.clone()),
        before_path_encoding: change.before_path.as_ref().map(|path| path.encoding),
        after_path: change.after_path.as_ref().map(|path| path.display.clone()),
        after_path_encoding: change.after_path.as_ref().map(|path| path.encoding),
        before_mode: change.before_mode,
        after_mode: change.after_mode,
        before_blob: change.before_blob,
        after_blob: change.after_blob,
        before_bytes: before.as_ref().and_then(|blob| blob.size),
        after_bytes: after.as_ref().and_then(|blob| blob.size),
        line_change_envelope,
        language: None,
        priority: ReviewPriority::ReviewFirst,
        lane: ReviewLane::ReviewFirst,
        checkpoint_state: None,
        checkpoint_match_basis: None,
        reason: String::new(),
        evidence: None,
    };

    if let Some(reason) = file
        .before_blob
        .iter()
        .chain(file.after_blob.iter())
        .find_map(|object_id| blob_loader.errors.get(object_id))
    {
        file.lane = ReviewLane::Unverified;
        file.reason = reason.clone();
        return Ok(file);
    }

    if change
        .before_path
        .iter()
        .chain(change.after_path.iter())
        .any(|path| path.encoding != PathEncoding::Utf8)
    {
        file.lane = ReviewLane::Unverified;
        file.reason =
            "Git path is not UTF-8; the reversible git-bytes:%XX label is retained for manual review"
                .to_owned();
        return Ok(file);
    }

    match file.status {
        FileStatus::Added => {
            file.reason = "new file; all content belongs to the review residue".to_owned();
            return Ok(file);
        }
        FileStatus::Deleted => {
            file.reason = "deleted file; removal belongs to the review residue".to_owned();
            return Ok(file);
        }
        FileStatus::Modified
        | FileStatus::Renamed
        | FileStatus::Copied
        | FileStatus::TypeChanged => {}
    }

    if file.before_blob == file.after_blob {
        file.lane = ReviewLane::ContentPreserved;
        file.reason = if file.before_mode.as_deref() == Some("160000") {
            "Git reports the same gitlink target commit; path and repository effects remain in the first pass"
                .to_owned()
        } else {
            "Git reports the same object ID; path, copy, type, and file-mode effects remain in the first pass"
                .to_owned()
        };
        return Ok(file);
    }

    let before = before.context("paired git change is missing its before blob")?;
    let after = after.context("paired git change is missing its after blob")?;
    if let Some(reason) = &before.unavailable_reason {
        file.lane = ReviewLane::Unverified;
        file.reason = reason.clone();
        return Ok(file);
    }
    if let Some(reason) = &after.unavailable_reason {
        file.lane = ReviewLane::Unverified;
        file.reason = reason.clone();
        return Ok(file);
    }
    let before_path = Path::new(
        file.before_path
            .as_deref()
            .context("paired git change is missing its before path")?,
    );
    let after_path = Path::new(
        file.after_path
            .as_deref()
            .context("paired git change is missing its after path")?,
    );
    let before_language = match Language::detect(before_path) {
        Ok(language) => language,
        Err(error) => {
            file.lane = ReviewLane::Unverified;
            file.reason = format!("before path has no supported parser: {error}");
            return Ok(file);
        }
    };
    let after_language = match Language::detect(after_path) {
        Ok(language) => language,
        Err(error) => {
            file.lane = ReviewLane::Unverified;
            file.reason = format!("after path has no supported parser: {error}");
            return Ok(file);
        }
    };
    if before_language != after_language {
        file.lane = ReviewLane::Unverified;
        file.reason =
            format!("file language changed from {before_language:?} to {after_language:?}");
        return Ok(file);
    }
    file.language = Some(before_language);

    let (before_bytes, after_bytes) = match load_structural_blob_pair(
        repository,
        file.before_blob
            .as_deref()
            .context("paired structural change is missing its before blob")?,
        before.size.context("before blob size is unavailable")?,
        file.after_blob
            .as_deref()
            .context("paired structural change is missing its after blob")?,
        after.size.context("after blob size is unavailable")?,
        blob_loader,
    )? {
        StructuralBlobPair::Available { before, after } => (before, after),
        StructuralBlobPair::Unavailable(reason) => {
            file.lane = ReviewLane::Unverified;
            file.reason = reason;
            return Ok(file);
        }
    };

    let report = match analyze_bytes_with_limits(
        before_bytes,
        after_bytes,
        file.before_path.clone().context("missing before path")?,
        file.after_path.clone().context("missing after path")?,
        before_language,
        limits,
    ) {
        Ok(report) => report,
        Err(error) => {
            file.lane = ReviewLane::Unverified;
            file.reason = format!("structural analysis did not complete: {error:#}");
            return Ok(file);
        }
    };
    let encoded = serde_json::to_vec(&report).context("failed to encode per-file evidence")?;
    if encoded.len() > limits.max_report_bytes {
        file.lane = ReviewLane::Unverified;
        file.reason = format!(
            "per-file evidence exceeds the {} byte report limit",
            limits.max_report_bytes
        );
        return Ok(file);
    }
    file.lane = classify_report(&report);
    file.priority = classify_priority(&report);
    let metadata_changed =
        file.before_path != file.after_path || file.before_mode != file.after_mode;
    file.reason = match file.lane {
        ReviewLane::SyntaxPreserved => {
            "Tree-sitter representation matches under StrataDiff's syntax_equal predicate; bytes differ, behavior was not checked, and the file remains in the first pass".to_owned()
        }
        ReviewLane::ReviewFirst if metadata_changed => {
            "path or file mode changed; byte/CST evidence does not prove metadata effects"
                .to_owned()
        }
        ReviewLane::ReviewFirst if !report.ambiguities.is_empty() => {
            "CST syntax is unchanged, but correspondence ambiguity remains in review first"
                .to_owned()
        }
        ReviewLane::ReviewFirst => {
            "the single-file diff patch rebuilt the target bytes exactly; the parser/model report still contains a structural delta that remains in the first pass".to_owned()
        }
        ReviewLane::ContentPreserved | ReviewLane::Unverified => {
            unreachable!("paired structural analysis produces only review or syntax lanes")
        }
    };
    file.evidence = Some(evidence(&report, &encoded));
    Ok(file)
}

fn load_structural_blob_pair(
    repository: &Path,
    before_id: &str,
    before_size: usize,
    after_id: &str,
    after_size: usize,
    blob_loader: &mut BlobLoader,
) -> Result<StructuralBlobPair> {
    let candidates = [(before_id, before_size), (after_id, after_size)];
    let mut missing = Vec::new();
    for (object_id, size) in candidates {
        if !blob_loader.structural_oids.contains(object_id)
            && !missing
                .iter()
                .any(|(missing_id, _)| *missing_id == object_id)
        {
            missing.push((object_id, size));
        }
    }
    let additional_bytes = missing.iter().try_fold(0_usize, |total, (_, size)| {
        total
            .checked_add(*size)
            .context("aggregate source byte count exceeds usize capacity")
    })?;
    let next_total = blob_loader
        .structural_source_bytes
        .checked_add(additional_bytes)
        .context("aggregate source byte count exceeds usize capacity")?;
    if next_total > MAX_REVIEW_TOTAL_SOURCE_BYTES {
        return Ok(StructuralBlobPair::Unavailable(format!(
            "repository review exceeds the {MAX_REVIEW_TOTAL_SOURCE_BYTES} byte aggregate analysis limit"
        )));
    }

    for (object_id, size) in &missing {
        if let BlobLoad::Unavailable(reason) =
            load_blob_once(repository, object_id, *size, blob_loader)
        {
            return Ok(StructuralBlobPair::Unavailable(reason));
        }
    }
    blob_loader
        .structural_oids
        .extend(missing.iter().map(|(object_id, _)| (*object_id).to_owned()));
    blob_loader.structural_source_bytes = next_total;
    Ok(StructuralBlobPair::Available {
        before: blob_loader
            .bytes
            .get(before_id)
            .context("before structural blob was not cached")?
            .clone(),
        after: blob_loader
            .bytes
            .get(after_id)
            .context("after structural blob was not cached")?
            .clone(),
    })
}

fn load_line_changes(
    repository: &Path,
    status: FileStatus,
    before_id: Option<&str>,
    before: Option<&Blob>,
    after_id: Option<&str>,
    after: Option<&Blob>,
    blob_loader: &mut BlobLoader,
) -> Result<Option<LineChangeEnvelope>> {
    let key = (before_id.map(str::to_owned), after_id.map(str::to_owned));
    if let Some(changes) = blob_loader.line_changes.get(&key) {
        return Ok(changes.clone());
    }
    let sources = match status {
        FileStatus::Added => match available_line_source(after_id, after) {
            Some(source) => vec![source],
            None => return Ok(None),
        },
        FileStatus::Deleted => match available_line_source(before_id, before) {
            Some(source) => vec![source],
            None => return Ok(None),
        },
        FileStatus::Copied
        | FileStatus::Modified
        | FileStatus::Renamed
        | FileStatus::TypeChanged => {
            let Some(before) = available_line_source(before_id, before) else {
                return Ok(None);
            };
            let Some(after) = available_line_source(after_id, after) else {
                return Ok(None);
            };
            vec![before, after]
        }
    };

    let additional_bytes = sources.iter().try_fold(0_usize, |total, (_, size)| {
        total
            .checked_add(*size)
            .context("line-stat source byte count exceeds usize capacity")
    })?;
    let next_total = blob_loader
        .line_stat_source_bytes
        .checked_add(additional_bytes)
        .context("line-stat source byte count exceeds usize capacity")?;
    if next_total > MAX_REVIEW_TOTAL_LINE_STAT_BYTES {
        return cache_line_changes(key, None, blob_loader);
    }

    for (object_id, size) in &sources {
        if let BlobLoad::Unavailable(_) = load_blob_once(repository, object_id, *size, blob_loader)
        {
            return cache_line_changes(key, None, blob_loader);
        }
    }
    blob_loader.line_stat_source_bytes = next_total;
    let changes = match status {
        FileStatus::Added => line_changes(
            &[],
            blob_loader
                .bytes
                .get(after_id.context("added file is missing its blob id")?)
                .context("added line-stat blob was not cached")?,
        ),
        FileStatus::Deleted => line_changes(
            blob_loader
                .bytes
                .get(before_id.context("deleted file is missing its blob id")?)
                .context("deleted line-stat blob was not cached")?,
            &[],
        ),
        FileStatus::Copied
        | FileStatus::Modified
        | FileStatus::Renamed
        | FileStatus::TypeChanged => line_changes(
            blob_loader
                .bytes
                .get(before_id.context("paired file is missing its before blob id")?)
                .context("before line-stat blob was not cached")?,
            blob_loader
                .bytes
                .get(after_id.context("paired file is missing its after blob id")?)
                .context("after line-stat blob was not cached")?,
        ),
    };
    cache_line_changes(key, changes, blob_loader)
}

fn available_line_source<'a>(
    object_id: Option<&'a str>,
    blob: Option<&Blob>,
) -> Option<(&'a str, usize)> {
    let object_id = object_id?;
    let blob = blob?;
    if blob.unavailable_reason.is_some() {
        return None;
    }
    Some((object_id, blob.size?))
}

fn cache_line_changes(
    key: (Option<String>, Option<String>),
    changes: Option<LineChangeEnvelope>,
    blob_loader: &mut BlobLoader,
) -> Result<Option<LineChangeEnvelope>> {
    blob_loader.line_changes.insert(key, changes.clone());
    Ok(changes)
}

fn inspect_optional_blob(
    repository: &Path,
    object_id: Option<&str>,
    mode: Option<&str>,
    limits: &VerificationLimits,
    blob_loader: &mut BlobLoader,
) -> Result<Option<Blob>> {
    let Some(object_id) = object_id else {
        return Ok(None);
    };
    ensure!(
        is_object_id(object_id),
        "git diff returned an invalid object id"
    );
    let mode = mode.context("git object is missing its file mode")?;
    if mode == "160000" {
        return Ok(Some(Blob {
            size: None,
            unavailable_reason: Some(
                "gitlink/submodule target changed and requires manual review".to_owned(),
            ),
        }));
    }
    if !matches!(mode, "100644" | "100755") {
        return Ok(Some(Blob {
            size: None,
            unavailable_reason: Some(format!(
                "Git object mode {mode} is not a regular source file"
            )),
        }));
    }
    if let Some(reason) = blob_loader.errors.get(object_id) {
        return Ok(Some(Blob {
            size: None,
            unavailable_reason: Some(reason.clone()),
        }));
    }
    let size = match blob_loader.sizes.get(object_id) {
        Some(size) => *size,
        None => {
            let size = match git_text(repository, &["cat-file", "-s", object_id]).and_then(|size| {
                trim_line_ending(&size)
                    .parse::<usize>()
                    .context("git blob size is not a non-negative integer")
            }) {
                Ok(size) => size,
                Err(error) => {
                    let reason = format!("Git blob metadata {object_id} is unavailable: {error:#}");
                    blob_loader
                        .errors
                        .insert(object_id.to_owned(), reason.clone());
                    return Ok(Some(Blob {
                        size: None,
                        unavailable_reason: Some(reason),
                    }));
                }
            };
            blob_loader.sizes.insert(object_id.to_owned(), size);
            size
        }
    };
    if size > limits.max_source_bytes {
        return Ok(Some(Blob {
            size: Some(size),
            unavailable_reason: Some(format!(
                "blob exceeds the {} byte per-file analysis limit",
                limits.max_source_bytes
            )),
        }));
    }
    Ok(Some(Blob {
        size: Some(size),
        unavailable_reason: None,
    }))
}

fn load_blob_once(
    repository: &Path,
    object_id: &str,
    size: usize,
    blob_loader: &mut BlobLoader,
) -> BlobLoad {
    if blob_loader.bytes.contains_key(object_id) {
        return BlobLoad::Available;
    }
    if let Some(reason) = blob_loader.errors.get(object_id) {
        return BlobLoad::Unavailable(reason.clone());
    }
    match read_blob(repository, object_id, size) {
        Ok(bytes) => {
            blob_loader.bytes.insert(object_id.to_owned(), bytes);
            BlobLoad::Available
        }
        Err(error) => {
            let reason = format!("Git blob {object_id} is unavailable: {error:#}");
            blob_loader
                .errors
                .insert(object_id.to_owned(), reason.clone());
            BlobLoad::Unavailable(reason)
        }
    }
}

fn read_blob(repository: &Path, object_id: &str, size: usize) -> Result<Vec<u8>> {
    let output = git_output(repository, &["cat-file", "blob", object_id])?;
    ensure!(
        output.stdout.len() == size,
        "git blob size changed while reading {object_id}"
    );
    Ok(output.stdout)
}

fn load_review_source(
    repository: &Path,
    object_id: Option<&str>,
    mode: Option<&str>,
    expected_size: Option<usize>,
    limit: usize,
) -> Result<Vec<u8>> {
    let Some(object_id) = object_id else {
        ensure!(
            mode.is_none(),
            "missing review blob has an unexpected file mode"
        );
        ensure!(
            expected_size.is_none(),
            "missing review blob has an unexpected byte length"
        );
        return Ok(Vec::new());
    };
    ensure!(
        is_object_id(object_id),
        "review file has an invalid object id"
    );
    let mode = mode.context("review blob is missing its file mode")?;
    ensure!(
        mode != "160000",
        "gitlink/submodule targets do not contain displayable file bytes"
    );

    let object_type = git_text(repository, &["cat-file", "-t", object_id])?;
    ensure!(
        trim_line_ending(&object_type) == "blob",
        "review object {object_id} is not a Git blob"
    );
    let observed_size = git_text(repository, &["cat-file", "-s", object_id])?;
    let observed_size = trim_line_ending(&observed_size)
        .parse::<usize>()
        .context("review blob size is not a non-negative integer")?;
    ensure!(
        observed_size <= limit,
        "review source bytes limit exceeded: observed {observed_size}, limit {limit}"
    );
    if let Some(expected_size) = expected_size {
        ensure!(
            observed_size == expected_size,
            "review blob size changed: expected {expected_size}, observed {observed_size}"
        );
    }
    read_blob(repository, object_id, observed_size)
}

fn line_changes(before: &[u8], after: &[u8]) -> Option<LineChangeEnvelope> {
    let before = std::str::from_utf8(before).ok()?;
    let after = std::str::from_utf8(after).ok()?;

    // A common-prefix/suffix envelope is deterministic, linear, and conservative. It may count
    // more changed lines than a minimal diff, but it cannot be driven into quadratic alignment.
    let before_lines = before.split_inclusive('\n').count();
    let after_lines = after.split_inclusive('\n').count();
    let prefix = before
        .split_inclusive('\n')
        .zip(after.split_inclusive('\n'))
        .take_while(|(before, after)| before == after)
        .count();
    let suffix_limit = before_lines.min(after_lines).saturating_sub(prefix);
    let suffix = before
        .split_inclusive('\n')
        .rev()
        .zip(after.split_inclusive('\n').rev())
        .take(suffix_limit)
        .take_while(|(before, after)| before == after)
        .count();
    Some(LineChangeEnvelope {
        additions: after_lines - prefix - suffix,
        deletions: before_lines - prefix - suffix,
    })
}

fn evidence(report: &DiffReport, encoded: &[u8]) -> FileEvidence {
    let mut changes = ChangeCounts::default();
    for change in &report.changes {
        match change.kind {
            ChangeKind::Insert => changes.insertions += 1,
            ChangeKind::Delete => changes.deletions += 1,
            ChangeKind::EquivalentRelocation => changes.equivalent_relocations += 1,
            ChangeKind::ChildOrderChanged => changes.child_order_changes += 1,
            ChangeKind::ModelForcedUpdate => changes.model_forced_updates += 1,
            ChangeKind::SuggestedUpdate => changes.suggested_updates += 1,
            ChangeKind::FormattingOnly => changes.formatting_only += 1,
        }
    }
    FileEvidence {
        report_blake3: blake3::hash(encoded).to_hex().to_string(),
        replay_check_passed_during_analysis: report.certificate.patch_verified,
        model_forced_relations: report.summary.model_forced_relations,
        suggested_relations: report.summary.suggested_relations,
        ambiguity_groups: report.summary.ambiguity_groups,
        byte_edits: report.patch.edits.len(),
        changes,
    }
}

fn summarize(files: &[ReviewFile], retired_change_count: Option<usize>) -> ReviewSummary {
    let first_pass_files = files
        .iter()
        .filter(|file| file.priority == ReviewPriority::ReviewFirst)
        .count();
    let review_first_files = files
        .iter()
        .filter(|file| file.lane == ReviewLane::ReviewFirst)
        .count();
    let syntax_preserved_files = files
        .iter()
        .filter(|file| file.lane == ReviewLane::SyntaxPreserved)
        .count();
    let content_preserved_files = files
        .iter()
        .filter(|file| file.lane == ReviewLane::ContentPreserved)
        .count();
    let unverified_files = files
        .iter()
        .filter(|file| file.lane == ReviewLane::Unverified)
        .count();
    let replay_check_passed_files = files
        .iter()
        .filter(|file| {
            file.evidence
                .as_ref()
                .is_some_and(|evidence| evidence.replay_check_passed_during_analysis)
        })
        .count();
    let replay_check_not_run_files = files.len() - replay_check_passed_files;
    let line_envelope_complete = files.iter().all(|file| file.line_change_envelope.is_some());
    let (changed_line_envelope, first_pass_line_envelope) = if line_envelope_complete {
        let changed = files
            .iter()
            .map(|file| file.line_change_envelope.as_ref().expect("checked").total())
            .sum::<usize>();
        let priority = files
            .iter()
            .filter(|file| file.priority == ReviewPriority::ReviewFirst)
            .map(|file| file.line_change_envelope.as_ref().expect("checked").total())
            .sum::<usize>();
        (Some(changed), Some(priority))
    } else {
        (None, None)
    };
    let checkpoint = retired_change_count.map(|retired_change_count| {
        debug_assert!(files.iter().all(|file| file.checkpoint_state.is_some()));
        CheckpointSummary {
            needs_review_now_files: files
                .iter()
                .filter(|file| file.checkpoint_state == Some(CheckpointState::NeedsReviewNow))
                .count(),
            unchanged_since_checkpoint_files: files
                .iter()
                .filter(|file| {
                    file.checkpoint_state == Some(CheckpointState::UnchangedSinceCheckpoint)
                })
                .count(),
            retired_change_count,
        }
    });

    ReviewSummary {
        changed_files: files.len(),
        first_pass_files,
        review_first_files,
        syntax_preserved_files,
        content_preserved_files,
        unverified_files,
        replay_check_passed_files,
        replay_check_not_run_files,
        line_envelope_complete,
        changed_line_envelope,
        first_pass_line_envelope,
        checkpoint,
    }
}

fn checkpoint_state_rank(state: Option<CheckpointState>) -> u8 {
    match state {
        None | Some(CheckpointState::NeedsReviewNow) => 0,
        Some(CheckpointState::UnchangedSinceCheckpoint) => 1,
    }
}

fn priority_rank(priority: ReviewPriority) -> u8 {
    match priority {
        ReviewPriority::ReviewFirst => 0,
        ReviewPriority::EvidenceFollowUp => 1,
    }
}

fn lane_rank(lane: ReviewLane) -> u8 {
    match lane {
        ReviewLane::Unverified => 0,
        ReviewLane::ReviewFirst => 1,
        ReviewLane::ContentPreserved => 2,
        ReviewLane::SyntaxPreserved => 3,
    }
}

fn percent_encode_git_path(bytes: &[u8]) -> String {
    let mut encoded = String::from("git-bytes:");
    for byte in bytes {
        encoded.push('%');
        encoded.push_str(&format!("{byte:02X}"));
    }
    encoded
}

fn format_git_change(file: &ReviewFile, status: &str) -> String {
    match (&file.before_mode, &file.after_mode) {
        (Some(before), Some(after)) if before != after => {
            format!("{status}; mode {before} -> {after}")
        }
        _ => status.to_owned(),
    }
}

fn file_word(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

fn markdown_cell(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '|' => escaped.push_str("&#124;"),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            character if is_unsafe_text(character) => {
                escaped.extend(character.escape_unicode());
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn markdown_code(value: &str) -> String {
    format!(
        "<code>{}</code>",
        markdown_cell(value).replace('`', "&#96;")
    )
}

fn is_unsafe_text(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{00ad}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061c}'
                | '\u{06dd}'
                | '\u{070f}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08e2}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{110bd}'
                | '\u{110cd}'
                | '\u{13430}'..='\u{1343f}'
                | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0001}'
                | '\u{e0020}'..='\u{e007f}'
        )
}

fn git_text(repository: &Path, arguments: &[&str]) -> Result<String> {
    let output = git_output(repository, arguments)?;
    String::from_utf8(output.stdout).context("git output is not valid UTF-8")
}

fn git_output(repository: &Path, arguments: &[&str]) -> Result<Output> {
    let output = isolated_git_command(repository)
        .args(arguments)
        .output()
        .with_context(|| format!("failed to run git in {}", repository.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git {} failed with {}: {}",
            arguments.join(" "),
            output.status,
            stderr.trim()
        );
    }
    Ok(output)
}

fn git_output_bounded(
    repository: &Path,
    arguments: &[&str],
    max_stdout_bytes: usize,
) -> Result<Output> {
    let mut child = isolated_git_command(repository)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run git in {}", repository.display()))?;
    let stdout = child
        .stdout
        .take()
        .context("failed to capture git stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("failed to capture git stderr")?;
    let stdout_reader = std::thread::spawn(move || read_bounded(stdout, max_stdout_bytes));
    let stderr_reader = std::thread::spawn(move || read_bounded(stderr, 64 * 1024));
    let (stdout, stdout_exceeded) = stdout_reader
        .join()
        .map_err(|_| anyhow::anyhow!("git stdout reader panicked"))??;
    if stdout_exceeded {
        child
            .kill()
            .context("failed to stop git after its output exceeded the limit")?;
    }
    let status = child.wait().context("failed to wait for git")?;
    let (stderr, stderr_exceeded) = stderr_reader
        .join()
        .map_err(|_| anyhow::anyhow!("git stderr reader panicked"))??;
    ensure!(
        !stdout_exceeded,
        "git raw diff exceeds the {max_stdout_bytes} byte metadata limit"
    );
    ensure!(
        !stderr_exceeded,
        "git diagnostics exceed the 65536 byte limit"
    );
    if !status.success() {
        bail!(
            "git {} failed with {}: {}",
            arguments.join(" "),
            status,
            String::from_utf8_lossy(&stderr).trim()
        );
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn read_bounded(mut reader: impl Read, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::with_capacity(limit.min(64 * 1024));
    let mut exceeded = false;
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        let remaining = limit.saturating_sub(retained.len());
        let keep = remaining.min(count);
        retained.extend_from_slice(&chunk[..keep]);
        if keep < count {
            exceeded = true;
            break;
        }
    }
    Ok((retained, exceeded))
}

fn allowed_raw_diff_diagnostics(stderr: &[u8]) -> bool {
    stderr.is_empty()
        || stderr == b"warning: lazy fetching disabled; some objects may not be available\n"
}

fn isolated_git_command(repository: &Path) -> Command {
    let mut command = Command::new("git");
    for (name, _) in env::vars_os() {
        if unsafe_git_environment(&name) {
            command.env_remove(name);
        }
    }
    command
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_GRAFT_FILE", "")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .arg("--no-replace-objects")
        .arg("-c")
        .arg(format!("diff.orderFile={}", null_device()))
        .arg("-C")
        .arg(repository);
    command
}

fn unsafe_git_environment(name: &OsStr) -> bool {
    name.as_encoded_bytes()
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"GIT_"))
}

#[cfg(windows)]
fn null_device() -> &'static str {
    "NUL"
}

#[cfg(not(windows))]
fn null_device() -> &'static str {
    "/dev/null"
}

#[cfg(test)]
mod tests {
    use super::{
        FileStatus, GitChange, GitChangeIdentity, GitPath, MAX_REVIEW_MARKDOWN_BYTES, PathEncoding,
        RepositoryReview, ReviewFile, ReviewLane, ReviewPriority, ReviewSummary,
        allowed_raw_diff_diagnostics, line_changes, markdown_cell, markdown_code, markdown_report,
        pair_unique_exact_relocations, parse_raw_diff, read_bounded,
    };

    #[test]
    fn parses_modified_added_deleted_and_renamed_records() {
        let zero = "0000000000000000000000000000000000000000";
        let one = "1111111111111111111111111111111111111111";
        let two = "2222222222222222222222222222222222222222";
        let bytes = format!(
            ":100644 100644 {one} {two} M\0src/a.rs\0\
             :000000 100644 {zero} {one} A\0src/new.rs\0\
             :100644 000000 {one} {zero} D\0src/old.rs\0\
             :100644 100644 {one} {two} R087\0src/from.rs\0src/to.rs\0"
        );
        let changes = parse_raw_diff(bytes.as_bytes(), 10).unwrap();
        assert_eq!(changes.len(), 4);
        assert_eq!(changes[0].status, FileStatus::Modified);
        assert_eq!(changes[1].status, FileStatus::Added);
        assert_eq!(changes[2].status, FileStatus::Deleted);
        assert_eq!(
            changes[3],
            GitChange {
                status: FileStatus::Renamed,
                similarity_percent: Some(87),
                before_path: Some(GitPath {
                    display: "src/from.rs".to_owned(),
                    encoding: PathEncoding::Utf8,
                }),
                after_path: Some(GitPath {
                    display: "src/to.rs".to_owned(),
                    encoding: PathEncoding::Utf8,
                }),
                before_mode: Some("100644".to_owned()),
                after_mode: Some("100644".to_owned()),
                before_blob: Some(one.to_owned()),
                after_blob: Some(two.to_owned()),
            }
        );
    }

    #[test]
    fn accepts_only_the_expected_no_lazy_fetch_warning() {
        assert!(allowed_raw_diff_diagnostics(b""));
        assert!(allowed_raw_diff_diagnostics(
            b"warning: lazy fetching disabled; some objects may not be available\n"
        ));
        assert!(!allowed_raw_diff_diagnostics(
            b"warning: lazy fetching disabled; some objects may not be available\nother warning\n"
        ));
        assert!(!allowed_raw_diff_diagnostics(b"warning: other warning\n"));
    }

    #[test]
    fn parses_copy_and_type_change_records_and_enforces_the_file_limit() {
        let one = "1111111111111111111111111111111111111111";
        let two = "2222222222222222222222222222222222222222";
        let bytes = format!(
            ":100644 100644 {one} {two} C100\0src/from.rs\0src/copy.rs\0\
             :100644 100755 {one} {two} T\0script.sh\0"
        );
        let changes = parse_raw_diff(bytes.as_bytes(), 2).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].status, FileStatus::Copied);
        assert_eq!(changes[0].similarity_percent, Some(100));
        assert_eq!(changes[1].status, FileStatus::TypeChanged);
        assert_eq!(changes[1].before_mode.as_deref(), Some("100644"));
        assert_eq!(changes[1].after_mode.as_deref(), Some("100755"));

        let error = parse_raw_diff(bytes.as_bytes(), 1).unwrap_err();
        assert!(error.to_string().contains("changed file limit exceeded"));
    }

    #[test]
    fn markdown_cells_do_not_break_tables_or_emit_controls() {
        assert_eq!(
            markdown_cell("a|b\\c\nc\u{1b}\u{202e}"),
            "a&#124;b\\\\c\\nc\\u{1b}\\u{202e}"
        );
        assert_eq!(
            markdown_code("tick`|path"),
            "<code>tick&#96;&#124;path</code>"
        );
    }

    #[test]
    fn non_utf8_paths_are_retained_with_a_reversible_encoding() {
        let one = "1111111111111111111111111111111111111111";
        let two = "2222222222222222222222222222222222222222";
        let mut bytes = format!(":100644 100644 {one} {two} M\0").into_bytes();
        bytes.extend_from_slice(b"bad-\xff.py\0");
        let changes = parse_raw_diff(&bytes, 1).unwrap();
        assert_eq!(
            changes[0].before_path,
            Some(GitPath {
                display: "git-bytes:%62%61%64%2D%FF%2E%70%79".to_owned(),
                encoding: PathEncoding::GitBytesPercentEncoded,
            })
        );
    }

    #[test]
    fn pairs_only_unique_exact_delete_add_relocations() {
        let zero = "0000000000000000000000000000000000000000";
        let one = "1111111111111111111111111111111111111111";
        let bytes = format!(
            ":100644 000000 {one} {zero} D\0src/from.rs\0\
             :000000 100644 {zero} {one} A\0bin/to.rs\0"
        );
        let changes = pair_unique_exact_relocations(parse_raw_diff(bytes.as_bytes(), 2).unwrap());
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].status, FileStatus::Renamed);
        assert_eq!(changes[0].similarity_percent, Some(100));
        assert_eq!(
            changes[0]
                .before_path
                .as_ref()
                .map(|path| path.display.as_str()),
            Some("src/from.rs")
        );
        assert_eq!(
            changes[0]
                .after_path
                .as_ref()
                .map(|path| path.display.as_str()),
            Some("bin/to.rs")
        );
        assert_eq!(changes[0].before_mode.as_deref(), Some("100644"));
        assert_eq!(changes[0].after_mode.as_deref(), Some("100644"));
    }

    #[test]
    fn duplicate_exact_relocations_remain_explicit_adds_and_deletes() {
        let zero = "0000000000000000000000000000000000000000";
        let one = "1111111111111111111111111111111111111111";
        let bytes = format!(
            ":100644 000000 {one} {zero} D\0from.rs\0\
             :000000 100644 {zero} {one} A\0copy-a.rs\0\
             :000000 100644 {zero} {one} A\0copy-b.rs\0"
        );
        let changes = pair_unique_exact_relocations(parse_raw_diff(bytes.as_bytes(), 3).unwrap());
        assert_eq!(changes.len(), 3);
        assert_eq!(
            changes
                .iter()
                .filter(|change| change.status == FileStatus::Renamed)
                .count(),
            0
        );
    }

    #[test]
    fn exact_objects_with_different_modes_are_not_paired_as_renames() {
        let zero = "0000000000000000000000000000000000000000";
        let one = "1111111111111111111111111111111111111111";
        let bytes = format!(
            ":100644 000000 {one} {zero} D\0from.txt\0\
             :000000 120000 {zero} {one} A\0to-link\0"
        );
        let changes = pair_unique_exact_relocations(parse_raw_diff(bytes.as_bytes(), 2).unwrap());
        assert_eq!(changes.len(), 2);
        assert!(
            changes
                .iter()
                .all(|change| change.status != FileStatus::Renamed)
        );
    }

    #[test]
    fn checkpoint_identity_includes_every_git_change_field() {
        let original = GitChange {
            status: FileStatus::Renamed,
            similarity_percent: Some(100),
            before_path: Some(GitPath {
                display: "src/before.rs".to_owned(),
                encoding: PathEncoding::Utf8,
            }),
            after_path: Some(GitPath {
                display: "src/after.rs".to_owned(),
                encoding: PathEncoding::Utf8,
            }),
            before_mode: Some("100644".to_owned()),
            after_mode: Some("100755".to_owned()),
            before_blob: Some("1".repeat(40)),
            after_blob: Some("2".repeat(40)),
        };
        let identity = GitChangeIdentity::from(&original);
        let assert_distinct = |candidate: GitChange| {
            assert_ne!(identity, GitChangeIdentity::from(&candidate));
        };

        let mut candidate = original.clone();
        candidate.status = FileStatus::Copied;
        assert_distinct(candidate);
        let mut candidate = original.clone();
        candidate.similarity_percent = Some(99);
        assert_distinct(candidate);
        let mut candidate = original.clone();
        candidate.before_path.as_mut().unwrap().display = "src/other-before.rs".to_owned();
        assert_distinct(candidate);
        let mut candidate = original.clone();
        candidate.before_path.as_mut().unwrap().encoding = PathEncoding::GitBytesPercentEncoded;
        assert_distinct(candidate);
        let mut candidate = original.clone();
        candidate.after_path.as_mut().unwrap().display = "src/other-after.rs".to_owned();
        assert_distinct(candidate);
        let mut candidate = original.clone();
        candidate.after_path.as_mut().unwrap().encoding = PathEncoding::GitBytesPercentEncoded;
        assert_distinct(candidate);
        let mut candidate = original.clone();
        candidate.before_mode = Some("100755".to_owned());
        assert_distinct(candidate);
        let mut candidate = original.clone();
        candidate.after_mode = Some("100644".to_owned());
        assert_distinct(candidate);
        let mut candidate = original.clone();
        candidate.before_blob = Some("3".repeat(40));
        assert_distinct(candidate);
        let mut candidate = original;
        candidate.after_blob = Some("4".repeat(40));
        assert_distinct(candidate);
    }

    #[test]
    fn bounded_reader_stops_after_retaining_the_limit() {
        let (bytes, exceeded) = read_bounded(std::io::Cursor::new(vec![7_u8; 32]), 10).unwrap();
        assert_eq!(bytes, vec![7_u8; 10]);
        assert!(exceeded);
    }

    #[test]
    fn line_envelope_is_linear_and_conservatively_keeps_the_middle() {
        let changes = line_changes(
            b"before\nshared\ntail-before\n",
            b"after\nshared\ntail-after\n",
        )
        .unwrap();
        assert_eq!(changes.additions, 3);
        assert_eq!(changes.deletions, 3);
    }

    #[test]
    fn markdown_summary_is_bounded_and_reports_omissions() {
        let file = ReviewFile {
            status: FileStatus::Added,
            similarity_percent: None,
            before_path: None,
            before_path_encoding: None,
            after_path: Some(format!("{}.rs", "nested/".repeat(500))),
            after_path_encoding: Some(PathEncoding::Utf8),
            before_mode: None,
            after_mode: Some("100644".to_owned()),
            before_blob: None,
            after_blob: Some("1".repeat(40)),
            before_bytes: None,
            after_bytes: Some(1),
            line_change_envelope: None,
            language: None,
            priority: ReviewPriority::ReviewFirst,
            lane: ReviewLane::ReviewFirst,
            checkpoint_state: None,
            checkpoint_match_basis: None,
            reason: "new file".to_owned(),
            evidence: None,
        };
        let review = RepositoryReview {
            schema: "schema".to_owned(),
            engine_version: "version".to_owned(),
            requested_base: "base".to_owned(),
            requested_head: "head".to_owned(),
            base_commit: "1".repeat(40),
            head_commit: "2".repeat(40),
            comparison: "merge_base_to_head".to_owned(),
            checkpoint: None,
            summary: ReviewSummary {
                changed_files: 1_000,
                first_pass_files: 1_000,
                review_first_files: 1_000,
                syntax_preserved_files: 0,
                content_preserved_files: 0,
                unverified_files: 0,
                replay_check_passed_files: 0,
                replay_check_not_run_files: 1_000,
                line_envelope_complete: false,
                changed_line_envelope: None,
                first_pass_line_envelope: None,
                checkpoint: None,
            },
            files: vec![file; 1_000],
        };
        let markdown = markdown_report(&review);
        assert!(markdown.len() <= MAX_REVIEW_MARKDOWN_BYTES);
        assert!(markdown.contains("additional files omitted"));
    }

    #[test]
    fn lane_labels_are_stable() {
        assert_eq!(ReviewLane::ReviewFirst.label(), "structural delta");
        assert_eq!(
            ReviewLane::SyntaxPreserved.label(),
            "parser model matched (non-semantic)"
        );
        assert_eq!(ReviewLane::ContentPreserved.label(), "same Git object");
        assert_eq!(ReviewLane::Unverified.label(), "unverified");
    }
}
