//! Axum routes mirroring legacy Tauri invoke commands.

use std::collections::HashSet;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Instant;

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use teshi_core::{BddFeature, BddProject, FeatureRenderPayload, StepIndex};
use teshi_web_protocol::{
    Channel, ClientHello, ClientMessage, ErrorCode, HostedCapability, ProtocolError,
    Request as ProtocolRequest, ServerMessage, CONTROL_PROTOCOL_VERSION, PREVIEW_PROTOCOL_VERSION,
};

use crate::session::{Role, SessionStore};
use teshi_engine::{
    check_project_switch_allowed, confirm_locator, default_api_service_script, delete_profile,
    dispatch_cases, get_active_step, get_pending_locator, get_profile_public, get_project_root,
    get_recent_projects, highlight_locator, list_dir, list_profiles, list_runnable_scenarios,
    load_llm_config_public, load_project_settings, open_project, reject_locator, render_feature,
    resize_terminal, save_profile, save_stored_llm_config, send_api_command, set_active_id,
    spawn_terminal, start_browser_sidecar, step_binding_statuses, stop_browser_sidecar,
    sync_active_step, teardown_runtime, unbind_step, write_terminal, ActiveStep, ApiStyle,
    BrowserError, BrowserMode, BrowserStartResult, DirEntry, DispatchCase, LlmConfigPublic,
    LlmConfigWrite, ModelProfile, ModelProfileList, ModelProfilePublic, PendingLocator,
    ProjectSettings, RuntimeEvent, StepBinding, StepBindingStatus, TeshiEngine, PROVIDER_OPENAI,
};
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

type SharedRuntime = Arc<TeshiEngine>;

/// Shared state with idle tracking and session store for the daemon.
#[derive(Clone)]
struct DaemonState {
    rt: SharedRuntime,
    sessions: SessionStore,
    active_ws: Arc<AtomicUsize>,
    last_request: Arc<StdMutex<Instant>>,
    shutdown_token: CancellationToken,
    active_control_sessions: Arc<StdMutex<HashSet<String>>>,
    hosted_session_gate: Arc<tokio::sync::RwLock<()>>,
}

impl DaemonState {
    fn touch(&self) {
        if let Ok(mut t) = self.last_request.lock() {
            *t = Instant::now();
        }
    }
}

fn browser_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        // Preserve the daemon's existing cross-origin API support. LLM config
        // mutations are rejected separately by the explicit origin middleware.
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers(Any)
}

async fn same_origin_only(request: Request, next: Next) -> Response {
    let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        // Non-browser clients do not normally send Origin and remain supported.
        return next.run(request).await;
    };
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    // Direct daemon traffic is HTTP. A TLS reverse proxy can report the
    // externally visible scheme so HTTPS deployments remain same-origin.
    let scheme = request
        .headers()
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .unwrap_or("http");
    let prefix = format!("{scheme}://");
    let origin_host = origin
        .strip_prefix(&prefix)
        .map(|value| value.trim_end_matches('/'));

    if host.is_some_and(|host| {
        origin_host.is_some_and(|origin_host| origin_host.eq_ignore_ascii_case(host))
    }) {
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

/// Binds `addr` and serves the daemon APIs. `dist` is retained only for an
/// explicitly requested local static-file diagnostic route; production `teshi
/// web` opens the hosted Pages UI instead.
/// Returns when the server shuts down (via signal, idle timeout, or API call).
pub async fn run_server(
    addr: SocketAddr,
    rt: SharedRuntime,
    dist: PathBuf,
    project_root: Option<std::path::PathBuf>,
) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    run_server_with_listener(listener, rt, dist, project_root).await
}

/// Serve using a listener that has already been bound by the daemon launcher.
///
/// Binding before writing the project manifest is important for the hosted
/// launcher: a port of `0` is resolved by the OS and the manifest must expose
/// that actual bound port, not a probe that can race another process.
pub async fn run_server_with_listener(
    listener: tokio::net::TcpListener,
    rt: SharedRuntime,
    dist: PathBuf,
    project_root: Option<std::path::PathBuf>,
) -> Result<()> {
    let shutdown_token = CancellationToken::new();
    let sessions = SessionStore::new();
    let state = DaemonState {
        rt,
        sessions: sessions.clone(),
        active_ws: Arc::new(AtomicUsize::new(0)),
        last_request: Arc::new(StdMutex::new(Instant::now())),
        shutdown_token: shutdown_token.clone(),
        active_control_sessions: Arc::new(StdMutex::new(HashSet::new())),
        hosted_session_gate: Arc::new(tokio::sync::RwLock::new(())),
    };

    let cors = browser_cors_layer();

    // Session bootstrap remains available to local clients only. Remote clients
    // must use a token minted by a process on the daemon host.
    let session_routes = Router::new()
        .route("/api/v1/sessions", post(api_create_session))
        .route("/api/v1/sessions/{token}", get(api_get_session))
        .route("/api/v1/sessions/{token}", delete(api_delete_session))
        .route_layer(middleware::from_fn(loopback_only))
        .route_layer(middleware::from_fn(same_origin_only));

    let llm_mutation_routes = Router::new()
        .route("/api/v1/llm/config", put(api_put_llm_config))
        .route("/api/v1/llm/profiles", put(api_put_llm_profile))
        .route("/api/v1/llm/profiles/{id}", delete(api_delete_llm_profile))
        .route(
            "/api/v1/llm/profiles/{id}/activate",
            post(api_activate_llm_profile),
        )
        .route_layer(middleware::from_fn(same_origin_only));

    let preview_routes = Router::new()
        .route("/api/v1/browser/stream", get(browser_stream_ws))
        .route("/api/v1/browser/sessions", get(api_browser_sessions))
        .route(
            "/api/v1/browser/activate-tab",
            post(api_browser_activate_tab),
        )
        .route_layer(middleware::from_fn(same_origin_only));

    // ── Protected routes (checked by auth middleware) ─────────────────────────
    let protected_routes = Router::new()
        .route("/api/v1/events", get(events_ws))
        .route("/api/v1/projects/open", post(api_open_project))
        .route("/api/v1/projects/teardown", post(api_teardown))
        .route("/api/v1/projects/switch-allowed", get(api_switch_allowed))
        .route("/api/v1/settings/recent", get(api_recent))
        .route("/api/v1/fs/list", get(api_list_dir))
        .route("/api/v1/gherkin/render", post(api_render_feature))
        .route("/api/v1/gherkin/scenarios", get(api_gherkin_scenarios))
        .route("/api/v1/api/exchange", post(api_get_exchange))
        .route("/api/v1/locator/sync-step", post(api_sync_step))
        .route("/api/v1/locator/active-step", get(api_active_step))
        .route("/api/v1/locator/pending", get(api_pending_locator))
        .route("/api/v1/steps/statuses", get(api_step_statuses))
        .route("/api/v1/steps/unbind", post(api_unbind_step))
        .route("/api/v1/settings/project", get(api_project_settings))
        .route("/api/v1/llm/config", get(api_get_llm_config))
        .route("/api/v1/llm/profiles", get(api_list_llm_profiles))
        .route("/api/v1/llm/profiles/{id}", get(api_get_llm_profile))
        .merge(llm_mutation_routes)
        .route("/api/v1/locator/confirm", post(api_confirm_locator))
        .route("/api/v1/locator/reject", post(api_reject_locator))
        .route("/api/v1/locator/highlight", post(api_highlight_locator))
        .route("/api/v1/browser/start", post(api_browser_start))
        .route("/api/v1/browser/stop", post(api_browser_stop))
        .merge(preview_routes)
        .route("/api/v1/terminal/spawn", post(api_terminal_spawn))
        .route("/api/v1/terminal/stop", post(api_terminal_stop))
        .route("/api/v1/terminal/resize", post(api_terminal_resize))
        .route("/api/v1/terminal/write", post(api_terminal_write))
        .route("/api/v1/fs/read", get(api_read_file))
        .route("/api/v1/daemon/run", post(api_run))
        .route("/api/v1/daemon/shutdown", post(api_daemon_shutdown))
        .route("/api/v1/steps/catalog", get(api_step_catalog))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .route_layer(middleware::from_fn(same_origin_only));

    let app = Router::new()
        .merge(session_routes)
        .merge(
            Router::new()
                .route("/ws/control", get(control_ws))
                .route("/ws/preview", get(preview_ws))
                .route_layer(middleware::from_fn(trusted_hosted_origin_only)),
        )
        .merge(protected_routes)
        .fallback_service(ServeDir::new(dist).append_index_html_on_directories(true))
        .layer(cors)
        .with_state(state.clone());

    // Spawn idle watchdog
    let token = shutdown_token.clone();
    let active_ws = state.active_ws.clone();
    let last_request = state.last_request.clone();
    let idle_project_root = project_root.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            if token.is_cancelled() {
                return;
            }
            let ws_count = active_ws.load(Ordering::Relaxed);
            let idle = last_request
                .lock()
                .map(|t| t.elapsed())
                .unwrap_or(std::time::Duration::ZERO);
            if daemon_should_shutdown_for_idle(ws_count, idle) {
                tracing::info!(
                    "idle watchdog: {:?} since last request, {} active WS — shutting down",
                    idle,
                    ws_count
                );
                token.cancel();
                return;
            }
        }
    });

    // Also listen for Ctrl+C
    let token_ctrlc = shutdown_token.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        token_ctrlc.cancel();
    });

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move { shutdown_token.cancelled().await })
    .await?;

    // Clean up on exit
    if let Some(root) = idle_project_root {
        teshi_engine::remove_daemon_manifest(&root);
    }

    Ok(())
}

// ---- WebSocket ----

const HOSTED_WEB_ORIGINS: [&str; 2] = ["https://teshi.org", "https://teshi-org.github.io"];
const CONTROL_RESPONSE_QUEUE_CAPACITY: usize = 64;
const CONTROL_EVENT_QUEUE_CAPACITY: usize = 64;
const CONTROL_REQUEST_CONCURRENCY: usize = 16;
const PREVIEW_CONTROL_QUEUE_CAPACITY: usize = 8;
const PREVIEW_RECONNECT_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
const DAEMON_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

fn daemon_should_shutdown_for_idle(active_ws: usize, idle: std::time::Duration) -> bool {
    active_ws == 0 && idle > DAEMON_IDLE_TIMEOUT
}

/// Browser WebSocket upgrades are a separate trust gate from token
/// authentication. CORS does not authorize WebSockets, so the daemon checks
/// the exact production Origin before accepting either hosted channel.
async fn trusted_hosted_origin_only(request: Request, next: Next) -> Response {
    let trusted = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_trusted_hosted_origin);
    if trusted {
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

fn is_trusted_hosted_origin(origin: &str) -> bool {
    if HOSTED_WEB_ORIGINS.contains(&origin) {
        return true;
    }
    // Development/test builds may opt into one explicit origin for a local
    // hosted-page harness. Release builds never consult this override and
    // therefore retain the exact production allowlist.
    #[cfg(debug_assertions)]
    if std::env::var("TESHI_DEV_WEB_ORIGIN").ok().as_deref() == Some(origin) {
        return true;
    }
    false
}

async fn control_ws(State(state): State<DaemonState>, ws: WebSocketUpgrade) -> Response {
    state.touch();
    ws.on_upgrade(move |socket| handle_control_socket(state, socket))
}

async fn preview_ws(State(state): State<DaemonState>, ws: WebSocketUpgrade) -> Response {
    state.touch();
    // Preview relay is deliberately separate from the legacy REST stream. The
    // first-message gate is shared with control and will reject unauthenticated
    // clients before any sidecar is contacted.
    ws.on_upgrade(move |socket| handle_preview_socket(state, socket))
}

fn daemon_build_identity() -> teshi_web_protocol::CliBuildIdentity {
    let identity = teshi_core::version::build_identity();
    let channel = serde_json::to_value(identity.channel)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "dev".to_string());
    teshi_web_protocol::CliBuildIdentity {
        semver: identity.semver,
        channel,
        git_sha: identity.git_sha,
        build_timestamp: identity.build_timestamp,
        build_sequence: identity.build_sequence,
    }
}

fn handshake_error(code: ErrorCode, message: impl Into<String>) -> ProtocolError {
    ProtocolError {
        code,
        message: message.into(),
        details: None,
    }
}

async fn send_protocol_error(socket: &mut WebSocket, error: ProtocolError) {
    let message = ServerMessage::Error(error);
    if let Ok(text) = serde_json::to_string(&message) {
        let _ = socket.send(Message::Text(text.into())).await;
    }
}

async fn receive_client_message(socket: &mut WebSocket) -> Result<ClientMessage, ProtocolError> {
    let incoming = tokio::time::timeout(std::time::Duration::from_secs(5), socket.recv())
        .await
        .map_err(|_| handshake_error(ErrorCode::HandshakeTimeout, "first message timed out"))?
        .ok_or_else(|| {
            handshake_error(
                ErrorCode::HandshakeRequired,
                "socket closed before handshake",
            )
        })?
        .map_err(|_| handshake_error(ErrorCode::InvalidMessage, "WebSocket message failed"))?;
    let Message::Text(text) = incoming else {
        return Err(handshake_error(
            ErrorCode::InvalidMessage,
            "first message must be a JSON text hello",
        ));
    };
    serde_json::from_str(text.as_ref())
        .map_err(|_| handshake_error(ErrorCode::InvalidMessage, "invalid client message JSON"))
}

async fn authenticate_hello(
    state: &DaemonState,
    hello: ClientHello,
    expected_channel: Channel,
) -> Result<(String, teshi_web_protocol::CliBuildIdentity), ProtocolError> {
    if hello.channel != expected_channel {
        return Err(handshake_error(
            ErrorCode::WrongChannel,
            "hello channel does not match endpoint",
        ));
    }
    let expected_protocol = match expected_channel {
        Channel::Control => CONTROL_PROTOCOL_VERSION,
        Channel::Preview => PREVIEW_PROTOCOL_VERSION,
    };
    if hello.protocol_version != expected_protocol {
        return Err(handshake_error(
            ErrorCode::IncompatibleProtocol,
            "unsupported WebSocket protocol version",
        ));
    }
    let session = state.sessions.get_session(&hello.token).ok_or_else(|| {
        handshake_error(ErrorCode::InvalidToken, "invalid or expired session token")
    })?;
    if session.role != Role::HostedWebUi {
        return Err(handshake_error(
            ErrorCode::Forbidden,
            "session is not authorized for hosted Web UI",
        ));
    }
    let daemon = daemon_build_identity();
    if !hello.ui.supports(&daemon) {
        return Err(handshake_error(
            ErrorCode::IncompatibleCli,
            "hosted Web UI requires a newer compatible nightly CLI",
        ));
    }
    Ok((hello.token, daemon))
}

fn control_param_string(params: &Value, name: &str) -> Result<String, ProtocolError> {
    params
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            handshake_error(
                ErrorCode::InvalidMessage,
                format!("control parameter '{name}' must be a non-empty string"),
            )
        })
}

fn control_method_capability(method: &str) -> Option<HostedCapability> {
    // Administrative shutdown is intentionally outside the HostedWebUi
    // capability matrix. The dispatcher still returns a stable Forbidden
    // error for an explicit attempt so clients get a deterministic response.
    if method == "runtime.shutdown" {
        return None;
    }
    let prefix = method.split('.').next()?;
    if method == "bdd.run" {
        return Some(HostedCapability::BddRun);
    }
    Some(match prefix {
        "project" => HostedCapability::Project,
        "filesystem" => HostedCapability::Filesystem,
        "bdd" => HostedCapability::Gherkin,
        "api" => HostedCapability::ApiExchange,
        "locator" => HostedCapability::Locator,
        "steps" => HostedCapability::Steps,
        "llm" => HostedCapability::LlmConfig,
        "browser" => HostedCapability::BrowserSessions,
        "terminal" => HostedCapability::Terminal,
        "agent" => HostedCapability::Agent,
        "runtime" => HostedCapability::RuntimeEvents,
        _ => return None,
    })
}

fn claim_control_session(active: &Arc<StdMutex<HashSet<String>>>, token: &str) -> bool {
    let mut active = active.lock().unwrap();
    active.insert(token.to_owned())
}

fn release_control_session(active: &Arc<StdMutex<HashSet<String>>>, token: &str) {
    active.lock().unwrap().remove(token);
}

fn decode_control_params<T: DeserializeOwned>(params: Value) -> Result<T, ProtocolError> {
    serde_json::from_value(params)
        .map_err(|error| handshake_error(ErrorCode::InvalidMessage, error.to_string()))
}

fn encode_control_value<T: Serialize>(value: T) -> Result<Value, ProtocolError> {
    serde_json::to_value(value)
        .map_err(|error| handshake_error(ErrorCode::RequestFailed, error.to_string()))
}

/// The browser sidecar has private connection coordinates (and, for Chrome,
/// a command-channel token) that must never cross the hosted WebSocket
/// boundary. Keep the REST response unchanged for local legacy clients, but
/// expose only the stable startup state to the hosted UI.
fn hosted_browser_start_value(result: BrowserStartResult) -> Value {
    json!({
        "started": true,
        "mode": result.mode,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HostedBrowserSessionList {
    #[serde(default)]
    extension_connected: bool,
    #[serde(default)]
    ambiguous_browser_target: bool,
    #[serde(default)]
    sessions: Vec<HostedBrowserSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HostedBrowserSession {
    #[serde(default)]
    identity: HostedBrowserSessionIdentity,
    #[serde(default)]
    browser: HostedBrowserMetadata,
    #[serde(default)]
    health: String,
    #[serde(default)]
    last_heartbeat_age_ms: u64,
    #[serde(default)]
    windows: Vec<HostedBrowserWindow>,
    #[serde(default)]
    lease: Option<HostedBrowserLease>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HostedBrowserSessionIdentity {
    #[serde(default)]
    extension_instance_id: String,
    #[serde(default)]
    profile_label: Option<String>,
    #[serde(default)]
    extension_version: String,
    #[serde(default)]
    protocol_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HostedBrowserMetadata {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    platform: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HostedBrowserLease {
    #[serde(default)]
    owner_label: String,
    #[serde(default)]
    expires_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HostedBrowserWindow {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    tabs: Vec<HostedBrowserTab>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HostedBrowserTab {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    window_id: Option<i64>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    active: bool,
    #[serde(default = "default_true")]
    debuggable: bool,
}

/// Parse the broker's discovery response through an allowlisted DTO. Unknown
/// fields such as `ws_url`, `extension_frame_ws_url`, `project_root`, and
/// broker credentials are discarded before serialization to the hosted UI.
fn hosted_browser_sessions_value(payload: Value) -> Result<Value, ProtocolError> {
    let snapshot: HostedBrowserSessionList = serde_json::from_value(payload).map_err(|error| {
        handshake_error(
            ErrorCode::RequestFailed,
            format!("decode browser session discovery response: {error}"),
        )
    })?;
    Ok(redact_hosted_value(encode_control_value(snapshot)?, None))
}

fn hosted_private_event_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "ws_url"
            | "extension_frame_ws_url"
            | "cdp_endpoint_path"
            | "discovery_url"
            | "project_root"
            | "root"
            | "token"
            | "command_token"
            | "lease_token"
    ) || key.ends_with("_token")
}

fn hosted_sensitive_value_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "authorization"
            | "proxy_authorization"
            | "cookie"
            | "set_cookie"
            | "api_key"
            | "access_token"
            | "refresh_token"
            | "client_secret"
            | "password"
            | "secret"
    ) || key.contains("credential")
        || key.contains("private_key")
        || key.contains("api-key")
        || key.contains("apikey")
        || key.contains("authorization")
        || key.contains("cookie")
}

fn hosted_path_value_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "path" | "feature_path" | "file_path" | "project_path"
    )
}

fn hosted_url_value_key(key: &str) -> bool {
    matches!(key.to_ascii_lowercase().as_str(), "url" | "page_url")
}

fn hosted_body_value_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "request_body" | "response_body" | "body"
    )
}

fn hosted_assertions_value(value: Value) -> Value {
    let Value::Array(assertions) = value else {
        return Value::String("<redacted-assertions>".into());
    };
    Value::Array(
        assertions
            .into_iter()
            .map(|assertion| match assertion {
                Value::Object(assertion) => Value::Object(
                    assertion
                        .into_iter()
                        .map(|(key, value)| {
                            let safe = key == "passed" && value.is_boolean();
                            (
                                key,
                                if safe {
                                    value
                                } else {
                                    Value::String("***".into())
                                },
                            )
                        })
                        .collect(),
                ),
                _ => Value::String("***".into()),
            })
            .collect(),
    )
}

fn looks_like_absolute_local_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    FsPath::new(value).is_absolute()
        || value.starts_with("\\\\")
        || (bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/'))
}

fn contains_embedded_local_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with("\\\\")
        || bytes.windows(3).any(|window| {
            window[0].is_ascii_alphabetic()
                && window[1] == b':'
                && (window[2] == b'\\' || window[2] == b'/')
        })
        || value.split_whitespace().any(|part| {
            part.trim_matches(|character: char| {
                matches!(
                    character,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | ',' | ';'
                )
            })
            .starts_with('/')
        })
}

fn hosted_relative_path(project_root: Option<&FsPath>, value: &str) -> String {
    if !looks_like_absolute_local_path(value) {
        return value.to_owned();
    }
    let Some(root) = project_root else {
        return "<local-path>".into();
    };
    let candidate = FsPath::new(value);
    let relative = candidate
        .canonicalize()
        .ok()
        .and_then(|path| path.strip_prefix(root).ok().map(FsPath::to_path_buf))
        .or_else(|| candidate.strip_prefix(root).ok().map(FsPath::to_path_buf));
    relative
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "<local-path>".into())
}

fn hosted_public_url(value: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(value) else {
        return if looks_like_absolute_local_path(value) || contains_embedded_local_path(value) {
            "<local-path>".into()
        } else {
            "<redacted-url>".into()
        };
    };
    if !matches!(url.scheme(), "http" | "https") {
        return "<redacted-url>".into();
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

fn hosted_body_value(value: Value, project_root: Option<&FsPath>) -> Value {
    match value {
        Value::Object(_) | Value::Array(_) => redact_hosted_value(value, project_root),
        Value::Null => Value::Null,
        _ => Value::String("<redacted-body>".into()),
    }
}

/// Remove private coordinates, credentials, and local paths from values that
/// cross the hosted WebSocket boundary. Legacy REST responses are preserved
/// for local/Admin clients, while the hosted role receives this projection.
fn redact_hosted_value(value: Value, project_root: Option<&FsPath>) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter_map(|(key, value)| {
                    if hosted_private_event_key(&key) {
                        return None;
                    }
                    if hosted_sensitive_value_key(&key) {
                        return Some((key, Value::String("***".into())));
                    }
                    if hosted_body_value_key(&key) {
                        return Some((key, hosted_body_value(value, project_root)));
                    }
                    if key.eq_ignore_ascii_case("asserts") {
                        return Some((key, hosted_assertions_value(value)));
                    }
                    if hosted_url_value_key(&key) {
                        let value = match value {
                            Value::String(value) => Value::String(hosted_public_url(&value)),
                            value => redact_hosted_value(value, project_root),
                        };
                        return Some((key, value));
                    }
                    if hosted_path_value_key(&key) {
                        let value = match value {
                            Value::String(value) => {
                                Value::String(hosted_relative_path(project_root, &value))
                            }
                            value => redact_hosted_value(value, project_root),
                        };
                        return Some((key, value));
                    }
                    Some((key, redact_hosted_value(value, project_root)))
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| redact_hosted_value(value, project_root))
                .collect(),
        ),
        Value::String(value) if looks_like_absolute_local_path(&value) => {
            Value::String(hosted_relative_path(project_root, &value))
        }
        Value::String(value) if contains_embedded_local_path(&value) => {
            Value::String("<local-path>".into())
        }
        Value::String(value) if reqwest::Url::parse(&value).is_ok() => {
            Value::String(hosted_public_url(&value))
        }
        Value::String(value) if value.contains("://") || value.starts_with("//") => {
            Value::String("<redacted-url>".into())
        }
        value => value,
    }
}

/// Remove private sidecar coordinates/tokens from runtime events while
/// retaining unrelated Terminal/Agent/browser state for the hosted client.
fn redact_hosted_event_value(value: Value) -> Value {
    redact_hosted_value(value, None)
}

fn hosted_control_value(state: &DaemonState, value: Value) -> Value {
    let project_root = state.rt.project.root.lock().unwrap().clone();
    redact_hosted_value(value, project_root.as_deref())
}

fn control_api_error(error: ApiError) -> ProtocolError {
    ProtocolError {
        code: ErrorCode::RequestFailed,
        message: hosted_error_message(error.message),
        details: None,
    }
}

fn hosted_error_message(message: String) -> String {
    if message.contains("://") || message.starts_with("//") {
        return "local operation failed; private endpoint details were redacted".into();
    }
    let value = serde_json::from_str::<Value>(&message).unwrap_or(Value::String(message));
    match redact_hosted_value(value, None) {
        Value::String(message) => message,
        value => value.to_string(),
    }
}

fn control_status(status: StatusCode) -> Value {
    json!({ "status": status.as_u16() })
}

fn control_route_error(error: (StatusCode, Json<Value>)) -> ProtocolError {
    let (status, Json(payload)) = error;
    let message = match payload.get("error") {
        Some(Value::Object(error)) => hosted_error_message(
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("local route failed")
                .to_string(),
        ),
        Some(Value::String(message)) => hosted_error_message(message.clone()),
        _ => "local route failed".to_string(),
    };
    let code = if status == StatusCode::FORBIDDEN {
        ErrorCode::Forbidden
    } else {
        ErrorCode::RequestFailed
    };
    ProtocolError {
        code,
        message,
        details: None,
    }
}

async fn dispatch_control_request(
    state: &DaemonState,
    method: &str,
    params: Value,
) -> Result<Value, ProtocolError> {
    state.touch();
    if method == "runtime.shutdown" {
        return Err(handshake_error(
            ErrorCode::Forbidden,
            "hosted Web UI cannot shut down the daemon",
        ));
    }
    if control_method_capability(method).is_none() {
        return Err(handshake_error(
            ErrorCode::UnknownMethod,
            format!("control method '{method}' is unknown"),
        ));
    }
    let value = match method {
        "project.open" => {
            let body = decode_control_params::<OpenProjectBody>(params)?;
            let _ = api_open_project(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            // The local project root is daemon-private. The hosted client
            // only needs an acknowledgement and must not receive the
            // canonical absolute path.
            json!({ "opened": true })
        }
        "project.teardown" => {
            let status = api_teardown(State(state.clone()))
                .await
                .map_err(control_api_error)?;
            control_status(status)
        }
        "project.switch_allowed" => json!(api_switch_allowed(State(state.clone())).await.0),
        "project.list_recent" => {
            let result = api_recent().await.map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "filesystem.list" => {
            let path = params.get("path").and_then(Value::as_str).unwrap_or("");
            hosted_list_directory(state, path)?
        }
        "filesystem.read" => {
            let path = control_param_string(&params, "path")?;
            json!(hosted_read_project_file(state, &path)?)
        }
        "bdd.render_feature" => {
            let path = control_param_string(&params, "path")?;
            let path = hosted_feature_path(state, &path)?;
            let result = api_render_feature(State(state.clone()), Json(RenderBody { path }))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "bdd.list_scenarios" => {
            let root = state
                .rt
                .project
                .root
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| handshake_error(ErrorCode::RequestFailed, "no project is open"))?;
            let scenarios = list_runnable_scenarios(&root)
                .into_iter()
                .filter(|scenario| !hosted_private_path(&root, FsPath::new(&scenario.feature_path)))
                .collect::<Vec<_>>();
            hosted_control_value(state, encode_control_value(scenarios)?)
        }
        "bdd.run" => {
            let mut body = decode_control_params::<RunApiBody>(params)?;
            let requested = body.feature_path.as_deref().ok_or_else(|| {
                handshake_error(
                    ErrorCode::Forbidden,
                    "hosted BDD runs require an explicit project-relative feature path",
                )
            })?;
            body.feature_path = Some(hosted_feature_path(state, requested)?);
            let response = api_run(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
                .await
                .map_err(|error| handshake_error(ErrorCode::RequestFailed, error.to_string()))?;
            let text = String::from_utf8(bytes.to_vec()).map_err(|error| {
                handshake_error(
                    ErrorCode::RequestFailed,
                    format!("run output is not UTF-8: {error}"),
                )
            })?;
            let project_root = get_project_root(&state.rt).map(PathBuf::from);
            let events = text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    let payload: Value = serde_json::from_str(line).map_err(|error| {
                        handshake_error(ErrorCode::RequestFailed, error.to_string())
                    })?;
                    Ok(json!({
                        "type_name": payload.get("type").and_then(Value::as_str).unwrap_or_default(),
                        "payload": redact_hosted_value(payload, project_root.as_deref()),
                    }))
                })
                .collect::<Result<Vec<_>, ProtocolError>>()?;
            Value::Array(events)
        }
        "steps.catalog" => {
            let query = decode_control_params::<StepCatalogQuery>(params)?;
            hosted_control_value(
                state,
                build_step_catalog(state, query, true).map_err(control_api_error)?,
            )
        }
        "locator.sync_step" => {
            let mut body = decode_control_params::<SyncStepBody>(params)?;
            body.feature_path = hosted_feature_path(state, &body.feature_path)?;
            let result = api_sync_step(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "locator.active_step" => {
            let result = api_active_step(State(state.clone()))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "locator.pending" => {
            let result = api_pending_locator(State(state.clone()))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "steps.statuses" => {
            let path = control_param_string(&params, "feature_path")?;
            let path = hosted_feature_path(state, &path)?;
            let result = api_step_statuses(
                State(state.clone()),
                Query(StepStatusesQuery { feature_path: path }),
            )
            .await
            .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "steps.unbind" => {
            let mut body = decode_control_params::<UnbindStepBody>(params)?;
            body.feature_path = hosted_feature_path(state, &body.feature_path)?;
            let result = api_unbind_step(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "project.get_settings" => {
            let result = api_project_settings(State(state.clone()))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "llm.get_config" => {
            let result = api_get_llm_config(State(state.clone()))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "llm.set_config" => {
            let body = decode_control_params::<LlmConfigWrite>(params)?;
            let result = api_put_llm_config(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "llm.list_profiles" => {
            let result = api_list_llm_profiles(State(state.clone()))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "llm.get_profile" => {
            let id = control_param_string(&params, "id")?;
            let result = api_get_llm_profile(State(state.clone()), Path(id))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "llm.save_profile" => {
            let body = decode_control_params::<ProfileWriteBody>(params)?;
            let result = api_put_llm_profile(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, encode_control_value(result.0)?)
        }
        "llm.delete_profile" => {
            let id = control_param_string(&params, "id")?;
            let status = api_delete_llm_profile(State(state.clone()), Path(id))
                .await
                .map_err(control_api_error)?;
            control_status(status)
        }
        "llm.activate_profile" => {
            let id = control_param_string(&params, "id")?;
            let status = api_activate_llm_profile(State(state.clone()), Path(id))
                .await
                .map_err(control_api_error)?;
            control_status(status)
        }
        "locator.confirm" => {
            let body = decode_control_params::<ConfirmBody>(params)?;
            let status = api_confirm_locator(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            control_status(status)
        }
        "locator.reject" => {
            let status = api_reject_locator(State(state.clone()))
                .await
                .map_err(control_api_error)?;
            control_status(status)
        }
        "locator.highlight" => {
            let body = decode_control_params::<HighlightLocatorBody>(params)?;
            let status = api_highlight_locator(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            control_status(status)
        }
        "browser.start" => {
            let body = decode_control_params::<BrowserStartBody>(params)?;
            let result = api_browser_start(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            hosted_browser_start_value(result.0)
        }
        "browser.stop" => {
            let status = api_browser_stop(State(state.clone()))
                .await
                .map_err(control_api_error)?;
            control_status(status)
        }
        "browser.list_sessions" => {
            let result = api_browser_sessions(State(state.clone()))
                .await
                .map_err(control_route_error)?;
            hosted_browser_sessions_value(result.0)?
        }
        "browser.activate_tab" => {
            let body = decode_control_params::<BrowserActivateTabBody>(params)?;
            let result = api_browser_activate_tab(State(state.clone()), Json(body))
                .await
                .map_err(control_route_error)?;
            hosted_control_value(state, redact_hosted_event_value(result.0))
        }
        "api.get_exchange" => {
            let body = decode_control_params::<ExchangeApiBody>(params)?;
            // Plaintext exchange expansion is intentionally local-only. The
            // hosted page receives a redacted projection even if an older UI
            // asks for `redact: false`.
            let body = ExchangeApiBody {
                exchange_id: body.exchange_id,
                redact: Some(true),
            };
            let result = api_get_exchange(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            hosted_control_value(state, result.0)
        }
        "terminal.spawn" => {
            let body = decode_control_params::<SpawnBody>(params)?;
            let status = api_terminal_spawn(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            control_status(status)
        }
        "terminal.stop" => {
            let status = api_terminal_stop(State(state.clone()))
                .await
                .map_err(control_api_error)?;
            control_status(status)
        }
        "terminal.resize" => {
            let body = decode_control_params::<ResizeBody>(params)?;
            let status = api_terminal_resize(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            control_status(status)
        }
        "terminal.write" => {
            let body = decode_control_params::<WriteBody>(params)?;
            let status = api_terminal_write(State(state.clone()), Json(body))
                .await
                .map_err(control_api_error)?;
            control_status(status)
        }
        "runtime.shutdown" => {
            let status = api_daemon_shutdown(State(state.clone())).await;
            control_status(status)
        }
        _ => {
            return Err(handshake_error(
                ErrorCode::UnknownMethod,
                format!("control method '{method}' is not migrated yet"),
            ));
        }
    };
    Ok(value)
}

async fn dispatch_authenticated_control_request(
    state: &DaemonState,
    token: &str,
    method: &str,
    params: Value,
) -> Result<Value, ProtocolError> {
    // Project teardown owns the write side inside `api_teardown`; taking a
    // read guard here would deadlock. Every other request holds a read guard
    // across the domain operation, so session replacement/teardown waits for
    // in-flight work and no invalid token can start a new operation.
    let _session_guard = if method == "project.teardown" {
        None
    } else {
        Some(state.hosted_session_gate.read().await)
    };
    if state.sessions.get_session(token).is_none() {
        return Err(handshake_error(
            ErrorCode::SessionExpired,
            "hosted Web UI session has expired",
        ));
    }
    dispatch_control_request(state, method, params).await
}

async fn handle_control_socket(state: DaemonState, mut socket: WebSocket) {
    // A live control socket is itself activity: the idle watchdog must not
    // tear down a daemon while the hosted UI is connected but temporarily
    // quiet. The guard also covers handshake failures and early disconnects.
    state.active_ws.fetch_add(1, Ordering::Relaxed);
    let _guard = WsGuard(state.active_ws.clone());

    let hello = match receive_client_message(&mut socket).await {
        Ok(ClientMessage::ClientHello(hello)) => hello,
        Ok(_) => {
            send_protocol_error(
                &mut socket,
                handshake_error(
                    ErrorCode::HandshakeRequired,
                    "first message must be client_hello",
                ),
            )
            .await;
            return;
        }
        Err(error) => {
            send_protocol_error(&mut socket, error).await;
            return;
        }
    };
    let (token, daemon) = match authenticate_hello(&state, hello, Channel::Control).await {
        Ok(value) => value,
        Err(error) => {
            send_protocol_error(&mut socket, error).await;
            return;
        }
    };
    {
        if !claim_control_session(&state.active_control_sessions, &token) {
            send_protocol_error(
                &mut socket,
                handshake_error(
                    ErrorCode::Forbidden,
                    "hosted control session is already connected",
                ),
            )
            .await;
            return;
        }
    }
    let active_token = token;
    let session_id = format!("ws_{}", uuid::Uuid::new_v4().simple());
    let hello = ServerMessage::ServerHello {
        daemon,
        channel: Channel::Control,
        protocol_version: CONTROL_PROTOCOL_VERSION,
        session_id,
        capabilities: vec![
            HostedCapability::Project,
            HostedCapability::Filesystem,
            HostedCapability::Gherkin,
            HostedCapability::BddRun,
            HostedCapability::ApiExchange,
            HostedCapability::Locator,
            HostedCapability::Steps,
            HostedCapability::LlmConfig,
            HostedCapability::BrowserSessions,
            HostedCapability::Terminal,
            HostedCapability::Agent,
            HostedCapability::RuntimeEvents,
        ],
    };
    if let Ok(text) = serde_json::to_string(&hello) {
        if socket.send(Message::Text(text.into())).await.is_err() {
            release_control_session(&state.active_control_sessions, &active_token);
            return;
        }
    }

    // The writer is the only task touching the sink. Responses and events use
    // separate bounded queues so a burst of terminal/agent events cannot make
    // control RPC responses wait behind an unbounded producer.
    let (mut sink, mut stream) = socket.split();
    let (response_tx, mut response_rx) =
        tokio::sync::mpsc::channel::<ServerMessage>(CONTROL_RESPONSE_QUEUE_CAPACITY);
    let (event_tx, mut event_rx) =
        tokio::sync::mpsc::channel::<ServerMessage>(CONTROL_EVENT_QUEUE_CAPACITY);
    let writer = tokio::spawn(async move {
        loop {
            let message = match response_rx.try_recv() {
                Ok(message) => message,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)
                | Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    tokio::select! {
                        biased;
                        message = response_rx.recv() => match message {
                            Some(message) => message,
                            None => break,
                        },
                        message = event_rx.recv() => match message {
                            Some(message) => message,
                            None => break,
                        },
                    }
                }
            };
            let Ok(encoded) = serde_json::to_string(&message) else {
                continue;
            };
            if sink.send(Message::Text(encoded.into())).await.is_err() {
                break;
            }
        }
    });
    let permits = Arc::new(tokio::sync::Semaphore::new(CONTROL_REQUEST_CONCURRENCY));
    let mut request_tasks = tokio::task::JoinSet::new();
    let mut events = state.rt.events.subscribe();
    let mut dropped_events = 0u64;
    let mut session_watch = tokio::time::interval(std::time::Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = session_watch.tick() => {
                if state.sessions.get_session(&active_token).is_none() {
                    break;
                }
            }
            incoming = stream.next() => {
                let Some(Ok(incoming)) = incoming else { break; };
                match incoming {
                    Message::Text(text) => match serde_json::from_str::<ClientMessage>(text.as_ref()) {
                        Ok(ClientMessage::Request(ProtocolRequest { id, method, params })) => {
                            // Session invalidation is checked at request time,
                            // not only by the periodic watcher. This closes
                            // the small replacement/teardown window in which
                            // an already-authenticated socket could otherwise
                            // submit one more business request.
                            if state.sessions.get_session(&active_token).is_none() {
                                let _ = response_tx
                                    .send(ServerMessage::Response {
                                        id,
                                        ok: false,
                                        result: None,
                                        error: Some(handshake_error(
                                            ErrorCode::SessionExpired,
                                            "hosted Web UI session has expired",
                                        )),
                                    })
                                    .await;
                                break;
                            }
                            let Ok(permit) = permits.clone().try_acquire_owned() else {
                                let _ = response_tx.send(ServerMessage::Response {
                                    id,
                                    ok: false,
                                    result: None,
                                    error: Some(handshake_error(
                                        ErrorCode::RequestFailed,
                                        "control request queue is full",
                                    )),
                                }).await;
                                continue;
                            };
                            let state_for_request = state.clone();
                            let response_for_request = response_tx.clone();
                            let token_for_request = active_token.clone();
                            request_tasks.spawn(async move {
                                let response = match dispatch_authenticated_control_request(
                                    &state_for_request,
                                    &token_for_request,
                                    &method,
                                    params,
                                ).await {
                                    Ok(result) => ServerMessage::Response {
                                        id,
                                        ok: true,
                                        result: Some(result),
                                        error: None,
                                    },
                                    Err(error) => ServerMessage::Response {
                                        id,
                                        ok: false,
                                        result: None,
                                        error: Some(error),
                                    },
                                };
                                let _ = response_for_request.send(response).await;
                                drop(permit);
                            });
                        }
                        Ok(ClientMessage::Close) | Ok(ClientMessage::ClientHello(_)) => break,
                        Err(_) => {
                            let _ = response_tx.send(ServerMessage::Error(
                                handshake_error(ErrorCode::InvalidMessage, "invalid control message"),
                            )).await;
                        }
                    },
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            event = events.recv() => {
                let message = match event {
                    Ok(RuntimeEvent { name, payload }) => Some(ServerMessage::Event {
                        event: name,
                        payload: {
                            let project_root = state.rt.project.root.lock().unwrap().clone();
                            redact_hosted_value(payload, project_root.as_deref())
                        },
                    }),
                    // RuntimeEvents is a bounded broadcast channel. Report a
                    // lag explicitly so the hosted UI can refresh state rather
                    // than mistaking a dropped terminal/agent burst for success.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        dropped_events = dropped_events.saturating_add(count);
                        None
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if dropped_events > 0 {
                    let overflow = ServerMessage::Event {
                        event: "runtime.overflow".into(),
                        payload: json!({ "dropped": dropped_events }),
                    };
                    match event_tx.try_send(overflow) {
                        Ok(()) => dropped_events = 0,
                        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                            dropped_events = dropped_events.saturating_add(1);
                            continue;
                        }
                        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
                    }
                }
                let Some(message) = message else {
                    continue;
                };
                if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) = event_tx.try_send(message) {
                    dropped_events = dropped_events.saturating_add(1);
                } else if event_tx.is_closed() {
                    break;
                }
            }
            Some(_) = request_tasks.join_next(), if !request_tasks.is_empty() => {}
        }
    }
    request_tasks.abort_all();
    while request_tasks.join_next().await.is_some() {}
    drop(response_tx);
    drop(event_tx);
    let _ = writer.await;
    release_control_session(&state.active_control_sessions, &active_token);
}

async fn handle_preview_socket(state: DaemonState, mut socket: WebSocket) {
    let hello = match receive_client_message(&mut socket).await {
        Ok(ClientMessage::ClientHello(hello)) => hello,
        Ok(_) => {
            send_protocol_error(
                &mut socket,
                handshake_error(
                    ErrorCode::HandshakeRequired,
                    "first message must be client_hello",
                ),
            )
            .await;
            return;
        }
        Err(error) => {
            send_protocol_error(&mut socket, error).await;
            return;
        }
    };
    let (token, daemon) = match authenticate_hello(&state, hello, Channel::Preview).await {
        Ok(value) => value,
        Err(error) => {
            send_protocol_error(&mut socket, error).await;
            return;
        }
    };
    if !state
        .active_control_sessions
        .lock()
        .unwrap()
        .contains(&token)
    {
        send_protocol_error(
            &mut socket,
            handshake_error(
                ErrorCode::SessionExpired,
                "preview requires a live control session",
            ),
        )
        .await;
        return;
    }
    let hello = ServerMessage::ServerHello {
        daemon,
        channel: Channel::Preview,
        protocol_version: PREVIEW_PROTOCOL_VERSION,
        session_id: format!("preview_{}", uuid::Uuid::new_v4().simple()),
        capabilities: vec![],
    };
    if let Ok(text) = serde_json::to_string(&hello) {
        if socket.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }

    let Some(ws_url) = state.rt.sidecar.browser_ws_url() else {
        let _ = socket
            .send(Message::Text(
                json!({
                    "type": "frame_error",
                    "error": "preview sidecar is not running",
                })
                .to_string()
                .into(),
            ))
            .await;
        return;
    };
    let mode = state.rt.sidecar.browser_mode();
    if !preview_stream_supported(mode) {
        let _ = socket
            .send(Message::Text(
                json!({
                    "type": "frame_error",
                    "error": "active sidecar does not emit a screenshot stream",
                })
                .to_string()
                .into(),
            ))
            .await;
        return;
    }
    let attach_process = (mode == Some(BrowserMode::WinApp)).then(|| {
        std::env::var("TESHI_WINAPP_PROCESS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "TargetApp.exe".into())
    });
    let active_ws = state.active_ws.clone();
    let active_sessions = state.active_control_sessions.clone();
    let session_token = token;
    let relay = handle_browser_stream_socket(ws_url, attach_process, active_ws, socket, true);
    tokio::pin!(relay);
    let mut missing_since = None;
    loop {
        tokio::select! {
            _ = &mut relay => {
                break;
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {
                if state.sessions.get_session(&session_token).is_none() {
                    break;
                }
                let control_alive = active_sessions.lock().unwrap().contains(&session_token);
                if control_alive {
                    missing_since = None;
                } else {
                    let first_missing = missing_since.get_or_insert_with(std::time::Instant::now);
                    if first_missing.elapsed() >= PREVIEW_RECONNECT_GRACE {
                        break;
                    }
                }
            }
        }
    }
}

async fn events_ws(State(state): State<DaemonState>, ws: WebSocketUpgrade) -> Response {
    let rt = state.rt.clone();
    let active_ws = state.active_ws.clone();
    ws.on_upgrade(move |socket| handle_events_socket(rt, active_ws, socket))
}

struct WsGuard(Arc<AtomicUsize>);
impl Drop for WsGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn handle_events_socket(
    rt: SharedRuntime,
    active_ws: Arc<AtomicUsize>,
    mut socket: WebSocket,
) {
    active_ws.fetch_add(1, Ordering::Relaxed);
    let _guard = WsGuard(active_ws);
    let mut rx = rt.events.subscribe();
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(RuntimeEvent { name, payload }) => {
                        let envelope = json!({ "event": name, "payload": payload });
                        let text = match serde_json::to_string(&envelope) {
                            Ok(t) => t,
                            Err(_) => continue,
                        };
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    // Slow clients may skip bursts (e.g. PTY flood); stay connected.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                if incoming.is_none() || matches!(incoming, Some(Ok(Message::Close(_)))) {
                    break;
                }
            }
        }
    }
}

#[derive(Debug)]
enum PreviewRelayMessage {
    Text(String),
    Binary(Vec<u8>),
}

fn is_preview_frame(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value.get("type").and_then(Value::as_str).map(str::to_owned))
        .is_some_and(|kind| kind == "frame")
}

/// Project only the sidecar's public preview messages. The sidecar protocol
/// also carries command responses, broker coordinates, target metadata, and
/// arbitrary extension payloads; none of those are part of the hosted preview
/// contract and must not be relayed across the hosted-origin boundary.
fn sanitize_preview_message(text: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    let kind = value.get("type").and_then(Value::as_str)?;
    match kind {
        "frame" => {
            let data = value.get("data").and_then(Value::as_str)?;
            let mut frame = serde_json::Map::new();
            frame.insert("type".into(), Value::String("frame".into()));
            frame.insert("data".into(), Value::String(data.to_owned()));
            if let Some(url) = value.get("url").and_then(Value::as_str) {
                frame.insert("url".into(), Value::String(hosted_public_url(url)));
            }
            for key in ["tab_id", "width", "height"] {
                if let Some(field) = value.get(key).filter(|field| field.is_number()) {
                    frame.insert(key.into(), field.clone());
                }
            }
            if let Some(backend) =
                value
                    .get("capture_backend")
                    .and_then(Value::as_str)
                    .filter(|backend| {
                        matches!(
                            *backend,
                            "wgc" | "imagegrab" | "chrome" | "embedded" | "winapp"
                        )
                    })
            {
                frame.insert("capture_backend".into(), Value::String(backend.into()));
            }
            serde_json::to_string(&Value::Object(frame)).ok()
        }
        "frame_error" => serde_json::to_string(&json!({
            "type": "frame_error",
            "error": "preview capture failed",
        }))
        .ok(),
        _ => None,
    }
}

/// Chrome, embedded Playwright, and WinApp sidecars all emit `{type:frame}` JPEGs.
fn preview_stream_supported(mode: Option<BrowserMode>) -> bool {
    matches!(
        mode,
        Some(BrowserMode::WinApp | BrowserMode::Chrome | BrowserMode::Embedded)
    )
}

async fn browser_stream_ws(State(state): State<DaemonState>, ws: WebSocketUpgrade) -> Response {
    state.touch();
    let Some(ws_url) = state.rt.sidecar.browser_ws_url() else {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "preview sidecar is not running" })),
        )
            .into_response();
    };
    let mode = state.rt.sidecar.browser_mode();
    if !preview_stream_supported(mode) {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "active sidecar does not emit a screenshot stream" })),
        )
            .into_response();
    }

    let attach_process = (mode == Some(BrowserMode::WinApp)).then(|| {
        std::env::var("TESHI_WINAPP_PROCESS")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "TargetApp.exe".into())
    });
    let active_ws = state.active_ws.clone();
    ws.on_upgrade(move |socket| {
        handle_browser_stream_socket(ws_url, attach_process, active_ws, socket, false)
    })
}

async fn handle_browser_stream_socket(
    ws_url: String,
    attach_process: Option<String>,
    active_ws: Arc<AtomicUsize>,
    mut downstream: WebSocket,
    hosted_boundary: bool,
) {
    active_ws.fetch_add(1, Ordering::Relaxed);
    let _guard = WsGuard(active_ws);

    let mut upstream = match tokio_tungstenite::connect_async(&ws_url).await {
        Ok((socket, _)) => socket,
        Err(_error) => {
            let payload = json!({
                "type": "frame_error",
                // Do not expose the private sidecar URL (or connection
                // credentials that may be embedded in it) to the browser.
                "error": "connect to preview sidecar failed",
            });
            let _ = downstream
                .send(Message::Text(payload.to_string().into()))
                .await;
            return;
        }
    };

    if let Some(process_name) = attach_process {
        let attach = json!({
            "cmd": "attach_window",
            "request_id": "gpui-preview-attach",
            "process_name": process_name,
        });
        if let Err(_error) = upstream
            .send(tokio_tungstenite::tungstenite::Message::Text(
                attach.to_string(),
            ))
            .await
        {
            let payload = json!({
                "type": "frame_error",
                "error": "attach to target application failed",
            });
            let _ = downstream
                .send(Message::Text(payload.to_string().into()))
                .await;
            return;
        }
    }

    let (frame_tx, mut frame_rx) = tokio::sync::watch::channel(None::<String>);
    let (control_tx, mut control_rx) =
        tokio::sync::mpsc::channel::<PreviewRelayMessage>(PREVIEW_CONTROL_QUEUE_CAPACITY);
    let upstream_reader = tokio::spawn(async move {
        while let Some(message) = upstream.next().await {
            match message {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    let text = text.to_string();
                    if hosted_boundary {
                        if let Some(safe_text) = sanitize_preview_message(&text) {
                            if is_preview_frame(&safe_text) {
                                frame_tx.send_replace(Some(safe_text));
                            } else if control_tx
                                .send(PreviewRelayMessage::Text(safe_text))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    } else if is_preview_frame(&text) {
                        frame_tx.send_replace(Some(text));
                    } else if control_tx
                        .send(PreviewRelayMessage::Text(text))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes)) => {
                    if !hosted_boundary
                        && control_tx
                            .send(PreviewRelayMessage::Binary(bytes.to_vec()))
                            .await
                            .is_err()
                    {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Ping(payload)) => {
                    if upstream
                        .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });

    let mut frames_open = true;
    let mut controls_open = true;
    loop {
        tokio::select! {
            changed = frame_rx.changed(), if frames_open => {
                if changed.is_err() {
                    frames_open = false;
                } else {
                    let frame = { frame_rx.borrow_and_update().clone() };
                    if let Some(frame) = frame {
                        if downstream.send(Message::Text(frame.into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            control = control_rx.recv(), if controls_open => {
                let message = match control {
                    Some(PreviewRelayMessage::Text(text)) => Message::Text(text.into()),
                    Some(PreviewRelayMessage::Binary(bytes)) => Message::Binary(bytes.into()),
                    None => {
                        controls_open = false;
                        if !frames_open { break; }
                        continue;
                    }
                };
                if downstream.send(message).await.is_err() {
                    break;
                }
            }
            incoming = downstream.recv() => {
                if incoming.is_none() || matches!(incoming, Some(Ok(Message::Close(_)))) {
                    break;
                }
            }
        }
        if !frames_open && !controls_open {
            break;
        }
    }
    upstream_reader.abort();
}

// ── Auth middleware ─────────────────────────────────────────────────────────

/// Axum middleware that checks `X-Teshi-Token` against the session store.
///
/// Tokenless requests are accepted as `Admin` only from the loopback interface
/// for compatibility with the local web UI. Remote requests require a valid
/// session token, and an invalid token always fails closed.
async fn auth_middleware(
    State(state): State<DaemonState>,
    req: Request,
    next: middleware::Next,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let path = req.uri().path().to_string();

    // Extract token from header
    let token = req
        .headers()
        .get("x-teshi-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let is_loopback = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|ConnectInfo(peer)| peer.ip().is_loopback());

    // Local UI requests may omit the token. Every supplied token must be valid,
    // and remote requests may never obtain implicit Admin access.
    let role = if token.is_empty() {
        if is_loopback {
            Role::Admin
        } else {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "A valid X-Teshi-Token is required" })),
            ));
        }
    } else {
        let Some(session) = state.sessions.get_session(token) else {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Invalid or expired X-Teshi-Token" })),
            ));
        };
        session.role
    };

    if !role.can_execute(&path) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": format!(
                    "Security Guard: Action '{}' is not allowed for role '{:?}'",
                    path, role
                )
            })),
        ));
    }

    Ok(next.run(req).await)
}

async fn loopback_only(req: Request, next: Next) -> Response {
    let is_loopback = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|ConnectInfo(peer)| peer.ip().is_loopback());
    if is_loopback {
        next.run(req).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

// ── Session API ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateSessionBody {
    role: String,
    #[serde(default)]
    metadata: Option<std::collections::HashMap<String, String>>,
}

#[derive(Serialize)]
struct CreateSessionResponse {
    token: String,
    role: String,
}

async fn api_create_session(
    State(state): State<DaemonState>,
    Json(body): Json<CreateSessionBody>,
) -> Result<Json<CreateSessionResponse>, (StatusCode, Json<Value>)> {
    let role = match body.role.to_lowercase().as_str() {
        "admin" => Role::Admin,
        "agent_recorder" | "agentrecorder" | "agent-recorder" => Role::AgentRecorder,
        "batch_runner" | "batchrunner" | "batch-runner" => Role::BatchRunner,
        "hosted_web_ui" | "hostedwebui" | "hosted-web-ui" => Role::HostedWebUi,
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!(
                        "Unknown role '{other}'. Valid roles: admin, agent_recorder, batch_runner, hosted_web_ui"
                    )
                })),
            ));
        }
    };
    let token = if role == Role::HostedWebUi {
        let _session_guard = state.hosted_session_gate.write().await;
        // A fresh `teshi web` launch supersedes the previous hosted page. Do
        // this atomically in SessionStore so an old fragment cannot remain a
        // second valid control/preview credential.
        state.sessions.create_hosted_session()
    } else {
        state.sessions.create_session(role, body.metadata)
    };
    Ok(Json(CreateSessionResponse {
        token,
        role: format!("{role:?}"),
    }))
}

#[derive(Serialize)]
struct SessionInfo {
    token: String,
    role: String,
    created_at_secs: f64,
}

async fn api_get_session(
    State(state): State<DaemonState>,
    Path(token): Path<String>,
) -> Result<Json<SessionInfo>, (StatusCode, Json<Value>)> {
    let session = state.sessions.get_session(&token).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "session not found" })),
        )
    })?;
    Ok(Json(SessionInfo {
        token: session.token,
        role: format!("{:?}", session.role),
        created_at_secs: session.created_at_secs,
    }))
}

async fn api_delete_session(
    State(state): State<DaemonState>,
    Path(token): Path<String>,
) -> StatusCode {
    state.sessions.remove_session(&token);
    StatusCode::NO_CONTENT
}

// ---- Request types ----

#[derive(Deserialize)]
struct OpenProjectBody {
    path: String,
}

#[derive(Serialize)]
struct OpenProjectResponse {
    root: String,
}

// ---- Handlers ----

async fn api_open_project(
    State(state): State<DaemonState>,
    Json(body): Json<OpenProjectBody>,
) -> Result<Json<OpenProjectResponse>, ApiError> {
    state.touch();
    open_project(Arc::clone(&state.rt), body.path).await?;
    let root = get_project_root(&state.rt)
        .ok_or_else(|| ApiError::internal("project root missing after open"))?;
    Ok(Json(OpenProjectResponse { root }))
}

async fn api_teardown(State(state): State<DaemonState>) -> Result<StatusCode, ApiError> {
    state.touch();
    let _session_guard = state.hosted_session_gate.write().await;
    // Runtime teardown is an explicit end of the hosted launch lifecycle.
    // Invalidate all hosted credentials while preserving local automation
    // sessions that are unrelated to this page. Invalidate before attempting
    // cleanup so a partial teardown failure cannot leave the old credential
    // usable.
    state.sessions.remove_hosted_sessions();
    teardown_runtime(&state.rt).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_switch_allowed(State(state): State<DaemonState>) -> Json<bool> {
    state.touch();
    Json(check_project_switch_allowed(&state.rt))
}

async fn api_recent() -> Result<Json<Vec<String>>, ApiError> {
    Ok(Json(get_recent_projects()?))
}

#[derive(Deserialize)]
struct ListDirQuery {
    path: String,
}

async fn api_read_file(
    State(state): State<DaemonState>,
    Query(q): Query<ListDirQuery>,
) -> Result<String, (StatusCode, String)> {
    let project_root =
        get_project_root(&state.rt).ok_or((StatusCode::CONFLICT, "no project open".to_string()))?;
    read_project_file(FsPath::new(&project_root), FsPath::new(&q.path))
}

fn read_project_file(
    project_root: &FsPath,
    requested_path: &FsPath,
) -> Result<String, (StatusCode, String)> {
    let canonical_root = project_root.canonicalize().map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            "project root is unavailable".to_string(),
        )
    })?;
    let canonical_path = requested_path
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "file not found".to_string()))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err((
            StatusCode::FORBIDDEN,
            "file path is outside the open project".to_string(),
        ));
    }
    fs::read_to_string(&canonical_path).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            "file is not readable text".to_string(),
        )
    })
}

fn hosted_project_root(state: &DaemonState) -> Result<PathBuf, ProtocolError> {
    get_project_root(&state.rt)
        .map(PathBuf::from)
        .ok_or_else(|| handshake_error(ErrorCode::RequestFailed, "no project is open"))
}

fn hosted_private_component(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".teshi"
        || lower == ".git"
        || lower == ".ssh"
        || lower == ".env"
        || lower.starts_with(".env.")
        || matches!(lower.as_str(), ".aws" | ".docker" | ".kube")
        || matches!(
            lower.as_str(),
            ".npmrc"
                | ".netrc"
                | ".pypirc"
                | "auth.json"
                | "token.json"
                | "tokens.json"
                | "cookie.json"
                | "cookies.json"
                | "id_rsa"
                | "id_ed25519"
                | "id_ecdsa"
                | "id_dsa"
                | "credentials.json"
                | "service-account.json"
        )
        || lower.contains("secret")
        || lower.contains("credential")
        || lower.contains("password")
        || matches!(lower.as_str(), value if value.ends_with(".pem") || value.ends_with(".key") || value.ends_with(".p12") || value.ends_with(".pfx"))
}

fn hosted_private_path(project_root: &FsPath, path: &FsPath) -> bool {
    path.strip_prefix(project_root)
        .ok()
        .into_iter()
        .flat_map(FsPath::components)
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .any(hosted_private_component)
}

fn resolve_hosted_project_path(
    project_root: &FsPath,
    requested: &str,
) -> Result<PathBuf, ProtocolError> {
    let requested = requested.trim();
    if requested.is_empty() {
        return Ok(project_root.to_path_buf());
    }
    let requested_path = FsPath::new(requested);
    if requested_path.is_absolute() {
        return Err(handshake_error(
            ErrorCode::Forbidden,
            "hosted file paths must be project-relative",
        ));
    }
    let canonical_root = project_root
        .canonicalize()
        .map_err(|_| handshake_error(ErrorCode::RequestFailed, "project root is unavailable"))?;
    let canonical_path = canonical_root
        .join(requested_path)
        .canonicalize()
        .map_err(|_| handshake_error(ErrorCode::RequestFailed, "project path is unavailable"))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(handshake_error(
            ErrorCode::Forbidden,
            "file path is outside the open project",
        ));
    }
    if hosted_private_path(&canonical_root, &canonical_path) {
        return Err(handshake_error(
            ErrorCode::Forbidden,
            "private project metadata is not available to the hosted UI",
        ));
    }
    Ok(canonical_path)
}

fn hosted_relative_entry_path(project_root: &FsPath, path: &FsPath) -> String {
    path.strip_prefix(project_root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "<local-path>".into())
}

fn hosted_list_directory(state: &DaemonState, requested: &str) -> Result<Value, ProtocolError> {
    let project_root = hosted_project_root(state)?;
    let directory = resolve_hosted_project_path(&project_root, requested)?;
    let entries = list_dir(&state.rt, directory.to_string_lossy().into_owned())
        .map_err(|error| handshake_error(ErrorCode::RequestFailed, error))?;
    let entries = entries
        .into_iter()
        .filter_map(|mut entry| {
            let path = FsPath::new(&entry.path);
            if hosted_private_path(&project_root, path) {
                return None;
            }
            entry.path = hosted_relative_entry_path(&project_root, path);
            Some(entry)
        })
        .collect::<Vec<_>>();
    encode_control_value(entries)
}

fn hosted_read_project_file(state: &DaemonState, requested: &str) -> Result<String, ProtocolError> {
    let project_root = hosted_project_root(state)?;
    let path = resolve_hosted_project_path(&project_root, requested)?;
    fs::read_to_string(path)
        .map_err(|_| handshake_error(ErrorCode::RequestFailed, "file is not readable text"))
}

fn hosted_feature_path(state: &DaemonState, requested: &str) -> Result<String, ProtocolError> {
    let project_root = hosted_project_root(state)?;
    let path = resolve_hosted_project_path(&project_root, requested)?;
    if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("feature") {
        return Err(handshake_error(
            ErrorCode::Forbidden,
            "hosted BDD paths must identify a project feature file",
        ));
    }
    Ok(path.to_string_lossy().into_owned())
}

async fn api_list_dir(
    State(state): State<DaemonState>,
    Query(q): Query<ListDirQuery>,
) -> Result<Json<Vec<DirEntry>>, ApiError> {
    state.touch();
    Ok(Json(list_dir(&state.rt, q.path)?))
}

#[derive(Deserialize)]
struct RenderBody {
    path: String,
}

async fn api_render_feature(
    State(state): State<DaemonState>,
    Json(body): Json<RenderBody>,
) -> Result<Json<FeatureRenderPayload>, ApiError> {
    state.touch();
    Ok(Json(render_feature(&state.rt, body.path)?))
}

#[derive(Deserialize)]
struct SyncStepBody {
    feature_path: String,
    step_line: u32,
}

async fn api_sync_step(
    State(state): State<DaemonState>,
    Json(body): Json<SyncStepBody>,
) -> Result<Json<ActiveStep>, ApiError> {
    state.touch();
    Ok(Json(
        sync_active_step(&state.rt, body.feature_path, body.step_line).await?,
    ))
}

async fn api_active_step(
    State(state): State<DaemonState>,
) -> Result<Json<Option<ActiveStep>>, ApiError> {
    state.touch();
    Ok(Json(get_active_step(&state.rt)?))
}

async fn api_pending_locator(
    State(state): State<DaemonState>,
) -> Result<Json<Option<PendingLocator>>, ApiError> {
    state.touch();
    Ok(Json(get_pending_locator(&state.rt)?))
}

#[derive(Deserialize)]
struct StepStatusesQuery {
    feature_path: String,
}

async fn api_step_statuses(
    State(state): State<DaemonState>,
    Query(q): Query<StepStatusesQuery>,
) -> Result<Json<Vec<StepBindingStatus>>, ApiError> {
    state.touch();
    let project_root = state
        .rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;
    Ok(Json(
        step_binding_statuses(&project_root, &q.feature_path).map_err(|e| e.to_string())?,
    ))
}

#[derive(Deserialize)]
struct UnbindStepBody {
    feature_path: String,
    step_line: u32,
}

async fn api_unbind_step(
    State(state): State<DaemonState>,
    Json(body): Json<UnbindStepBody>,
) -> Result<Json<Option<StepBinding>>, ApiError> {
    state.touch();
    Ok(Json(
        unbind_step(&state.rt, body.feature_path, body.step_line)
            .await
            .map_err(|e| e.to_string())?,
    ))
}

async fn api_project_settings(
    State(state): State<DaemonState>,
) -> Result<Json<ProjectSettings>, ApiError> {
    state.touch();
    let project_root = state
        .rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;
    Ok(Json(
        load_project_settings(&project_root).map_err(|e| e.to_string())?,
    ))
}

async fn api_get_llm_config(
    State(state): State<DaemonState>,
) -> Result<Json<LlmConfigPublic>, ApiError> {
    state.touch();
    // Never log the stored API key; only return the masked public snapshot.
    Ok(Json(
        load_llm_config_public().map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

async fn api_put_llm_config(
    State(state): State<DaemonState>,
    Json(body): Json<LlmConfigWrite>,
) -> Result<Json<LlmConfigPublic>, ApiError> {
    state.touch();
    let stored = save_stored_llm_config(&body).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(teshi_engine::to_public(&stored)))
}

/// Daemon body for creating/updating a model profile (mirrors engine fields).
#[derive(Debug, Deserialize)]
struct ProfileWriteBody {
    #[serde(default)]
    id: String,
    name: String,
    #[serde(default = "default_provider")]
    provider: String,
    #[serde(default)]
    api_style: ApiStyle,
    #[serde(default)]
    model_id: String,
    #[serde(default)]
    max_context_tokens: Option<u32>,
    #[serde(default = "default_max_output")]
    max_output_tokens: u32,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default = "default_true")]
    stream: bool,
    #[serde(default)]
    http_headers: std::collections::HashMap<String, String>,
    #[serde(default)]
    chat_options: std::collections::HashMap<String, Value>,
}

fn default_provider() -> String {
    PROVIDER_OPENAI.to_string()
}

fn default_max_output() -> u32 {
    1024
}

fn default_true() -> bool {
    true
}

async fn api_list_llm_profiles(
    State(state): State<DaemonState>,
) -> Result<Json<ModelProfileList>, ApiError> {
    state.touch();
    Ok(Json(
        list_profiles().map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

async fn api_get_llm_profile(
    State(state): State<DaemonState>,
    Path(id): Path<String>,
) -> Result<Json<ModelProfilePublic>, ApiError> {
    state.touch();
    Ok(Json(
        get_profile_public(&id).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

async fn api_put_llm_profile(
    State(state): State<DaemonState>,
    Json(body): Json<ProfileWriteBody>,
) -> Result<Json<ModelProfilePublic>, ApiError> {
    state.touch();
    let id = if body.id.trim().is_empty() {
        teshi_engine::generate_id()
    } else {
        body.id
    };
    let mut profile = ModelProfile {
        id,
        name: body.name,
        provider: body.provider,
        api_style: body.api_style,
        model_id: body.model_id,
        max_context_tokens: body.max_context_tokens,
        max_output_tokens: body.max_output_tokens,
        base_url: body.base_url,
        api_key: body.api_key,
        stream: body.stream,
        http_headers: body.http_headers,
        chat_options: body.chat_options,
    };
    Ok(Json(
        save_profile(&mut profile).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

async fn api_delete_llm_profile(
    State(state): State<DaemonState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.touch();
    delete_profile(&id).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_activate_llm_profile(
    State(state): State<DaemonState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.touch();
    set_active_id(&id).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct StepCatalogQuery {
    min_count: Option<usize>,
    top: Option<usize>,
    no_locations: Option<bool>,
}

async fn api_step_catalog(
    State(state): State<DaemonState>,
    Query(q): Query<StepCatalogQuery>,
) -> Result<Json<Value>, ApiError> {
    state.touch();
    Ok(Json(build_step_catalog(&state, q, false)?))
}

fn build_step_catalog(
    state: &DaemonState,
    q: StepCatalogQuery,
    hosted: bool,
) -> Result<Value, ApiError> {
    let root = state
        .rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    // Scan .feature files recursively
    let mut features = Vec::new();
    scan_feature_files(&root, &root, hosted, &mut features)?;

    let project = BddProject {
        root_dir: root.clone(),
        features,
    };
    let index = StepIndex::build(&project);

    let mut entries: Vec<Value> = index
        .most_common(usize::MAX)
        .into_iter()
        .filter(|(_, count)| q.min_count.is_none_or(|m| *count >= m))
        .map(|(text, count)| {
            let locations = index.usages.get(&text).map(|locs| {
                locs.iter()
                    .map(|loc| {
                        let feature = &project.features[loc.feature_idx];
                        json!({
                            "feature": feature.file_path.strip_prefix(&root).unwrap_or(&feature.file_path).to_string_lossy(),
                            "scenario": if loc.scenario_idx == usize::MAX { "<Background>".to_string() } else {
                                feature
                                    .scenario_at(loc.scenario_idx)
                                    .map(|s| s.name.clone())
                                    .unwrap_or_else(|| format!("<unknown-{}>", loc.scenario_idx))
                            },
                            "line": loc.step_idx,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

            let mut entry = json!({
                "text": text,
                "normalized": text,
                "count": count,
            });
            if !q.no_locations.unwrap_or(false) {
                entry["locations"] = json!(locations);
            }
            entry
        })
        .collect();

    // Apply top limit
    if let Some(top) = q.top {
        entries.truncate(top);
    }

    Ok(json!({
        "project_root": root.to_string_lossy(),
        "total_raw_steps": index.usages.values().map(|v| v.len()).sum::<usize>(),
        "unique_normalized": index.usages.len(),
        "num_features": project.features.len(),
        "generated_at": chrono::Local::now().to_rfc3339(),
        "entries": entries,
    }))
}

fn scan_feature_files(
    dir: &std::path::Path,
    project_root: &std::path::Path,
    hosted: bool,
    features: &mut Vec<BddFeature>,
) -> Result<(), ApiError> {
    for entry in fs::read_dir(dir).map_err(|e| ApiError::internal(e.to_string()))? {
        let entry = entry.map_err(|e| ApiError::internal(e.to_string()))?;
        let path = entry.path();
        if hosted
            && (entry
                .file_type()
                .map_err(|e| ApiError::internal(e.to_string()))?
                .is_symlink()
                || hosted_private_path(project_root, &path))
        {
            continue;
        }
        if path.is_dir() {
            scan_feature_files(&path, project_root, hosted, features)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("feature") {
            let content =
                fs::read_to_string(&path).map_err(|e| ApiError::internal(e.to_string()))?;
            let feature = teshi_core::parse_feature(&content, path);
            features.push(feature);
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct ConfirmBody {
    candidate_rank: u32,
    #[serde(default)]
    edited_value: Option<String>,
}

async fn api_confirm_locator(
    State(state): State<DaemonState>,
    Json(body): Json<ConfirmBody>,
) -> Result<StatusCode, ApiError> {
    state.touch();
    confirm_locator(&state.rt, body.candidate_rank, body.edited_value).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_reject_locator(State(state): State<DaemonState>) -> Result<StatusCode, ApiError> {
    state.touch();
    reject_locator(&state.rt).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct HighlightLocatorBody {
    selector: String,
}

async fn api_highlight_locator(
    State(state): State<DaemonState>,
    Json(body): Json<HighlightLocatorBody>,
) -> Result<StatusCode, ApiError> {
    state.touch();
    highlight_locator(&state.rt, body.selector).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct BrowserStartBody {
    mode: Option<String>,
}

const BROWSER_BROKER_DISCOVERY_URL: &str = "http://127.0.0.1:17373/v1/bridge";

fn browser_broker_client() -> Result<reqwest::Client, ApiError> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|error| ApiError::internal(format!("create browser broker client: {error}")))
}

fn browser_broker_unavailable(error: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "ok": false,
            "code": "browser_unavailable",
            "error": format!(
                "local Chrome bridge is unavailable: {error}; click Connect Chrome and reload teshi-bridge"
            ),
        })),
    )
}

async fn api_browser_sessions(
    State(state): State<DaemonState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state.touch();
    let client =
        browser_broker_client().map_err(|error| browser_broker_unavailable(error.message))?;
    let response = client
        .get(BROWSER_BROKER_DISCOVERY_URL)
        .send()
        .await
        .map_err(browser_broker_unavailable)?;
    if !response.status().is_success() {
        return Err(browser_broker_unavailable(format!(
            "broker returned HTTP {}",
            response.status()
        )));
    }
    response.json::<Value>().await.map(Json).map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "ok": false,
                "code": "invalid_browser_response",
                "error": format!("decode browser broker discovery response: {error}"),
            })),
        )
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct BrowserActivateTabBody {
    extension_instance_id: String,
    window_id: i64,
    tab_id: i64,
}

async fn api_browser_activate_tab(
    State(state): State<DaemonState>,
    Json(body): Json<BrowserActivateTabBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state.touch();
    let project_root = get_project_root(&state.rt).ok_or_else(|| {
        (
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "code": "browser_unavailable",
                "error": "no project is open in the daemon",
            })),
        )
    })?;
    let client =
        browser_broker_client().map_err(|error| browser_broker_unavailable(error.message))?;
    let response = client
        .post(format!("{BROWSER_BROKER_DISCOVERY_URL}/activate_tab"))
        .json(&json!({
            "project_root": project_root,
            "extension_instance_id": body.extension_instance_id,
            "window_id": body.window_id,
            "tab_id": body.tab_id,
        }))
        .send()
        .await
        .map_err(browser_broker_unavailable)?;
    if !response.status().is_success() {
        return Err(browser_broker_unavailable(format!(
            "broker returned HTTP {}",
            response.status()
        )));
    }
    let payload = response.json::<Value>().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "ok": false,
                "code": "invalid_browser_response",
                "error": format!("decode browser tab activation response: {error}"),
            })),
        )
    })?;
    if payload.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(Json(payload))
    } else {
        Err((StatusCode::CONFLICT, Json(payload)))
    }
}

async fn api_browser_start(
    State(state): State<DaemonState>,
    Json(body): Json<BrowserStartBody>,
) -> Result<Json<BrowserStartResult>, ApiError> {
    state.touch();
    let mode = match body.mode.as_deref() {
        Some("chrome") => BrowserMode::Chrome,
        Some("winapp") => BrowserMode::WinApp,
        _ => BrowserMode::Embedded,
    };
    start_browser_sidecar(state.rt, mode)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn api_browser_stop(State(state): State<DaemonState>) -> Result<StatusCode, ApiError> {
    state.touch();
    stop_browser_sidecar(&state.rt).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SpawnBody {
    cols: u16,
    rows: u16,
}

async fn api_terminal_spawn(
    State(state): State<DaemonState>,
    Json(body): Json<SpawnBody>,
) -> Result<StatusCode, ApiError> {
    state.touch();
    spawn_terminal(state.rt, body.cols, body.rows).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_terminal_stop(State(state): State<DaemonState>) -> Result<StatusCode, ApiError> {
    state.touch();
    teshi_engine::stop_terminal(&state.rt)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ResizeBody {
    cols: u16,
    rows: u16,
}

async fn api_terminal_resize(
    State(state): State<DaemonState>,
    Json(body): Json<ResizeBody>,
) -> Result<StatusCode, ApiError> {
    state.touch();
    resize_terminal(&state.rt, body.cols, body.rows)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct WriteBody {
    data: String,
}

async fn api_terminal_write(
    State(state): State<DaemonState>,
    Json(body): Json<WriteBody>,
) -> Result<StatusCode, ApiError> {
    state.touch();
    write_terminal(&state.rt, body.data)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_daemon_shutdown(State(state): State<DaemonState>) -> StatusCode {
    state.shutdown_token.cancel();
    StatusCode::OK
}

// ── Run endpoint ──────────────────────────────────────────────────────────

fn case_is_mixed(case: &Value) -> bool {
    let Some(path) = case.get("feature_path").and_then(Value::as_str) else {
        return false;
    };
    let Some(name) = case.get("scenario").and_then(Value::as_str) else {
        return false;
    };
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    let feature = teshi_core::parse_feature(&content, PathBuf::from(path));
    let Some(scenario) = feature
        .all_scenarios()
        .into_iter()
        .find(|item| item.name == name)
    else {
        return false;
    };
    teshi_core::scenario_engine_mode(&feature, scenario) == teshi_core::EngineMode::Mixed
}

async fn api_gherkin_scenarios(
    State(state): State<DaemonState>,
) -> Result<Json<Vec<teshi_engine::RunnableScenario>>, ApiError> {
    state.touch();
    let project_root = state
        .rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| ApiError::internal("no project open"))?;
    Ok(Json(list_runnable_scenarios(&project_root)))
}

#[derive(Deserialize)]
struct ExchangeApiBody {
    exchange_id: String,
    redact: Option<bool>,
}

async fn api_get_exchange(
    State(state): State<DaemonState>,
    Json(body): Json<ExchangeApiBody>,
) -> Result<Json<Value>, ApiError> {
    state.touch();
    let project_root = state
        .rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| ApiError::internal("no project open"))?;
    let command = json!({
        "cmd": "get_exchange",
        "request_id": "daemon-exchange",
        "exchange_id": body.exchange_id,
        "redact": body.redact.unwrap_or(true),
    });
    let response = tokio::task::spawn_blocking(move || {
        send_api_command(&project_root, command, std::time::Duration::from_secs(5))
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(response))
}

async fn dispatch_run_ndjson(
    project_root: PathBuf,
    cases: Vec<Value>,
) -> Result<Response, ApiError> {
    let dispatch: Vec<DispatchCase> = cases
        .iter()
        .filter_map(|case| {
            Some(DispatchCase {
                id: case.get("id")?.as_str()?.to_string(),
                feature_path: PathBuf::from(case.get("feature_path")?.as_str()?),
                scenario: case.get("scenario")?.as_str()?.to_string(),
            })
        })
        .collect();
    let script = default_api_service_script();
    let lines = tokio::task::spawn_blocking(move || {
        let mut lines = Vec::new();
        dispatch_cases(&project_root, &script, &dispatch, |value| {
            lines.push(value.to_string());
        })
        .map(|_| lines)
    })
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let body = lines.join("\n") + "\n";
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/x-ndjson")
        .body(axum::body::Body::from(body))
        .map_err(|e| ApiError::internal(e.to_string()))
}

#[derive(Deserialize)]
struct RunApiBody {
    feature_path: Option<String>,
    scenario: Option<String>,
    scenario_ids: Option<Vec<String>>,
}

/// POST /api/v1/daemon/run — execute BDD scenarios via the NDJSON runner.
async fn api_run(
    State(state): State<DaemonState>,
    Json(body): Json<RunApiBody>,
) -> Result<Response, ApiError> {
    state.touch();

    let project_root = state
        .rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| ApiError::internal("no project open"))?;

    // Build the feature path
    let feature_path = if let Some(ref fp) = body.feature_path {
        project_root.join(fp)
    } else {
        project_root.clone()
    };

    // Collect cases from feature file(s)
    let mut cases = Vec::new();
    if feature_path.is_dir() {
        let project = teshi_core::parse_project(&feature_path);
        for (fi, feature) in project.features.iter().enumerate() {
            for (si, scenario) in feature.all_scenarios().into_iter().enumerate() {
                if let Some(ref name) = body.scenario {
                    if scenario.name != *name {
                        continue;
                    }
                }
                let until_line = scenario.steps.last().map(|s| s.line_number);
                cases.push(serde_json::json!({
                    "id": format!("f{fi}:s{si}"),
                    "feature_path": feature.file_path.to_string_lossy(),
                    "scenario": scenario.name,
                    "line_number": scenario.line_number,
                    "until_line": until_line,
                }));
            }
        }
    } else {
        let content = std::fs::read_to_string(&feature_path)
            .map_err(|e| ApiError::internal(format!("read feature: {e}")))?;
        let feature = teshi_core::parse_feature(&content, feature_path.clone());
        for (si, scenario) in feature.all_scenarios().into_iter().enumerate() {
            if let Some(ref name) = body.scenario {
                if scenario.name != *name {
                    continue;
                }
            }
            let until_line = scenario.steps.last().map(|s| s.line_number);
            cases.push(serde_json::json!({
                "id": format!("s{si}"),
                "feature_path": feature.file_path.to_string_lossy(),
                "scenario": scenario.name,
                "line_number": scenario.line_number,
                "until_line": until_line,
            }));
        }
    }

    if cases.is_empty() {
        return Err(ApiError::internal("no scenarios found"));
    }

    if let Some(ids) = &body.scenario_ids {
        cases.retain(|case| {
            case.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| ids.iter().any(|want| want == id))
        });
        if cases.is_empty() {
            return Err(ApiError::internal("no scenarios matched scenario_ids"));
        }
    }

    let teshi_dispatch = body.scenario_ids.is_some() || cases.iter().any(case_is_mixed);
    if teshi_dispatch {
        return dispatch_run_ndjson(project_root, cases).await;
    }

    let request = serde_json::json!({
        "command": "run",
        "cases": cases,
        "meta": {
            "project_root": project_root.to_string_lossy().to_string(),
        }
    });

    // Load runner config from project's teshi.toml
    let (runner_cmd, runner_args) = load_daemon_runner_config(&project_root)?;

    let mut child = tokio::process::Command::new(&runner_cmd)
        .args(&runner_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| ApiError::internal(format!("spawn runner: {e}")))?;

    // Write the NDJSON request to stdin
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let payload = serde_json::to_string(&request).unwrap();
        let _ = stdin.write_all(payload.as_bytes()).await;
        let _ = stdin.write_all(b"\n").await;
        drop(stdin);
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ApiError::internal("no runner stdout"))?;

    // Stream stdout back as NDJSON
    use axum::body::Body;
    use tokio_util::io::ReaderStream;

    let stream = ReaderStream::new(stdout);
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/x-ndjson")
        .body(body)
        .map_err(|e| ApiError::internal(e.to_string()))
}

/// Load runner config from project's `teshi.toml` for daemon use.
///
/// Resolution order: `teshi.toml` `[runner]` → env overrides (`TESHI_RUNNER_CMD`,
/// `TESHI_RUNNER_ARGS`) — mirrors `runner::load_runner_config`.
fn load_daemon_runner_config(
    project_root: &std::path::Path,
) -> Result<(String, Vec<String>), ApiError> {
    let config_path = project_root.join("teshi.toml");
    let default_cmd = "teshi-runner".to_string();

    let (cmd, args) = if let Ok(raw) = std::fs::read_to_string(&config_path) {
        let val: toml::Value = match toml::from_str(&raw) {
            Ok(v) => v,
            Err(_) => {
                return Ok((default_cmd, vec![]));
            }
        };
        let r = val.get("runner");
        let c = r
            .and_then(|v| v.get("cmd"))
            .and_then(|v| v.as_str())
            .unwrap_or(&default_cmd)
            .to_string();
        let a: Vec<String> = r
            .and_then(|v| v.get("args"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        (c, a)
    } else {
        (default_cmd, vec![])
    };

    // Allow env overrides
    let cmd = std::env::var("TESHI_RUNNER_CMD").unwrap_or(cmd);
    let args = std::env::var("TESHI_RUNNER_ARGS")
        .ok()
        .map(|s| s.split_whitespace().map(|v| v.to_string()).collect())
        .unwrap_or(args);

    Ok((cmd, args))
}

// ── ApiError ──────────────────────────────────────────────────────────────
#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl From<String> for ApiError {
    fn from(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }
}

impl From<BrowserError> for ApiError {
    fn from(err: BrowserError) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: serde_json::to_string(&err).unwrap_or(err.message),
        }
    }
}

impl ApiError {
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            [(header::CONTENT_TYPE, "application/json")],
            Json(json!({ "error": self.message })),
        )
            .into_response()
    }
}
// ── Integration tests (calls router directly via clone+oneshot) ─────────────

#[cfg(test)]
mod integration {
    use super::*;
    use axum::body::Body;
    use axum::http::{HeaderValue, Request, StatusCode};
    use http_body_util::BodyExt;
    use teshi_engine::{DaemonManifest, DaemonManifestExt};
    use tower::ServiceExt;

    fn test_state() -> DaemonState {
        let rt = teshi_engine::TeshiEngine::new(
            teshi_engine::RuntimeConfig {
                browser_service_script: PathBuf::from(""),
                winapp_service_script: PathBuf::from(""),
                embedded_no_preview_stream: false,
                requirements_root: None,
            },
            None,
        );
        DaemonState {
            rt,
            sessions: SessionStore::new(),
            active_ws: Arc::new(AtomicUsize::new(0)),
            last_request: Arc::new(StdMutex::new(Instant::now())),
            shutdown_token: CancellationToken::new(),
            active_control_sessions: Arc::new(StdMutex::new(HashSet::new())),
            hosted_session_gate: Arc::new(tokio::sync::RwLock::new(())),
        }
    }

    #[test]
    fn hosted_browser_start_response_drops_private_sidecar_coordinates() {
        let value = hosted_browser_start_value(BrowserStartResult {
            ws_url: "ws://127.0.0.1:17373/?token=secret".into(),
            cdp_endpoint_path: "C:\\project\\.teshi\\cdp-endpoint.json".into(),
            mode: "chrome".into(),
        });
        assert_eq!(value, json!({ "started": true, "mode": "chrome" }));
        assert!(!value.to_string().contains("secret"));
        assert!(!value.to_string().contains("cdp-endpoint"));
    }

    #[test]
    fn hosted_browser_session_list_is_allowlisted() {
        let value = hosted_browser_sessions_value(json!({
            "schema_version": 2,
            "ws_url": "ws://127.0.0.1:17373/?token=secret",
            "extension_frame_ws_url": "ws://127.0.0.1:17373/frames?token=secret",
            "project_root": "C:/private/project",
            "extension_connected": true,
            "ambiguous_browser_target": false,
            "sessions": [{
                "identity": {
                    "extension_instance_id": "ext-1",
                    "profile_label": "Default",
                    "extension_version": "1.0",
                    "protocol_version": 1
                },
                "browser": {"name": "Chrome", "version": "1", "platform": "windows"},
                "health": "ready",
                "last_heartbeat_age_ms": 4,
                "windows": [{"id": 7, "focused": true, "tabs": [{
                    "id": 8,
                    "window_id": 7,
                    "title": "Teshi",
                    "url": "https://example.test",
                    "active": true,
                    "debuggable": true
                }]}],
                "lease": null,
                "capabilities": {"command_token": "secret"}
            }]
        }))
        .expect("broker discovery should match the public session contract");
        let text = value.to_string();
        assert!(!text.contains("ws_url"));
        assert!(!text.contains("extension_frame_ws_url"));
        assert!(!text.contains("project_root"));
        assert!(!text.contains("secret"));
        assert_eq!(
            value["sessions"][0]["identity"]["extension_instance_id"],
            "ext-1"
        );
        assert_eq!(value["sessions"][0]["windows"][0]["tabs"][0]["id"], 8);
    }

    #[test]
    fn hosted_runtime_event_redaction_is_recursive() {
        let value = redact_hosted_event_value(json!({
            "mode": "chrome",
            "ws_url": "ws://127.0.0.1:1/?token=secret",
            "nested": [{"command_token": "secret", "ok": true}],
            "project_root": "C:/private/project"
        }));
        assert_eq!(value, json!({ "mode": "chrome", "nested": [{"ok": true}] }));
    }

    #[tokio::test]
    async fn shutdown_endpoint_releases_listener_and_cleans_manifest() {
        let project_root =
            std::env::temp_dir().join(format!("teshi-daemon-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(project_root.join(".teshi")).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        DaemonManifest {
            pid: std::process::id(),
            port: address.port(),
            started: chrono::Utc::now(),
        }
        .save_manifest(&project_root)
        .unwrap();

        let state = test_state();
        let server = tokio::spawn(run_server_with_listener(
            listener,
            state.rt,
            project_root.join("missing-web-dist"),
            Some(project_root.clone()),
        ));
        let client = reqwest::Client::new();
        let url = format!("http://{}/api/v1/daemon/shutdown", address);
        let mut shutdown_accepted = false;
        for _ in 0..40 {
            if let Ok(response) = client.post(&url).send().await {
                if response.status().is_success() {
                    shutdown_accepted = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(shutdown_accepted, "daemon did not accept shutdown request");
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .expect("daemon shutdown timed out")
            .expect("daemon task panicked")
            .expect("daemon server failed");
        assert!(!DaemonManifest::manifest_path(&project_root).exists());
        assert!(std::net::TcpStream::connect_timeout(
            &address,
            std::time::Duration::from_millis(200)
        )
        .is_err());
        std::fs::remove_dir_all(project_root).unwrap();
    }

    fn build_router(state: DaemonState) -> Router {
        let public = Router::new()
            .route("/api/v1/sessions", post(api_create_session))
            .route("/api/v1/sessions/{token}", get(api_get_session))
            .route("/api/v1/sessions/{token}", delete(api_delete_session))
            .route_layer(middleware::from_fn(loopback_only))
            .route_layer(middleware::from_fn(same_origin_only));

        let protected = Router::new()
            .route("/api/v1/_ping", get(|| async { "pong" }))
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .route_layer(middleware::from_fn(same_origin_only));

        let hosted_ws = Router::new()
            .route("/ws/control", get(control_ws))
            .route("/ws/preview", get(preview_ws))
            .route_layer(middleware::from_fn(trusted_hosted_origin_only));

        Router::new()
            .merge(public)
            .merge(protected)
            .merge(hosted_ws)
            .with_state(state)
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn build_req(method: &str, uri: &str, body: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(body_str) = body {
            b = b.header("content-length", body_str.len().to_string());
        }
        let mut req = b.body(Body::from(body.unwrap_or("").to_string())).unwrap();
        req.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:41000".parse::<SocketAddr>().unwrap(),
        ));
        req
    }

    fn with_token(req: Request<Body>, token: &str) -> Request<Body> {
        let (mut parts, body) = req.into_parts();
        parts
            .headers
            .insert("x-teshi-token", HeaderValue::from_str(token).unwrap());
        Request::from_parts(parts, body)
    }

    fn from_remote(mut req: Request<Body>) -> Request<Body> {
        req.extensions_mut().insert(ConnectInfo(
            "192.0.2.10:41000".parse::<SocketAddr>().unwrap(),
        ));
        req
    }

    async fn exec(router: &mut Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let resp = router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let val: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap_or_default();
        (status, val)
    }

    async fn exec_status(router: &mut Router, req: Request<Body>) -> StatusCode {
        let resp = router.clone().oneshot(req).await.unwrap();
        resp.status()
    }

    #[tokio::test]
    async fn cors_preflight_preserves_non_llm_post_and_delete_support() {
        let router = Router::new()
            .route("/mutation", post(|| async { StatusCode::NO_CONTENT }))
            .layer(browser_cors_layer());

        for method in ["GET", "POST", "DELETE"] {
            let request = Request::builder()
                .method("OPTIONS")
                .uri("/mutation")
                .header("origin", "https://attacker.example")
                .header("access-control-request-method", method)
                .body(Body::empty())
                .unwrap();
            let response = router.clone().oneshot(request).await.unwrap();
            let allowed = response
                .headers()
                .get("access-control-allow-methods")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            assert!(allowed.split(',').any(|value| value.trim() == method));
        }

        let put_preflight = Request::builder()
            .method("OPTIONS")
            .uri("/mutation")
            .header("origin", "https://client.example")
            .header("access-control-request-method", "PUT")
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(put_preflight).await.unwrap();
        let allowed = response
            .headers()
            .get("access-control-allow-methods")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(!allowed.split(',').any(|value| value.trim() == "PUT"));
    }

    fn websocket_upgrade_request(path: &str, origin: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(path)
            .header("origin", origin)
            .header("host", "127.0.0.1:41000")
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn hosted_websocket_origin_is_exactly_allowlisted() {
        let router = build_router(test_state());
        for origin in ["https://teshi.org", "https://teshi-org.github.io"] {
            let accepted = router
                .clone()
                .oneshot(websocket_upgrade_request("/ws/control", origin))
                .await
                .unwrap();
            // Axum may reject the synthetic in-process upgrade with 426
            // because there is no live upgrade transport; origin middleware
            // must still allow it past the trust gate (i.e. not return 403).
            assert_ne!(accepted.status(), StatusCode::FORBIDDEN);
        }

        for origin in [
            "https://attacker.example",
            "https://www.teshi.org",
            "http://teshi.org",
            "null",
        ] {
            let rejected = router
                .clone()
                .oneshot(websocket_upgrade_request("/ws/preview", origin))
                .await
                .unwrap();
            assert_eq!(rejected.status(), StatusCode::FORBIDDEN, "origin={origin}");
        }
        let missing_origin = Request::builder()
            .method("GET")
            .uri("/ws/control")
            .header("host", "127.0.0.1:41000")
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router.oneshot(missing_origin).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn hosted_handshake_rejects_unknown_token_before_business_logic() {
        let state = test_state();
        let hello = ClientHello {
            token: "tk_missing".into(),
            channel: Channel::Control,
            protocol_version: CONTROL_PROTOCOL_VERSION,
            ui: teshi_web_protocol::UiCompatibility {
                manifest_schema: teshi_web_protocol::MANIFEST_SCHEMA_VERSION,
                ui_source_sha: "b".repeat(40),
                minimum_cli: daemon_build_identity(),
                minimum_build_sequence: 0,
                control_protocol: CONTROL_PROTOCOL_VERSION,
                preview_protocol: PREVIEW_PROTOCOL_VERSION,
            },
        };
        let error = authenticate_hello(&state, hello, Channel::Control)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidToken);
    }

    #[tokio::test]
    async fn hosted_handshake_rejects_a_token_after_session_teardown() {
        let state = test_state();
        let token = state.sessions.create_hosted_session();
        state.sessions.remove_session(&token);
        let hello = ClientHello {
            token,
            channel: Channel::Control,
            protocol_version: CONTROL_PROTOCOL_VERSION,
            ui: teshi_web_protocol::UiCompatibility {
                manifest_schema: teshi_web_protocol::MANIFEST_SCHEMA_VERSION,
                ui_source_sha: "b".repeat(40),
                minimum_cli: daemon_build_identity(),
                minimum_build_sequence: 1,
                control_protocol: CONTROL_PROTOCOL_VERSION,
                preview_protocol: PREVIEW_PROTOCOL_VERSION,
            },
        };
        let error = authenticate_hello(&state, hello, Channel::Control)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidToken);
    }

    #[tokio::test]
    async fn hosted_launch_replaces_old_token_and_runtime_teardown_invalidates_it() {
        let state = test_state();
        let admin = state.sessions.create_session(Role::Admin, None);

        let first = api_create_session(
            State(state.clone()),
            Json(CreateSessionBody {
                role: "hosted_web_ui".into(),
                metadata: None,
            }),
        )
        .await
        .unwrap()
        .0
        .token;
        let second = api_create_session(
            State(state.clone()),
            Json(CreateSessionBody {
                role: "hosted_web_ui".into(),
                metadata: None,
            }),
        )
        .await
        .unwrap()
        .0
        .token;

        assert_ne!(first, second);
        assert!(state.sessions.get_session(&first).is_none());
        assert!(state.sessions.get_session(&second).is_some());

        api_teardown(State(state.clone())).await.unwrap();

        assert!(state.sessions.get_session(&second).is_none());
        assert!(state.sessions.get_session(&admin).is_some());
    }

    #[tokio::test]
    async fn control_dispatch_returns_correlated_capability_errors() {
        let state = test_state();
        let error = dispatch_control_request(&state, "unknown.method", json!({}))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownMethod);
        let allowed = dispatch_control_request(&state, "project.switch_allowed", json!({}))
            .await
            .unwrap();
        assert!(allowed.is_boolean());
        let forbidden = dispatch_control_request(&state, "runtime.shutdown", json!({}))
            .await
            .unwrap_err();
        assert_eq!(forbidden.code, ErrorCode::Forbidden);
    }

    #[test]
    fn hosted_capability_matrix_is_explicit_and_namespaced() {
        assert_eq!(
            control_method_capability("project.open"),
            Some(HostedCapability::Project)
        );
        assert_eq!(
            control_method_capability("bdd.run"),
            Some(HostedCapability::BddRun)
        );
        assert_eq!(
            control_method_capability("terminal.write"),
            Some(HostedCapability::Terminal)
        );
        assert_eq!(control_method_capability("runtime.shutdown"), None);
        assert_eq!(control_method_capability("admin.delete_everything"), None);
    }

    #[test]
    fn control_session_ownership_is_single_and_cleanup_is_deterministic() {
        let active = Arc::new(StdMutex::new(HashSet::new()));
        assert!(claim_control_session(&active, "tk_one"));
        assert!(!claim_control_session(&active, "tk_one"));
        assert!(claim_control_session(&active, "tk_two"));
        release_control_session(&active, "tk_one");
        assert!(claim_control_session(&active, "tk_one"));
        release_control_session(&active, "tk_one");
        release_control_session(&active, "tk_two");
        assert!(active.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn control_dispatch_supports_concurrent_independent_requests() {
        let state = test_state();
        let (first, second) = tokio::join!(
            dispatch_control_request(&state, "project.switch_allowed", json!({})),
            dispatch_control_request(&state, "project.switch_allowed", json!({})),
        );
        assert!(first.unwrap().is_boolean());
        assert!(second.unwrap().is_boolean());
    }

    #[tokio::test]
    async fn bounded_control_queues_preserve_rpc_priority_and_report_overflow() {
        let (tx, mut rx) =
            tokio::sync::mpsc::channel::<ServerMessage>(CONTROL_RESPONSE_QUEUE_CAPACITY);
        for index in 0..CONTROL_RESPONSE_QUEUE_CAPACITY {
            tx.try_send(ServerMessage::Event {
                event: "ordered".into(),
                payload: json!({ "index": index }),
            })
            .unwrap();
        }
        assert!(tx
            .try_send(ServerMessage::Event {
                event: "ordered".into(),
                payload: json!({ "index": CONTROL_RESPONSE_QUEUE_CAPACITY }),
            })
            .is_err());
        for index in 0..CONTROL_RESPONSE_QUEUE_CAPACITY {
            let ServerMessage::Event { payload, .. } = rx.recv().await.unwrap() else {
                panic!("expected ordered event")
            };
            assert_eq!(payload["index"], index);
        }

        let (event_tx, _event_rx) =
            tokio::sync::mpsc::channel::<ServerMessage>(CONTROL_EVENT_QUEUE_CAPACITY);
        for index in 0..CONTROL_EVENT_QUEUE_CAPACITY {
            event_tx
                .try_send(ServerMessage::Event {
                    event: "burst".into(),
                    payload: json!({ "index": index }),
                })
                .unwrap();
        }
        // Filling the event queue cannot consume response capacity.
        let (response_tx, mut response_rx) =
            tokio::sync::mpsc::channel::<ServerMessage>(CONTROL_RESPONSE_QUEUE_CAPACITY);
        response_tx
            .try_send(ServerMessage::Response {
                id: "priority".into(),
                ok: true,
                result: Some(json!({ "ok": true })),
                error: None,
            })
            .unwrap();
        assert!(matches!(
            response_rx.recv().await,
            Some(ServerMessage::Response { id, .. }) if id == "priority"
        ));

        let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel(1);
        broadcast_tx.send(json!("first")).unwrap();
        broadcast_tx.send(json!("second")).unwrap();
        assert!(matches!(
            broadcast_rx.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(1))
        ));
    }

    #[tokio::test]
    async fn stalled_preview_consumer_keeps_frames_bounded_and_control_ready() {
        // A watch slot retains only the newest frame even when a consumer is
        // completely stalled. This models the relay's large-frame path with a
        // burst much larger than the production queue limits.
        let (frame_tx, mut frame_rx) = tokio::sync::watch::channel(None::<String>);
        for index in 0..100_000usize {
            frame_tx.send_replace(Some(format!("frame-{index}")));
        }
        assert_eq!(frame_rx.borrow_and_update().as_deref(), Some("frame-99999"));

        // Non-frame sidecar messages are explicitly bounded. A stalled
        // downstream cannot turn an upstream burst into unbounded memory.
        let (preview_tx, _preview_rx) =
            tokio::sync::mpsc::channel::<PreviewRelayMessage>(PREVIEW_CONTROL_QUEUE_CAPACITY);
        for index in 0..PREVIEW_CONTROL_QUEUE_CAPACITY {
            preview_tx
                .try_send(PreviewRelayMessage::Text(format!("control-{index}")))
                .unwrap();
        }
        assert!(matches!(
            preview_tx.try_send(PreviewRelayMessage::Text("overflow".into())),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_))
        ));

        // Preview saturation is independent of control response delivery. A
        // local response must remain observable within the recorded 100 ms
        // acceptance budget even while the event path is full.
        let (event_tx, _event_rx) =
            tokio::sync::mpsc::channel::<ServerMessage>(CONTROL_EVENT_QUEUE_CAPACITY);
        for index in 0..CONTROL_EVENT_QUEUE_CAPACITY {
            event_tx
                .try_send(ServerMessage::Event {
                    event: "preview.burst".into(),
                    payload: json!({ "index": index }),
                })
                .unwrap();
        }
        let (response_tx, mut response_rx) =
            tokio::sync::mpsc::channel::<ServerMessage>(CONTROL_RESPONSE_QUEUE_CAPACITY);
        response_tx
            .try_send(ServerMessage::Response {
                id: "preview-isolated".into(),
                ok: true,
                result: Some(json!({ "ok": true })),
                error: None,
            })
            .unwrap();
        let response =
            tokio::time::timeout(std::time::Duration::from_millis(100), response_rx.recv())
                .await
                .expect("control response exceeded preview isolation budget")
                .expect("control response queue closed");
        assert!(matches!(
            response,
            ServerMessage::Response { id, ok: true, .. } if id == "preview-isolated"
        ));
    }

    #[test]
    fn idle_shutdown_requires_no_active_websocket_and_exceeds_timeout() {
        assert!(!daemon_should_shutdown_for_idle(
            1,
            DAEMON_IDLE_TIMEOUT + std::time::Duration::from_secs(1)
        ));
        assert!(!daemon_should_shutdown_for_idle(0, DAEMON_IDLE_TIMEOUT));
        assert!(daemon_should_shutdown_for_idle(
            0,
            DAEMON_IDLE_TIMEOUT + std::time::Duration::from_secs(1)
        ));
    }

    #[tokio::test]
    async fn mutation_origin_guard_rejects_simple_cross_origin_post() {
        let router = Router::new()
            .route("/activate", post(|| async { StatusCode::NO_CONTENT }))
            .route_layer(middleware::from_fn(same_origin_only));

        let cross_origin = Request::builder()
            .method("POST")
            .uri("/activate")
            .header("host", "127.0.0.1:3000")
            .header("origin", "https://attacker.example")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router.clone().oneshot(cross_origin).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );

        let wrong_scheme = Request::builder()
            .method("POST")
            .uri("/activate")
            .header("host", "127.0.0.1:3000")
            .header("origin", "https://127.0.0.1:3000")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router.clone().oneshot(wrong_scheme).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );

        let same_origin = Request::builder()
            .method("POST")
            .uri("/activate")
            .header("host", "127.0.0.1:3000")
            .header("origin", "http://127.0.0.1:3000")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router.clone().oneshot(same_origin).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );

        let non_browser = Request::builder()
            .method("POST")
            .uri("/activate")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router.oneshot(non_browser).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn preview_origin_guard_accepts_forwarded_https_same_origin() {
        let router = Router::new()
            .route(
                "/preview",
                get(|| async { StatusCode::SWITCHING_PROTOCOLS }),
            )
            .route_layer(middleware::from_fn(same_origin_only));

        let same_origin = Request::builder()
            .uri("/preview")
            .header("host", "teshi.example:443")
            .header("origin", "https://teshi.example:443")
            .header("x-forwarded-proto", "https")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router.clone().oneshot(same_origin).await.unwrap().status(),
            StatusCode::SWITCHING_PROTOCOLS
        );

        let cross_origin = Request::builder()
            .uri("/preview")
            .header("host", "teshi.example:443")
            .header("origin", "https://attacker.example")
            .header("x-forwarded-proto", "https")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router.oneshot(cross_origin).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn preview_frame_classifier_only_selects_frame_messages() {
        assert!(is_preview_frame(r#"{"type":"frame","data":"jpeg"}"#));
        assert!(is_preview_frame(
            r#"{"type":"frame","data":"jpeg","capture_backend":"wgc"}"#
        ));
        assert!(!is_preview_frame(
            r#"{"type":"response","request_id":"gpui-preview-attach"}"#
        ));
        assert!(!is_preview_frame(r#"{"type":"frame_error"}"#));
        assert!(!is_preview_frame("not json"));
    }

    #[test]
    fn preview_relay_allowlist_drops_sidecar_metadata_and_scrubs_url() {
        let message = sanitize_preview_message(
            r#"{"type":"frame","data":"jpeg","url":"https://user:pw@example.test/?token=secret#x","target":{"project_root":"C:\\private"},"extension_instance_id":"ext-1","tab_id":7,"capture_backend":"C:\\private"}"#,
        )
        .expect("frame should be accepted");
        let value: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(value["type"], "frame");
        assert_eq!(value["data"], "jpeg");
        assert_eq!(value["url"], "https://example.test/");
        assert_eq!(value["tab_id"], 7);
        assert!(value.get("target").is_none());
        assert!(value.get("extension_instance_id").is_none());
        assert!(value.get("capture_backend").is_none());
        assert!(!message.contains("secret"));

        let private_scheme = sanitize_preview_message(
            r#"{"type":"frame","data":"jpeg","url":"file:///C:/private/secret.png"}"#,
        )
        .expect("frame with a private URL scheme should be normalized");
        let private_value: Value = serde_json::from_str(&private_scheme).unwrap();
        assert_eq!(private_value["url"], "<redacted-url>");
    }

    #[test]
    fn preview_relay_drops_unknown_messages_and_normalizes_errors() {
        assert!(sanitize_preview_message(
            r#"{"type":"response","request_id":"secret","url":"https://example.test/?token=secret"}"#
        )
        .is_none());
        let error =
            sanitize_preview_message(r#"{"type":"frame_error","error":"C:\\private\\stderr.log"}"#)
                .map(|value| serde_json::from_str::<Value>(&value).unwrap());
        assert_eq!(
            error,
            Some(json!({
                "type": "frame_error",
                "error": "preview capture failed"
            }))
        );
    }

    #[test]
    fn preview_stream_accepts_chrome_embedded_and_winapp() {
        assert!(preview_stream_supported(Some(BrowserMode::Chrome)));
        assert!(preview_stream_supported(Some(BrowserMode::Embedded)));
        assert!(preview_stream_supported(Some(BrowserMode::WinApp)));
        assert!(!preview_stream_supported(None));
    }

    #[test]
    fn file_reads_are_confined_to_the_open_project() {
        let base = std::env::temp_dir().join(format!("teshi-daemon-{}", uuid::Uuid::new_v4()));
        let project = base.join("project");
        fs::create_dir_all(&project).unwrap();
        let inside = project.join("feature.txt");
        let outside = base.join("secret.txt");
        fs::write(&inside, "feature").unwrap();
        fs::write(&outside, "secret").unwrap();

        assert_eq!(read_project_file(&project, &inside).unwrap(), "feature");
        assert_eq!(
            read_project_file(&project, &outside).unwrap_err().0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            read_project_file(&project, &project.join("missing.txt"))
                .unwrap_err()
                .0,
            StatusCode::NOT_FOUND
        );

        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn hosted_value_projection_hides_private_paths_and_secrets() {
        let base =
            std::env::temp_dir().join(format!("teshi-hosted-projection-{}", uuid::Uuid::new_v4()));
        let project = base.join("project");
        fs::create_dir_all(project.join("features")).unwrap();
        let feature = project.join("features/login.feature");
        fs::write(&feature, "Feature: Login").unwrap();
        let value = redact_hosted_value(
            json!({
                "root": project.to_string_lossy(),
                "feature_path": feature.to_string_lossy(),
                "url": "https://user:pw@example.test/?access_token=secret",
                "request_headers": {
                    "Authorization": "Bearer secret",
                    "X-Api-Key": "secret",
                },
                "request_body": "password=secret",
                "nested": feature.to_string_lossy(),
            }),
            Some(&project),
        );
        assert!(value.get("root").is_none());
        assert_eq!(value["feature_path"], "features/login.feature");
        assert_eq!(value["url"], "https://example.test/");
        assert_eq!(value["request_headers"]["Authorization"], "***");
        assert_eq!(value["request_headers"]["X-Api-Key"], "***");
        assert_eq!(value["request_body"], "<redacted-body>");
        assert_eq!(value["nested"], "features/login.feature");
        assert!(!value.to_string().contains("secret"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn hosted_project_paths_reject_private_metadata() {
        let base =
            std::env::temp_dir().join(format!("teshi-hosted-files-{}", uuid::Uuid::new_v4()));
        let project = base.join("project");
        fs::create_dir_all(project.join(".teshi")).unwrap();
        fs::write(project.join(".teshi/cdp-endpoint.json"), "secret").unwrap();
        let root = project.canonicalize().unwrap();
        let error = resolve_hosted_project_path(&root, ".teshi/cdp-endpoint.json").unwrap_err();
        assert_eq!(error.code, ErrorCode::Forbidden);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn hosted_project_paths_reject_common_tool_credentials() {
        let base =
            std::env::temp_dir().join(format!("teshi-hosted-credentials-{}", uuid::Uuid::new_v4()));
        let project = base.join("project");
        fs::create_dir_all(&project).unwrap();
        for name in [
            ".npmrc",
            ".netrc",
            ".pypirc",
            ".aws",
            ".docker",
            ".kube",
            "id_ecdsa",
            "id_dsa",
            "credentials.json",
            "service-account.json",
        ] {
            fs::write(project.join(name), "secret").unwrap();
            let root = project.canonicalize().unwrap();
            let error = resolve_hosted_project_path(&root, name).unwrap_err();
            assert_eq!(error.code, ErrorCode::Forbidden, "{name}");
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn hosted_step_catalog_skips_private_features() {
        let base =
            std::env::temp_dir().join(format!("teshi-hosted-catalog-{}", uuid::Uuid::new_v4()));
        let project = base.join("project");
        fs::create_dir_all(project.join("features")).unwrap();
        fs::create_dir_all(project.join(".teshi")).unwrap();
        fs::write(
            project.join("features/public.feature"),
            "Feature: Public\n  Scenario: Visible\n    Given a public step\n",
        )
        .unwrap();
        fs::write(
            project.join(".teshi/private.feature"),
            "Feature: Private\n  Scenario: Hidden\n    Given password secret-value\n",
        )
        .unwrap();
        let state = test_state();
        *state.rt.project.root.lock().unwrap() = Some(project.clone());
        let value = build_step_catalog(
            &state,
            StepCatalogQuery {
                min_count: None,
                top: None,
                no_locations: None,
            },
            true,
        )
        .unwrap();
        let text = value.to_string();
        assert!(text.contains("a public step"));
        assert!(!text.contains("password secret-value"));
        assert!(!text.contains(".teshi"));
        fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn hosted_session_invalidation_waits_for_in_flight_requests() {
        let state = test_state();
        let token = state.sessions.create_hosted_session();
        let guard = state.hosted_session_gate.read().await;
        let teardown_state = state.clone();
        let teardown = tokio::spawn(async move { api_teardown(State(teardown_state)).await });

        tokio::task::yield_now().await;
        assert!(!teardown.is_finished());
        drop(guard);
        teardown.await.unwrap().unwrap();
        assert!(state.sessions.get_session(&token).is_none());
    }

    #[test]
    fn hosted_control_errors_do_not_include_local_diagnostic_paths() {
        let protocol = control_api_error(ApiError::from(BrowserError {
            message: "browser_service.py not found at C:\\private\\browser_service.py".into(),
            hint: Some("Inspect C:\\private\\stderr.log".into()),
        }));
        assert!(!protocol.message.contains("C:\\private"));
        assert!(protocol.message.contains("<local-path>"));

        let unix_protocol = control_api_error(ApiError::from(BrowserError {
            message: "failed to open /home/user/project/a.feature".into(),
            hint: None,
        }));
        assert!(!unix_protocol.message.contains("/home/user/project"));
        assert!(unix_protocol.message.contains("<local-path>"));

        let endpoint_protocol = control_api_error(ApiError::from(BrowserError {
            message: "connect ws://127.0.0.1:17373/?token=secret failed".into(),
            hint: None,
        }));
        assert!(!endpoint_protocol.message.contains("127.0.0.1"));
        assert!(!endpoint_protocol.message.contains("secret"));
    }

    // ── Main test ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn all_scenarios() {
        let state = test_state();
        let sessions = state.sessions.clone();
        let mut router = build_router(state);

        // ── 1. Create AgentRecorder session ──────────────────────────────────
        let (status, body) = exec(
            &mut router,
            build_req(
                "POST",
                "/api/v1/sessions",
                Some(r#"{"role":"agent_recorder"}"#),
            ),
        )
        .await;
        assert_eq!(status, 200);
        let token_a = body["token"].as_str().unwrap().to_string();
        assert!(token_a.starts_with("tk_"), "token prefix tk_");
        assert_eq!(body["role"], "AgentRecorder");

        // ── 2. Create Admin session ──────────────────────────────────────────
        let (status, body) = exec(
            &mut router,
            build_req("POST", "/api/v1/sessions", Some(r#"{"role":"admin"}"#)),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["role"], "Admin");

        // ── 3. Read session ──────────────────────────────────────────────────
        let (status, body) = exec(
            &mut router,
            build_req("GET", &format!("/api/v1/sessions/{token_a}"), None),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["role"], "AgentRecorder");
        assert!(body["created_at_secs"].as_f64().unwrap() > 0.0);

        // ── 4. Read unknown → 404 ────────────────────────────────────────────
        let status = exec_status(
            &mut router,
            build_req("GET", "/api/v1/sessions/tk_doesnotexist", None),
        )
        .await;
        assert_eq!(status, 404);

        // ── 5. Delete session ────────────────────────────────────────────────
        let status = exec_status(
            &mut router,
            build_req("DELETE", &format!("/api/v1/sessions/{token_a}"), None),
        )
        .await;
        assert_eq!(status, 204);

        // ── 6. Read after delete → 404 ───────────────────────────────────────
        let status = exec_status(
            &mut router,
            build_req("GET", &format!("/api/v1/sessions/{token_a}"), None),
        )
        .await;
        assert_eq!(status, 404);

        // ── 7. Invalid role → 400 ────────────────────────────────────────────
        let status = exec_status(
            &mut router,
            build_req(
                "POST",
                "/api/v1/sessions",
                Some(r#"{"role":"super_admin"}"#),
            ),
        )
        .await;
        assert_eq!(status, 400);

        // ── 8. No token → Admin → allowed ────────────────────────────────────
        let status = exec_status(&mut router, build_req("GET", "/api/v1/_ping", None)).await;
        assert_eq!(status, 200, "no token = Admin");

        // ── 9. Unknown token → rejected ──────────────────────────────────────
        let status = exec_status(
            &mut router,
            with_token(build_req("GET", "/api/v1/_ping", None), "tk_nevercreated"),
        )
        .await;
        assert_eq!(status, 401, "unknown token fails closed");

        // ── 10. Admin token → allowed ────────────────────────────────────────
        let admin_tok = sessions.create_session(Role::Admin, None);
        let status = exec_status(
            &mut router,
            with_token(build_req("GET", "/api/v1/_ping", None), &admin_tok),
        )
        .await;
        assert_eq!(status, 200, "Admin allowed");

        // ── 11. AgentRecorder → blocked on /api/v1/_ping ─────────────────────
        let restricted_tok = sessions.create_session(Role::AgentRecorder, None);
        let (status, body) = exec(
            &mut router,
            with_token(build_req("GET", "/api/v1/_ping", None), &restricted_tok),
        )
        .await;
        assert_eq!(status, 403, "AgentRecorder blocked on _ping");
        assert!(
            body["error"].as_str().unwrap().contains("AgentRecorder"),
            "error mentions role"
        );

        // ── 12. BatchRunner → blocked on /api/v1/_ping ───────────────────────
        let batch_tok = sessions.create_session(Role::BatchRunner, None);
        let status = exec_status(
            &mut router,
            with_token(build_req("GET", "/api/v1/_ping", None), &batch_tok),
        )
        .await;
        assert_eq!(status, 403, "BatchRunner blocked on _ping");

        // ── 13. AgentRecorder IS allowed on whitelisted paths ────────────────
        for path in &[
            "/api/v1/locator/confirm",
            "/api/v1/locator/highlight",
            "/api/v1/locator/active-step",
            "/api/v1/steps/statuses",
            "/api/v1/gherkin/render",
            "/api/v1/events",
        ] {
            let status = exec_status(
                &mut router,
                with_token(build_req("GET", path, None), &restricted_tok),
            )
            .await;
            // Routes don't exist in test router → 404, but NOT 403 (auth passes)
            assert_ne!(
                status, 403,
                "AgentRecorder not blocked on whitelisted {path}"
            );
        }

        // ── 14. Session independence ─────────────────────────────────────────
        sessions.remove_session(&restricted_tok);

        // Deleted tokens remain invalid rather than escalating to Admin.
        let status = exec_status(
            &mut router,
            with_token(build_req("GET", "/api/v1/_ping", None), &restricted_tok),
        )
        .await;
        assert_eq!(status, 401, "deleted token fails closed");

        // Admin token still works independently
        let status = exec_status(
            &mut router,
            with_token(build_req("GET", "/api/v1/_ping", None), &admin_tok),
        )
        .await;
        assert_eq!(status, 200, "Admin token unaffected");

        // Remote clients cannot obtain implicit Admin access or mint sessions.
        let status = exec_status(
            &mut router,
            from_remote(build_req("GET", "/api/v1/_ping", None)),
        )
        .await;
        assert_eq!(status, 401, "remote tokenless request is rejected");

        let status = exec_status(
            &mut router,
            from_remote(build_req(
                "POST",
                "/api/v1/sessions",
                Some(r#"{"role":"admin"}"#),
            )),
        )
        .await;
        assert_eq!(status, 403, "remote session bootstrap is rejected");

        let status = exec_status(
            &mut router,
            from_remote(with_token(
                build_req("GET", "/api/v1/_ping", None),
                &admin_tok,
            )),
        )
        .await;
        assert_eq!(status, 200, "valid remote Admin token is accepted");
    }
}
