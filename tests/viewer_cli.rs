use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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
