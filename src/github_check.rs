//! Deterministic GitHub Check Run request bodies for verified review coverage.
//!
//! This module does not publish checks. GitHub only grants Check Run write access to
//! GitHub Apps, so callers must submit the returned payload with an installation token
//! that has `checks:write`; an ordinary personal access token is not sufficient.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use axum::http::Uri;
use serde::Serialize;

use crate::{
    codeowners::CodeownerIdentity,
    coverage::{
        CoverageFileScope, FileCoverage, FileCoverageState, ReviewCoveragePassport,
        verify_review_coverage_passport,
    },
    review::PathEncoding,
};

pub const MAX_GITHUB_CHECK_RUN_ANNOTATIONS: usize = 50;
pub const MAX_GITHUB_CHECK_RUN_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const REVIEW_COVERAGE_CHECK_NAME: &str = "StrataDiff review coverage";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct GithubCheckRunPayload {
    pub name: String,
    pub head_sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details_url: Option<String>,
    pub external_id: String,
    pub status: GithubCheckRunStatus,
    pub conclusion: GithubCheckRunConclusion,
    pub output: GithubCheckRunOutput,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubCheckRunStatus {
    Completed,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubCheckRunConclusion {
    Success,
    Failure,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct GithubCheckRunOutput {
    pub title: String,
    pub summary: String,
    pub annotations: Vec<GithubCheckRunAnnotation>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct GithubCheckRunAnnotation {
    pub path: String,
    pub start_line: u64,
    pub end_line: u64,
    pub annotation_level: GithubCheckRunAnnotationLevel,
    pub title: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubCheckRunAnnotationLevel {
    Failure,
}

/// Verifies the signed passport against exact local Git objects and then binds the request
/// body to a separately observed live pull-request base and head. No network request is made.
pub fn build_github_check_run_payload(
    repository: &Path,
    passport: &ReviewCoveragePassport,
    trusted_receiver_public_key: &str,
    expected_live_base: &str,
    expected_live_head: &str,
    details_url: Option<&str>,
) -> Result<GithubCheckRunPayload> {
    verify_review_coverage_passport(repository, passport, trusted_receiver_public_key)?;
    ensure!(
        passport.body.protected_base_commit == expected_live_base,
        "live pull request base changed: expected {}, verified passport binds {}",
        expected_live_base,
        passport.body.protected_base_commit
    );
    ensure!(
        passport.body.head_commit == expected_live_head,
        "live pull request head changed: expected {}, verified passport binds {}",
        expected_live_head,
        passport.body.head_commit
    );

    let details_url = details_url.map(validate_details_url).transpose()?;
    let mut annotation_candidates = passport
        .body
        .files
        .iter()
        .filter(|file| file.state != FileCoverageState::Covered)
        .filter_map(annotation_for_file)
        .collect::<Vec<_>>();
    annotation_candidates.sort_by(|left, right| left.path.cmp(&right.path));
    let emitted_annotations = annotation_candidates
        .len()
        .min(MAX_GITHUB_CHECK_RUN_ANNOTATIONS);
    let omitted_annotations = annotation_candidates
        .len()
        .checked_sub(emitted_annotations)
        .context("emitted annotation count exceeds candidate count")?;
    annotation_candidates.truncate(MAX_GITHUB_CHECK_RUN_ANNOTATIONS);

    let uncovered_requirements = passport
        .body
        .summary
        .needs_review_files
        .checked_add(passport.body.summary.blocked_files)
        .context("coverage requirement count overflow")?;
    let addressable_requirements = annotation_candidates
        .len()
        .checked_add(omitted_annotations)
        .context("annotation candidate count overflow")?;
    let check_level_only = uncovered_requirements
        .checked_sub(addressable_requirements)
        .context("annotation candidates exceed uncovered coverage requirements")?;
    let gate_label = if passport.body.summary.gate_passed {
        "PASSED"
    } else {
        "FAILED"
    };
    let summary = format!(
        "### Review coverage {gate_label}\n\n\
         | Requirement | Count |\n\
         | --- | ---: |\n\
         | Covered | {} |\n\
         | Needs review | {} |\n\
         | Blocked | {} |\n\
         | Retired residue | {} |\n\
         | Unresolved retired residue | {} |\n\
         | Total | {} |\n\n\
         Annotations: {} emitted, {} omitted by GitHub's {}-annotation request limit, \
         {} reported at check level only. Retired and non-UTF-8 requirements are never \
         represented as line annotations.\n\n\
         Verified base `{}` and head `{}` before payload generation.",
        passport.body.summary.covered_files,
        passport.body.summary.needs_review_files,
        passport.body.summary.blocked_files,
        passport.body.summary.retired_residue_files,
        passport.body.summary.unresolved_residue,
        passport.body.summary.total_requirements,
        annotation_candidates.len(),
        omitted_annotations,
        MAX_GITHUB_CHECK_RUN_ANNOTATIONS,
        check_level_only,
        passport.body.protected_base_commit,
        passport.body.head_commit,
    );

    Ok(GithubCheckRunPayload {
        name: REVIEW_COVERAGE_CHECK_NAME.to_owned(),
        head_sha: passport.body.head_commit.clone(),
        details_url,
        external_id: format!("review-coverage-v1:{}", passport.attestation.body_sha256),
        status: GithubCheckRunStatus::Completed,
        conclusion: if passport.body.summary.gate_passed {
            GithubCheckRunConclusion::Success
        } else {
            GithubCheckRunConclusion::Failure
        },
        output: GithubCheckRunOutput {
            title: format!("Review coverage {gate_label}"),
            summary,
            annotations: annotation_candidates,
        },
    })
}

fn annotation_for_file(file: &FileCoverage) -> Option<GithubCheckRunAnnotation> {
    if file.scope != CoverageFileScope::CurrentChange
        || file.path_encoding != PathEncoding::Utf8
        || file.change.after_path_encoding != Some(PathEncoding::Utf8)
    {
        return None;
    }
    let path = file.change.after_path.as_ref()?;
    if path != &file.path {
        return None;
    }
    let owners = file
        .matching_rule
        .as_ref()
        .map(|rule| {
            rule.owner_alternatives
                .iter()
                .map(owner_label)
                .collect::<Vec<_>>()
                .join(" or ")
        })
        .filter(|owners| !owners.is_empty())
        .unwrap_or_else(|| "no resolved CODEOWNER".to_owned());
    let (title, action) = match file.state {
        FileCoverageState::NeedsReview => (
            "CODEOWNER review required",
            "No active authorized review carries this exact change",
        ),
        FileCoverageState::Blocked => (
            "Review coverage blocked",
            "Required ownership or checkpoint evidence is incomplete",
        ),
        FileCoverageState::Covered => return None,
    };
    Some(GithubCheckRunAnnotation {
        path: path.clone(),
        start_line: 1,
        end_line: 1,
        annotation_level: GithubCheckRunAnnotationLevel::Failure,
        title: title.to_owned(),
        message: format!("Owners: {owners}. {action}."),
    })
}

fn owner_label(owner: &CodeownerIdentity) -> String {
    match owner {
        CodeownerIdentity::User { login } => format!("@{login}"),
        CodeownerIdentity::Team { organization, slug } => {
            format!("@{organization}/{slug}")
        }
        CodeownerIdentity::Email { address } => address.clone(),
    }
}

fn validate_details_url(value: &str) -> Result<String> {
    ensure!(
        !value.is_empty() && value.len() <= 2_048,
        "details URL is empty or exceeds its byte limit"
    );
    ensure!(
        !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace()),
        "details URL contains whitespace or control characters"
    );
    let uri = value
        .parse::<Uri>()
        .context("details URL is not a valid absolute URI")?;
    ensure!(
        uri.scheme_str() == Some("https"),
        "details URL must use HTTPS"
    );
    let authority = uri
        .authority()
        .context("details URL must contain an authority")?;
    ensure!(
        !authority.host().is_empty() && !authority.as_str().contains('@'),
        "details URL must not contain credentials and must have a host"
    );
    Ok(value.to_owned())
}
