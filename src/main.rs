use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
        mpsc,
    },
    thread,
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
    write_github_ownership_snapshot,
};
use stratadiff::ledger::{
    GithubReviewLedger, GithubWebhookIngest, IngestOutcome, MAX_GITHUB_LEDGER_BYTES,
    MAX_GITHUB_WEBHOOK_BYTES, decode_ed25519_signing_key, ingest_github_webhook,
};
use stratadiff::ownership::{
    GithubOwnershipSnapshot, MAX_OWNERSHIP_SNAPSHOT_BYTES, github_provider_hostname,
};
use stratadiff::review::{
    github_review_delta_annotations, github_workflow_annotations, markdown_report,
    review_git_range_with_checkpoint, review_git_resume_delta,
};
use stratadiff::{
    AmbiguityConstraint, DiffReport, Language, VerificationLimits, analyze_bytes, apply_patch,
    verify_and_replay_report_bytes, verify_report_bytes,
};

mod viewer;

const LEGACY_REPORT_SCHEMA_V1: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v1.schema.json";
const LEGACY_REPORT_SCHEMA_V2: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v2.schema.json";
const BUILD_INFO_SCHEMA: &str = "stratadiff-build-info-v1";
const BUILD_GIT_REVISION: &str = env!("STRATADIFF_BUILD_GIT_REVISION");
const BUILD_GIT_DIRTY: &str = env!("STRATADIFF_BUILD_GIT_DIRTY");
const BUILD_CARGO_LOCK_SHA256: &str = env!("STRATADIFF_BUILD_CARGO_LOCK_SHA256");
const BUILD_PROFILE: &str = env!("STRATADIFF_BUILD_PROFILE");
const BUILD_RUSTC_VERSION: &str = env!("STRATADIFF_BUILD_RUSTC_VERSION");
const GITHUB_API_HEADER_BYTES: usize = 64 * 1024;
const GITHUB_API_TIMEOUT: Duration = Duration::from_secs(30);
const GITHUB_OWNERSHIP_TOTAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const PROCESS_PIPE_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

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

#[derive(Debug)]
struct BoundedProcessOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_bounded_process(
    command: &mut ProcessCommand,
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
    label: &str,
) -> Result<BoundedProcessOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {label}"))?;
    let stdout = child
        .stdout
        .take()
        .with_context(|| format!("failed to capture {label} stdout"))?;
    let stderr = child
        .stderr
        .take()
        .with_context(|| format!("failed to capture {label} stderr"))?;
    let overflow = Arc::new(AtomicU8::new(0));
    let stdout_overflow = Arc::clone(&overflow);
    let stderr_overflow = Arc::clone(&overflow);
    let (stdout_sender, stdout_receiver) = mpsc::sync_channel(1);
    let (stderr_sender, stderr_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = stdout_sender.send(read_capped_stream(
            stdout,
            stdout_limit,
            &stdout_overflow,
            1,
        ));
    });
    thread::spawn(move || {
        let _ = stderr_sender.send(read_capped_stream(
            stderr,
            stderr_limit,
            &stderr_overflow,
            2,
        ));
    });
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if overflow.load(Ordering::Acquire) != 0 {
            break terminate_child(&mut child, label)?;
        }
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed to wait for {label}"))?
        {
            terminate_remaining_process_group(&child, label)?;
            break status;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            break terminate_child(&mut child, label)?;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let pipe_deadline = Instant::now() + PROCESS_PIPE_CLOSE_TIMEOUT;
    let (stdout, stdout_exceeded) =
        receive_bounded_stream(&stdout_receiver, pipe_deadline, &format!("{label} stdout"))?;
    let (stderr, stderr_exceeded) =
        receive_bounded_stream(&stderr_receiver, pipe_deadline, &format!("{label} stderr"))?;
    ensure!(
        !stdout_exceeded,
        "{label} stdout bytes limit exceeded: limit {stdout_limit}"
    );
    ensure!(
        !stderr_exceeded,
        "{label} stderr bytes limit exceeded: limit {stderr_limit}"
    );
    ensure!(
        !timed_out,
        "{label} timed out after {} milliseconds",
        timeout.as_millis()
    );
    Ok(BoundedProcessOutput {
        status,
        stdout,
        stderr,
    })
}

fn receive_bounded_stream(
    receiver: &mpsc::Receiver<std::io::Result<(Vec<u8>, bool)>>,
    deadline: Instant,
    label: &str,
) -> Result<(Vec<u8>, bool)> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    receiver
        .recv_timeout(remaining)
        .with_context(|| format!("{label} pipe did not close after the child exited"))?
        .with_context(|| format!("failed to read {label}"))
}

fn terminate_child(
    child: &mut std::process::Child,
    label: &str,
) -> Result<std::process::ExitStatus> {
    terminate_remaining_process_group(child, label)?;
    match child.kill() {
        Ok(()) => child
            .wait()
            .with_context(|| format!("failed to reap {label} after killing it")),
        Err(kill_error) => match child
            .try_wait()
            .with_context(|| format!("failed to inspect {label} after kill failed"))?
        {
            Some(status) => Ok(status),
            None => Err(kill_error).with_context(|| format!("failed to kill {label}")),
        },
    }
}

#[cfg(unix)]
fn terminate_remaining_process_group(child: &std::process::Child, label: &str) -> Result<()> {
    let process_group = i32::try_from(child.id()).context("child process ID exceeds i32")?;
    // The command is spawned as its own process-group leader immediately above.
    let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error).with_context(|| format!("failed to terminate remaining {label} processes"))
    }
}

#[cfg(not(unix))]
fn terminate_remaining_process_group(_child: &std::process::Child, _label: &str) -> Result<()> {
    Ok(())
}

fn read_capped_stream(
    mut reader: impl Read,
    limit: usize,
    overflow: &AtomicU8,
    overflow_bit: u8,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::new();
    let mut exceeded = false;
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let available = limit.saturating_sub(retained.len());
        let keep = available.min(read);
        retained.extend_from_slice(&buffer[..keep]);
        exceeded |= keep < read;
        if exceeded {
            overflow.fetch_or(overflow_bit, Ordering::Release);
        }
    }
    Ok((retained, exceeded))
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
    use std::{
        fs,
        process::Command,
        time::{Duration, Instant},
    };

    use base64::{Engine, engine::general_purpose::STANDARD};
    use proptest::prelude::*;
    use stratadiff::{ByteEdit, Language, analyze_bytes};

    use super::{
        display_bytes, display_text, escape_terminal_unsafe_json, is_terminal_unsafe,
        parse_gh_included_response, read_bounded, run_bounded_process, write_exact_byte_edits,
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

    #[cfg(unix)]
    #[test]
    fn bounded_process_terminates_after_its_deadline() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do :; done"]);
        let started = Instant::now();

        let error = run_bounded_process(
            &mut command,
            1024,
            1024,
            Duration::from_millis(100),
            "timeout fixture",
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("timed out after 100 milliseconds")
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn bounded_process_closes_pipes_held_by_descendants() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & exit 0"]);
        let started = Instant::now();

        let output = run_bounded_process(
            &mut command,
            1024,
            1024,
            Duration::from_secs(5),
            "descendant fixture",
        )
        .unwrap();

        assert!(output.status.success());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bounded_process_does_not_wait_forever_for_an_escaped_descendant_pipe() {
        let directory = tempfile::tempdir().unwrap();
        let pid_path = directory.path().join("escaped.pid");
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "setsid sh -c 'printf \"%s\" \"$$\" > \"$ESCAPED_PID_PATH\"; sleep 30' & while [ ! -s \"$ESCAPED_PID_PATH\" ]; do :; done",
            ])
            .env("ESCAPED_PID_PATH", &pid_path);
        let started = Instant::now();

        let error = run_bounded_process(
            &mut command,
            1024,
            1024,
            Duration::from_secs(5),
            "escaped descendant fixture",
        )
        .unwrap_err();
        let escaped_pid = fs::read_to_string(pid_path)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        // The fixture deliberately escapes the managed process group; clean it up explicitly.
        unsafe {
            libc::kill(-escaped_pid, libc::SIGKILL);
        }

        assert!(error.to_string().contains("pipe did not close"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn bounded_process_stops_when_output_exceeds_its_limit() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do printf 0123456789; done"]);
        let started = Instant::now();

        let error = run_bounded_process(
            &mut command,
            1024,
            1024,
            Duration::from_secs(5),
            "output fixture",
        )
        .unwrap_err();

        assert!(error.to_string().contains("stdout bytes limit exceeded"));
        assert!(started.elapsed() < Duration::from_secs(2));
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
