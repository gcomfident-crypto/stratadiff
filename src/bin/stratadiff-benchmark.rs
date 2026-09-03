use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde::Serialize;
use stratadiff::diffbenchmark::{
    GodMappingGroup, GodMappingRecord, jdt_node_role, parse_god_info, parse_god_report,
};

const DIFFBENCHMARK_REVISION: &str = "870592abd559d0bd822a27eb5c8ea45aee47015b";
const ORACLE_ROOT: &str = "hrd-oracle/adb-paper/literature-exp";

#[derive(Debug, Parser)]
#[command(name = "stratadiff-benchmark")]
#[command(about = "Audit a pinned DiffBenchmark oracle checkout")]
#[command(version)]
struct Cli {
    /// Root of the DiffBenchmark checkout.
    checkout: PathBuf,
    /// Exit successfully even when JSON or mapping info is invalid.
    #[arg(long)]
    allow_invalid: bool,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileCounts {
    total: usize,
    valid_json: usize,
    invalid_json: usize,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct MappingCounts {
    matched_elements: usize,
    mappings: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InvalidFile {
    path: String,
    error: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InfoError {
    path: String,
    scope: &'static str,
    group: Option<String>,
    collection: &'static str,
    index: usize,
    error: String,
}

#[derive(Clone, Copy)]
enum MappingScope {
    IntraFile,
    InterFile,
}

impl MappingScope {
    fn label(self) -> &'static str {
        match self {
            Self::IntraFile => "intraFile",
            Self::InterFile => "interFile",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditReport {
    revision: String,
    checkout_root: PathBuf,
    oracle_root: PathBuf,
    files: FileCounts,
    intra_file: MappingCounts,
    inter_file: MappingCounts,
    unsupported_jdt_types: BTreeMap<String, usize>,
    info_parse_errors: usize,
    invalid_files: Vec<InvalidFile>,
    info_errors: Vec<InfoError>,
}

impl AuditReport {
    fn has_invalid_input(&self) -> bool {
        self.files.invalid_json != 0 || self.info_parse_errors != 0
    }
}

fn main() -> Result<ExitCode> {
    let cli = Cli::parse();
    let revision = validate_revision(&cli.checkout)?;
    let report = audit_checkout(&cli.checkout, revision)?;

    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, &report)?;
    writeln!(output)?;

    if report.has_invalid_input() && !cli.allow_invalid {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn validate_revision(checkout: &Path) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["rev-parse", "--verify", "HEAD^{commit}"])
        .output()
        .with_context(|| format!("failed to run git in {}", checkout.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git rev-parse failed for {}: {}",
            checkout.display(),
            stderr.trim_end()
        );
    }

    let stdout = std::str::from_utf8(&output.stdout).context("git revision is not UTF-8")?;
    let revision = stdout.strip_suffix('\n').unwrap_or(stdout);
    let revision = revision.strip_suffix('\r').unwrap_or(revision);
    if revision != DIFFBENCHMARK_REVISION {
        bail!(
            "DiffBenchmark revision mismatch: expected {DIFFBENCHMARK_REVISION}, found {revision}"
        );
    }
    Ok(revision.to_owned())
}

fn audit_checkout(checkout: &Path, revision: String) -> Result<AuditReport> {
    let oracle_root = checkout.join(ORACLE_ROOT);
    let god_files = find_god_files(&oracle_root)?;
    let mut report = AuditReport {
        revision,
        checkout_root: checkout.to_owned(),
        oracle_root,
        files: FileCounts {
            total: god_files.len(),
            ..FileCounts::default()
        },
        intra_file: MappingCounts::default(),
        inter_file: MappingCounts::default(),
        unsupported_jdt_types: BTreeMap::new(),
        info_parse_errors: 0,
        invalid_files: Vec::new(),
        info_errors: Vec::new(),
    };

    for path in god_files {
        let relative_path = path
            .strip_prefix(checkout)
            .expect("oracle file must be beneath the checkout root")
            .to_string_lossy()
            .into_owned();
        let bytes = std::fs::read(&path)
            .with_context(|| format!("failed to read oracle file {}", path.display()))?;
        let god_report = match parse_god_report(&bytes) {
            Ok(god_report) => god_report,
            Err(error) => {
                report.files.invalid_json += 1;
                report.invalid_files.push(InvalidFile {
                    path: relative_path,
                    error: format!("{error:#}"),
                });
                continue;
            }
        };

        report.files.valid_json += 1;
        report.audit_group(
            &god_report.intra_file_mappings,
            &relative_path,
            MappingScope::IntraFile,
            None,
        );
        for (group_name, group) in &god_report.inter_file_mappings {
            report.audit_group(
                group,
                &relative_path,
                MappingScope::InterFile,
                Some(group_name.as_str()),
            );
        }
    }
    report.info_parse_errors = report.info_errors.len();
    Ok(report)
}

fn find_god_files(oracle_root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    visit_oracle_directory(oracle_root, &mut files)?;
    files.sort();
    Ok(files)
}

fn visit_oracle_directory(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(directory)
        .with_context(|| format!("failed to read oracle directory {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut entries = entries;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            visit_oracle_directory(&entry.path(), files)?;
        } else if file_type.is_file() && entry.file_name() == OsStr::new("GOD.json") {
            files.push(entry.path());
        }
    }
    Ok(())
}

impl AuditReport {
    fn audit_group(
        &mut self,
        group: &GodMappingGroup,
        path: &str,
        scope: MappingScope,
        group_name: Option<&str>,
    ) {
        let counts = match scope {
            MappingScope::IntraFile => &mut self.intra_file,
            MappingScope::InterFile => &mut self.inter_file,
        };
        counts.matched_elements += group.matched_elements.len();
        counts.mappings += group.mappings.len();
        self.audit_records(
            &group.matched_elements,
            path,
            scope,
            group_name,
            "matchedElements",
        );
        self.audit_records(&group.mappings, path, scope, group_name, "mappings");
    }

    fn audit_records(
        &mut self,
        records: &[GodMappingRecord],
        path: &str,
        scope: MappingScope,
        group_name: Option<&str>,
        collection: &'static str,
    ) {
        for (index, record) in records.iter().enumerate() {
            match parse_god_info(&record.info) {
                Ok(mapping) => {
                    for node_type in [&mapping.before.node_type, &mapping.after.node_type] {
                        if jdt_node_role(node_type).is_none() {
                            *self
                                .unsupported_jdt_types
                                .entry(node_type.clone())
                                .or_default() += 1;
                        }
                    }
                }
                Err(error) => self.info_errors.push(InfoError {
                    path: path.to_owned(),
                    scope: scope.label(),
                    group: group_name.map(str::to_owned),
                    collection,
                    index,
                    error: format!("{error:#}"),
                }),
            }
        }
    }
}
