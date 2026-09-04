use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use stratadiff::{
    AmbiguityConstraint, DiffReport, Language, VerificationLimits, analyze_bytes, apply_patch,
    verify_and_replay_report_bytes, verify_report_bytes,
};

mod viewer;

const LEGACY_REPORT_SCHEMA_V1: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v1.schema.json";
const LEGACY_REPORT_SCHEMA_V2: &str = "https://raw.githubusercontent.com/gcomfident-crypto/stratadiff/main/schema/report-v2.schema.json";

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
    }
    Ok(())
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
        "replay certificate: {}",
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
        display_bytes, display_text, escape_terminal_unsafe_json, is_terminal_unsafe, read_bounded,
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
