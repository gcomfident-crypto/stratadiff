use std::{
    collections::HashSet,
    env,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::Args;
use serde::Deserialize;
use stratadiff::{
    github::{
        MAX_GITHUB_COMMIT_OBJECT_BYTES, MAX_GITHUB_REVIEWS_BYTES,
        resolve_github_review_checkpoint_slurp_pages, verify_github_commit_object,
    },
    review::review_git_range_with_checkpoint,
};
use tempfile::{Builder as TempDirBuilder, TempDir};

use crate::{
    process::{
        CapturedOutput, SignalState, run_bounded_process, run_bounded_process_recording_pid,
        run_inherited_process, run_short_critical_process,
    },
    viewer,
};

const COMMAND_STDERR_LIMIT: usize = 64 * 1024;
const SMALL_STDOUT_LIMIT: usize = 64 * 1024;
const FETCH_PACK_CAPTURE_LIMIT: usize = 4 * 1024;
const FETCH_PACK_PROTOCOL_LIMIT: usize = 256;
const LOCAL_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const REMOTE_FETCH_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const VIEWER_SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

const VIEWER_ENVIRONMENT_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "XAUTHORITY",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_ADDRESS",
    "LC_COLLATE",
    "LC_CTYPE",
    "LC_IDENTIFICATION",
    "LC_MEASUREMENT",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NAME",
    "LC_NUMERIC",
    "LC_PAPER",
    "LC_TELEPHONE",
    "LC_TIME",
];

const GIT_ENVIRONMENT_DENYLIST: &[&str] = &[
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
    "STRATADIFF_GITHUB_TOKEN",
    "github_token",
    "git_authorization",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "FTP_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "ftp_proxy",
    "no_proxy",
    "CURL_CA_BUNDLE",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

#[derive(Debug, Args)]
pub(crate) struct ResumeArgs {
    /// Pull request number, URL, or branch accepted by `gh pr view`.
    pub(crate) pull_request: String,
    /// Exact reviewer login; defaults to the authenticated `gh` user.
    #[arg(long)]
    pub(crate) reviewer: Option<String>,
    /// GitHub repository in [HOST/]OWNER/REPO form.
    #[arg(short = 'R', long = "repo", value_name = "REPO")]
    pub(crate) repository: Option<String>,
    /// Existing local Git worktree or bare repository.
    #[arg(long, value_name = "PATH")]
    pub(crate) repo_dir: Option<PathBuf>,
    /// Loopback workbench port; zero asks the operating system to choose one.
    #[arg(long, default_value_t = 0)]
    pub(crate) port: u16,
    /// Print the workbench URL without opening a browser.
    #[arg(long)]
    pub(crate) no_open: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ResumeWorkbenchArgs {
    pub(crate) base: String,
    pub(crate) head: String,
    #[arg(long)]
    pub(crate) checkpoint: String,
    #[arg(long)]
    pub(crate) repo: PathBuf,
    #[arg(long, default_value_t = 0)]
    pub(crate) port: u16,
    #[arg(long)]
    pub(crate) no_open: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryRecord {
    name_with_owner: String,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PullRequestCoordinates {
    number: u64,
    base_ref_oid: String,
    head_ref_oid: String,
    url: String,
}

#[derive(Debug)]
struct RepositoryIdentity {
    full_name: String,
    url: String,
    host: String,
}

impl RepositoryIdentity {
    fn selector(&self) -> String {
        format!("{}/{}", self.host, self.full_name)
    }

    fn git_url(&self) -> String {
        format!("{}.git", self.url)
    }
}

struct ResumeSession<'a> {
    signals: &'a SignalState,
    scratch: Option<TempDir>,
    session_name: String,
    repository: Option<PathBuf>,
    provider_repository: Option<PathBuf>,
    provider_home: Option<PathBuf>,
    authorization: Option<String>,
    temporary_refs: Vec<(String, String)>,
    temporary_pack_keep_files: Vec<(PathBuf, Vec<u8>)>,
    cleaned: bool,
}

impl<'a> ResumeSession<'a> {
    fn new(signals: &'a SignalState) -> Result<Self> {
        let scratch = TempDirBuilder::new()
            .prefix("gh-stratadiff-resume-")
            .tempdir()
            .context("failed to create the resume scratch directory")?;
        let session_name = scratch
            .path()
            .file_name()
            .and_then(OsStr::to_str)
            .context("resume scratch directory has no portable name")?
            .to_owned();
        Ok(Self {
            signals,
            scratch: Some(scratch),
            session_name,
            repository: None,
            provider_repository: None,
            provider_home: None,
            authorization: None,
            temporary_refs: Vec::new(),
            temporary_pack_keep_files: Vec::new(),
            cleaned: false,
        })
    }

    fn scratch_path(&self) -> &Path {
        self.scratch
            .as_ref()
            .expect("resume scratch remains present until cleanup")
            .path()
    }

    fn repository(&self) -> &Path {
        self.repository
            .as_deref()
            .expect("resume repository is selected before object resolution")
    }

    fn select_repository(&mut self, args: &ResumeArgs) -> Result<()> {
        let repository = match (&args.repository, &args.repo_dir) {
            (None, None) => canonicalize_git_repository(
                Path::new("."),
                self.signals,
                "current directory is not inside a Git repository; use -R HOST/OWNER/REPO to run without a checkout",
            )?,
            (None, Some(repo_dir)) | (Some(_), Some(repo_dir)) => canonicalize_git_repository(
                repo_dir,
                self.signals,
                "--repo-dir is not a Git worktree or bare repository",
            )?,
            (Some(_), None) => {
                let repository = self.scratch_path().join("repository.git");
                let mut command = clean_git_command();
                command.args(["init", "--bare", "--quiet"]).arg(&repository);
                let output = run_bounded_process(
                    &mut command,
                    SMALL_STDOUT_LIMIT,
                    COMMAND_STDERR_LIMIT,
                    LOCAL_COMMAND_TIMEOUT,
                    "git init --bare",
                    Some(self.signals),
                )?;
                ensure_process_success(&output, "git init --bare")?;
                fs::canonicalize(&repository).with_context(|| {
                    format!(
                        "failed to resolve temporary bare repository {}",
                        repository.display()
                    )
                })?
            }
        };
        self.repository = Some(repository);
        Ok(())
    }

    fn ensure_local_commit(
        &mut self,
        identity: &RepositoryIdentity,
        object_id: &str,
        label: &str,
        ref_name: &str,
        provider_verified: bool,
    ) -> Result<()> {
        if self.local_commit_exists(object_id)? {
            return self.verify_local_commit(object_id, label);
        }
        if !provider_verified {
            verify_provider_commit(identity, object_id, label, self.signals)?;
        }
        self.materialize_provider_commit(identity, object_id, label, ref_name)
    }

    fn local_commit_exists(&self, object_id: &str) -> Result<bool> {
        let mut command = clean_git_command();
        command
            .arg("-C")
            .arg(self.repository())
            .args(["cat-file", "-e"])
            .arg(format!("{object_id}^{{commit}}"));
        let output = run_bounded_process(
            &mut command,
            SMALL_STDOUT_LIMIT,
            COMMAND_STDERR_LIMIT,
            LOCAL_COMMAND_TIMEOUT,
            "git cat-file",
            Some(self.signals),
        )?;
        Ok(output.status.success())
    }

    fn verify_local_commit(&self, object_id: &str, label: &str) -> Result<()> {
        let mut command = clean_git_command();
        command
            .arg("-C")
            .arg(self.repository())
            .args(["rev-parse", "--verify"])
            .arg(format!("{object_id}^{{commit}}"));
        let output = run_bounded_process(
            &mut command,
            SMALL_STDOUT_LIMIT,
            COMMAND_STDERR_LIMIT,
            LOCAL_COMMAND_TIMEOUT,
            "git rev-parse",
            Some(self.signals),
        )?;
        ensure_process_success(&output, "git rev-parse")?;
        let resolved = required_single_line(&output.stdout, "resolved Git commit")?;
        ensure!(
            resolved == object_id,
            "{label} resolved to {resolved}, expected {object_id}"
        );
        Ok(())
    }

    fn initialize_provider_repository(&mut self) -> Result<()> {
        if self.provider_repository.is_some() {
            return Ok(());
        }
        let provider_repository = self.scratch_path().join("provider.git");
        let provider_home = self.scratch_path().join("provider-home");
        create_private_directory(&provider_home)?;
        let mut command = clean_git_command();
        command
            .args(["init", "--bare", "--quiet"])
            .arg(&provider_repository)
            .env("HOME", &provider_home)
            .env("XDG_CONFIG_HOME", &provider_home);
        let output = run_bounded_process(
            &mut command,
            SMALL_STDOUT_LIMIT,
            COMMAND_STDERR_LIMIT,
            LOCAL_COMMAND_TIMEOUT,
            "git init --bare provider repository",
            Some(self.signals),
        )?;
        ensure_process_success(&output, "git init --bare provider repository")?;
        self.provider_repository = Some(provider_repository);
        self.provider_home = Some(provider_home);
        Ok(())
    }

    fn load_fetch_authorization(&mut self, host: &str) -> Result<()> {
        if self.authorization.is_some() {
            return Ok(());
        }
        let mut command = gh_command();
        command.args(["auth", "token", "--hostname", host]);
        let output = run_bounded_process(
            &mut command,
            SMALL_STDOUT_LIMIT,
            COMMAND_STDERR_LIMIT,
            LOCAL_COMMAND_TIMEOUT,
            "gh auth token",
            Some(self.signals),
        )?;
        ensure!(
            output.status.success(),
            "could not read a GitHub token for {host}: {}",
            stderr_summary(&output.stderr)
        );
        let token = required_single_line(&output.stdout, "authenticated GitHub token")?;
        ensure!(
            !token.contains(['\n', '\r']),
            "the authenticated GitHub token contains a line break"
        );
        let authorization = STANDARD.encode(format!("x-access-token:{token}"));
        ensure!(
            !authorization.is_empty(),
            "could not encode the GitHub fetch credential"
        );
        self.authorization = Some(authorization);
        Ok(())
    }

    fn materialize_provider_commit(
        &mut self,
        identity: &RepositoryIdentity,
        object_id: &str,
        label: &str,
        ref_name: &str,
    ) -> Result<()> {
        self.initialize_provider_repository()?;
        self.load_fetch_authorization(&identity.host)?;
        let provider_repository = self
            .provider_repository
            .as_ref()
            .expect("provider repository was initialized")
            .clone();
        let provider_home = self
            .provider_home
            .as_ref()
            .expect("provider home was initialized")
            .clone();
        let provider_ref = format!("refs/stratadiff/provider/{ref_name}-{object_id}");
        let authorization = self
            .authorization
            .as_ref()
            .expect("fetch authorization was loaded")
            .clone();

        let mut fetch = clean_git_command();
        fetch
            .arg(format!("--git-dir={}", provider_repository.display()))
            .args(["fetch", "--quiet", "--no-tags", "--no-recurse-submodules"])
            .arg(identity.git_url())
            .arg(format!("{object_id}:{provider_ref}"))
            .env("HOME", &provider_home)
            .env("XDG_CONFIG_HOME", &provider_home)
            .env("GIT_CONFIG_COUNT", "10")
            .env("GIT_CONFIG_KEY_0", "http.extraHeader")
            .env("GIT_CONFIG_VALUE_0", "")
            .env("GIT_CONFIG_KEY_1", "http.extraHeader")
            .env(
                "GIT_CONFIG_VALUE_1",
                format!("AUTHORIZATION: basic {authorization}"),
            )
            .env("GIT_CONFIG_KEY_2", "http.followRedirects")
            .env("GIT_CONFIG_VALUE_2", "false")
            .env("GIT_CONFIG_KEY_3", "http.sslVerify")
            .env("GIT_CONFIG_VALUE_3", "true")
            .env("GIT_CONFIG_KEY_4", "credential.helper")
            .env("GIT_CONFIG_VALUE_4", "")
            .env("GIT_CONFIG_KEY_5", "protocol.allow")
            .env("GIT_CONFIG_VALUE_5", "never")
            .env("GIT_CONFIG_KEY_6", "protocol.https.allow")
            .env("GIT_CONFIG_VALUE_6", "always")
            .env("GIT_CONFIG_KEY_7", "protocol.file.allow")
            .env("GIT_CONFIG_VALUE_7", "never")
            .env("GIT_CONFIG_KEY_8", "http.proxy")
            .env("GIT_CONFIG_VALUE_8", "")
            .env("GIT_CONFIG_KEY_9", "fetch.fsckObjects")
            .env("GIT_CONFIG_VALUE_9", "true")
            .env("GIT_TRACE", "0")
            .env("GIT_TRACE_CURL", "0")
            .env("GIT_TRACE_PACKET", "0")
            .env("GIT_TRACE_REDACT", "1");
        let fetched = run_bounded_process(
            &mut fetch,
            SMALL_STDOUT_LIMIT,
            COMMAND_STDERR_LIMIT,
            REMOTE_FETCH_TIMEOUT,
            "git fetch exact provider commit",
            Some(self.signals),
        )?;
        if !fetched.status.success() {
            self.authorization = None;
            bail!(
                "GitHub verified {label} {object_id}, but no longer serves that exact commit over Git HTTPS"
            );
        }

        let mut resolve_provider = clean_git_command();
        resolve_provider
            .arg(format!("--git-dir={}", provider_repository.display()))
            .args(["rev-parse", "--verify"])
            .arg(format!("{provider_ref}^{{commit}}"))
            .env("HOME", &provider_home)
            .env("XDG_CONFIG_HOME", &provider_home);
        let resolved = run_bounded_process(
            &mut resolve_provider,
            SMALL_STDOUT_LIMIT,
            COMMAND_STDERR_LIMIT,
            LOCAL_COMMAND_TIMEOUT,
            "git rev-parse provider commit",
            Some(self.signals),
        )?;
        ensure_process_success(&resolved, "git rev-parse provider commit")?;
        let resolved = required_single_line(&resolved.stdout, "provider Git commit")?;
        ensure!(
            resolved == object_id,
            "provider fetch resolved {label} to {resolved}, expected {object_id}"
        );

        let imported_ref = format!("refs/stratadiff/resume/{}/{ref_name}", self.session_name);
        self.ensure_temporary_ref_absent(&imported_ref)?;
        self.verify_local_source_url(&provider_repository, &provider_home)?;
        let pack_directory = self.resolve_pack_directory()?;
        let keep_files_before = list_pack_keep_files(&pack_directory)?;

        let mut import = clean_git_command();
        import
            .arg("-C")
            .arg(self.repository())
            .args(["fetch-pack", "--no-progress"])
            .arg(&provider_repository)
            .arg(&provider_ref)
            .env("HOME", &provider_home)
            .env("XDG_CONFIG_HOME", &provider_home)
            .env("GIT_CONFIG_COUNT", "7")
            .env("GIT_CONFIG_KEY_0", "http.extraHeader")
            .env("GIT_CONFIG_VALUE_0", "")
            .env("GIT_CONFIG_KEY_1", "credential.helper")
            .env("GIT_CONFIG_VALUE_1", "")
            .env("GIT_CONFIG_KEY_2", "protocol.allow")
            .env("GIT_CONFIG_VALUE_2", "never")
            .env("GIT_CONFIG_KEY_3", "protocol.file.allow")
            .env("GIT_CONFIG_VALUE_3", "always")
            .env("GIT_CONFIG_KEY_4", "protocol.https.allow")
            .env("GIT_CONFIG_VALUE_4", "never")
            .env("GIT_CONFIG_KEY_5", "fetch.fsckObjects")
            .env("GIT_CONFIG_VALUE_5", "true")
            .env("GIT_CONFIG_KEY_6", "transfer.fsckObjects")
            .env("GIT_CONFIG_VALUE_6", "true");
        let mut fetch_pack_pid = None;
        let imported = run_bounded_process_recording_pid(
            &mut import,
            FETCH_PACK_CAPTURE_LIMIT,
            COMMAND_STDERR_LIMIT,
            REMOTE_FETCH_TIMEOUT,
            "git fetch-pack",
            Some(self.signals),
            &mut fetch_pack_pid,
        );
        let keep_registration = self.register_fetch_pack_keeps(
            &pack_directory,
            &keep_files_before,
            fetch_pack_pid,
            imported
                .as_ref()
                .ok()
                .map(|output| output.stdout.as_slice()),
        );
        let imported = match (imported, keep_registration) {
            (Ok(imported), Ok(())) => imported,
            (Err(error), Ok(())) => return Err(error),
            (Ok(_), Err(error)) => return Err(error),
            (Err(error), Err(keep_error)) => {
                return Err(anyhow!(
                    "git fetch-pack failed: {error:#}; keep-file registration also failed: {keep_error:#}"
                ));
            }
        };
        ensure!(
            imported.status.success(),
            "failed to import verified {label} {object_id} into the local Git object store: {}",
            stderr_summary(&imported.stderr)
        );
        self.validate_fetch_pack_output(&imported.stdout, object_id, &provider_ref, label)?;

        self.create_temporary_ref(&imported_ref, object_id)?;
        self.release_pack_keeps()?;

        let mut resolve_import = clean_git_command();
        resolve_import
            .arg("-C")
            .arg(self.repository())
            .args(["rev-parse", "--verify"])
            .arg(format!("{imported_ref}^{{commit}}"));
        let resolved = run_bounded_process(
            &mut resolve_import,
            SMALL_STDOUT_LIMIT,
            COMMAND_STDERR_LIMIT,
            LOCAL_COMMAND_TIMEOUT,
            "git rev-parse imported commit",
            Some(self.signals),
        )?;
        ensure_process_success(&resolved, "git rev-parse imported commit")?;
        let resolved = required_single_line(&resolved.stdout, "imported Git commit")?;
        ensure!(
            resolved == object_id,
            "imported {label} resolved to {resolved}, expected {object_id}"
        );
        self.verify_local_commit(object_id, label)
    }

    fn create_temporary_ref(&mut self, imported_ref: &str, object_id: &str) -> Result<()> {
        self.signals.check()?;
        let mut update_ref = clean_git_command();
        update_ref
            .arg("-C")
            .arg(self.repository())
            .args(["update-ref", "--no-deref"])
            .arg(imported_ref)
            .arg(object_id)
            .arg("");
        let mut spawned_pid = None;
        let update = run_short_critical_process(
            &mut update_ref,
            LOCAL_COMMAND_TIMEOUT,
            "git update-ref",
            &mut spawned_pid,
        );
        if spawned_pid.is_some() {
            self.temporary_refs
                .push((imported_ref.to_owned(), object_id.to_owned()));
        }
        let status = update?;
        if !status.success() {
            self.temporary_refs.pop();
            bail!(
                "temporary StrataDiff ref already exists or changed concurrently: {imported_ref}"
            );
        }
        self.signals.check()
    }

    fn ensure_temporary_ref_absent(&self, imported_ref: &str) -> Result<()> {
        let mut command = clean_git_command();
        command.arg("-C").arg(self.repository()).args([
            "show-ref",
            "--verify",
            "--quiet",
            imported_ref,
        ]);
        let output = run_bounded_process(
            &mut command,
            SMALL_STDOUT_LIMIT,
            COMMAND_STDERR_LIMIT,
            LOCAL_COMMAND_TIMEOUT,
            "git show-ref",
            Some(self.signals),
        )?;
        match output.status.code() {
            Some(1) => Ok(()),
            Some(0) => bail!("temporary StrataDiff ref already exists: {imported_ref}"),
            _ => bail!(
                "failed to inspect temporary StrataDiff ref {imported_ref}: {}",
                stderr_summary(&output.stderr)
            ),
        }
    }

    fn verify_local_source_url(
        &self,
        provider_repository: &Path,
        provider_home: &Path,
    ) -> Result<()> {
        let mut command = clean_git_command();
        command
            .arg("-C")
            .arg(self.repository())
            .args(["ls-remote", "--get-url"])
            .arg(provider_repository)
            .env("HOME", provider_home)
            .env("XDG_CONFIG_HOME", provider_home);
        let output = run_bounded_process(
            &mut command,
            SMALL_STDOUT_LIMIT,
            COMMAND_STDERR_LIMIT,
            LOCAL_COMMAND_TIMEOUT,
            "git ls-remote --get-url",
            Some(self.signals),
        )?;
        ensure_process_success(&output, "git ls-remote --get-url")?;
        let effective = required_single_line(&output.stdout, "isolated commit source")?;
        ensure!(
            Path::new(&effective) == provider_repository,
            "repository Git configuration rewrites the isolated commit source"
        );
        Ok(())
    }

    fn resolve_pack_directory(&self) -> Result<PathBuf> {
        let mut command = clean_git_command();
        command
            .arg("-C")
            .arg(self.repository())
            .args(["rev-parse", "--git-path", "objects/pack"]);
        let output = run_bounded_process(
            &mut command,
            SMALL_STDOUT_LIMIT,
            COMMAND_STDERR_LIMIT,
            LOCAL_COMMAND_TIMEOUT,
            "git rev-parse --git-path objects/pack",
            Some(self.signals),
        )?;
        ensure_process_success(&output, "git rev-parse --git-path objects/pack")?;
        let raw_pack_directory = PathBuf::from(required_single_line(
            &output.stdout,
            "Git object pack directory",
        )?);
        let pack_directory = if raw_pack_directory.is_absolute() {
            raw_pack_directory
        } else {
            self.repository().join(raw_pack_directory)
        };
        fs::canonicalize(&pack_directory).with_context(|| {
            format!(
                "failed to resolve Git object pack directory {}",
                pack_directory.display()
            )
        })
    }

    fn register_fetch_pack_keeps(
        &mut self,
        pack_directory: &Path,
        keep_files_before: &HashSet<OsString>,
        fetch_pack_pid: Option<u32>,
        output: Option<&[u8]>,
    ) -> Result<()> {
        let Some(fetch_pack_pid) = fetch_pack_pid else {
            return Ok(());
        };
        let expected_owner = format!("fetch-pack {fetch_pack_pid} on ");
        if let Some(output) = output {
            self.register_first_keep_record(
                pack_directory,
                keep_files_before,
                expected_owner.as_bytes(),
                output,
            )?;
        }
        for file_name in list_pack_keep_files(pack_directory)? {
            if keep_files_before.contains(&file_name) {
                continue;
            }
            let keep_path = pack_directory.join(&file_name);
            if pack_keep_has_owner(&keep_path, expected_owner.as_bytes()).with_context(|| {
                format!("failed to inspect pack keep file {}", keep_path.display())
            })? {
                self.register_pack_keep(keep_path, expected_owner.as_bytes());
            }
        }
        Ok(())
    }

    fn register_first_keep_record(
        &mut self,
        pack_directory: &Path,
        keep_files_before: &HashSet<OsString>,
        expected_owner: &[u8],
        output: &[u8],
    ) -> Result<()> {
        let first = output
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        let Some(keep) = first.strip_prefix(b"keep\t") else {
            return Ok(());
        };
        let keep = std::str::from_utf8(keep).context("fetch-pack keep object is not UTF-8")?;
        if is_object_id(keep) {
            let file_name = OsString::from(format!("pack-{keep}.keep"));
            if !keep_files_before.contains(&file_name) {
                let keep_path = pack_directory.join(file_name);
                if pack_keep_has_owner(&keep_path, expected_owner).with_context(|| {
                    format!("failed to inspect pack keep file {}", keep_path.display())
                })? {
                    self.register_pack_keep(keep_path, expected_owner);
                }
            }
        }
        Ok(())
    }

    fn register_pack_keep(&mut self, keep_path: PathBuf, expected_owner: &[u8]) {
        if !self
            .temporary_pack_keep_files
            .iter()
            .any(|(registered, _)| registered == &keep_path)
        {
            self.temporary_pack_keep_files
                .push((keep_path, expected_owner.to_vec()));
        }
    }

    fn release_pack_keeps(&mut self) -> Result<()> {
        let mut failures = Vec::new();
        for (keep_path, expected_owner) in self.temporary_pack_keep_files.drain(..).rev() {
            match pack_keep_has_owner(&keep_path, &expected_owner) {
                Ok(false) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    failures.push(format!(
                        "failed to inspect pack keep file {}: {error}",
                        keep_path.display()
                    ));
                    continue;
                }
                Ok(true) => {}
            }
            match fs::remove_file(&keep_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!(
                    "failed to remove temporary pack keep file {}: {error}",
                    keep_path.display()
                )),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            bail!(failures.join("; "))
        }
    }

    fn validate_fetch_pack_output(
        &mut self,
        output: &[u8],
        object_id: &str,
        provider_ref: &str,
        label: &str,
    ) -> Result<()> {
        ensure!(
            output.len() <= FETCH_PACK_PROTOCOL_LIMIT,
            "local import returned oversized output"
        );
        let body = output.strip_suffix(b"\n").with_context(|| {
            format!("local import did not return the exact verified {label} ref")
        })?;
        let records = body.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        let expected_ref = format!("{object_id} {provider_ref}");
        let valid = match records.as_slice() {
            [record] => *record == expected_ref.as_bytes(),
            [keep, record] => {
                let Some(keep_object) = keep.strip_prefix(b"keep\t") else {
                    return Err(anyhow!(
                        "local import did not return the exact verified {label} ref"
                    ));
                };
                let keep_object = std::str::from_utf8(keep_object).with_context(|| {
                    format!("local import did not return the exact verified {label} ref")
                })?;
                is_object_id(keep_object) && *record == expected_ref.as_bytes()
            }
            _ => false,
        };
        ensure!(
            valid,
            "local import did not return the exact verified {label} ref"
        );
        Ok(())
    }

    fn clear_authorization(&mut self) {
        self.authorization = None;
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.cleaned {
            return Ok(());
        }
        self.cleaned = true;
        self.clear_authorization();
        let mut failures = Vec::new();

        if let Some(repository) = self.repository.as_ref() {
            for (reference, object_id) in self.temporary_refs.drain(..).rev() {
                let mut inspect = clean_git_command();
                inspect
                    .arg("-C")
                    .arg(repository)
                    .args(["rev-parse", "--verify", "--quiet", "--end-of-options"])
                    .arg(&reference);
                match run_bounded_process(
                    &mut inspect,
                    SMALL_STDOUT_LIMIT,
                    COMMAND_STDERR_LIMIT,
                    LOCAL_COMMAND_TIMEOUT,
                    "git rev-parse cleanup",
                    None,
                ) {
                    Ok(output) if output.status.code() == Some(1) => continue,
                    Ok(output) if output.status.success() => {
                        match required_single_line(&output.stdout, "temporary ref commit") {
                            Ok(current) if current == object_id => {}
                            Ok(current) => {
                                failures.push(format!(
                                    "refusing to remove changed temporary ref {reference}: expected {object_id}, found {current}"
                                ));
                                continue;
                            }
                            Err(error) => {
                                failures.push(format!(
                                    "failed to inspect temporary ref {reference}: {error:#}"
                                ));
                                continue;
                            }
                        }
                    }
                    Ok(output) => {
                        failures.push(format!(
                            "failed to inspect temporary ref {reference}: {}",
                            stderr_summary(&output.stderr)
                        ));
                        continue;
                    }
                    Err(error) => {
                        failures.push(format!(
                            "failed to inspect temporary ref {reference}: {error:#}"
                        ));
                        continue;
                    }
                }
                let mut command = clean_git_command();
                command
                    .arg("-C")
                    .arg(repository)
                    .args(["update-ref", "--no-deref", "-d"])
                    .arg(&reference)
                    .arg(&object_id);
                match run_bounded_process(
                    &mut command,
                    SMALL_STDOUT_LIMIT,
                    COMMAND_STDERR_LIMIT,
                    LOCAL_COMMAND_TIMEOUT,
                    "git update-ref cleanup",
                    None,
                ) {
                    Ok(output) if output.status.success() => {}
                    Ok(output) => failures.push(format!(
                        "failed to remove temporary ref {reference}: {}",
                        stderr_summary(&output.stderr)
                    )),
                    Err(error) => failures.push(format!(
                        "failed to remove temporary ref {reference}: {error:#}"
                    )),
                }
            }
        }

        if let Err(error) = self.release_pack_keeps() {
            failures.push(format!("keep-file cleanup failed: {error:#}"));
        }

        if let Some(scratch) = self.scratch.take()
            && let Err(error) = scratch.close()
        {
            failures.push(format!(
                "failed to remove resume scratch directory: {error}"
            ));
        }

        if failures.is_empty() {
            Ok(())
        } else {
            bail!("resume cleanup failed: {}", failures.join("; "))
        }
    }
}

impl Drop for ResumeSession<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("Error: {error:#}");
        }
    }
}

pub(crate) fn run(args: ResumeArgs) -> Result<()> {
    let signals = SignalState::register()?;
    let mut session = ResumeSession::new(&signals)?;
    let operation = run_with_session(&args, &mut session);
    session.clear_authorization();
    let cleanup = session.cleanup();
    match (operation, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(error), Err(cleanup_error)) => Err(anyhow!(
            "resume failed: {error:#}; cleanup also failed: {cleanup_error:#}"
        )),
    }
}

fn run_with_session(args: &ResumeArgs, session: &mut ResumeSession<'_>) -> Result<()> {
    session.select_repository(args)?;
    let identity = resolve_repository_identity(
        session.repository(),
        args.repository.as_deref(),
        session.signals,
    )?;
    let reviewer = match &args.reviewer {
        Some(reviewer) => reviewer.clone(),
        None => resolve_authenticated_reviewer(&identity.host, session.signals)?,
    };
    validate_reviewer(&reviewer)?;

    let first = read_pull_request_coordinates(
        &args.pull_request,
        &identity.selector(),
        session.repository(),
        session.signals,
    )?;
    validate_pull_request_coordinates(&first, &identity.url)?;

    session.ensure_local_commit(
        &identity,
        &first.base_ref_oid,
        "pull request base",
        "base",
        false,
    )?;
    session.ensure_local_commit(
        &identity,
        &first.head_ref_oid,
        "pull request head",
        "head",
        false,
    )?;

    let checkpoint = resolve_review_checkpoint(
        &identity,
        first.number,
        &reviewer,
        session.repository(),
        session.signals,
    )?;
    verify_provider_commit(&identity, &checkpoint, "review checkpoint", session.signals)?;
    session.ensure_local_commit(
        &identity,
        &checkpoint,
        "review checkpoint",
        "checkpoint",
        true,
    )?;
    session.clear_authorization();

    let latest = read_pull_request_coordinates(
        &first.number.to_string(),
        &identity.selector(),
        session.repository(),
        session.signals,
    )?;
    validate_pull_request_coordinates(&latest, &identity.url)?;
    ensure!(
        latest.number == first.number
            && latest.base_ref_oid == first.base_ref_oid
            && latest.head_ref_oid == first.head_ref_oid,
        "pull request base or head changed while review coverage was being resolved; rerun the command"
    );

    eprintln!(
        "Resuming @{reviewer} review of {}#{} at exact checkpoint {checkpoint}.",
        identity.full_name, first.number
    );
    run_viewer_child(
        &first.base_ref_oid,
        &first.head_ref_oid,
        &checkpoint,
        session.repository(),
        args.port,
        args.no_open,
        session.signals,
    )
}

pub(crate) fn run_workbench(args: ResumeWorkbenchArgs) -> Result<()> {
    let review = review_git_range_with_checkpoint(
        &args.repo,
        &args.base,
        &args.head,
        Some(&args.checkpoint),
    )?;
    viewer::serve_review(review, args.repo, args.port, !args.no_open)
}

fn resolve_repository_identity(
    repository: &Path,
    requested: Option<&str>,
    signals: &SignalState,
) -> Result<RepositoryIdentity> {
    let mut command = gh_command();
    command.args(["repo", "view"]);
    if let Some(requested) = requested {
        command.arg(requested);
    }
    command
        .args(["--json", "nameWithOwner,url"])
        .current_dir(repository);
    let output = run_bounded_process(
        &mut command,
        SMALL_STDOUT_LIMIT,
        COMMAND_STDERR_LIMIT,
        LOCAL_COMMAND_TIMEOUT,
        "gh repo view",
        Some(signals),
    )?;
    ensure_process_success(&output, "gh repo view")?;
    let record: RepositoryRecord = serde_json::from_slice(&output.stdout)
        .context("failed to decode GitHub repository metadata")?;
    validate_owner_repository(&record.name_with_owner)?;
    let prefix = "https://";
    let remainder = record
        .url
        .strip_prefix(prefix)
        .context("GitHub repository URL is not a canonical HTTPS repository URL")?;
    let (host, path) = remainder
        .split_once('/')
        .context("GitHub repository URL is not a canonical HTTPS repository URL")?;
    validate_host(host)?;
    ensure!(
        path == record.name_with_owner,
        "GitHub repository URL does not match {}",
        record.name_with_owner
    );
    ensure!(
        record.url == format!("https://{host}/{}", record.name_with_owner),
        "GitHub repository URL is not a canonical HTTPS repository URL"
    );
    if let Some(requested_host) = requested.and_then(requested_repository_host) {
        ensure!(
            requested_host == host,
            "GitHub repository host {host} does not match requested host {requested_host}"
        );
    }
    let host = host.to_owned();
    Ok(RepositoryIdentity {
        full_name: record.name_with_owner,
        url: record.url,
        host,
    })
}

fn resolve_authenticated_reviewer(host: &str, signals: &SignalState) -> Result<String> {
    let mut command = gh_command();
    command.args(["api", "--hostname", host, "user", "--jq", ".login"]);
    let output = run_bounded_process(
        &mut command,
        SMALL_STDOUT_LIMIT,
        COMMAND_STDERR_LIMIT,
        LOCAL_COMMAND_TIMEOUT,
        "gh api authenticated user",
        Some(signals),
    )?;
    ensure_process_success(&output, "gh api authenticated user")?;
    required_single_line(&output.stdout, "authenticated GitHub reviewer")
}

fn read_pull_request_coordinates(
    pull_request: &str,
    selector: &str,
    repository: &Path,
    signals: &SignalState,
) -> Result<PullRequestCoordinates> {
    let mut command = gh_command();
    command
        .args(["pr", "view", pull_request, "--repo", selector, "--json"])
        .arg("number,baseRefOid,headRefOid,url")
        .current_dir(repository);
    let output = run_bounded_process(
        &mut command,
        SMALL_STDOUT_LIMIT,
        COMMAND_STDERR_LIMIT,
        LOCAL_COMMAND_TIMEOUT,
        "gh pr view",
        Some(signals),
    )?;
    ensure_process_success(&output, "gh pr view")?;
    serde_json::from_slice(&output.stdout).context("failed to decode pull request metadata")
}

fn resolve_review_checkpoint(
    identity: &RepositoryIdentity,
    pull_request: u64,
    reviewer: &str,
    repository: &Path,
    signals: &SignalState,
) -> Result<String> {
    let endpoint = format!(
        "repos/{}/pulls/{pull_request}/reviews?per_page=100",
        identity.full_name
    );
    let mut command = gh_command();
    command
        .args([
            "api",
            "--paginate",
            "--slurp",
            "--hostname",
            &identity.host,
            &endpoint,
        ])
        .current_dir(repository);
    let output = run_bounded_process(
        &mut command,
        MAX_GITHUB_REVIEWS_BYTES,
        COMMAND_STDERR_LIMIT,
        REMOTE_FETCH_TIMEOUT,
        "gh api pull request reviews",
        Some(signals),
    )?;
    ensure!(
        output.status.success(),
        "could not load reviews for {}#{pull_request}: {}",
        identity.full_name,
        stderr_summary(&output.stderr)
    );
    let resolution = resolve_github_review_checkpoint_slurp_pages(&output.stdout, reviewer)?;
    let checkpoint = resolution.checkpoint.with_context(|| {
        format!(
            "@{reviewer} has no eligible completed review on {}#{pull_request}",
            identity.full_name
        )
    })?;
    Ok(checkpoint.commit_id)
}

fn verify_provider_commit(
    identity: &RepositoryIdentity,
    object_id: &str,
    label: &str,
    signals: &SignalState,
) -> Result<()> {
    let endpoint = format!("repos/{}/git/commits/{object_id}", identity.full_name);
    let mut command = gh_command();
    command.args(["api", "--hostname", &identity.host, &endpoint]);
    let output = run_bounded_process(
        &mut command,
        MAX_GITHUB_COMMIT_OBJECT_BYTES,
        COMMAND_STDERR_LIMIT,
        LOCAL_COMMAND_TIMEOUT,
        "gh api Git commit",
        Some(signals),
    )?;
    ensure!(
        output.status.success(),
        "GitHub no longer exposes exact {label} {object_id}; the provider cannot materialize that commit"
    );
    verify_github_commit_object(&output.stdout, object_id)
        .with_context(|| format!("provider verification failed for {label} {object_id}"))
}

fn run_viewer_child(
    base: &str,
    head: &str,
    checkpoint: &str,
    repository: &Path,
    port: u16,
    no_open: bool,
    signals: &SignalState,
) -> Result<()> {
    let executable = env::current_exe().context("failed to resolve the StrataDiff executable")?;
    let mut command = Command::new(executable);
    command
        .arg("__resume-workbench")
        .arg(base)
        .arg(head)
        .args(["--checkpoint", checkpoint, "--repo"])
        .arg(repository)
        .args(["--port", &port.to_string()])
        .current_dir(repository)
        .env_clear();
    for name in VIEWER_ENVIRONMENT_ALLOWLIST {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/bin/false")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1");
    if no_open {
        command.arg("--no-open");
    }
    let status = run_inherited_process(
        &mut command,
        signals,
        VIEWER_SHUTDOWN_GRACE,
        "StrataDiff review workbench",
    )?;
    ensure!(
        status.success(),
        "StrataDiff review workbench exited with {status}"
    );
    Ok(())
}

fn canonicalize_git_repository(
    requested: &Path,
    signals: &SignalState,
    failure_message: &str,
) -> Result<PathBuf> {
    let mut inspect = clean_git_command();
    inspect
        .arg("-C")
        .arg(requested)
        .args(["rev-parse", "--is-bare-repository"]);
    let inspected = run_bounded_process(
        &mut inspect,
        SMALL_STDOUT_LIMIT,
        COMMAND_STDERR_LIMIT,
        LOCAL_COMMAND_TIMEOUT,
        "git rev-parse repository kind",
        Some(signals),
    )?;
    ensure!(inspected.status.success(), "{failure_message}");
    let bare = required_single_line(&inspected.stdout, "Git repository kind")?;
    ensure!(bare == "true" || bare == "false", "{failure_message}");

    let mut resolve = clean_git_command();
    resolve.arg("-C").arg(requested).arg("rev-parse");
    if bare == "true" {
        resolve.arg("--absolute-git-dir");
    } else {
        resolve.arg("--show-toplevel");
    }
    let resolved = run_bounded_process(
        &mut resolve,
        SMALL_STDOUT_LIMIT,
        COMMAND_STDERR_LIMIT,
        LOCAL_COMMAND_TIMEOUT,
        "git rev-parse repository path",
        Some(signals),
    )?;
    ensure!(resolved.status.success(), "{failure_message}");
    let path = PathBuf::from(required_single_line(
        &resolved.stdout,
        "Git repository path",
    )?);
    fs::canonicalize(&path).with_context(|| failure_message.to_owned())
}

fn gh_command() -> Command {
    let mut command = Command::new("gh");
    remove_inherited_git_environment(&mut command);
    command
        .env_remove("GH_REPO")
        .env_remove("GH_DEBUG")
        .env_remove("DEBUG")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("FORCE_COLOR")
        .env_remove("GH_FORCE_TTY")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_PAGER", "cat")
        .env("NO_COLOR", "1");
    command
}

fn clean_git_command() -> Command {
    let mut command = Command::new("git");
    remove_matching_environment(&mut command, should_remove_from_git_environment);
    command
        .arg("-c")
        .arg(format!("core.hooksPath={}", null_device()))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/bin/false")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1");
    command
}

fn remove_inherited_git_environment(command: &mut Command) {
    remove_matching_environment(command, is_git_environment_name);
}

fn remove_matching_environment(command: &mut Command, remove: fn(&OsStr) -> bool) {
    let inherited_names = env::vars_os().map(|(name, _)| name).collect::<Vec<_>>();
    for name in inherited_names {
        if remove(&name) {
            command.env_remove(name);
        }
    }
}

fn should_remove_from_git_environment(name: &OsStr) -> bool {
    is_git_environment_name(name)
        || GIT_ENVIRONMENT_DENYLIST
            .iter()
            .any(|denied| name == OsStr::new(denied))
}

fn is_git_environment_name(name: &OsStr) -> bool {
    name.as_encoded_bytes()
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"GIT_"))
}

#[cfg(windows)]
fn null_device() -> &'static str {
    "NUL"
}

#[cfg(not(windows))]
fn null_device() -> &'static str {
    "/dev/null"
}

fn ensure_process_success(output: &CapturedOutput, label: &str) -> Result<()> {
    ensure!(
        output.status.success(),
        "{label} failed with {}: {}",
        output.status,
        stderr_summary(&output.stderr)
    );
    Ok(())
}

fn stderr_summary(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr).trim_end().to_owned()
}

fn required_single_line(bytes: &[u8], label: &str) -> Result<String> {
    let value = std::str::from_utf8(bytes).with_context(|| format!("{label} is not UTF-8"))?;
    let value = value
        .trim_end_matches('\n')
        .strip_suffix('\r')
        .unwrap_or(value.trim_end_matches('\n'));
    ensure!(!value.is_empty(), "{label} is empty");
    ensure!(
        !value.contains(['\n', '\r']),
        "{label} contains more than one line"
    );
    Ok(value.to_owned())
}

fn validate_pull_request_coordinates(
    coordinates: &PullRequestCoordinates,
    repository_url: &str,
) -> Result<()> {
    ensure!(
        coordinates.number > 0,
        "pull request metadata is incomplete"
    );
    ensure!(
        is_sha1(&coordinates.base_ref_oid),
        "pull request base is not a full lowercase Git object ID"
    );
    ensure!(
        is_sha1(&coordinates.head_ref_oid),
        "pull request head is not a full lowercase Git object ID"
    );
    ensure!(
        coordinates.url == format!("{repository_url}/pull/{}", coordinates.number),
        "pull request URL does not match the selected repository"
    );
    Ok(())
}

fn validate_reviewer(reviewer: &str) -> Result<()> {
    ensure!(
        !reviewer.is_empty() && !reviewer.chars().any(char::is_whitespace),
        "reviewer login is empty or contains whitespace"
    );
    Ok(())
}

fn validate_owner_repository(repository: &str) -> Result<()> {
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    ensure!(
        !owner.is_empty()
            && !name.is_empty()
            && parts.next().is_none()
            && owner.chars().all(valid_repository_character)
            && name.chars().all(valid_repository_character),
        "GitHub repository is not a valid OWNER/REPO name"
    );
    Ok(())
}

fn valid_repository_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
}

fn validate_host(host: &str) -> Result<()> {
    ensure!(
        host.as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')),
        "GitHub repository URL has an unsupported host"
    );
    Ok(())
}

fn requested_repository_host(repository: &str) -> Option<&str> {
    let mut parts = repository.split('/');
    let host = parts.next()?;
    parts.next()?;
    parts.next()?;
    parts.next().is_none().then_some(host)
}

fn is_sha1(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn list_pack_keep_files(pack_directory: &Path) -> Result<HashSet<OsString>> {
    let mut keep_files = HashSet::new();
    for entry in fs::read_dir(pack_directory).with_context(|| {
        format!(
            "failed to inspect Git object pack directory {}",
            pack_directory.display()
        )
    })? {
        let entry = entry.with_context(|| {
            format!(
                "failed to inspect Git object pack directory {}",
                pack_directory.display()
            )
        })?;
        if !entry
            .file_type()
            .with_context(|| format!("failed to inspect pack entry {}", entry.path().display()))?
            .is_file()
        {
            continue;
        }
        let file_name = entry.file_name();
        let Some(file_name_text) = file_name.to_str() else {
            continue;
        };
        let Some(object_id) = file_name_text
            .strip_prefix("pack-")
            .and_then(|name| name.strip_suffix(".keep"))
        else {
            continue;
        };
        if is_object_id(object_id) {
            keep_files.insert(file_name);
        }
    }
    Ok(keep_files)
}

fn pack_keep_has_owner(path: &Path, expected_owner: &[u8]) -> std::io::Result<bool> {
    let mut prefix = Vec::new();
    File::open(path)?.take(256).read_to_end(&mut prefix)?;
    Ok(prefix.starts_with(expected_owner))
}

fn create_private_directory(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .with_context(|| format!("failed to create private directory {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, fs, process::Command};

    use super::{
        PullRequestCoordinates, RepositoryRecord, ResumeSession, SignalState,
        is_git_environment_name, is_object_id, is_sha1, list_pack_keep_files, pack_keep_has_owner,
        requested_repository_host, should_remove_from_git_environment, validate_owner_repository,
        validate_pull_request_coordinates, validate_reviewer,
    };

    fn repository_with_commit() -> (tempfile::TempDir, String) {
        let repository = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(repository.path())
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(repository.path())
                .args([
                    "-c",
                    "user.name=StrataDiff Test",
                    "-c",
                    "user.email=stratadiff@example.invalid",
                    "commit",
                    "--quiet",
                    "--allow-empty",
                    "-m",
                    "fixture",
                ])
                .status()
                .unwrap()
                .success()
        );
        let commit = Command::new("git")
            .arg("-C")
            .arg(repository.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert!(commit.status.success());
        let commit = String::from_utf8(commit.stdout).unwrap().trim().to_owned();
        (repository, commit)
    }

    #[test]
    fn strict_identifiers_reject_ambiguous_input() {
        assert!(validate_owner_repository("owner/repo").is_ok());
        for invalid in ["", "owner", "owner/repo/extra", "owner/re po", "/repo"] {
            assert!(validate_owner_repository(invalid).is_err(), "{invalid}");
        }
        assert!(validate_reviewer("octocat").is_ok());
        assert!(validate_reviewer("octo cat").is_err());
        assert!(validate_reviewer("").is_err());
        assert!(is_sha1(&"a".repeat(40)));
        assert!(!is_sha1(&"a".repeat(64)));
        assert!(is_object_id(&"a".repeat(40)));
        assert!(is_object_id(&"a".repeat(64)));
        assert!(!is_object_id(&"A".repeat(40)));
    }

    #[test]
    fn requested_host_is_present_only_in_three_component_form() {
        assert_eq!(
            requested_repository_host("ghe.example/owner/repo"),
            Some("ghe.example")
        );
        assert_eq!(requested_repository_host("owner/repo"), None);
        assert_eq!(requested_repository_host("a/b/c/d"), None);
    }

    #[test]
    fn pull_request_coordinates_require_number_and_sha1_objects() {
        let valid = PullRequestCoordinates {
            number: 17,
            base_ref_oid: "a".repeat(40),
            head_ref_oid: "b".repeat(40),
            url: "https://github.com/owner/repo/pull/17".to_owned(),
        };
        assert!(validate_pull_request_coordinates(&valid, "https://github.com/owner/repo").is_ok());
        let invalid = PullRequestCoordinates { number: 0, ..valid };
        assert!(
            validate_pull_request_coordinates(&invalid, "https://github.com/owner/repo").is_err()
        );
    }

    #[test]
    fn pull_request_coordinates_reject_a_cross_repository_url() {
        let coordinates = PullRequestCoordinates {
            number: 17,
            base_ref_oid: "a".repeat(40),
            head_ref_oid: "b".repeat(40),
            url: "https://github.com/other/repository/pull/17".to_owned(),
        };

        assert!(
            validate_pull_request_coordinates(&coordinates, "https://github.com/owner/repository")
                .is_err()
        );
    }

    #[test]
    fn repository_record_rejects_unknown_provider_fields() {
        let encoded =
            br#"{"nameWithOwner":"owner/repo","url":"https://github.com/owner/repo","extra":true}"#;
        assert!(serde_json::from_slice::<RepositoryRecord>(encoded).is_err());
    }

    #[test]
    fn git_environment_filter_removes_all_github_token_names_and_git_overrides() {
        for name in [
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "GH_ENTERPRISE_TOKEN",
            "GITHUB_ENTERPRISE_TOKEN",
            "STRATADIFF_GITHUB_TOKEN",
            "GIT_DIR",
            "git_config_count",
        ] {
            assert!(
                should_remove_from_git_environment(OsStr::new(name)),
                "{name}"
            );
        }
        assert!(is_git_environment_name(OsStr::new("GIT_CONFIG_GLOBAL")));
        assert!(is_git_environment_name(OsStr::new("git_config_global")));
        assert!(!is_git_environment_name(OsStr::new("GH_TOKEN")));
        assert!(!should_remove_from_git_environment(OsStr::new("GH_HOST")));
    }

    #[test]
    fn fetch_pack_keep_discovery_is_bounded_to_valid_owned_files() {
        let directory = tempfile::tempdir().unwrap();
        let pack = directory.path();
        let existing_name = format!("pack-{}.keep", "a".repeat(40));
        let owned_name = format!("pack-{}.keep", "b".repeat(40));
        let unrelated_name = format!("pack-{}.keep", "c".repeat(40));
        fs::write(pack.join(&existing_name), b"fetch-pack 41 on host\n").unwrap();
        let before = list_pack_keep_files(pack).unwrap();
        fs::write(pack.join(&owned_name), b"fetch-pack 42 on host\n").unwrap();
        fs::write(pack.join(&unrelated_name), b"fetch-pack 43 on host\n").unwrap();
        fs::write(
            pack.join("pack-not-an-object.keep"),
            b"fetch-pack 42 on host\n",
        )
        .unwrap();

        let after = list_pack_keep_files(pack).unwrap();
        let new_files = after.difference(&before).collect::<Vec<_>>();
        assert_eq!(new_files.len(), 2);
        assert!(pack_keep_has_owner(&pack.join(&owned_name), b"fetch-pack 42 on ").unwrap());
        assert!(!pack_keep_has_owner(&pack.join(&unrelated_name), b"fetch-pack 42 on ").unwrap());
    }

    #[test]
    fn fetch_pack_stdout_does_not_claim_a_pre_existing_keep_file() {
        let directory = tempfile::tempdir().unwrap();
        let pack = directory.path();
        let object_id = "a".repeat(40);
        let keep_path = pack.join(format!("pack-{object_id}.keep"));
        fs::write(&keep_path, b"fetch-pack 42 on host\n").unwrap();
        let before = list_pack_keep_files(pack).unwrap();
        let output = format!("keep\t{object_id}\n");
        let signals = SignalState::register().unwrap();
        let mut session = ResumeSession {
            signals: &signals,
            scratch: None,
            session_name: "test-session".to_owned(),
            repository: None,
            provider_repository: None,
            provider_home: None,
            authorization: None,
            temporary_refs: Vec::new(),
            temporary_pack_keep_files: Vec::new(),
            cleaned: false,
        };

        session
            .register_fetch_pack_keeps(pack, &before, Some(42), Some(output.as_bytes()))
            .unwrap();
        session.cleanup().unwrap();

        assert!(keep_path.exists());
    }

    #[test]
    fn fetch_pack_stdout_does_not_claim_a_concurrent_process_keep_file() {
        let directory = tempfile::tempdir().unwrap();
        let pack = directory.path();
        let object_id = "a".repeat(40);
        let keep_path = pack.join(format!("pack-{object_id}.keep"));
        let before = list_pack_keep_files(pack).unwrap();
        fs::write(&keep_path, b"fetch-pack 43 on host\n").unwrap();
        let output = format!("keep\t{object_id}\n");
        let signals = SignalState::register().unwrap();
        let mut session = ResumeSession {
            signals: &signals,
            scratch: None,
            session_name: "test-session".to_owned(),
            repository: None,
            provider_repository: None,
            provider_home: None,
            authorization: None,
            temporary_refs: Vec::new(),
            temporary_pack_keep_files: Vec::new(),
            cleaned: false,
        };

        session
            .register_fetch_pack_keeps(pack, &before, Some(42), Some(output.as_bytes()))
            .unwrap();
        session.cleanup().unwrap();

        assert!(keep_path.exists());
    }

    #[test]
    fn cleanup_preserves_a_keep_file_reowned_after_registration() {
        let directory = tempfile::tempdir().unwrap();
        let pack = directory.path();
        let object_id = "a".repeat(40);
        let keep_path = pack.join(format!("pack-{object_id}.keep"));
        let owned_keep_path = pack.join(format!("pack-{}.keep", "b".repeat(40)));
        let before = list_pack_keep_files(pack).unwrap();
        fs::write(&keep_path, b"fetch-pack 42 on host\n").unwrap();
        fs::write(&owned_keep_path, b"fetch-pack 42 on host\n").unwrap();
        let output = format!("keep\t{object_id}\n");
        let signals = SignalState::register().unwrap();
        let mut session = ResumeSession {
            signals: &signals,
            scratch: None,
            session_name: "test-session".to_owned(),
            repository: None,
            provider_repository: None,
            provider_home: None,
            authorization: None,
            temporary_refs: Vec::new(),
            temporary_pack_keep_files: Vec::new(),
            cleaned: false,
        };
        session
            .register_fetch_pack_keeps(pack, &before, Some(42), Some(output.as_bytes()))
            .unwrap();
        fs::write(&keep_path, b"fetch-pack 43 on host\n").unwrap();

        session.cleanup().unwrap();

        assert!(keep_path.exists());
        assert!(!owned_keep_path.exists());
    }

    #[test]
    fn cleanup_accepts_a_pre_registered_ref_that_was_never_created() {
        let (repository, commit) = repository_with_commit();
        let signals = SignalState::register().unwrap();
        let mut session = ResumeSession {
            signals: &signals,
            scratch: None,
            session_name: "test-session".to_owned(),
            repository: Some(repository.path().to_path_buf()),
            provider_repository: None,
            provider_home: None,
            authorization: None,
            temporary_refs: vec![(
                "refs/stratadiff/resume/test-session/checkpoint".to_owned(),
                commit,
            )],
            temporary_pack_keep_files: Vec::new(),
            cleaned: false,
        };

        session.cleanup().unwrap();
    }

    #[test]
    fn failed_temporary_ref_cas_does_not_claim_a_competing_ref() {
        let (repository, commit) = repository_with_commit();
        let reference = "refs/stratadiff/resume/test-session/checkpoint";
        let signals = SignalState::register().unwrap();
        let mut session = ResumeSession {
            signals: &signals,
            scratch: None,
            session_name: "test-session".to_owned(),
            repository: Some(repository.path().to_path_buf()),
            provider_repository: None,
            provider_home: None,
            authorization: None,
            temporary_refs: Vec::new(),
            temporary_pack_keep_files: Vec::new(),
            cleaned: false,
        };
        session.ensure_temporary_ref_absent(reference).unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(repository.path())
                .args(["update-ref", reference, &commit])
                .status()
                .unwrap()
                .success()
        );

        assert!(session.create_temporary_ref(reference, &commit).is_err());
        assert!(session.temporary_refs.is_empty());
        session.cleanup().unwrap();

        let resolved = Command::new("git")
            .arg("-C")
            .arg(repository.path())
            .args(["rev-parse", "--verify", reference])
            .output()
            .unwrap();
        assert!(resolved.status.success());
        assert_eq!(String::from_utf8(resolved.stdout).unwrap().trim(), commit);
    }
}
