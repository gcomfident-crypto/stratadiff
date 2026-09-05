use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn git(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn commit(repository: &Path, message: &str) -> String {
    git(repository, &["add", "--all"]);
    git(repository, &["commit", "-q", "-m", message]);
    git(repository, &["rev-parse", "HEAD"])
}

#[test]
fn viewer_serves_a_token_bound_verified_session_on_loopback() {
    let directory = tempfile::tempdir().unwrap();
    let before_path = directory.path().join("before.py");
    let after_path = directory.path().join("after.py");
    fs::write(
        &before_path,
        b"def total(values):\n    return sum(values)\n",
    )
    .unwrap();
    fs::write(
        &after_path,
        b"def total(values):\n    return sum(values, start=1)\n",
    )
    .unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("view")
        .arg(&before_path)
        .arg(&after_path)
        .arg("--port")
        .arg("0")
        .arg("--no-open")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let stderr = child.0.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let _reader = thread::spawn(move || {
        let mut stderr = BufReader::new(stderr);
        let mut first_line = String::new();
        let result = stderr.read_line(&mut first_line).map(|_| first_line);
        let mut stop_hint = String::new();
        let _ = stderr.read_line(&mut stop_hint);
        let _ = sender.send(result);
    });
    let first_line = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("viewer did not print its URL within 30 seconds")
        .unwrap();
    let url = first_line
        .trim_end()
        .strip_prefix("StrataDiff Evidence Workbench: http://")
        .unwrap();
    let (address, token) = url.split_once("/?token=").unwrap();
    assert!(address.starts_with("127.0.0.1:"), "{address}");
    assert_eq!(token.len(), 64);

    let index = get(address, "/");
    assert!(index.starts_with("HTTP/1.1 200 OK\r\n"), "{index}");
    assert!(index.contains("content-security-policy:"), "{index}");
    assert!(index.contains("StrataDiff"), "{index}");

    let denied = get(address, "/api/session?token=invalid");
    assert!(denied.starts_with("HTTP/1.1 404 Not Found\r\n"), "{denied}");

    let session = get(address, &format!("/api/session?token={token}"));
    assert!(session.starts_with("HTTP/1.1 200 OK\r\n"), "{session}");
    assert!(session.contains("cache-control: no-store"), "{session}");
    let (_, body) = session.split_once("\r\n\r\n").unwrap();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["verification"]["verified"], true);
    assert_eq!(payload["kind"], "file_diff");
    assert_eq!(payload["report"]["certificate"]["patch_verified"], true);
    assert_eq!(
        payload["report"]["before"]["path"],
        before_path.to_string_lossy().as_ref()
    );
    assert_eq!(
        payload["report"]["after"]["path"],
        after_path.to_string_lossy().as_ref()
    );

    let before = get_bytes(address, &format!("/api/source/before?token={token}"));
    assert!(before.starts_with(b"HTTP/1.1 200 OK\r\n"), "{before:?}");
    assert!(
        before
            .windows(b"content-type: application/octet-stream".len())
            .any(|window| window == b"content-type: application/octet-stream"),
        "{before:?}"
    );
    let (_, before_body) = split_response(&before);
    assert_eq!(before_body, b"def total(values):\n    return sum(values)\n");

    let after = get_bytes(address, &format!("/api/source/after?token={token}"));
    let (_, after_body) = split_response(&after);
    assert_eq!(
        after_body,
        b"def total(values):\n    return sum(values, start=1)\n"
    );

    let wrong_host = get_with_host(
        address,
        &format!("/api/session?token={token}"),
        "example.test",
    );
    assert!(
        wrong_host.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "{wrong_host}"
    );
}

#[test]
fn repository_workbench_serves_checkpoint_delta_and_commit_bound_sources() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);
    fs::write(root.join("stable.py"), b"value = 0\n").unwrap();
    fs::write(root.join("changing.py"), b"value = 0\n").unwrap();
    let base = commit(root, "base");

    fs::write(root.join("stable.py"), b"value = 1\n").unwrap();
    fs::write(root.join("changing.py"), b"value = 1\n").unwrap();
    let checkpoint = commit(root, "reviewed checkpoint");

    git(root, &["checkout", "-q", &base]);
    fs::write(root.join("stable.py"), b"value = 1\n").unwrap();
    fs::write(root.join("changing.py"), b"value = 2\n").unwrap();
    let head = commit(root, "current head");

    let child = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg("--repo")
        .arg(root)
        .arg("--checkpoint")
        .arg(&checkpoint)
        .arg("--workbench")
        .arg("--port")
        .arg("0")
        .arg("--no-open")
        .arg("--")
        .arg(&base)
        .arg(&head)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let stderr = child.0.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let _reader = thread::spawn(move || {
        let mut stderr = BufReader::new(stderr);
        let mut first_line = String::new();
        let result = stderr.read_line(&mut first_line).map(|_| first_line);
        let mut stop_hint = String::new();
        let _ = stderr.read_line(&mut stop_hint);
        let _ = sender.send(result);
    });
    let first_line = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("review workbench did not print its URL within 30 seconds")
        .unwrap();
    let url = first_line
        .trim_end()
        .strip_prefix("StrataDiff Review Resume Workbench: http://")
        .unwrap();
    let (address, token) = url.split_once("/?token=").unwrap();

    let session = get(address, &format!("/api/session?token={token}"));
    assert!(session.starts_with("HTTP/1.1 200 OK\r\n"), "{session}");
    let (_, body) = session.split_once("\r\n\r\n").unwrap();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["kind"], "repository_review");
    assert!(payload.get("verification").is_none());
    assert_eq!(payload["assessment"]["status"], "producer_attested");
    assert_eq!(payload["review"]["summary"]["changed_files"], 2);
    assert_eq!(
        payload["review"]["summary"]["checkpoint"]["needs_review_now_files"],
        1
    );
    assert_eq!(
        payload["review"]["summary"]["checkpoint"]["unchanged_since_checkpoint_files"],
        1
    );
    assert_eq!(
        payload["review"]["summary"]["checkpoint"]["retired_change_count"],
        1
    );
    assert_eq!(payload["resume_delta"]["comparison"], "checkpoint_to_head");
    assert_eq!(payload["resume_delta"]["checkpoint_commit"], checkpoint);
    assert_eq!(payload["resume_delta"]["current_base_commit"], base);
    assert_eq!(payload["resume_delta"]["summary"]["displayable_files"], 1);
    assert_eq!(
        payload["resume_delta"]["entries"][0]["file"]["after_path"],
        "changing.py"
    );

    fs::write(root.join("changing.py"), b"uncommitted worktree mutation\n").unwrap();
    let detail = get(
        address,
        &format!("/api/session?token={token}&file=0&scope=resume"),
    );
    let (_, detail_body) = detail.split_once("\r\n\r\n").unwrap();
    let detail_payload: serde_json::Value = serde_json::from_str(detail_body).unwrap();
    assert_eq!(detail_payload["kind"], "file_diff");
    assert_eq!(detail_payload["verification"]["verified"], true);
    assert_eq!(detail_payload["repository_context"]["file_index"], 0);
    assert_eq!(detail_payload["repository_context"]["scope"], "resume");
    assert!(
        detail_payload["repository_context"]
            .get("checkpoint_state")
            .is_none()
    );
    assert!(
        detail_payload["repository_context"]
            .get("checkpoint_match_basis")
            .is_none()
    );

    let before = get_bytes(
        address,
        &format!("/api/source/before?token={token}&file=0&scope=resume"),
    );
    let (_, before_body) = split_response(&before);
    assert_eq!(before_body, b"value = 1\n");
    let after = get_bytes(
        address,
        &format!("/api/source/after?token={token}&file=0&scope=resume"),
    );
    let (_, after_body) = split_response(&after);
    assert_eq!(after_body, b"value = 2\n");

    let denied = get(
        address,
        "/api/source/after?token=invalid&file=0&scope=resume",
    );
    assert!(denied.starts_with("HTTP/1.1 404 Not Found\r\n"), "{denied}");

    let missing_session = get(
        address,
        &format!("/api/session?token={token}&file=999&scope=resume"),
    );
    assert!(
        missing_session.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "{missing_session}"
    );
    let missing_source = get(
        address,
        &format!("/api/source/after?token={token}&file=999&scope=full"),
    );
    assert!(
        missing_source.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "{missing_source}"
    );
}

#[test]
fn repository_workbench_serves_pr_relative_residue_after_base_change() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "StrataDiff Test"]);
    git(root, &["config", "user.email", "stratadiff@example.test"]);
    fs::write(root.join("shared.py"), b"value = 0\n").unwrap();
    fs::write(root.join("current.py"), b"value = 0\n").unwrap();
    let original_base = commit(root, "original base");

    fs::write(root.join("shared.py"), b"value = 1\n").unwrap();
    fs::write(root.join("current.py"), b"value = 1\n").unwrap();
    let checkpoint = commit(root, "reviewed checkpoint");

    git(root, &["checkout", "-q", &original_base]);
    fs::write(root.join("upstream-only.py"), b"base update\n").unwrap();
    let current_base = commit(root, "advanced base");
    fs::write(root.join("shared.py"), b"value = 1\n").unwrap();
    fs::write(root.join("current.py"), b"value = 2\n").unwrap();
    let head = commit(root, "current head");

    let child = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("review")
        .arg("--repo")
        .arg(root)
        .arg("--checkpoint")
        .arg(&checkpoint)
        .arg("--workbench")
        .arg("--port")
        .arg("0")
        .arg("--no-open")
        .arg("--")
        .arg(&current_base)
        .arg(&head)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let stderr = child.0.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let _reader = thread::spawn(move || {
        let mut stderr = BufReader::new(stderr);
        let mut first_line = String::new();
        let result = stderr.read_line(&mut first_line).map(|_| first_line);
        let mut stop_hint = String::new();
        let _ = stderr.read_line(&mut stop_hint);
        let _ = sender.send(result);
    });
    let first_line = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("review workbench did not print its URL within 30 seconds")
        .unwrap();
    let url = first_line
        .trim_end()
        .strip_prefix("StrataDiff Review Resume Workbench: http://")
        .unwrap();
    let (address, token) = url.split_once("/?token=").unwrap();

    let session = get(address, &format!("/api/session?token={token}"));
    assert!(session.starts_with("HTTP/1.1 200 OK\r\n"), "{session}");
    let (_, body) = session.split_once("\r\n\r\n").unwrap();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        payload["resume_delta"]["comparison"],
        "per_file_review_baseline_to_head"
    );
    assert_eq!(
        payload["review"]["checkpoint"]["match_basis"],
        "exact_git_change_identity_or_noninteracting_four_way_byte_replay"
    );
    assert_eq!(
        payload["assessment"]["basis"],
        "exact_git_change_identity_or_noninteracting_four_way_byte_replay"
    );
    assert_eq!(payload["resume_delta"]["old_base_commit"], original_base);
    assert_eq!(payload["resume_delta"]["checkpoint_commit"], checkpoint);
    assert_eq!(payload["resume_delta"]["current_base_commit"], current_base);
    assert_eq!(payload["resume_delta"]["head_commit"], head);
    let queue = payload["resume_delta"]["entries"].as_array().unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0]["file"]["after_path"], "current.py");
    assert_eq!(queue[0]["baseline_basis"], "reconstructed_review_baseline");
    assert_eq!(queue[0]["before_source"]["kind"], "reconstructed_bytes");
    assert_eq!(queue[0]["after_source"]["kind"], "git_object");
    let reconstructed_blake3 = queue[0]["baseline_reconstruction"]["reconstructed_blake3"]
        .as_str()
        .unwrap();
    assert!(
        queue
            .iter()
            .all(|entry| entry["file"]["after_path"] != "shared.py")
    );
    assert!(
        queue
            .iter()
            .all(|entry| entry["file"]["after_path"] != "upstream-only.py")
    );

    let detail = get(
        address,
        &format!("/api/session?token={token}&file=0&scope=resume"),
    );
    let (_, detail_body) = detail.split_once("\r\n\r\n").unwrap();
    let detail_payload: serde_json::Value = serde_json::from_str(detail_body).unwrap();
    assert_eq!(
        detail_payload["repository_context"]["checkpoint_state"],
        "needs_review_now"
    );
    assert_eq!(
        detail_payload["repository_context"]["baseline_basis"],
        "reconstructed_review_baseline"
    );
    assert_eq!(
        detail_payload["report"]["certificate"]["before_blake3"],
        reconstructed_blake3
    );
    assert!(
        detail_payload["repository_context"]
            .get("checkpoint_match_basis")
            .is_none()
    );

    let carried_detail = get(
        address,
        &format!("/api/session?token={token}&file=1&scope=full"),
    );
    let (_, carried_detail_body) = carried_detail.split_once("\r\n\r\n").unwrap();
    let carried_payload: serde_json::Value = serde_json::from_str(carried_detail_body).unwrap();
    assert_eq!(
        carried_payload["repository_context"]["checkpoint_state"],
        "unchanged_since_checkpoint"
    );
    assert_eq!(
        carried_payload["repository_context"]["checkpoint_match_basis"],
        "exact_git_change_identity"
    );

    fs::write(root.join("current.py"), b"uncommitted worktree mutation\n").unwrap();
    let before = get_bytes(
        address,
        &format!("/api/source/before?token={token}&file=0&scope=resume"),
    );
    let (_, before_body) = split_response(&before);
    assert_eq!(before_body, b"value = 1\n");
    let after = get_bytes(
        address,
        &format!("/api/source/after?token={token}&file=0&scope=resume"),
    );
    let (_, after_body) = split_response(&after);
    assert_eq!(after_body, b"value = 2\n");

    let full_before = get_bytes(
        address,
        &format!("/api/source/before?token={token}&file=0&scope=full"),
    );
    let (_, full_before_body) = split_response(&full_before);
    assert_eq!(full_before_body, b"value = 0\n");
    let full_after = get_bytes(
        address,
        &format!("/api/source/after?token={token}&file=0&scope=full"),
    );
    let (_, full_after_body) = split_response(&full_after);
    assert_eq!(full_after_body, b"value = 2\n");
}

#[cfg(unix)]
#[test]
fn offline_demo_runs_outside_a_checkout_and_cleans_up_after_ctrl_c() {
    let sandbox = tempfile::tempdir().unwrap();
    let runtime = sandbox.path().join("runtime");
    let non_git = sandbox.path().join("non-git");
    fs::create_dir(&runtime).unwrap();
    fs::create_dir(&non_git).unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("demo")
        .arg("--port")
        .arg("0")
        .arg("--no-open")
        .current_dir(&non_git)
        .env("TMPDIR", &runtime)
        .env("GIT_DIR", "/must/not/reach/demo/git")
        .env("GIT_CONFIG_GLOBAL", "/must/not/reach/demo/config")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let stderr = child.0.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let _reader = thread::spawn(move || {
        let mut stderr = BufReader::new(stderr);
        let mut first_line = String::new();
        let result = stderr.read_line(&mut first_line).map(|_| first_line);
        let _ = sender.send(result);
    });
    let first_line = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("offline demo did not print its URL within 30 seconds")
        .unwrap();
    let url = first_line
        .trim_end()
        .strip_prefix("StrataDiff Review Resume Workbench: http://")
        .unwrap();
    let (address, token) = url.split_once("/?token=").unwrap();

    let session = get(address, &format!("/api/session?token={token}"));
    let (_, body) = session.split_once("\r\n\r\n").unwrap();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["kind"], "repository_review");
    assert_eq!(
        payload["resume_delta"]["comparison"],
        "per_file_review_baseline_to_head"
    );
    assert_eq!(payload["resume_delta"]["summary"]["displayable_files"], 1);
    assert_eq!(payload["resume_delta"]["summary"]["needs_review_files"], 1);
    assert_eq!(
        payload["resume_delta"]["entries"][0]["baseline_basis"],
        "reconstructed_review_baseline"
    );
    assert_eq!(
        payload["resume_delta"]["entries"][0]["file"]["line_change_envelope"],
        serde_json::json!({"additions": 1, "deletions": 1})
    );

    let before = get_bytes(
        address,
        &format!("/api/source/before?token={token}&file=0&scope=resume"),
    );
    let (_, before_body) = split_response(&before);
    assert_eq!(before_body, b"title = 'new'\nreviewed = 1\nfollowup = 0\n");
    let after = get_bytes(
        address,
        &format!("/api/source/after?token={token}&file=0&scope=resume"),
    );
    let (_, after_body) = split_response(&after);
    assert_eq!(after_body, b"title = 'new'\nreviewed = 1\nfollowup = 1\n");

    let signal_result = unsafe { libc::kill(child.0.id() as i32, libc::SIGINT) };
    assert_eq!(signal_result, 0);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "offline demo did not stop on Ctrl+C"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "offline demo exited with {status}");
    assert!(!non_git.join(".git").exists());
    assert_eq!(fs::read_dir(&runtime).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn offline_demo_registers_shutdown_before_publishing_readiness() {
    for signal in [libc::SIGTERM, libc::SIGHUP] {
        assert_demo_shutdown_after_readiness(signal);
    }
}

#[cfg(unix)]
fn assert_demo_shutdown_after_readiness(signal: i32) {
    let sandbox = tempfile::tempdir().unwrap();
    let runtime = sandbox.path().join("runtime");
    let non_git = sandbox.path().join("non-git");
    fs::create_dir(&runtime).unwrap();
    fs::create_dir(&non_git).unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("demo")
        .arg("--port")
        .arg("0")
        .arg("--no-open")
        .current_dir(&non_git)
        .env("TMPDIR", &runtime)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let mut stderr = BufReader::new(child.0.stderr.take().unwrap());
    let mut first_line = String::new();
    stderr.read_line(&mut first_line).unwrap();
    assert!(
        first_line.starts_with("StrataDiff Review Resume Workbench: http://"),
        "{first_line}"
    );

    let signal_result = unsafe { libc::kill(child.0.id() as i32, signal) };
    assert_eq!(signal_result, 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "offline demo did not stop after publishing readiness"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "offline demo exited with {status}");
    assert_eq!(fs::read_dir(&runtime).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn offline_demo_bounds_shutdown_with_a_stalled_connection_and_cleans_up_after_signals() {
    for signal in [libc::SIGTERM, libc::SIGHUP] {
        assert_stalled_demo_shutdown_cleans_up(signal);
    }
}

#[cfg(unix)]
fn assert_stalled_demo_shutdown_cleans_up(signal: i32) {
    let sandbox = tempfile::tempdir().unwrap();
    let runtime = sandbox.path().join("runtime");
    let non_git = sandbox.path().join("non-git");
    fs::create_dir(&runtime).unwrap();
    fs::create_dir(&non_git).unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_stratadiff"))
        .arg("demo")
        .arg("--port")
        .arg("0")
        .arg("--no-open")
        .current_dir(&non_git)
        .env("TMPDIR", &runtime)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let mut stderr = BufReader::new(child.0.stderr.take().unwrap());
    let mut first_line = String::new();
    stderr.read_line(&mut first_line).unwrap();
    let url = first_line
        .trim_end()
        .strip_prefix("StrataDiff Review Resume Workbench: http://")
        .unwrap();
    let (address, _) = url.split_once("/?token=").unwrap();

    let address: SocketAddr = address.parse().unwrap();
    let mut stalled = TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
    stalled.write_all(b"GET / HTTP/1.1\r\nHost:").unwrap();

    let signal_result = unsafe { libc::kill(child.0.id() as i32, signal) };
    assert_eq!(signal_result, 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "offline demo did not bound graceful shutdown"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "offline demo exited with {status}");
    assert_eq!(fs::read_dir(&runtime).unwrap().count(), 0);
}

fn get(address: &str, path: &str) -> String {
    get_with_host(address, path, address)
}

fn get_with_host(address: &str, path: &str, host: &str) -> String {
    String::from_utf8(get_bytes_with_host(address, path, host)).unwrap()
}

fn get_bytes(address: &str, path: &str) -> Vec<u8> {
    get_bytes_with_host(address, path, address)
}

fn get_bytes_with_host(address: &str, path: &str, host: &str) -> Vec<u8> {
    let address: SocketAddr = address.parse().unwrap();
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    response
}

fn split_response(response: &[u8]) -> (&[u8], &[u8]) {
    let boundary = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    (&response[..boundary], &response[boundary + 4..])
}
