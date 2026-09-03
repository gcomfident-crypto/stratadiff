use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use csv::{ReaderBuilder, StringRecord};
use serde::{Deserialize, Serialize};

const DIFFBENCHMARK_REVISION: &str = "870592abd559d0bd822a27eb5c8ea45aee47015b";
const ORACLE_ROOT: &str = "hrd-oracle/adb-paper/literature-exp";
const LITERATURE_CSV: &str = "csv-outputs/adb-paper/literature-exp-INTRA_FILE_ONLY-NO_FILTER-RefOracle-NO_COMMENTS_AND_JAVADOCS-2025_04_10 18:15:50.csv";
const EXPECTED_CASES: usize = 285;
const MANIFEST_SCHEMA: &str = "stratadiff-diffbenchmark-materialization-v2";
const REPOSITORY_MAP_SCHEMA: &str = "stratadiff-repository-mirrors-v1";
const MARKER_NAME: &str = ".stratadiff-materialize";
const MARKER_CONTENTS: &str = "stratadiff-diffbenchmark-materialization-v1\n";
const MANIFEST_NAME: &str = "manifest.json";
const MANIFEST_PART_NAME: &str = ".manifest.json.part";

#[derive(Debug, Parser)]
#[command(name = "stratadiff-materialize")]
#[command(about = "Materialize Java sources for the pinned DiffBenchmark literature corpus")]
#[command(version)]
struct Cli {
    /// Root of the pinned DiffBenchmark checkout.
    checkout: PathBuf,
    /// New output directory, or a cache previously created by this command.
    output: PathBuf,
    /// Explicit repository mirrors, each restricted to exact commits.
    #[arg(long, value_name = "JSON")]
    repository_map: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema: String,
    dataset_revision: String,
    case_count: usize,
    cases: Vec<ManifestCase>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestCase {
    oracle_path: String,
    oracle_blake3: String,
    oracle_repository_url: String,
    fetched_repository_url: String,
    commit: String,
    parent: String,
    before: SourceArtifact,
    after: SourceArtifact,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceArtifact {
    repository_path: String,
    materialized_path: String,
    content_blake3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct LiteratureKey {
    project: String,
    commit: String,
    source_name: String,
}

#[derive(Clone, Debug)]
struct LiteratureMetadata {
    owner: String,
    repository: String,
    oracle_repository_url: String,
    commit: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct InfoKey {
    commit: String,
    encoded_path: String,
}

#[derive(Clone, Debug)]
struct CasePlan {
    oracle_path: String,
    oracle_blake3: String,
    owner: String,
    repository: String,
    oracle_repository_url: String,
    commit: String,
    source_path: String,
}

#[derive(Clone, Debug)]
struct ResolvedCase {
    fetched_repository: RepositoryLocation,
    parent: String,
    before_path: String,
    after_path: String,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubCommitPage {
    sha: String,
    parents: Vec<GithubParent>,
    files: Vec<GithubFile>,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubParent {
    sha: String,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubFile {
    filename: String,
    status: String,
    previous_filename: Option<String>,
}

#[derive(Clone, Debug)]
struct GithubCommit {
    parent: String,
    files: Vec<GithubFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RepositoryLocation {
    owner: String,
    repository: String,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryMapFile {
    schema: String,
    mirrors: Vec<RepositoryMapEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryMapEntry {
    original_repository_url: String,
    mirror_repository_url: String,
    commits: Vec<String>,
}

#[derive(Debug, Default)]
struct RepositoryRegistry {
    mirrors: BTreeMap<(String, String), RepositoryLocation>,
}

#[derive(Clone, Debug)]
struct FetchedCommit {
    repository: RepositoryLocation,
    commit: GithubCommit,
}

#[derive(Debug)]
struct GhApiError(String);

impl std::fmt::Display for GhApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for GhApiError {}

fn main() -> Result<()> {
    let cli = Cli::parse();
    validate_revision(&cli.checkout)?;
    let repositories = read_repository_registry(cli.repository_map.as_deref())?;
    let plans = build_case_plans(&cli.checkout)?;
    prepare_output(&cli.output, plans.len())?;

    let manifest_path = cli.output.join(MANIFEST_NAME);
    let cached_manifest = read_cached_manifest(&manifest_path)?;
    let had_cached_manifest = cached_manifest.is_some();
    let manifest = match cached_manifest {
        Some(manifest) => {
            validate_cached_manifest(&manifest, &plans, &repositories)?;
            complete_cached_manifest(&cli.output, manifest, &plans, &repositories)?
        }
        None => materialize_new_manifest(&cli.output, &plans, &repositories)?,
    };

    let encoded = encode_manifest(&manifest)?;
    if had_cached_manifest {
        let cached_bytes = fs::read(&manifest_path)
            .with_context(|| format!("failed to re-read {}", manifest_path.display()))?;
        ensure!(
            cached_bytes == encoded,
            "cached manifest is not in its stable serialized form: {}",
            manifest_path.display()
        );
        let stale_part = manifest_path.with_file_name(MANIFEST_PART_NAME);
        if stale_part.try_exists()? {
            fs::remove_file(&stale_part).with_context(|| {
                format!(
                    "failed to remove stale manifest file {}",
                    stale_part.display()
                )
            })?;
        }
    } else {
        write_manifest(&manifest_path, &encoded)?;
    }
    io::stdout().lock().write_all(&encoded)?;
    Ok(())
}

fn validate_revision(checkout: &Path) -> Result<()> {
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
    ensure!(
        revision == DIFFBENCHMARK_REVISION,
        "DiffBenchmark revision mismatch: expected {DIFFBENCHMARK_REVISION}, found {revision}"
    );
    Ok(())
}

fn read_repository_registry(path: Option<&Path>) -> Result<RepositoryRegistry> {
    let Some(path) = path else {
        return Ok(RepositoryRegistry::default());
    };
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read repository map {}", path.display()))?;
    let file: RepositoryMapFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid repository map {}", path.display()))?;
    ensure!(
        file.schema == REPOSITORY_MAP_SCHEMA,
        "unsupported repository map schema {}",
        file.schema
    );
    ensure!(
        !file.mirrors.is_empty(),
        "repository map contains no mirrors"
    );

    let mut registry = RepositoryRegistry::default();
    for entry in file.mirrors {
        let original = parse_repository_url(&entry.original_repository_url).with_context(|| {
            format!(
                "invalid original repository URL {}",
                entry.original_repository_url
            )
        })?;
        let mirror = parse_repository_url(&entry.mirror_repository_url).with_context(|| {
            format!(
                "invalid mirror repository URL {}",
                entry.mirror_repository_url
            )
        })?;
        ensure!(
            original != mirror,
            "repository mirror must differ from original {}",
            original.url
        );
        ensure!(
            !entry.commits.is_empty(),
            "repository mirror {} contains no allowed commits",
            mirror.url
        );
        for commit in entry.commits {
            validate_sha(&commit).with_context(|| {
                format!(
                    "invalid allowed commit for mirror {} -> {}",
                    original.url, mirror.url
                )
            })?;
            let key = (original.url.clone(), commit.clone());
            ensure!(
                registry.mirrors.insert(key, mirror.clone()).is_none(),
                "duplicate repository mirror authorization for {} at {commit}",
                original.url
            );
        }
    }
    Ok(registry)
}

fn build_case_plans(checkout: &Path) -> Result<Vec<CasePlan>> {
    let literature_path = checkout.join(LITERATURE_CSV);
    let mut literature = read_literature_metadata(&literature_path)?;
    let literature_commits: BTreeSet<_> = literature
        .values()
        .map(|metadata| metadata.commit.clone())
        .collect();
    let mut info = read_info_metadata(&checkout.join("info.csv"), &literature_commits)?;
    let oracle_files = find_god_files(&checkout.join(ORACLE_ROOT))?;
    ensure!(
        oracle_files.len() == EXPECTED_CASES,
        "expected {EXPECTED_CASES} literature GOD.json files, found {}",
        oracle_files.len()
    );

    let mut plans = Vec::with_capacity(oracle_files.len());
    for oracle_file in oracle_files {
        let relative = oracle_file
            .strip_prefix(checkout)
            .expect("oracle file must be beneath the checkout root");
        let oracle_path = slash_path(relative)?;
        let components = path_components(
            relative
                .strip_prefix(ORACLE_ROOT)
                .expect("oracle file must be beneath the oracle root"),
        )?;
        ensure!(
            components.len() == 4 && components[3] == "GOD.json",
            "unexpected literature oracle path {oracle_path}"
        );
        let project = components[0].clone();
        let commit = components[1].clone();
        let source_name = components[2].clone();
        validate_sha(&commit).with_context(|| format!("invalid oracle commit in {oracle_path}"))?;
        validate_source_name(&source_name)
            .with_context(|| format!("invalid encoded source name in {oracle_path}"))?;

        let literature_key = LiteratureKey {
            project,
            commit: commit.clone(),
            source_name: source_name.clone(),
        };
        let metadata = literature.remove(&literature_key).with_context(|| {
            format!(
                "missing literature CSV join for oracle {oracle_path} using ({}, {}, {})",
                literature_key.project, literature_key.commit, literature_key.source_name
            )
        })?;
        let info_key = InfoKey {
            commit: commit.clone(),
            encoded_path: source_name,
        };
        let source_path = info.remove(&info_key).with_context(|| {
            format!(
                "missing info.csv join for oracle {oracle_path} using ({}, {})",
                info_key.commit, info_key.encoded_path
            )
        })?;
        let oracle_bytes = fs::read(&oracle_file)
            .with_context(|| format!("failed to read oracle {}", oracle_file.display()))?;
        plans.push(CasePlan {
            oracle_path,
            oracle_blake3: digest(&oracle_bytes),
            owner: metadata.owner,
            repository: metadata.repository,
            oracle_repository_url: metadata.oracle_repository_url,
            commit: metadata.commit,
            source_path,
        });
    }

    ensure!(
        literature.is_empty(),
        "literature CSV contains {} row(s) with no GOD.json oracle",
        literature.len()
    );
    Ok(plans)
}

fn read_literature_metadata(path: &Path) -> Result<BTreeMap<LiteratureKey, LiteratureMetadata>> {
    let mut reader = ReaderBuilder::new()
        .flexible(false)
        .from_path(path)
        .with_context(|| format!("failed to open literature CSV {}", path.display()))?;
    let headers = reader
        .headers()
        .with_context(|| format!("failed to parse literature CSV header {}", path.display()))?
        .clone();
    validate_headers(&headers, &["url", "srcFileName"], path)?;
    let url_index = required_column(&headers, "url", path)?;
    let source_index = required_column(&headers, "srcFileName", path)?;
    let mut metadata = BTreeMap::new();

    for record in reader.records() {
        let record =
            record.with_context(|| format!("failed to parse literature CSV {}", path.display()))?;
        let line = record
            .position()
            .context("literature CSV record has no source position")?
            .line();
        let url = &record[url_index];
        let source_name = &record[source_index];
        ensure!(!url.is_empty(), "empty literature URL at line {line}");
        validate_source_name(source_name)
            .with_context(|| format!("invalid srcFileName at literature CSV line {line}"))?;
        let parsed = parse_commit_url(url)
            .with_context(|| format!("invalid literature URL at line {line}"))?;
        let key = LiteratureKey {
            project: format!("{}.{}", parsed.owner, parsed.repository),
            commit: parsed.commit.clone(),
            source_name: source_name.to_owned(),
        };
        let value = LiteratureMetadata {
            owner: parsed.owner,
            repository: parsed.repository,
            oracle_repository_url: parsed.repository_url,
            commit: parsed.commit,
        };
        ensure!(
            metadata.insert(key.clone(), value).is_none(),
            "duplicate literature CSV join key ({}, {}, {})",
            key.project,
            key.commit,
            key.source_name
        );
    }
    ensure!(!metadata.is_empty(), "literature CSV contains no data rows");
    Ok(metadata)
}

fn read_info_metadata(
    path: &Path,
    literature_commits: &BTreeSet<String>,
) -> Result<BTreeMap<InfoKey, String>> {
    let mut reader = ReaderBuilder::new()
        .flexible(false)
        .from_path(path)
        .with_context(|| format!("failed to open info CSV {}", path.display()))?;
    let headers = reader
        .headers()
        .with_context(|| format!("failed to parse info CSV header {}", path.display()))?
        .clone();
    validate_headers(&headers, &["commit", "file"], path)?;
    let commit_index = required_column(&headers, "commit", path)?;
    let file_index = required_column(&headers, "file", path)?;
    let mut metadata = BTreeMap::new();

    for record in reader.records() {
        let record =
            record.with_context(|| format!("failed to parse info CSV {}", path.display()))?;
        let line = record
            .position()
            .context("info CSV record has no source position")?
            .line();
        let commit = &record[commit_index];
        let source_path = &record[file_index];
        if !literature_commits.contains(commit) {
            continue;
        }
        validate_sha(commit).with_context(|| format!("invalid commit at info.csv line {line}"))?;
        validate_repository_path(source_path)
            .with_context(|| format!("invalid file path at info.csv line {line}"))?;
        let encoded_path = encode_info_path(source_path)?;
        let key = InfoKey {
            commit: commit.to_owned(),
            encoded_path,
        };
        ensure!(
            metadata
                .insert(key.clone(), source_path.to_owned())
                .is_none(),
            "duplicate info.csv join key ({}, {})",
            key.commit,
            key.encoded_path
        );
    }
    ensure!(!metadata.is_empty(), "info.csv contains no data rows");
    Ok(metadata)
}

fn validate_headers(headers: &StringRecord, required: &[&str], path: &Path) -> Result<()> {
    ensure!(!headers.is_empty(), "CSV {} has no header", path.display());
    let mut seen = BTreeSet::new();
    for header in headers {
        ensure!(
            !header.is_empty(),
            "CSV {} has an empty header",
            path.display()
        );
        ensure!(
            seen.insert(header),
            "CSV {} has duplicate header {header}",
            path.display()
        );
    }
    for name in required {
        ensure!(
            seen.contains(name),
            "CSV {} is missing required header {name}",
            path.display()
        );
    }
    Ok(())
}

fn required_column(headers: &StringRecord, name: &str, path: &Path) -> Result<usize> {
    headers
        .iter()
        .position(|header| header == name)
        .with_context(|| format!("CSV {} is missing required header {name}", path.display()))
}

struct CommitUrl {
    owner: String,
    repository: String,
    repository_url: String,
    commit: String,
}

fn parse_commit_url(url: &str) -> Result<CommitUrl> {
    let suffix = url
        .strip_prefix("https://github.com/")
        .context("URL must use https://github.com/")?;
    let components: Vec<_> = suffix.split('/').collect();
    ensure!(
        components.len() == 4 && components[2] == "commit",
        "URL must have owner/repository/commit/revision"
    );
    ensure!(
        !components[0].is_empty() && !components[1].is_empty(),
        "URL owner and repository must be non-empty"
    );
    validate_sha(components[3])?;
    Ok(CommitUrl {
        owner: components[0].to_owned(),
        repository: components[1].to_owned(),
        repository_url: format!("https://github.com/{}/{}", components[0], components[1]),
        commit: components[3].to_owned(),
    })
}

fn parse_repository_url(url: &str) -> Result<RepositoryLocation> {
    let suffix = url
        .strip_prefix("https://github.com/")
        .context("repository URL must use https://github.com/")?;
    let components: Vec<_> = suffix.split('/').collect();
    ensure!(
        components.len() == 2 && !components[0].is_empty() && !components[1].is_empty(),
        "repository URL must have exactly owner/repository"
    );
    Ok(RepositoryLocation {
        owner: components[0].to_owned(),
        repository: components[1].to_owned(),
        url: url.to_owned(),
    })
}

fn encode_info_path(path: &str) -> Result<String> {
    let without_extension = path
        .strip_suffix(".java")
        .context("info.csv file path must end in .java")?;
    Ok(without_extension.replace('/', "."))
}

fn find_god_files(oracle_root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    visit_oracle_directory(oracle_root, &mut files)?;
    files.sort();
    Ok(files)
}

fn visit_oracle_directory(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("failed to read oracle directory {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
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

fn prepare_output(output: &Path, case_count: usize) -> Result<()> {
    match fs::symlink_metadata(output) {
        Ok(metadata) => ensure!(
            metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
            "output path {} is not a directory",
            output.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(output)
            .with_context(|| format!("failed to create output directory {}", output.display()))?,
        Err(error) => return Err(error).context("failed to inspect output directory"),
    }

    let marker = output.join(MARKER_NAME);
    let entries = fs::read_dir(output)?.collect::<std::result::Result<Vec<_>, _>>()?;
    if entries.is_empty() {
        write_new_file(&marker, MARKER_CONTENTS.as_bytes())?;
    } else {
        let marker_metadata = fs::symlink_metadata(&marker).with_context(|| {
            format!(
                "refusing non-empty output directory without {MARKER_NAME}: {}",
                output.display()
            )
        })?;
        ensure!(
            marker_metadata.file_type().is_file() && !marker_metadata.file_type().is_symlink(),
            "output marker is not a regular file: {}",
            marker.display()
        );
        let marker_contents = fs::read(&marker)?;
        ensure!(
            marker_contents == MARKER_CONTENTS.as_bytes(),
            "output marker has unexpected contents: {}",
            marker.display()
        );
    }

    validate_output_layout(output, case_count)?;
    fs::create_dir_all(output.join("sources"))?;
    Ok(())
}

fn validate_output_layout(output: &Path, case_count: usize) -> Result<()> {
    for entry in fs::read_dir(output)? {
        let entry = entry?;
        let name = entry.file_name();
        let file_type = entry.file_type()?;
        match name.to_str() {
            Some(MARKER_NAME | MANIFEST_NAME | MANIFEST_PART_NAME) => ensure!(
                file_type.is_file() && !file_type.is_symlink(),
                "unexpected output entry {}",
                entry.path().display()
            ),
            Some("sources") => ensure!(
                file_type.is_dir() && !file_type.is_symlink(),
                "unexpected output entry {}",
                entry.path().display()
            ),
            _ => bail!("unexpected output entry {}", entry.path().display()),
        }
    }

    let sources = output.join("sources");
    if !sources.try_exists()? {
        return Ok(());
    }
    let expected_directories: BTreeSet<_> = (0..case_count).map(case_directory).collect();
    for entry in fs::read_dir(&sources)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("non-UTF-8 source cache entry"))?;
        let file_type = entry.file_type()?;
        ensure!(
            expected_directories.contains(&name) && file_type.is_dir() && !file_type.is_symlink(),
            "unexpected source cache entry {}",
            entry.path().display()
        );
        for source in fs::read_dir(entry.path())? {
            let source = source?;
            let source_type = source.file_type()?;
            let source_name = source.file_name();
            ensure!(
                matches!(
                    source_name.to_str(),
                    Some("before.java" | "after.java" | "before.java.part" | "after.java.part")
                ) && source_type.is_file()
                    && !source_type.is_symlink(),
                "unexpected source cache entry {}",
                source.path().display()
            );
        }
    }
    Ok(())
}

fn read_cached_manifest(path: &Path) -> Result<Option<Manifest>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid cached manifest {}", path.display()))
            .map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn validate_cached_manifest(
    manifest: &Manifest,
    plans: &[CasePlan],
    repositories: &RepositoryRegistry,
) -> Result<()> {
    ensure!(
        manifest.schema == MANIFEST_SCHEMA,
        "cached manifest schema mismatch"
    );
    ensure!(
        manifest.dataset_revision == DIFFBENCHMARK_REVISION,
        "cached manifest dataset revision mismatch"
    );
    ensure!(
        manifest.case_count == plans.len() && manifest.cases.len() == plans.len(),
        "cached manifest case count mismatch"
    );
    for (index, (case, plan)) in manifest.cases.iter().zip(plans).enumerate() {
        ensure!(
            case.oracle_path == plan.oracle_path
                && case.oracle_blake3 == plan.oracle_blake3
                && case.oracle_repository_url == plan.oracle_repository_url
                && case.commit == plan.commit,
            "cached manifest identity mismatch for {}",
            plan.oracle_path
        );
        validate_digest(&case.oracle_blake3)?;
        validate_sha(&case.parent)?;
        let fetched_repository = parse_repository_url(&case.fetched_repository_url)?;
        if case.fetched_repository_url != case.oracle_repository_url {
            let configured = repositories
                .mirrors
                .get(&(case.oracle_repository_url.clone(), case.commit.clone()))
                .with_context(|| {
                    format!(
                        "cached manifest requires an unconfigured mirror for {} at {}",
                        case.oracle_repository_url, case.commit
                    )
                })?;
            ensure!(
                configured == &fetched_repository,
                "cached manifest mirror disagrees with repository map for {} at {}",
                case.oracle_repository_url,
                case.commit
            );
        }
        validate_repository_path(&case.before.repository_path)?;
        validate_repository_path(&case.after.repository_path)?;
        ensure!(
            plan.source_path == case.before.repository_path
                || plan.source_path == case.after.repository_path,
            "cached manifest source path mismatch for {}",
            plan.oracle_path
        );
        let (before_path, after_path) = materialized_paths(index);
        ensure!(
            case.before.materialized_path == before_path
                && case.after.materialized_path == after_path,
            "cached manifest materialized path mismatch for {}",
            plan.oracle_path
        );
        validate_digest(&case.before.content_blake3)?;
        validate_digest(&case.after.content_blake3)?;
    }
    Ok(())
}

fn complete_cached_manifest(
    output: &Path,
    manifest: Manifest,
    plans: &[CasePlan],
    repositories: &RepositoryRegistry,
) -> Result<Manifest> {
    let mut commits = BTreeMap::new();
    for (index, (case, plan)) in manifest.cases.iter().zip(plans).enumerate() {
        let before_exists = validate_cached_source(output, &case.before)?;
        let after_exists = validate_cached_source(output, &case.after)?;
        if before_exists && after_exists {
            report_progress("verified", index + 1, plans.len(), &plan.oracle_path);
            continue;
        }

        let resolved = resolve_case(plan, repositories, &mut commits)?;
        ensure!(
            resolved.fetched_repository.url == case.fetched_repository_url
                && resolved.parent == case.parent
                && resolved.before_path == case.before.repository_path
                && resolved.after_path == case.after.repository_path,
            "GitHub metadata disagrees with cached manifest for {}",
            plan.oracle_path
        );
        if !before_exists {
            let bytes = fetch_source(
                &resolved.fetched_repository.owner,
                &resolved.fetched_repository.repository,
                &resolved.parent,
                &resolved.before_path,
            )?;
            ensure!(
                digest(&bytes) == case.before.content_blake3,
                "downloaded before source digest disagrees with cached manifest for {}",
                plan.oracle_path
            );
            write_source(output, &case.before.materialized_path, &bytes)?;
        }
        if !after_exists {
            let bytes = fetch_source(
                &resolved.fetched_repository.owner,
                &resolved.fetched_repository.repository,
                &plan.commit,
                &resolved.after_path,
            )?;
            ensure!(
                digest(&bytes) == case.after.content_blake3,
                "downloaded after source digest disagrees with cached manifest for {}",
                plan.oracle_path
            );
            write_source(output, &case.after.materialized_path, &bytes)?;
        }
        report_progress("recovered", index + 1, plans.len(), &plan.oracle_path);
    }
    Ok(manifest)
}

fn materialize_new_manifest(
    output: &Path,
    plans: &[CasePlan],
    repositories: &RepositoryRegistry,
) -> Result<Manifest> {
    let mut commits = BTreeMap::new();
    let mut cases = Vec::with_capacity(plans.len());
    for (index, plan) in plans.iter().enumerate() {
        let resolved = resolve_case(plan, repositories, &mut commits)?;
        let before_bytes = fetch_source(
            &resolved.fetched_repository.owner,
            &resolved.fetched_repository.repository,
            &resolved.parent,
            &resolved.before_path,
        )?;
        let after_bytes = fetch_source(
            &resolved.fetched_repository.owner,
            &resolved.fetched_repository.repository,
            &plan.commit,
            &resolved.after_path,
        )?;
        let (before_path, after_path) = materialized_paths(index);
        write_source(output, &before_path, &before_bytes)?;
        write_source(output, &after_path, &after_bytes)?;
        cases.push(ManifestCase {
            oracle_path: plan.oracle_path.clone(),
            oracle_blake3: plan.oracle_blake3.clone(),
            oracle_repository_url: plan.oracle_repository_url.clone(),
            fetched_repository_url: resolved.fetched_repository.url,
            commit: plan.commit.clone(),
            parent: resolved.parent,
            before: SourceArtifact {
                repository_path: resolved.before_path,
                materialized_path: before_path,
                content_blake3: digest(&before_bytes),
            },
            after: SourceArtifact {
                repository_path: resolved.after_path,
                materialized_path: after_path,
                content_blake3: digest(&after_bytes),
            },
        });
        report_progress("materialized", index + 1, plans.len(), &plan.oracle_path);
    }
    Ok(Manifest {
        schema: MANIFEST_SCHEMA.to_owned(),
        dataset_revision: DIFFBENCHMARK_REVISION.to_owned(),
        case_count: cases.len(),
        cases,
    })
}

fn validate_cached_source(output: &Path, artifact: &SourceArtifact) -> Result<bool> {
    let path = output.join(&artifact.materialized_path);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            ensure!(
                metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
                "cached source is not a regular file: {}",
                path.display()
            );
            let actual = digest(&fs::read(&path)?);
            ensure!(
                actual == artifact.content_blake3,
                "cached source digest mismatch for {}: expected {}, found {actual}",
                path.display(),
                artifact.content_blake3
            );
            remove_stale_part(&path)?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

fn resolve_case(
    plan: &CasePlan,
    repositories: &RepositoryRegistry,
    cache: &mut BTreeMap<(String, String), FetchedCommit>,
) -> Result<ResolvedCase> {
    let key = (plan.oracle_repository_url.clone(), plan.commit.clone());
    if !cache.contains_key(&key) {
        let original = RepositoryLocation {
            owner: plan.owner.clone(),
            repository: plan.repository.clone(),
            url: plan.oracle_repository_url.clone(),
        };
        let fetched = match fetch_commit(&original.owner, &original.repository, &plan.commit) {
            Ok(commit) => FetchedCommit {
                repository: original,
                commit,
            },
            Err(error) if error.downcast_ref::<GhApiError>().is_some() => {
                let original_failure = format!("{error:#}");
                let mirror = repositories
                    .mirrors
                    .get(&key)
                    .cloned()
                    .ok_or_else(|| {
                        error.context(format!(
                            "original repository {} failed and no repository mirror is configured for exact commit {}",
                            plan.oracle_repository_url, plan.commit
                        ))
                    })?;
                eprintln!(
                    "repository mirror: {}@{} -> {}",
                    plan.oracle_repository_url, plan.commit, mirror.url
                );
                let commit = fetch_commit(&mirror.owner, &mirror.repository, &plan.commit)
                    .with_context(|| {
                        format!(
                            "configured repository mirror {} failed for exact commit {} after original repository {} failed: {original_failure}",
                            mirror.url, plan.commit, plan.oracle_repository_url,
                        )
                    })?;
                FetchedCommit {
                    repository: mirror,
                    commit,
                }
            }
            Err(error) => return Err(error),
        };
        cache.insert(key.clone(), fetched);
    }
    let fetched = &cache[&key];
    let candidates: Vec<_> = fetched
        .commit
        .files
        .iter()
        .filter(|file| {
            file.filename == plan.source_path
                || file.previous_filename.as_deref() == Some(plan.source_path.as_str())
        })
        .collect();
    ensure!(
        candidates.len() == 1,
        "expected one GitHub changed-file match for {} at {}, found {}",
        plan.source_path,
        plan.commit,
        candidates.len()
    );
    let file = candidates[0];
    let (before_path, after_path) = match file.status.as_str() {
        "modified" => {
            ensure!(
                file.filename == plan.source_path && file.previous_filename.is_none(),
                "modified GitHub file has inconsistent paths for {}",
                plan.oracle_path
            );
            (file.filename.clone(), file.filename.clone())
        }
        "renamed" => {
            let previous = file
                .previous_filename
                .clone()
                .context("renamed GitHub file is missing previous_filename")?;
            (previous, file.filename.clone())
        }
        status => bail!(
            "unsupported GitHub file status {status} for {} at {}",
            plan.source_path,
            plan.commit
        ),
    };
    validate_repository_path(&before_path)?;
    validate_repository_path(&after_path)?;
    Ok(ResolvedCase {
        fetched_repository: fetched.repository.clone(),
        parent: fetched.commit.parent.clone(),
        before_path,
        after_path,
    })
}

fn fetch_commit(owner: &str, repository: &str, commit: &str) -> Result<GithubCommit> {
    let mut page_number = 1usize;
    let mut first_parent = None;
    let mut files = Vec::new();
    let mut filenames = BTreeSet::new();
    loop {
        ensure!(
            page_number <= 10_000,
            "GitHub commit pagination did not terminate"
        );
        let endpoint = format!(
            "repos/{}/{}/commits/{commit}?per_page=100&page={page_number}",
            encode_uri_component(owner),
            encode_uri_component(repository)
        );
        let bytes = run_gh_api(&endpoint, None).with_context(|| {
            format!("failed to fetch GitHub commit {owner}/{repository}@{commit}")
        })?;
        let page: GithubCommitPage = serde_json::from_slice(&bytes).with_context(|| {
            format!("invalid GitHub commit response for {owner}/{repository}@{commit}")
        })?;
        ensure!(page.sha == commit, "GitHub commit response SHA mismatch");
        ensure!(
            !page.parents.is_empty(),
            "GitHub commit has no parent: {commit}"
        );
        validate_sha(&page.parents[0].sha)?;
        match &first_parent {
            Some(parent) => ensure!(
                parent == &page.parents[0].sha,
                "GitHub parent changed across pages"
            ),
            None => first_parent = Some(page.parents[0].sha.clone()),
        }
        let page_file_count = page.files.len();
        for file in page.files {
            ensure!(
                filenames.insert(file.filename.clone()),
                "duplicate changed file {} in GitHub response",
                file.filename
            );
            files.push(file);
        }
        if page_file_count < 100 {
            break;
        }
        page_number += 1;
    }
    ensure!(
        !files.is_empty(),
        "GitHub commit contains no changed files: {commit}"
    );
    Ok(GithubCommit {
        parent: first_parent.expect("first parent is set after a non-empty response"),
        files,
    })
}

fn fetch_source(owner: &str, repository: &str, reference: &str, path: &str) -> Result<Vec<u8>> {
    let endpoint = format!(
        "repos/{}/{}/contents/{}?ref={reference}",
        encode_uri_component(owner),
        encode_uri_component(repository),
        encode_uri_path(path)
    );
    run_gh_api(&endpoint, Some("application/vnd.github.raw+json"))
        .with_context(|| format!("failed to download {owner}/{repository}/{path} at {reference}"))
}

fn run_gh_api(endpoint: &str, accept: Option<&str>) -> Result<Vec<u8>> {
    let mut command = Command::new("gh");
    command.arg("api").arg("--method").arg("GET");
    if let Some(accept) = accept {
        command.arg("-H").arg(format!("Accept: {accept}"));
    }
    let output = command
        .arg(endpoint)
        .output()
        .map_err(|error| GhApiError(format!("failed to start gh api for {endpoint}: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GhApiError(format!(
            "gh api failed for {endpoint} with {}: {}",
            output.status,
            stderr.trim_end()
        ))
        .into());
    }
    Ok(output.stdout)
}

fn write_source(output: &Path, relative: &str, expected: &[u8]) -> Result<()> {
    let path = output.join(relative);
    if path.try_exists()? {
        let actual_bytes = fs::read(&path)?;
        let expected_digest = digest(expected);
        let actual_digest = digest(&actual_bytes);
        ensure!(
            actual_bytes == expected,
            "existing source differs from downloaded content for {}: expected {}, found {}",
            path.display(),
            expected_digest,
            actual_digest
        );
        remove_stale_part(&path)?;
        return Ok(());
    }

    let parent = path.parent().context("materialized source has no parent")?;
    fs::create_dir_all(parent)?;
    let part = part_path(&path)?;
    if part.try_exists()? {
        fs::remove_file(&part)
            .with_context(|| format!("failed to remove stale cache file {}", part.display()))?;
    }
    write_new_file(&part, expected)?;
    ensure!(
        !path.try_exists()?,
        "source appeared concurrently: {}",
        path.display()
    );
    fs::rename(&part, &path)
        .with_context(|| format!("failed to install materialized source {}", path.display()))?;
    Ok(())
}

fn remove_stale_part(path: &Path) -> Result<()> {
    let part = part_path(path)?;
    if part.try_exists()? {
        fs::remove_file(&part)
            .with_context(|| format!("failed to remove stale cache file {}", part.display()))?;
    }
    Ok(())
}

fn part_path(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .context("source cache path has no UTF-8 filename")?;
    Ok(path.with_file_name(format!("{name}.part")))
}

fn write_manifest(path: &Path, encoded: &[u8]) -> Result<()> {
    let part = path.with_file_name(MANIFEST_PART_NAME);
    if part.try_exists()? {
        fs::remove_file(&part)
            .with_context(|| format!("failed to remove stale manifest file {}", part.display()))?;
    }
    write_new_file(&part, encoded)?;
    ensure!(
        !path.try_exists()?,
        "manifest appeared concurrently: {}",
        path.display()
    );
    fs::rename(&part, path)
        .with_context(|| format!("failed to install manifest {}", path.display()))?;
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync {}", path.display()))?;
    Ok(())
}

fn encode_manifest(manifest: &Manifest) -> Result<Vec<u8>> {
    let mut encoded = serde_json::to_vec_pretty(manifest)?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn materialized_paths(index: usize) -> (String, String) {
    let directory = case_directory(index);
    (
        format!("sources/{directory}/before.java"),
        format!("sources/{directory}/after.java"),
    )
}

fn case_directory(index: usize) -> String {
    format!("{index:04}")
}

fn path_components(path: &Path) -> Result<Vec<String>> {
    path.components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .context("dataset path contains non-UTF-8 component"),
            _ => bail!("dataset path contains a non-normal component"),
        })
        .collect()
}

fn slash_path(path: &Path) -> Result<String> {
    Ok(path_components(path)?.join("/"))
}

fn validate_sha(value: &str) -> Result<()> {
    ensure!(
        value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "expected a lowercase 40-character Git SHA, found {value}"
    );
    Ok(())
}

fn validate_digest(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid BLAKE3 digest {value}"
    );
    Ok(())
}

fn validate_source_name(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\']),
        "invalid encoded source name {value}"
    );
    Ok(())
}

fn validate_repository_path(value: &str) -> Result<()> {
    ensure!(!value.is_empty(), "repository path is empty");
    ensure!(
        value.ends_with(".java"),
        "repository path is not a Java file: {value}"
    );
    ensure!(
        !value.starts_with('/') && !value.contains('\\'),
        "repository path must be relative and use forward slashes: {value}"
    );
    ensure!(
        value
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != ".."),
        "repository path contains an invalid component: {value}"
    );
    Ok(())
}

fn encode_uri_component(value: &str) -> String {
    encode_uri(value, false)
}

fn encode_uri_path(value: &str) -> String {
    encode_uri(value, true)
}

fn encode_uri(value: &str, preserve_slash: bool) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~')
            || (preserve_slash && byte == b'/')
        {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn report_progress(action: &str, completed: usize, total: usize, oracle_path: &str) {
    if completed.is_multiple_of(10) || completed == total {
        eprintln!("{action} {completed}/{total}: {oracle_path}");
    }
}
