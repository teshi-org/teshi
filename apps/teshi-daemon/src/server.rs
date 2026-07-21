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
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use teshi_core::{BddFeature, BddProject, FeatureRenderPayload, StepIndex};

use crate::session::{Role, SessionStore};
use teshi_engine::{
    check_project_switch_allowed, confirm_locator, get_active_step, get_pending_locator,
    get_project_root, get_recent_projects, highlight_locator, list_dir, load_project_settings,
    open_project, reject_locator, render_feature, resize_terminal, spawn_terminal,
    start_browser_sidecar, step_binding_statuses, stop_browser_sidecar, sync_active_step,
    teardown_runtime, unbind_step, write_terminal, ActiveStep, BrowserError, BrowserMode,
    BrowserStartResult, DirEntry, PendingLocator, ProjectSettings, RuntimeEvent, StepBinding,
    StepBindingStatus, TeshiEngine,
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

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers(Any);

    // ── Public routes (no auth required) ──────────────────────────────────────
    let public_routes = Router::new()
        .route("/api/v1/events", get(events_ws))
        .route("/api/v1/sessions", post(api_create_session))
        .route("/api/v1/sessions/{token}", get(api_get_session))
        .route("/api/v1/sessions/{token}", delete(api_delete_session));

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
        .route("/api/v1/locator/confirm", post(api_confirm_locator))
        .route("/api/v1/locator/reject", post(api_reject_locator))
        .route("/api/v1/locator/highlight", post(api_highlight_locator))
        .route("/api/v1/browser/start", post(api_browser_start))
        .route("/api/v1/browser/stop", post(api_browser_stop))
        .route("/api/v1/terminal/spawn", post(api_terminal_spawn))
        .route("/api/v1/terminal/stop", post(api_terminal_stop))
        .route("/api/v1/terminal/resize", post(api_terminal_resize))
        .route("/api/v1/terminal/write", post(api_terminal_write))
        .route("/api/v1/fs/read", get(api_read_file))
        .route("/api/v1/daemon/run", post(api_run))
        .route("/api/v1/daemon/shutdown", post(api_daemon_shutdown))
        .route("/api/v1/steps/catalog", get(api_step_catalog))
        .route(
            "/api/v1/requirements/generate",
            post(api_requirements_generate),
        )
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

// ---- Requirements-to-testpoints types ----

#[derive(Debug, Deserialize)]
struct RequirementsGenerateRequest {
    requirements_text: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Segment {
    id: String,
    text: String,
    pos: [usize; 2],
}

#[derive(Debug, Serialize)]
struct RequirementsGenerateResponse {
    slug: String,
    segments: Vec<Segment>,
    mindmap_xml: String,
    mock_html: String,
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
                                if loc.scenario_idx < feature.scenarios.len() {
                                    feature.scenarios[loc.scenario_idx].name.clone()
                                } else {
                                    format!("<Rule-{}>", loc.scenario_idx)
                                }
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
            for (si, scenario) in feature.scenarios.iter().enumerate() {
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
        for (si, scenario) in feature.scenarios.iter().enumerate() {
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

// ── Requirements-to-testpoints handler ──────────────────────────────────────

async fn api_requirements_generate(
    State(state): State<DaemonState>,
    Json(body): Json<RequirementsGenerateRequest>,
) -> Result<Json<RequirementsGenerateResponse>, ApiError> {
    state.touch();

    if body.requirements_text.trim().is_empty() {
        return Err("Requirements text is empty".to_string().into());
    }

    // Load LLM config
    let (api_key, base_url, model) =
        teshi_engine::llm::llm_config_from_env().map_err(ApiError::internal)?;

    // Build the system prompt
    let system_prompt = build_requirements_system_prompt();

    // Build the tool definition
    let tool_params = serde_json::json!({
        "type": "object",
        "properties": {
            "segments": {
                "type": "array",
                "description": "Word-level segments of the requirements text, covering every character exactly once",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Unique segment ID, e.g. w1, w2" },
                        "text": { "type": "string", "description": "The text of this segment" },
                        "pos": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "minItems": 2,
                            "maxItems": 2,
                            "description": "Character position range [start, end] in the original text"
                        }
                    },
                    "required": ["id", "text", "pos"]
                }
            },
            "mindmap_xml": {
                "type": "string",
                "description": "FreeMind-compatible XML mindmap with test points. Each leaf node must have a LINK attribute with comma-separated segment IDs (e.g., LINK=\"w1,w3\")"
            },
            "mock_html": {
                "type": "string",
                "description": "Complete self-contained high-fidelity HTML page demonstrating the UI described by the requirements"
            }
        },
        "required": ["segments", "mindmap_xml", "mock_html"]
    });

    // Call LLM
    let result = teshi_engine::llm::call_llm_with_tool(
        &api_key,
        &base_url,
        &model,
        &system_prompt,
        &body.requirements_text,
        "generate_testpoints",
        "Generate test points, mindmap, and mock HTML from requirements text",
        tool_params,
    )
    .await
    .map_err(ApiError::internal)?;

    // Parse response
    let segments: Vec<Segment> =
        serde_json::from_value(result.get("segments").cloned().unwrap_or_default())
            .map_err(|e| ApiError::internal(format!("Failed to parse segments: {}", e)))?;

    let mindmap_xml: String = result
        .get("mindmap_xml")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mock_html: String = result
        .get("mock_html")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Validate
    validate_segments(&segments, &body.requirements_text)?;
    validate_mindmap_xml(&mindmap_xml)?;

    // Generate slug and persist
    let slug = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    persist_testpoints(&state.rt, &slug, &mindmap_xml, &mock_html)
        .map_err(|e| ApiError::internal(format!("Failed to save: {}", e)))?;

    Ok(Json(RequirementsGenerateResponse {
        slug,
        segments,
        mindmap_xml,
        mock_html,
    }))
}

fn build_requirements_system_prompt() -> String {
    r#"You are a requirements analysis assistant. Given a free-text requirements document, you must:

1. **Segment the text** into word-level semantic units. Each segment gets a unique ID (w1, w2, ...), the text content, and character position range [start, end]. The `pos` indices MUST use JavaScript UTF-16 code unit positions (matching `String.length`), not Unicode scalar values or byte offsets — this is critical for correct highlighting in the browser. Segments must cover the ENTIRE input text exactly once with no gaps or overlaps.

2. **Generate test points** as a FreeMind XML mindmap. The root node is the system/module name. Intermediate nodes are feature categories. Leaf nodes are individual test points. Each leaf node MUST have a LINK attribute with comma-separated segment IDs that this test point verifies.

3. **Generate mock HTML** - a complete, self-contained HTML document with inline CSS that demonstrates the user interface described by the requirements. Include realistic form elements, buttons, navigation, and content. Make it look like a real application.

IMPORTANT RULES:
- Only generate test points for requirements that are ACTUALLY mentioned in the text. Do not invent test points for unmentioned features.
- Segment the text at word/phrase level, not character-by-character.
- The mindmap XML must be valid FreeMind format (version 1.0.1).
- LINK attributes use comma-separated segment IDs like LINK="w1,w3,w5".
- The mock HTML must be complete with <!DOCTYPE html> and all styles inline.
"#
    .to_string()
}

fn validate_segments(segments: &[Segment], text: &str) -> Result<(), ApiError> {
    // Use UTF-16 code unit length — matches JavaScript String.length that the
    // LLM is instructed to use for pos indices.
    let text_len = text.encode_utf16().count();

    if segments.is_empty() {
        return Err("No segments returned".to_string().into());
    }

    // Validate individual ranges
    for seg in segments {
        if seg.pos[0] >= seg.pos[1] {
            return Err(format!(
                "Segment {} has invalid (empty/reversed) range {:?}",
                seg.id, seg.pos
            )
            .into());
        }
        if seg.pos[1] > text_len {
            return Err(format!(
                "Segment {} range {:?} exceeds text length {}",
                seg.id, seg.pos, text_len
            )
            .into());
        }
    }

    // Sort by start position and check for gaps & overlaps
    let mut sorted: Vec<&Segment> = segments.iter().collect();
    sorted.sort_by_key(|s| s.pos[0]);

    let mut expected = 0usize;
    for seg in &sorted {
        if seg.pos[0] < expected {
            return Err(format!(
                "Segment {} overlaps with previous segment (starts at {} but expected >= {})",
                seg.id, seg.pos[0], expected
            )
            .into());
        }
        if seg.pos[0] > expected {
            // Build a preview of the gap text
            let preview = gap_preview_utf16(text, expected, seg.pos[0]);
            return Err(format!(
                "Gap at UTF-16 positions [{}, {}): {}",
                expected, seg.pos[0], preview
            )
            .into());
        }
        expected = seg.pos[1];
    }

    if expected < text_len {
        let preview = gap_preview_utf16(text, expected, text_len);
        return Err(format!(
            "Trailing gap at UTF-16 positions [{}, {}): {}",
            expected, text_len, preview
        )
        .into());
    }

    Ok(())
}

/// Return a short preview of the text within the given UTF-16 code unit range.
fn gap_preview_utf16(text: &str, start: usize, end: usize) -> String {
    let mut result = String::new();
    let mut pos: usize = 0;
    for ch in text.chars() {
        let ch_units = ch.len_utf16();
        let ch_end = pos + ch_units;
        if ch_end > start && pos < end {
            result.push(ch);
            if result.len() >= 40 {
                result.push('…');
                break;
            }
        }
        pos = ch_end;
        if pos >= end {
            break;
        }
    }
    if result.is_empty() {
        "«end of text»".to_string()
    } else {
        format!("«{}»", result)
    }
}

fn validate_mindmap_xml(xml: &str) -> Result<(), ApiError> {
    if xml.is_empty() {
        return Err("Mindmap XML is empty".to_string().into());
    }
    if !xml.contains("<map") || !xml.contains("</map>") {
        return Err("Mindmap XML must contain a <map> root element"
            .to_string()
            .into());
    }
    Ok(())
}

fn persist_testpoints(
    rt: &SharedRuntime,
    slug: &str,
    mindmap_xml: &str,
    mock_html: &str,
) -> Result<(), String> {
    let project_root = std::path::PathBuf::from(
        get_project_root(rt).ok_or_else(|| "No project is open".to_string())?,
    );

    let dir = project_root.join(".teshi").join("testpoints").join(slug);
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create directory {}: {}", dir.display(), e))?;

    fs::write(dir.join("requirements.mm"), mindmap_xml)
        .map_err(|e| format!("Failed to write requirements.mm: {}", e))?;

    fs::write(dir.join("mock.html"), mock_html)
        .map_err(|e| format!("Failed to write mock.html: {}", e))?;

    Ok(())
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
