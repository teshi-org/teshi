//! Axum routes mirroring legacy Tauri invoke commands.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use teshi_gherkin::FeatureRenderPayload;
use teshi_runtime::{
    check_project_switch_allowed, confirm_locator, get_active_step, get_pending_locator,
    get_recent_projects, list_dir, open_project, reject_locator, render_feature, resize_terminal,
    spawn_terminal, start_browser_sidecar, stop_browser_sidecar, sync_active_step,
    teardown_runtime, write_terminal, ActiveStep, BrowserError, BrowserStartResult, DirEntry,
    PendingLocator, RuntimeEvent, TeshiRuntime,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

type SharedRuntime = Arc<TeshiRuntime>;

/// Binds `addr` and serves the API plus static UI from `dist`.
pub async fn run_server(addr: SocketAddr, rt: SharedRuntime, dist: PathBuf) -> Result<()> {
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
        .route("/api/v1/locator/confirm", post(api_confirm_locator))
        .route("/api/v1/locator/reject", post(api_reject_locator))
        .route("/api/v1/browser/start", post(api_browser_start))
        .route("/api/v1/browser/stop", post(api_browser_stop))
        .route("/api/v1/terminal/spawn", post(api_terminal_spawn))
        .route("/api/v1/terminal/stop", post(api_terminal_stop))
        .route("/api/v1/terminal/resize", post(api_terminal_resize))
        .route("/api/v1/terminal/write", post(api_terminal_write))
        .fallback_service(ServeDir::new(dist).append_index_html_on_directories(true))
        .layer(cors)
        .with_state(rt);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn events_ws(State(rt): State<SharedRuntime>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| handle_events_socket(rt, socket))
}

async fn handle_events_socket(rt: SharedRuntime, mut socket: WebSocket) {
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

#[derive(Deserialize)]
struct OpenProjectBody {
    path: String,
}

async fn api_open_project(
    State(rt): State<SharedRuntime>,
    Json(body): Json<OpenProjectBody>,
) -> Result<StatusCode, ApiError> {
    open_project(rt, body.path).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_teardown(State(rt): State<SharedRuntime>) -> Result<StatusCode, ApiError> {
    teardown_runtime(&rt).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_switch_allowed(State(rt): State<SharedRuntime>) -> Json<bool> {
    Json(check_project_switch_allowed(&rt))
}

async fn api_recent() -> Result<Json<Vec<String>>, ApiError> {
    Ok(Json(get_recent_projects()?))
}

#[derive(Deserialize)]
struct ListDirQuery {
    path: String,
}

async fn api_list_dir(
    State(rt): State<SharedRuntime>,
    Query(q): Query<ListDirQuery>,
) -> Result<Json<Vec<DirEntry>>, ApiError> {
    Ok(Json(list_dir(&rt, q.path)?))
}

#[derive(Deserialize)]
struct RenderBody {
    path: String,
}

async fn api_render_feature(
    State(rt): State<SharedRuntime>,
    Json(body): Json<RenderBody>,
) -> Result<Json<FeatureRenderPayload>, ApiError> {
    Ok(Json(render_feature(&rt, body.path)?))
}

#[derive(Deserialize)]
struct SyncStepBody {
    feature_path: String,
    step_line: u32,
}

async fn api_sync_step(
    State(rt): State<SharedRuntime>,
    Json(body): Json<SyncStepBody>,
) -> Result<Json<ActiveStep>, ApiError> {
    Ok(Json(
        sync_active_step(&rt, body.feature_path, body.step_line).await?,
    ))
}

async fn api_active_step(
    State(rt): State<SharedRuntime>,
) -> Result<Json<Option<ActiveStep>>, ApiError> {
    Ok(Json(get_active_step(&rt)?))
}

async fn api_pending_locator(
    State(rt): State<SharedRuntime>,
) -> Result<Json<Option<PendingLocator>>, ApiError> {
    Ok(Json(get_pending_locator(&rt)?))
}

#[derive(Deserialize)]
struct ConfirmBody {
    candidate_rank: u32,
    #[serde(default)]
    edited_value: Option<String>,
}

async fn api_confirm_locator(
    State(rt): State<SharedRuntime>,
    Json(body): Json<ConfirmBody>,
) -> Result<StatusCode, ApiError> {
    confirm_locator(&rt, body.candidate_rank, body.edited_value).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_reject_locator(State(rt): State<SharedRuntime>) -> Result<StatusCode, ApiError> {
    reject_locator(&rt).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_browser_start(
    State(rt): State<SharedRuntime>,
) -> Result<Json<BrowserStartResult>, ApiError> {
    start_browser_sidecar(rt)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn api_browser_stop(State(rt): State<SharedRuntime>) -> Result<StatusCode, ApiError> {
    stop_browser_sidecar(&rt).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SpawnBody {
    cols: u16,
    rows: u16,
}

async fn api_terminal_spawn(
    State(rt): State<SharedRuntime>,
    Json(body): Json<SpawnBody>,
) -> Result<StatusCode, ApiError> {
    spawn_terminal(rt, body.cols, body.rows).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn api_terminal_stop(State(rt): State<SharedRuntime>) -> Result<StatusCode, ApiError> {
    teshi_runtime::stop_terminal(&rt)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ResizeBody {
    cols: u16,
    rows: u16,
}

async fn api_terminal_resize(
    State(rt): State<SharedRuntime>,
    Json(body): Json<ResizeBody>,
) -> Result<StatusCode, ApiError> {
    resize_terminal(&rt, body.cols, body.rows)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct WriteBody {
    data: String,
}

async fn api_terminal_write(
    State(rt): State<SharedRuntime>,
    Json(body): Json<WriteBody>,
) -> Result<StatusCode, ApiError> {
    write_terminal(&rt, body.data)?;
    Ok(StatusCode::NO_CONTENT)
}

/// JSON API error envelope.
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
