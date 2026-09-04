use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use stratadiff::{
    AmbiguityConstraint, DiffReport, Language, VerificationLimits, analyze_files,
    verify_and_replay_report_bytes, verify_report_bytes,
};

const LEGACY_REPORT_SCHEMA_V1: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v1.schema.json";

#[derive(Debug, Parser)]
#[command(name = "stratadiff")]
#[command(about = "Proof-carrying, ambiguity-aware structural code differencing")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compare two source files and produce a structural report.
    Diff {
        before: PathBuf,
        after: PathBuf,
        #[arg(long, value_enum)]
        language: Option<Language>,
        /// Write the complete JSON report and replay certificate to this path.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Print the complete report as JSON instead of the terminal summary.
        #[arg(long)]
        json: bool,
    },
    /// Re-run all independently checkable predicates and the byte replay certificate.
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
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Diff {
            before,
            after,
            language,
            output,
            json,
        } => {
            let report = analyze_files(&before, &after, language)?;
            let encoded = serde_json::to_vec(&report)?;
            let report_limit = VerificationLimits::default().max_report_bytes;
            if encoded.len() > report_limit {
                bail!(
                    "generated report bytes limit exceeded: observed {}, limit {report_limit}",
                    encoded.len()
                );
            }
            if let Some(path) = output {
                std::fs::write(&path, &encoded)
                    .with_context(|| format!("failed to write {}", path.display()))?;
                eprintln!("wrote proof-carrying report to {}", path.display());
            }
            if json {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(&encoded)?;
                stdout.write_all(b"\n")?;
            } else {
                print_summary(&report);
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
                "verified: replay, parser manifest, relations, ambiguities, changes, and summary"
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
                        format!("failed to verify and apply report {}", report.display())
                    })?;
            std::fs::write(&output, rebuilt)
                .with_context(|| format!("failed to write {}", output.display()))?;
            println!("rebuilt certified target at {}", output.display());
        }
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: usize, label: &str) -> Result<Vec<u8>> {
    let read_limit = limit
        .checked_add(1)
        .with_context(|| format!("{label} limit cannot be incremented safely"))?;
    let read_limit = u64::try_from(read_limit)
        .with_context(|| format!("{label} limit cannot be represented by the file reader"))?;
    let file = File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {}", path.display()))?;
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
        .with_context(|| format!("failed to parse {} as JSON", path.display()))?;
    if envelope.schema.as_deref() == Some(LEGACY_REPORT_SCHEMA_V1) {
        bail!(
            "report schema v1 cannot represent coupled ambiguity constraints or be losslessly upgraded; rerun StrataDiff on the original snapshots to create a v2 report"
        );
    }
    Ok(())
}

fn print_summary(report: &DiffReport) {
    println!(
        "{} -> {} ({:?})",
        report.before.path, report.after.path, report.parser.language
    );
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
        "replay certificate: {}",
        if report.certificate.patch_verified {
            "verified"
        } else {
            "invalid"
        }
    );
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::read_bounded;

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
}
