#![cfg(unix)]

use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use stratadiff::inbox_event::{InboxEventBinding, InboxEventEnvelope, InboxEventTrigger};

const FETCH_HEAD_SENTINEL: &[u8] = b"caller-owned fetch state\n";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug)]
enum RepositoryMode {
    CurrentWorktree,
    CurrentWorktreeBranch,
    RepoDir,
    BareRepoDir,
    PullRequestUrl,
    PullRequestUrlInCurrentWorktree,
    PullRequestUrlAndRepository,
    PullRequestUrlAndRepoDir,
    RepositoryOnly,
    RepositoryAndRepoDir,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PausePoint {
    None,
    FetchPack,
    UpdateRef,
}

impl PausePoint {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::FetchPack => "fetch-pack",
            Self::UpdateRef => "update-ref",
        }
    }
}

struct Fixture {
    _temporary: tempfile::TempDir,
    local: PathBuf,
    bare_local: PathBuf,
    isolated: PathBuf,
    poisoned: PathBuf,
    outside: PathBuf,
    temporary_root: PathBuf,
    home: PathBuf,
    bin: PathBuf,
    log: PathBuf,
    pause_marker: PathBuf,
    base: String,
    checkpoint: String,
    head: String,
    host: String,
}

impl Fixture {
    fn new(host: &str, drift: bool) -> Self {
        Self::configured(host, drift, PausePoint::None, false)
    }

    fn with_pause(host: &str, drift: bool, pause_point: PausePoint) -> Self {
        Self::configured(host, drift, pause_point, false)
    }

    fn with_cross_repository_pr() -> Self {
        Self::configured("github.com", false, PausePoint::None, true)
    }

    fn configured(
        host: &str,
        drift: bool,
        pause_point: PausePoint,
        cross_repository_pr: bool,
    ) -> Self {
        let temporary = test_tempdir();
        let root = fs::canonicalize(temporary.path()).unwrap();
        let source = root.join("source");
        let provider = root.join("provider.git");
        let isolated = root.join("isolated.git");
        let poisoned = root.join("poisoned");
        let local = root.join("local");
        let bare_local = root.join("local-bare.git");
        let outside = root.join("outside");
        let temporary_root = root.join("tmp");
        let home = root.join("home");
        let bin = root.join("bin");
        let state = root.join("state");
        let log = root.join("calls.txt");
        let pause_marker = root.join("pause-ready.txt");
        for directory in [
            &source,
            &local,
            &poisoned,
            &outside,
            &temporary_root,
            &home,
            &bin,
            &state,
        ] {
            fs::create_dir(directory).unwrap();
        }
        fs::write(&log, b"").unwrap();

        let real_git = executable_path("git");
        git_at(&real_git, &source, &["init", "--quiet"]);
        git_at(
            &real_git,
            &source,
            &["config", "user.name", "StrataDiff Resume Test"],
        );
        git_at(
            &real_git,
            &source,
            &["config", "user.email", "resume@stratadiff.test"],
        );
        fs::write(source.join("app.rs"), b"fn value() -> i32 { 0 }\n").unwrap();
        let base = commit(&real_git, &source, "base");

        git_at(
            &real_git,
            &source,
            &["checkout", "--quiet", "-b", "reviewed", &base],
        );
        fs::write(source.join("app.rs"), b"fn value() -> i32 { 1 }\n").unwrap();
        let checkpoint = commit(&real_git, &source, "review checkpoint");

        git_at(
            &real_git,
            &source,
            &["checkout", "--quiet", "-b", "pull-request", &base],
        );
        fs::write(source.join("app.rs"), b"fn value() -> i32 { 2 }\n").unwrap();
        let head = commit(&real_git, &source, "current pull request head");

        command_success(
            Command::new(&real_git)
                .args(["clone", "--bare", "--quiet"])
                .arg(&source)
                .arg(&provider),
            "clone provider repository",
        );
        for (label, object_id) in [
            ("base", base.as_str()),
            ("checkpoint", checkpoint.as_str()),
            ("head", head.as_str()),
        ] {
            git_at(
                &real_git,
                &provider,
                &[
                    "update-ref",
                    &format!("refs/stratadiff/provider/{label}-{object_id}"),
                    object_id,
                ],
            );
        }
        command_success(
            Command::new(&real_git)
                .args(["init", "--bare", "--quiet"])
                .arg(&isolated),
            "initialize isolated bare repository",
        );
        git_at(&real_git, &poisoned, &["init", "--quiet"]);

        command_success(
            Command::new(&real_git)
                .args(["init", "--bare", "--quiet"])
                .arg(&bare_local),
            "initialize existing bare repository",
        );
        let provider_url = format!("file://{}", provider.display());
        git_at(
            &real_git,
            &bare_local,
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                &provider_url,
                "refs/heads/pull-request:refs/heads/pull-request",
            ],
        );
        let bare_checkpoint_probe = Command::new(&real_git)
            .arg("-C")
            .arg(&bare_local)
            .args(["cat-file", "-e", &format!("{checkpoint}^{{commit}}")])
            .output()
            .unwrap();
        assert!(
            !bare_checkpoint_probe.status.success(),
            "the bare fixture must omit the reviewed sibling commit"
        );
        fs::write(bare_local.join("FETCH_HEAD"), FETCH_HEAD_SENTINEL).unwrap();
        assert!(
            pack_keep_files(&bare_local.join("objects/pack")).is_empty(),
            "the existing bare fixture must not begin with a pack keep"
        );

        git_at(&real_git, &local, &["init", "--quiet"]);
        git_at(
            &real_git,
            &local,
            &[
                "fetch",
                "--quiet",
                "--no-tags",
                &provider_url,
                "refs/heads/pull-request",
            ],
        );
        git_at(
            &real_git,
            &local,
            &["checkout", "--quiet", "--detach", "FETCH_HEAD"],
        );
        let checkpoint_probe = Command::new(&real_git)
            .arg("-C")
            .arg(&local)
            .args(["cat-file", "-e", &format!("{checkpoint}^{{commit}}")])
            .output()
            .unwrap();
        assert!(
            !checkpoint_probe.status.success(),
            "the local fixture must omit the reviewed sibling commit"
        );
        fs::write(local.join(".git/FETCH_HEAD"), FETCH_HEAD_SENTINEL).unwrap();

        let canonical_provider_url = format!("https://{host}/acme/widget.git");
        write_executable(
            &bin.join("git"),
            &git_proxy_script(
                &real_git,
                &provider,
                &isolated,
                &canonical_provider_url,
                &log,
                pause_point,
                &pause_marker,
            ),
        );
        write_executable(
            &bin.join("gh"),
            &gh_stub_script(
                host,
                &base,
                &checkpoint,
                &head,
                &state,
                &log,
                drift,
                cross_repository_pr,
            ),
        );

        Self {
            _temporary: temporary,
            local,
            bare_local,
            isolated,
            poisoned,
            outside,
            temporary_root,
            home,
            bin,
            log,
            pause_marker,
            base,
            checkpoint,
            head,
            host: host.to_owned(),
        }
    }

    fn resume_command(&self, mode: RepositoryMode, default_reviewer: bool) -> Command {
        let pull_request = match mode {
            RepositoryMode::CurrentWorktreeBranch => "pull-request".to_owned(),
            RepositoryMode::PullRequestUrl
            | RepositoryMode::PullRequestUrlInCurrentWorktree
            | RepositoryMode::PullRequestUrlAndRepository
            | RepositoryMode::PullRequestUrlAndRepoDir => {
                format!("https://{}/acme/widget/pull/17", self.host)
            }
            _ => "17".to_owned(),
        };
        self.resume_command_with_pull_request(mode, default_reviewer, &pull_request)
    }

    fn resume_command_with_pull_request(
        &self,
        mode: RepositoryMode,
        default_reviewer: bool,
        pull_request: &str,
    ) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_stratadiff"));
        command.args(["resume", pull_request]);
        match mode {
            RepositoryMode::CurrentWorktree
            | RepositoryMode::CurrentWorktreeBranch
            | RepositoryMode::PullRequestUrlInCurrentWorktree => {
                command.current_dir(&self.local);
            }
            RepositoryMode::RepoDir => {
                command
                    .current_dir(&self.outside)
                    .arg("--repo-dir")
                    .arg(&self.local);
            }
            RepositoryMode::BareRepoDir => {
                command
                    .current_dir(&self.outside)
                    .arg("--repo-dir")
                    .arg(&self.bare_local);
            }
            RepositoryMode::PullRequestUrl => {
                command.current_dir(&self.outside);
            }
            RepositoryMode::PullRequestUrlAndRepository | RepositoryMode::RepositoryOnly => {
                command
                    .current_dir(&self.outside)
                    .args(["-R", &format!("{}/acme/widget", self.host)]);
            }
            RepositoryMode::PullRequestUrlAndRepoDir => {
                command
                    .current_dir(&self.outside)
                    .arg("--repo-dir")
                    .arg(&self.local);
            }
            RepositoryMode::RepositoryAndRepoDir => {
                command
                    .current_dir(&self.outside)
                    .args(["-R", &format!("{}/acme/widget", self.host)])
                    .arg("--repo-dir")
                    .arg(&self.local);
            }
        }
        if !default_reviewer {
            command.args(["--reviewer", "alice"]);
        }
        let inherited_path = std::env::var_os("PATH").unwrap();
        let mut path = vec![self.bin.clone()];
        path.extend(std::env::split_paths(&inherited_path));
        command
            .args(["--port", "0", "--no-open"])
            .env_clear()
            .env("PATH", std::env::join_paths(path).unwrap())
            .env("HOME", &self.home)
            .env("TMPDIR", &self.temporary_root)
            .env("LANG", "C.UTF-8")
            .env("GH_TOKEN", "must-not-reach-git-or-workbench")
            .env("GITHUB_TOKEN", "must-not-reach-git-or-workbench")
            .env("GH_ENTERPRISE_TOKEN", "must-not-reach-git-or-workbench")
            .env("GITHUB_ENTERPRISE_TOKEN", "must-not-reach-git-or-workbench")
            .env("STRATADIFF_GITHUB_TOKEN", "must-not-reach-git-or-workbench")
            .env("github_token", "must-not-reach-git-or-workbench")
            .env("git_authorization", "must-not-reach-git-or-workbench")
            .env("CALLER_SECRET", "must-not-reach-workbench")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if matches!(
            mode,
            RepositoryMode::CurrentWorktree
                | RepositoryMode::CurrentWorktreeBranch
                | RepositoryMode::PullRequestUrlInCurrentWorktree
                | RepositoryMode::RepoDir
                | RepositoryMode::PullRequestUrlAndRepoDir
        ) {
            command.env("GIT_DIR", self.poisoned.join(".git"));
        }
        command
    }

    fn assert_fetch_head_unchanged(&self) {
        assert_eq!(
            fs::read(self.local.join(".git/FETCH_HEAD")).unwrap(),
            FETCH_HEAD_SENTINEL
        );
        assert_eq!(
            fs::read(self.bare_local.join("FETCH_HEAD")).unwrap(),
            FETCH_HEAD_SENTINEL
        );
    }

    fn assert_no_resume_refs(&self) {
        let real_git = executable_path("git");
        for repository in [&self.local, &self.bare_local, &self.isolated] {
            let output = Command::new(&real_git)
                .arg("-C")
                .arg(repository)
                .args([
                    "for-each-ref",
                    "--format=%(refname)",
                    "refs/stratadiff/resume",
                ])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert!(
                output.stdout.is_empty(),
                "temporary resume refs remain in {}: {}",
                repository.display(),
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }

    fn assert_no_pack_keep_files(&self) {
        for pack_directory in [
            self.local.join(".git/objects/pack"),
            self.bare_local.join("objects/pack"),
        ] {
            let keep_files = pack_keep_files(&pack_directory);
            assert!(
                keep_files.is_empty(),
                "temporary pack keeps remain: {keep_files:?}"
            );
        }
    }

    fn assert_no_scratch_directories(&self) {
        let leftovers = fs::read_dir(&self.temporary_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("gh-stratadiff-resume-")
            })
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "resume scratch remains: {leftovers:?}"
        );
    }

    fn calls(&self) -> String {
        fs::read_to_string(&self.log).unwrap()
    }

    fn inbox_event_binding(&self) -> InboxEventBinding {
        InboxEventBinding {
            provider_host: self.host.clone(),
            repository: "acme/widget".to_owned(),
            repository_node_id: "R_widget".to_owned(),
            pull_request_number: 17,
            pull_request_node_id: "PR_17".to_owned(),
            reviewer_login: "alice".to_owned(),
            reviewer_node_id: "U_alice".to_owned(),
            review_database_id: 101,
            review_state: "approved".to_owned(),
            review_node_id: "PRR_101".to_owned(),
            checkpoint_oid: self.checkpoint.clone(),
            checkpoint_base_oid: None,
            current_base_oid: Some(self.base.clone()),
            head_oid: self.head.clone(),
            review_request_active: false,
            triggers: vec![InboxEventTrigger::HeadChanged],
        }
    }

    fn inbox_event(&self) -> InboxEventEnvelope {
        InboxEventEnvelope::new(self.inbox_event_binding()).unwrap()
    }

    fn inbox_event_token(&self) -> String {
        self.inbox_event().to_token().unwrap()
    }
}

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn child_mut(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }

    fn take(&mut self) -> Child {
        self.0.take().unwrap()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct CapturedChild {
    child: ChildGuard,
    captured: Arc<Mutex<String>>,
    line_receiver: mpsc::Receiver<String>,
    done_receiver: mpsc::Receiver<()>,
}

impl CapturedChild {
    fn spawn(mut command: Command) -> Self {
        let mut child = ChildGuard(Some(command.spawn().unwrap()));
        let stderr = child.child_mut().stderr.take().unwrap();
        let captured = Arc::new(Mutex::new(String::new()));
        let reader_capture = Arc::clone(&captured);
        let (line_sender, line_receiver) = mpsc::channel();
        let (done_sender, done_receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        reader_capture.lock().unwrap().push_str(&line);
                        let _ = line_sender.send(line);
                    }
                    Err(error) => {
                        reader_capture
                            .lock()
                            .unwrap()
                            .push_str(&format!("stderr read failed: {error}\n"));
                        break;
                    }
                }
            }
            let _ = done_sender.send(());
        });
        Self {
            child,
            captured,
            line_receiver,
            done_receiver,
        }
    }

    fn stderr(&self) -> String {
        self.captured.lock().unwrap().clone()
    }

    fn signal_and_wait(mut self, signal: i32) -> (ExitStatus, String) {
        let pid = i32::try_from(self.child.child_mut().id()).unwrap();
        assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
        let mut owned_child = self.child.take();
        let status = wait_for_exit(&mut owned_child, PROCESS_TIMEOUT);
        self.done_receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("resume descendants retained the inherited stderr pipe");
        (status, self.stderr())
    }
}

#[test]
fn resume_scrubs_poisoned_environment_across_repository_modes_and_sigterm_cleanup() {
    let cases = [
        (RepositoryMode::CurrentWorktree, "github.com", false),
        (RepositoryMode::CurrentWorktreeBranch, "github.com", false),
        (RepositoryMode::RepoDir, "github.com", false),
        (RepositoryMode::BareRepoDir, "github.com", false),
        (RepositoryMode::PullRequestUrl, "github.com", true),
        (
            RepositoryMode::PullRequestUrlInCurrentWorktree,
            "ghe-current.example",
            true,
        ),
        (
            RepositoryMode::PullRequestUrlAndRepository,
            "ghe-explicit.example",
            true,
        ),
        (
            RepositoryMode::PullRequestUrlAndRepoDir,
            "ghe-local.example",
            true,
        ),
        (RepositoryMode::RepositoryOnly, "github.com", false),
        (RepositoryMode::RepositoryAndRepoDir, "ghe.example", true),
    ];

    for (mode, host, default_reviewer) in cases {
        let fixture = Fixture::new(host, false);
        let result = run_until_ready_then_signal(
            fixture.resume_command(mode, default_reviewer),
            libc::SIGTERM,
        );
        assert_eq!(result.status.code(), Some(128 + libc::SIGTERM), "{mode:?}");
        assert!(
            result
                .stderr
                .contains(&format!("exact checkpoint {}", fixture.checkpoint)),
            "{mode:?} stderr:\n{}",
            result.stderr
        );
        assert!(
            result.session.contains(r#""kind":"repository_review""#),
            "{mode:?} session:\n{}",
            result.session
        );
        assert!(
            TcpStream::connect_timeout(&result.address, Duration::from_millis(250)).is_err(),
            "{mode:?} workbench still accepts connections after SIGTERM"
        );
        fixture.assert_fetch_head_unchanged();
        fixture.assert_no_resume_refs();
        fixture.assert_no_pack_keep_files();
        fixture.assert_no_scratch_directories();

        let calls = fixture.calls();
        assert_eq!(
            calls.matches(" gh pr view ").count(),
            2,
            "{mode:?}:\n{calls}"
        );
        assert!(
            calls
                .lines()
                .filter(|line| line.contains(" gh pr view "))
                .all(|line| !line.contains("://")),
            "a pull request URL reached gh in {mode:?}:\n{calls}"
        );
        assert!(
            calls
                .lines()
                .filter(|line| line.starts_with("git "))
                .all(|line| {
                    line.contains(" secrets=clean")
                        && line.contains(" enterprise_token=absent")
                        && (!line.contains(" phase=workbench") || line.contains(" auth=none"))
                }),
            "secret reached git or the isolated workbench in {mode:?}:\n{calls}"
        );
        assert!(calls.contains(" phase=workbench"), "{mode:?}:\n{calls}");
        assert!(
            calls
                .lines()
                .filter(|line| line.starts_with("cwd=") && line.contains(" gh "))
                .all(|line| line.contains(" git_dir=clean")),
            "poisoned GIT_DIR reached gh in {mode:?}:\n{calls}"
        );
        assert!(
            calls.contains(" rev-parse --git-path objects/pack"),
            "fetch-pack did not exercise pack-keep cleanup in {mode:?}:\n{calls}"
        );

        let repo_call = calls
            .lines()
            .find(|line| line.contains(" gh repo view "))
            .unwrap();
        match mode {
            RepositoryMode::CurrentWorktree
            | RepositoryMode::CurrentWorktreeBranch
            | RepositoryMode::PullRequestUrlInCurrentWorktree
            | RepositoryMode::RepoDir
            | RepositoryMode::PullRequestUrlAndRepoDir => {
                assert!(repo_call.starts_with(&format!("cwd={} ", fixture.local.display())));
                assert!(!repo_call.contains("github.com/acme/widget"));
                assert!(
                    !calls.lines().any(|line| {
                        line.contains(" init --bare --quiet ") && line.contains("/repository.git")
                    }),
                    "{mode:?}:\n{calls}"
                );
                if matches!(mode, RepositoryMode::CurrentWorktreeBranch) {
                    assert!(
                        calls.contains(" gh pr view pull-request --repo github.com/acme/widget "),
                        "{mode:?}:\n{calls}"
                    );
                }
                if matches!(
                    mode,
                    RepositoryMode::PullRequestUrlInCurrentWorktree
                        | RepositoryMode::PullRequestUrlAndRepoDir
                ) {
                    assert!(
                        calls.contains(&format!(
                            " gh pr view 17 --repo {}/acme/widget ",
                            fixture.host
                        )),
                        "{mode:?}:\n{calls}"
                    );
                }
            }
            RepositoryMode::BareRepoDir => {
                assert!(repo_call.starts_with(&format!("cwd={} ", fixture.bare_local.display())));
                assert!(!repo_call.contains("github.com/acme/widget"));
            }
            RepositoryMode::PullRequestUrl
            | RepositoryMode::PullRequestUrlAndRepository
            | RepositoryMode::RepositoryOnly => {
                assert!(repo_call.contains(&format!("{}/acme/widget", fixture.host)));
                assert!(repo_call.contains("/gh-stratadiff-resume-"));
                assert!(repo_call.contains("/repository.git gh repo view"));
                assert!(calls.contains(" init --bare --quiet "));
                if matches!(
                    mode,
                    RepositoryMode::PullRequestUrl | RepositoryMode::PullRequestUrlAndRepository
                ) {
                    assert!(
                        calls.contains(&format!(
                            " gh pr view 17 --repo {}/acme/widget ",
                            fixture.host
                        )),
                        "{mode:?}:\n{calls}"
                    );
                    assert!(calls.contains(&format!(
                        "gh api --hostname {} user --jq .login",
                        fixture.host
                    )));
                    assert!(result.stderr.contains("Resuming @authenticated-reviewer"));
                }
            }
            RepositoryMode::RepositoryAndRepoDir => {
                assert!(repo_call.starts_with(&format!("cwd={} ", fixture.local.display())));
                assert!(repo_call.contains("ghe.example/acme/widget"));
                assert!(calls.contains("gh api --hostname ghe.example user --jq .login"));
                assert!(calls.contains("--repo ghe.example/acme/widget"));
                assert!(calls.contains("https://ghe.example/acme/widget.git"));
                assert!(result.stderr.contains("Resuming @authenticated-reviewer"));
            }
        }
    }
}

#[test]
fn resume_forwards_sigint_and_sighup_and_cleans_up() {
    for signal in [libc::SIGINT, libc::SIGHUP] {
        let fixture = Fixture::new("github.com", false);
        let result = run_until_ready_then_signal(
            fixture.resume_command(RepositoryMode::RepoDir, false),
            signal,
        );

        assert_eq!(result.status.code(), Some(128 + signal), "signal {signal}");
        assert!(
            result.session.contains(r#""kind":"repository_review""#),
            "signal {signal} session:\n{}",
            result.session
        );
        assert!(
            TcpStream::connect_timeout(&result.address, Duration::from_millis(250)).is_err(),
            "workbench still accepts connections after signal {signal}"
        );
        fixture.assert_fetch_head_unchanged();
        fixture.assert_no_resume_refs();
        fixture.assert_no_pack_keep_files();
        fixture.assert_no_scratch_directories();
    }
}

#[test]
fn signal_after_update_ref_side_effect_removes_the_pre_registered_ref() {
    let fixture = Fixture::with_pause("github.com", false, PausePoint::UpdateRef);
    let mut process = CapturedChild::spawn(fixture.resume_command(RepositoryMode::RepoDir, false));
    wait_for_marker(&mut process, &fixture.pause_marker);

    let refs = resume_refs(&fixture.local);
    assert_eq!(refs.len(), 1, "refs while update-ref is paused: {refs:?}");
    assert!(refs[0].ends_with("/checkpoint"), "refs: {refs:?}");
    assert_eq!(
        git_output(
            &executable_path("git"),
            &fixture.local,
            &["rev-parse", "--verify", &refs[0]],
        ),
        fixture.checkpoint
    );

    let (status, stderr) = process.signal_and_wait(libc::SIGTERM);
    assert_eq!(status.code(), Some(128 + libc::SIGTERM), "{stderr}");
    assert!(!stderr.contains("Review Resume Workbench:"), "{stderr}");
    fixture.assert_fetch_head_unchanged();
    fixture.assert_no_resume_refs();
    fixture.assert_no_pack_keep_files();
    fixture.assert_no_scratch_directories();

    let calls = fixture.calls();
    assert!(
        calls.contains(" update-ref --no-deref refs/stratadiff/resume/")
            && calls.contains(" update-ref --no-deref -d refs/stratadiff/resume/"),
        "CAS ref creation or cleanup was not observed:\n{calls}"
    );
}

#[test]
fn signal_after_fetch_pack_side_effect_removes_the_pid_owned_keep() {
    let fixture = Fixture::with_pause("github.com", false, PausePoint::FetchPack);
    let mut process = CapturedChild::spawn(fixture.resume_command(RepositoryMode::RepoDir, false));
    wait_for_marker(&mut process, &fixture.pause_marker);

    let keep_files = pack_keep_files(&fixture.local.join(".git/objects/pack"));
    assert_eq!(
        keep_files.len(),
        1,
        "keeps while fetch-pack is paused: {keep_files:?}"
    );
    assert!(
        fs::read_to_string(&keep_files[0])
            .unwrap()
            .starts_with("fetch-pack ")
    );
    assert!(resume_refs(&fixture.local).is_empty());

    let (status, stderr) = process.signal_and_wait(libc::SIGTERM);
    assert_eq!(status.code(), Some(128 + libc::SIGTERM), "{stderr}");
    assert!(!stderr.contains("Review Resume Workbench:"), "{stderr}");
    fixture.assert_fetch_head_unchanged();
    fixture.assert_no_resume_refs();
    fixture.assert_no_pack_keep_files();
    fixture.assert_no_scratch_directories();
}

#[test]
fn resume_rejects_cross_repository_pull_request_url_before_remote_resolution() {
    let fixture = Fixture::with_cross_repository_pr();
    let pull_request = "https://github.com/other/widget/pull/17";
    let output = fixture
        .resume_command_with_pull_request(RepositoryMode::RepoDir, false, pull_request)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let calls = fixture.calls();
    assert!(
        stderr.contains("pull request URL does not match the selected repository"),
        "stderr:\n{stderr}\ncalls:\n{calls}"
    );
    assert!(
        !stderr.contains("Review Resume Workbench:"),
        "stderr:\n{stderr}"
    );
    fixture.assert_fetch_head_unchanged();
    fixture.assert_no_resume_refs();
    fixture.assert_no_pack_keep_files();
    fixture.assert_no_scratch_directories();

    assert_eq!(calls.matches(" gh pr view ").count(), 0, "{calls}");
    let repo_call = calls
        .lines()
        .find(|line| line.contains(" gh repo view "))
        .unwrap();
    assert!(repo_call.starts_with(&format!("cwd={} ", fixture.local.display())));
    assert!(!repo_call.contains("other/widget"));
    for forbidden in [
        "/pulls/17/reviews",
        " gh auth token ",
        "https://github.com/acme/widget.git",
        " fetch-pack ",
        " phase=workbench",
    ] {
        assert!(
            !calls.contains(forbidden),
            "unexpected call containing {forbidden:?}:\n{calls}"
        );
    }
}

#[test]
fn resume_rejects_a_pull_request_url_that_conflicts_with_explicit_repository() {
    let fixture = Fixture::with_cross_repository_pr();
    let pull_request = "https://github.com/other/widget/pull/17";
    let output = fixture
        .resume_command_with_pull_request(RepositoryMode::RepositoryOnly, false, pull_request)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let calls = fixture.calls();
    assert!(
        stderr.contains("pull request URL does not match the selected repository"),
        "stderr:\n{stderr}\ncalls:\n{calls}"
    );
    assert_eq!(calls.matches(" gh pr view ").count(), 0, "{calls}");
    assert!(
        calls.contains(" gh repo view github.com/acme/widget "),
        "{calls}"
    );
    for forbidden in [
        "/pulls/17/reviews",
        " gh auth token ",
        "https://github.com/acme/widget.git",
        " fetch-pack ",
        " phase=workbench",
    ] {
        assert!(
            !calls.contains(forbidden),
            "unexpected call containing {forbidden:?}:\n{calls}"
        );
    }
    fixture.assert_fetch_head_unchanged();
    fixture.assert_no_resume_refs();
    fixture.assert_no_pack_keep_files();
    fixture.assert_no_scratch_directories();
}

#[test]
fn resume_rejects_cross_host_urls_before_gh_pr_view_with_explicit_repository_inputs() {
    for mode in [RepositoryMode::RepositoryOnly, RepositoryMode::RepoDir] {
        let fixture = Fixture::new("github.com", false);
        let pull_request = "https://credential-probe.invalid/acme/widget/pull/17";
        let output = fixture
            .resume_command_with_pull_request(mode, false, pull_request)
            .output()
            .unwrap();

        assert!(!output.status.success(), "{mode:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        let calls = fixture.calls();
        assert!(
            stderr.contains("pull request URL does not match the selected repository"),
            "{mode:?} stderr:\n{stderr}\ncalls:\n{calls}"
        );
        assert_eq!(calls.matches(" gh repo view ").count(), 1, "{calls}");
        assert_eq!(calls.matches(" gh pr view ").count(), 0, "{calls}");
        assert!(!calls.contains("credential-probe.invalid"), "{calls}");
        for forbidden in [
            " gh api ",
            " gh auth token ",
            " fetch-pack ",
            " phase=workbench",
        ] {
            assert!(
                !calls.contains(forbidden),
                "unexpected call containing {forbidden:?} in {mode:?}:\n{calls}"
            );
        }
        fixture.assert_fetch_head_unchanged();
        fixture.assert_no_resume_refs();
        fixture.assert_no_pack_keep_files();
        fixture.assert_no_scratch_directories();
    }
}

#[test]
fn url_only_ghes_requires_an_explicit_repository_or_local_checkout() {
    let fixture = Fixture::new("ghe-url.example", false);
    let output = fixture
        .resume_command(RepositoryMode::PullRequestUrl, false)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let calls = fixture.calls();
    assert!(
        stderr.contains("canonical github.com pull request URL"),
        "stderr:\n{stderr}\ncalls:\n{calls}"
    );
    assert!(!calls.lines().any(|line| line.contains(" gh ")), "{calls}");
    fixture.assert_fetch_head_unchanged();
    fixture.assert_no_resume_refs();
    fixture.assert_no_pack_keep_files();
    fixture.assert_no_scratch_directories();
}

#[test]
fn malformed_pull_request_urls_fail_before_repository_or_network_resolution() {
    for pull_request in [
        "http://github.com/acme/widget/pull/17",
        "HTTPS://github.com/acme/widget/pull/17",
        "https:/github.com/acme/widget/pull/17",
        "https://github.com/acme/widget/pull/17?view=files",
    ] {
        let fixture = Fixture::new("github.com", false);
        let output = fixture
            .resume_command_with_pull_request(RepositoryMode::RepoDir, false, pull_request)
            .output()
            .unwrap();

        assert!(!output.status.success(), "{pull_request}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        let calls = fixture.calls();
        assert!(
            stderr.contains("pull request URL must be exactly https://HOST/OWNER/REPO/pull/NUMBER"),
            "{pull_request} stderr:\n{stderr}\ncalls:\n{calls}"
        );
        assert!(calls.is_empty(), "{pull_request}:\n{calls}");
        fixture.assert_no_scratch_directories();
    }
}

#[test]
fn inbox_event_is_revalidated_before_the_workbench_opens() {
    let fixture = Fixture::new("github.com", false);
    let token = fixture.inbox_event_token();
    let mut command = fixture.resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false);
    command.args(["--inbox-event", &token]);
    let result = run_until_ready_then_signal(command, libc::SIGTERM);

    assert_eq!(result.status.code(), Some(128 + libc::SIGTERM));
    assert!(result.stderr.contains("Review Resume Workbench:"));
    let calls = fixture.calls();
    assert!(
        calls.contains(" --json id\\,nameWithOwner\\,url "),
        "{calls}"
    );
    assert!(
        calls.contains(" repos/acme/widget/pulls/17/reviews/101 "),
        "{calls}"
    );
    assert!(
        calls.contains(" repos/acme/widget/pulls/17/requested_reviewers\\?per_page=100 "),
        "{calls}"
    );
    fixture.assert_fetch_head_unchanged();
    fixture.assert_no_resume_refs();
    fixture.assert_no_pack_keep_files();
    fixture.assert_no_scratch_directories();
}

#[test]
fn malformed_inbox_event_fails_before_repository_or_network_resolution() {
    let fixture = Fixture::new("github.com", false);
    let output = fixture
        .resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false)
        .args(["--inbox-event", "not+a-token"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Inbox event token"));
    assert!(fixture.calls().is_empty());
    fixture.assert_no_scratch_directories();
}

#[test]
fn malformed_inbox_event_does_not_touch_an_opted_in_value_log() {
    let fixture = Fixture::new("github.com", false);
    let log = fixture.temporary_root.join("value-funnel.jsonl");
    fs::write(&log, b"caller-owned log\n").unwrap();
    let output = fixture
        .resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false)
        .args([
            "--inbox-event",
            "not+a-token",
            "--value-log",
            log.to_str().unwrap(),
            "--transition-id",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(fs::read(&log).unwrap(), b"caller-owned log\n");
    assert!(fixture.calls().is_empty());
    fixture.assert_no_scratch_directories();
}

#[test]
fn inbox_event_rejects_a_conflicting_repository_selector_before_resolution() {
    let fixture = Fixture::new("github.com", false);
    let token = fixture.inbox_event_token();
    let output = fixture
        .resume_command(RepositoryMode::PullRequestUrl, false)
        .args(["-R", "github.com/acme/other", "--inbox-event", &token])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--repo does not match"));
    assert!(fixture.calls().is_empty());
    fixture.assert_no_scratch_directories();
}

#[test]
fn inbox_event_rejects_reused_repository_identity_before_workbench() {
    let fixture = Fixture::new("github.com", false);
    let mut binding = fixture.inbox_event_binding();
    binding.repository_node_id = "R_recreated".to_owned();
    let token = InboxEventEnvelope::new(binding)
        .unwrap()
        .to_token()
        .unwrap();
    let output = fixture
        .resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false)
        .args(["--inbox-event", &token])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Inbox event no longer matches"), "{stderr}");
    assert!(!stderr.contains("Review Resume Workbench:"), "{stderr}");
    assert!(!fixture.calls().contains("phase=workbench"));
    fixture.assert_no_scratch_directories();
}

#[test]
fn inbox_event_rejects_live_identity_and_pull_request_drift_before_workbench() {
    for variable in [
        "STRATADIFF_TEST_REPOSITORY_NODE_DRIFT",
        "STRATADIFF_TEST_PULL_REQUEST_NODE_DRIFT",
        "STRATADIFF_TEST_REVIEWER_NODE_DRIFT",
        "STRATADIFF_TEST_REVIEW_DATABASE_DRIFT",
        "STRATADIFF_TEST_REVIEW_NODE_DRIFT",
        "STRATADIFF_TEST_REVIEW_STATE_DRIFT",
        "STRATADIFF_TEST_REVIEW_CHECKPOINT_DRIFT",
        "STRATADIFF_TEST_PULL_REQUEST_CLOSED",
        "STRATADIFF_TEST_BOUND_BASE_DRIFT",
        "STRATADIFF_TEST_BOUND_HEAD_DRIFT",
    ] {
        let fixture = Fixture::new("github.com", false);
        let token = fixture.inbox_event_token();
        let output = fixture
            .resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false)
            .args(["--inbox-event", &token])
            .env(variable, "1")
            .output()
            .unwrap();

        assert!(!output.status.success(), "{variable}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Inbox event") || stderr.contains("pull request"),
            "{variable}: {stderr}"
        );
        let calls = fixture.calls();
        assert!(!calls.contains("phase=workbench"), "{variable}: {calls}");
        fixture.assert_no_scratch_directories();
    }
}

#[test]
fn inbox_event_rejects_new_review_or_request_state_before_workbench() {
    for variable in [
        "STRATADIFF_TEST_REVIEW_DRIFT",
        "STRATADIFF_TEST_REVIEW_REQUEST_DRIFT",
        "STRATADIFF_TEST_RECREATED_REVIEW_REQUEST",
    ] {
        let fixture = Fixture::new("github.com", false);
        let token = fixture.inbox_event_token();
        let output = fixture
            .resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false)
            .args(["--inbox-event", &token])
            .env(variable, "1")
            .output()
            .unwrap();

        assert!(!output.status.success(), "{variable}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Inbox event") || stderr.contains("checkpoint base OID"),
            "{variable}: {stderr}"
        );
        assert!(!stderr.contains("Review Resume Workbench:"), "{stderr}");
        assert!(!fixture.calls().contains("phase=workbench"));
        fixture.assert_no_scratch_directories();
    }
}

#[test]
fn inbox_event_rejects_review_or_request_drift_between_complete_observations() {
    for variable in [
        "STRATADIFF_TEST_LATE_REVIEW_DRIFT",
        "STRATADIFF_TEST_LATE_REVIEW_REQUEST_DRIFT",
    ] {
        let fixture = Fixture::new("github.com", false);
        let token = fixture.inbox_event_token();
        let output = fixture
            .resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false)
            .args(["--inbox-event", &token])
            .env(variable, "1")
            .output()
            .unwrap();

        assert!(!output.status.success(), "{variable}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("Inbox event"), "{variable}: {stderr}");
        let calls = fixture.calls();
        assert!(!calls.contains("phase=workbench"), "{variable}: {calls}");
        fixture.assert_no_scratch_directories();
    }
}

#[test]
fn unrelated_bot_review_request_does_not_block_the_bound_user() {
    let fixture = Fixture::new("github.com", false);
    let token = fixture.inbox_event_token();
    let mut command = fixture.resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false);
    command
        .args(["--inbox-event", &token])
        .env("STRATADIFF_TEST_UNRELATED_BOT_REQUEST", "1");
    let result = run_until_ready_then_signal(command, libc::SIGTERM);

    assert_eq!(result.status.code(), Some(128 + libc::SIGTERM));
    assert!(result.stderr.contains("Review Resume Workbench:"));
    fixture.assert_no_scratch_directories();
}

#[test]
fn requested_reviewer_on_a_later_page_is_detected() {
    let fixture = Fixture::new("github.com", false);
    let mut binding = fixture.inbox_event_binding();
    binding.review_request_active = true;
    binding.triggers.push(InboxEventTrigger::ReviewReRequested);
    let token = InboxEventEnvelope::new(binding)
        .unwrap()
        .to_token()
        .unwrap();
    let mut command = fixture.resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false);
    command
        .args(["--inbox-event", &token])
        .env("STRATADIFF_TEST_REVIEW_REQUEST_SECOND_PAGE", "1");
    let result = run_until_ready_then_signal(command, libc::SIGTERM);

    assert_eq!(result.status.code(), Some(128 + libc::SIGTERM));
    assert!(result.stderr.contains("Review Resume Workbench:"));
    let calls = fixture.calls();
    assert_eq!(
        calls.matches("requested_reviewers\\?per_page=100").count(),
        2,
        "{calls}"
    );
    assert!(
        calls
            .lines()
            .filter(|line| line.contains("requested_reviewers\\?per_page=100"))
            .all(|line| line.contains(" --paginate ") && line.contains(" --slurp ")),
        "{calls}"
    );
    fixture.assert_no_scratch_directories();
}

#[test]
fn malformed_or_oversized_requested_reviewer_pages_fail_closed() {
    for (variable, diagnostic) in [
        (
            "STRATADIFF_TEST_REVIEW_REQUEST_TOO_MANY",
            "requested reviewer count limit exceeded",
        ),
        (
            "STRATADIFF_TEST_DUPLICATE_REVIEW_REQUEST_NODE",
            "duplicate requested reviewer or team identity",
        ),
        (
            "STRATADIFF_TEST_INVALID_REVIEW_REQUEST_NODE",
            "requested review team has an invalid node ID",
        ),
        (
            "STRATADIFF_TEST_MALFORMED_REVIEW_REQUEST_PAGE",
            "failed to decode requested reviewers",
        ),
    ] {
        let fixture = Fixture::new("github.com", false);
        let token = fixture.inbox_event_token();
        let output = fixture
            .resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false)
            .args(["--inbox-event", &token])
            .env(variable, "1")
            .output()
            .unwrap();

        assert!(!output.status.success(), "{variable}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(diagnostic), "{variable}: {stderr}");
        let calls = fixture.calls();
        assert!(!calls.contains("phase=workbench"), "{variable}: {calls}");
        fixture.assert_no_scratch_directories();
    }
}

#[test]
fn inbox_event_rejects_a_transient_new_review_before_materializing_its_checkpoint() {
    let fixture = Fixture::new("github.com", false);
    let token = fixture.inbox_event_token();
    let output = fixture
        .resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false)
        .args(["--inbox-event", &token])
        .env("STRATADIFF_TEST_REVERSE_REVIEW_DRIFT", "1")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("selected review checkpoint no longer matches"),
        "{stderr}"
    );
    let calls = fixture.calls();
    assert!(!calls.contains("/git/commits/"), "{calls}");
    assert!(!calls.contains("phase=workbench"), "{calls}");
    fixture.assert_no_scratch_directories();
}

#[test]
fn inbox_event_rejects_a_final_pull_request_change_before_workbench() {
    let fixture = Fixture::new("github.com", false);
    let token = fixture.inbox_event_token();
    let output = fixture
        .resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false)
        .args(["--inbox-event", &token])
        .env("STRATADIFF_TEST_FINAL_PR_DRIFT", "1")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("pull request changed while the Inbox event was being revalidated"),
        "{stderr}"
    );
    let calls = fixture.calls();
    assert!(!calls.contains("phase=workbench"), "{calls}");
    fixture.assert_no_scratch_directories();
}

#[test]
fn inbox_event_with_unverifiable_checkpoint_base_fails_before_repository_resolution() {
    let fixture = Fixture::new("github.com", false);
    let mut binding = fixture.inbox_event_binding();
    binding.checkpoint_base_oid = Some(binding.current_base_oid.clone().unwrap());
    let token = InboxEventEnvelope::new(binding)
        .unwrap()
        .to_token()
        .unwrap();
    let output = fixture
        .resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false)
        .args(["--inbox-event", &token])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot verify an event-attested checkpoint base"));
    assert!(fixture.calls().is_empty());
    fixture.assert_no_scratch_directories();
}

#[test]
fn inbox_event_with_base_drift_or_rerequest_only_is_explicitly_unsupported() {
    let fixture = Fixture::new("github.com", false);
    let mut cases = Vec::new();

    let mut base_drift = fixture.inbox_event_binding();
    base_drift.checkpoint_base_oid = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned());
    base_drift.triggers = vec![InboxEventTrigger::HeadChanged, InboxEventTrigger::BaseDrift];
    cases.push(base_drift);

    let mut rerequest_only = fixture.inbox_event_binding();
    rerequest_only.head_oid = rerequest_only.checkpoint_oid.clone();
    rerequest_only.checkpoint_base_oid = rerequest_only.current_base_oid.clone();
    rerequest_only.review_request_active = true;
    rerequest_only.triggers = vec![InboxEventTrigger::ReviewReRequested];
    cases.push(rerequest_only);

    for binding in cases {
        let token = InboxEventEnvelope::new(binding)
            .unwrap()
            .to_token()
            .unwrap();
        let output = fixture
            .resume_command(RepositoryMode::PullRequestUrlAndRepoDir, false)
            .args(["--inbox-event", &token])
            .output()
            .unwrap();

        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("cannot verify an event-attested checkpoint base")
        );
        assert!(fixture.calls().is_empty());
    }
    fixture.assert_no_scratch_directories();
}

#[test]
fn resume_rejects_a_second_pull_request_snapshot_that_drifted() {
    let fixture = Fixture::new("github.com", true);
    let output = fixture
        .resume_command(RepositoryMode::RepoDir, true)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let calls = fixture.calls();
    assert!(
        stderr.contains(
            "pull request base or head changed while review coverage was being resolved; rerun the command"
        ),
        "stderr:\n{stderr}\ncalls:\n{calls}"
    );
    assert!(
        !stderr.contains("Review Resume Workbench:"),
        "stderr:\n{stderr}"
    );
    fixture.assert_fetch_head_unchanged();
    fixture.assert_no_resume_refs();
    fixture.assert_no_pack_keep_files();
    fixture.assert_no_scratch_directories();

    assert_eq!(calls.matches(" gh pr view ").count(), 2, "{calls}");
    assert!(calls.contains("gh api --hostname github.com user --jq .login"));
    assert!(calls.contains(" gh pr view 17 --repo github.com/acme/widget "));
}

struct TerminatedRun {
    status: ExitStatus,
    stderr: String,
    session: String,
    address: SocketAddr,
}

fn run_until_ready_then_signal(mut command: Command, signal: i32) -> TerminatedRun {
    command.stderr(Stdio::piped());
    let process = CapturedChild::spawn(command);

    let deadline = Instant::now() + PROCESS_TIMEOUT;
    let url = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "resume did not start its workbench:\n{}",
            process.stderr()
        );
        let line = process
            .line_receiver
            .recv_timeout(remaining)
            .unwrap_or_else(|error| {
                panic!(
                    "resume did not start its workbench ({error}):\n{}",
                    process.stderr()
                )
            });
        if let Some(url) = line
            .trim_end()
            .strip_prefix("StrataDiff Review Resume Workbench: ")
        {
            break url.to_owned();
        }
    };
    let (address, token) = parse_workbench_url(&url);
    let session = http_get(address, &format!("/api/session?token={token}"));
    assert!(session.starts_with("HTTP/1.1 200 OK\r\n"), "{session}");

    let (status, stderr) = process.signal_and_wait(signal);

    TerminatedRun {
        status,
        stderr,
        session,
        address,
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("resume did not exit after signal delivery");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_marker(process: &mut CapturedChild, marker: &Path) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if marker.exists() {
            thread::sleep(Duration::from_millis(100));
            assert!(
                process.child.child_mut().try_wait().unwrap().is_none(),
                "resume exited after creating the pause marker:\n{}",
                process.stderr()
            );
            return;
        }
        if let Some(status) = process.child.child_mut().try_wait().unwrap() {
            panic!(
                "resume exited with {status} before the side-effect marker:\n{}",
                process.stderr()
            );
        }
        assert!(
            Instant::now() < deadline,
            "resume did not reach the side-effect marker:\n{}",
            process.stderr()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn parse_workbench_url(url: &str) -> (SocketAddr, String) {
    let remainder = url.strip_prefix("http://").unwrap();
    let (address, token) = remainder.split_once("/?token=").unwrap();
    let address = address.parse().unwrap();
    assert_eq!(token.len(), 64);
    (address, token.to_owned())
}

fn http_get(address: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn pack_keep_files(pack_directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(pack_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "keep")
        })
        .collect()
}

fn resume_refs(repository: &Path) -> Vec<String> {
    let output = git_output(
        &executable_path("git"),
        repository,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/stratadiff/resume",
        ],
    );
    output.lines().map(str::to_owned).collect()
}

fn commit(real_git: &Path, repository: &Path, message: &str) -> String {
    git_at(real_git, repository, &["add", "--all"]);
    git_at(real_git, repository, &["commit", "--quiet", "-m", message]);
    git_output(real_git, repository, &["rev-parse", "HEAD"])
}

fn git_at(real_git: &Path, repository: &Path, arguments: &[&str]) {
    command_success(
        Command::new(real_git)
            .arg("-C")
            .arg(repository)
            .args(arguments),
        &format!("git {}", arguments.join(" ")),
    );
}

fn git_output(real_git: &Path, repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new(real_git)
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed:\n{}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn command_success(command: &mut Command, operation: &str) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{operation} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn executable_path(name: &str) -> PathBuf {
    let output = Command::new("sh")
        .args(["-c", &format!("command -v {name}")])
        .output()
        .unwrap();
    assert!(output.status.success());
    fs::canonicalize(String::from_utf8(output.stdout).unwrap().trim()).unwrap()
}

fn test_tempdir() -> tempfile::TempDir {
    #[cfg(target_os = "linux")]
    {
        let shared_memory = Path::new("/dev/shm");
        assert!(shared_memory.is_dir(), "/dev/shm is required on Linux");
        tempfile::Builder::new()
            .prefix("stratadiff-resume-test-")
            .tempdir_in(shared_memory)
            .unwrap()
    }
    #[cfg(not(target_os = "linux"))]
    {
        tempfile::tempdir().unwrap()
    }
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn shell_quote(value: &Path) -> String {
    let value = value.to_str().unwrap();
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn git_proxy_script(
    real_git: &Path,
    provider: &Path,
    isolated: &Path,
    provider_url: &str,
    log: &Path,
    pause_point: PausePoint,
    pause_marker: &Path,
) -> String {
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
trap 'exit 143' TERM
trap 'exit 130' INT
trap 'exit 129' HUP
real_git={real_git}
provider={provider}
isolated={isolated}
provider_url={provider_url:?}
log={log}
pause_point={pause_point:?}
pause_marker={pause_marker}

secrets=clean
if [[ -n "${{GH_TOKEN+x}}${{GITHUB_TOKEN+x}}${{GH_ENTERPRISE_TOKEN+x}}${{GITHUB_ENTERPRISE_TOKEN+x}}${{STRATADIFF_GITHUB_TOKEN+x}}${{github_token+x}}${{git_authorization+x}}" ]]; then
  secrets=leaked
fi
enterprise_token=absent
if [[ -n "${{GITHUB_ENTERPRISE_TOKEN+x}}" ]]; then
  enterprise_token=present
fi
auth=none
if [[ "${{GIT_CONFIG_VALUE_1:-}}" == AUTHORIZATION:* ]]; then
  auth=present
fi
phase=orchestrator
if [[ -z "${{CALLER_SECRET+x}}" ]]; then
  phase=workbench
fi
printf 'git secrets=%s enterprise_token=%s auth=%s phase=%s cwd=%s' "$secrets" "$enterprise_token" "$auth" "$phase" "$PWD" >> "$log"
printf ' %q' "$@" >> "$log"
printf '\n' >> "$log"

arguments=("$@")
if [[ " $* " == *" init --bare --quiet "*"/repository.git " ]]; then
  cp -R "$isolated" "${{!#}}"
  exit 0
fi
provider_fetch=false
for ((index = 0; index < ${{#arguments[@]}}; index++)); do
  if [[ "${{arguments[index]}}" == "$provider_url" ]]; then
    provider_fetch=true
  fi
done
if [[ "$provider_fetch" == true ]]; then
  exit 0
fi
if [[ " $* " == *" rev-parse --verify refs/stratadiff/provider/"* ]]; then
  object_id="${{!#}}"
  object_id="${{object_id%\^\{{commit\}}}}"
  object_id="${{object_id##*-}}"
  printf '%s\n' "$object_id"
  exit 0
fi
if [[ " $* " == *" ls-remote --get-url "*"/provider.git " ]]; then
  printf '%s\n' "${{!#}}"
  exit 0
fi
if [[ " $* " == *" fetch-pack --no-progress "*"/provider.git "* ]]; then
  for ((index = 0; index < ${{#arguments[@]}}; index++)); do
    if [[ "${{arguments[index]}}" == */provider.git ]]; then
      arguments[index]="$provider"
    fi
  done
  if [[ "$pause_point" == fetch-pack ]]; then
    output="${{pause_marker}}.stdout"
    "$real_git" -c fetch.unpackLimit=1 "${{arguments[@]}}" > "$output"
    target=
    for ((index = 0; index < ${{#arguments[@]}} - 1; index++)); do
      if [[ "${{arguments[index]}}" == -C ]]; then
        target="${{arguments[index + 1]}}"
        break
      fi
    done
    [[ -n "$target" ]]
    pack_directory="$target/.git/objects/pack"
    if [[ "$target" == *.git ]]; then
      pack_directory="$target/objects/pack"
    fi
    keep_files=("$pack_directory"/*.keep)
    [[ -e "${{keep_files[0]}}" ]]
    printf 'fetch-pack %s on deterministic-fixture\n' "$$" > "${{keep_files[0]}}"
    printf 'ready\n' > "$pause_marker"
    while :; do sleep 1; done
  fi
  exec "$real_git" -c fetch.unpackLimit=1 "${{arguments[@]}}"
fi
if [[ "$pause_point" == update-ref && " $* " == *" update-ref --no-deref refs/stratadiff/resume/"* && " $* " != *" update-ref --no-deref -d "* ]]; then
  "$real_git" "${{arguments[@]}}"
  printf 'ready\n' > "$pause_marker"
  sleep 1
  exit 0
fi
exec "$real_git" "${{arguments[@]}}"
"#,
        real_git = shell_quote(real_git),
        provider = shell_quote(provider),
        isolated = shell_quote(isolated),
        provider_url = provider_url,
        log = shell_quote(log),
        pause_point = pause_point.as_str(),
        pause_marker = shell_quote(pause_marker),
    )
}

#[allow(clippy::too_many_arguments)]
fn gh_stub_script(
    host: &str,
    base: &str,
    checkpoint: &str,
    head: &str,
    state: &Path,
    log: &Path,
    drift: bool,
    cross_repository_pr: bool,
) -> String {
    let drift_head = if drift { base } else { head };
    let first_pr_url = if cross_repository_pr {
        format!("https://{host}/other/widget/pull/17")
    } else {
        format!("https://{host}/acme/widget/pull/17")
    };
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
host={host:?}
base={base:?}
checkpoint={checkpoint:?}
head={head:?}
drift_head={drift_head:?}
first_pr_url={first_pr_url:?}
state={state}
log={log}

git_dir=clean
if [[ -n "${{GIT_DIR+x}}" ]]; then
  git_dir=poisoned
fi
printf 'cwd=%s gh' "$PWD" >> "$log"
printf ' %q' "$@" >> "$log"
printf ' git_dir=%s\n' "$git_dir" >> "$log"
arguments=" $* "

case "${{1:-}} ${{2:-}}" in
  "repo view")
    if [[ "$arguments" == *" --json id,nameWithOwner,url "* ]]; then
      repository_id=R_widget
      if [[ "${{STRATADIFF_TEST_REPOSITORY_NODE_DRIFT:-}}" == 1 ]]; then
        repository_id=R_recreated
      fi
      printf '{{"id":"%s","nameWithOwner":"acme/widget","url":"https://%s/acme/widget"}}\n' "$repository_id" "$host"
    else
      printf '{{"nameWithOwner":"acme/widget","url":"https://%s/acme/widget"}}\n' "$host"
    fi
    ;;
  "pr view")
    [[ "${{3:-}}" != *"://"* ]]
    [[ "$arguments" == *" --repo $host/acme/widget "* ]]
    if [[ "$arguments" != *" --json number,baseRefOid,headRefOid,url "* && "$arguments" != *" --json id,number,state,url,baseRefOid,headRefOid "* ]]; then
      printf 'unexpected gh pr view fields: %s\n' "$*" >&2
      exit 70
    fi
    count=0
    if [[ -f "$state/pr-count" ]]; then
      read -r count < "$state/pr-count"
    fi
    count=$((count + 1))
    printf '%s\n' "$count" > "$state/pr-count"
    selected_head="$head"
    selected_url="https://$host/acme/widget/pull/17"
    if (( count == 1 )); then
      selected_url="$first_pr_url"
    fi
    if (( count > 1 )); then
      selected_head="$drift_head"
    fi
    bound_count=0
    if [[ "$arguments" == *" --json id,number,state,url,baseRefOid,headRefOid "* ]]; then
      if [[ -f "$state/bound-pr-count" ]]; then
        read -r bound_count < "$state/bound-pr-count"
      fi
      bound_count=$((bound_count + 1))
      printf '%s\n' "$bound_count" > "$state/bound-pr-count"
      if [[ "${{STRATADIFF_TEST_FINAL_PR_DRIFT:-}}" == 1 && "$bound_count" -gt 1 ]]; then
        selected_head="$base"
      fi
    fi
    if [[ "$arguments" == *" --json id,number,state,url,baseRefOid,headRefOid "* ]]; then
      pull_request_id=PR_17
      pull_request_state=OPEN
      selected_base="$base"
      if [[ "${{STRATADIFF_TEST_PULL_REQUEST_NODE_DRIFT:-}}" == 1 ]]; then
        pull_request_id=PR_recreated
      fi
      if [[ "${{STRATADIFF_TEST_PULL_REQUEST_CLOSED:-}}" == 1 ]]; then
        pull_request_state=CLOSED
      fi
      if [[ "${{STRATADIFF_TEST_BOUND_BASE_DRIFT:-}}" == 1 ]]; then
        selected_base="$checkpoint"
      fi
      if [[ "${{STRATADIFF_TEST_BOUND_HEAD_DRIFT:-}}" == 1 ]]; then
        selected_head="$base"
      fi
      printf '{{"id":"%s","number":17,"state":"%s","baseRefOid":"%s","headRefOid":"%s","url":"%s"}}\n' "$pull_request_id" "$pull_request_state" "$selected_base" "$selected_head" "$selected_url"
    else
      printf '{{"number":17,"baseRefOid":"%s","headRefOid":"%s","url":"%s"}}\n' "$base" "$selected_head" "$selected_url"
    fi
    ;;
  "auth token")
    [[ "$arguments" == *" --hostname $host "* ]]
    printf 'deterministic-test-token\n'
    ;;
  "api --hostname")
    [[ "$arguments" == *" --hostname $host "* ]]
    if [[ "$arguments" == *" user --jq .login "* ]]; then
      printf 'authenticated-reviewer\n'
    elif [[ "$arguments" == *" repos/acme/widget/pulls/17/reviews/101 "* ]]; then
      review_database_id=101
      review_node_id=PRR_101
      reviewer_node_id=U_alice
      review_state=APPROVED
      review_checkpoint="$checkpoint"
      if [[ "${{STRATADIFF_TEST_REVIEW_DATABASE_DRIFT:-}}" == 1 ]]; then
        review_database_id=999
      fi
      if [[ "${{STRATADIFF_TEST_REVIEW_NODE_DRIFT:-}}" == 1 ]]; then
        review_node_id=PRR_recreated
      fi
      if [[ "${{STRATADIFF_TEST_REVIEWER_NODE_DRIFT:-}}" == 1 ]]; then
        reviewer_node_id=U_recreated
      fi
      if [[ "${{STRATADIFF_TEST_REVIEW_STATE_DRIFT:-}}" == 1 ]]; then
        review_state=CHANGES_REQUESTED
      fi
      if [[ "${{STRATADIFF_TEST_REVIEW_CHECKPOINT_DRIFT:-}}" == 1 ]]; then
        review_checkpoint="$base"
      fi
      printf '{{"id":%s,"node_id":"%s","user":{{"login":"alice","node_id":"%s","type":"User"}},"state":"%s","html_url":"https://%s/acme/widget/pull/17#pullrequestreview-101","commit_id":"%s","submitted_at":"2026-09-04T17:10:09Z","author_association":"MEMBER"}}\n' "$review_database_id" "$review_node_id" "$reviewer_node_id" "$review_state" "$host" "$review_checkpoint"
    elif [[ "$arguments" == *" repos/acme/widget/pulls/17/reviews/103 "* ]]; then
      printf '{{"id":103,"node_id":"PRR_103","user":{{"login":"alice","node_id":"U_alice","type":"User"}},"state":"APPROVED","html_url":"https://%s/acme/widget/pull/17#pullrequestreview-103","commit_id":"%s","submitted_at":"2026-09-04T19:10:09Z","author_association":"MEMBER"}}\n' "$host" "$head"
    elif [[ "$arguments" == *"/git/commits/"* ]]; then
      object_id="${{arguments##*/git/commits/}}"
      object_id="${{object_id%% *}}"
      printf '{{"sha":"%s"}}\n' "$object_id"
    else
      printf 'unexpected gh api invocation: %s\n' "$*" >&2
      exit 70
    fi
    ;;
  "api --paginate")
    [[ "$arguments" == *" --hostname $host "* ]]
    [[ "$arguments" == *" --slurp "* ]]
    if [[ "$arguments" == *" repos/acme/widget/pulls/17/reviews?per_page=100 "* ]]; then
      review_count=0
      if [[ -f "$state/review-count" ]]; then
        read -r review_count < "$state/review-count"
      fi
      review_count=$((review_count + 1))
      printf '%s\n' "$review_count" > "$state/review-count"
      extra=
      if [[ "${{STRATADIFF_TEST_REVIEW_DRIFT:-}}" == 1 && "$review_count" -gt 1 ]] || [[ "${{STRATADIFF_TEST_REVERSE_REVIEW_DRIFT:-}}" == 1 && "$review_count" == 1 ]] || [[ "${{STRATADIFF_TEST_LATE_REVIEW_DRIFT:-}}" == 1 && "$review_count" -gt 2 ]]; then
        extra=',{{"id":103,"user":{{"login":"alice","type":"User"}},"state":"APPROVED","html_url":"https://%s/acme/widget/pull/17#pullrequestreview-103","commit_id":"%s","submitted_at":"2026-09-04T19:10:09Z","author_association":"MEMBER"}}'
        extra=$(printf "$extra" "$host" "$head")
      fi
      printf '[[{{"id":101,"user":{{"login":"alice","type":"User"}},"state":"APPROVED","html_url":"https://%s/acme/widget/pull/17#pullrequestreview-101","commit_id":"%s","submitted_at":"2026-09-04T17:10:09Z","author_association":"MEMBER"}},{{"id":102,"user":{{"login":"authenticated-reviewer","type":"User"}},"state":"CHANGES_REQUESTED","html_url":"https://%s/acme/widget/pull/17#pullrequestreview-102","commit_id":"%s","submitted_at":"2026-09-04T18:10:09Z","author_association":"MEMBER"}}%s]]\n' "$host" "$checkpoint" "$host" "$checkpoint" "$extra"
    elif [[ "$arguments" == *" repos/acme/widget/pulls/17/requested_reviewers?per_page=100 "* ]]; then
      request_count=0
      if [[ -f "$state/request-count" ]]; then
        read -r request_count < "$state/request-count"
      fi
      request_count=$((request_count + 1))
      printf '%s\n' "$request_count" > "$state/request-count"
      if [[ "${{STRATADIFF_TEST_REVIEW_REQUEST_TOO_MANY:-}}" == 1 ]]; then
        printf '[{{"users":['
        for ((requested_index = 1; requested_index <= 100; requested_index++)); do
          if (( requested_index > 1 )); then
            printf ','
          fi
          printf '{{"login":"reviewer%s","node_id":"U_%s","type":"User"}}' "$requested_index" "$requested_index"
        done
        printf '],"teams":[]}},{{"users":[],"teams":[{{"node_id":"T_overflow"}}]}}]\n'
      elif [[ "${{STRATADIFF_TEST_DUPLICATE_REVIEW_REQUEST_NODE:-}}" == 1 ]]; then
        printf '[{{"users":[{{"login":"other","node_id":"U_duplicate","type":"User"}}],"teams":[]}},{{"users":[],"teams":[{{"node_id":"U_duplicate"}}]}}]\n'
      elif [[ "${{STRATADIFF_TEST_INVALID_REVIEW_REQUEST_NODE:-}}" == 1 ]]; then
        printf '[{{"users":[],"teams":[{{"node_id":"bad node"}}]}}]\n'
      elif [[ "${{STRATADIFF_TEST_MALFORMED_REVIEW_REQUEST_PAGE:-}}" == 1 ]]; then
        printf '[{{"users":[]}}]\n'
      elif [[ "${{STRATADIFF_TEST_REVIEW_REQUEST_SECOND_PAGE:-}}" == 1 ]]; then
        printf '[{{"users":[{{"login":"other","node_id":"U_other","type":"User"}}],"teams":[]}},{{"users":[{{"login":"alice","node_id":"U_alice","type":"User"}}],"teams":[]}}]\n'
      elif [[ "${{STRATADIFF_TEST_REVIEW_REQUEST_DRIFT:-}}" == 1 || "${{STRATADIFF_TEST_LATE_REVIEW_REQUEST_DRIFT:-}}" == 1 && "$request_count" -gt 1 ]]; then
        printf '[{{"users":[{{"login":"alice","node_id":"U_alice","type":"User"}}],"teams":[]}}]\n'
      elif [[ "${{STRATADIFF_TEST_RECREATED_REVIEW_REQUEST:-}}" == 1 ]]; then
        printf '[{{"users":[{{"login":"alice","node_id":"U_recreated","type":"User"}}],"teams":[]}}]\n'
      elif [[ "${{STRATADIFF_TEST_UNRELATED_BOT_REQUEST:-}}" == 1 ]]; then
        printf '[{{"users":[{{"login":"copilot-pull-request-reviewer[bot]","node_id":"BOT_copilot","type":"Bot"}}],"teams":[]}}]\n'
      else
        printf '[{{"users":[],"teams":[]}}]\n'
      fi
    else
      printf 'unexpected paginated gh api invocation: %s\n' "$*" >&2
      exit 70
    fi
    ;;
  *)
    printf 'unexpected gh invocation: %s\n' "$*" >&2
    exit 70
    ;;
esac
"#,
        host = host,
        base = base,
        checkpoint = checkpoint,
        head = head,
        drift_head = drift_head,
        first_pr_url = first_pr_url,
        state = shell_quote(state),
        log = shell_quote(log),
    )
}
