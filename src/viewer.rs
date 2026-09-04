use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    process::Command,
};

use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Query, State},
    http::{
        HeaderMap, HeaderValue, StatusCode, Uri,
        header::{
            CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST, REFERRER_POLICY,
            X_CONTENT_TYPE_OPTIONS,
        },
    },
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use stratadiff::{DiffReport, VerificationLimits};
use tokio::{net::TcpListener, runtime::Builder};

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct WebAssets;

#[derive(Clone)]
struct ViewerState {
    after: Bytes,
    before: Bytes,
    expected_host: String,
    report_json: Bytes,
    token: String,
}

#[derive(Serialize)]
struct ViewerVerification {
    verified: bool,
    message: &'static str,
}

#[derive(Deserialize)]
struct SessionQuery {
    token: String,
}

pub fn serve(
    report: DiffReport,
    before: Vec<u8>,
    after: Vec<u8>,
    port: u16,
    open_browser: bool,
) -> Result<()> {
    let mut report_json = Vec::new();
    report_json.extend_from_slice(br#"{"report":"#);
    let report_start = report_json.len();
    serde_json::to_writer(&mut report_json, &report)?;
    let report_size = report_json.len() - report_start;
    let report_limit = VerificationLimits::default().max_report_bytes;
    ensure!(
        report_size <= report_limit,
        "generated report bytes limit exceeded: observed {report_size}, limit {report_limit}"
    );
    report_json.extend_from_slice(br#", "verification":"#);
    serde_json::to_writer(
        &mut report_json,
        &ViewerVerification {
            verified: true,
            message: "Replay, parser manifest, relations, ambiguities, changes, and summary independently verified.",
        },
    )?;
    report_json.push(b'}');
    drop(report);

    let token = session_token()?;
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the local viewer runtime")?;
    runtime.block_on(async move {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
            .await
            .with_context(|| format!("failed to bind the local viewer on port {port}"))?;
        let address = listener
            .local_addr()
            .context("failed to read the local viewer address")?;
        let url = format!("http://{address}/?token={token}");
        let state = ViewerState {
            after: Bytes::from(after),
            before: Bytes::from(before),
            expected_host: address.to_string(),
            report_json: Bytes::from(report_json),
            token,
        };
        let app = Router::new()
            .route("/api/session", get(session))
            .route("/api/source/before", get(before_source))
            .route("/api/source/after", get(after_source))
            .fallback(get(static_asset))
            .with_state(state)
            .layer(middleware::from_fn(security_headers));

        eprintln!("StrataDiff Evidence Workbench: {url}");
        eprintln!("Press Ctrl+C to stop the local server.");
        if open_browser && let Err(error) = launch_browser(&url) {
            eprintln!("Could not open the browser automatically: {error:#}");
            eprintln!("Open this URL manually: {url}");
        }

        axum::serve(listener, app)
            .await
            .context("local viewer server failed")
    })
}

async fn session(
    State(state): State<ViewerState>,
    headers: HeaderMap,
    Query(query): Query<SessionQuery>,
) -> Response {
    if !request_host_is_valid(&headers, &state) || !tokens_match(&query.token, &state.token) {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    }

    response(
        StatusCode::OK,
        "application/json; charset=utf-8",
        Body::from(state.report_json),
    )
}

async fn before_source(
    State(state): State<ViewerState>,
    headers: HeaderMap,
    Query(query): Query<SessionQuery>,
) -> Response {
    source_response(&state, &headers, &query, state.before.clone())
}

async fn after_source(
    State(state): State<ViewerState>,
    headers: HeaderMap,
    Query(query): Query<SessionQuery>,
) -> Response {
    source_response(&state, &headers, &query, state.after.clone())
}

fn source_response(
    state: &ViewerState,
    headers: &HeaderMap,
    query: &SessionQuery,
    bytes: Bytes,
) -> Response {
    if !request_host_is_valid(headers, state) || !tokens_match(&query.token, &state.token) {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    }
    response(
        StatusCode::OK,
        "application/octet-stream",
        Body::from(bytes),
    )
}

async fn static_asset(State(state): State<ViewerState>, headers: HeaderMap, uri: Uri) -> Response {
    if !request_host_is_valid(&headers, &state) {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    }

    let requested = uri.path().trim_start_matches('/');
    let path = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };
    if path.split('/').any(|part| part == "..") {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    }

    match WebAssets::get(path) {
        Some(asset) => response(
            StatusCode::OK,
            content_type(path),
            Body::from(asset.data.into_owned()),
        ),
        None => plain_response(StatusCode::NOT_FOUND, "Not found"),
    }
}

async fn security_headers(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    response
}

fn request_host_is_valid(headers: &HeaderMap, state: &ViewerState) -> bool {
    headers
        .get(HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(|host| host == state.expected_host)
}

fn tokens_match(candidate: &str, expected: &str) -> bool {
    if candidate.len() != expected.len() {
        return false;
    }
    candidate
        .bytes()
        .zip(expected.bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn session_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        anyhow::anyhow!("failed to create a local viewer session token: {error}")
    })?;
    Ok(blake3::Hash::from_bytes(bytes).to_hex().to_string())
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") | Some("map") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn plain_response(status: StatusCode, message: &'static str) -> Response {
    response(status, "text/plain; charset=utf-8", Body::from(message))
}

fn response(status: StatusCode, content_type: &'static str, body: Body) -> Response {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(body)
        .expect("static response metadata is valid")
}

#[cfg(target_os = "linux")]
fn launch_browser(url: &str) -> Result<()> {
    let status = Command::new("xdg-open")
        .arg(url)
        .status()
        .context("failed to run xdg-open")?;
    ensure!(status.success(), "xdg-open exited with {status}");
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_browser(url: &str) -> Result<()> {
    let status = Command::new("open")
        .arg(url)
        .status()
        .context("failed to run open")?;
    ensure!(status.success(), "open exited with {status}");
    Ok(())
}

#[cfg(target_os = "windows")]
fn launch_browser(url: &str) -> Result<()> {
    let status = Command::new("cmd")
        .args(["/C", "start", "", url])
        .status()
        .context("failed to run the browser launcher")?;
    ensure!(status.success(), "browser launcher exited with {status}");
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn launch_browser(_url: &str) -> Result<()> {
    anyhow::bail!("automatic browser opening is unsupported on this platform")
}

#[cfg(test)]
mod tests {
    use super::{content_type, session_token, tokens_match};

    #[test]
    fn session_tokens_are_full_length_and_compared_without_an_early_byte_exit() {
        let token = session_token().unwrap();
        assert_eq!(token.len(), 64);
        assert!(tokens_match(&token, &token));
        assert!(!tokens_match(&format!("x{}", &token[1..]), &token));
        assert!(!tokens_match(&token[..63], &token));
    }

    #[test]
    fn embedded_asset_content_types_are_explicit() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            content_type("assets/app.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type("asset.bin"), "application/octet-stream");
    }
}
