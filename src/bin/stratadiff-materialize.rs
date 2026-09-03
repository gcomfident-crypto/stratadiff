use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, ValueEnum};
use csv::{ReaderBuilder, StringRecord};
use serde::Deserialize;
use stratadiff::diffbenchmark_materialization::{
    DIFFBENCHMARK_LITERATURE_CASES, DIFFBENCHMARK_REVISION, MATERIALIZATION_MANIFEST_SCHEMA,
    MaterializationManifest, MaterializedCase, MaterializedSource,
};

const ORACLE_ROOT: &str = "hrd-oracle/adb-paper/literature-exp";
const LITERATURE_CSV: &str = "csv-outputs/adb-paper/literature-exp-INTRA_FILE_ONLY-NO_FILTER-RefOracle-NO_COMMENTS_AND_JAVADOCS-2025_04_10 18:15:50.csv";
const REPOSITORY_MAP_SCHEMA: &str = "stratadiff-repository-mirrors-v1";
const MARKER_NAME: &str = ".stratadiff-materialize";
const MARKER_CONTENTS: &str = "stratadiff-diffbenchmark-materialization-v1\n";
const MANIFEST_NAME: &str = "manifest.json";
const MANIFEST_PART_NAME: &str = ".manifest.json.part";
const MANIFEST_NEXT_NAME: &str = ".manifest.json.next";
const GIT_CACHE_MARKER: &str = ".stratadiff-git-source-cache";
const GIT_CACHE_MARKER_CONTENTS: &str = "stratadiff-git-source-cache-v1\n";
const GIT_CACHE_STAGING_SUFFIX: &str = ".staging";

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
    /// Source download transport. Git uses a caller-owned partial-clone cache.
    #[arg(long, value_enum, default_value_t = SourceBackend::GithubApi)]
    source_backend: SourceBackend,
    /// Absolute cache directory required by --source-backend git.
    #[arg(long, value_name = "DIRECTORY")]
    git_cache: Option<PathBuf>,
    /// Transport used by --source-backend git.
    #[arg(long, value_enum, default_value_t = GitTransport::Https)]
    git_transport: GitTransport,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SourceBackend {
    GithubApi,
    Git,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum GitTransport {
    Https,
    Ssh,
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

enum SourceFetcher {
    GithubApi,
    Git(GitBlobCache),
}

struct GitBlobCache {
    root: PathBuf,
    transport: GitTransport,
    prepared: BTreeSet<String>,
    fetched: BTreeSet<(String, String)>,
}

#[derive(Debug)]
struct GhApiError(String);

impl std::fmt::Display for GhApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for GhApiError {}

impl SourceFetcher {
    fn new(
        backend: SourceBackend,
        git_cache: Option<&Path>,
        git_transport: GitTransport,
    ) -> Result<Self> {
        match backend {
            SourceBackend::GithubApi => {
                ensure!(
                    git_cache.is_none(),
                    "--git-cache is valid only with --source-backend git"
                );
                ensure!(
                    git_transport == GitTransport::Https,
                    "--git-transport is valid only with --source-backend git"
                );
                Ok(Self::GithubApi)
            }
            SourceBackend::Git => {
                let root = git_cache.context("--source-backend git requires --git-cache")?;
                Ok(Self::Git(GitBlobCache::new(root, git_transport)?))
            }
        }
    }

    fn fetch(
        &mut self,
        repository: &RepositoryLocation,
        revision: &str,
        path: &str,
    ) -> Result<Vec<u8>> {
        match self {
            Self::GithubApi => {
                fetch_source(&repository.owner, &repository.repository, revision, path)
            }
            Self::Git(cache) => cache.fetch(repository, revision, path),
        }
    }
}

impl GitBlobCache {
    fn new(root: &Path, transport: GitTransport) -> Result<Self> {
        ensure!(root.is_absolute(), "git cache directory must be absolute");
        match fs::symlink_metadata(root) {
            Ok(metadata) => ensure!(
                metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
                "git cache is not a regular directory: {}",
                root.display()
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(root)
                .with_context(|| format!("failed to create git cache {}", root.display()))?,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect git cache {}", root.display()));
            }
        }

        let root = fs::canonicalize(root)
            .with_context(|| format!("failed to resolve git cache {}", root.display()))?;
        ensure!(
            root != Path::new("/"),
            "refusing / as the git cache directory"
        );
        let marker = root.join(GIT_CACHE_MARKER);
        let entries = fs::read_dir(&root)?.collect::<std::result::Result<Vec<_>, _>>()?;
        if entries.is_empty() {
            write_new_file(&marker, GIT_CACHE_MARKER_CONTENTS.as_bytes())?;
        } else {
            let metadata = fs::symlink_metadata(&marker).with_context(|| {
                format!(
                    "refusing non-empty git cache without {GIT_CACHE_MARKER}: {}",
                    root.display()
                )
            })?;
            ensure!(
                metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
                "git cache marker is not a regular file: {}",
                marker.display()
            );
            ensure!(
                fs::read(&marker)? == GIT_CACHE_MARKER_CONTENTS.as_bytes(),
                "git cache marker has unexpected contents: {}",
                marker.display()
            );
        }
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            if entry.file_name() == GIT_CACHE_MARKER {
                continue;
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF-8 git cache entry"))?;
            if is_git_cache_staging_name(&name) {
                bail!(
                    "stale git cache staging directory must be removed manually: {}",
                    entry.path().display()
                );
            }
            ensure!(
                name.len() == 64
                    && name.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && entry.file_type()?.is_dir()
                    && !entry.file_type()?.is_symlink(),
                "unexpected git cache entry {}",
                entry.path().display()
            );
        }
        Ok(Self {
            root,
            transport,
            prepared: BTreeSet::new(),
            fetched: BTreeSet::new(),
        })
    }

    fn fetch(
        &mut self,
        repository: &RepositoryLocation,
        revision: &str,
        path: &str,
    ) -> Result<Vec<u8>> {
        validate_sha(revision)?;
        validate_repository_path(path)?;
        let remote_url = match self.transport {
            GitTransport::Https => repository.url.clone(),
            GitTransport::Ssh => format!(
                "git@github.com:{}/{}.git",
                repository.owner, repository.repository
            ),
        };
        let directory = self.root.join(digest(remote_url.as_bytes()));
        if !self.prepared.contains(&remote_url) {
            self.prepare_repository(&directory, &remote_url)?;
            self.prepared.insert(remote_url.clone());
        }
        let key = (remote_url, revision.to_owned());
        if !self.fetched.contains(&key) {
            if !git_commit_is_cached(&directory, revision)? {
                run_git_checked(
                    &directory,
                    &[
                        "fetch",
                        "--no-tags",
                        "--depth=1",
                        "--filter=blob:none",
                        "origin",
                        revision,
                    ],
                    "failed to fetch exact source revision",
                )?;
            }
            self.fetched.insert(key);
        }
        let resolved = run_git_checked_without_lazy_fetch(
            &directory,
            &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
            "failed to resolve fetched source revision",
        )?;
        ensure!(
            trim_line(&resolved)? == revision,
            "git cache resolved a different source revision"
        );
        read_verified_git_blob(&directory, revision, path).with_context(|| {
            format!(
                "failed to download {}/{} at {revision}",
                repository.url, path
            )
        })
    }

    fn prepare_repository(&self, directory: &Path, url: &str) -> Result<()> {
        if !directory.try_exists()? {
            install_cache_directory(directory, |staging| initialize_git_repository(staging, url))
        } else {
            validate_git_repository(directory, url)
        }
    }
}

fn initialize_git_repository(directory: &Path, url: &str) -> Result<()> {
    let output = isolated_git_command()
        .args(["init", "--bare", "--template="])
        .arg(directory)
        .output()
        .context("failed to start git init for source cache")?;
    ensure!(
        output.status.success(),
        "git init failed for source cache: {}",
        String::from_utf8_lossy(&output.stderr).trim_end()
    );
    run_git_checked(
        directory,
        &["remote", "add", "origin", url],
        "failed to configure source cache remote",
    )?;
    run_git_checked(
        directory,
        &["config", "extensions.partialClone", "origin"],
        "failed to configure partial clone extension",
    )?;
    run_git_checked(
        directory,
        &["config", "remote.origin.promisor", "true"],
        "failed to configure promisor remote",
    )?;
    run_git_checked(
        directory,
        &["config", "remote.origin.partialclonefilter", "blob:none"],
        "failed to configure partial clone filter",
    )?;
    validate_git_repository(directory, url)
}

fn install_cache_directory(
    directory: &Path,
    initialize: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let staging = git_cache_staging_path(directory)?;
    ensure!(
        !staging.try_exists()?,
        "stale git cache staging directory must be removed manually: {}",
        staging.display()
    );
    fs::create_dir(&staging)
        .with_context(|| format!("failed to create git cache staging {}", staging.display()))?;

    let result = (|| {
        initialize(&staging)?;
        ensure!(
            !directory.try_exists()?,
            "git cache entry appeared concurrently: {}",
            directory.display()
        );
        fs::rename(&staging, directory).with_context(|| {
            format!("failed to install git cache entry {}", directory.display())
        })?;
        Ok(())
    })();
    match result {
        Ok(()) => Ok(()),
        Err(error) => match remove_git_cache_staging(&staging) {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(error.context(format!(
                "failed to clean git cache staging {}: {cleanup_error:#}",
                staging.display()
            ))),
        },
    }
}

fn remove_git_cache_staging(staging: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(staging) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect staging path {}", staging.display()));
        }
    };
    ensure!(
        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
        "refusing to remove non-directory git cache staging path {}",
        staging.display()
    );
    fs::remove_dir_all(staging)
        .with_context(|| format!("failed to remove git cache staging {}", staging.display()))
}

fn git_cache_staging_path(directory: &Path) -> Result<PathBuf> {
    let name = directory
        .file_name()
        .and_then(OsStr::to_str)
        .context("git cache entry has no UTF-8 name")?;
    Ok(directory.with_file_name(format!(".{name}{GIT_CACHE_STAGING_SUFFIX}")))
}

fn is_git_cache_staging_name(name: &str) -> bool {
    name.strip_prefix('.')
        .and_then(|name| name.strip_suffix(GIT_CACHE_STAGING_SUFFIX))
        .is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn validate_git_repository(directory: &Path, url: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    ensure!(
        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
        "git source cache repository is not a directory: {}",
        directory.display()
    );
    let canonical = fs::canonicalize(directory).with_context(|| {
        format!(
            "failed to resolve git source cache repository {}",
            directory.display()
        )
    })?;
    ensure!(
        canonical == directory,
        "git source cache repository does not have a canonical path: {}",
        directory.display()
    );
    let bare = run_git_checked(
        directory,
        &["rev-parse", "--is-bare-repository"],
        "git source cache entry is not a valid bare repository",
    )?;
    ensure!(
        trim_line(&bare)? == "true",
        "git source cache entry is not bare: {}",
        directory.display()
    );
    validate_git_local_config(directory)?;
    validate_git_directory_path(directory, "--absolute-git-dir", "git directory")?;
    validate_git_directory_path(directory, "--git-common-dir", "common directory")?;
    validate_git_object_storage(directory)?;

    let replacements = run_git_checked(
        directory,
        &["for-each-ref", "--format=%(refname)", "refs/replace/"],
        "failed to inspect source cache replacement refs",
    )?;
    ensure!(
        replacements.is_empty(),
        "git source cache contains replacement refs: {}",
        directory.display()
    );

    let configured = run_git_checked(
        directory,
        &["remote", "get-url", "origin"],
        "failed to inspect source cache remote",
    )?;
    ensure!(
        trim_line(&configured)? == url,
        "git source cache remote URL mismatch for {}",
        directory.display()
    );
    validate_git_config(directory, "extensions.partialClone", "origin")?;
    validate_git_config(directory, "remote.origin.promisor", "true")?;
    validate_git_config(directory, "remote.origin.partialclonefilter", "blob:none")?;
    Ok(())
}

fn validate_git_local_config(directory: &Path) -> Result<()> {
    const ALLOWED: &[&str] = &[
        "core.bare",
        "core.filemode",
        "core.repositoryformatversion",
        "extensions.partialclone",
        "remote.origin.fetch",
        "remote.origin.partialclonefilter",
        "remote.origin.promisor",
        "remote.origin.url",
    ];
    let output = run_git_checked(
        directory,
        &[
            "config",
            "--local",
            "--no-includes",
            "--name-only",
            "--list",
        ],
        "failed to inspect source cache local config",
    )?;
    let output = std::str::from_utf8(&output).context("git config output is not UTF-8")?;
    let mut seen = BTreeSet::new();
    for key in output.lines() {
        let key = key.to_ascii_lowercase();
        ensure!(
            ALLOWED.binary_search(&key.as_str()).is_ok(),
            "unsupported git source cache config {key} in {}",
            directory.display()
        );
        ensure!(
            seen.insert(key.clone()),
            "duplicate git source cache config {key} in {}",
            directory.display()
        );
    }
    Ok(())
}

fn validate_git_directory_path(directory: &Path, argument: &str, label: &str) -> Result<()> {
    let output = run_git_checked(
        directory,
        &["rev-parse", argument],
        &format!("failed to inspect source cache {label}"),
    )?;
    let reported = PathBuf::from(trim_line(&output)?);
    ensure!(
        reported.is_absolute(),
        "git source cache {label} is not absolute: {}",
        reported.display()
    );
    let reported = fs::canonicalize(&reported).with_context(|| {
        format!(
            "failed to resolve source cache {label} {}",
            reported.display()
        )
    })?;
    ensure!(
        reported == directory,
        "git source cache {label} escapes its cache entry: expected {}, found {}",
        directory.display(),
        reported.display()
    );
    Ok(())
}

fn validate_git_config(directory: &Path, key: &str, expected: &str) -> Result<()> {
    let value = run_git_checked(
        directory,
        &["config", "--get", key],
        &format!("failed to inspect source cache config {key}"),
    )?;
    ensure!(
        trim_line(&value)? == expected,
        "git source cache config {key} mismatch for {}",
        directory.display()
    );
    Ok(())
}

fn validate_git_object_storage(directory: &Path) -> Result<()> {
    for path in [directory.join("objects"), directory.join("objects/info")] {
        let metadata = fs::symlink_metadata(&path).with_context(|| {
            format!("failed to inspect Git object directory {}", path.display())
        })?;
        ensure!(
            metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
            "Git object directory is not a regular directory: {}",
            path.display()
        );
    }
    for name in ["alternates", "http-alternates"] {
        let path = directory.join("objects/info").join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => bail!(
                "git source cache must not use an alternate object store: {}",
                path.display()
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
            }
        }
    }
    Ok(())
}

fn isolated_git_command() -> Command {
    let mut command = Command::new("git");
    for (name, _) in env::vars_os() {
        if unsafe_git_environment(&name) {
            command.env_remove(name);
        }
    }
    for name in [
        "HOME",
        "XDG_CONFIG_HOME",
        "USERPROFILE",
        "GIT_ASKPASS",
        "GIT_PROXY_COMMAND",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "SSH_ASKPASS",
        "SSH_ASKPASS_REQUIRE",
    ] {
        command.env_remove(name);
    }
    command.env("GIT_CONFIG_GLOBAL", null_device());
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.env("GIT_CONFIG_SYSTEM", null_device());
    command.env("GIT_NO_REPLACE_OBJECTS", "1");
    command.args(["--no-replace-objects", "-c", "core.sshCommand=ssh"]);
    command
}

#[cfg(windows)]
fn null_device() -> &'static str {
    "NUL"
}

#[cfg(not(windows))]
fn null_device() -> &'static str {
    "/dev/null"
}

fn unsafe_git_environment(name: &OsStr) -> bool {
    const EXACT: &[&str] = &[
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_DEFAULT_HASH",
        "GIT_DEFAULT_REF_FORMAT",
        "GIT_DIR",
        "GIT_EXEC_PATH",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_INTERNAL_SUPER_PREFIX",
        "GIT_NAMESPACE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
        "GIT_QUARANTINE_PATH",
        "GIT_REPLACE_REF_BASE",
        "GIT_SHALLOW_FILE",
        "GIT_TEMPLATE_DIR",
        "GIT_WORK_TREE",
    ];
    EXACT.iter().any(|candidate| name == OsStr::new(candidate))
        || name
            .to_str()
            .is_some_and(|name| name == "GIT_CONFIG" || name.starts_with("GIT_CONFIG_"))
}

fn run_git_checked(directory: &Path, arguments: &[&str], context: &str) -> Result<Vec<u8>> {
    let output = git_repository_command(directory)
        .args(arguments)
        .output()
        .with_context(|| context.to_owned())?;
    ensure!(
        output.status.success(),
        "{context}: {}",
        String::from_utf8_lossy(&output.stderr).trim_end()
    );
    Ok(output.stdout)
}

fn run_git_checked_without_lazy_fetch(
    directory: &Path,
    arguments: &[&str],
    context: &str,
) -> Result<Vec<u8>> {
    let output = git_repository_command(directory)
        .env("GIT_NO_LAZY_FETCH", "1")
        .args(arguments)
        .output()
        .with_context(|| context.to_owned())?;
    ensure!(
        output.status.success(),
        "{context}: {}",
        String::from_utf8_lossy(&output.stderr).trim_end()
    );
    Ok(output.stdout)
}

fn read_verified_git_blob(directory: &Path, revision: &str, path: &str) -> Result<Vec<u8>> {
    let object = format!("{revision}:{path}");
    let expected_oid = run_git_checked(
        directory,
        &["rev-parse", "--verify", &object],
        "failed to resolve source blob in git cache",
    )?;
    let expected_oid = trim_line(&expected_oid)?;
    validate_sha(expected_oid).context("git source blob has an invalid object ID")?;

    let bytes = run_git_checked(
        directory,
        &["cat-file", "blob", expected_oid],
        "failed to read source blob from git cache",
    )?;
    let actual_oid = hash_git_blob(directory, &bytes)?;
    ensure!(
        actual_oid == expected_oid,
        "git source blob object ID mismatch: expected {expected_oid}, found {actual_oid}"
    );
    Ok(bytes)
}

fn hash_git_blob(directory: &Path, bytes: &[u8]) -> Result<String> {
    let mut child = git_repository_command(directory)
        .env("GIT_NO_LAZY_FETCH", "1")
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start git hash-object for source blob")?;
    let write_result = child
        .stdin
        .take()
        .context("git hash-object stdin is unavailable")?
        .write_all(bytes);
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error).context("failed to write source blob to git hash-object");
    }
    let output = child
        .wait_with_output()
        .context("failed to wait for git hash-object")?;
    ensure!(
        output.status.success(),
        "git hash-object failed for source blob: {}",
        String::from_utf8_lossy(&output.stderr).trim_end()
    );
    let oid = trim_line(&output.stdout)?;
    validate_sha(oid).context("git hash-object returned an invalid object ID")?;
    Ok(oid.to_owned())
}

fn git_commit_is_cached(directory: &Path, revision: &str) -> Result<bool> {
    let output = git_repository_command(directory)
        .env("GIT_NO_LAZY_FETCH", "1")
        .args(["rev-parse", "--verify", &format!("{revision}^{{commit}}")])
        .output()
        .context("failed to inspect exact source revision in git cache")?;
    if !output.status.success() {
        return Ok(false);
    }
    ensure!(
        trim_line(&output.stdout)? == revision,
        "git cache resolved a different source revision"
    );
    Ok(true)
}

fn git_repository_command(directory: &Path) -> Command {
    let mut command = isolated_git_command();
    command.arg("--git-dir").arg(directory);
    command
}

fn trim_line(bytes: &[u8]) -> Result<&str> {
    let value = std::str::from_utf8(bytes).context("git output is not UTF-8")?;
    Ok(value.trim_end_matches(['\r', '\n']))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    validate_revision(&cli.checkout)?;
    let repositories = read_repository_registry(cli.repository_map.as_deref())?;
    let plans = build_case_plans(&cli.checkout)?;
    prepare_output(&cli.output, plans.len())?;
    let mut source_fetcher = SourceFetcher::new(
        cli.source_backend,
        cli.git_cache.as_deref(),
        cli.git_transport,
    )?;

    let manifest_path = cli.output.join(MANIFEST_NAME);
    let cached_manifest = read_cached_manifest(&manifest_path)?;
    let had_cached_manifest = cached_manifest.is_some();
    let manifest = match cached_manifest {
        Some(manifest) => {
            validate_cached_manifest(&manifest, &plans, &repositories)?;
            complete_cached_manifest(&cli.output, manifest, &plans, &mut source_fetcher)?
        }
        None => {
            let checkpoint = read_checkpoint(&cli.output)?;
            materialize_new_manifest(
                &cli.output,
                &plans,
                &repositories,
                checkpoint,
                &mut source_fetcher,
            )?
        }
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
        remove_checkpoints(&cli.output)?;
    } else {
        write_manifest(&manifest_path, &encoded)?;
        remove_checkpoints(&cli.output)?;
    }
    io::stdout().lock().write_all(&encoded)?;
    Ok(())
}

fn validate_revision(checkout: &Path) -> Result<()> {
    let output = isolated_git_command()
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
        oracle_files.len() == DIFFBENCHMARK_LITERATURE_CASES,
        "expected {DIFFBENCHMARK_LITERATURE_CASES} literature GOD.json files, found {}",
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
            Some(MARKER_NAME | MANIFEST_NAME | MANIFEST_PART_NAME | MANIFEST_NEXT_NAME) => ensure!(
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
                    Some(
                        "before.source"
                            | "after.source"
                            | "before.source.part"
                            | "after.source.part"
                    )
                ) && source_type.is_file()
                    && !source_type.is_symlink(),
                "unexpected source cache entry {}",
                source.path().display()
            );
        }
    }
    Ok(())
}

fn read_cached_manifest(path: &Path) -> Result<Option<MaterializationManifest>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid cached manifest {}", path.display()))
            .map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn read_checkpoint(output: &Path) -> Result<Option<MaterializationManifest>> {
    let part = read_cached_manifest(&output.join(MANIFEST_PART_NAME));
    let next = read_cached_manifest(&output.join(MANIFEST_NEXT_NAME));
    match (part, next) {
        (Ok(part), Ok(next)) => Ok([part, next]
            .into_iter()
            .flatten()
            .max_by_key(|manifest| manifest.cases.len())),
        (Ok(Some(part)), Err(_)) => Ok(Some(part)),
        (Err(_), Ok(Some(next))) => Ok(Some(next)),
        (Ok(None), Err(error)) | (Err(error), Ok(None)) | (Err(error), Err(_)) => Err(error),
    }
}

fn validate_cached_manifest(
    manifest: &MaterializationManifest,
    plans: &[CasePlan],
    repositories: &RepositoryRegistry,
) -> Result<()> {
    ensure!(
        manifest.schema == MATERIALIZATION_MANIFEST_SCHEMA,
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
    manifest: MaterializationManifest,
    plans: &[CasePlan],
    source_fetcher: &mut SourceFetcher,
) -> Result<MaterializationManifest> {
    for (index, (case, plan)) in manifest.cases.iter().zip(plans).enumerate() {
        let before_exists = validate_cached_source(output, &case.before)?;
        let after_exists = validate_cached_source(output, &case.after)?;
        if before_exists && after_exists {
            report_progress("verified", index + 1, plans.len(), &plan.oracle_path);
            continue;
        }

        let fetched_repository = parse_repository_url(&case.fetched_repository_url)?;
        if !before_exists {
            let bytes = source_fetcher.fetch(
                &fetched_repository,
                &case.parent,
                &case.before.repository_path,
            )?;
            ensure!(
                digest(&bytes) == case.before.content_blake3,
                "downloaded before source digest disagrees with cached manifest for {}",
                plan.oracle_path
            );
            write_source(output, &case.before.materialized_path, &bytes)?;
        }
        if !after_exists {
            let bytes = source_fetcher.fetch(
                &fetched_repository,
                &plan.commit,
                &case.after.repository_path,
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
    checkpoint: Option<MaterializationManifest>,
    source_fetcher: &mut SourceFetcher,
) -> Result<MaterializationManifest> {
    let mut commits = BTreeMap::new();
    let mut manifest = checkpoint.unwrap_or_else(|| MaterializationManifest {
        schema: MATERIALIZATION_MANIFEST_SCHEMA.to_owned(),
        dataset_revision: DIFFBENCHMARK_REVISION.to_owned(),
        case_count: plans.len(),
        cases: Vec::with_capacity(plans.len()),
    });
    validate_checkpoint(&manifest, plans, repositories)?;
    for (index, case) in manifest.cases.iter().enumerate() {
        ensure!(
            validate_cached_source(output, &case.before)?
                && validate_cached_source(output, &case.after)?,
            "checkpoint source is missing for {}",
            case.oracle_path
        );
        report_progress("resumed", index + 1, plans.len(), &case.oracle_path);
    }

    for (index, plan) in plans.iter().enumerate().skip(manifest.cases.len()) {
        let resolved = resolve_case(plan, repositories, &mut commits)?;
        let before_bytes = source_fetcher.fetch(
            &resolved.fetched_repository,
            &resolved.parent,
            &resolved.before_path,
        )?;
        let after_bytes = source_fetcher.fetch(
            &resolved.fetched_repository,
            &plan.commit,
            &resolved.after_path,
        )?;
        let (before_path, after_path) = materialized_paths(index);
        write_source(output, &before_path, &before_bytes)?;
        write_source(output, &after_path, &after_bytes)?;
        manifest.cases.push(MaterializedCase {
            oracle_path: plan.oracle_path.clone(),
            oracle_blake3: plan.oracle_blake3.clone(),
            oracle_repository_url: plan.oracle_repository_url.clone(),
            fetched_repository_url: resolved.fetched_repository.url,
            commit: plan.commit.clone(),
            parent: resolved.parent,
            before: MaterializedSource {
                repository_path: resolved.before_path,
                materialized_path: before_path,
                content_blake3: digest(&before_bytes),
            },
            after: MaterializedSource {
                repository_path: resolved.after_path,
                materialized_path: after_path,
                content_blake3: digest(&after_bytes),
            },
        });
        write_checkpoint(output, &manifest)?;
        report_progress("materialized", index + 1, plans.len(), &plan.oracle_path);
    }
    Ok(manifest)
}

fn validate_checkpoint(
    manifest: &MaterializationManifest,
    plans: &[CasePlan],
    repositories: &RepositoryRegistry,
) -> Result<()> {
    ensure!(
        manifest.schema == MATERIALIZATION_MANIFEST_SCHEMA,
        "checkpoint manifest schema mismatch"
    );
    ensure!(
        manifest.dataset_revision == DIFFBENCHMARK_REVISION,
        "checkpoint manifest dataset revision mismatch"
    );
    ensure!(
        manifest.case_count == plans.len() && manifest.cases.len() <= plans.len(),
        "checkpoint manifest case count mismatch"
    );
    let prefix = MaterializationManifest {
        schema: manifest.schema.clone(),
        dataset_revision: manifest.dataset_revision.clone(),
        case_count: manifest.cases.len(),
        cases: manifest.cases.clone(),
    };
    validate_cached_manifest(&prefix, &plans[..manifest.cases.len()], repositories)
        .context("invalid materialization checkpoint")
}

fn validate_cached_source(output: &Path, artifact: &MaterializedSource) -> Result<bool> {
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
    let actual_bytes = fs::read(&path)
        .with_context(|| format!("failed to read back materialized source {}", path.display()))?;
    let expected_digest = digest(expected);
    let actual_digest = digest(&actual_bytes);
    ensure!(
        actual_bytes == expected,
        "installed source differs from downloaded content for {}: expected {}, found {}",
        path.display(),
        expected_digest,
        actual_digest
    );
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
    let next = path.with_file_name(MANIFEST_NEXT_NAME);
    if next.try_exists()? {
        fs::remove_file(&next)
            .with_context(|| format!("failed to remove stale manifest file {}", next.display()))?;
    }
    write_new_file(&next, encoded)?;
    ensure!(
        !path.try_exists()?,
        "manifest appeared concurrently: {}",
        path.display()
    );
    fs::rename(&next, path)
        .with_context(|| format!("failed to install manifest {}", path.display()))?;
    Ok(())
}

fn write_checkpoint(output: &Path, manifest: &MaterializationManifest) -> Result<()> {
    let path = output.join(MANIFEST_PART_NAME);
    let next = output.join(MANIFEST_NEXT_NAME);
    if next.try_exists()? {
        fs::remove_file(&next)
            .with_context(|| format!("failed to remove stale checkpoint {}", next.display()))?;
    }
    write_new_file(&next, &encode_manifest(manifest)?)?;
    fs::rename(&next, &path)
        .with_context(|| format!("failed to install checkpoint {}", path.display()))?;
    Ok(())
}

fn remove_checkpoints(output: &Path) -> Result<()> {
    for name in [MANIFEST_PART_NAME, MANIFEST_NEXT_NAME] {
        let path = output.join(name);
        if path.try_exists()? {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove checkpoint {}", path.display()))?;
        }
    }
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

fn encode_manifest(manifest: &MaterializationManifest) -> Result<Vec<u8>> {
    let mut encoded = serde_json::to_vec_pretty(manifest)?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn materialized_paths(index: usize) -> (String, String) {
    let directory = case_directory(index);
    (
        format!("sources/{directory}/before.source"),
        format!("sources/{directory}/after.source"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const FIRST_SOURCE: &[u8] = b"class Demo { int first; }\n";
    const SECOND_SOURCE: &[u8] = b"class Demo { int second; }\n";
    const ENV_CHILD: &str = "STRATADIFF_GIT_CACHE_ENV_CHILD";
    const ENV_REMOTE: &str = "STRATADIFF_GIT_CACHE_ENV_REMOTE";
    const ENV_CACHE: &str = "STRATADIFF_GIT_CACHE_ENV_ROOT";
    const ENV_REVISION: &str = "STRATADIFF_GIT_CACHE_ENV_REVISION";

    struct LocalGitFixture {
        temporary_directory: TempDir,
        work: PathBuf,
        remote: PathBuf,
        first_revision: String,
        second_revision: String,
        repository: RepositoryLocation,
    }

    impl LocalGitFixture {
        fn new() -> Self {
            let temporary_directory = tempfile::tempdir().unwrap();
            let work = temporary_directory.path().join("work");
            let remote = temporary_directory.path().join("remote.git");

            let mut command = isolated_git_command();
            command.arg("init").arg(&work);
            checked(&mut command);
            for (key, value) in [
                ("user.name", "StrataDiff Test"),
                ("user.email", "stratadiff@example.invalid"),
            ] {
                let mut command = isolated_git_command();
                command.arg("-C").arg(&work).args(["config", key, value]);
                checked(&mut command);
            }

            fs::write(work.join("Demo.java"), FIRST_SOURCE).unwrap();
            commit_all(&work, "first");
            let first_revision = revision(&work);
            fs::write(work.join("Demo.java"), SECOND_SOURCE).unwrap();
            commit_all(&work, "second");
            let second_revision = revision(&work);

            let mut command = isolated_git_command();
            command.arg("clone").arg("--bare").arg(&work).arg(&remote);
            checked(&mut command);
            for key in [
                "uploadpack.allowFilter",
                "uploadpack.allowReachableSHA1InWant",
                "uploadpack.allowAnySHA1InWant",
            ] {
                let mut command = isolated_git_command();
                command
                    .arg("--git-dir")
                    .arg(&remote)
                    .args(["config", key, "true"]);
                checked(&mut command);
            }

            let url = format!("file://{}", remote.display());
            Self {
                temporary_directory,
                work,
                remote,
                first_revision,
                second_revision,
                repository: RepositoryLocation {
                    owner: "local".to_owned(),
                    repository: "fixture".to_owned(),
                    url,
                },
            }
        }

        fn cache_root(&self, name: &str) -> PathBuf {
            self.temporary_directory.path().join(name)
        }

        fn cache_entry(&self, root: &Path) -> PathBuf {
            fs::canonicalize(root)
                .unwrap()
                .join(digest(self.repository.url.as_bytes()))
        }
    }

    #[test]
    fn git_cache_fetches_an_exact_blob_from_a_real_repository() {
        let fixture = LocalGitFixture::new();
        let root = fixture.cache_root("cache");
        let mut cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();

        let bytes = cache
            .fetch(&fixture.repository, &fixture.second_revision, "Demo.java")
            .unwrap();

        assert_eq!(bytes, SECOND_SOURCE);
        validate_git_repository(&fixture.cache_entry(&root), &fixture.repository.url).unwrap();
    }

    #[test]
    fn git_cache_probe_does_not_lazy_fetch_a_missing_commit() {
        let fixture = LocalGitFixture::new();
        let root = fixture.cache_root("cache");
        let cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();
        let directory = fixture.cache_entry(&root);
        cache
            .prepare_repository(&directory, &fixture.repository.url)
            .unwrap();

        assert!(!git_commit_is_cached(&directory, &fixture.second_revision).unwrap());
    }

    #[test]
    fn git_cache_hit_skips_the_network_fetch() {
        let fixture = LocalGitFixture::new();
        let root = fixture.cache_root("cache");
        let mut cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();
        assert_eq!(
            cache
                .fetch(&fixture.repository, &fixture.second_revision, "Demo.java")
                .unwrap(),
            SECOND_SOURCE
        );
        drop(cache);
        fs::rename(
            &fixture.remote,
            fixture
                .temporary_directory
                .path()
                .join("remote-unavailable"),
        )
        .unwrap();

        let mut cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();
        assert_eq!(
            cache
                .fetch(&fixture.repository, &fixture.second_revision, "Demo.java")
                .unwrap(),
            SECOND_SOURCE
        );
    }

    #[test]
    fn completed_manifest_metadata_enables_offline_git_recovery() {
        let fixture = LocalGitFixture::new();
        let root = fixture.cache_root("cache");
        let repository = RepositoryLocation {
            owner: "offline".to_owned(),
            repository: "fixture".to_owned(),
            url: "https://github.com/offline/fixture".to_owned(),
        };
        let cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();
        let directory = cache.root.join(digest(repository.url.as_bytes()));
        cache
            .prepare_repository(&directory, &repository.url)
            .unwrap();
        run_git_checked(
            &directory,
            &[
                "fetch",
                "--no-tags",
                "--depth=2",
                &fixture.repository.url,
                &fixture.second_revision,
            ],
            "failed to pre-populate offline test cache",
        )
        .unwrap();
        fs::rename(
            &fixture.remote,
            fixture
                .temporary_directory
                .path()
                .join("remote-unavailable"),
        )
        .unwrap();

        let output = fixture.cache_root("output");
        let before_path = "sources/0000/before.source";
        let after_path = "sources/0000/after.source";
        write_source(&output, before_path, FIRST_SOURCE).unwrap();
        let manifest = MaterializationManifest {
            schema: MATERIALIZATION_MANIFEST_SCHEMA.to_owned(),
            dataset_revision: DIFFBENCHMARK_REVISION.to_owned(),
            case_count: 1,
            cases: vec![MaterializedCase {
                oracle_path: "oracle/GOD.json".to_owned(),
                oracle_blake3: digest(b"oracle"),
                oracle_repository_url: repository.url.clone(),
                fetched_repository_url: repository.url,
                commit: fixture.second_revision.clone(),
                parent: fixture.first_revision.clone(),
                before: MaterializedSource {
                    repository_path: "Demo.java".to_owned(),
                    materialized_path: before_path.to_owned(),
                    content_blake3: digest(FIRST_SOURCE),
                },
                after: MaterializedSource {
                    repository_path: "Demo.java".to_owned(),
                    materialized_path: after_path.to_owned(),
                    content_blake3: digest(SECOND_SOURCE),
                },
            }],
        };
        let plans = vec![CasePlan {
            oracle_path: "oracle/GOD.json".to_owned(),
            oracle_blake3: digest(b"oracle"),
            owner: "offline".to_owned(),
            repository: "fixture".to_owned(),
            oracle_repository_url: "https://github.com/offline/fixture".to_owned(),
            commit: fixture.second_revision.clone(),
            source_path: "Demo.java".to_owned(),
        }];
        let mut source_fetcher =
            SourceFetcher::Git(GitBlobCache::new(&root, GitTransport::Https).unwrap());

        complete_cached_manifest(&output, manifest, &plans, &mut source_fetcher).unwrap();

        assert_eq!(fs::read(output.join(after_path)).unwrap(), SECOND_SOURCE);
    }

    #[test]
    fn git_cache_rejects_blob_content_that_does_not_match_its_object_id() {
        let fixture = LocalGitFixture::new();
        let git_directory = fixture.work.join(".git");
        let first_oid = run_git_checked_without_lazy_fetch(
            &git_directory,
            &[
                "rev-parse",
                "--verify",
                &format!("{}:Demo.java", fixture.first_revision),
            ],
            "failed to resolve first test blob",
        )
        .unwrap();
        let first_oid = trim_line(&first_oid).unwrap();
        let second_oid = run_git_checked_without_lazy_fetch(
            &git_directory,
            &[
                "rev-parse",
                "--verify",
                &format!("{}:Demo.java", fixture.second_revision),
            ],
            "failed to resolve second test blob",
        )
        .unwrap();
        let second_oid = trim_line(&second_oid).unwrap();
        let first_object = loose_object_path(&git_directory, first_oid);
        let second_object = loose_object_path(&git_directory, second_oid);
        fs::remove_file(&first_object).unwrap();
        fs::copy(second_object, first_object).unwrap();

        let error = read_verified_git_blob(&git_directory, &fixture.first_revision, "Demo.java")
            .unwrap_err();

        assert!(
            error.to_string().contains("object ID mismatch"),
            "{error:#}"
        );
    }

    #[test]
    fn git_cache_rejects_an_existing_non_bare_repository() {
        let fixture = LocalGitFixture::new();
        let root = fixture.cache_root("cache");
        let mut cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();
        let directory = fixture.cache_entry(&root);
        let mut command = isolated_git_command();
        command.arg("init").arg(&directory);
        checked(&mut command);

        let error = cache
            .fetch(&fixture.repository, &fixture.second_revision, "Demo.java")
            .unwrap_err();

        assert!(
            format!("{error:#}").contains("not a valid bare repository"),
            "{error:#}"
        );
    }

    #[test]
    fn git_cache_rejects_replacement_refs() {
        let fixture = LocalGitFixture::new();
        let root = fixture.cache_root("cache");
        let mut cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();
        cache
            .fetch(&fixture.repository, &fixture.first_revision, "Demo.java")
            .unwrap();
        cache
            .fetch(&fixture.repository, &fixture.second_revision, "Demo.java")
            .unwrap();
        let directory = fixture.cache_entry(&root);
        let mut command = isolated_git_command();
        command.arg("--git-dir").arg(&directory).args([
            "replace",
            &fixture.first_revision,
            &fixture.second_revision,
        ]);
        checked(&mut command);
        let mut cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();

        let error = cache
            .fetch(&fixture.repository, &fixture.first_revision, "Demo.java")
            .unwrap_err();

        assert!(
            error.to_string().contains("contains replacement refs"),
            "{error:#}"
        );
    }

    #[test]
    fn git_cache_rejects_an_alternate_object_store() {
        let fixture = LocalGitFixture::new();
        let root = fixture.cache_root("cache");
        let mut cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();
        cache
            .fetch(&fixture.repository, &fixture.second_revision, "Demo.java")
            .unwrap();
        let directory = fixture.cache_entry(&root);
        fs::write(
            directory.join("objects/info/alternates"),
            format!("{}\n", fixture.remote.join("objects").display()),
        )
        .unwrap();
        let mut cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();

        let error = cache
            .fetch(&fixture.repository, &fixture.second_revision, "Demo.java")
            .unwrap_err();

        assert!(
            error.to_string().contains("alternate object store"),
            "{error:#}"
        );
    }

    #[test]
    fn git_cache_rejects_unapproved_local_configuration() {
        let fixture = LocalGitFixture::new();
        let root = fixture.cache_root("cache");
        let mut cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();
        cache
            .fetch(&fixture.repository, &fixture.second_revision, "Demo.java")
            .unwrap();
        let directory = fixture.cache_entry(&root);
        let mut command = isolated_git_command();
        command.arg("--git-dir").arg(&directory).args([
            "config",
            "core.sshCommand",
            "/definitely/not/an/approved/ssh",
        ]);
        checked(&mut command);
        let mut cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();

        let error = cache
            .fetch(&fixture.repository, &fixture.second_revision, "Demo.java")
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unsupported git source cache config core.sshcommand"),
            "{error:#}"
        );
    }

    #[test]
    fn git_cache_environment_pollution_does_not_redirect_operations() {
        let fixture = LocalGitFixture::new();
        let cache_root = fixture.cache_root("polluted-cache");
        let hijacked = fixture.cache_root("hijacked.git");
        let foreign_objects = fixture.cache_root("foreign-objects");
        let poisoned_config = fixture.cache_root("poisoned.gitconfig");
        let missing_template = fixture.cache_root("missing-template");
        fs::create_dir(&foreign_objects).unwrap();
        fs::write(
            &poisoned_config,
            "[remote \"origin\"]\n\turl = file:///not-the-requested-repository\n",
        )
        .unwrap();
        let mut command = isolated_git_command();
        command.args(["init", "--bare"]).arg(&hijacked);
        checked(&mut command);

        let output = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::git_cache_environment_child",
                "--nocapture",
            ])
            .env(ENV_CHILD, "1")
            .env(ENV_REMOTE, &fixture.remote)
            .env(ENV_CACHE, &cache_root)
            .env(ENV_REVISION, &fixture.second_revision)
            .env("GIT_DIR", &hijacked)
            .env("GIT_WORK_TREE", fixture.temporary_directory.path())
            .env("GIT_COMMON_DIR", &hijacked)
            .env("GIT_OBJECT_DIRECTORY", &foreign_objects)
            .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", hijacked.join("objects"))
            .env("GIT_CONFIG_GLOBAL", &poisoned_config)
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "remote.origin.url")
            .env(
                "GIT_CONFIG_VALUE_0",
                "file:///also-not-the-requested-repository",
            )
            .env("GIT_DEFAULT_HASH", "sha256")
            .env("GIT_REPLACE_REF_BASE", "refs/heads")
            .env("GIT_SHALLOW_FILE", fixture.cache_root("foreign-shallow"))
            .env("GIT_TEMPLATE_DIR", &missing_template)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(fs::read_dir(&foreign_objects).unwrap().next().is_none());
        let mut command = isolated_git_command();
        command.arg("--git-dir").arg(&hijacked).arg("remote");
        assert!(checked(&mut command).is_empty());
    }

    #[test]
    fn isolated_git_commands_remove_external_config_and_ssh_overrides() {
        let command = isolated_git_command();
        for name in [
            "HOME",
            "XDG_CONFIG_HOME",
            "USERPROFILE",
            "GIT_ASKPASS",
            "GIT_PROXY_COMMAND",
            "GIT_SSH",
            "GIT_SSH_COMMAND",
            "SSH_ASKPASS",
            "SSH_ASKPASS_REQUIRE",
        ] {
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| key == OsStr::new(name) && value.is_none()),
                "{name} was inherited"
            );
        }
        for (name, expected) in [
            ("GIT_CONFIG_GLOBAL", null_device()),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_SYSTEM", null_device()),
        ] {
            assert!(command.get_envs().any(|(key, value)| {
                key == OsStr::new(name) && value == Some(OsStr::new(expected))
            }));
        }
        let arguments: Vec<_> = command.get_args().collect();
        assert_eq!(
            arguments,
            [
                OsStr::new("--no-replace-objects"),
                OsStr::new("-c"),
                OsStr::new("core.sshCommand=ssh")
            ]
        );
    }

    #[test]
    fn git_cache_environment_child() {
        if env::var_os(ENV_CHILD).is_none() {
            return;
        }
        let remote = PathBuf::from(env::var_os(ENV_REMOTE).unwrap());
        let root = PathBuf::from(env::var_os(ENV_CACHE).unwrap());
        let revision = env::var(ENV_REVISION).unwrap();
        let repository = RepositoryLocation {
            owner: "local".to_owned(),
            repository: "fixture".to_owned(),
            url: format!("file://{}", remote.display()),
        };
        let mut cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();

        assert_eq!(
            cache.fetch(&repository, &revision, "Demo.java").unwrap(),
            SECOND_SOURCE
        );
    }

    #[test]
    fn failed_atomic_cache_initialization_removes_its_staging_directory() {
        let temporary_directory = tempfile::tempdir().unwrap();
        let directory = temporary_directory.path().join("a".repeat(64));
        let staging = git_cache_staging_path(&directory).unwrap();

        let error = install_cache_directory(&directory, |staging| {
            fs::write(staging.join("partial"), b"partial")?;
            bail!("injected initialization failure")
        })
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("injected initialization failure")
        );
        assert!(!directory.exists());
        assert!(!staging.exists());
    }

    #[test]
    fn stale_git_cache_staging_is_diagnosed() {
        let temporary_directory = tempfile::tempdir().unwrap();
        let root = temporary_directory.path().join("cache");
        let cache = GitBlobCache::new(&root, GitTransport::Https).unwrap();
        let directory = cache.root.join("b".repeat(64));
        let staging = git_cache_staging_path(&directory).unwrap();
        fs::create_dir(&staging).unwrap();

        let error = match GitBlobCache::new(&root, GitTransport::Https) {
            Ok(_) => panic!("stale staging directory was accepted"),
            Err(error) => error,
        };

        assert!(
            error.to_string().contains("stale git cache staging"),
            "{error:#}"
        );
    }

    #[test]
    fn materialized_source_is_read_back_with_exact_bytes() {
        let temporary_directory = tempfile::tempdir().unwrap();

        write_source(
            temporary_directory.path(),
            "sources/0000/before.source",
            FIRST_SOURCE,
        )
        .unwrap();

        assert_eq!(
            fs::read(
                temporary_directory
                    .path()
                    .join("sources/0000/before.source")
            )
            .unwrap(),
            FIRST_SOURCE
        );
    }

    #[test]
    fn failed_final_manifest_install_preserves_the_durable_checkpoint() {
        let temporary_directory = tempfile::tempdir().unwrap();
        let manifest_path = temporary_directory.path().join(MANIFEST_NAME);
        let checkpoint_path = temporary_directory.path().join(MANIFEST_PART_NAME);
        fs::write(&manifest_path, b"existing manifest\n").unwrap();
        fs::write(&checkpoint_path, b"durable checkpoint\n").unwrap();

        let error = write_manifest(&manifest_path, b"replacement manifest\n").unwrap_err();

        assert!(error.to_string().contains("manifest appeared concurrently"));
        assert_eq!(fs::read(checkpoint_path).unwrap(), b"durable checkpoint\n");
    }

    fn loose_object_path(repository: &Path, oid: &str) -> PathBuf {
        repository.join("objects").join(&oid[..2]).join(&oid[2..])
    }

    fn commit_all(repository: &Path, message: &str) {
        let mut command = isolated_git_command();
        command
            .arg("-C")
            .arg(repository)
            .args(["add", "--", "Demo.java"]);
        checked(&mut command);
        let mut command = isolated_git_command();
        command
            .arg("-C")
            .arg(repository)
            .args(["commit", "--no-gpg-sign", "-m", message]);
        checked(&mut command);
    }

    fn revision(repository: &Path) -> String {
        let mut command = isolated_git_command();
        command
            .arg("-C")
            .arg(repository)
            .args(["rev-parse", "HEAD"]);
        let output = checked(&mut command);
        trim_line(&output).unwrap().to_owned()
    }

    fn checked(command: &mut Command) -> Vec<u8> {
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
}
