use std::{
    env,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use axum::{
    body::Body,
    extract::{connect_info::ConnectInfo, OriginalUri, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use bytes::Bytes;
use reqwest::Client;
use serde::Serialize;
use tokio::net::TcpListener;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Clone)]
struct AppState {
    client: Client,
    targets: Arc<Vec<String>>,
    next_target: Arc<AtomicUsize>,
    stats: Arc<Stats>,
    started_at: Instant,
}

#[derive(Default)]
struct Stats {
    total_requests: AtomicU64,
    successful_requests: AtomicU64,
    failed_requests: AtomicU64,
    last_target_index: AtomicUsize,
}

#[derive(Serialize)]
struct StatsResponse {
    total_requests: u64,
    successful_requests: u64,
    failed_requests: u64,
    last_target_index: usize,
    targets: Vec<String>,
    uptime_seconds: u64,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = Config::from_env()?;
    let state = AppState::new(config.targets)?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/admin/stats", get(stats))
        .fallback(proxy)
        .with_state(state);

    let listener = TcpListener::bind(config.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.listen_addr))?;

    info!("sentinel-proxy listening on {}", config.listen_addr);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("server failed")?;

    Ok(())
}

struct Config {
    listen_addr: SocketAddr,
    targets: Vec<String>,
}

impl Config {
    fn from_env() -> Result<Self> {
        let port = env::var("PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(3000);

        let targets = env::var("BACKEND_TARGETS")
            .unwrap_or_else(|_| "https://jsonplaceholder.typicode.com".to_string())
            .split(',')
            .map(str::trim)
            .filter(|target| !target.is_empty())
            .map(trim_trailing_slash)
            .collect::<Vec<_>>();

        if targets.is_empty() {
            return Err(anyhow!(
                "BACKEND_TARGETS must include at least one upstream URL"
            ));
        }

        Ok(Self {
            listen_addr: SocketAddr::from(([0, 0, 0, 0], port)),
            targets,
        })
    }
}

impl AppState {
    fn new(targets: Vec<String>) -> Result<Self> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            client,
            targets: Arc::new(targets),
            next_target: Arc::new(AtomicUsize::new(0)),
            stats: Arc::new(Stats::default()),
            started_at: Instant::now(),
        })
    }

    fn choose_target(&self) -> (usize, String) {
        let index = self.next_target.fetch_add(1, Ordering::Relaxed) % self.targets.len();
        self.stats.last_target_index.store(index, Ordering::Relaxed);
        (index, self.targets[index].clone())
    }
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn stats(State(state): State<AppState>) -> Json<StatsResponse> {
    Json(StatsResponse {
        total_requests: state.stats.total_requests.load(Ordering::Relaxed),
        successful_requests: state.stats.successful_requests.load(Ordering::Relaxed),
        failed_requests: state.stats.failed_requests.load(Ordering::Relaxed),
        last_target_index: state.stats.last_target_index.load(Ordering::Relaxed),
        targets: state.targets.as_ref().clone(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
    })
}

async fn proxy(
    State(state): State<AppState>,
    ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    mut headers: HeaderMap,
    body: Bytes,
) -> Response {
    state.stats.total_requests.fetch_add(1, Ordering::Relaxed);

    let (_target_index, target) = state.choose_target();
    let upstream_url = match build_upstream_url(&target, &original_uri) {
        Ok(url) => url,
        Err(err) => {
            state.stats.failed_requests.fetch_add(1, Ordering::Relaxed);
            warn!(error = %err, "invalid upstream URL");
            return (StatusCode::BAD_GATEWAY, err.to_string()).into_response();
        }
    };

    info!(%method, %upstream_url, "proxying request");

    apply_forwarding_headers(&mut headers, &client_addr);

    let mut request = state.client.request(method, upstream_url).body(body);
    for (name, value) in headers.iter() {
        if should_forward_request_header(name) {
            request = request.header(name, value);
        }
    }

    match request.send().await {
        Ok(upstream_response) => {
            let status = upstream_response.status();
            let mut response_builder = Response::builder().status(status);

            for (name, value) in upstream_response.headers() {
                if should_forward_response_header(name) {
                    response_builder = response_builder.header(name, value);
                }
            }

            response_builder = response_builder.header("x-sentinel-upstream", target.as_str());

            match upstream_response.bytes().await {
                Ok(bytes) => {
                    state
                        .stats
                        .successful_requests
                        .fetch_add(1, Ordering::Relaxed);
                    response_builder
                        .body(Body::from(bytes))
                        .unwrap_or_else(|err| internal_error(&state, err))
                }
                Err(err) => {
                    state.stats.failed_requests.fetch_add(1, Ordering::Relaxed);
                    error!(error = %err, "failed to read upstream response");
                    (StatusCode::BAD_GATEWAY, "failed to read upstream response").into_response()
                }
            }
        }
        Err(err) => {
            state.stats.failed_requests.fetch_add(1, Ordering::Relaxed);
            error!(error = %err, "upstream request failed");
            (StatusCode::BAD_GATEWAY, "upstream request failed").into_response()
        }
    }
}

fn build_upstream_url(target: &str, original_uri: &Uri) -> Result<String> {
    let path_and_query = original_uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");

    Ok(format!("{target}{path_and_query}"))
}

fn apply_forwarding_headers(headers: &mut HeaderMap, client_addr: &SocketAddr) {
    append_header_value(headers, "x-forwarded-for", &client_addr.ip().to_string());

    if !headers.contains_key("x-forwarded-proto") {
        headers.insert("x-forwarded-proto", HeaderValue::from_static("http"));
    }

    if !headers.contains_key("x-forwarded-host") {
        if let Some(host) = headers.get("host").cloned() {
            headers.insert("x-forwarded-host", host);
        }
    }
}

fn append_header_value(headers: &mut HeaderMap, key: &'static str, value: &str) {
    let next_value = match headers.get(key).and_then(|current| current.to_str().ok()) {
        Some(current) if !current.trim().is_empty() => format!("{current}, {value}"),
        _ => value.to_string(),
    };

    if let Ok(header_value) = HeaderValue::from_str(&next_value) {
        headers.insert(key, header_value);
    }
}

fn should_forward_request_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "host"
            | "connection"
            | "content-length"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn should_forward_response_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "connection"
            | "content-length"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn internal_error(state: &AppState, err: http::Error) -> Response {
    state.stats.failed_requests.fetch_add(1, Ordering::Relaxed);
    error!(error = %err, "failed to build response");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "failed to build proxy response",
    )
        .into_response()
}

fn trim_trailing_slash(value: &str) -> String {
    value.trim_end_matches('/').to_string()
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        error!(error = %err, "failed to listen for shutdown signal");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_header_value_sets_new_header() {
        let mut headers = HeaderMap::new();

        append_header_value(&mut headers, "x-forwarded-for", "127.0.0.1");

        assert_eq!(headers["x-forwarded-for"], "127.0.0.1");
    }

    #[test]
    fn append_header_value_appends_to_existing_chain() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("10.0.0.1"));

        append_header_value(&mut headers, "x-forwarded-for", "127.0.0.1");

        assert_eq!(headers["x-forwarded-for"], "10.0.0.1, 127.0.0.1");
    }
}
