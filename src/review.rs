use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsStr,
    fmt,
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
pub const REVIEW_DELTA_SCHEMA: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/review-delta-v1.schema.json";
pub const MAX_REVIEW_FILES: usize = 1_000;
pub const MAX_REVIEW_TOTAL_SOURCE_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_GITHUB_WORKFLOW_ANNOTATIONS: usize = 20;
const MAX_REVIEW_TOTAL_LINE_STAT_BYTES: usize = 128 * 1024 * 1024;
const MAX_REVIEW_MARKDOWN_BYTES: usize = 900 * 1024;
const MAX_GIT_RAW_DIFF_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDeltaComparison {
    CheckpointToHead,
    PerFileReviewBaselineToHead,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDeltaBaselineBasis {
    CheckpointSnapshot,
    CurrentBaseNoCheckpointChange,
    ReconstructedReviewBaseline,
    CurrentBaseFallback,
    CheckpointHeadFallback,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDeltaFallbackReason {
    OverlapOrAdjacent,
    BinaryNul,
    SourceUnavailable,
    UnsupportedChange,
    TranslationFailed,
    ReplayOrdersMismatch,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDeltaUnresolvedReason {
    NonUtf8GitPath,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct ReviewChangeIdentity {
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

    pub fn change_identity(&self) -> ReviewChangeIdentity {
        ReviewChangeIdentity {
            status: self.status,
            similarity_percent: self.similarity_percent,
            before_path: self.before_path.clone(),
            before_path_encoding: self.before_path_encoding,
            after_path: self.after_path.clone(),
            after_path_encoding: self.after_path_encoding,
            before_mode: self.before_mode.clone(),
            after_mode: self.after_mode.clone(),
            before_blob: self.before_blob.clone(),
            after_blob: self.after_blob.clone(),
        }
    }

    pub fn ownership_path(&self) -> Option<(&str, PathEncoding)> {
        self.after_path
            .as_deref()
            .zip(self.after_path_encoding)
            .or_else(|| self.before_path.as_deref().zip(self.before_path_encoding))
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
    pub schema: String,
    pub engine_version: String,
    pub comparison: ReviewDeltaComparison,
    pub old_base_commit: String,
    pub checkpoint_commit: String,
    pub current_base_commit: String,
    pub head_commit: String,
    pub summary: ReviewDeltaSummary,
    pub entries: Vec<ReviewDeltaFile>,
    pub unresolved_retired_changes: Vec<ReviewDeltaUnresolvedChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewDeltaSummary {
    pub displayable_files: usize,
    pub unresolved_retired_changes: usize,
    pub needs_review_files: usize,
    pub gate_passed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewDeltaUnresolvedChange {
    pub path: String,
    pub path_encoding: PathEncoding,
    pub reason: ReviewDeltaUnresolvedReason,
}

impl ReviewDelta {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == REVIEW_DELTA_SCHEMA,
            "unsupported review delta schema"
        );
        ensure!(
            !self.engine_version.is_empty(),
            "review delta engine version is empty"
        );
        for commit in [
            &self.old_base_commit,
            &self.checkpoint_commit,
            &self.current_base_commit,
            &self.head_commit,
        ] {
            ensure!(
                is_object_id(commit),
                "review delta contains an invalid commit id"
            );
        }
        ensure!(
            self.summary.displayable_files == self.entries.len(),
            "review delta displayable file count is inconsistent"
        );
        ensure!(
            self.summary.unresolved_retired_changes == self.unresolved_retired_changes.len(),
            "review delta unresolved retired change count is inconsistent"
        );
        let needs_review_files = self.entries.len() + self.unresolved_retired_changes.len();
        ensure!(
            self.summary.needs_review_files == needs_review_files,
            "review delta queue total is inconsistent"
        );
        ensure!(
            self.summary.gate_passed == (needs_review_files == 0),
            "review delta gate state is inconsistent"
        );
        ensure!(
            self.comparison != ReviewDeltaComparison::CheckpointToHead
                || self.old_base_commit == self.current_base_commit,
            "checkpoint-to-head delta has inconsistent merge bases"
        );

        for entry in &self.entries {
            validate_review_delta_entry(entry, self)?;
        }
        let mut unresolved_paths = HashSet::new();
        for change in &self.unresolved_retired_changes {
            ensure!(
                change.reason != ReviewDeltaUnresolvedReason::NonUtf8GitPath
                    || change.path_encoding == PathEncoding::GitBytesPercentEncoded,
                "non-UTF-8 unresolved change has the wrong path encoding"
            );
            ensure!(
                !change.path.is_empty() && unresolved_paths.insert(&change.path),
                "review delta contains an empty or duplicate unresolved path"
            );
        }
        Ok(())
    }
}

fn validate_review_delta_entry(entry: &ReviewDeltaFile, delta: &ReviewDelta) -> Result<()> {
    let expected_before_commit = match entry.baseline_basis {
        ReviewDeltaBaselineBasis::CheckpointSnapshot
        | ReviewDeltaBaselineBasis::CheckpointHeadFallback => &delta.checkpoint_commit,
        ReviewDeltaBaselineBasis::CurrentBaseNoCheckpointChange
        | ReviewDeltaBaselineBasis::CurrentBaseFallback
        | ReviewDeltaBaselineBasis::ReconstructedReviewBaseline => &delta.current_base_commit,
    };
    match entry.baseline_basis {
        ReviewDeltaBaselineBasis::ReconstructedReviewBaseline => {
            let reconstruction = entry
                .baseline_reconstruction
                .as_ref()
                .context("reconstructed review delta has no reconstruction evidence")?;
            ensure!(
                entry.fallback_reason.is_none(),
                "reconstructed review delta unexpectedly records a fallback"
            );
            ensure!(
                reconstruction.algorithm == "bidirectional_noninteracting_byte_replay_v1",
                "unsupported review baseline reconstruction algorithm"
            );
            ensure!(
                reconstruction.reviewed_on_current_base_blake3
                    == reconstruction.upstream_on_checkpoint_blake3
                    && reconstruction.reconstructed_blake3
                        == reconstruction.reviewed_on_current_base_blake3,
                "review baseline replay orders disagree"
            );
            ensure!(
                [
                    &reconstruction.old_base_blob,
                    &reconstruction.reviewed_blob,
                    &reconstruction.current_base_blob,
                ]
                .into_iter()
                .all(|object_id| is_object_id(object_id)),
                "review baseline reconstruction contains an invalid object id"
            );
            ensure!(
                [
                    &reconstruction.reviewed_on_current_base_blake3,
                    &reconstruction.upstream_on_checkpoint_blake3,
                    &reconstruction.reconstructed_blake3,
                ]
                .into_iter()
                .all(|digest| is_blake3(digest)),
                "review baseline reconstruction contains an invalid digest"
            );
            match &entry.before_source {
                ReviewDeltaSource::ReconstructedBytes { blake3, byte_len } => {
                    ensure!(
                        is_blake3(blake3)
                            && blake3 == &reconstruction.reconstructed_blake3
                            && *byte_len == reconstruction.byte_len
                            && entry.file.before_bytes == Some(*byte_len)
                            && entry.file.before_blob.is_none(),
                        "reconstructed review baseline metadata is inconsistent"
                    );
                }
                ReviewDeltaSource::GitObject { .. } | ReviewDeltaSource::Empty => {
                    bail!("reconstructed review delta has the wrong before source")
                }
            }
        }
        ReviewDeltaBaselineBasis::CurrentBaseFallback
        | ReviewDeltaBaselineBasis::CheckpointHeadFallback => {
            ensure!(
                entry.fallback_reason.is_some(),
                "review delta fallback has no reason"
            );
            ensure!(
                entry.baseline_reconstruction.is_none(),
                "review delta fallback unexpectedly has reconstruction evidence"
            );
            validate_review_delta_git_source(
                &entry.before_source,
                expected_before_commit,
                entry.file.before_blob.as_deref(),
                entry.file.before_bytes,
            )?;
        }
        ReviewDeltaBaselineBasis::CheckpointSnapshot
        | ReviewDeltaBaselineBasis::CurrentBaseNoCheckpointChange => {
            ensure!(
                entry.fallback_reason.is_none() && entry.baseline_reconstruction.is_none(),
                "exact review delta entry contains fallback evidence"
            );
            validate_review_delta_git_source(
                &entry.before_source,
                expected_before_commit,
                entry.file.before_blob.as_deref(),
                entry.file.before_bytes,
            )?;
        }
    }
    validate_review_delta_git_source(
        &entry.after_source,
        &delta.head_commit,
        entry.file.after_blob.as_deref(),
        entry.file.after_bytes,
    )
}

fn validate_review_delta_git_source(
    source: &ReviewDeltaSource,
    expected_commit: &str,
    expected_object_id: Option<&str>,
    expected_byte_len: Option<usize>,
) -> Result<()> {
    match source {
        ReviewDeltaSource::GitObject {
            commit,
            object_id,
            byte_len,
        } => {
            ensure!(
                is_object_id(object_id),
                "review delta source has an invalid object id"
            );
            ensure!(
                commit == expected_commit
                    && Some(object_id.as_str()) == expected_object_id
                    && *byte_len == expected_byte_len,
                "review delta Git source metadata is inconsistent"
            );
        }
        ReviewDeltaSource::Empty => {
            ensure!(
                expected_object_id.is_none() && expected_byte_len.is_none(),
                "empty review delta source contradicts file metadata"
            );
        }
        ReviewDeltaSource::ReconstructedBytes { .. } => {
            bail!("unexpected reconstructed Git source")
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewFileSources {
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewDeltaSource {
    GitObject {
        commit: String,
        object_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        byte_len: Option<usize>,
    },
    ReconstructedBytes {
        blake3: String,
        byte_len: usize,
    },
    Empty,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewBaselineReconstruction {
    pub algorithm: String,
    pub old_base_blob: String,
    pub reviewed_blob: String,
    pub current_base_blob: String,
    pub reviewed_on_current_base_blake3: String,
    pub upstream_on_checkpoint_blake3: String,
    pub reconstructed_blake3: String,
    pub byte_len: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewDeltaFile {
    pub file: ReviewFile,
    pub baseline_basis: ReviewDeltaBaselineBasis,
    pub before_source: ReviewDeltaSource,
    pub after_source: ReviewDeltaSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_reconstruction: Option<ReviewBaselineReconstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<ReviewDeltaFallbackReason>,
    #[serde(skip)]
    source_override: Option<ReviewFileSources>,
}

impl ReviewDeltaFile {
    pub fn display_path(&self) -> String {
        self.file.display_path()
    }
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

#[derive(Debug)]
pub(crate) struct ReviewAnalysisBudgetExceeded {
    resource: &'static str,
    observed: usize,
    limit: usize,
}

impl fmt::Display for ReviewAnalysisBudgetExceeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "review analysis {} budget exceeded: observed at least {}, limit {}",
            self.resource, self.observed, self.limit
        )
    }
}

impl std::error::Error for ReviewAnalysisBudgetExceeded {}

pub(crate) struct ReviewAnalysisContext {
    max_file_visits: Option<usize>,
    file_visits: usize,
    max_source_bytes: Option<usize>,
    source_bytes: usize,
    processed_source_bytes: usize,
    metadata_bytes: usize,
    diff_queries: usize,
    source_oids: HashSet<String>,
    retain_source_overrides: bool,
    changes: HashMap<(String, String), Vec<GitChange>>,
    analyzed_files: HashMap<GitChangeIdentity, ReviewFile>,
    blob_loader: BlobLoader,
}

impl ReviewAnalysisContext {
    fn unbounded() -> Self {
        Self {
            max_file_visits: None,
            file_visits: 0,
            max_source_bytes: None,
            source_bytes: 0,
            processed_source_bytes: 0,
            metadata_bytes: 0,
            diff_queries: 0,
            source_oids: HashSet::new(),
            retain_source_overrides: true,
            changes: HashMap::new(),
            analyzed_files: HashMap::new(),
            blob_loader: BlobLoader::default(),
        }
    }

    pub(crate) fn bounded(max_file_visits: usize, max_source_bytes: usize) -> Self {
        Self {
            max_file_visits: Some(max_file_visits),
            file_visits: 0,
            max_source_bytes: Some(max_source_bytes),
            source_bytes: 0,
            processed_source_bytes: 0,
            metadata_bytes: 0,
            diff_queries: 0,
            source_oids: HashSet::new(),
            retain_source_overrides: false,
            changes: HashMap::new(),
            analyzed_files: HashMap::new(),
            blob_loader: BlobLoader::default(),
        }
    }

    fn consume_file_visits(&mut self, additional: usize) -> Result<()> {
        let Some(limit) = self.max_file_visits else {
            return Ok(());
        };
        self.file_visits =
            checked_analysis_budget_add(self.file_visits, additional, limit, "file visit")?;
        Ok(())
    }

    fn consume_diff_query(&mut self) -> Result<()> {
        let Some(limit) = self.max_file_visits else {
            return Ok(());
        };
        self.diff_queries = checked_analysis_budget_add(self.diff_queries, 1, limit, "diff query")?;
        Ok(())
    }

    fn consume_metadata_bytes(&mut self, additional: usize) -> Result<()> {
        let Some(limit) = self.max_source_bytes else {
            return Ok(());
        };
        self.metadata_bytes = checked_analysis_budget_add(
            self.metadata_bytes,
            additional,
            limit,
            "diff metadata byte",
        )?;
        Ok(())
    }

    fn consume_processed_source_bytes(&mut self, additional: usize) -> Result<()> {
        let Some(limit) = self.max_source_bytes else {
            return Ok(());
        };
        self.processed_source_bytes = checked_analysis_budget_add(
            self.processed_source_bytes,
            additional,
            limit,
            "processed source byte",
        )?;
        Ok(())
    }

    fn discover_changes(
        &mut self,
        repository: &Path,
        from_commit: &str,
        to_commit: &str,
    ) -> Result<Vec<GitChange>> {
        let key = (from_commit.to_owned(), to_commit.to_owned());
        if let Some(changes) = self.changes.get(&key) {
            return Ok(changes.clone());
        }
        self.consume_diff_query()?;
        let remaining_metadata_bytes = self
            .max_source_bytes
            .map(|limit| limit.saturating_sub(self.metadata_bytes));
        let output_limit = remaining_metadata_bytes.map_or(MAX_GIT_RAW_DIFF_BYTES, |remaining| {
            remaining.min(MAX_GIT_RAW_DIFF_BYTES)
        });
        let captured =
            discover_git_change_output(repository, from_commit, to_commit, output_limit)?;
        let observed_bytes = captured
            .output
            .stdout
            .len()
            .saturating_add(usize::from(captured.stdout_exceeded));
        self.consume_metadata_bytes(observed_bytes)?;
        if captured.stdout_exceeded {
            let (resource, limit) = if output_limit < MAX_GIT_RAW_DIFF_BYTES {
                (
                    "diff metadata byte",
                    self.max_source_bytes.expect("bounded metadata budget"),
                )
            } else {
                ("raw diff metadata byte", MAX_GIT_RAW_DIFF_BYTES)
            };
            return Err(ReviewAnalysisBudgetExceeded {
                resource,
                observed: if output_limit < MAX_GIT_RAW_DIFF_BYTES {
                    self.metadata_bytes.saturating_add(1)
                } else {
                    MAX_GIT_RAW_DIFF_BYTES + 1
                },
                limit,
            }
            .into());
        }
        let changes = parse_git_change_output(captured)?;
        self.changes.insert(key, changes.clone());
        Ok(changes)
    }

    fn reserve_sources<'a>(
        &mut self,
        repository: &Path,
        changes: impl IntoIterator<Item = &'a GitChange>,
        limits: &VerificationLimits,
    ) -> Result<()> {
        let Some(limit) = self.max_source_bytes else {
            return Ok(());
        };
        let mut candidates = HashMap::<String, String>::new();
        for change in changes {
            for (object_id, mode) in [
                (change.before_blob.as_ref(), change.before_mode.as_ref()),
                (change.after_blob.as_ref(), change.after_mode.as_ref()),
            ] {
                let (Some(object_id), Some(mode)) = (object_id, mode) else {
                    continue;
                };
                if !matches!(mode.as_str(), "100644" | "100755") {
                    continue;
                }
                if self.source_oids.contains(object_id) || candidates.contains_key(object_id) {
                    continue;
                }
                candidates.insert(object_id.clone(), mode.clone());
            }
        }

        let mut reservations = Vec::new();
        let mut additional_bytes = 0_usize;
        for (object_id, mode) in candidates {
            let Some(blob) = inspect_optional_blob(
                repository,
                Some(&object_id),
                Some(&mode),
                limits,
                &mut self.blob_loader,
            )?
            else {
                continue;
            };
            let Some(size) = blob.size.filter(|_| blob.unavailable_reason.is_none()) else {
                continue;
            };
            additional_bytes =
                checked_analysis_budget_add(additional_bytes, size, usize::MAX, "source byte")?;
            reservations.push(object_id);
        }
        self.source_bytes =
            checked_analysis_budget_add(self.source_bytes, additional_bytes, limit, "source byte")?;
        self.source_oids.extend(reservations);
        Ok(())
    }

    fn source_work_bytes<'a>(
        &mut self,
        repository: &Path,
        sources: impl IntoIterator<Item = (Option<&'a String>, Option<&'a String>)>,
        limits: &VerificationLimits,
    ) -> Result<usize> {
        let mut bytes = 0_usize;
        for (object_id, mode) in sources {
            let (Some(object_id), Some(mode)) = (object_id, mode) else {
                continue;
            };
            let Some(blob) = inspect_optional_blob(
                repository,
                Some(object_id),
                Some(mode),
                limits,
                &mut self.blob_loader,
            )?
            else {
                continue;
            };
            let Some(size) = blob.size.filter(|_| blob.unavailable_reason.is_none()) else {
                continue;
            };
            bytes = checked_analysis_budget_add(bytes, size, usize::MAX, "processed source byte")?;
        }
        Ok(bytes)
    }

    fn reserve_change_work(
        &mut self,
        repository: &Path,
        change: &GitChange,
        limits: &VerificationLimits,
    ) -> Result<()> {
        let bytes = self.source_work_bytes(
            repository,
            [
                (change.before_blob.as_ref(), change.before_mode.as_ref()),
                (change.after_blob.as_ref(), change.after_mode.as_ref()),
            ],
            limits,
        )?;
        self.consume_processed_source_bytes(bytes)
    }

    fn reconstruct_review_baseline(
        &mut self,
        repository: &Path,
        checkpoint: &GitChange,
        current: &GitChange,
        limits: &VerificationLimits,
    ) -> Result<BaselineReconstructionOutcome> {
        if replay_candidate_metadata_matches(checkpoint, current) {
            let bytes = self.source_work_bytes(
                repository,
                [
                    (
                        checkpoint.before_blob.as_ref(),
                        checkpoint.before_mode.as_ref(),
                    ),
                    (
                        checkpoint.after_blob.as_ref(),
                        checkpoint.after_mode.as_ref(),
                    ),
                    (current.before_blob.as_ref(), current.before_mode.as_ref()),
                    (current.after_blob.as_ref(), current.after_mode.as_ref()),
                ],
                limits,
            )?;
            self.consume_processed_source_bytes(bytes)?;
        }
        reconstruct_review_baseline(
            repository,
            checkpoint,
            current,
            limits,
            &mut self.blob_loader,
        )
    }

    fn analyze_change(
        &mut self,
        repository: &Path,
        change: GitChange,
        limits: &VerificationLimits,
    ) -> Result<ReviewFile> {
        let identity = GitChangeIdentity::from(&change);
        if let Some(file) = self.analyzed_files.get(&identity) {
            return Ok(file.clone());
        }
        self.reserve_change_work(repository, &change, limits)?;
        let file = analyze_change(repository, change, limits, &mut self.blob_loader)?;
        self.analyzed_files.insert(identity, file.clone());
        Ok(file)
    }
}

fn checked_analysis_budget_add(
    current: usize,
    additional: usize,
    limit: usize,
    resource: &'static str,
) -> Result<usize> {
    let Some(observed) = current.checked_add(additional) else {
        return Err(ReviewAnalysisBudgetExceeded {
            resource,
            observed: usize::MAX,
            limit,
        }
        .into());
    };
    if observed > limit {
        return Err(ReviewAnalysisBudgetExceeded {
            resource,
            observed,
            limit,
        }
        .into());
    }
    Ok(observed)
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
    let mut context = ReviewAnalysisContext::unbounded();
    review_git_range_with_analysis(repository, base, head, None, &mut context)
}

pub fn review_git_range_with_checkpoint(
    repository: &Path,
    base: &str,
    head: &str,
    checkpoint: Option<&str>,
) -> Result<RepositoryReview> {
    let mut context = ReviewAnalysisContext::unbounded();
    review_git_range_with_analysis(repository, base, head, checkpoint, &mut context)
}

pub(crate) fn review_git_range_with_analysis(
    repository: &Path,
    base: &str,
    head: &str,
    checkpoint: Option<&str>,
    context: &mut ReviewAnalysisContext,
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
            context.discover_changes(repository, &checkpoint.base_commit, &checkpoint.commit)
        })
        .transpose()?;
    let changes = context.discover_changes(repository, &base_commit, &head_commit)?;
    let base_changed = checkpoint
        .as_ref()
        .is_some_and(|checkpoint| checkpoint.base_commit != base_commit);

    let limits = VerificationLimits::default();
    let checkpoint_file_count = checkpoint_changes.as_ref().map_or(0, Vec::len);
    context.consume_file_visits(changes.len())?;
    context.consume_file_visits(checkpoint_file_count)?;
    context.reserve_sources(
        repository,
        changes
            .iter()
            .chain(checkpoint_changes.as_deref().unwrap_or_default().iter()),
        &limits,
    )?;
    let mut files = Vec::with_capacity(changes.len());
    let mut matched_checkpoint_indices = HashSet::new();
    for change in changes {
        let mut carried_by_replay = false;
        let mut checkpoint_match_basis = None;
        let checkpoint_state = checkpoint_changes
            .as_ref()
            .map(|checkpoint_changes| -> Result<CheckpointState> {
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
                    return Ok(CheckpointState::UnchangedSinceCheckpoint);
                }

                let replay_match = if base_changed {
                    if let Some((index, checkpoint_change)) =
                        unique_replay_candidate(checkpoint_changes, &change)
                    {
                        if matched_checkpoint_indices.contains(&index) {
                            false
                        } else {
                            match independent_four_way_replay_matches(
                                repository,
                                checkpoint_change,
                                &change,
                                &limits,
                                context,
                            ) {
                                Ok(true) => {
                                    matched_checkpoint_indices.insert(index);
                                    true
                                }
                                Ok(false) => false,
                                Err(error) => {
                                    if error
                                        .downcast_ref::<ReviewAnalysisBudgetExceeded>()
                                        .is_some()
                                    {
                                        return Err(error);
                                    }
                                    false
                                }
                            }
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };
                if replay_match {
                    carried_by_replay = true;
                    checkpoint_match_basis =
                        Some(CheckpointCarryBasis::ExactNoninteractingFourWayByteReplay);
                    Ok(CheckpointState::UnchangedSinceCheckpoint)
                } else {
                    Ok(CheckpointState::NeedsReviewNow)
                }
            })
            .transpose()?;
        let mut file = context.analyze_change(repository, change, &limits)?;
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
    let mut context = ReviewAnalysisContext::unbounded();
    review_git_snapshot_delta_with_analysis(repository, from, to, &mut context)
}

fn review_git_snapshot_delta_with_analysis(
    repository: &Path,
    from: &str,
    to: &str,
    context: &mut ReviewAnalysisContext,
) -> Result<ReviewDelta> {
    let from_commit = resolve_commit(repository, from)?;
    let to_commit = resolve_commit(repository, to)?;
    let changes = context.discover_changes(repository, &from_commit, &to_commit)?;
    let limits = VerificationLimits::default();
    context.consume_file_visits(changes.len())?;
    context.reserve_sources(repository, &changes, &limits)?;
    let mut files = Vec::with_capacity(changes.len());
    for change in changes {
        files.push(context.analyze_change(repository, change, &limits)?);
    }
    files.sort_by_key(|file| {
        (
            priority_rank(file.priority),
            lane_rank(file.lane),
            file.display_path(),
        )
    });
    let entries = files
        .into_iter()
        .map(|file| {
            review_delta_git_entry(
                file,
                ReviewDeltaBaselineBasis::CheckpointSnapshot,
                &from_commit,
                &to_commit,
                None,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let unresolved_retired_changes = Vec::new();
    let summary = summarize_review_delta(&entries, &unresolved_retired_changes);
    let delta = ReviewDelta {
        schema: REVIEW_DELTA_SCHEMA.to_owned(),
        engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        comparison: ReviewDeltaComparison::CheckpointToHead,
        old_base_commit: from_commit.clone(),
        checkpoint_commit: from_commit.clone(),
        current_base_commit: from_commit,
        head_commit: to_commit,
        summary,
        entries,
        unresolved_retired_changes,
    };
    delta.validate()?;
    Ok(delta)
}

pub fn review_git_resume_delta(
    repository: &Path,
    review: &RepositoryReview,
) -> Result<ReviewDelta> {
    let mut context = ReviewAnalysisContext::unbounded();
    review_git_resume_delta_with_analysis(repository, review, &mut context)
}

pub(crate) fn review_git_resume_delta_with_analysis(
    repository: &Path,
    review: &RepositoryReview,
    context: &mut ReviewAnalysisContext,
) -> Result<ReviewDelta> {
    let checkpoint = review
        .checkpoint
        .as_ref()
        .context("review residue requires a checkpoint")?;
    if checkpoint.base_commit == review.base_commit {
        let mut delta = review_git_snapshot_delta_with_analysis(
            repository,
            &checkpoint.commit,
            &review.head_commit,
            context,
        )?;
        delta.old_base_commit = checkpoint.base_commit.clone();
        delta.current_base_commit = review.base_commit.clone();
        delta.validate()?;
        return Ok(delta);
    }

    let checkpoint_changes =
        context.discover_changes(repository, &checkpoint.base_commit, &checkpoint.commit)?;
    let current_changes =
        context.discover_changes(repository, &review.base_commit, &review.head_commit)?;
    let limits = VerificationLimits::default();
    context.consume_file_visits(checkpoint_changes.len())?;
    context.consume_file_visits(current_changes.len())?;
    context.reserve_sources(
        repository,
        checkpoint_changes.iter().chain(&current_changes),
        &limits,
    )?;
    // A multi-path checkpoint change is consumed only after every distinct path side has been
    // accounted for. A current change touching one side of a rename must not hide the other side.
    let mut associated_checkpoint_paths = HashSet::new();
    let mut entries = Vec::new();

    for current_change in &current_changes {
        let file = review_file_for_change(review, current_change)?.clone();
        let exact_indices = checkpoint_changes
            .iter()
            .enumerate()
            .filter_map(|(index, checkpoint_change)| {
                (GitChangeIdentity::from(checkpoint_change)
                    == GitChangeIdentity::from(current_change))
                .then_some(index)
            })
            .collect::<Vec<_>>();
        for index in exact_indices {
            associate_all_checkpoint_paths(
                &mut associated_checkpoint_paths,
                index,
                &checkpoint_changes[index],
            );
        }
        if file.checkpoint_state == Some(CheckpointState::UnchangedSinceCheckpoint) {
            if let Some((index, _)) = unique_replay_candidate(&checkpoint_changes, current_change) {
                associate_all_checkpoint_paths(
                    &mut associated_checkpoint_paths,
                    index,
                    &checkpoint_changes[index],
                );
            }
            continue;
        }

        if let Some((index, checkpoint_change)) =
            unique_replay_candidate(&checkpoint_changes, current_change)
        {
            associate_all_checkpoint_paths(
                &mut associated_checkpoint_paths,
                index,
                checkpoint_change,
            );
            match context.reconstruct_review_baseline(
                repository,
                checkpoint_change,
                current_change,
                &limits,
            )? {
                BaselineReconstructionOutcome::Reconstructed(reconstruction) => {
                    if reconstruction.baseline != reconstruction.current_after {
                        entries.push(reconstructed_delta_entry(
                            file,
                            current_change,
                            *reconstruction,
                            &review.head_commit,
                            &limits,
                            context.retain_source_overrides,
                        )?);
                    }
                }
                BaselineReconstructionOutcome::Unavailable(reason) => {
                    entries.push(review_delta_git_entry(
                        file,
                        ReviewDeltaBaselineBasis::CurrentBaseFallback,
                        &review.base_commit,
                        &review.head_commit,
                        Some(reason),
                    )?);
                }
            }
            continue;
        }

        let related_paths = related_checkpoint_paths(&checkpoint_changes, current_change);
        if related_paths.is_empty() {
            entries.push(review_delta_git_entry(
                file,
                ReviewDeltaBaselineBasis::CurrentBaseNoCheckpointChange,
                &review.base_commit,
                &review.head_commit,
                None,
            )?);
        } else {
            associated_checkpoint_paths.extend(related_paths);
            entries.push(review_delta_git_entry(
                file,
                ReviewDeltaBaselineBasis::CurrentBaseFallback,
                &review.base_commit,
                &review.head_commit,
                Some(ReviewDeltaFallbackReason::UnsupportedChange),
            )?);
        }
    }

    let mut unresolved_retired_changes = Vec::new();
    for (index, checkpoint_change) in checkpoint_changes.iter().enumerate() {
        let unassociated_paths = checkpoint_change_paths(checkpoint_change)
            .into_iter()
            .filter(|path| !associated_checkpoint_paths.contains(&(index, path.clone())))
            .collect::<Vec<_>>();
        if unassociated_paths.is_empty() {
            continue;
        }
        context.consume_file_visits(1)?;
        let Some(current_snapshot) = same_path_snapshot_change(
            repository,
            &review.base_commit,
            &review.head_commit,
            checkpoint_change,
        )?
        else {
            let fallback = checkpoint_head_fallback_entries(
                repository,
                &unassociated_paths,
                &checkpoint.commit,
                &review.head_commit,
                &limits,
                context,
            )?;
            entries.extend(fallback.entries);
            unresolved_retired_changes.extend(fallback.unresolved);
            continue;
        };

        context.reserve_sources(repository, std::iter::once(&current_snapshot), &limits)?;
        match context.reconstruct_review_baseline(
            repository,
            checkpoint_change,
            &current_snapshot,
            &limits,
        )? {
            BaselineReconstructionOutcome::Reconstructed(reconstruction) => {
                if reconstruction.baseline != reconstruction.current_after {
                    let fallback_file =
                        context.analyze_change(repository, current_snapshot.clone(), &limits)?;
                    entries.push(reconstructed_delta_entry(
                        fallback_file,
                        &current_snapshot,
                        *reconstruction,
                        &review.head_commit,
                        &limits,
                        context.retain_source_overrides,
                    )?);
                }
            }
            BaselineReconstructionOutcome::Unavailable(_) => {
                let fallback = checkpoint_head_fallback_entries(
                    repository,
                    &unassociated_paths,
                    &checkpoint.commit,
                    &review.head_commit,
                    &limits,
                    context,
                )?;
                entries.extend(fallback.entries);
                unresolved_retired_changes.extend(fallback.unresolved);
            }
        }
    }

    entries.sort_by_key(|entry| {
        (
            priority_rank(entry.file.priority),
            lane_rank(entry.file.lane),
            entry.display_path(),
        )
    });
    unresolved_retired_changes.sort_by(|left, right| left.path.cmp(&right.path));
    unresolved_retired_changes.dedup();
    let summary = summarize_review_delta(&entries, &unresolved_retired_changes);
    let delta = ReviewDelta {
        schema: REVIEW_DELTA_SCHEMA.to_owned(),
        engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        comparison: ReviewDeltaComparison::PerFileReviewBaselineToHead,
        old_base_commit: checkpoint.base_commit.clone(),
        checkpoint_commit: checkpoint.commit.clone(),
        current_base_commit: review.base_commit.clone(),
        head_commit: review.head_commit.clone(),
        summary,
        entries,
        unresolved_retired_changes,
    };
    delta.validate()?;
    Ok(delta)
}

pub fn load_review_delta_file_sources(
    repository: &Path,
    entry: &ReviewDeltaFile,
) -> Result<ReviewFileSources> {
    match &entry.source_override {
        Some(sources) => Ok(sources.clone()),
        None => load_review_file_sources(repository, &entry.file),
    }
}

fn summarize_review_delta(
    entries: &[ReviewDeltaFile],
    unresolved: &[ReviewDeltaUnresolvedChange],
) -> ReviewDeltaSummary {
    let displayable_files = entries.len();
    let unresolved_retired_changes = unresolved.len();
    let needs_review_files = displayable_files + unresolved_retired_changes;
    ReviewDeltaSummary {
        displayable_files,
        unresolved_retired_changes,
        needs_review_files,
        gate_passed: needs_review_files == 0,
    }
}

fn review_delta_git_entry(
    file: ReviewFile,
    baseline_basis: ReviewDeltaBaselineBasis,
    before_commit: &str,
    after_commit: &str,
    fallback_reason: Option<ReviewDeltaFallbackReason>,
) -> Result<ReviewDeltaFile> {
    let before_source = review_delta_git_source(
        before_commit,
        file.before_blob.as_deref(),
        file.before_bytes,
    )?;
    let after_source =
        review_delta_git_source(after_commit, file.after_blob.as_deref(), file.after_bytes)?;
    Ok(ReviewDeltaFile {
        file,
        baseline_basis,
        before_source,
        after_source,
        baseline_reconstruction: None,
        fallback_reason,
        source_override: None,
    })
}

fn review_delta_git_source(
    commit: &str,
    object_id: Option<&str>,
    byte_len: Option<usize>,
) -> Result<ReviewDeltaSource> {
    match object_id {
        Some(object_id) => {
            ensure!(
                is_object_id(object_id),
                "review delta source has an invalid object id"
            );
            Ok(ReviewDeltaSource::GitObject {
                commit: commit.to_owned(),
                object_id: object_id.to_owned(),
                byte_len,
            })
        }
        None => {
            ensure!(
                byte_len.is_none(),
                "empty review delta source has an unexpected byte length"
            );
            Ok(ReviewDeltaSource::Empty)
        }
    }
}

fn review_file_for_change<'a>(
    review: &'a RepositoryReview,
    change: &GitChange,
) -> Result<&'a ReviewFile> {
    let mut candidates = review
        .files
        .iter()
        .filter(|file| review_file_matches_change(file, change));
    let file = candidates
        .next()
        .context("current Git change is missing from the repository review")?;
    ensure!(
        candidates.next().is_none(),
        "current Git change matches multiple repository review files"
    );
    Ok(file)
}

fn review_file_matches_change(file: &ReviewFile, change: &GitChange) -> bool {
    file.status == change.status
        && file.similarity_percent == change.similarity_percent
        && file.before_path.as_deref()
            == change
                .before_path
                .as_ref()
                .map(|path| path.display.as_str())
        && file.before_path_encoding == change.before_path.as_ref().map(|path| path.encoding)
        && file.after_path.as_deref()
            == change.after_path.as_ref().map(|path| path.display.as_str())
        && file.after_path_encoding == change.after_path.as_ref().map(|path| path.encoding)
        && file.before_mode == change.before_mode
        && file.after_mode == change.after_mode
        && file.before_blob == change.before_blob
        && file.after_blob == change.after_blob
}

fn checkpoint_change_paths(change: &GitChange) -> Vec<GitPath> {
    let mut paths = change
        .before_path
        .iter()
        .chain(change.after_path.iter())
        .cloned()
        .collect::<Vec<_>>();
    paths.dedup();
    paths
}

fn associate_all_checkpoint_paths(
    associated: &mut HashSet<(usize, GitPath)>,
    index: usize,
    change: &GitChange,
) {
    associated.extend(
        checkpoint_change_paths(change)
            .into_iter()
            .map(|path| (index, path)),
    );
}

fn related_checkpoint_paths(
    checkpoint_changes: &[GitChange],
    current: &GitChange,
) -> Vec<(usize, GitPath)> {
    let current_paths = current
        .before_path
        .iter()
        .chain(current.after_path.iter())
        .collect::<HashSet<_>>();
    checkpoint_changes
        .iter()
        .enumerate()
        .flat_map(|(index, checkpoint)| {
            checkpoint_change_paths(checkpoint)
                .into_iter()
                .filter(|path| current_paths.contains(path))
                .map(move |path| (index, path))
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GitTreeEntry {
    mode: String,
    object_id: String,
}

fn same_path_snapshot_change(
    repository: &Path,
    current_base_commit: &str,
    head_commit: &str,
    checkpoint: &GitChange,
) -> Result<Option<GitChange>> {
    if checkpoint.status != FileStatus::Modified {
        return Ok(None);
    }
    let Some(path) = checkpoint.before_path.as_ref() else {
        return Ok(None);
    };
    if checkpoint.after_path.as_ref() != Some(path) || path.encoding != PathEncoding::Utf8 {
        return Ok(None);
    }
    let Some(before) = load_tree_entry(repository, current_base_commit, path)? else {
        return Ok(None);
    };
    let Some(after) = load_tree_entry(repository, head_commit, path)? else {
        return Ok(None);
    };
    Ok(Some(GitChange {
        status: if before.mode == after.mode {
            FileStatus::Modified
        } else {
            FileStatus::TypeChanged
        },
        similarity_percent: None,
        before_path: Some(path.clone()),
        after_path: Some(path.clone()),
        before_mode: Some(before.mode),
        after_mode: Some(after.mode),
        before_blob: Some(before.object_id),
        after_blob: Some(after.object_id),
    }))
}

fn load_tree_entry(
    repository: &Path,
    commit: &str,
    path: &GitPath,
) -> Result<Option<GitTreeEntry>> {
    if path.encoding != PathEncoding::Utf8 {
        return Ok(None);
    }
    let output = git_output_bounded(
        repository,
        &["ls-tree", "-z", "--full-tree", commit, "--", &path.display],
        path.display.len().saturating_add(512),
    )?;
    if output.stdout.is_empty() {
        return Ok(None);
    }
    let record = output
        .stdout
        .strip_suffix(&[0])
        .context("git ls-tree record is missing its NUL terminator")?;
    ensure!(
        !record.contains(&0),
        "git ls-tree returned multiple entries for one file path"
    );
    let tab = record
        .iter()
        .position(|byte| *byte == b'\t')
        .context("git ls-tree record is missing its path separator")?;
    let metadata =
        std::str::from_utf8(&record[..tab]).context("git ls-tree metadata is not valid UTF-8")?;
    let columns = metadata.split_ascii_whitespace().collect::<Vec<_>>();
    ensure!(columns.len() == 3, "unexpected git ls-tree metadata");
    ensure!(
        &record[tab + 1..] == path.display.as_bytes(),
        "git ls-tree returned an unexpected path"
    );
    ensure!(
        matches!(columns[1], "blob" | "commit"),
        "git ls-tree returned an unsupported object type"
    );
    ensure!(
        is_object_id(columns[2]),
        "git ls-tree returned an invalid object id"
    );
    Ok(Some(GitTreeEntry {
        mode: columns[0].to_owned(),
        object_id: columns[2].to_owned(),
    }))
}

struct CheckpointHeadFallback {
    entries: Vec<ReviewDeltaFile>,
    unresolved: Vec<ReviewDeltaUnresolvedChange>,
}

fn checkpoint_head_fallback_entries(
    repository: &Path,
    paths: &[GitPath],
    checkpoint_commit: &str,
    head_commit: &str,
    limits: &VerificationLimits,
    context: &mut ReviewAnalysisContext,
) -> Result<CheckpointHeadFallback> {
    ensure!(
        !paths.is_empty(),
        "checkpoint fallback has no unassociated path"
    );
    context.consume_file_visits(paths.len())?;

    let mut entries = Vec::new();
    let mut unresolved = Vec::new();
    for path in paths {
        if path.encoding != PathEncoding::Utf8 {
            unresolved.push(ReviewDeltaUnresolvedChange {
                path: path.display.clone(),
                path_encoding: path.encoding,
                reason: ReviewDeltaUnresolvedReason::NonUtf8GitPath,
            });
            continue;
        }

        let checkpoint_entry = load_tree_entry(repository, checkpoint_commit, path)?;
        let head_entry = load_tree_entry(repository, head_commit, path)?;
        if checkpoint_entry == head_entry {
            continue;
        }
        let (before_mode, before_blob) = match checkpoint_entry {
            Some(entry) => (Some(entry.mode), Some(entry.object_id)),
            None => (None, None),
        };
        let (after_mode, after_blob) = match head_entry {
            Some(entry) => (Some(entry.mode), Some(entry.object_id)),
            None => (None, None),
        };
        let status = match (&before_blob, &after_blob) {
            (None, Some(_)) => FileStatus::Added,
            (Some(_), None) => FileStatus::Deleted,
            (Some(_), Some(_)) if before_mode == after_mode => FileStatus::Modified,
            (Some(_), Some(_)) => FileStatus::TypeChanged,
            (None, None) => bail!("checkpoint fallback unexpectedly has two empty snapshots"),
        };
        let change = GitChange {
            status,
            similarity_percent: None,
            before_path: before_blob.as_ref().map(|_| path.clone()),
            after_path: after_blob.as_ref().map(|_| path.clone()),
            before_mode,
            after_mode,
            before_blob,
            after_blob,
        };
        context.reserve_sources(repository, std::iter::once(&change), limits)?;
        let mut file = context.analyze_change(repository, change, limits)?;
        file.checkpoint_state = Some(CheckpointState::NeedsReviewNow);
        file.reason.push_str(
            "; the reviewed change no longer has a current PR identity and could not be rebased exactly, so this conservative checkpoint-to-head fallback may include upstream edits",
        );
        entries.push(review_delta_git_entry(
            file,
            ReviewDeltaBaselineBasis::CheckpointHeadFallback,
            checkpoint_commit,
            head_commit,
            Some(ReviewDeltaFallbackReason::UnsupportedChange),
        )?);
    }
    Ok(CheckpointHeadFallback {
        entries,
        unresolved,
    })
}

fn reconstructed_delta_entry(
    current_file: ReviewFile,
    current_change: &GitChange,
    reconstruction: ReconstructedReviewBaseline,
    head_commit: &str,
    limits: &VerificationLimits,
    retain_source_override: bool,
) -> Result<ReviewDeltaFile> {
    let path = current_change
        .after_path
        .as_ref()
        .context("reconstructed review delta is missing its current path")?;
    let after_blob = current_change
        .after_blob
        .clone()
        .context("reconstructed review delta is missing its head blob")?;
    let after_mode = current_change
        .after_mode
        .clone()
        .context("reconstructed review delta is missing its head mode")?;
    let before_mode = current_change
        .before_mode
        .clone()
        .context("reconstructed review delta is missing its baseline mode")?;
    let before_bytes = reconstruction.baseline.len();
    let after_bytes = reconstruction.current_after.len();
    let mut file = ReviewFile {
        status: FileStatus::Modified,
        similarity_percent: None,
        before_path: Some(path.display.clone()),
        before_path_encoding: Some(path.encoding),
        after_path: Some(path.display.clone()),
        after_path_encoding: Some(path.encoding),
        before_mode: Some(before_mode),
        after_mode: Some(after_mode),
        before_blob: None,
        after_blob: Some(after_blob.clone()),
        before_bytes: Some(before_bytes),
        after_bytes: Some(after_bytes),
        line_change_envelope: line_changes(&reconstruction.baseline, &reconstruction.current_after),
        language: None,
        priority: ReviewPriority::ReviewFirst,
        lane: ReviewLane::ReviewFirst,
        checkpoint_state: Some(CheckpointState::NeedsReviewNow),
        checkpoint_match_basis: None,
        reason: String::new(),
        evidence: None,
    };
    analyze_materialized_review_delta(
        &mut file,
        &reconstruction.baseline,
        &reconstruction.current_after,
        limits,
    )?;
    if current_file.checkpoint_state == Some(CheckpointState::UnchangedSinceCheckpoint) {
        bail!("reconstructed review delta was created for a carried current change");
    }

    let baseline_hash = blake3::hash(&reconstruction.baseline).to_hex().to_string();
    ensure!(
        baseline_hash == reconstruction.evidence.reconstructed_blake3,
        "reconstructed review baseline digest changed"
    );
    let source_override = retain_source_override.then_some(ReviewFileSources {
        before: reconstruction.baseline,
        after: reconstruction.current_after,
    });
    Ok(ReviewDeltaFile {
        file,
        baseline_basis: ReviewDeltaBaselineBasis::ReconstructedReviewBaseline,
        before_source: ReviewDeltaSource::ReconstructedBytes {
            blake3: baseline_hash,
            byte_len: before_bytes,
        },
        after_source: ReviewDeltaSource::GitObject {
            commit: head_commit.to_owned(),
            object_id: after_blob,
            byte_len: Some(after_bytes),
        },
        baseline_reconstruction: Some(reconstruction.evidence),
        fallback_reason: None,
        source_override,
    })
}

fn analyze_materialized_review_delta(
    file: &mut ReviewFile,
    before: &[u8],
    after: &[u8],
    limits: &VerificationLimits,
) -> Result<()> {
    ensure!(
        before.len() <= limits.max_source_bytes && after.len() <= limits.max_source_bytes,
        "reconstructed review delta exceeds the per-file source limit"
    );
    if file.before_path_encoding != Some(PathEncoding::Utf8)
        || file.after_path_encoding != Some(PathEncoding::Utf8)
    {
        file.lane = ReviewLane::Unverified;
        file.reason =
            "review delta uses a reconstructed baseline, but the Git path is not UTF-8".to_owned();
        return Ok(());
    }
    let before_path = Path::new(
        file.before_path
            .as_deref()
            .context("reconstructed review delta is missing its before path")?,
    );
    let after_path = Path::new(
        file.after_path
            .as_deref()
            .context("reconstructed review delta is missing its after path")?,
    );
    let before_language = match Language::detect(before_path) {
        Ok(language) => language,
        Err(error) => {
            file.lane = ReviewLane::Unverified;
            file.reason = format!(
                "review delta uses a reconstructed baseline; before path has no supported parser: {error}"
            );
            return Ok(());
        }
    };
    let after_language = match Language::detect(after_path) {
        Ok(language) => language,
        Err(error) => {
            file.lane = ReviewLane::Unverified;
            file.reason = format!(
                "review delta uses a reconstructed baseline; after path has no supported parser: {error}"
            );
            return Ok(());
        }
    };
    if before_language != after_language {
        file.lane = ReviewLane::Unverified;
        file.reason = format!(
            "review delta uses a reconstructed baseline; file language changed from {before_language:?} to {after_language:?}"
        );
        return Ok(());
    }
    file.language = Some(before_language);

    let report = match analyze_bytes_with_limits(
        before.to_vec(),
        after.to_vec(),
        file.before_path.clone().context("missing before path")?,
        file.after_path.clone().context("missing after path")?,
        before_language,
        limits,
    ) {
        Ok(report) => report,
        Err(error) => {
            file.lane = ReviewLane::Unverified;
            file.reason = format!(
                "review delta uses a reconstructed baseline; structural analysis did not complete: {error:#}"
            );
            return Ok(());
        }
    };
    let encoded = serde_json::to_vec(&report).context("failed to encode per-file evidence")?;
    if encoded.len() > limits.max_report_bytes {
        file.lane = ReviewLane::Unverified;
        file.reason = format!(
            "review delta uses a reconstructed baseline; per-file evidence exceeds the {} byte report limit",
            limits.max_report_bytes
        );
        return Ok(());
    }
    file.lane = classify_report(&report);
    file.priority = classify_priority(&report);
    file.reason = match file.lane {
        ReviewLane::SyntaxPreserved => "review delta against a bidirectionally reconstructed baseline; the Tree-sitter representation matches under StrataDiff's syntax_equal predicate, while bytes differ and behavior was not checked".to_owned(),
        ReviewLane::ReviewFirst if !report.ambiguities.is_empty() => "review delta against a bidirectionally reconstructed baseline; CST syntax is unchanged, but correspondence ambiguity remains in review first".to_owned(),
        ReviewLane::ReviewFirst => "review delta against a bidirectionally reconstructed baseline; the single-file patch rebuilt the current head bytes exactly and the structural delta remains in review first".to_owned(),
        ReviewLane::ContentPreserved | ReviewLane::Unverified => {
            unreachable!("paired structural analysis produces only review or syntax lanes")
        }
    };
    file.evidence = Some(evidence(&report, &encoded));
    Ok(())
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

fn discover_git_change_output(
    repository: &Path,
    base_commit: &str,
    head_commit: &str,
    max_stdout_bytes: usize,
) -> Result<CapturedGitOutput> {
    git_output_bounded_capture(
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
        max_stdout_bytes,
    )
}

fn parse_git_change_output(diff: CapturedGitOutput) -> Result<Vec<GitChange>> {
    ensure!(
        !diff.stderr_exceeded,
        "git diagnostics exceed the 65536 byte limit"
    );
    if !diff.output.status.success() {
        bail!(
            "git diff failed with {}: {}",
            diff.output.status,
            String::from_utf8_lossy(&diff.output.stderr).trim()
        );
    }
    ensure!(
        allowed_raw_diff_diagnostics(&diff.output.stderr),
        "git diff produced diagnostics: {}",
        String::from_utf8_lossy(&diff.output.stderr).trim()
    );
    let changes = parse_raw_diff(&diff.output.stdout, MAX_REVIEW_FILES * 2)?;
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

struct ReconstructedReviewBaseline {
    baseline: Vec<u8>,
    current_after: Vec<u8>,
    evidence: ReviewBaselineReconstruction,
}

enum BaselineReconstructionOutcome {
    Reconstructed(Box<ReconstructedReviewBaseline>),
    Unavailable(ReviewDeltaFallbackReason),
}

fn independent_four_way_replay_matches(
    repository: &Path,
    checkpoint: &GitChange,
    current: &GitChange,
    limits: &VerificationLimits,
    context: &mut ReviewAnalysisContext,
) -> Result<bool> {
    Ok(
        match context.reconstruct_review_baseline(repository, checkpoint, current, limits)? {
            BaselineReconstructionOutcome::Reconstructed(reconstruction) => {
                reconstruction.baseline == reconstruction.current_after
            }
            BaselineReconstructionOutcome::Unavailable(_) => false,
        },
    )
}

fn reconstruct_review_baseline(
    repository: &Path,
    checkpoint: &GitChange,
    current: &GitChange,
    limits: &VerificationLimits,
    blob_loader: &mut BlobLoader,
) -> Result<BaselineReconstructionOutcome> {
    if !replay_candidate_metadata_matches(checkpoint, current) {
        return Ok(BaselineReconstructionOutcome::Unavailable(
            ReviewDeltaFallbackReason::UnsupportedChange,
        ));
    }
    let Some((checkpoint_before, checkpoint_after)) =
        load_replay_blob_pair(repository, checkpoint, limits, blob_loader)?
    else {
        return Ok(BaselineReconstructionOutcome::Unavailable(
            ReviewDeltaFallbackReason::SourceUnavailable,
        ));
    };
    let Some((current_before, current_after)) =
        load_replay_blob_pair(repository, current, limits, blob_loader)?
    else {
        return Ok(BaselineReconstructionOutcome::Unavailable(
            ReviewDeltaFallbackReason::SourceUnavailable,
        ));
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
        return Ok(BaselineReconstructionOutcome::Unavailable(
            ReviewDeltaFallbackReason::BinaryNul,
        ));
    }

    let reviewed_patch = create_patch(&checkpoint_before, &checkpoint_after);
    let upstream_patch = create_patch(&checkpoint_before, &current_before);
    if patches_interact(&reviewed_patch, &upstream_patch) {
        return Ok(BaselineReconstructionOutcome::Unavailable(
            ReviewDeltaFallbackReason::OverlapOrAdjacent,
        ));
    }
    let Some(reviewed_on_current) = translate_patch(&reviewed_patch, &upstream_patch) else {
        return Ok(BaselineReconstructionOutcome::Unavailable(
            ReviewDeltaFallbackReason::TranslationFailed,
        ));
    };
    let Some(upstream_on_reviewed) = translate_patch(&upstream_patch, &reviewed_patch) else {
        return Ok(BaselineReconstructionOutcome::Unavailable(
            ReviewDeltaFallbackReason::TranslationFailed,
        ));
    };

    let reviewed_result = apply_patch(&current_before, &reviewed_on_current)?;
    let upstream_result = apply_patch(&checkpoint_after, &upstream_on_reviewed)?;
    if reviewed_result != upstream_result {
        return Ok(BaselineReconstructionOutcome::Unavailable(
            ReviewDeltaFallbackReason::ReplayOrdersMismatch,
        ));
    }
    if reviewed_result.len() > limits.max_source_bytes {
        return Ok(BaselineReconstructionOutcome::Unavailable(
            ReviewDeltaFallbackReason::SourceUnavailable,
        ));
    }

    let reviewed_hash = blake3::hash(&reviewed_result).to_hex().to_string();
    let upstream_hash = blake3::hash(&upstream_result).to_hex().to_string();
    let evidence = ReviewBaselineReconstruction {
        algorithm: "bidirectional_noninteracting_byte_replay_v1".to_owned(),
        old_base_blob: checkpoint
            .before_blob
            .clone()
            .context("replay candidate is missing its old-base blob")?,
        reviewed_blob: checkpoint
            .after_blob
            .clone()
            .context("replay candidate is missing its reviewed blob")?,
        current_base_blob: current
            .before_blob
            .clone()
            .context("replay candidate is missing its current-base blob")?,
        reviewed_on_current_base_blake3: reviewed_hash.clone(),
        upstream_on_checkpoint_blake3: upstream_hash,
        reconstructed_blake3: reviewed_hash,
        byte_len: reviewed_result.len(),
    };
    Ok(BaselineReconstructionOutcome::Reconstructed(Box::new(
        ReconstructedReviewBaseline {
            baseline: reviewed_result,
            current_after,
            evidence,
        },
    )))
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

pub fn github_workflow_annotations(review: &RepositoryReview) -> String {
    if review.checkpoint.is_none() {
        return "::error title=Review checkpoint required::No completed review checkpoint was resolved; the complete PR remains in review.\n".to_owned();
    }

    let needs_review: Vec<_> = review
        .files
        .iter()
        .filter(|file| file.checkpoint_state == Some(CheckpointState::NeedsReviewNow))
        .collect();
    let mut output = String::new();
    for file in needs_review.iter().take(MAX_GITHUB_WORKFLOW_ANNOTATIONS) {
        let message = "This current PR change has no exact-identity or non-interacting four-way carry from the reviewed checkpoint.";
        if let Some(path) = github_annotation_path(file) {
            output.push_str(&format!(
                "::error file={},title=Review required after checkpoint::{}\n",
                github_command_property(path),
                github_command_data(message)
            ));
        } else {
            output.push_str(&format!(
                "::error title=Review required after checkpoint::{} needs review. {}\n",
                github_command_data(&file.display_path()),
                github_command_data(message)
            ));
        }
    }
    let omitted = needs_review
        .len()
        .saturating_sub(MAX_GITHUB_WORKFLOW_ANNOTATIONS);
    if omitted > 0 {
        output.push_str(&format!(
            "::notice title=Additional review residue::{omitted} additional current PR {} need review; see the step summary and JSON report.\n",
            file_word(omitted)
        ));
    }
    output
}

pub fn github_review_delta_annotations(delta: &ReviewDelta) -> String {
    let mut output = String::new();
    for entry in delta.entries.iter().take(MAX_GITHUB_WORKFLOW_ANNOTATIONS) {
        let message = match entry.baseline_basis {
            ReviewDeltaBaselineBasis::ReconstructedReviewBaseline => {
                "This file differs from the exact review baseline reconstructed across the base change."
            }
            ReviewDeltaBaselineBasis::CurrentBaseFallback => {
                "An exact review baseline could not be reconstructed; review the complete current-base-to-head file change."
            }
            ReviewDeltaBaselineBasis::CheckpointHeadFallback => {
                "A retired checkpoint change could not be rebased exactly; review this conservative checkpoint-to-head file change."
            }
            ReviewDeltaBaselineBasis::CheckpointSnapshot => {
                "This file changed after the reviewed checkpoint."
            }
            ReviewDeltaBaselineBasis::CurrentBaseNoCheckpointChange => {
                "This current PR file has no corresponding change at the reviewed checkpoint."
            }
        };
        if let Some(path) = github_annotation_path(&entry.file) {
            output.push_str(&format!(
                "::error file={},title=Review required after checkpoint::{}\n",
                github_command_property(path),
                github_command_data(message)
            ));
        } else {
            output.push_str(&format!(
                "::error title=Review required after checkpoint::{} needs review. {}\n",
                github_command_data(&entry.display_path()),
                github_command_data(message)
            ));
        }
    }
    let remaining_slots = MAX_GITHUB_WORKFLOW_ANNOTATIONS.saturating_sub(delta.entries.len());
    for change in delta
        .unresolved_retired_changes
        .iter()
        .take(remaining_slots)
    {
        output.push_str(&format!(
            "::error title=Unresolved retired review change::{} could not be reconstructed or displayed ({}) and still requires review.\n",
            github_command_data(&change.path),
            github_command_data(match change.reason {
                ReviewDeltaUnresolvedReason::NonUtf8GitPath => "non-UTF-8 Git path",
            })
        ));
    }
    let total = delta.summary.needs_review_files;
    let omitted = total.saturating_sub(MAX_GITHUB_WORKFLOW_ANNOTATIONS);
    if omitted > 0 {
        output.push_str(&format!(
            "::notice title=Additional review delta::{omitted} additional {} need review; open the Review Resume Workbench for the complete queue.\n",
            file_word(omitted)
        ));
    }
    output
}

fn github_annotation_path(file: &ReviewFile) -> Option<&str> {
    match (&file.after_path, file.after_path_encoding) {
        (Some(path), Some(PathEncoding::Utf8)) => Some(path),
        _ => match (&file.before_path, file.before_path_encoding) {
            (Some(path), Some(PathEncoding::Utf8)) => Some(path),
            _ => None,
        },
    }
}

fn github_command_data(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn github_command_property(value: &str) -> String {
    github_command_data(value)
        .replace(':', "%3A")
        .replace(',', "%2C")
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

fn is_blake3(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
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
            file.reason =
                "new file; all content remains in the intrinsic review-first pass".to_owned();
            return Ok(file);
        }
        FileStatus::Deleted => {
            file.reason =
                "deleted file; the removal remains in the intrinsic review-first pass".to_owned();
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
    let captured = git_output_bounded_capture(repository, arguments, max_stdout_bytes)?;
    ensure!(
        !captured.stdout_exceeded,
        "git raw diff exceeds the {max_stdout_bytes} byte metadata limit"
    );
    ensure!(
        !captured.stderr_exceeded,
        "git diagnostics exceed the 65536 byte limit"
    );
    if !captured.output.status.success() {
        bail!(
            "git {} failed with {}: {}",
            arguments.join(" "),
            captured.output.status,
            String::from_utf8_lossy(&captured.output.stderr).trim()
        );
    }
    Ok(captured.output)
}

struct CapturedGitOutput {
    output: Output,
    stdout_exceeded: bool,
    stderr_exceeded: bool,
}

fn git_output_bounded_capture(
    repository: &Path,
    arguments: &[&str],
    max_stdout_bytes: usize,
) -> Result<CapturedGitOutput> {
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
    Ok(CapturedGitOutput {
        output: Output {
            status,
            stdout,
            stderr,
        },
        stdout_exceeded,
        stderr_exceeded,
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
    use std::{fs, path::Path, process::Command};

    use crate::{ByteEdit, LosslessPatch};

    use super::{
        BaselineReconstructionOutcome, FileStatus, GitChange, GitChangeIdentity, GitPath,
        MAX_REVIEW_MARKDOWN_BYTES, PathEncoding, RepositoryReview, ReviewAnalysisBudgetExceeded,
        ReviewAnalysisContext, ReviewDeltaBaselineBasis, ReviewFile, ReviewLane, ReviewPriority,
        ReviewSummary, allowed_raw_diff_diagnostics, line_changes, markdown_cell, markdown_code,
        markdown_report, pair_unique_exact_relocations, parse_raw_diff, patches_interact,
        read_bounded, review_git_range_with_analysis, review_git_resume_delta_with_analysis,
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
        git(repository, &["commit", "--quiet", "-m", message]);
        git(repository, &["rev-parse", "HEAD"])
    }

    fn repository_with_one_change(
        before: &[u8],
        after: &[u8],
    ) -> (tempfile::TempDir, String, String) {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "--quiet"]);
        git(
            directory.path(),
            &["config", "user.name", "StrataDiff Test"],
        );
        git(
            directory.path(),
            &["config", "user.email", "stratadiff@example.test"],
        );
        fs::write(directory.path().join("change.txt"), before).unwrap();
        let base = commit(directory.path(), "base");
        fs::write(directory.path().join("change.txt"), after).unwrap();
        let head = commit(directory.path(), "head");
        (directory, base, head)
    }

    fn replay_history() -> (tempfile::TempDir, String, String, String) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "--quiet"]);
        git(root, &["config", "user.name", "StrataDiff Test"]);
        git(root, &["config", "user.email", "stratadiff@example.test"]);
        let padding = "x".repeat(4 * 1024);
        fs::write(
            root.join("shared.py"),
            format!("title = 'old'\nstable = '{padding}'\nreviewed = 0\nfollowup = 0\n"),
        )
        .unwrap();
        let original_base = commit(root, "original base");
        fs::write(
            root.join("shared.py"),
            format!("title = 'old'\nstable = '{padding}'\nreviewed = 1\nfollowup = 0\n"),
        )
        .unwrap();
        let checkpoint = commit(root, "reviewed checkpoint");
        git(root, &["checkout", "--quiet", &original_base]);
        fs::write(
            root.join("shared.py"),
            format!("title = 'new'\nstable = '{padding}'\nreviewed = 0\nfollowup = 0\n"),
        )
        .unwrap();
        let current_base = commit(root, "advanced base");
        fs::write(
            root.join("shared.py"),
            format!("title = 'new'\nstable = '{padding}'\nreviewed = 1\nfollowup = 1\n"),
        )
        .unwrap();
        let head = commit(root, "rebased head");
        (directory, current_base, checkpoint, head)
    }

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
    fn four_way_replay_treats_adjacent_edits_as_interacting() {
        let patch = |start, end| LosslessPatch {
            algorithm: "test".to_owned(),
            edits: vec![ByteEdit {
                old_start: start,
                old_end: end,
                replacement_base64: String::new(),
            }],
        };
        assert!(patches_interact(&patch(1, 4), &patch(4, 7)));
        assert!(!patches_interact(&patch(1, 3), &patch(4, 7)));
    }

    #[test]
    fn bounded_context_rejects_the_6001st_file_visit_before_blob_analysis() {
        let (directory, base, head) = repository_with_one_change(b"before\n", b"after\n");
        let mut context = ReviewAnalysisContext::bounded(6_000, 1024 * 1024);
        context.consume_file_visits(6_000).unwrap();

        let error =
            review_git_range_with_analysis(directory.path(), &base, &head, None, &mut context)
                .unwrap_err();

        assert!(
            error
                .downcast_ref::<ReviewAnalysisBudgetExceeded>()
                .is_some()
        );
        assert!(error.to_string().contains("file visit"));
        assert!(context.blob_loader.bytes.is_empty());
        assert!(context.analyzed_files.is_empty());
    }

    #[test]
    fn source_byte_limit_fails_before_blob_content_is_read() {
        let before = vec![b'a'; 8 * 1024];
        let after = vec![b'b'; 8 * 1024];
        let (directory, base, head) = repository_with_one_change(&before, &after);
        let mut context = ReviewAnalysisContext::bounded(10, before.len() + after.len() - 1);

        let error =
            review_git_range_with_analysis(directory.path(), &base, &head, None, &mut context)
                .unwrap_err();

        assert!(error.to_string().contains("source byte budget exceeded"));
        assert!(context.blob_loader.bytes.is_empty());
        assert!(context.analyzed_files.is_empty());
    }

    #[test]
    fn processed_source_budget_counts_same_oids_for_distinct_paths() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "--quiet"]);
        git(root, &["config", "user.name", "StrataDiff Test"]);
        git(root, &["config", "user.email", "stratadiff@example.test"]);
        let before = vec![b'a'; 8 * 1024];
        let after = vec![b'b'; 8 * 1024];
        fs::write(root.join("a.txt"), &before).unwrap();
        fs::write(root.join("b.txt"), &before).unwrap();
        let base = commit(root, "base");
        fs::write(root.join("a.txt"), &after).unwrap();
        fs::write(root.join("b.txt"), &after).unwrap();
        let head = commit(root, "head");
        let per_identity = before.len() + after.len();
        let mut context = ReviewAnalysisContext::bounded(10, per_identity);

        let error =
            review_git_range_with_analysis(root, &base, &head, None, &mut context).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("processed source byte budget exceeded")
        );
        assert_eq!(context.source_oids.len(), 2);
        assert_eq!(context.analyzed_files.len(), 1);
        assert_eq!(context.processed_source_bytes, per_identity);
    }

    #[test]
    fn repeated_unavailable_diffs_consume_the_query_budget() {
        let (directory, base, _) = repository_with_one_change(b"before\n", b"after\n");
        let missing = "f".repeat(40);
        let mut context = ReviewAnalysisContext::bounded(2, 1024 * 1024);

        for _ in 0..2 {
            let error = context
                .discover_changes(directory.path(), &base, &missing)
                .unwrap_err();
            assert!(
                error
                    .downcast_ref::<ReviewAnalysisBudgetExceeded>()
                    .is_none()
            );
        }
        let error = context
            .discover_changes(directory.path(), &base, &missing)
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<ReviewAnalysisBudgetExceeded>()
                .is_some()
        );
        assert!(error.to_string().contains("diff query budget exceeded"));
        assert_eq!(context.diff_queries, 2);
    }

    #[test]
    fn repeated_reconstruction_counts_source_bytes_by_occurrence() {
        let path = GitPath {
            display: "shared.py".to_owned(),
            encoding: PathEncoding::Utf8,
        };
        let checkpoint = GitChange {
            status: FileStatus::Modified,
            similarity_percent: None,
            before_path: Some(path.clone()),
            after_path: Some(path.clone()),
            before_mode: Some("100644".to_owned()),
            after_mode: Some("100644".to_owned()),
            before_blob: Some("1".repeat(40)),
            after_blob: Some("2".repeat(40)),
        };
        let current = GitChange {
            before_blob: Some("3".repeat(40)),
            after_blob: Some("4".repeat(40)),
            ..checkpoint.clone()
        };
        let sources = [
            (
                "1".repeat(40),
                b"title = 'old'\nstable = 0\nreviewed = 0\n".to_vec(),
            ),
            (
                "2".repeat(40),
                b"title = 'old'\nstable = 0\nreviewed = 1\n".to_vec(),
            ),
            (
                "3".repeat(40),
                b"title = 'new'\nstable = 0\nreviewed = 0\n".to_vec(),
            ),
            (
                "4".repeat(40),
                b"title = 'new'\nstable = 0\nreviewed = 1\n".to_vec(),
            ),
        ];
        let work_bytes = sources.iter().map(|(_, bytes)| bytes.len()).sum();
        let mut context = ReviewAnalysisContext::bounded(10, work_bytes);
        for (object_id, bytes) in sources {
            context
                .blob_loader
                .sizes
                .insert(object_id.clone(), bytes.len());
            context.blob_loader.bytes.insert(object_id, bytes);
        }

        assert!(matches!(
            context
                .reconstruct_review_baseline(
                    Path::new("/unused-cached-repository"),
                    &checkpoint,
                    &current,
                    &crate::VerificationLimits::default(),
                )
                .unwrap(),
            BaselineReconstructionOutcome::Reconstructed(_)
        ));
        let error = match context.reconstruct_review_baseline(
            Path::new("/unused-cached-repository"),
            &checkpoint,
            &current,
            &crate::VerificationLimits::default(),
        ) {
            Ok(_) => panic!("second reconstruction unexpectedly fit the work budget"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("processed source byte budget exceeded")
        );
        assert_eq!(context.processed_source_bytes, work_bytes);
    }

    #[test]
    fn replay_budget_error_is_not_downgraded_to_needs_review() {
        let (directory, current_base, checkpoint, head) = replay_history();
        let root = directory.path();
        let commits = [&checkpoint, &current_base, &head];
        let mut object_ids = commits
            .iter()
            .map(|commit| git(root, &["rev-parse", &format!("{commit}:shared.py")]))
            .collect::<Vec<_>>();
        let original_base = git(root, &["merge-base", &checkpoint, &current_base]);
        object_ids.push(git(
            root,
            &["rev-parse", &format!("{original_base}:shared.py")],
        ));
        object_ids.sort();
        object_ids.dedup();
        let source_bytes = object_ids
            .iter()
            .map(|object_id| {
                git(root, &["cat-file", "-s", object_id])
                    .parse::<usize>()
                    .unwrap()
            })
            .sum();
        let mut context = ReviewAnalysisContext::bounded(10, source_bytes);
        context.consume_processed_source_bytes(1).unwrap();

        let error = review_git_range_with_analysis(
            root,
            &current_base,
            &head,
            Some(&checkpoint),
            &mut context,
        )
        .unwrap_err();

        assert!(
            error
                .downcast_ref::<ReviewAnalysisBudgetExceeded>()
                .is_some()
        );
        assert!(
            error
                .to_string()
                .contains("processed source byte budget exceeded")
        );
        assert!(context.analyzed_files.is_empty());
        assert!(context.blob_loader.bytes.is_empty());
    }

    #[test]
    fn bounded_resume_reuses_caches_and_does_not_retain_reconstructed_sources() {
        let (directory, current_base, checkpoint, head) = replay_history();
        let root = directory.path();
        let mut context = ReviewAnalysisContext::bounded(100, 1024 * 1024);
        let review = review_git_range_with_analysis(
            root,
            &current_base,
            &head,
            Some(&checkpoint),
            &mut context,
        )
        .unwrap();
        let cache_sizes = (
            context.changes.len(),
            context.analyzed_files.len(),
            context.blob_loader.bytes.len(),
            context.source_oids.len(),
        );
        let repeated = review_git_range_with_analysis(
            root,
            &current_base,
            &head,
            Some(&checkpoint),
            &mut context,
        )
        .unwrap();
        assert_eq!(repeated.files, review.files);
        assert_eq!(
            (
                context.changes.len(),
                context.analyzed_files.len(),
                context.blob_loader.bytes.len(),
                context.source_oids.len(),
            ),
            cache_sizes
        );

        let delta = review_git_resume_delta_with_analysis(root, &review, &mut context).unwrap();
        let reconstructed = delta
            .entries
            .iter()
            .find(|entry| {
                entry.baseline_basis == ReviewDeltaBaselineBasis::ReconstructedReviewBaseline
            })
            .unwrap();
        assert!(reconstructed.source_override.is_none());
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
