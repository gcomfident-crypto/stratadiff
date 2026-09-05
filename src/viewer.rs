use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    process::Command,
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Query, State},
    http::{
        HeaderMap, HeaderValue, StatusCode, Uri,
        header::{
            CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST,
            REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
        },
    },
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use stratadiff::{
    DiffReport, VerificationLimits,
    coverage::{MAX_REVIEW_COVERAGE_BYTES, ReviewCoveragePassport},
    review::{
        CheckpointCarryBasis, CheckpointState, RepositoryReview, ReviewBaselineReconstruction,
        ReviewDeltaBaselineBasis, ReviewDeltaFile, ReviewDeltaSource, ReviewFileSources,
        load_review_delta_file_sources, load_review_file_sources, regenerate_review_file_report,
        review_git_resume_delta,
    },
};
use tokio::{net::TcpListener, runtime::Builder};

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct WebAssets;

#[derive(Clone)]
struct ViewerState {
    content: Arc<ViewerContent>,
    expected_host: String,
    token: String,
}

enum ViewerContent {
    File(FileSession),
    Repository(Box<RepositorySession>),
    ReviewCoverage(ReviewCoverageSession),
}

#[derive(Clone)]
struct FileSession {
    after: Bytes,
    before: Bytes,
    session_json: Bytes,
}

struct ReviewCoverageSession {
    passport_json: Bytes,
    session_json: Bytes,
}

struct RepositorySession {
    cache: tokio::sync::Mutex<Option<Arc<CachedReviewFile>>>,
    repository: PathBuf,
    queue: Vec<ReviewDeltaFile>,
    review: RepositoryReview,
    session_json: Bytes,
}

struct CachedReviewFile {
    evidence: Option<FileSession>,
    index: usize,
    scope: ReviewScope,
    sources: ReviewFileSources,
}

#[derive(Serialize)]
struct ViewerVerification {
    verified: bool,
    message: &'static str,
}

#[derive(Serialize)]
struct RepositoryAssessment {
    status: &'static str,
    basis: &'static str,
    message: &'static str,
}

#[derive(Serialize)]
struct RepositoryContext {
    file_index: usize,
    scope: ReviewScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    checkpoint_state: Option<CheckpointState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checkpoint_match_basis: Option<CheckpointCarryBasis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_basis: Option<ReviewDeltaBaselineBasis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    before_source: Option<ReviewDeltaSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after_source: Option<ReviewDeltaSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_reconstruction: Option<ReviewBaselineReconstruction>,
}

#[derive(Deserialize)]
struct SessionQuery {
    token: String,
    file: Option<usize>,
    scope: Option<ReviewScope>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReviewScope {
    Resume,
    Full,
}

pub fn serve(
    report: DiffReport,
    before: Vec<u8>,
    after: Vec<u8>,
    port: u16,
    open_browser: bool,
) -> Result<()> {
    let session = file_session(report, before, after, None)?;
    serve_content(
        ViewerContent::File(session),
        "StrataDiff Evidence Workbench",
        port,
        open_browser,
    )
}

pub fn serve_review(
    review: RepositoryReview,
    repository: PathBuf,
    port: u16,
    open_browser: bool,
) -> Result<()> {
    let repository = std::fs::canonicalize(&repository)
        .with_context(|| format!("failed to resolve repository {}", repository.display()))?;
    let checkpoint = review
        .checkpoint
        .as_ref()
        .context("repository review workbench requires a checkpoint")?;
    let base_changed = checkpoint.base_commit != review.base_commit;
    let resume_delta = review_git_resume_delta(&repository, &review)?;
    let queue = resume_delta.entries.clone();
    let mut session_json = Vec::new();
    session_json.extend_from_slice(br#"{"kind":"repository_review","review":"#);
    serde_json::to_writer(&mut session_json, &review)?;
    session_json.extend_from_slice(br#", "resume_delta":"#);
    serde_json::to_writer(&mut session_json, &resume_delta)?;
    session_json.extend_from_slice(br#", "assessment":"#);
    serde_json::to_writer(
        &mut session_json,
        &RepositoryAssessment {
            status: "producer_attested",
            basis: if base_changed {
                "exact_git_change_identity_or_noninteracting_four_way_byte_replay"
            } else {
                "exact_git_change_identity"
            },
            message: if base_changed {
                "The merge base changed. Each queue row is either an exact reconstructed review-baseline delta or an explicitly marked conservative fallback. Upstream-only files are excluded. This is not proof of review, semantic safety, or approval."
            } else {
                "Checkpoint carry-forward is an exact Git identity comparison. It is not proof of review, semantic safety, or approval."
            },
        },
    )?;
    session_json.push(b'}');
    let report_limit = VerificationLimits::default().max_report_bytes;
    ensure!(
        session_json.len() <= report_limit,
        "repository viewer session bytes limit exceeded: observed {}, limit {report_limit}",
        session_json.len()
    );

    serve_content(
        ViewerContent::Repository(Box::new(RepositorySession {
            cache: tokio::sync::Mutex::new(None),
            repository,
            queue,
            review,
            session_json: Bytes::from(session_json),
        })),
        "StrataDiff Review Resume Workbench",
        port,
        open_browser,
    )
}

pub fn serve_review_coverage(
    passport: ReviewCoveragePassport,
    passport_json: Vec<u8>,
    port: u16,
    open_browser: bool,
) -> Result<()> {
    ensure!(
        passport_json.len() <= MAX_REVIEW_COVERAGE_BYTES,
        "review coverage passport bytes limit exceeded: observed {}, limit {MAX_REVIEW_COVERAGE_BYTES}",
        passport_json.len()
    );
    let mut session_json = Vec::new();
    session_json.extend_from_slice(br#"{"kind":"review_coverage_passport","passport":"#);
    serde_json::to_writer(&mut session_json, &passport)?;
    session_json.extend_from_slice(br#", "verification":"#);
    serde_json::to_writer(
        &mut session_json,
        &ViewerVerification {
            verified: true,
            message: "The receiver signature and embedded coverage decision were independently recomputed from the exact offline Git objects before this viewer opened.",
        },
    )?;
    session_json.push(b'}');

    serve_content(
        ViewerContent::ReviewCoverage(ReviewCoverageSession {
            passport_json: Bytes::from(passport_json),
            session_json: Bytes::from(session_json),
        }),
        "StrataDiff Review Coverage Passport",
        port,
        open_browser,
    )
}

fn file_session(
    report: DiffReport,
    before: Vec<u8>,
    after: Vec<u8>,
    repository_context: Option<RepositoryContext>,
) -> Result<FileSession> {
    let mut session_json = Vec::new();
    session_json.extend_from_slice(br#"{"kind":"file_diff","report":"#);
    let report_start = session_json.len();
    serde_json::to_writer(&mut session_json, &report)?;
    let report_size = session_json.len() - report_start;
    let report_limit = VerificationLimits::default().max_report_bytes;
    ensure!(
        report_size <= report_limit,
        "generated report bytes limit exceeded: observed {report_size}, limit {report_limit}"
    );
    session_json.extend_from_slice(br#", "verification":"#);
    serde_json::to_writer(
        &mut session_json,
        &ViewerVerification {
            verified: true,
            message: "Single-file patch reconstruction, parser manifest, relations, ambiguities, changes, and summary independently verified.",
        },
    )?;
    if let Some(context) = repository_context {
        session_json.extend_from_slice(br#", "repository_context":"#);
        serde_json::to_writer(&mut session_json, &context)?;
    }
    session_json.push(b'}');
    drop(report);

    Ok(FileSession {
        after: Bytes::from(after),
        before: Bytes::from(before),
        session_json: Bytes::from(session_json),
    })
}

fn serve_content(
    content: ViewerContent,
    label: &'static str,
    port: u16,
    open_browser: bool,
) -> Result<()> {
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
            content: Arc::new(content),
            expected_host: address.to_string(),
            token,
        };
        let app = Router::new()
            .route("/api/session", get(session))
            .route("/api/passport", get(passport_download))
            .route("/api/source/before", get(before_source))
            .route("/api/source/after", get(after_source))
            .fallback(get(static_asset))
            .with_state(state)
            .layer(middleware::from_fn(security_headers));

        eprintln!("{label}: {url}");
        eprintln!("Press Ctrl+C to stop the local server.");
        if open_browser && let Err(error) = launch_browser(&url) {
            eprintln!("Could not open the browser automatically: {error:#}");
            eprintln!("Open this URL manually: {url}");
        }

        let shutdown = async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                eprintln!("Could not listen for Ctrl+C; stopping the local server: {error}");
            }
        };
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
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

    let session_json = match state.content.as_ref() {
        ViewerContent::File(file) => {
            if query.file.is_some() {
                return plain_response(StatusCode::NOT_FOUND, "Not found");
            }
            file.session_json.clone()
        }
        ViewerContent::Repository(repository) => match query.file {
            Some(index) => {
                let scope = query.scope.unwrap_or(ReviewScope::Resume);
                if !review_file_index_is_valid(repository, scope, index) {
                    return plain_response(StatusCode::NOT_FOUND, "Not found");
                }
                match cached_review_file(repository, scope, index).await {
                    Ok(file) => match &file.evidence {
                        Some(evidence) => evidence.session_json.clone(),
                        None => {
                            return plain_response(
                                StatusCode::UNPROCESSABLE_ENTITY,
                                "This file has source snapshots but no independently verified structural report",
                            );
                        }
                    },
                    Err(_) => {
                        return plain_response(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            "Could not materialize this repository file from its recorded Git objects",
                        );
                    }
                }
            }
            None => repository.session_json.clone(),
        },
        ViewerContent::ReviewCoverage(coverage) => {
            if query.file.is_some() {
                return plain_response(StatusCode::NOT_FOUND, "Not found");
            }
            coverage.session_json.clone()
        }
    };
    response(
        StatusCode::OK,
        "application/json; charset=utf-8",
        Body::from(session_json),
    )
}

async fn passport_download(
    State(state): State<ViewerState>,
    headers: HeaderMap,
    Query(query): Query<SessionQuery>,
) -> Response {
    if !request_host_is_valid(&headers, &state)
        || !tokens_match(&query.token, &state.token)
        || query.file.is_some()
    {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    }
    let ViewerContent::ReviewCoverage(coverage) = state.content.as_ref() else {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .header(
            CONTENT_DISPOSITION,
            "attachment; filename=review-coverage-passport.json",
        )
        .body(Body::from(coverage.passport_json.clone()))
        .expect("static passport response metadata is valid")
}

async fn before_source(
    State(state): State<ViewerState>,
    headers: HeaderMap,
    Query(query): Query<SessionQuery>,
) -> Response {
    source_response(&state, &headers, &query, SourceSide::Before).await
}

async fn after_source(
    State(state): State<ViewerState>,
    headers: HeaderMap,
    Query(query): Query<SessionQuery>,
) -> Response {
    source_response(&state, &headers, &query, SourceSide::After).await
}

#[derive(Clone, Copy)]
enum SourceSide {
    Before,
    After,
}

async fn source_response(
    state: &ViewerState,
    headers: &HeaderMap,
    query: &SessionQuery,
    side: SourceSide,
) -> Response {
    if !request_host_is_valid(headers, state) || !tokens_match(&query.token, &state.token) {
        return plain_response(StatusCode::NOT_FOUND, "Not found");
    }
    let bytes = match state.content.as_ref() {
        ViewerContent::File(file) => {
            if query.file.is_some() {
                return plain_response(StatusCode::NOT_FOUND, "Not found");
            }
            match side {
                SourceSide::Before => file.before.clone(),
                SourceSide::After => file.after.clone(),
            }
        }
        ViewerContent::Repository(repository) => {
            let Some(index) = query.file else {
                return plain_response(StatusCode::NOT_FOUND, "Not found");
            };
            let scope = query.scope.unwrap_or(ReviewScope::Resume);
            if !review_file_index_is_valid(repository, scope, index) {
                return plain_response(StatusCode::NOT_FOUND, "Not found");
            }
            let file = match cached_review_file(repository, scope, index).await {
                Ok(file) => file,
                Err(_) => {
                    return plain_response(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "Could not materialize this repository file from its recorded Git objects",
                    );
                }
            };
            match side {
                SourceSide::Before => Bytes::copy_from_slice(&file.sources.before),
                SourceSide::After => Bytes::copy_from_slice(&file.sources.after),
            }
        }
        ViewerContent::ReviewCoverage(_) => {
            return plain_response(StatusCode::NOT_FOUND, "Not found");
        }
    };
    response(
        StatusCode::OK,
        "application/octet-stream",
        Body::from(bytes),
    )
}

fn review_file_index_is_valid(
    repository: &RepositorySession,
    scope: ReviewScope,
    index: usize,
) -> bool {
    match scope {
        ReviewScope::Resume => index < repository.queue.len(),
        ReviewScope::Full => index < repository.review.files.len(),
    }
}

async fn cached_review_file(
    repository: &RepositorySession,
    scope: ReviewScope,
    index: usize,
) -> Result<Arc<CachedReviewFile>> {
    let mut cache = repository.cache.lock().await;
    if let Some(cached) = cache.as_ref()
        && cached.index == index
        && cached.scope == scope
    {
        return Ok(Arc::clone(cached));
    }

    let (file, resume_entry) = match scope {
        ReviewScope::Resume => {
            let entry = repository
                .queue
                .get(index)
                .cloned()
                .context("repository review file index is out of range")?;
            (entry.file.clone(), Some(entry))
        }
        ReviewScope::Full => (
            repository
                .review
                .files
                .get(index)
                .cloned()
                .context("repository review file index is out of range")?,
            None,
        ),
    };
    let repository_path = repository.repository.clone();
    let cached = tokio::task::spawn_blocking(move || -> Result<CachedReviewFile> {
        let sources = match &resume_entry {
            Some(entry) => load_review_delta_file_sources(&repository_path, entry)?,
            None => load_review_file_sources(&repository_path, &file)?,
        };
        let evidence = if file.evidence.is_some() {
            let report = regenerate_review_file_report(&file, &sources)?;
            Some(file_session(
                report,
                sources.before.clone(),
                sources.after.clone(),
                Some(RepositoryContext {
                    file_index: index,
                    scope,
                    checkpoint_state: file.checkpoint_state,
                    checkpoint_match_basis: file.checkpoint_match_basis,
                    baseline_basis: resume_entry.as_ref().map(|entry| entry.baseline_basis),
                    before_source: resume_entry
                        .as_ref()
                        .map(|entry| entry.before_source.clone()),
                    after_source: resume_entry
                        .as_ref()
                        .map(|entry| entry.after_source.clone()),
                    baseline_reconstruction: resume_entry
                        .as_ref()
                        .and_then(|entry| entry.baseline_reconstruction.clone()),
                }),
            )?)
        } else {
            None
        };
        Ok(CachedReviewFile {
            evidence,
            index,
            scope,
            sources,
        })
    })
    .await
    .context("repository file materialization task failed")??;
    let cached = Arc::new(cached);
    *cache = Some(Arc::clone(&cached));
    Ok(cached)
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
        Some("txt") => "text/plain; charset=utf-8",
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
    use super::{WebAssets, content_type, session_token, tokens_match};

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
        assert_eq!(
            content_type("THIRD_PARTY_NOTICES.txt"),
            "text/plain; charset=utf-8"
        );
        assert_eq!(content_type("asset.bin"), "application/octet-stream");
    }

    #[test]
    fn third_party_notices_are_embedded() {
        let asset = <WebAssets as rust_embed::RustEmbed>::get("THIRD_PARTY_NOTICES.txt")
            .expect("production dependency notices must be embedded");
        let notices = std::str::from_utf8(asset.data.as_ref()).unwrap();
        assert!(notices.starts_with("StrataDiff Evidence Workbench Third-Party Notices\n"));
        assert!(notices.contains("\nreact@"));
        assert!(notices.contains("\nlucide-react@"));
    }
}
