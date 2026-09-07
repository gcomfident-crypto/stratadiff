use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use stratadiff::codeowners::CodeownersPolicy;
use stratadiff::coverage::{
    MAX_REVIEW_COVERAGE_BYTES, ReviewCoveragePassport, build_review_coverage_passport,
    verify_review_coverage_passport,
};
use stratadiff::doctor::{
    DoctorVerdict, evaluate_pull_request_doctor_v3, render_pull_request_doctor_v3_markdown,
};
use stratadiff::github::{
    MAX_GITHUB_COMMIT_OBJECT_BYTES, MAX_GITHUB_REVIEWS_BYTES,
    MAX_GITHUB_REVIEWS_INCLUDED_RESPONSE_BYTES, resolve_github_review_checkpoint,
    resolve_github_review_checkpoint_included_response,
    resolve_github_review_checkpoint_slurp_pages, verify_github_commit_object,
};
use stratadiff::github_check::{
    MAX_GITHUB_CHECK_RUN_PAYLOAD_BYTES, build_github_check_run_payload,
};
use stratadiff::github_ownership::{
    GITHUB_API_VERSION, GithubOwnershipApi, GithubOwnershipApiResponse, GithubOwnershipMediaType,
    MAX_GITHUB_OWNERSHIP_API_RESPONSE_BYTES, collect_github_ownership_snapshot,
    current_utc_timestamp, write_github_ownership_snapshot,
};
use stratadiff::ledger::{
    GithubReviewLedger, GithubWebhookIngest, IngestOutcome, MAX_GITHUB_LEDGER_BYTES,
    MAX_GITHUB_WEBHOOK_BYTES, decode_ed25519_signing_key, ingest_github_webhook,
};
use stratadiff::ownership::{
    GithubOwnershipSnapshot, MAX_OWNERSHIP_SNAPSHOT_BYTES, github_provider_hostname,
};
use stratadiff::readiness::{
    AuditVerdict, evaluate_merge_readiness, render_merge_readiness_markdown,
};
use stratadiff::readiness_audit::{
    GithubPullRequestDoctorApi, GithubReadinessApi, GithubReadinessApiResponse,
    MAX_READINESS_API_RESPONSE_BYTES, MergeReadinessCollection, PullRequestDoctorCollection,
    collect_merge_readiness_snapshot, collect_pull_request_doctor_snapshot_v3,
};
use stratadiff::review::{
    github_review_delta_annotations, github_workflow_annotations, markdown_report,
    review_git_range_with_checkpoint, review_git_resume_delta,
};
use stratadiff::review_cache::{
    MAX_REVIEW_CACHE_JSON_BYTES, ReviewCacheContextBuild, ReviewCachePreflight,
    ReviewCacheReceiptBundle, ReviewCacheReceiptIssue, build_review_cache_context,
    issue_review_cache_receipt, review_cache_preflight,
};
use stratadiff::{
    AmbiguityConstraint, DiffReport, Language, VerificationLimits, analyze_bytes, apply_patch,
    verify_and_replay_report_bytes, verify_report_bytes,
};

mod demo;
mod inbox;
mod process;
mod resume;
mod value_funnel;
mod viewer;

use process::run_bounded_process;

const LEGACY_REPORT_SCHEMA_V1: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v1.schema.json";
const LEGACY_REPORT_SCHEMA_V2: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v2.schema.json";
const BUILD_INFO_SCHEMA: &str = "stratadiff-build-info-v1";
const BUILD_GIT_REVISION: &str = env!("STRATADIFF_BUILD_GIT_REVISION");
const BUILD_GIT_DIRTY: &str = env!("STRATADIFF_BUILD_GIT_DIRTY");
const BUILD_CARGO_LOCK_SHA256: &str = env!("STRATADIFF_BUILD_CARGO_LOCK_SHA256");
const BUILD_PROFILE: &str = env!("STRATADIFF_BUILD_PROFILE");
const BUILD_RUSTC_VERSION: &str = env!("STRATADIFF_BUILD_RUSTC_VERSION");
const THIRD_PARTY_NOTICES: &str = include_str!("../THIRD_PARTY_NOTICES.txt");
const GITHUB_API_HEADER_BYTES: usize = 64 * 1024;
const GITHUB_API_TIMEOUT: Duration = Duration::from_secs(30);
const GITHUB_OWNERSHIP_TOTAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const GITHUB_READINESS_TOTAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Parser)]
#[command(name = "stratadiff")]
#[command(about = "Resume code review from exact Git evidence")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect required-check signals on GitHub's exact evaluation candidate.
    Doctor {
        /// Positive pull request number or canonical HTTPS pull request URL.
        pull_request: String,
        /// Canonical GitHub repository in OWNER/REPO form. Required for a numeric selector.
        #[arg(short = 'R', long)]
        repository: Option<String>,
        /// GitHub or GitHub Enterprise Server hostname.
        #[arg(long)]
        hostname: Option<String>,
        /// Render a human-readable Markdown report or stable JSON.
        #[arg(long, value_enum, default_value_t = ReadinessOutput::Markdown)]
        format: ReadinessOutput,
        /// Write the report to this path instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Exit unsuccessfully unless required checks are clear on a supported head target.
        #[arg(long)]
        require_clear: bool,
    },
    /// Audit current branch rules, required-check sources, and sampled exact PR heads.
    ReadinessAudit {
        /// Canonical GitHub repository in OWNER/REPO form.
        #[arg(short = 'R', long)]
        repository: String,
        /// GitHub or GitHub Enterprise Server hostname.
        #[arg(long, default_value = "github.com")]
        hostname: String,
        /// Maximum recent open or merged pull requests to sample.
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u8).range(1..=25))]
        limit: u8,
        /// Render a human-readable Markdown report or stable JSON.
        #[arg(long, value_enum, default_value_t = ReadinessOutput::Markdown)]
        format: ReadinessOutput,
        /// Write the report to this path instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Also retain the complete bounded GitHub observation snapshot as JSON.
        #[arg(long)]
        snapshot_output: Option<PathBuf>,
        /// Exit unsuccessfully after writing the report when an actionable finding exists.
        #[arg(long)]
        fail_on_findings: bool,
    },
    /// Find open pull requests where one reviewer's completed checkpoint has moved.
    Inbox(inbox::InboxArgs),
    /// Resume one reviewer's latest completed GitHub review from exact Git evidence.
    Resume(resume::ResumeArgs),
    /// Verify and aggregate an explicitly enabled, local-only value-funnel log.
    ValueReport(value_funnel::ValueReportArgs),
    /// Internal isolated workbench entry point used by `resume`.
    #[command(name = "__resume-workbench", hide = true)]
    ResumeWorkbench(resume::ResumeWorkbenchArgs),
    /// Open a deterministic offline Review Resume scenario.
    Demo {
        /// Loopback port to listen on. Zero asks the operating system to choose one.
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Print the workbench URL without opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Print the third-party notices embedded in this executable.
    Licenses,
    /// Print machine-readable provenance for this exact executable.
    BuildInfo,
    /// Resolve one reviewer's latest completed GitHub review to a commit checkpoint.
    GithubCheckpoint {
        /// JSON array returned by GitHub's list pull request reviews endpoint.
        reviews: PathBuf,
        /// Exact GitHub login whose review history should be resumed.
        #[arg(long)]
        reviewer: String,
        /// Decode the nested page array emitted by `gh api --paginate --slurp`.
        #[arg(long, conflicts_with = "gh_included_response")]
        gh_slurp_pages: bool,
        /// Decode the status, headers, and JSON body emitted by `gh api --include`.
        #[arg(long, conflicts_with = "gh_slurp_pages")]
        gh_included_response: bool,
        /// Print only the commit ID or the complete selection record.
        #[arg(long, value_enum, default_value_t = GithubCheckpointOutput::Sha)]
        format: GithubCheckpointOutput,
        /// Write the result to this path instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Verify that GitHub's Git commit-object response is bound to an expected review commit.
    GithubCommitObject {
        /// JSON returned by GitHub's get-a-Git-commit endpoint.
        object: PathBuf,
        /// Full commit ID selected from the pull request's review records.
        #[arg(long)]
        expected: String,
    },
    /// Collect a fail-closed exact-base GitHub ownership snapshot through `gh` authentication.
    GithubOwnershipSnapshot {
        /// Full protected-branch base commit containing the authoritative CODEOWNERS file.
        base: String,
        /// Git worktree or repository containing the exact base commit.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Canonical GitHub repository in OWNER/REPO form.
        #[arg(long)]
        github_repository: String,
        /// Canonical GitHub or GitHub Enterprise Server origin.
        #[arg(long)]
        provider_url: String,
        /// Destination for the private, atomically replaced snapshot.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Verify and append one GitHub review event to an immutable-fact ledger.
    GithubLedgerIngest {
        /// Raw GitHub webhook request body.
        payload: PathBuf,
        /// Existing ledger to extend. Omit only for the first delivery.
        #[arg(long)]
        ledger: Option<PathBuf>,
        /// Exact X-GitHub-Event header value.
        #[arg(long)]
        event: String,
        /// Exact X-GitHub-Delivery header value.
        #[arg(long)]
        delivery_id: String,
        /// Receiver observation time in UTC, with second precision.
        #[arg(long)]
        received_at: String,
        /// Exact X-Hub-Signature-256 header value.
        #[arg(long)]
        signature: String,
        /// Canonical GitHub or GitHub Enterprise Server URL.
        #[arg(long)]
        provider_url: String,
        /// Stable identifier for the receiver key attesting accepted deliveries.
        #[arg(long)]
        receiver_key_id: String,
        /// Destination for the deterministic ledger JSON.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Build a signed reviewer × CODEOWNERS coverage passport for one exact PR head.
    ReviewCoverage {
        /// Exact protected-branch base commit containing the authoritative CODEOWNERS file.
        base: String,
        /// Exact pull request head commit to gate.
        head: String,
        /// Git worktree or repository directory containing every required object.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// HMAC-verified, receiver-attested GitHub review ledger.
        #[arg(long)]
        ledger: PathBuf,
        /// Exact-base GitHub identity, permission, team, and membership snapshot.
        #[arg(long)]
        ownership: PathBuf,
        /// Destination for the signed review-coverage passport.
        #[arg(short, long)]
        output: PathBuf,
        /// Exit unsuccessfully after writing the passport if any required owner coverage is open.
        #[arg(long)]
        fail_on_missing_coverage: bool,
    },
    /// Verify a signed coverage passport and recompute it from exact offline Git objects.
    ReviewCoverageVerify {
        /// Signed review-coverage passport.
        passport: PathBuf,
        /// Offline Git object store or worktree.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Trusted receiver Ed25519 public key as 64 lowercase hexadecimal characters.
        #[arg(long)]
        trusted_receiver_public_key: String,
    },
    /// Open an offline-verified review coverage passport in the local workbench.
    ReviewCoverageView {
        /// Signed review-coverage passport.
        passport: PathBuf,
        /// Offline Git object store or worktree used to recompute the coverage decision.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Trusted receiver Ed25519 public key as 64 lowercase hexadecimal characters.
        #[arg(long)]
        trusted_receiver_public_key: String,
        /// Loopback port to listen on. Zero asks the operating system to choose one.
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Print the workbench URL without opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Build an offline-verified GitHub App Check Run request body without publishing it.
    ///
    /// GitHub grants Check Run write access only to GitHub Apps. This command writes JSON;
    /// an ordinary personal access token cannot publish it as a Check Run.
    GithubCheckRun {
        /// Signed review-coverage passport.
        passport: PathBuf,
        /// Offline Git object store or worktree used to recompute the passport.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Trusted receiver Ed25519 public key as 64 lowercase hexadecimal characters.
        #[arg(long)]
        trusted_receiver_public_key: String,
        /// Live pull-request base SHA observed immediately before publishing.
        #[arg(long)]
        expected_base: String,
        /// Live pull-request head SHA observed immediately before publishing.
        #[arg(long)]
        expected_head: String,
        /// Optional safe HTTPS page containing the complete review queue.
        #[arg(long)]
        details_url: Option<String>,
        /// Destination for the deterministic create-check-run JSON request body.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Build a complete review context from explicit reviewer and Git inputs.
    ReviewCacheContext {
        /// Complete, non-shallow Git repository containing the transition history.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Requested PR base commit used to resolve both merge bases.
        #[arg(long)]
        base: String,
        /// Exact commit previously reviewed, or the current head for a first review.
        #[arg(long)]
        checkpoint: String,
        /// Current PR head commit.
        #[arg(long)]
        head: String,
        /// Versioned manifest binding reviewer, dependencies, and prior dispositions.
        #[arg(long)]
        reviewer_manifest: PathBuf,
        /// GitHub or GitHub Enterprise hostname.
        #[arg(long)]
        provider_host: String,
        /// Repository owner.
        #[arg(long)]
        owner: String,
        /// Repository name.
        #[arg(long)]
        name: String,
        /// Stable provider repository node ID.
        #[arg(long)]
        repository_id: String,
        /// Stable provider pull-request node ID.
        #[arg(long)]
        pull_request_node_id: String,
        /// Pull-request number.
        #[arg(long)]
        pull_request_number: u64,
        /// Requested base ref name.
        #[arg(long)]
        base_ref: String,
        /// Pull-request head ref name.
        #[arg(long)]
        head_ref: String,
        /// UTC RFC 3339 time at which provider metadata was observed.
        #[arg(long)]
        observed_at: String,
        /// SHA-256 of canonical reviewer-visible PR metadata excluding refs, OIDs, and time.
        #[arg(long)]
        canonical_metadata_sha256: String,
        /// Exact reviewer input scope.
        #[arg(long, value_parser = ["selected_payload_only", "declared_repository_closure"])]
        review_input_scope: String,
        /// Destination for the canonical review-context-v1 artifact.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Route an existing reviewer to skip, residue, full, or blocked from exact Git evidence.
    ReviewCache {
        /// Exact commit previously reviewed. A signed receipt is required before anything carries.
        checkpoint: String,
        /// Complete current review-context-v1 JSON artifact.
        #[arg(long)]
        context: PathBuf,
        /// Complete, non-shallow Git repository containing the transition history.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Signed review-receipt-v1 JSON. Omit for a conservative full route.
        #[arg(
            long,
            requires_all = [
                "prior_input",
                "prior_payload",
                "prior_result",
                "trusted_key_id",
                "trusted_public_key",
                "trust_domain",
                "trust_policy_sha256"
            ]
        )]
        receipt: Option<PathBuf>,
        /// Exact canonical review-input JSON bound by the receipt.
        #[arg(long, requires = "receipt")]
        prior_input: Option<PathBuf>,
        /// Exact canonical selected-payload JSON bound by the receipt.
        #[arg(long, requires = "receipt")]
        prior_payload: Option<PathBuf>,
        /// Exact canonical reviewer result JSON bound by the receipt.
        #[arg(long, requires = "receipt")]
        prior_result: Option<PathBuf>,
        /// Trusted receipt key identifier.
        #[arg(long, requires = "receipt")]
        trusted_key_id: Option<String>,
        /// Trusted Ed25519 public key as 64 lowercase hexadecimal characters.
        #[arg(long, requires = "receipt")]
        trusted_public_key: Option<String>,
        /// Trusted receipt issuer domain.
        #[arg(long, requires = "receipt")]
        trust_domain: Option<String>,
        /// Digest of the exact trust policy authorizing the receipt key.
        #[arg(long, requires = "receipt")]
        trust_policy_sha256: Option<String>,
        /// UTC RFC 3339 timestamp recorded in the deterministic route artifact.
        #[arg(long)]
        generated_at: String,
        /// Destination for the canonical selected reviewer payload.
        #[arg(long)]
        payload_output: PathBuf,
        /// Destination for the exact projection that the adapter may expose to the reviewer.
        #[arg(long)]
        reviewer_input_output: PathBuf,
        /// Destination for the canonical review-input routing decision. Omit to write stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Append stable outputs for a GitHub Actions step.
        #[arg(long, requires = "output")]
        github_output: Option<PathBuf>,
    },
    /// Issue a signed receipt only after an adapter proves complete review coverage.
    ReviewCacheReceipt {
        /// Complete, non-shallow Git repository containing the reviewed transition.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Complete review-context-v1 JSON used by preflight.
        #[arg(long)]
        context: PathBuf,
        /// Canonical review-input-v1 routing decision produced by preflight.
        #[arg(long)]
        review_input: PathBuf,
        /// Canonical selected payload consumed by the reviewer.
        #[arg(long)]
        selected_payload: PathBuf,
        /// Canonical review-cache-result-v1 completion manifest produced by the adapter.
        #[arg(long)]
        result: PathBuf,
        /// Live PR base observed by the adapter immediately after reviewer completion.
        #[arg(long)]
        expected_base: String,
        /// Live PR head observed by the adapter immediately after reviewer completion.
        #[arg(long)]
        expected_head: String,
        /// Prior signed receipt whose exact carries were used by a residue execution.
        #[arg(
            long,
            requires_all = [
                "prior_input",
                "prior_payload",
                "prior_result",
                "prior_key_id",
                "prior_public_key",
                "prior_trust_domain",
                "prior_trust_policy_sha256"
            ]
        )]
        prior_receipt: Option<PathBuf>,
        /// Exact canonical review-input JSON bound by the prior receipt.
        #[arg(long, requires = "prior_receipt")]
        prior_input: Option<PathBuf>,
        /// Exact canonical selected-payload JSON bound by the prior receipt.
        #[arg(long, requires = "prior_receipt")]
        prior_payload: Option<PathBuf>,
        /// Exact canonical reviewer result JSON bound by the prior receipt.
        #[arg(long, requires = "prior_receipt")]
        prior_result: Option<PathBuf>,
        /// Trusted key identifier for the prior receipt.
        #[arg(long, requires = "prior_receipt")]
        prior_key_id: Option<String>,
        /// Trusted Ed25519 public key for the prior receipt.
        #[arg(long, requires = "prior_receipt")]
        prior_public_key: Option<String>,
        /// Trusted issuer domain for the prior receipt.
        #[arg(long, requires = "prior_receipt")]
        prior_trust_domain: Option<String>,
        /// Digest of the trust policy authorizing the prior receipt key.
        #[arg(long, requires = "prior_receipt")]
        prior_trust_policy_sha256: Option<String>,
        /// Unique receipt identifier.
        #[arg(long)]
        receipt_id: String,
        /// UTC RFC 3339 receipt issuance time.
        #[arg(long)]
        issued_at: String,
        /// Stable identifier for the reviewer adapter issuing the receipt.
        #[arg(long)]
        issuer_id: String,
        /// Trust domain in which the signing key is authorized.
        #[arg(long)]
        trust_domain: String,
        /// SHA-256 of the exact policy authorizing this signing key.
        #[arg(long)]
        trust_policy_sha256: String,
        /// Stable Ed25519 signing-key identifier.
        #[arg(long)]
        key_id: String,
        /// File containing the 32-byte Ed25519 signing key as 64 lowercase hex characters.
        #[arg(long)]
        signing_key_file: PathBuf,
        /// Destination for the canonical signed review receipt.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Compare two source files and produce a structural report.
    Diff {
        before: PathBuf,
        after: PathBuf,
        #[arg(long, value_enum)]
        language: Option<Language>,
        /// Write the complete JSON report and patch reconstruction certificate to this path.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Print the complete report as JSON instead of the terminal summary.
        #[arg(long)]
        json: bool,
    },
    /// Re-run all independently checkable predicates and the patch reconstruction certificate.
    Verify {
        report: PathBuf,
        before: PathBuf,
        after: PathBuf,
    },
    /// Rebuild the target bytes from a report and the original source.
    Apply {
        report: PathBuf,
        before: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Open an independently verified report in the local evidence workbench.
    View {
        before: PathBuf,
        after: PathBuf,
        #[arg(long, value_enum)]
        language: Option<Language>,
        /// Loopback port to listen on. Zero asks the operating system to choose one.
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Print the workbench URL without opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Triage a Git commit range into evidence-backed review lanes.
    Review {
        /// Base revision. The comparison starts at its merge base with the requested head.
        base: String,
        /// Head revision to review.
        #[arg(default_value = "HEAD")]
        head: String,
        /// Commit whose complete PR change set the caller has already reviewed.
        #[arg(long)]
        checkpoint: Option<String>,
        /// Git worktree or repository directory.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Render a GitHub-ready Markdown summary or stable JSON.
        #[arg(long, value_enum, default_value_t = ReviewOutput::Markdown)]
        format: ReviewOutput,
        /// Write the review report to this path instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Write the checkpoint-to-head review delta, including reconstructed-baseline evidence.
        #[arg(long, requires = "checkpoint", conflicts_with = "workbench")]
        review_delta_output: Option<PathBuf>,
        /// Append Markdown to the path named by GITHUB_STEP_SUMMARY.
        #[arg(long)]
        github_summary: bool,
        /// Emit GitHub workflow error annotations for the current review residue.
        #[arg(long, requires = "output", conflicts_with = "workbench")]
        github_annotations: bool,
        /// Exit unsuccessfully unless a checkpoint exists and its exact review queue is empty.
        #[arg(long)]
        fail_on_review_residue: bool,
        /// Open the repository review queue in the local Evidence Workbench.
        #[arg(
            long,
            requires = "checkpoint",
            conflicts_with_all = ["output", "github_summary", "format"]
        )]
        workbench: bool,
        /// Loopback port for --workbench. Zero asks the operating system to choose one.
        #[arg(long, default_value_t = 0, requires = "workbench")]
        port: u16,
        /// Print the workbench URL without opening a browser.
        #[arg(long, requires = "workbench")]
        no_open: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ReviewOutput {
    Markdown,
    Json,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ReadinessOutput {
    Markdown,
    Json,
}

#[derive(Debug, PartialEq, Eq)]
struct DoctorSelection {
    provider_url: String,
    hostname: String,
    repository: String,
    pull_request_number: u64,
}

fn valid_doctor_repository_component(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_doctor_repository(value: &str) -> Result<()> {
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    ensure!(
        parts.next().is_none()
            && valid_doctor_repository_component(owner)
            && valid_doctor_repository_component(name),
        "doctor repository must use canonical OWNER/REPO form"
    );
    Ok(())
}

fn parse_doctor_pull_number(value: &str) -> Result<u64> {
    ensure!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
        "doctor pull request must be a positive number or canonical HTTPS pull request URL"
    );
    let number = value
        .parse::<u64>()
        .context("doctor pull request number exceeds u64")?;
    ensure!(
        number > 0 && value == number.to_string(),
        "doctor pull request number must be positive and canonical"
    );
    Ok(number)
}

fn validate_doctor_hostname(value: &str) -> Result<String> {
    let mut authority = value.split(':');
    let host = authority.next().unwrap_or_default();
    let port = authority.next();
    ensure!(
        authority.next().is_none()
            && !host.is_empty()
            && host.len() <= 253
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
                    && label.as_bytes()[0].is_ascii_alphanumeric()
                    && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
            }),
        "doctor hostname must be a canonical lowercase DNS name"
    );
    if let Some(port) = port {
        let port_number = port
            .parse::<u16>()
            .context("doctor hostname port must be a positive decimal u16")?;
        ensure!(
            port_number > 0 && port == port_number.to_string(),
            "doctor hostname port must be positive and canonical"
        );
    }
    let provider_url = format!("https://{value}");
    Ok(github_provider_hostname(&provider_url)?.to_owned())
}

fn resolve_doctor_selection(
    pull_request: &str,
    repository: Option<&str>,
    hostname: Option<&str>,
) -> Result<DoctorSelection> {
    if pull_request.bytes().all(|byte| byte.is_ascii_digit()) {
        let repository =
            repository.context("a numeric doctor pull request requires --repository OWNER/REPO")?;
        validate_doctor_repository(repository)?;
        let hostname = validate_doctor_hostname(hostname.unwrap_or("github.com"))?;
        return Ok(DoctorSelection {
            provider_url: format!("https://{hostname}"),
            hostname,
            repository: repository.to_owned(),
            pull_request_number: parse_doctor_pull_number(pull_request)?,
        });
    }

    let path = pull_request.strip_prefix("https://").context(
        "doctor pull request must be a positive number or canonical HTTPS pull request URL",
    )?;
    let parts = path.split('/').collect::<Vec<_>>();
    ensure!(
        parts.len() == 5 && parts[3] == "pull",
        "doctor pull request URL must use https://HOST/OWNER/REPO/pull/NUMBER"
    );
    let url_hostname = validate_doctor_hostname(parts[0])?;
    let url_repository = format!("{}/{}", parts[1], parts[2]);
    validate_doctor_repository(&url_repository)?;
    let pull_request_number = parse_doctor_pull_number(parts[4])?;
    if let Some(repository) = repository {
        validate_doctor_repository(repository)?;
        ensure!(
            repository.eq_ignore_ascii_case(&url_repository),
            "doctor pull request URL does not match --repository"
        );
    }
    if let Some(hostname) = hostname {
        let hostname = validate_doctor_hostname(hostname)?;
        ensure!(
            hostname == url_hostname,
            "doctor pull request URL does not match --hostname"
        );
    }
    Ok(DoctorSelection {
        provider_url: format!("https://{url_hostname}"),
        hostname: url_hostname,
        repository: url_repository,
        pull_request_number,
    })
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum GithubCheckpointOutput {
    Sha,
    Json,
}

#[derive(Debug, Serialize)]
struct BuildInfo {
    schema: &'static str,
    engine_version: &'static str,
    git_revision: &'static str,
    git_dirty: Option<bool>,
    cargo_lock_sha256: &'static str,
    build_profile: &'static str,
    rustc_version: &'static str,
}

fn embedded_build_info() -> BuildInfo {
    let git_dirty = match BUILD_GIT_DIRTY {
        "false" => Some(false),
        "true" => Some(true),
        _ => None,
    };
    BuildInfo {
        schema: BUILD_INFO_SCHEMA,
        engine_version: env!("CARGO_PKG_VERSION"),
        git_revision: BUILD_GIT_REVISION,
        git_dirty,
        cargo_lock_sha256: BUILD_CARGO_LOCK_SHA256,
        build_profile: BUILD_PROFILE,
        rustc_version: BUILD_RUSTC_VERSION,
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            print!("{error}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("{}", escape_terminal_unsafe_text(&error.to_string()));
            return ExitCode::FAILURE;
        }
    };

    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if let Some(interrupted) = error.downcast_ref::<process::Interrupted>()
                && let Some(code) = 128_i32
                    .checked_add(interrupted.signal())
                    .and_then(|code| u8::try_from(code).ok())
            {
                return ExitCode::from(code);
            }
            eprintln!(
                "Error: {}",
                escape_terminal_unsafe_text(&format!("{error:#}"))
            );
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::Doctor {
            pull_request,
            repository,
            hostname,
            format,
            output,
            require_clear,
        } => {
            let selection = resolve_doctor_selection(
                &pull_request,
                repository.as_deref(),
                hostname.as_deref(),
            )?;
            let mut api = GhCliReadinessApi {
                hostname: selection.hostname.clone(),
                started: Instant::now(),
            };
            let captured_at = current_utc_timestamp()?;
            let snapshot = collect_pull_request_doctor_snapshot_v3(
                PullRequestDoctorCollection {
                    provider_url: &selection.provider_url,
                    repository: &selection.repository,
                    captured_at: &captured_at,
                    pull_request_number: selection.pull_request_number,
                },
                &mut api,
            )?;
            let report = evaluate_pull_request_doctor_v3(&snapshot)?;
            let mut encoded = match (format, output.is_some()) {
                (ReadinessOutput::Markdown, _) => {
                    render_pull_request_doctor_v3_markdown(&report).into_bytes()
                }
                (ReadinessOutput::Json, true) => serde_json::to_vec_pretty(&report)?,
                (ReadinessOutput::Json, false) => serde_json::to_vec(&report)?,
            };
            if let Some(path) = output {
                if !encoded.ends_with(b"\n") {
                    encoded.push(b'\n');
                }
                std::fs::write(&path, &encoded)
                    .with_context(|| format!("failed to write {}", display_path(&path)))?;
            } else {
                let mut stdout = std::io::stdout().lock();
                match format {
                    ReadinessOutput::Markdown => stdout.write_all(&encoded)?,
                    ReadinessOutput::Json => {
                        stdout.write_all(escape_terminal_unsafe_json(&encoded).as_bytes())?;
                        stdout.write_all(b"\n")?;
                    }
                }
            }
            if require_clear {
                let verdict = match report.verdict {
                    DoctorVerdict::ChecksClear => "checks_clear",
                    DoctorVerdict::ChecksBlocked => "checks_blocked",
                    DoctorVerdict::Inconclusive => "inconclusive",
                };
                ensure!(
                    report.verdict == DoctorVerdict::ChecksClear,
                    "pull request #{} required checks are not proven clear: {verdict}",
                    report.target.number
                );
            }
        }
        Command::ReadinessAudit {
            repository,
            hostname,
            limit,
            format,
            output,
            snapshot_output,
            fail_on_findings,
        } => {
            if let (Some(output), Some(snapshot_output)) = (&output, &snapshot_output) {
                ensure!(
                    output != snapshot_output,
                    "report and snapshot output paths must differ"
                );
            }
            let provider_url = format!("https://{hostname}");
            let canonical_hostname = github_provider_hostname(&provider_url)?.to_owned();
            let mut api = GhCliReadinessApi {
                hostname: canonical_hostname,
                started: Instant::now(),
            };
            let captured_at = current_utc_timestamp()?;
            let snapshot = collect_merge_readiness_snapshot(
                MergeReadinessCollection {
                    provider_url: &provider_url,
                    repository: &repository,
                    captured_at: &captured_at,
                    pull_request_limit: usize::from(limit),
                },
                &mut api,
            )?;
            if let Some(path) = snapshot_output {
                let mut encoded = serde_json::to_vec_pretty(&snapshot)?;
                encoded.push(b'\n');
                std::fs::write(&path, encoded)
                    .with_context(|| format!("failed to write {}", display_path(&path)))?;
            }
            let report = evaluate_merge_readiness(&snapshot)?;
            let mut encoded = match (format, output.is_some()) {
                (ReadinessOutput::Markdown, _) => {
                    render_merge_readiness_markdown(&report).into_bytes()
                }
                (ReadinessOutput::Json, true) => serde_json::to_vec_pretty(&report)?,
                (ReadinessOutput::Json, false) => serde_json::to_vec(&report)?,
            };
            if let Some(path) = output {
                if !encoded.ends_with(b"\n") {
                    encoded.push(b'\n');
                }
                std::fs::write(&path, &encoded)
                    .with_context(|| format!("failed to write {}", display_path(&path)))?;
            } else {
                let mut stdout = std::io::stdout().lock();
                match format {
                    ReadinessOutput::Markdown => stdout.write_all(&encoded)?,
                    ReadinessOutput::Json => {
                        stdout.write_all(escape_terminal_unsafe_json(&encoded).as_bytes())?;
                        stdout.write_all(b"\n")?;
                    }
                }
            }
            if fail_on_findings {
                ensure!(
                    report.summary.verdict != AuditVerdict::ActionRequired,
                    "merge-readiness audit found {} actionable issue(s)",
                    report.summary.findings
                );
            }
        }
        Command::Inbox(args) => inbox::run(args)?,
        Command::Resume(args) => resume::run(args)?,
        Command::ValueReport(args) => value_funnel::run(args)?,
        Command::ResumeWorkbench(args) => resume::run_workbench(args)?,
        Command::Demo { port, no_open } => demo::run(port, no_open)?,
        Command::Licenses => print!("{THIRD_PARTY_NOTICES}"),
        Command::BuildInfo => {
            let mut stdout = std::io::stdout().lock();
            serde_json::to_writer(&mut stdout, &embedded_build_info())?;
            stdout.write_all(b"\n")?;
        }
        Command::GithubCheckpoint {
            reviews,
            reviewer,
            gh_slurp_pages,
            gh_included_response,
            format,
            output,
        } => {
            let review_bytes_limit = if gh_included_response {
                MAX_GITHUB_REVIEWS_INCLUDED_RESPONSE_BYTES
            } else {
                MAX_GITHUB_REVIEWS_BYTES
            };
            let review_bytes = read_bounded(
                &reviews,
                review_bytes_limit,
                "GitHub pull request reviews bytes",
            )?;
            let resolution = if gh_included_response {
                resolve_github_review_checkpoint_included_response(&review_bytes, &reviewer)?
            } else if gh_slurp_pages {
                resolve_github_review_checkpoint_slurp_pages(&review_bytes, &reviewer)?
            } else {
                resolve_github_review_checkpoint(&review_bytes, &reviewer)?
            };
            let encoded = match format {
                GithubCheckpointOutput::Sha => resolution
                    .checkpoint
                    .as_ref()
                    .map(|checkpoint| checkpoint.commit_id.as_bytes().to_vec())
                    .unwrap_or_default(),
                GithubCheckpointOutput::Json => serde_json::to_vec(&resolution)?,
            };
            if let Some(path) = output {
                std::fs::write(&path, &encoded)
                    .with_context(|| format!("failed to write {}", display_path(&path)))?;
            } else if !encoded.is_empty() {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(&encoded)?;
                stdout.write_all(b"\n")?;
            }
        }
        Command::GithubCommitObject { object, expected } => {
            let object_bytes = read_bounded(
                &object,
                MAX_GITHUB_COMMIT_OBJECT_BYTES,
                "GitHub Git commit object bytes",
            )?;
            verify_github_commit_object(&object_bytes, &expected)?;
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(expected.as_bytes())?;
            stdout.write_all(b"\n")?;
        }
        Command::GithubOwnershipSnapshot {
            base,
            repo,
            github_repository,
            provider_url,
            output,
        } => {
            let policy = CodeownersPolicy::load(&repo, &base)?;
            let identities = policy.owner_identities();
            let hostname = github_provider_hostname(&provider_url)?.to_owned();
            let mut api = GhCliOwnershipApi {
                hostname,
                started: Instant::now(),
            };
            let snapshot = collect_github_ownership_snapshot(
                &provider_url,
                &github_repository,
                &base,
                &identities,
                &mut api,
            )?;
            write_github_ownership_snapshot(&output, &snapshot)?;
            eprintln!(
                "wrote stable GitHub ownership snapshot to {}: {} users, {} teams",
                display_path(&output),
                snapshot.users.len(),
                snapshot.teams.len()
            );
        }
        Command::GithubLedgerIngest {
            payload,
            ledger,
            event,
            delivery_id,
            received_at,
            signature,
            provider_url,
            receiver_key_id,
            output,
        } => {
            let payload_bytes = read_bounded(
                &payload,
                MAX_GITHUB_WEBHOOK_BYTES,
                "GitHub webhook payload bytes",
            )?;
            let ledger = ledger
                .map(|path| -> Result<GithubReviewLedger> {
                    let bytes =
                        read_bounded(&path, MAX_GITHUB_LEDGER_BYTES, "GitHub review ledger bytes")?;
                    serde_json::from_slice(&bytes).with_context(|| {
                        format!(
                            "failed to decode GitHub review ledger {}",
                            display_path(&path)
                        )
                    })
                })
                .transpose()?;
            let secret = std::env::var("STRATADIFF_GITHUB_WEBHOOK_SECRET")
                .context("github-ledger-ingest requires STRATADIFF_GITHUB_WEBHOOK_SECRET")?;
            let signing_key = std::env::var("STRATADIFF_RECEIPT_SIGNING_KEY")
                .context("github-ledger-ingest requires STRATADIFF_RECEIPT_SIGNING_KEY")?;
            let signing_key = decode_ed25519_signing_key(&signing_key)?;
            let (ledger, outcome) = ingest_github_webhook(
                ledger,
                GithubWebhookIngest {
                    provider_url: &provider_url,
                    event_name: &event,
                    delivery_id: &delivery_id,
                    received_at: &received_at,
                    signature_header: &signature,
                    secret: secret.as_bytes(),
                    receiver_key_id: &receiver_key_id,
                    receiver_signing_key: &signing_key,
                    payload: &payload_bytes,
                },
            )?;
            let encoded = serde_json::to_vec(&ledger)?;
            ensure!(
                encoded.len() <= MAX_GITHUB_LEDGER_BYTES,
                "generated GitHub review ledger exceeds the byte limit"
            );
            std::fs::write(&output, encoded)
                .with_context(|| format!("failed to write {}", display_path(&output)))?;
            let label = match outcome {
                IngestOutcome::Applied => "applied",
                IngestOutcome::Duplicate => "duplicate",
            };
            eprintln!(
                "{label} GitHub delivery {}; wrote review ledger to {}",
                display_text(&delivery_id),
                display_path(&output)
            );
        }
        Command::ReviewCoverage {
            base,
            head,
            repo,
            ledger,
            ownership,
            output,
            fail_on_missing_coverage,
        } => {
            let ledger_bytes = read_bounded(
                &ledger,
                MAX_GITHUB_LEDGER_BYTES,
                "GitHub review ledger bytes",
            )?;
            let ledger: GithubReviewLedger = serde_json::from_slice(&ledger_bytes)
                .with_context(|| format!("failed to decode {}", display_path(&ledger)))?;
            let ownership_bytes = read_bounded(
                &ownership,
                MAX_OWNERSHIP_SNAPSHOT_BYTES,
                "GitHub ownership snapshot bytes",
            )?;
            let ownership: GithubOwnershipSnapshot = serde_json::from_slice(&ownership_bytes)
                .with_context(|| format!("failed to decode {}", display_path(&ownership)))?;
            let signing_key = std::env::var("STRATADIFF_RECEIPT_SIGNING_KEY")
                .context("review-coverage requires STRATADIFF_RECEIPT_SIGNING_KEY")?;
            let signing_key = decode_ed25519_signing_key(&signing_key)?;
            let passport = build_review_coverage_passport(
                &repo,
                &base,
                &head,
                ledger,
                ownership,
                &signing_key,
            )?;
            let encoded = serde_json::to_vec(&passport)?;
            ensure!(
                encoded.len() <= MAX_REVIEW_COVERAGE_BYTES,
                "generated review coverage passport exceeds the byte limit"
            );
            std::fs::write(&output, encoded)
                .with_context(|| format!("failed to write {}", display_path(&output)))?;
            eprintln!(
                "wrote signed coverage passport to {}: {} covered, {} need review, {} blocked",
                display_path(&output),
                passport.body.summary.covered_files,
                passport.body.summary.needs_review_files,
                passport.body.summary.blocked_files
            );
            if fail_on_missing_coverage {
                ensure!(
                    passport.body.summary.gate_passed,
                    "review coverage gate is open: {} file(s) need review and {} file(s) are blocked",
                    passport.body.summary.needs_review_files,
                    passport.body.summary.blocked_files
                );
            }
        }
        Command::ReviewCoverageVerify {
            passport,
            repo,
            trusted_receiver_public_key,
        } => {
            let bytes = read_bounded(
                &passport,
                MAX_REVIEW_COVERAGE_BYTES,
                "review coverage passport bytes",
            )?;
            let passport: ReviewCoveragePassport = serde_json::from_slice(&bytes)
                .with_context(|| format!("failed to decode {}", display_path(&passport)))?;
            verify_review_coverage_passport(&repo, &passport, &trusted_receiver_public_key)?;
            println!(
                "verified review coverage passport for {} at {}",
                display_text(&passport.body.ledger.repository.full_name),
                passport.body.head_commit
            );
        }
        Command::ReviewCoverageView {
            passport,
            repo,
            trusted_receiver_public_key,
            port,
            no_open,
        } => {
            let bytes = read_bounded(
                &passport,
                MAX_REVIEW_COVERAGE_BYTES,
                "review coverage passport bytes",
            )?;
            let passport: ReviewCoveragePassport = serde_json::from_slice(&bytes)
                .with_context(|| format!("failed to decode {}", display_path(&passport)))?;
            verify_review_coverage_passport(&repo, &passport, &trusted_receiver_public_key)?;
            viewer::serve_review_coverage(passport, bytes, port, !no_open)?;
        }
        Command::GithubCheckRun {
            passport,
            repo,
            trusted_receiver_public_key,
            expected_base,
            expected_head,
            details_url,
            output,
        } => {
            let bytes = read_bounded(
                &passport,
                MAX_REVIEW_COVERAGE_BYTES,
                "review coverage passport bytes",
            )?;
            let passport: ReviewCoveragePassport = serde_json::from_slice(&bytes)
                .with_context(|| format!("failed to decode {}", display_path(&passport)))?;
            let payload = build_github_check_run_payload(
                &repo,
                &passport,
                &trusted_receiver_public_key,
                &expected_base,
                &expected_head,
                details_url.as_deref(),
            )?;
            let encoded = serde_json::to_vec(&payload)?;
            ensure!(
                encoded.len() <= MAX_GITHUB_CHECK_RUN_PAYLOAD_BYTES,
                "generated GitHub Check Run payload exceeds the byte limit"
            );
            std::fs::write(&output, encoded)
                .with_context(|| format!("failed to write {}", display_path(&output)))?;
            eprintln!(
                "wrote verified GitHub App Check Run payload for {} to {} (not published)",
                payload.head_sha,
                display_path(&output)
            );
        }
        Command::ReviewCacheContext {
            repo,
            base,
            checkpoint,
            head,
            reviewer_manifest,
            provider_host,
            owner,
            name,
            repository_id,
            pull_request_node_id,
            pull_request_number,
            base_ref,
            head_ref,
            observed_at,
            canonical_metadata_sha256,
            review_input_scope,
            output,
        } => {
            let reviewer_manifest = read_bounded(
                &reviewer_manifest,
                MAX_REVIEW_CACHE_JSON_BYTES,
                "reviewer manifest bytes",
            )?;
            let context = build_review_cache_context(ReviewCacheContextBuild {
                repository: &repo,
                requested_base: &base,
                checkpoint: &checkpoint,
                head: &head,
                provider_host: &provider_host,
                owner: &owner,
                name: &name,
                repository_id: &repository_id,
                pull_request_node_id: &pull_request_node_id,
                pull_request_number,
                base_ref: &base_ref,
                head_ref: &head_ref,
                observed_at: &observed_at,
                canonical_metadata_sha256: &canonical_metadata_sha256,
                review_input_scope: &review_input_scope,
                reviewer_manifest: &reviewer_manifest,
            })?;
            std::fs::write(&output, context)
                .with_context(|| format!("failed to write {}", display_path(&output)))?;
            eprintln!("wrote review cache context to {}", display_path(&output));
        }
        Command::ReviewCache {
            checkpoint,
            context,
            repo,
            receipt,
            prior_input,
            prior_payload,
            prior_result,
            trusted_key_id,
            trusted_public_key,
            trust_domain,
            trust_policy_sha256,
            generated_at,
            payload_output,
            reviewer_input_output,
            output,
            github_output,
        } => {
            let context_bytes = read_bounded(
                &context,
                MAX_REVIEW_CACHE_JSON_BYTES,
                "review context bytes",
            )?;
            let receipt_bytes = receipt
                .as_ref()
                .map(|path| read_bounded(path, MAX_REVIEW_CACHE_JSON_BYTES, "review receipt bytes"))
                .transpose()?;
            let prior_input_bytes = prior_input
                .as_ref()
                .map(|path| {
                    read_bounded(
                        path,
                        MAX_REVIEW_CACHE_JSON_BYTES,
                        "prior review input bytes",
                    )
                })
                .transpose()?;
            let prior_payload_bytes = prior_payload
                .as_ref()
                .map(|path| {
                    read_bounded(
                        path,
                        MAX_REVIEW_CACHE_JSON_BYTES,
                        "prior selected payload bytes",
                    )
                })
                .transpose()?;
            let prior_result_bytes = prior_result
                .as_ref()
                .map(|path| {
                    read_bounded(
                        path,
                        MAX_REVIEW_CACHE_JSON_BYTES,
                        "prior review result bytes",
                    )
                })
                .transpose()?;
            let receipt_bundle = match receipt_bytes.as_deref() {
                Some(receipt_bytes) => Some(ReviewCacheReceiptBundle {
                    receipt: receipt_bytes,
                    prior_input: prior_input_bytes
                        .as_deref()
                        .context("--receipt requires --prior-input")?,
                    prior_payload: prior_payload_bytes
                        .as_deref()
                        .context("--receipt requires --prior-payload")?,
                    prior_result: prior_result_bytes
                        .as_deref()
                        .context("--receipt requires --prior-result")?,
                    trusted_key_id: trusted_key_id
                        .as_deref()
                        .context("--receipt requires --trusted-key-id")?,
                    trusted_public_key: trusted_public_key
                        .as_deref()
                        .context("--receipt requires --trusted-public-key")?,
                    trust_domain: trust_domain
                        .as_deref()
                        .context("--receipt requires --trust-domain")?,
                    trust_policy_sha256: trust_policy_sha256
                        .as_deref()
                        .context("--receipt requires --trust-policy-sha256")?,
                }),
                None => None,
            };
            let artifacts = review_cache_preflight(ReviewCachePreflight {
                repository: &repo,
                checkpoint: &checkpoint,
                generated_at: &generated_at,
                current_context: &context_bytes,
                receipt: receipt_bundle,
            })?;
            let cached_blocking = artifacts.decision
                == stratadiff::review_cache::ReviewCacheDecision::Skip
                && artifacts
                    .cached_outcome
                    .as_deref()
                    .is_some_and(|outcome| matches!(outcome, "failed" | "changes_requested"));
            std::fs::write(&payload_output, &artifacts.selected_payload_bytes)
                .with_context(|| format!("failed to write {}", display_path(&payload_output)))?;
            std::fs::write(&reviewer_input_output, &artifacts.reviewer_input_bytes).with_context(
                || format!("failed to write {}", display_path(&reviewer_input_output)),
            )?;
            if let Some(path) = &output {
                std::fs::write(path, &artifacts.review_input_bytes)
                    .with_context(|| format!("failed to write {}", display_path(path)))?;
            } else {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(&artifacts.review_input_bytes)?;
                stdout.write_all(b"\n")?;
            }
            if let Some(path) = github_output {
                let review_input_path = std::fs::canonicalize(
                    output
                        .as_ref()
                        .context("--github-output requires --output")?,
                )
                .context("failed to resolve review cache output path")?;
                let payload_path = std::fs::canonicalize(&payload_output)
                    .context("failed to resolve review cache payload output path")?;
                let reviewer_input_path = std::fs::canonicalize(&reviewer_input_output)
                    .context("failed to resolve reviewer input output path")?;
                let review_input_path = review_input_path.to_string_lossy();
                let payload_path = payload_path.to_string_lossy();
                let reviewer_input_path = reviewer_input_path.to_string_lossy();
                ensure!(
                    !review_input_path.contains(['\r', '\n'])
                        && !payload_path.contains(['\r', '\n'])
                        && !reviewer_input_path.contains(['\r', '\n']),
                    "review cache output path cannot be represented in GITHUB_OUTPUT"
                );
                let should_run = matches!(
                    artifacts.decision,
                    stratadiff::review_cache::ReviewCacheDecision::Residue
                        | stratadiff::review_cache::ReviewCacheDecision::Full
                );
                let mut github = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .with_context(|| format!("failed to open {}", display_path(&path)))?;
                writeln!(github, "decision={}", artifacts.decision.as_str())?;
                writeln!(github, "should_run={should_run}")?;
                writeln!(
                    github,
                    "cached_outcome={}",
                    artifacts.cached_outcome.as_deref().unwrap_or("none")
                )?;
                writeln!(github, "cached_blocking={cached_blocking}")?;
                writeln!(github, "review_input={review_input_path}")?;
                writeln!(github, "selected_payload={payload_path}")?;
                writeln!(github, "reviewer_visible_input={reviewer_input_path}")?;
            }
            if let Some(notice) = artifacts.receipt_notice {
                eprintln!("{}", escape_terminal_unsafe_text(&notice));
            }
            eprintln!(
                "review cache decision: {} (payload {})",
                artifacts.decision.as_str(),
                display_path(&payload_output)
            );
            ensure!(
                !cached_blocking,
                "cached review outcome is blocking; the unchanged reviewed input remains rejected"
            );
        }
        Command::ReviewCacheReceipt {
            repo,
            context,
            review_input,
            selected_payload,
            result,
            expected_base,
            expected_head,
            prior_receipt,
            prior_input,
            prior_payload,
            prior_result,
            prior_key_id,
            prior_public_key,
            prior_trust_domain,
            prior_trust_policy_sha256,
            receipt_id,
            issued_at,
            issuer_id,
            trust_domain,
            trust_policy_sha256,
            key_id,
            signing_key_file,
            output,
        } => {
            let context_bytes = read_bounded(
                &context,
                MAX_REVIEW_CACHE_JSON_BYTES,
                "review context bytes",
            )?;
            let review_input_bytes = read_bounded(
                &review_input,
                MAX_REVIEW_CACHE_JSON_BYTES,
                "review input bytes",
            )?;
            let selected_payload_bytes = read_bounded(
                &selected_payload,
                MAX_REVIEW_CACHE_JSON_BYTES,
                "selected payload bytes",
            )?;
            let result_bytes =
                read_bounded(&result, MAX_REVIEW_CACHE_JSON_BYTES, "review result bytes")?;
            let prior_receipt_bytes = prior_receipt
                .as_ref()
                .map(|path| read_bounded(path, MAX_REVIEW_CACHE_JSON_BYTES, "prior receipt bytes"))
                .transpose()?;
            let prior_input_bytes = prior_input
                .as_ref()
                .map(|path| {
                    read_bounded(
                        path,
                        MAX_REVIEW_CACHE_JSON_BYTES,
                        "prior review input bytes",
                    )
                })
                .transpose()?;
            let prior_payload_bytes = prior_payload
                .as_ref()
                .map(|path| {
                    read_bounded(
                        path,
                        MAX_REVIEW_CACHE_JSON_BYTES,
                        "prior selected payload bytes",
                    )
                })
                .transpose()?;
            let prior_result_bytes = prior_result
                .as_ref()
                .map(|path| {
                    read_bounded(
                        path,
                        MAX_REVIEW_CACHE_JSON_BYTES,
                        "prior review result bytes",
                    )
                })
                .transpose()?;
            let prior_receipt_bundle = match prior_receipt_bytes.as_deref() {
                Some(receipt) => Some(ReviewCacheReceiptBundle {
                    receipt,
                    prior_input: prior_input_bytes
                        .as_deref()
                        .context("--prior-receipt requires --prior-input")?,
                    prior_payload: prior_payload_bytes
                        .as_deref()
                        .context("--prior-receipt requires --prior-payload")?,
                    prior_result: prior_result_bytes
                        .as_deref()
                        .context("--prior-receipt requires --prior-result")?,
                    trusted_key_id: prior_key_id
                        .as_deref()
                        .context("--prior-receipt requires --prior-key-id")?,
                    trusted_public_key: prior_public_key
                        .as_deref()
                        .context("--prior-receipt requires --prior-public-key")?,
                    trust_domain: prior_trust_domain
                        .as_deref()
                        .context("--prior-receipt requires --prior-trust-domain")?,
                    trust_policy_sha256: prior_trust_policy_sha256
                        .as_deref()
                        .context("--prior-receipt requires --prior-trust-policy-sha256")?,
                }),
                None => None,
            };
            let signing_key_bytes =
                read_bounded(&signing_key_file, 66, "Ed25519 signing key file bytes")?;
            let signing_key_text = std::str::from_utf8(&signing_key_bytes)
                .context("Ed25519 signing key file is not UTF-8")?;
            let signing_key = signing_key_text
                .strip_suffix("\r\n")
                .or_else(|| signing_key_text.strip_suffix('\n'))
                .unwrap_or(signing_key_text);
            let receipt = issue_review_cache_receipt(ReviewCacheReceiptIssue {
                repository: &repo,
                expected_base: &expected_base,
                expected_head: &expected_head,
                current_context: &context_bytes,
                review_input: &review_input_bytes,
                selected_payload: &selected_payload_bytes,
                result: &result_bytes,
                prior_receipt: prior_receipt_bundle,
                receipt_id: &receipt_id,
                issued_at: &issued_at,
                issuer_id: &issuer_id,
                trust_domain: &trust_domain,
                trust_policy_sha256: &trust_policy_sha256,
                key_id: &key_id,
                signing_key,
            })?;
            std::fs::write(&output, receipt)
                .with_context(|| format!("failed to write {}", display_path(&output)))?;
            eprintln!(
                "wrote signed review cache receipt to {}",
                display_path(&output)
            );
        }
        Command::Diff {
            before,
            after,
            language,
            output,
            json,
        } => {
            let limits = VerificationLimits::default();
            let before_bytes =
                read_bounded(&before, limits.max_source_bytes, "before source bytes")?;
            let after_bytes = read_bounded(&after, limits.max_source_bytes, "after source bytes")?;
            let language = select_language(&before, &after, language)?;
            let report = analyze_bytes(
                before_bytes.clone(),
                after_bytes.clone(),
                before.to_string_lossy().into_owned(),
                after.to_string_lossy().into_owned(),
                language,
            )?;
            let encoded = serde_json::to_vec(&report)?;
            let report_limit = limits.max_report_bytes;
            if encoded.len() > report_limit {
                bail!(
                    "generated report bytes limit exceeded: observed {}, limit {report_limit}",
                    encoded.len()
                );
            }
            let terminal_encoded = if json {
                let terminal_encoded = escape_terminal_unsafe_json(&encoded);
                let output_len = terminal_encoded
                    .len()
                    .checked_add(1)
                    .context("terminal JSON output size exceeds usize capacity")?;
                if output_len > report_limit {
                    bail!(
                        "generated terminal JSON bytes limit exceeded: observed {output_len}, limit {report_limit}"
                    );
                }
                Some(terminal_encoded)
            } else {
                None
            };
            if let Some(path) = output {
                std::fs::write(&path, &encoded)
                    .with_context(|| format!("failed to write {}", display_path(&path)))?;
                eprintln!("wrote proof-carrying report to {}", display_path(&path));
            }
            if let Some(encoded) = terminal_encoded {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(encoded.as_bytes())?;
                stdout.write_all(b"\n")?;
            } else {
                print_summary(&report, &before_bytes, &after_bytes)?;
            }
        }
        Command::Verify {
            report,
            before,
            after,
        } => {
            let limits = VerificationLimits::default();
            let report_bytes = read_bounded(&report, limits.max_report_bytes, "report bytes")?;
            reject_legacy_schema(&report, &report_bytes)?;
            let before_bytes =
                read_bounded(&before, limits.max_source_bytes, "before source bytes")?;
            let after_bytes = read_bounded(&after, limits.max_source_bytes, "after source bytes")?;
            verify_report_bytes(&report_bytes, &before_bytes, &after_bytes, &limits)?;
            println!(
                "verified: patch reconstruction, parser manifest, relations, ambiguities, changes, and summary"
            );
        }
        Command::Apply {
            report,
            before,
            output,
        } => {
            let limits = VerificationLimits::default();
            let report_bytes = read_bounded(&report, limits.max_report_bytes, "report bytes")?;
            reject_legacy_schema(&report, &report_bytes)?;
            let before_bytes =
                read_bounded(&before, limits.max_source_bytes, "before source bytes")?;
            let (rebuilt, _) =
                verify_and_replay_report_bytes(&report_bytes, &before_bytes, &limits)
                    .with_context(|| {
                        format!(
                            "failed to verify and apply report {}",
                            display_path(&report)
                        )
                    })?;
            std::fs::write(&output, rebuilt)
                .with_context(|| format!("failed to write {}", display_path(&output)))?;
            println!("rebuilt certified target at {}", display_path(&output));
        }
        Command::View {
            before,
            after,
            language,
            port,
            no_open,
        } => {
            let limits = VerificationLimits::default();
            let before_bytes =
                read_bounded(&before, limits.max_source_bytes, "before source bytes")?;
            let after_bytes = read_bounded(&after, limits.max_source_bytes, "after source bytes")?;
            let language = select_language(&before, &after, language)?;
            let report = analyze_bytes(
                before_bytes.clone(),
                after_bytes.clone(),
                before.to_string_lossy().into_owned(),
                after.to_string_lossy().into_owned(),
                language,
            )?;
            viewer::serve(report, before_bytes, after_bytes, port, !no_open)?;
        }
        Command::Review {
            base,
            head,
            checkpoint,
            repo,
            format,
            output,
            review_delta_output,
            github_summary,
            github_annotations,
            fail_on_review_residue,
            workbench,
            port,
            no_open,
        } => {
            let review =
                review_git_range_with_checkpoint(&repo, &base, &head, checkpoint.as_deref())?;
            if workbench {
                return viewer::serve_review(review, repo, port, !no_open);
            }
            let resume_delta =
                if (github_annotations || fail_on_review_residue || review_delta_output.is_some())
                    && review.checkpoint.is_some()
                {
                    Some(review_git_resume_delta(&repo, &review)?)
                } else {
                    None
                };
            if github_summary {
                let summary_path = std::env::var_os("GITHUB_STEP_SUMMARY")
                    .context("--github-summary requires GITHUB_STEP_SUMMARY")?;
                let mut summary = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&summary_path)
                    .with_context(|| {
                        format!(
                            "failed to open GitHub step summary {}",
                            display_path(Path::new(&summary_path))
                        )
                    })?;
                summary.write_all(markdown_report(&review).as_bytes())?;
            }
            let encoded = match format {
                ReviewOutput::Markdown => markdown_report(&review).into_bytes(),
                ReviewOutput::Json => serde_json::to_vec(&review)?,
            };
            if let Some(path) = output {
                std::fs::write(&path, &encoded)
                    .with_context(|| format!("failed to write {}", display_path(&path)))?;
                eprintln!("wrote repository review to {}", display_path(&path));
            } else {
                let mut stdout = std::io::stdout().lock();
                match format {
                    ReviewOutput::Markdown => stdout.write_all(&encoded)?,
                    ReviewOutput::Json => {
                        stdout.write_all(escape_terminal_unsafe_json(&encoded).as_bytes())?;
                        stdout.write_all(b"\n")?;
                    }
                }
            }
            if let Some(path) = review_delta_output {
                let delta = resume_delta
                    .as_ref()
                    .context("review delta output requires a resolved checkpoint")?;
                let encoded = serde_json::to_vec(delta)?;
                std::fs::write(&path, encoded)
                    .with_context(|| format!("failed to write {}", display_path(&path)))?;
                eprintln!("wrote review delta to {}", display_path(&path));
            }
            if github_annotations {
                let mut stdout = std::io::stdout().lock();
                let annotations = resume_delta.as_ref().map_or_else(
                    || github_workflow_annotations(&review),
                    github_review_delta_annotations,
                );
                stdout.write_all(annotations.as_bytes())?;
            }
            if fail_on_review_residue {
                let delta = resume_delta
                    .as_ref()
                    .context("review residue gate requires a resolved checkpoint")?;
                let needs_review = delta.summary.needs_review_files;
                let gate_message = if needs_review == 1 {
                    "1 file needs review".to_owned()
                } else {
                    format!("{needs_review} files need review")
                };
                ensure!(
                    needs_review == 0,
                    "review delta gate is open: {gate_message}"
                );
            }
        }
    }
    Ok(())
}

struct GhCliReadinessApi {
    hostname: String,
    started: Instant,
}

impl GithubReadinessApi for GhCliReadinessApi {
    fn get(&mut self, endpoint: &str) -> Result<GithubReadinessApiResponse> {
        let remaining = GITHUB_READINESS_TOTAL_TIMEOUT
            .checked_sub(self.started.elapsed())
            .context("GitHub readiness collection exceeded its 10-minute deadline")?;
        ensure!(
            !remaining.is_zero(),
            "GitHub readiness collection exceeded its 10-minute deadline"
        );
        let mut command = ProcessCommand::new("gh");
        command
            .arg("api")
            .arg("--include")
            .arg("--method")
            .arg("GET")
            .arg("--hostname")
            .arg(&self.hostname)
            .arg("--header")
            .arg("Accept: application/vnd.github+json")
            .arg("--header")
            .arg(format!("X-GitHub-Api-Version: {GITHUB_API_VERSION}"))
            .arg(endpoint)
            .env_remove("GH_DEBUG")
            .env_remove("DEBUG")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("FORCE_COLOR")
            .env_remove("GH_FORCE_TTY")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_PAGER", "cat")
            .env("NO_COLOR", "1");
        let output = run_bounded_process(
            &mut command,
            MAX_READINESS_API_RESPONSE_BYTES + GITHUB_API_HEADER_BYTES + 4,
            64 * 1024,
            remaining.min(GITHUB_API_TIMEOUT),
            "gh api",
            None,
        )?;
        let response = match parse_gh_readiness_included_response(&output.stdout, endpoint) {
            Ok(response) => response,
            Err(error) if !output.status.success() => bail!(
                "gh api failed for {endpoint} with {}: {}; response parse error: {error:#}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim_end()
            ),
            Err(error) => return Err(error),
        };
        if !output.status.success() && successful_http_status(response.status) {
            bail!(
                "gh api failed for {endpoint} with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim_end()
            );
        }
        Ok(response)
    }
}

impl GithubPullRequestDoctorApi for GhCliReadinessApi {
    fn graphql(
        &mut self,
        query: &str,
        variables: &serde_json::Value,
    ) -> Result<GithubReadinessApiResponse> {
        let remaining = GITHUB_READINESS_TOTAL_TIMEOUT
            .checked_sub(self.started.elapsed())
            .context("GitHub readiness collection exceeded its 10-minute deadline")?;
        ensure!(
            !remaining.is_zero(),
            "GitHub readiness collection exceeded its 10-minute deadline"
        );
        let variables = variables
            .as_object()
            .context("GraphQL variables must be a JSON object")?;
        let mut command = ProcessCommand::new("gh");
        command
            .arg("api")
            .arg("graphql")
            .arg("--include")
            .arg("--hostname")
            .arg(&self.hostname)
            .arg("--raw-field")
            .arg(format!("query={query}"));
        for (name, value) in variables {
            ensure!(
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
                "GraphQL variable name is invalid"
            );
            match value {
                serde_json::Value::String(value) => {
                    command.arg("--raw-field").arg(format!("{name}={value}"));
                }
                serde_json::Value::Number(value) => {
                    command.arg("--field").arg(format!("{name}={value}"));
                }
                serde_json::Value::Bool(value) => {
                    command.arg("--field").arg(format!("{name}={value}"));
                }
                _ => bail!("GraphQL variable {name} must be a string, number, or boolean"),
            }
        }
        command
            .env_remove("GH_DEBUG")
            .env_remove("DEBUG")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("FORCE_COLOR")
            .env_remove("GH_FORCE_TTY")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_PAGER", "cat")
            .env("NO_COLOR", "1");
        let output = run_bounded_process(
            &mut command,
            MAX_READINESS_API_RESPONSE_BYTES + GITHUB_API_HEADER_BYTES + 4,
            64 * 1024,
            remaining.min(GITHUB_API_TIMEOUT),
            "gh api graphql",
            None,
        )?;
        let response = match parse_gh_graphql_included_response(&output.stdout) {
            Ok(response) => response,
            Err(error) if !output.status.success() => bail!(
                "gh api graphql failed with {}: {}; response parse error: {error:#}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim_end()
            ),
            Err(error) => return Err(error),
        };
        if !output.status.success() && successful_http_status(response.status) {
            bail!(
                "gh api graphql failed with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim_end()
            );
        }
        Ok(response)
    }
}

fn successful_http_status(status: u16) -> bool {
    (200..300).contains(&status)
}

fn parse_gh_readiness_included_response(
    included: &[u8],
    endpoint: &str,
) -> Result<GithubReadinessApiResponse> {
    parse_gh_json_included_response(included, endpoint, true)
}

fn parse_gh_graphql_included_response(included: &[u8]) -> Result<GithubReadinessApiResponse> {
    parse_gh_json_included_response(included, "graphql", false)
}

fn parse_gh_json_included_response(
    included: &[u8],
    endpoint: &str,
    require_selected_api_version: bool,
) -> Result<GithubReadinessApiResponse> {
    let (header_end, delimiter_len) = find_header_boundary(included)
        .context("gh api --include response did not contain a header boundary")?;
    ensure!(
        header_end <= GITHUB_API_HEADER_BYTES,
        "gh api response headers exceeded {GITHUB_API_HEADER_BYTES} bytes"
    );
    let body = included[header_end + delimiter_len..].to_vec();
    ensure!(
        body.len() <= MAX_READINESS_API_RESPONSE_BYTES,
        "gh api response body bytes limit exceeded for {endpoint}: observed {}, limit {MAX_READINESS_API_RESPONSE_BYTES}",
        body.len()
    );
    let headers = std::str::from_utf8(&included[..header_end])
        .context("gh api response headers were not valid UTF-8")?;
    let mut lines = headers.lines();
    let status_line = lines
        .next()
        .context("gh api response status line is missing")?;
    let mut status_parts = status_line.trim_end_matches('\r').split_ascii_whitespace();
    let protocol = status_parts.next().unwrap_or_default();
    let status = status_parts
        .next()
        .context("gh api response status is missing")?
        .parse::<u16>()
        .context("gh api response status is invalid")?;
    ensure!(
        protocol.starts_with("HTTP/") && (100..=599).contains(&status),
        "gh api returned a malformed included status for {endpoint}: {status_line}"
    );

    let mut content_type = None;
    let mut link_header = None;
    let mut selected_api_version = None;
    for line in lines {
        let line = line.trim_end_matches('\r');
        ensure!(
            !line.is_empty() && !line.starts_with([' ', '\t']),
            "gh api returned a malformed response header for {endpoint}"
        );
        let (name, value) = line
            .split_once(':')
            .context("gh api returned a malformed response header")?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-type") {
            ensure!(
                content_type.replace(value).is_none(),
                "gh api returned duplicate Content-Type headers for {endpoint}"
            );
        } else if name.eq_ignore_ascii_case("link") {
            ensure!(
                link_header.replace(value.to_owned()).is_none(),
                "gh api returned duplicate Link headers for {endpoint}"
            );
        } else if name.eq_ignore_ascii_case("x-github-api-version-selected") {
            ensure!(
                selected_api_version.replace(value).is_none(),
                "gh api returned duplicate X-GitHub-Api-Version-Selected headers for {endpoint}"
            );
        }
    }
    let content_type = content_type.context("gh api response is missing Content-Type")?;
    ensure!(
        content_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json")),
        "gh api returned unsupported Content-Type {content_type} for {endpoint}"
    );
    if require_selected_api_version {
        let selected_api_version = selected_api_version
            .context("gh api response is missing X-GitHub-Api-Version-Selected")?;
        ensure!(
            selected_api_version == GITHUB_API_VERSION,
            "gh api selected version {selected_api_version} for {endpoint}, expected {GITHUB_API_VERSION}"
        );
    }
    Ok(GithubReadinessApiResponse {
        status,
        body,
        link_header,
    })
}

struct GhCliOwnershipApi {
    hostname: String,
    started: Instant,
}

impl GithubOwnershipApi for GhCliOwnershipApi {
    fn get(
        &mut self,
        endpoint: &str,
        media_type: GithubOwnershipMediaType,
    ) -> Result<GithubOwnershipApiResponse> {
        let remaining = GITHUB_OWNERSHIP_TOTAL_TIMEOUT
            .checked_sub(self.started.elapsed())
            .context("GitHub ownership collection exceeded its 10-minute deadline")?;
        ensure!(
            !remaining.is_zero(),
            "GitHub ownership collection exceeded its 10-minute deadline"
        );
        let mut command = ProcessCommand::new("gh");
        command
            .arg("api")
            .arg("--include")
            .arg("--method")
            .arg("GET")
            .arg("--hostname")
            .arg(&self.hostname)
            .arg("--header")
            .arg(format!("Accept: {}", media_type.accept_header()))
            .arg("--header")
            .arg(format!("X-GitHub-Api-Version: {GITHUB_API_VERSION}"))
            .arg(endpoint)
            .env_remove("GH_DEBUG")
            .env_remove("DEBUG")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("FORCE_COLOR")
            .env_remove("GH_FORCE_TTY")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_PAGER", "cat")
            .env("NO_COLOR", "1");
        let output = run_bounded_process(
            &mut command,
            MAX_GITHUB_OWNERSHIP_API_RESPONSE_BYTES + GITHUB_API_HEADER_BYTES + 4,
            64 * 1024,
            remaining.min(GITHUB_API_TIMEOUT),
            "gh api",
            None,
        )?;
        ensure!(
            output.status.success(),
            "gh api failed for {endpoint} with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
        parse_gh_included_response(&output.stdout, endpoint)
    }
}

fn parse_gh_included_response(
    included: &[u8],
    endpoint: &str,
) -> Result<GithubOwnershipApiResponse> {
    let (header_end, delimiter_len) = find_header_boundary(included)
        .context("gh api --include response did not contain a header boundary")?;
    ensure!(
        header_end <= GITHUB_API_HEADER_BYTES,
        "gh api response headers exceeded {GITHUB_API_HEADER_BYTES} bytes"
    );
    let header_bytes = &included[..header_end];
    let body = included[header_end + delimiter_len..].to_vec();
    ensure!(
        body.len() <= MAX_GITHUB_OWNERSHIP_API_RESPONSE_BYTES,
        "gh api response body bytes limit exceeded for {endpoint}: observed {}, limit {MAX_GITHUB_OWNERSHIP_API_RESPONSE_BYTES}",
        body.len()
    );

    let headers = std::str::from_utf8(header_bytes)
        .context("gh api response headers were not valid UTF-8")?;
    let mut lines = headers.lines();
    let status_line = lines
        .next()
        .context("gh api response status line is missing")?;
    let mut status_parts = status_line.trim_end_matches('\r').split_ascii_whitespace();
    let protocol = status_parts.next().unwrap_or_default();
    let status = status_parts.next().unwrap_or_default();
    ensure!(
        protocol.starts_with("HTTP/") && status == "200",
        "gh api returned malformed or unexpected included status for {endpoint}: {status_line}"
    );

    let mut content_type = None;
    let mut link_header = None;
    let mut selected_api_version = None;
    for line in lines {
        let line = line.trim_end_matches('\r');
        ensure!(
            !line.is_empty() && !line.starts_with([' ', '\t']),
            "gh api returned a malformed response header for {endpoint}"
        );
        let (name, value) = line
            .split_once(':')
            .context("gh api returned a malformed response header")?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-type") {
            ensure!(
                content_type.replace(value).is_none(),
                "gh api returned duplicate Content-Type headers for {endpoint}"
            );
        } else if name.eq_ignore_ascii_case("link") {
            ensure!(
                link_header.replace(value.to_owned()).is_none(),
                "gh api returned duplicate Link headers for {endpoint}"
            );
        } else if name.eq_ignore_ascii_case("x-github-api-version-selected") {
            ensure!(
                selected_api_version.replace(value).is_none(),
                "gh api returned duplicate X-GitHub-Api-Version-Selected headers for {endpoint}"
            );
        }
    }
    let content_type = content_type.context("gh api response is missing Content-Type")?;
    ensure!(
        content_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json")),
        "gh api returned unsupported Content-Type {content_type} for {endpoint}"
    );
    let selected_api_version =
        selected_api_version.context("gh api response is missing X-GitHub-Api-Version-Selected")?;
    ensure!(
        selected_api_version == GITHUB_API_VERSION,
        "gh api selected version {selected_api_version} for {endpoint}, expected {GITHUB_API_VERSION}"
    );
    Ok(GithubOwnershipApiResponse { body, link_header })
}

fn find_header_boundary(included: &[u8]) -> Option<(usize, usize)> {
    let crlf = included.windows(4).position(|window| window == b"\r\n\r\n");
    let lf = included.windows(2).position(|window| window == b"\n\n");
    match (crlf, lf) {
        (Some(crlf), Some(lf)) if lf < crlf => Some((lf, 2)),
        (Some(crlf), _) => Some((crlf, 4)),
        (None, Some(lf)) => Some((lf, 2)),
        (None, None) => None,
    }
}

fn select_language(before: &Path, after: &Path, requested: Option<Language>) -> Result<Language> {
    if let Some(language) = requested {
        return Ok(language);
    }
    let before_language = Language::detect(before)?;
    let after_language = Language::detect(after)?;
    if before_language != after_language {
        bail!(
            "input languages differ ({before_language:?} and {after_language:?}); pass --language only when both files use the same grammar"
        );
    }
    Ok(before_language)
}

fn read_bounded(path: &Path, limit: usize, label: &str) -> Result<Vec<u8>> {
    let read_limit = limit
        .checked_add(1)
        .with_context(|| format!("{label} limit cannot be incremented safely"))?;
    let read_limit = u64::try_from(read_limit)
        .with_context(|| format!("{label} limit cannot be represented by the file reader"))?;
    let file =
        File::open(path).with_context(|| format!("failed to read {}", display_path(path)))?;
    let mut bytes = Vec::new();
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {}", display_path(path)))?;
    if bytes.len() > limit {
        bail!(
            "{label} limit exceeded: observed at least {}, limit {limit}",
            bytes.len()
        );
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct ReportSchemaEnvelope {
    schema: Option<String>,
}

fn reject_legacy_schema(path: &Path, bytes: &[u8]) -> Result<()> {
    let envelope: ReportSchemaEnvelope = serde_json::from_slice(bytes)
        .with_context(|| format!("failed to parse {} as JSON", display_path(path)))?;
    if envelope.schema.as_deref() == Some(LEGACY_REPORT_SCHEMA_V1) {
        bail!(
            "report schema v1 cannot represent coupled ambiguity constraints or be losslessly upgraded; rerun StrataDiff on the original snapshots to create a v3 report"
        );
    }
    if envelope.schema.as_deref() == Some(LEGACY_REPORT_SCHEMA_V2) {
        bail!(
            "report schema v2 uses the previous parser and patch contracts and cannot be verified as v3; rerun StrataDiff on the original snapshots to create a v3 report"
        );
    }
    Ok(())
}

fn print_summary(report: &DiffReport, before: &[u8], after: &[u8]) -> Result<()> {
    println!(
        "{} -> {} ({:?})",
        display_text(&report.before.path),
        display_text(&report.after.path),
        report.parser.language
    );
    print_exact_byte_edits(report, before, after)?;
    println!(
        "{} model-forced relations, {} suggestions, {} ambiguity groups, {} structural changes",
        report.summary.model_forced_relations,
        report.summary.suggested_relations,
        report.summary.ambiguity_groups,
        report.summary.structural_changes
    );
    for change in &report.changes {
        let before = change
            .before
            .as_ref()
            .map(|node| {
                format!(
                    "{}@{}..{}",
                    node.kind, node.span.start_byte, node.span.end_byte
                )
            })
            .unwrap_or_else(|| "-".to_owned());
        let after = change
            .after
            .as_ref()
            .map(|node| {
                format!(
                    "{}@{}..{}",
                    node.kind, node.span.start_byte, node.span.end_byte
                )
            })
            .unwrap_or_else(|| "-".to_owned());
        println!("  {:?}: {before} -> {after}", change.kind);
    }
    for ambiguity in &report.ambiguities {
        match &ambiguity.constraint {
            AmbiguityConstraint::ExactOrderedAlignment {
                required_matches,
                possible_pairs,
                ..
            } => println!(
                "  ambiguous: choose {required_matches} ordered matches from {} explicit pairs under nodes {} -> {}",
                possible_pairs.len(),
                ambiguity.parent_before,
                ambiguity.parent_after
            ),
            AmbiguityConstraint::SymbolicAbstention { cause, .. } => println!(
                "  ambiguous: abstained from pair claims for {} -> {} endpoints under nodes {} -> {} ({cause:?})",
                ambiguity.before.len(),
                ambiguity.after.len(),
                ambiguity.parent_before,
                ambiguity.parent_after
            ),
        }
    }
    println!(
        "patch reconstruction certificate: {}",
        if report.certificate.patch_verified {
            "verified"
        } else {
            "invalid"
        }
    );
    Ok(())
}

fn print_exact_byte_edits(report: &DiffReport, before: &[u8], after: &[u8]) -> Result<()> {
    let stdout = std::io::stdout();
    write_exact_byte_edits(&mut stdout.lock(), report, before, after)
}

fn write_exact_byte_edits(
    output: &mut impl Write,
    report: &DiffReport,
    before: &[u8],
    after: &[u8],
) -> Result<()> {
    let rebuilt = apply_patch(before, &report.patch)?;
    ensure!(
        rebuilt == after,
        "internal invariant failed: displayed byte edits do not reconstruct the target"
    );
    if report.patch.edits.is_empty() {
        writeln!(output, "exact byte diff: no changes")?;
        return Ok(());
    }

    writeln!(output, "exact byte edits ({}):", report.patch.edits.len())?;
    let mut old_cursor = 0_usize;
    let mut new_cursor = 0_usize;
    for edit in &report.patch.edits {
        let unchanged = edit
            .old_start
            .checked_sub(old_cursor)
            .context("patch edits are not ordered")?;
        let new_start = new_cursor
            .checked_add(unchanged)
            .context("displayed after offset exceeds usize capacity")?;
        let replacement = STANDARD
            .decode(&edit.replacement_base64)
            .context("generated patch replacement is not valid base64")?;
        let new_end = new_start
            .checked_add(replacement.len())
            .context("displayed after range exceeds usize capacity")?;
        let removed = before
            .get(edit.old_start..edit.old_end)
            .context("generated patch edit is outside the before snapshot")?;
        writeln!(
            output,
            "  @@ before bytes {}..{} -> after bytes {new_start}..{new_end} @@",
            edit.old_start, edit.old_end
        )?;
        writeln!(output, "  - {}", display_bytes(removed))?;
        writeln!(output, "  + {}", display_bytes(&replacement))?;
        old_cursor = edit.old_end;
        new_cursor = new_end;
    }
    Ok(())
}

fn display_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => format!("utf8 {}", display_text(text)),
        Err(_) => format!("base64 \"{}\"", STANDARD.encode(bytes)),
    }
}

fn display_path(path: &Path) -> String {
    display_text(&path.to_string_lossy())
}

fn escape_terminal_unsafe_json(encoded: &[u8]) -> String {
    let json = std::str::from_utf8(encoded).expect("serde_json always emits UTF-8");
    escape_terminal_unsafe_text(json)
}

fn escape_terminal_unsafe_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if is_terminal_unsafe(character) {
            push_json_unicode_escape(&mut escaped, character);
        } else {
            escaped.push(character);
        }
    }
    escaped
}

fn display_text(text: &str) -> String {
    let json = serde_json::to_string(text).expect("a UTF-8 string always serializes as JSON");
    escape_terminal_unsafe_text(&json)
}

fn is_terminal_unsafe(character: char) -> bool {
    // JSON may emit DEL, C1, line separators, and Unicode format controls literally.
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

fn push_json_unicode_escape(output: &mut String, character: char) {
    let codepoint = character as u32;
    if codepoint <= 0xffff {
        output.push_str(&format!("\\u{codepoint:04x}"));
        return;
    }

    let surrogate = codepoint - 0x1_0000;
    let high = 0xd800 + (surrogate >> 10);
    let low = 0xdc00 + (surrogate & 0x3ff);
    output.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
}

#[cfg(test)]
mod tests {
    use std::fs;

    use base64::{Engine, engine::general_purpose::STANDARD};
    use proptest::prelude::*;
    use stratadiff::{ByteEdit, Language, analyze_bytes};

    use super::{
        display_bytes, display_text, escape_terminal_unsafe_json, is_terminal_unsafe,
        parse_gh_graphql_included_response, parse_gh_included_response, read_bounded,
        write_exact_byte_edits,
    };

    #[test]
    fn bounded_reader_accepts_limit_and_rejects_one_more_byte() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("input.bin");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(read_bounded(&path, 3, "test bytes").unwrap(), b"abc");

        let error = read_bounded(&path, 2, "test bytes").unwrap_err();
        assert_eq!(
            error.to_string(),
            "test bytes limit exceeded: observed at least 3, limit 2"
        );
    }

    #[test]
    fn included_github_response_separates_headers_body_and_link() {
        let included = concat!(
            "HTTP/2.0 200 OK\n",
            "Content-Type: application/json; charset=utf-8\r\n",
            "X-GitHub-Api-Version-Selected: 2022-11-28\r\n",
            "Link: <https://api.github.com/items?page=2>; rel=\"next\"\r\n",
            "\r\n",
            "{\"ok\":true}"
        );

        let response = parse_gh_included_response(included.as_bytes(), "items?page=1").unwrap();

        assert_eq!(response.body, br#"{"ok":true}"#);
        assert_eq!(
            response.link_header.as_deref(),
            Some("<https://api.github.com/items?page=2>; rel=\"next\"")
        );
    }

    #[test]
    fn included_github_response_requires_json_content_type() {
        let included = b"HTTP/2.0 200 OK\r\nContent-Type: text/html\r\n\r\n{}";

        let error = parse_gh_included_response(included, "items").unwrap_err();

        assert!(error.to_string().contains("unsupported Content-Type"));
    }

    #[test]
    fn included_github_response_requires_the_selected_api_version() {
        let included = b"HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n{}";

        let error = parse_gh_included_response(included, "items").unwrap_err();

        assert!(
            error
                .to_string()
                .contains("missing X-GitHub-Api-Version-Selected")
        );
    }

    #[test]
    fn included_graphql_response_does_not_require_a_rest_api_version_header() {
        let included = b"HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"data\":{}}";

        let response = parse_gh_graphql_included_response(included).unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.body, br#"{"data":{}}"#);
        assert!(response.link_header.is_none());
    }

    #[test]
    fn display_bytes_is_lossless_and_terminal_safe() {
        assert_eq!(display_bytes(b"line\r\n"), "utf8 \"line\\r\\n\"");
        assert_eq!(display_bytes("中文🙂".as_bytes()), "utf8 \"中文🙂\"");
        assert_eq!(display_bytes(&[0xff, 0x00]), "base64 \"/wA=\"");

        let controls = "\u{1b}[31mred\u{1b}[0m\u{7f}\u{85}\u{9b}\u{61c}\u{200b}\u{2028}\u{202e}\u{2066}\u{feff}\u{e0001}";
        let displayed = display_bytes(controls.as_bytes());
        assert_eq!(
            displayed,
            "utf8 \"\\u001b[31mred\\u001b[0m\\u007f\\u0085\\u009b\\u061c\\u200b\\u2028\\u202e\\u2066\\ufeff\\udb40\\udc01\""
        );
        assert!(!displayed.chars().any(is_terminal_unsafe));
        let json = displayed.strip_prefix("utf8 ").unwrap();
        assert_eq!(serde_json::from_str::<String>(json).unwrap(), controls);
    }

    #[test]
    fn displayed_text_quotes_terminal_control_characters() {
        let text = "before\u{1b}[31m\n\u{202e}.py";
        let displayed = display_text(text);

        assert_eq!(displayed, "\"before\\u001b[31m\\n\\u202e.py\"");
        assert!(!displayed.chars().any(is_terminal_unsafe));
        assert_eq!(serde_json::from_str::<String>(&displayed).unwrap(), text);
    }

    #[test]
    fn terminal_json_escaping_preserves_the_json_value() {
        let value = serde_json::json!({"path": "x\u{9b}\u{202e}\u{e0001}.py"});
        let encoded = serde_json::to_vec(&value).unwrap();
        let displayed = escape_terminal_unsafe_json(&encoded);

        assert!(!displayed.chars().any(is_terminal_unsafe));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&displayed).unwrap(),
            value
        );
    }

    proptest! {
        #[test]
        fn displayed_bytes_round_trip_without_terminal_controls(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
            let displayed = display_bytes(&bytes);
            prop_assert!(!displayed.chars().any(is_terminal_unsafe));

            let decoded = if let Some(json) = displayed.strip_prefix("utf8 ") {
                serde_json::from_str::<String>(json).unwrap().into_bytes()
            } else {
                let encoded = displayed
                    .strip_prefix("base64 \"")
                    .unwrap()
                    .strip_suffix('"')
                    .unwrap();
                STANDARD.decode(encoded).unwrap()
            };
            prop_assert_eq!(decoded, bytes);
        }
    }

    #[test]
    fn exact_byte_edit_offsets_include_prior_length_changes() {
        let before = b"0123456789";
        let after = "01中文456X9".as_bytes();
        let mut report = analyze_bytes(
            before.to_vec(),
            after.to_vec(),
            "before.bin".to_owned(),
            "after.bin".to_owned(),
            Language::Universal,
        )
        .unwrap();
        report.patch.edits = vec![
            ByteEdit {
                old_start: 2,
                old_end: 4,
                replacement_base64: "5Lit5paH".to_owned(),
            },
            ByteEdit {
                old_start: 7,
                old_end: 9,
                replacement_base64: "WA==".to_owned(),
            },
        ];

        let mut output = Vec::new();
        write_exact_byte_edits(&mut output, &report, before, after).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            concat!(
                "exact byte edits (2):\n",
                "  @@ before bytes 2..4 -> after bytes 2..8 @@\n",
                "  - utf8 \"23\"\n",
                "  + utf8 \"中文\"\n",
                "  @@ before bytes 7..9 -> after bytes 11..12 @@\n",
                "  - utf8 \"78\"\n",
                "  + utf8 \"X\"\n",
            )
        );
    }
}
