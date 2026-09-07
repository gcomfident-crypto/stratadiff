use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use stratadiff::inbox_event::InboxEventEnvelope;

#[cfg(unix)]
use std::os::unix::{
    fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    io::AsRawFd,
};

const EVENT_SCHEMA: &str = "stratadiff-value-funnel-event-v1";
const REPORT_SCHEMA: &str = "stratadiff-value-funnel-report-v1";
const ZERO_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_LOG_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVENTS: usize = 100_000;
const MAX_BASELINE_CANDIDATES: u64 = 100;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ValueReportFormat {
    Markdown,
    Json,
}

#[derive(Debug, Args)]
pub(crate) struct ValueReportArgs {
    /// Local opt-in value-funnel log produced by `inbox --value-log`.
    log: PathBuf,
    /// Aggregate report format. Reports never contain transition identifiers.
    #[arg(long, value_enum, default_value_t = ValueReportFormat::Markdown)]
    format: ValueReportFormat,
    /// Write the aggregate report to a private file instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StoredEvent {
    schema: String,
    tool_version: String,
    sequence: u64,
    observed_at_unix_seconds: u64,
    event_id: String,
    previous_event_sha256: String,
    #[serde(flatten)]
    event: FunnelEvent,
    event_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct UnsignedEvent {
    schema: String,
    tool_version: String,
    sequence: u64,
    observed_at_unix_seconds: u64,
    event_id: String,
    previous_event_sha256: String,
    #[serde(flatten)]
    event: FunnelEvent,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
enum FunnelEvent {
    Baseline {
        scan_id: String,
        inspected_candidates: u64,
        completed_review_prs: u64,
        collection_complete: bool,
    },
    CoveredTransition {
        transition_id: String,
        attempt_id: String,
    },
    GapDiscovery {
        scan_id: String,
        transition_id: String,
    },
    InboxDelivery {
        scan_id: String,
    },
    ResumeInvoked {
        transition_id: String,
        attempt_id: String,
    },
    TransitionBound {
        transition_id: String,
        attempt_id: String,
    },
    WorkbenchReady {
        transition_id: String,
        attempt_id: String,
    },
    AttemptFailed {
        transition_id: String,
        attempt_id: String,
        stage: FailureStage,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FailureStage {
    BeforeTransitionBound,
    BeforeCoverage,
    BeforeWorkbenchReady,
    AfterWorkbenchReady,
}

#[derive(Debug, Serialize)]
struct ValueReport {
    schema: &'static str,
    tool_version: &'static str,
    source_schema: &'static str,
    event_count: usize,
    chain_tip_sha256: String,
    privacy: ReportPrivacy,
    claim_boundary: ClaimBoundary,
    summary: ReportSummary,
    conversion: ReportConversion,
    failures: ReportFailures,
}

#[derive(Debug, Serialize)]
struct ReportPrivacy {
    automatic_upload: bool,
    source_content_exported: bool,
    local_paths_exported: bool,
    repository_identity_exported: bool,
    pull_request_identity_exported: bool,
    reviewer_identity_exported: bool,
    pseudonymous_transition_digests_exported: bool,
    transition_identifiers_exported: bool,
}

#[derive(Debug, Serialize)]
struct ClaimBoundary {
    local_post_discovery_activation_supported: bool,
    clean_install_success_supported: bool,
    reviewer_time_savings_supported: bool,
    issue_recall_supported: bool,
    market_prevalence_supported: bool,
}

#[derive(Debug, Serialize)]
struct ReportSummary {
    scans: u64,
    partial_scans: u64,
    delivery_confirmed_scans: u64,
    delivery_unconfirmed_scans: u64,
    inspected_candidates: u64,
    completed_review_checkpoints: u64,
    covered_transitions: u64,
    gap_discoveries: u64,
    unique_gap_transitions: u64,
    delivered_gap_discoveries: u64,
    unique_delivered_gap_transitions: u64,
    unique_resumed_transitions: u64,
    unique_ready_transitions: u64,
    resume_attempts: u64,
    transition_bound_attempts: u64,
    covered_attempts: u64,
    workbench_ready_attempts: u64,
    unresolved_attempts: u64,
}

#[derive(Debug, Serialize)]
struct ReportConversion {
    delivered_gap_to_resume: Rate,
    resume_attempt_to_covered_transition: Rate,
    covered_transition_to_workbench_ready: Rate,
    resume_attempt_to_workbench_ready: Rate,
}

#[derive(Debug, Serialize)]
struct ReportFailures {
    total: u64,
    before_transition_bound: u64,
    before_coverage: u64,
    before_workbench_ready: u64,
    after_workbench_ready: u64,
}

#[derive(Debug, Serialize)]
struct Rate {
    numerator: u64,
    denominator: u64,
    status: &'static str,
    basis_points: Option<u64>,
}

struct VerifiedLog {
    events: Vec<StoredEvent>,
    chain_tip: String,
    validation: FunnelValidationState,
}

#[derive(Default)]
struct FunnelValidationState {
    scans: HashMap<String, ScanValidationState>,
    gaps: HashSet<String>,
    attempts: HashMap<String, AttemptValidationState>,
}

struct ScanValidationState {
    completed_review_prs: u64,
    gap_count: u64,
    delivery_confirmed: bool,
}

struct AttemptValidationState {
    transition_id: String,
    transition_bound: bool,
    covered: bool,
    workbench_ready: bool,
    failed: bool,
}

impl AttemptValidationState {
    fn ensure_active(&self) -> Result<()> {
        ensure!(
            !self.failed,
            "resume attempt is terminal after a recorded failure"
        );
        Ok(())
    }

    fn failure_stage(&self) -> FailureStage {
        if self.workbench_ready {
            FailureStage::AfterWorkbenchReady
        } else if self.covered {
            FailureStage::BeforeWorkbenchReady
        } else if self.transition_bound {
            FailureStage::BeforeCoverage
        } else {
            FailureStage::BeforeTransitionBound
        }
    }
}

impl FunnelValidationState {
    fn apply(&mut self, event: &FunnelEvent) -> Result<()> {
        match event {
            FunnelEvent::Baseline {
                scan_id,
                completed_review_prs,
                ..
            } => {
                ensure!(
                    self.scans
                        .insert(
                            scan_id.clone(),
                            ScanValidationState {
                                completed_review_prs: *completed_review_prs,
                                gap_count: 0,
                                delivery_confirmed: false,
                            },
                        )
                        .is_none(),
                    "value log contains a duplicate scan baseline"
                );
            }
            FunnelEvent::GapDiscovery {
                scan_id,
                transition_id,
            } => {
                let scan = self
                    .scans
                    .get_mut(scan_id)
                    .context("gap discovery is missing its scan baseline")?;
                ensure!(
                    !scan.delivery_confirmed,
                    "gap discovery follows delivery confirmation for its scan"
                );
                ensure!(
                    scan.gap_count < scan.completed_review_prs,
                    "gap discovery count exceeds completed reviews for its scan"
                );
                scan.gap_count += 1;
                self.gaps.insert(transition_id.clone());
            }
            FunnelEvent::InboxDelivery { scan_id } => {
                let scan = self
                    .scans
                    .get_mut(scan_id)
                    .context("Inbox delivery is missing its scan baseline")?;
                ensure!(
                    !scan.delivery_confirmed,
                    "value log contains a duplicate Inbox delivery"
                );
                scan.delivery_confirmed = true;
            }
            FunnelEvent::ResumeInvoked {
                transition_id,
                attempt_id,
            } => {
                ensure!(
                    self.gaps.contains(transition_id),
                    "resume invocation does not reference a discovered gap"
                );
                ensure!(
                    self.attempts
                        .insert(
                            attempt_id.clone(),
                            AttemptValidationState {
                                transition_id: transition_id.clone(),
                                transition_bound: false,
                                covered: false,
                                workbench_ready: false,
                                failed: false,
                            },
                        )
                        .is_none(),
                    "value log contains a duplicate resume attempt"
                );
            }
            FunnelEvent::TransitionBound {
                transition_id,
                attempt_id,
            } => {
                let attempt = self
                    .attempt_mut(transition_id, attempt_id)
                    .context("bound transition does not reference a resume invocation")?;
                attempt.ensure_active()?;
                attempt.transition_bound = true;
            }
            FunnelEvent::CoveredTransition {
                transition_id,
                attempt_id,
            } => {
                let attempt = self
                    .attempt_mut(transition_id, attempt_id)
                    .filter(|attempt| attempt.transition_bound)
                    .context("covered transition does not reference a bound transition")?;
                attempt.ensure_active()?;
                attempt.covered = true;
            }
            FunnelEvent::WorkbenchReady {
                transition_id,
                attempt_id,
            } => {
                let attempt = self
                    .attempt_mut(transition_id, attempt_id)
                    .filter(|attempt| attempt.covered)
                    .context("workbench readiness does not reference a covered transition")?;
                attempt.ensure_active()?;
                attempt.workbench_ready = true;
            }
            FunnelEvent::AttemptFailed {
                transition_id,
                attempt_id,
                stage,
            } => {
                let attempt = self
                    .attempt_mut(transition_id, attempt_id)
                    .context("failed attempt does not reference a resume invocation")?;
                ensure!(
                    *stage == attempt.failure_stage(),
                    "failed attempt stage does not match its completed funnel stages"
                );
                attempt.failed = true;
            }
        }
        Ok(())
    }

    fn attempt_mut(
        &mut self,
        transition_id: &str,
        attempt_id: &str,
    ) -> Option<&mut AttemptValidationState> {
        self.attempts
            .get_mut(attempt_id)
            .filter(|attempt| attempt.transition_id == transition_id)
    }
}

pub(crate) fn canonical_log_path(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .context("value log path must name a file")?;
    ensure!(
        file_name != "." && file_name != "..",
        "value log path must name a file"
    );
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent).with_context(|| {
        format!(
            "failed to resolve value log parent directory {}",
            parent.display()
        )
    })?;
    ensure!(parent.is_dir(), "value log parent is not a directory");
    let canonical = parent.join(file_name);
    let canonical_text = canonical
        .to_str()
        .context("value log path must be valid UTF-8")?;
    ensure!(
        canonical_text.len() <= 4096,
        "value log path exceeds 4096 bytes"
    );
    ensure!(
        canonical_text
            .chars()
            .all(|character| !crate::is_terminal_unsafe(character) && character != '`'),
        "value log path contains a terminal-unsafe character or backtick"
    );
    if let Ok(metadata) = fs::symlink_metadata(&canonical) {
        ensure!(
            metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
            "value log must be a regular file, not a symbolic link"
        );
    }
    Ok(canonical)
}

pub(crate) fn ensure_distinct_output(log: &Path, output: &Path) -> Result<()> {
    let output = canonical_log_path(output)?;
    ensure!(
        log != output,
        "value log and command output must use different files"
    );
    #[cfg(unix)]
    if let (Ok(log_metadata), Ok(output_metadata)) = (fs::metadata(log), fs::metadata(&output)) {
        ensure!(
            log_metadata.dev() != output_metadata.dev()
                || log_metadata.ino() != output_metadata.ino(),
            "value log and command output must not alias the same file"
        );
    }
    Ok(())
}

pub(crate) struct TransitionIdentity<'a> {
    pub(crate) provider_hostname: &'a str,
    pub(crate) repository: &'a str,
    pub(crate) pull_request_number: u64,
    pub(crate) reviewer: &'a str,
    pub(crate) review_id: u64,
    pub(crate) review_state: &'a str,
    pub(crate) checkpoint: &'a str,
    pub(crate) head: &'a str,
}

pub(crate) fn transition_id(identity: &TransitionIdentity<'_>) -> String {
    let mut hasher = Sha256::new();
    let pull_request_number = identity.pull_request_number.to_string();
    let review_id = identity.review_id.to_string();
    for field in [
        "stratadiff-value-funnel-transition-v1",
        identity.provider_hostname,
        identity.repository,
        &pull_request_number,
        identity.reviewer,
        &review_id,
        identity.review_state,
        identity.checkpoint,
        identity.head,
    ] {
        hasher.update(field.as_bytes());
        hasher.update([0]);
    }
    hex(&hasher.finalize())
}

pub(crate) fn inbox_event_transition_id(event: &InboxEventEnvelope) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"stratadiff-value-funnel-transition-v2");
    hasher.update([0]);
    hasher.update(event.event_id.as_bytes());
    hasher.update([0]);
    hex(&hasher.finalize())
}

pub(crate) fn record_inbox_discovery(
    path: &Path,
    collection_complete: bool,
    inspected_candidates: usize,
    completed_review_prs: usize,
    transition_ids: &[String],
) -> Result<String> {
    ensure!(
        completed_review_prs <= inspected_candidates,
        "completed review count exceeds inspected candidates"
    );
    ensure!(
        transition_ids.len() <= completed_review_prs,
        "gap discovery count exceeds completed reviews"
    );
    let inspected_candidates = u64::try_from(inspected_candidates)
        .context("inspected candidate count does not fit in the value log")?;
    let completed_review_prs = u64::try_from(completed_review_prs)
        .context("completed review count does not fit in the value log")?;
    let scan_id = random_identifier()?;
    let mut events = vec![FunnelEvent::Baseline {
        scan_id: scan_id.clone(),
        inspected_candidates,
        completed_review_prs,
        collection_complete,
    }];
    for transition_id in transition_ids {
        require_sha256(transition_id, "transition ID")?;
        events.push(FunnelEvent::GapDiscovery {
            scan_id: scan_id.clone(),
            transition_id: transition_id.clone(),
        });
    }
    append_events(path, events)?;
    Ok(scan_id)
}

pub(crate) fn record_inbox_delivery(path: &Path, scan_id: &str) -> Result<()> {
    require_sha256(scan_id, "scan ID")?;
    append_events(
        path,
        vec![FunnelEvent::InboxDelivery {
            scan_id: scan_id.to_owned(),
        }],
    )
}

pub(crate) fn record_resume_invoked(path: &Path, transition_id: &str) -> Result<String> {
    require_sha256(transition_id, "transition ID")?;
    let attempt_id = random_identifier()?;
    append_events(
        path,
        vec![FunnelEvent::ResumeInvoked {
            transition_id: transition_id.to_owned(),
            attempt_id: attempt_id.clone(),
        }],
    )?;
    Ok(attempt_id)
}

pub(crate) fn record_covered_transition(
    path: &Path,
    transition_id: &str,
    attempt_id: &str,
) -> Result<()> {
    require_sha256(transition_id, "transition ID")?;
    require_sha256(attempt_id, "attempt ID")?;
    append_events(
        path,
        vec![FunnelEvent::CoveredTransition {
            transition_id: transition_id.to_owned(),
            attempt_id: attempt_id.to_owned(),
        }],
    )
}

pub(crate) fn record_transition_bound(
    path: &Path,
    transition_id: &str,
    attempt_id: &str,
) -> Result<()> {
    require_sha256(transition_id, "transition ID")?;
    require_sha256(attempt_id, "attempt ID")?;
    append_events(
        path,
        vec![FunnelEvent::TransitionBound {
            transition_id: transition_id.to_owned(),
            attempt_id: attempt_id.to_owned(),
        }],
    )
}

pub(crate) fn record_workbench_ready(
    path: &Path,
    transition_id: &str,
    attempt_id: &str,
) -> Result<()> {
    require_sha256(transition_id, "transition ID")?;
    require_sha256(attempt_id, "attempt ID")?;
    append_events(
        path,
        vec![FunnelEvent::WorkbenchReady {
            transition_id: transition_id.to_owned(),
            attempt_id: attempt_id.to_owned(),
        }],
    )
}

pub(crate) fn record_attempt_failed(
    path: &Path,
    transition_id: &str,
    attempt_id: &str,
) -> Result<()> {
    require_sha256(transition_id, "transition ID")?;
    require_sha256(attempt_id, "attempt ID")?;
    let log = read_log_path(path)?;
    ensure!(
        has_attempt(&log.events, transition_id, attempt_id, |event| {
            matches!(event, FunnelEvent::ResumeInvoked { .. })
        }),
        "failed attempt does not reference a resume invocation"
    );
    let stage = if has_attempt(&log.events, transition_id, attempt_id, |event| {
        matches!(event, FunnelEvent::WorkbenchReady { .. })
    }) {
        FailureStage::AfterWorkbenchReady
    } else if has_attempt(&log.events, transition_id, attempt_id, |event| {
        matches!(event, FunnelEvent::CoveredTransition { .. })
    }) {
        FailureStage::BeforeWorkbenchReady
    } else if has_attempt(&log.events, transition_id, attempt_id, |event| {
        matches!(event, FunnelEvent::TransitionBound { .. })
    }) {
        FailureStage::BeforeCoverage
    } else {
        FailureStage::BeforeTransitionBound
    };
    append_events(
        path,
        vec![FunnelEvent::AttemptFailed {
            transition_id: transition_id.to_owned(),
            attempt_id: attempt_id.to_owned(),
            stage,
        }],
    )
}

pub(crate) fn attempt_reached_workbench_ready(
    path: &Path,
    transition_id: &str,
    attempt_id: &str,
) -> Result<bool> {
    require_sha256(transition_id, "transition ID")?;
    require_sha256(attempt_id, "attempt ID")?;
    let log = read_log_path(path)?;
    ensure!(
        has_attempt(&log.events, transition_id, attempt_id, |event| {
            matches!(event, FunnelEvent::ResumeInvoked { .. })
        }),
        "workbench readiness query does not reference a resume invocation"
    );
    Ok(has_attempt(
        &log.events,
        transition_id,
        attempt_id,
        |event| matches!(event, FunnelEvent::WorkbenchReady { .. }),
    ))
}

pub(crate) fn run(args: ValueReportArgs) -> Result<()> {
    let log_path = canonical_log_path(&args.log)?;
    if let Some(output) = &args.output {
        ensure_distinct_output(&log_path, output)?;
    }
    let log = read_log_path(&log_path)?;
    let report = build_report(&log)?;
    let bytes = match args.format {
        ValueReportFormat::Json => serde_json::to_vec(&report)?,
        ValueReportFormat::Markdown => render_markdown(&report).into_bytes(),
    };
    if let Some(path) = args.output {
        write_private(&path, &bytes)?;
        eprintln!(
            "wrote aggregate value report to {}",
            crate::display_path(&path)
        );
    } else {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&bytes)?;
        if !bytes.ends_with(b"\n") {
            stdout.write_all(b"\n")?;
        }
    }
    Ok(())
}

fn append_events(path: &Path, requested: Vec<FunnelEvent>) -> Result<()> {
    ensure!(
        !requested.is_empty(),
        "value log append must contain an event"
    );
    let mut file = open_current_log_locked(path, true, lock_exclusive)?;
    let result = append_events_locked(path, &mut file, requested);
    unlock(&file);
    result
}

fn append_events_locked(path: &Path, file: &mut File, requested: Vec<FunnelEvent>) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take((MAX_LOG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_LOG_BYTES,
        "value log exceeds its {MAX_LOG_BYTES}-byte limit"
    );
    let mut verified = verify_log_bytes(&bytes)?;
    let mut by_id = HashMap::new();
    for event in &verified.events {
        ensure!(
            by_id
                .insert(event.event_id.clone(), event.event.clone())
                .is_none(),
            "value log contains a duplicate event ID"
        );
    }

    let mut encoded = Vec::new();
    for event in requested {
        validate_payload(&event)?;
        let business_key = business_key(&event);
        let event_id = sha256_hex(format!("{}\0{business_key}", event_kind(&event)).as_bytes());
        if let Some(existing) = by_id.get(&event_id) {
            ensure!(
                *existing == event,
                "value log contains a conflicting retry for {}",
                event_kind(&event)
            );
            continue;
        }
        let sequence = u64::try_from(verified.events.len() + 1)
            .context("value log sequence does not fit in u64")?;
        let observed_at_unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before the Unix epoch")?
            .as_secs();
        let unsigned = UnsignedEvent {
            schema: EVENT_SCHEMA.to_owned(),
            tool_version: env!("CARGO_PKG_VERSION").to_owned(),
            sequence,
            observed_at_unix_seconds,
            event_id: event_id.clone(),
            previous_event_sha256: verified.chain_tip.clone(),
            event: event.clone(),
        };
        let event_sha256 = sha256_hex(&serde_json::to_vec(&unsigned)?);
        let stored = StoredEvent {
            schema: unsigned.schema,
            tool_version: unsigned.tool_version,
            sequence,
            observed_at_unix_seconds,
            event_id: event_id.clone(),
            previous_event_sha256: unsigned.previous_event_sha256,
            event,
            event_sha256: event_sha256.clone(),
        };
        verified.validation.apply(&stored.event)?;
        serde_json::to_writer(&mut encoded, &stored)?;
        encoded.push(b'\n');
        verified.chain_tip = event_sha256;
        by_id.insert(event_id, stored.event.clone());
        verified.events.push(stored);
        ensure!(
            verified.events.len() <= MAX_EVENTS,
            "value log exceeds its {MAX_EVENTS}-event limit"
        );
    }

    if encoded.is_empty() {
        return Ok(());
    }
    let new_size = bytes
        .len()
        .checked_add(encoded.len())
        .context("value log byte count overflow")?;
    ensure!(
        new_size <= MAX_LOG_BYTES,
        "value log exceeds its {MAX_LOG_BYTES}-byte limit"
    );
    bytes.extend_from_slice(&encoded);
    replace_log_atomically(path, file, &bytes)
}

fn read_log_path(path: &Path) -> Result<VerifiedLog> {
    let file = open_current_log_locked(path, false, lock_shared)?;
    read_locked(file)
}

fn read_locked(file: File) -> Result<VerifiedLog> {
    let mut bytes = Vec::new();
    file.take((MAX_LOG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_LOG_BYTES,
        "value log exceeds its {MAX_LOG_BYTES}-byte limit"
    );
    verify_log_bytes(&bytes)
}

fn verify_log_bytes(bytes: &[u8]) -> Result<VerifiedLog> {
    if bytes.is_empty() {
        return Ok(VerifiedLog {
            events: Vec::new(),
            chain_tip: ZERO_SHA256.to_owned(),
            validation: FunnelValidationState::default(),
        });
    }
    ensure!(
        bytes.ends_with(b"\n"),
        "value log ends with a partial event"
    );
    let mut events = Vec::new();
    let mut chain_tip = ZERO_SHA256.to_owned();
    let mut event_ids = HashSet::new();
    let mut validation = FunnelValidationState::default();
    for (index, line) in bytes[..bytes.len() - 1]
        .split(|byte| *byte == b'\n')
        .enumerate()
    {
        ensure!(!line.is_empty(), "value log contains an empty event line");
        ensure!(
            events.len() < MAX_EVENTS,
            "value log exceeds its {MAX_EVENTS}-event limit"
        );
        let event: StoredEvent = serde_json::from_slice(line)
            .with_context(|| format!("failed to decode value event {}", index + 1))?;
        validate_event(&event, index + 1, &chain_tip)?;
        ensure!(
            serde_json::to_vec(&event)? == line,
            "value event {} is not canonical JSON",
            index + 1
        );
        ensure!(
            event_ids.insert(event.event_id.clone()),
            "value log contains a duplicate event ID"
        );
        validation.apply(&event.event)?;
        chain_tip.clone_from(&event.event_sha256);
        events.push(event);
    }
    Ok(VerifiedLog {
        events,
        chain_tip,
        validation,
    })
}

fn validate_event(event: &StoredEvent, expected_index: usize, previous: &str) -> Result<()> {
    ensure!(
        event.schema == EVENT_SCHEMA,
        "unsupported value event schema"
    );
    ensure!(
        is_semver(&event.tool_version),
        "value event tool version is invalid"
    );
    let expected_sequence =
        u64::try_from(expected_index).context("value event index does not fit in u64")?;
    ensure!(
        event.sequence == expected_sequence,
        "value event sequence is not contiguous"
    );
    ensure!(
        event.observed_at_unix_seconds > 0,
        "value event timestamp is invalid"
    );
    require_sha256(&event.event_id, "event ID")?;
    require_sha256(&event.previous_event_sha256, "previous event digest")?;
    require_sha256(&event.event_sha256, "event digest")?;
    ensure!(
        event.previous_event_sha256 == previous,
        "value event hash chain is broken at sequence {}",
        event.sequence
    );
    validate_payload(&event.event)?;
    let unsigned = UnsignedEvent {
        schema: event.schema.clone(),
        tool_version: event.tool_version.clone(),
        sequence: event.sequence,
        observed_at_unix_seconds: event.observed_at_unix_seconds,
        event_id: event.event_id.clone(),
        previous_event_sha256: event.previous_event_sha256.clone(),
        event: event.event.clone(),
    };
    let expected_digest = sha256_hex(&serde_json::to_vec(&unsigned)?);
    ensure!(
        event.event_sha256 == expected_digest,
        "value event digest mismatch at sequence {}",
        event.sequence
    );
    let expected_id = sha256_hex(
        format!(
            "{}\0{}",
            event_kind(&event.event),
            business_key(&event.event)
        )
        .as_bytes(),
    );
    ensure!(
        event.event_id == expected_id,
        "value event ID mismatch at sequence {}",
        event.sequence
    );
    Ok(())
}

fn is_semver(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (*part == "0" || !part.starts_with('0'))
        })
}

fn validate_payload(event: &FunnelEvent) -> Result<()> {
    match event {
        FunnelEvent::Baseline {
            scan_id,
            inspected_candidates,
            completed_review_prs,
            ..
        } => {
            require_sha256(scan_id, "scan ID")?;
            ensure!(
                *inspected_candidates <= MAX_BASELINE_CANDIDATES,
                "baseline inspected candidate count exceeds {MAX_BASELINE_CANDIDATES}"
            );
            ensure!(
                *completed_review_prs <= *inspected_candidates,
                "baseline completed review count exceeds inspected candidates"
            );
        }
        FunnelEvent::GapDiscovery {
            scan_id,
            transition_id,
        } => {
            require_sha256(scan_id, "scan ID")?;
            require_sha256(transition_id, "transition ID")?;
        }
        FunnelEvent::InboxDelivery { scan_id } => {
            require_sha256(scan_id, "scan ID")?;
        }
        FunnelEvent::CoveredTransition {
            transition_id,
            attempt_id,
        }
        | FunnelEvent::ResumeInvoked {
            transition_id,
            attempt_id,
        }
        | FunnelEvent::TransitionBound {
            transition_id,
            attempt_id,
        }
        | FunnelEvent::WorkbenchReady {
            transition_id,
            attempt_id,
        }
        | FunnelEvent::AttemptFailed {
            transition_id,
            attempt_id,
            ..
        } => {
            require_sha256(transition_id, "transition ID")?;
            require_sha256(attempt_id, "attempt ID")?;
        }
    }
    Ok(())
}

fn has_attempt(
    events: &[StoredEvent],
    transition_id: &str,
    attempt_id: &str,
    predicate: impl Fn(&FunnelEvent) -> bool,
) -> bool {
    events.iter().any(|event| {
        let matches_identity = match &event.event {
            FunnelEvent::ResumeInvoked {
                transition_id: candidate_transition,
                attempt_id: candidate_attempt,
            }
            | FunnelEvent::TransitionBound {
                transition_id: candidate_transition,
                attempt_id: candidate_attempt,
            }
            | FunnelEvent::CoveredTransition {
                transition_id: candidate_transition,
                attempt_id: candidate_attempt,
            }
            | FunnelEvent::WorkbenchReady {
                transition_id: candidate_transition,
                attempt_id: candidate_attempt,
            }
            | FunnelEvent::AttemptFailed {
                transition_id: candidate_transition,
                attempt_id: candidate_attempt,
                ..
            } => candidate_transition == transition_id && candidate_attempt == attempt_id,
            FunnelEvent::Baseline { .. }
            | FunnelEvent::GapDiscovery { .. }
            | FunnelEvent::InboxDelivery { .. } => false,
        };
        matches_identity && predicate(&event.event)
    })
}

fn build_report(log: &VerifiedLog) -> Result<ValueReport> {
    let mut scans = 0_u64;
    let mut partial_scans = 0_u64;
    let mut inspected_candidates = 0_u64;
    let mut completed_review_checkpoints = 0_u64;
    let mut gap_discoveries = 0_u64;
    let mut gaps = HashSet::new();
    let mut gaps_by_scan = HashMap::<String, Vec<String>>::new();
    let mut delivery_confirmed_scan_ids = HashSet::new();
    let mut resumed_transitions = HashSet::new();
    let mut covered_transitions = HashSet::new();
    let mut ready_transitions = HashSet::new();
    let mut resume_attempts = HashSet::new();
    let mut bound_attempts = HashSet::new();
    let mut covered_attempts = HashSet::new();
    let mut ready_attempts = HashSet::new();
    let mut failed_attempts = HashSet::new();
    let mut failures = ReportFailures {
        total: 0,
        before_transition_bound: 0,
        before_coverage: 0,
        before_workbench_ready: 0,
        after_workbench_ready: 0,
    };
    for event in &log.events {
        match &event.event {
            FunnelEvent::Baseline {
                inspected_candidates: inspected,
                completed_review_prs,
                collection_complete,
                ..
            } => {
                scans = scans.checked_add(1).context("scan count overflow")?;
                inspected_candidates = inspected_candidates
                    .checked_add(*inspected)
                    .context("inspected candidate count overflow")?;
                completed_review_checkpoints = completed_review_checkpoints
                    .checked_add(*completed_review_prs)
                    .context("completed review checkpoint count overflow")?;
                if !collection_complete {
                    partial_scans = partial_scans
                        .checked_add(1)
                        .context("partial scan count overflow")?;
                }
            }
            FunnelEvent::CoveredTransition {
                transition_id,
                attempt_id,
            } => {
                covered_transitions.insert(transition_id.clone());
                covered_attempts.insert(attempt_id.clone());
            }
            FunnelEvent::GapDiscovery {
                scan_id,
                transition_id,
            } => {
                gap_discoveries = gap_discoveries
                    .checked_add(1)
                    .context("gap discovery count overflow")?;
                gaps.insert(transition_id.clone());
                gaps_by_scan
                    .entry(scan_id.clone())
                    .or_default()
                    .push(transition_id.clone());
            }
            FunnelEvent::InboxDelivery { scan_id } => {
                delivery_confirmed_scan_ids.insert(scan_id.clone());
            }
            FunnelEvent::ResumeInvoked {
                transition_id,
                attempt_id,
            } => {
                resumed_transitions.insert(transition_id.clone());
                resume_attempts.insert(attempt_id.clone());
            }
            FunnelEvent::TransitionBound { attempt_id, .. } => {
                bound_attempts.insert(attempt_id.clone());
            }
            FunnelEvent::WorkbenchReady {
                transition_id,
                attempt_id,
            } => {
                ready_transitions.insert(transition_id.clone());
                ready_attempts.insert(attempt_id.clone());
            }
            FunnelEvent::AttemptFailed {
                attempt_id, stage, ..
            } => {
                failed_attempts.insert(attempt_id.clone());
                failures.total = failures
                    .total
                    .checked_add(1)
                    .context("failure count overflow")?;
                let stage_count = match stage {
                    FailureStage::BeforeTransitionBound => &mut failures.before_transition_bound,
                    FailureStage::BeforeCoverage => &mut failures.before_coverage,
                    FailureStage::BeforeWorkbenchReady => &mut failures.before_workbench_ready,
                    FailureStage::AfterWorkbenchReady => &mut failures.after_workbench_ready,
                };
                *stage_count = stage_count
                    .checked_add(1)
                    .context("failure stage count overflow")?;
            }
        }
    }
    ensure!(
        resumed_transitions.is_subset(&gaps),
        "resume set is not a subset of gaps"
    );
    ensure!(
        covered_transitions.is_subset(&resumed_transitions),
        "covered set is not a subset of resumes"
    );
    ensure!(
        ready_transitions.is_subset(&covered_transitions),
        "ready set is not a subset of covered"
    );
    ensure!(
        bound_attempts.is_subset(&resume_attempts),
        "bound attempts are not a subset of resume attempts"
    );
    ensure!(
        covered_attempts.is_subset(&bound_attempts),
        "covered attempts are not a subset of bound attempts"
    );
    ensure!(
        ready_attempts.is_subset(&covered_attempts),
        "ready attempts are not a subset of covered attempts"
    );
    ensure!(
        failed_attempts.is_subset(&resume_attempts),
        "failed attempts are not a subset of resume attempts"
    );
    let unique_gap_transitions = u64::try_from(gaps.len()).context("gap count overflow")?;
    let delivery_confirmed_scans = u64::try_from(delivery_confirmed_scan_ids.len())
        .context("delivery-confirmed scan count overflow")?;
    let delivery_unconfirmed_scans = scans
        .checked_sub(delivery_confirmed_scans)
        .context("delivery-confirmed scan count exceeds scan count")?;
    let mut delivered_gap_discoveries = 0_u64;
    let mut delivered_gap_transitions = HashSet::new();
    for scan_id in &delivery_confirmed_scan_ids {
        if let Some(transition_ids) = gaps_by_scan.get(scan_id) {
            delivered_gap_discoveries = delivered_gap_discoveries
                .checked_add(
                    u64::try_from(transition_ids.len())
                        .context("delivered gap discovery count overflow")?,
                )
                .context("delivered gap discovery count overflow")?;
            delivered_gap_transitions.extend(transition_ids.iter().cloned());
        }
    }
    let unique_delivered_gap_transitions = u64::try_from(delivered_gap_transitions.len())
        .context("delivered gap transition count overflow")?;
    let unique_resumed_transitions =
        u64::try_from(resumed_transitions.len()).context("resumed transition count overflow")?;
    let delivered_resumed_transitions = u64::try_from(
        resumed_transitions
            .intersection(&delivered_gap_transitions)
            .count(),
    )
    .context("delivered resumed transition count overflow")?;
    let covered_transition_count =
        u64::try_from(covered_transitions.len()).context("covered transition count overflow")?;
    let unique_ready_transitions =
        u64::try_from(ready_transitions.len()).context("ready transition count overflow")?;
    let resume_attempt_count =
        u64::try_from(resume_attempts.len()).context("resume attempt count overflow")?;
    let transition_bound_attempt_count =
        u64::try_from(bound_attempts.len()).context("bound attempt count overflow")?;
    let covered_attempt_count =
        u64::try_from(covered_attempts.len()).context("covered attempt count overflow")?;
    let ready_attempt_count =
        u64::try_from(ready_attempts.len()).context("ready attempt count overflow")?;
    let unresolved_attempt_count = u64::try_from(
        resume_attempts
            .iter()
            .filter(|attempt| {
                !ready_attempts.contains(*attempt) && !failed_attempts.contains(*attempt)
            })
            .count(),
    )
    .context("unresolved attempt count overflow")?;
    Ok(ValueReport {
        schema: REPORT_SCHEMA,
        tool_version: env!("CARGO_PKG_VERSION"),
        source_schema: EVENT_SCHEMA,
        event_count: log.events.len(),
        chain_tip_sha256: log.chain_tip.clone(),
        privacy: ReportPrivacy {
            automatic_upload: false,
            source_content_exported: false,
            local_paths_exported: false,
            repository_identity_exported: false,
            pull_request_identity_exported: false,
            reviewer_identity_exported: false,
            pseudonymous_transition_digests_exported: false,
            transition_identifiers_exported: false,
        },
        claim_boundary: ClaimBoundary {
            local_post_discovery_activation_supported: true,
            clean_install_success_supported: false,
            reviewer_time_savings_supported: false,
            issue_recall_supported: false,
            market_prevalence_supported: false,
        },
        summary: ReportSummary {
            scans,
            partial_scans,
            delivery_confirmed_scans,
            delivery_unconfirmed_scans,
            inspected_candidates,
            completed_review_checkpoints,
            covered_transitions: covered_transition_count,
            gap_discoveries,
            unique_gap_transitions,
            delivered_gap_discoveries,
            unique_delivered_gap_transitions,
            unique_resumed_transitions,
            unique_ready_transitions,
            resume_attempts: resume_attempt_count,
            transition_bound_attempts: transition_bound_attempt_count,
            covered_attempts: covered_attempt_count,
            workbench_ready_attempts: ready_attempt_count,
            unresolved_attempts: unresolved_attempt_count,
        },
        conversion: ReportConversion {
            delivered_gap_to_resume: rate(
                delivered_resumed_transitions,
                unique_delivered_gap_transitions,
            ),
            resume_attempt_to_covered_transition: rate(covered_attempt_count, resume_attempt_count),
            covered_transition_to_workbench_ready: rate(ready_attempt_count, covered_attempt_count),
            resume_attempt_to_workbench_ready: rate(ready_attempt_count, resume_attempt_count),
        },
        failures,
    })
}

fn rate(numerator: u64, denominator: u64) -> Rate {
    if denominator == 0 {
        Rate {
            numerator,
            denominator,
            status: "undefined",
            basis_points: None,
        }
    } else {
        let basis_points = numerator
            .checked_mul(10_000)
            .and_then(|value| value.checked_add(denominator / 2))
            .and_then(|value| value.checked_div(denominator))
            .expect("bounded value-funnel counts cannot overflow");
        Rate {
            numerator,
            denominator,
            status: "defined",
            basis_points: Some(basis_points),
        }
    }
}

fn render_markdown(report: &ValueReport) -> String {
    let format_rate = |rate: &Rate| match rate.basis_points {
        Some(value) => format!("{}.{:02}%", value / 100, value % 100),
        None => "undefined".to_owned(),
    };
    format!(
        "# StrataDiff local value funnel\n\n- Scans: {} ({} partial; {} output-delivery confirmed, {} unconfirmed)\n- Inspected candidates: {}\n- Completed-review checkpoints: {}\n- Gap discoveries: {} ({} unique transitions)\n- Delivered gap discoveries: {} ({} unique transitions)\n- Unique resumed transitions: {}\n- Unique covered transitions: {}\n- Unique ready transitions: {}\n- Resume attempts: {}\n- Transition-bound attempts: {}\n- Covered attempts: {}\n- Workbench-ready attempts: {}\n- Unresolved attempts: {}\n- Terminal failures: {} ({} before binding, {} before coverage, {} before readiness, {} after readiness)\n- Delivered gap → Resume: {}\n- Resume attempt → covered transition: {}\n- Covered transition → workbench ready: {}\n- Resume attempt → workbench ready: {}\n\nChain tip: `{}`\n\nAn `inbox_delivery` confirms only that the selected output sink accepted the complete Inbox bytes; it does not prove that a person read them. A scan without that event has unconfirmed delivery, which may reflect an output failure, interruption, or a later logging failure. Unresolved Resume attempts may still be running or may have ended without a recorded terminal event. This aggregate supports only the explicitly recorded local activation funnel after confirmed Inbox delivery. It does not establish clean-install success, reviewer time savings, issue recall, or market prevalence. No event is uploaded automatically.\n",
        report.summary.scans,
        report.summary.partial_scans,
        report.summary.delivery_confirmed_scans,
        report.summary.delivery_unconfirmed_scans,
        report.summary.inspected_candidates,
        report.summary.completed_review_checkpoints,
        report.summary.gap_discoveries,
        report.summary.unique_gap_transitions,
        report.summary.delivered_gap_discoveries,
        report.summary.unique_delivered_gap_transitions,
        report.summary.unique_resumed_transitions,
        report.summary.covered_transitions,
        report.summary.unique_ready_transitions,
        report.summary.resume_attempts,
        report.summary.transition_bound_attempts,
        report.summary.covered_attempts,
        report.summary.workbench_ready_attempts,
        report.summary.unresolved_attempts,
        report.failures.total,
        report.failures.before_transition_bound,
        report.failures.before_coverage,
        report.failures.before_workbench_ready,
        report.failures.after_workbench_ready,
        format_rate(&report.conversion.delivered_gap_to_resume),
        format_rate(&report.conversion.resume_attempt_to_covered_transition),
        format_rate(&report.conversion.covered_transition_to_workbench_ready),
        format_rate(&report.conversion.resume_attempt_to_workbench_ready),
        report.chain_tip_sha256,
    )
}

fn event_kind(event: &FunnelEvent) -> &'static str {
    match event {
        FunnelEvent::Baseline { .. } => "baseline",
        FunnelEvent::CoveredTransition { .. } => "covered_transition",
        FunnelEvent::GapDiscovery { .. } => "gap_discovery",
        FunnelEvent::InboxDelivery { .. } => "inbox_delivery",
        FunnelEvent::ResumeInvoked { .. } => "resume_invoked",
        FunnelEvent::TransitionBound { .. } => "transition_bound",
        FunnelEvent::WorkbenchReady { .. } => "workbench_ready",
        FunnelEvent::AttemptFailed { .. } => "attempt_failed",
    }
}

fn business_key(event: &FunnelEvent) -> String {
    match event {
        FunnelEvent::Baseline { scan_id, .. } => scan_id.clone(),
        FunnelEvent::GapDiscovery {
            scan_id,
            transition_id,
        } => format!("{scan_id}\0{transition_id}"),
        FunnelEvent::InboxDelivery { scan_id } => scan_id.clone(),
        FunnelEvent::CoveredTransition { attempt_id, .. }
        | FunnelEvent::ResumeInvoked { attempt_id, .. }
        | FunnelEvent::TransitionBound { attempt_id, .. }
        | FunnelEvent::WorkbenchReady { attempt_id, .. }
        | FunnelEvent::AttemptFailed { attempt_id, .. } => attempt_id.clone(),
    }
}

fn random_identifier() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("failed to generate a value-funnel scan ID: {error}"))?;
    Ok(hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn require_sha256(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{label} must be 64 lowercase hexadecimal characters"
    );
    Ok(())
}

fn open_log(path: &Path, create: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).append(create).create(create);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    let file = options
        .open(path)
        .with_context(|| format!("failed to open local value log {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "value log is not a regular file");
    Ok(file)
}

fn open_current_log_locked(
    path: &Path,
    create: bool,
    lock: fn(&File) -> Result<()>,
) -> Result<File> {
    loop {
        let file = open_log(path, create)?;
        lock(&file)?;
        if log_path_matches_file(path, &file)? {
            #[cfg(unix)]
            {
                let metadata = file.metadata()?;
                ensure!(
                    metadata.nlink() == 1,
                    "value log must not have multiple hard links"
                );
                ensure!(
                    metadata.permissions().mode() & 0o077 == 0,
                    "value log permissions must not grant group or other access"
                );
            }
            return Ok(file);
        }
        unlock(&file);
    }
}

#[cfg(unix)]
fn log_path_matches_file(path: &Path, file: &File) -> Result<bool> {
    let path_metadata = fs::symlink_metadata(path)?;
    let file_metadata = file.metadata()?;
    Ok(path_metadata.file_type().is_file()
        && path_metadata.dev() == file_metadata.dev()
        && path_metadata.ino() == file_metadata.ino())
}

#[cfg(not(unix))]
fn log_path_matches_file(_path: &Path, _file: &File) -> Result<bool> {
    Ok(true)
}

#[cfg(unix)]
fn lock_exclusive(file: &File) -> Result<()> {
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    ensure!(result == 0, "failed to lock the local value log");
    Ok(())
}

#[cfg(not(unix))]
fn lock_exclusive(_file: &File) -> Result<()> {
    anyhow::bail!("local value logs are supported only on Unix platforms")
}

#[cfg(unix)]
fn lock_shared(file: &File) -> Result<()> {
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH) };
    ensure!(result == 0, "failed to lock the local value log");
    Ok(())
}

#[cfg(not(unix))]
fn lock_shared(_file: &File) -> Result<()> {
    anyhow::bail!("local value logs are supported only on Unix platforms")
}

#[cfg(unix)]
fn unlock(file: &File) {
    unsafe {
        libc::flock(file.as_raw_fd(), libc::LOCK_UN);
    }
}

#[cfg(not(unix))]
fn unlock(_file: &File) {}

fn replace_log_atomically(path: &Path, locked: &File, bytes: &[u8]) -> Result<()> {
    replace_log_atomically_with(path, locked, bytes, |temporary, replacement| {
        temporary.write_all(replacement)?;
        temporary.as_file().sync_all()?;
        Ok(())
    })
}

fn replace_log_atomically_with(
    path: &Path,
    locked: &File,
    bytes: &[u8],
    stage: impl FnOnce(&mut tempfile::NamedTempFile, &[u8]) -> Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".stratadiff-value-log-")
        .tempfile_in(parent)
        .with_context(|| {
            format!(
                "failed to create temporary value log beside {}",
                path.display()
            )
        })?;
    stage(&mut temporary, bytes).context("failed to stage complete value-log transaction")?;
    ensure!(
        temporary.as_file().metadata()?.len() == u64::try_from(bytes.len())?,
        "temporary value-log transaction has an incomplete byte count"
    );
    ensure!(
        log_path_matches_file(path, locked)?,
        "value log changed while its transaction was staged"
    );
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to commit local value log {}", path.display()))?;
    #[cfg(unix)]
    File::open(parent)
        .with_context(|| format!("failed to open value log directory {}", parent.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync value log directory {}", parent.display()))?;
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    ensure!(parent.is_dir(), "output parent directory does not exist");
    let mut temporary = tempfile::Builder::new()
        .prefix(".stratadiff-value-report-")
        .tempfile_in(parent)
        .with_context(|| {
            format!(
                "failed to create temporary output beside {}",
                path.display()
            )
        })?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
        time::{Duration, Instant},
    };

    use super::*;

    fn transition(byte: char) -> String {
        std::iter::repeat_n(byte, 64).collect()
    }

    fn push_encoded_event(
        bytes: &mut Vec<u8>,
        chain_tip: &mut String,
        sequence: u64,
        event: FunnelEvent,
    ) {
        let event_id =
            sha256_hex(format!("{}\0{}", event_kind(&event), business_key(&event)).as_bytes());
        let unsigned = UnsignedEvent {
            schema: EVENT_SCHEMA.to_owned(),
            tool_version: env!("CARGO_PKG_VERSION").to_owned(),
            sequence,
            observed_at_unix_seconds: sequence,
            event_id: event_id.clone(),
            previous_event_sha256: chain_tip.clone(),
            event: event.clone(),
        };
        let event_sha256 = sha256_hex(&serde_json::to_vec(&unsigned).unwrap());
        let stored = StoredEvent {
            schema: unsigned.schema,
            tool_version: unsigned.tool_version,
            sequence,
            observed_at_unix_seconds: unsigned.observed_at_unix_seconds,
            event_id,
            previous_event_sha256: unsigned.previous_event_sha256,
            event,
            event_sha256: event_sha256.clone(),
        };
        serde_json::to_writer(&mut *bytes, &stored).unwrap();
        bytes.push(b'\n');
        *chain_tip = event_sha256;
    }

    #[test]
    fn large_log_validation_remains_linear() {
        const SCANS: u64 = 12_000;
        let mut bytes = Vec::new();
        let mut chain_tip = ZERO_SHA256.to_owned();
        let mut sequence = 1;
        for index in 0..SCANS {
            let scan_id = format!("{index:064x}");
            let transition_id = format!("{:064x}", index + SCANS);
            push_encoded_event(
                &mut bytes,
                &mut chain_tip,
                sequence,
                FunnelEvent::Baseline {
                    scan_id: scan_id.clone(),
                    inspected_candidates: 1,
                    completed_review_prs: 1,
                    collection_complete: true,
                },
            );
            sequence += 1;
            push_encoded_event(
                &mut bytes,
                &mut chain_tip,
                sequence,
                FunnelEvent::GapDiscovery {
                    scan_id,
                    transition_id,
                },
            );
            sequence += 1;
        }
        assert!(bytes.len() <= MAX_LOG_BYTES);

        let started = Instant::now();
        let verified = verify_log_bytes(&bytes).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(verified.events.len(), usize::try_from(SCANS * 2).unwrap());
        assert!(
            elapsed < Duration::from_secs(10),
            "large value log validation took {elapsed:?}"
        );
    }

    #[test]
    fn failed_transaction_staging_never_publishes_a_valid_partial_batch() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("value.jsonl");
        record_inbox_discovery(&path, true, 0, 0, &[]).unwrap();
        let before = fs::read(&path).unwrap();
        let verified = verify_log_bytes(&before).unwrap();
        let mut replacement = before.clone();
        let mut chain_tip = verified.chain_tip;
        let sequence = u64::try_from(verified.events.len()).unwrap() + 1;
        let scan_id = transition('1');
        push_encoded_event(
            &mut replacement,
            &mut chain_tip,
            sequence,
            FunnelEvent::Baseline {
                scan_id: scan_id.clone(),
                inspected_candidates: 1,
                completed_review_prs: 1,
                collection_complete: true,
            },
        );
        let valid_partial_len = replacement.len();
        push_encoded_event(
            &mut replacement,
            &mut chain_tip,
            sequence + 1,
            FunnelEvent::GapDiscovery {
                scan_id,
                transition_id: transition('2'),
            },
        );
        assert!(verify_log_bytes(&replacement[..valid_partial_len]).is_ok());

        let file = open_current_log_locked(&path, true, lock_exclusive).unwrap();
        let error = replace_log_atomically_with(&path, &file, &replacement, |temporary, bytes| {
            temporary.write_all(&bytes[..valid_partial_len])?;
            Err(std::io::Error::from_raw_os_error(libc::ENOSPC).into())
        })
        .unwrap_err();
        unlock(&file);
        assert!(error.to_string().contains("failed to stage"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(read_log_path(&path).unwrap().events.len(), 1);

        let file = open_current_log_locked(&path, true, lock_exclusive).unwrap();
        let error = replace_log_atomically_with(&path, &file, &replacement, |temporary, bytes| {
            temporary.write_all(bytes)?;
            anyhow::bail!("injected sync failure")
        })
        .unwrap_err();
        unlock(&file);
        assert!(error.to_string().contains("failed to stage"));
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn concurrent_atomic_appends_preserve_every_event() {
        const WORKERS: usize = 16;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("value.jsonl");
        let transition_id = transition('3');
        let scan_id =
            record_inbox_discovery(&path, true, 1, 1, std::slice::from_ref(&transition_id))
                .unwrap();
        record_inbox_delivery(&path, &scan_id).unwrap();
        let barrier = Arc::new(Barrier::new(WORKERS));
        let handles = (0..WORKERS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let path = path.clone();
                let transition_id = transition_id.clone();
                thread::spawn(move || {
                    barrier.wait();
                    record_resume_invoked(&path, &transition_id).unwrap()
                })
            })
            .collect::<Vec<_>>();
        let attempts = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<HashSet<_>>();
        assert_eq!(attempts.len(), WORKERS);
        let log = read_log_path(&path).unwrap();
        assert_eq!(log.events.len(), WORKERS + 3);
        assert_eq!(
            build_report(&log).unwrap().summary.resume_attempts,
            u64::try_from(WORKERS).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn value_log_with_multiple_hard_links_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("value.jsonl");
        record_inbox_discovery(&path, true, 0, 0, &[]).unwrap();
        std::fs::hard_link(&path, directory.path().join("alias.jsonl")).unwrap();

        let error = read_log_path(&path).err().unwrap();
        assert!(
            error
                .to_string()
                .contains("value log must not have multiple hard links")
        );
    }

    #[test]
    fn funnel_is_local_aggregate_and_integrity_chained() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("value.jsonl");
        let first = transition('a');
        let second = transition('b');
        let scan_id = record_inbox_discovery(&path, true, 4, 3, &[first.clone(), second]).unwrap();
        record_inbox_delivery(&path, &scan_id).unwrap();
        let attempt_id = record_resume_invoked(&path, &first).unwrap();
        record_transition_bound(&path, &first, &attempt_id).unwrap();
        record_covered_transition(&path, &first, &attempt_id).unwrap();
        record_workbench_ready(&path, &first, &attempt_id).unwrap();
        assert!(attempt_reached_workbench_ready(&path, &first, &attempt_id).unwrap());
        let log = read_log_path(&path).unwrap();
        let report = build_report(&log).unwrap();
        assert_eq!(report.summary.scans, 1);
        assert_eq!(report.summary.delivery_confirmed_scans, 1);
        assert_eq!(report.summary.delivery_unconfirmed_scans, 0);
        assert_eq!(report.summary.inspected_candidates, 4);
        assert_eq!(report.summary.completed_review_checkpoints, 3);
        assert_eq!(report.summary.covered_transitions, 1);
        assert_eq!(report.summary.unique_gap_transitions, 2);
        assert_eq!(report.summary.delivered_gap_discoveries, 2);
        assert_eq!(report.summary.unique_delivered_gap_transitions, 2);
        assert_eq!(report.summary.unique_resumed_transitions, 1);
        assert_eq!(report.summary.unique_ready_transitions, 1);
        assert_eq!(report.summary.resume_attempts, 1);
        assert_eq!(report.summary.transition_bound_attempts, 1);
        assert_eq!(report.summary.covered_attempts, 1);
        assert_eq!(report.summary.workbench_ready_attempts, 1);
        assert_eq!(report.summary.unresolved_attempts, 0);
        assert_eq!(
            report.conversion.delivered_gap_to_resume.basis_points,
            Some(5_000)
        );
        assert_eq!(
            report
                .conversion
                .resume_attempt_to_covered_transition
                .basis_points,
            Some(10_000)
        );
        assert_eq!(
            report
                .conversion
                .resume_attempt_to_workbench_ready
                .basis_points,
            Some(10_000)
        );
        let exported = serde_json::to_string(&report).unwrap();
        assert!(!exported.contains(&first));
        assert!(!exported.contains("acme/widget"));
        assert!(!exported.contains("octocat"));

        let mut bytes = fs::read(&path).unwrap();
        let position = bytes.iter().position(|byte| *byte == b'4').unwrap();
        bytes[position] = b'5';
        fs::write(&path, bytes).unwrap();
        assert!(read_log_path(&path).is_err());
    }

    #[test]
    fn attempts_are_distinct_stages_are_idempotent_and_out_of_order_events_fail() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("value.jsonl");
        let id = transition('c');
        record_inbox_discovery(&path, false, 1, 1, std::slice::from_ref(&id)).unwrap();
        let before = fs::read(&path).unwrap();
        let first_attempt = record_resume_invoked(&path, &id).unwrap();
        let once = fs::read(&path).unwrap();
        let second_attempt = record_resume_invoked(&path, &id).unwrap();
        assert_ne!(first_attempt, second_attempt);
        assert_ne!(fs::read(&path).unwrap(), once);
        assert_ne!(before, once);

        record_transition_bound(&path, &id, &first_attempt).unwrap();
        let bound_once = fs::read(&path).unwrap();
        record_transition_bound(&path, &id, &first_attempt).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bound_once);
        let report = build_report(&read_log_path(&path).unwrap()).unwrap();
        assert_eq!(report.summary.unresolved_attempts, 2);
        assert_eq!(report.summary.delivery_unconfirmed_scans, 1);
        assert_eq!(report.summary.unique_delivered_gap_transitions, 0);
        assert_eq!(
            report.conversion.delivered_gap_to_resume.status,
            "undefined"
        );

        let unknown = transition('d');
        let error = record_resume_invoked(&path, &unknown).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not reference a discovered gap")
        );
        let unknown_attempt = transition('e');
        let error = record_covered_transition(&path, &unknown, &unknown_attempt).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not reference a bound transition")
        );
        let error = record_workbench_ready(&path, &unknown, &unknown_attempt).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not reference a covered transition")
        );
    }

    #[test]
    fn delivery_conversion_excludes_resumes_from_unconfirmed_scans() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("value.jsonl");
        let unconfirmed = transition('4');
        record_inbox_discovery(&path, true, 1, 1, std::slice::from_ref(&unconfirmed)).unwrap();

        // Optional instrumentation must not block a Resume command the user already received.
        record_resume_invoked(&path, &unconfirmed).unwrap();

        let delivered = transition('5');
        let delivered_scan =
            record_inbox_discovery(&path, true, 1, 1, std::slice::from_ref(&delivered)).unwrap();
        record_inbox_delivery(&path, &delivered_scan).unwrap();

        let report = build_report(&read_log_path(&path).unwrap()).unwrap();
        assert_eq!(report.summary.unique_resumed_transitions, 1);
        assert_eq!(report.summary.unique_delivered_gap_transitions, 1);
        assert_eq!(report.conversion.delivered_gap_to_resume.numerator, 0);
        assert_eq!(report.conversion.delivered_gap_to_resume.denominator, 1);
        assert_eq!(
            report.conversion.delivered_gap_to_resume.basis_points,
            Some(0)
        );
    }

    #[test]
    fn zero_denominators_remain_explicitly_undefined() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("value.jsonl");
        let scan_id = record_inbox_discovery(&path, true, 0, 0, &[]).unwrap();
        record_inbox_delivery(&path, &scan_id).unwrap();
        let report = build_report(&read_log_path(&path).unwrap()).unwrap();
        assert_eq!(
            report.conversion.delivered_gap_to_resume.status,
            "undefined"
        );
        assert_eq!(report.conversion.delivered_gap_to_resume.basis_points, None);
        assert_eq!(
            report.conversion.resume_attempt_to_workbench_ready.status,
            "undefined"
        );
    }

    #[test]
    fn failed_attempt_is_terminal_and_oversized_baselines_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("value.jsonl");
        let id = transition('f');
        record_inbox_discovery(&path, true, 1, 1, std::slice::from_ref(&id)).unwrap();
        let attempt_id = record_resume_invoked(&path, &id).unwrap();
        assert!(!attempt_reached_workbench_ready(&path, &id, &attempt_id).unwrap());
        record_attempt_failed(&path, &id, &attempt_id).unwrap();
        record_attempt_failed(&path, &id, &attempt_id).unwrap();
        let report = build_report(&read_log_path(&path).unwrap()).unwrap();
        assert_eq!(report.summary.unresolved_attempts, 0);
        assert_eq!(report.failures.before_transition_bound, 1);

        let error = record_transition_bound(&path, &id, &attempt_id).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("terminal after a recorded failure")
        );

        let oversized = directory.path().join("oversized.jsonl");
        let error = record_inbox_discovery(&oversized, true, 101, 0, &[]).unwrap_err();
        assert!(error.to_string().contains("exceeds 100"));
    }
}
