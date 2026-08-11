//! Axum routes mirroring legacy Tauri invoke commands.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Instant;

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use teshi_core::{BddFeature, BddProject, FeatureRenderPayload, StepIndex};

use crate::session::{Role, SessionStore};
use teshi_engine::{
    check_project_switch_allowed, confirm_locator, delete_profile, get_active_step,
    get_pending_locator, get_profile_public, get_project_root, get_recent_projects,
    highlight_locator, list_dir, list_profiles, load_llm_config_public, load_project_settings,
    open_project, reject_locator, render_feature, resize_terminal, save_profile,
    save_stored_llm_config, set_active_id, spawn_terminal, start_browser_sidecar,
    step_binding_statuses, stop_browser_sidecar, sync_active_step, teardown_runtime, unbind_step,
    write_terminal, ActiveStep, ApiStyle, BrowserError, BrowserMode, BrowserStartResult, DirEntry,
    LlmConfigPublic, LlmConfigWrite, ModelProfile, ModelProfileList, ModelProfilePublic,
    PendingLocator, ProjectSettings, RuntimeEvent, StepBinding, StepBindingStatus, TeshiEngine,
    PROVIDER_OPENAI,
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

/// Binds `addr` and serves the API plus static UI from `dist`.
/// Returns when the server shuts down (via signal, idle timeout, or API call).
pub async fn run_server(
    addr: SocketAddr,
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
    };

    let cors = browser_cors_layer();

    // ── Public routes (no auth required) ──────────────────────────────────────
    let public_routes = Router::new()
        .route("/api/v1/events", get(events_ws))
        .route("/api/v1/sessions", post(api_create_session))
        .route("/api/v1/sessions/{token}", get(api_get_session))
        .route("/api/v1/sessions/{token}", delete(api_delete_session));

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
        .route("/api/v1/projects/open", post(api_open_project))
        .route("/api/v1/projects/teardown", post(api_teardown))
        .route("/api/v1/projects/switch-allowed", get(api_switch_allowed))
        .route("/api/v1/settings/recent", get(api_recent))
        .route("/api/v1/fs/list", get(api_list_dir))
        .route("/api/v1/gherkin/render", post(api_render_feature))
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
        ));

    let app = Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .fallback_service(ServeDir::new(dist).append_index_html_on_directories(true))
        .layer(cors)
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(addr).await?;

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
            if ws_count == 0 && idle > std::time::Duration::from_secs(300) {
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

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown_token.cancelled().await })
        .await?;

    // Clean up on exit
    if let Some(root) = idle_project_root {
        teshi_engine::remove_daemon_manifest(&root);
    }

    Ok(())
}

// ---- WebSocket ----

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

async fn browser_stream_ws(State(state): State<DaemonState>, ws: WebSocketUpgrade) -> Response {
    state.touch();
    let Some(ws_url) = state.rt.sidecar.browser_ws_url() else {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "WinApp sidecar is not running" })),
        )
            .into_response();
    };
    if state.rt.sidecar.browser_mode() != Some(BrowserMode::WinApp) {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "active sidecar is not in WinApp mode" })),
        )
            .into_response();
    }

    let process_name = std::env::var("TESHI_WINAPP_PROCESS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "TargetApp.exe".into());
    let active_ws = state.active_ws.clone();
    ws.on_upgrade(move |socket| {
        handle_browser_stream_socket(ws_url, process_name, active_ws, socket)
    })
}

async fn handle_browser_stream_socket(
    ws_url: String,
    process_name: String,
    active_ws: Arc<AtomicUsize>,
    mut downstream: WebSocket,
) {
    active_ws.fetch_add(1, Ordering::Relaxed);
    let _guard = WsGuard(active_ws);

    let mut upstream = match tokio_tungstenite::connect_async(&ws_url).await {
        Ok((socket, _)) => socket,
        Err(error) => {
            let payload = json!({
                "type": "frame_error",
                "error": format!("connect to WinApp sidecar: {error}"),
            });
            let _ = downstream
                .send(Message::Text(payload.to_string().into()))
                .await;
            return;
        }
    };

    let attach = json!({
        "cmd": "attach_window",
        "request_id": "gpui-preview-attach",
        "process_name": process_name,
    });
    if let Err(error) = upstream
        .send(tokio_tungstenite::tungstenite::Message::Text(
            attach.to_string(),
        ))
        .await
    {
        let payload = json!({
            "type": "frame_error",
            "error": format!("attach to target application: {error}"),
        });
        let _ = downstream
            .send(Message::Text(payload.to_string().into()))
            .await;
        return;
    }

    let (frame_tx, mut frame_rx) = tokio::sync::watch::channel(None::<String>);
    let (control_tx, mut control_rx) = tokio::sync::mpsc::channel::<PreviewRelayMessage>(8);
    let upstream_reader = tokio::spawn(async move {
        while let Some(message) = upstream.next().await {
            match message {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    let text = text.to_string();
                    if is_preview_frame(&text) {
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
                    if control_tx
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
/// Requests without a token default to `Admin` (backward compatible with the
/// web UI which does not yet send tokens).  Unknown tokens also fall back to
/// `Admin` rather than rejecting — only explicit restricted tokens are limited.
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

    // Determine role: explicit session token overrides, otherwise Admin
    let role = if token.is_empty() {
        Role::Admin
    } else {
        state
            .sessions
            .get_session(token)
            .map(|s| s.role)
            .unwrap_or(Role::Admin)
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
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!(
                        "Unknown role '{other}'. Valid roles: admin, agent_recorder, batch_runner"
                    )
                })),
            ));
        }
    };
    let token = state.sessions.create_session(role, body.metadata);
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

async fn api_read_file(Query(q): Query<ListDirQuery>) -> Result<String, (StatusCode, String)> {
    fs::read_to_string(&q.path)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("read {}: {e}", q.path)))
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
    let rt = &state.rt;
    let root = rt
        .project
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no project open".to_string())?;

    // Scan .feature files recursively
    let mut features = Vec::new();
    scan_feature_files(&root, &mut features)?;

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

    Ok(Json(json!({
        "project_root": root.to_string_lossy(),
        "total_raw_steps": index.usages.values().map(|v| v.len()).sum::<usize>(),
        "unique_normalized": index.usages.len(),
        "num_features": project.features.len(),
        "generated_at": chrono::Local::now().to_rfc3339(),
        "entries": entries,
    })))
}

fn scan_feature_files(
    dir: &std::path::Path,
    features: &mut Vec<BddFeature>,
) -> Result<(), ApiError> {
    for entry in fs::read_dir(dir).map_err(|e| ApiError::internal(e.to_string()))? {
        let entry = entry.map_err(|e| ApiError::internal(e.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            scan_feature_files(&path, features)?;
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

#[derive(Deserialize)]
struct RunApiBody {
    feature_path: Option<String>,
    scenario: Option<String>,
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
    use tower::ServiceExt;

    fn test_state() -> DaemonState {
        let rt = teshi_engine::TeshiEngine::new(
            teshi_engine::RuntimeConfig {
                browser_service_script: PathBuf::from(""),
                winapp_service_script: PathBuf::from(""),
                embedded_no_preview_stream: false,
            },
            None,
        );
        DaemonState {
            rt,
            sessions: SessionStore::new(),
            active_ws: Arc::new(AtomicUsize::new(0)),
            last_request: Arc::new(StdMutex::new(Instant::now())),
            shutdown_token: CancellationToken::new(),
        }
    }

    fn build_router(state: DaemonState) -> Router {
        let public = Router::new()
            .route("/api/v1/sessions", post(api_create_session))
            .route("/api/v1/sessions/{token}", get(api_get_session))
            .route("/api/v1/sessions/{token}", delete(api_delete_session));

        let protected = Router::new()
            .route("/api/v1/_ping", get(|| async { "pong" }))
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ));

        Router::new()
            .merge(public)
            .merge(protected)
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
        b.body(Body::from(body.unwrap_or("").to_string())).unwrap()
    }

    fn with_token(req: Request<Body>, token: &str) -> Request<Body> {
        let (mut parts, body) = req.into_parts();
        parts
            .headers
            .insert("x-teshi-token", HeaderValue::from_str(token).unwrap());
        Request::from_parts(parts, body)
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
        assert!(!is_preview_frame(
            r#"{"type":"response","request_id":"gpui-preview-attach"}"#
        ));
        assert!(!is_preview_frame(r#"{"type":"frame_error"}"#));
        assert!(!is_preview_frame("not json"));
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

        // ── 9. Unknown token → falls back to Admin ───────────────────────────
        let status = exec_status(
            &mut router,
            with_token(build_req("GET", "/api/v1/_ping", None), "tk_nevercreated"),
        )
        .await;
        assert_eq!(status, 200, "unknown token = Admin fallback");

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

        // Deleted token falls back to Admin
        let status = exec_status(
            &mut router,
            with_token(build_req("GET", "/api/v1/_ping", None), &restricted_tok),
        )
        .await;
        assert_eq!(status, 200, "deleted token = Admin fallback");

        // Admin token still works independently
        let status = exec_status(
            &mut router,
            with_token(build_req("GET", "/api/v1/_ping", None), &admin_tok),
        )
        .await;
        assert_eq!(status, 200, "Admin token unaffected");
    }
}
