use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

const MAX_SIGNALS: usize = 10_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSignalCollectionStatus {
    Complete,
    Gap,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CandidateSignalSurface {
    pub collection: CandidateSignalCollectionStatus,
    pub success: u64,
    pub other: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CandidateSignals {
    pub sha: String,
    pub check_runs: CandidateSignalSurface,
    pub commit_statuses: CandidateSignalSurface,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CandidateQueueEntry {
    pub id: String,
    pub state: String,
    pub position: u64,
    pub base_sha: String,
    pub head_sha: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CandidateTargetIdentity {
    pub state: String,
    pub base_sha: String,
    pub head_sha: String,
    pub is_merge_queue_enabled: bool,
    pub is_in_merge_queue: bool,
    pub potential_merge_sha: Option<String>,
    pub queue_entry: Option<CandidateQueueEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CandidateSelectionInput {
    pub before: CandidateTargetIdentity,
    pub after: CandidateTargetIdentity,
    pub signals: Vec<CandidateSignals>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSelectionStatus {
    Selected,
    Inconclusive,
    Retry,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    PrHead,
    TestMerge,
    MergeGroup,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSelectionReason {
    TestMergeSignal,
    TestMergeEmpty,
    PotentialMergeUnavailable,
    QueueEntryCandidate,
    QueueEntryUnavailable,
    SignalSurfaceGap,
    TargetDrift,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CandidateSelection {
    pub status: CandidateSelectionStatus,
    pub candidate_kind: Option<CandidateKind>,
    pub sha: Option<String>,
    pub base_sha: Option<String>,
    pub queue_entry_id: Option<String>,
    pub reason_code: CandidateSelectionReason,
    pub ignored_signal_shas: Vec<String>,
}

fn valid_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn bounded_text(value: &str, maximum: usize, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= maximum,
        "{label} must contain between 1 and {maximum} bytes"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "{label} must not contain control characters"
    );
    Ok(())
}

fn validate_surface(surface: &CandidateSignalSurface, label: &str) -> Result<()> {
    ensure!(
        surface.success.checked_add(surface.other).is_some(),
        "{label} signal count overflow"
    );
    Ok(())
}

fn validate_queue_entry(
    entry: &CandidateQueueEntry,
    target: &CandidateTargetIdentity,
) -> Result<()> {
    bounded_text(&entry.id, 255, "merge-queue entry ID")?;
    ensure!(
        matches!(
            entry.state.as_str(),
            "QUEUED" | "AWAITING_CHECKS" | "MERGEABLE" | "UNMERGEABLE" | "LOCKED"
        ),
        "merge-queue entry state is unsupported"
    );
    ensure!(
        entry.position > 0,
        "merge-queue entry position must be positive"
    );
    ensure!(
        valid_sha(&entry.base_sha) && valid_sha(&entry.head_sha),
        "merge-queue entry commits must be lowercase full Git object IDs"
    );
    ensure!(
        entry.base_sha != entry.head_sha,
        "merge-queue entry base and head must differ"
    );
    ensure!(
        entry.head_sha != target.head_sha,
        "merge-queue candidate must differ from the PR head"
    );
    Ok(())
}

fn validate_target(target: &CandidateTargetIdentity, label: &str) -> Result<()> {
    ensure!(target.state == "open", "{label} pull request must be open");
    ensure!(
        valid_sha(&target.base_sha) && valid_sha(&target.head_sha),
        "{label} PR commits must be lowercase full Git object IDs"
    );
    ensure!(
        target.base_sha != target.head_sha,
        "{label} PR base and head must differ"
    );
    if let Some(potential_merge_sha) = &target.potential_merge_sha {
        ensure!(
            valid_sha(potential_merge_sha),
            "{label} test-merge commit must be a lowercase full Git object ID"
        );
        ensure!(
            potential_merge_sha != &target.head_sha,
            "{label} test-merge commit must differ from the PR head"
        );
    }
    ensure!(
        !target.is_in_merge_queue || target.is_merge_queue_enabled,
        "{label} cannot be queued when its merge queue is disabled"
    );
    ensure!(
        target.is_in_merge_queue || target.queue_entry.is_none(),
        "{label} has a queue entry without active queue membership"
    );
    if let Some(entry) = &target.queue_entry {
        validate_queue_entry(entry, target)?;
    }
    Ok(())
}

fn has_signal(surface: &CandidateSignalSurface) -> bool {
    surface.success > 0 || surface.other > 0
}

fn observed_signal(signal: &CandidateSignals) -> bool {
    has_signal(&signal.check_runs) || has_signal(&signal.commit_statuses)
}

fn ignored_signal_shas(signals: &[CandidateSignals], selected_sha: Option<&str>) -> Vec<String> {
    let mut ignored: Vec<_> = signals
        .iter()
        .filter(|signal| {
            observed_signal(signal) && selected_sha.is_none_or(|sha| signal.sha != sha)
        })
        .map(|signal| signal.sha.clone())
        .collect();
    ignored.sort();
    ignored
}

fn unresolved(
    status: CandidateSelectionStatus,
    reason_code: CandidateSelectionReason,
    signals: &[CandidateSignals],
) -> CandidateSelection {
    CandidateSelection {
        status,
        candidate_kind: None,
        sha: None,
        base_sha: None,
        queue_entry_id: None,
        reason_code,
        ignored_signal_shas: ignored_signal_shas(signals, None),
    }
}

fn selected(
    candidate_kind: CandidateKind,
    sha: &str,
    base_sha: Option<&str>,
    queue_entry_id: Option<&str>,
    reason_code: CandidateSelectionReason,
    signals: &[CandidateSignals],
) -> CandidateSelection {
    CandidateSelection {
        status: CandidateSelectionStatus::Selected,
        candidate_kind: Some(candidate_kind),
        sha: Some(sha.to_owned()),
        base_sha: base_sha.map(str::to_owned),
        queue_entry_id: queue_entry_id.map(str::to_owned),
        reason_code,
        ignored_signal_shas: ignored_signal_shas(signals, Some(sha)),
    }
}

pub fn select_candidate(input: &CandidateSelectionInput) -> Result<CandidateSelection> {
    validate_target(&input.before, "initial")?;
    validate_target(&input.after, "final")?;
    ensure!(
        input.signals.len() <= MAX_SIGNALS,
        "candidate signal limit exceeded"
    );
    let mut signal_shas = BTreeSet::new();
    for signal in &input.signals {
        ensure!(
            valid_sha(&signal.sha),
            "signal SHA must be a lowercase full Git object ID"
        );
        ensure!(
            signal_shas.insert(signal.sha.as_str()),
            "candidate signals contain a duplicate SHA"
        );
        validate_surface(&signal.check_runs, "Check Run surface")?;
        validate_surface(&signal.commit_statuses, "commit-status surface")?;
    }

    if input.before != input.after {
        return Ok(unresolved(
            CandidateSelectionStatus::Retry,
            CandidateSelectionReason::TargetDrift,
            &input.signals,
        ));
    }

    let target = &input.after;
    if target.is_in_merge_queue {
        let Some(entry) = &target.queue_entry else {
            return Ok(unresolved(
                CandidateSelectionStatus::Inconclusive,
                CandidateSelectionReason::QueueEntryUnavailable,
                &input.signals,
            ));
        };
        return Ok(selected(
            CandidateKind::MergeGroup,
            &entry.head_sha,
            Some(&entry.base_sha),
            Some(&entry.id),
            CandidateSelectionReason::QueueEntryCandidate,
            &input.signals,
        ));
    }

    let Some(test_merge_sha) = &target.potential_merge_sha else {
        return Ok(unresolved(
            CandidateSelectionStatus::Inconclusive,
            CandidateSelectionReason::PotentialMergeUnavailable,
            &input.signals,
        ));
    };
    let Some(test_merge_signals) = input
        .signals
        .iter()
        .find(|signal| signal.sha == *test_merge_sha)
    else {
        return Ok(unresolved(
            CandidateSelectionStatus::Inconclusive,
            CandidateSelectionReason::SignalSurfaceGap,
            &input.signals,
        ));
    };
    if observed_signal(test_merge_signals) {
        return Ok(selected(
            CandidateKind::TestMerge,
            test_merge_sha,
            Some(&target.base_sha),
            None,
            CandidateSelectionReason::TestMergeSignal,
            &input.signals,
        ));
    }
    if test_merge_signals.check_runs.collection != CandidateSignalCollectionStatus::Complete
        || test_merge_signals.commit_statuses.collection
            != CandidateSignalCollectionStatus::Complete
    {
        return Ok(unresolved(
            CandidateSelectionStatus::Inconclusive,
            CandidateSelectionReason::SignalSurfaceGap,
            &input.signals,
        ));
    }
    Ok(selected(
        CandidateKind::PrHead,
        &target.head_sha,
        None,
        None,
        CandidateSelectionReason::TestMergeEmpty,
        &input.signals,
    ))
}
