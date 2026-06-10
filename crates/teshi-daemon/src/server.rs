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
use axum::extract::{Query, State};
use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use teshi_gherkin::FeatureRenderPayload;
use teshi_runtime::{
    check_project_switch_allowed, confirm_locator, get_active_step, get_pending_locator,
    get_project_root, get_recent_projects, highlight_locator, list_dir, load_project_settings,
    open_project, reject_locator, render_feature, resize_terminal, spawn_terminal,
    start_browser_sidecar, step_binding_statuses, stop_browser_sidecar, sync_active_step,
    teardown_runtime, unbind_step, write_terminal, ActiveStep, BrowserError, BrowserMode,
    BrowserStartResult, DirEntry, PendingLocator, ProjectSettings, RuntimeEvent, StepBinding,
    StepBindingStatus, TeshiRuntime,
};
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

type SharedRuntime = Arc<TeshiRuntime>;

/// Shared state with idle tracking for the daemon.
#[derive(Clone)]
struct DaemonState {
    rt: SharedRuntime,
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
    let state = DaemonState {
        rt,
        active_ws: Arc::new(AtomicUsize::new(0)),
        last_request: Arc::new(StdMutex::new(Instant::now())),
        shutdown_token: shutdown_token.clone(),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any);

    let app = Router::new()
        .route("/api/v1/events", get(events_ws))
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
                    idle, ws_count
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
        teshi_runtime::remove_daemon_manifest(&root);
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

async fn handle_events_socket(rt: SharedRuntime, active_ws: Arc<AtomicUsize>, mut socket: WebSocket) {
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

async fn api_read_file(
    Query(q): Query<ListDirQuery>,
) -> Result<String, (StatusCode, String)> {
    fs::read_to_string(&q.path)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                format!("read {}: {e}", q.path),
            )
        })
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
    teshi_runtime::stop_terminal(&state.rt)?;
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
        let project = teshi_gherkin::parse_project(&feature_path);
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
        let feature = teshi_gherkin::parse_feature(&content, feature_path.clone());
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
