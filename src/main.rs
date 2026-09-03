use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use stratadiff::{DiffReport, Language, analyze_files, apply_patch, verify_report};

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
            let encoded = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                std::fs::write(&path, &encoded)
                    .with_context(|| format!("failed to write {}", path.display()))?;
                eprintln!("wrote proof-carrying report to {}", path.display());
            }
            if json {
                println!("{encoded}");
            } else {
                print_summary(&report);
            }
        }
        Command::Verify {
            report,
            before,
            after,
        } => {
            let report: DiffReport = serde_json::from_slice(
                &std::fs::read(&report)
                    .with_context(|| format!("failed to read {}", report.display()))?,
            )?;
            let before = std::fs::read(&before)
                .with_context(|| format!("failed to read {}", before.display()))?;
            let after = std::fs::read(&after)
                .with_context(|| format!("failed to read {}", after.display()))?;
            verify_report(&report, &before, &after)?;
            println!(
                "verified: replay, parser manifest, relations, ambiguities, changes, and summary"
            );
        }
        Command::Apply {
            report,
            before,
            output,
        } => {
            let report: DiffReport = serde_json::from_slice(
                &std::fs::read(&report)
                    .with_context(|| format!("failed to read {}", report.display()))?,
            )?;
            let before = std::fs::read(&before)
                .with_context(|| format!("failed to read {}", before.display()))?;
            let rebuilt = apply_patch(&before, &report.patch)?;
            verify_report(&report, &before, &rebuilt)?;
            std::fs::write(&output, rebuilt)
                .with_context(|| format!("failed to write {}", output.display()))?;
            println!("rebuilt certified target at {}", output.display());
        }
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
        println!(
            "  ambiguous: {} candidates -> {} candidates under nodes {} -> {}",
            ambiguity.before.len(),
            ambiguity.after.len(),
            ambiguity.parent_before,
            ambiguity.parent_after
        );
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
