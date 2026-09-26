//! Bounded loopback HTTP and authenticated WebSocket transport.

use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, OriginalUri, State};
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    AUTHORIZATION, CONTENT_TYPE, HOST, ORIGIN, VARY,
};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::StreamExt;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio::sync::{RwLock, Semaphore, broadcast, mpsc, oneshot, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{interval, timeout};
use tower::ServiceBuilder;
use tower::limit::ConcurrencyLimitLayer;
use uuid::Uuid;

use crate::credential::{PrivateBrokerCredential, PrivateCredentialStore, create_identity_proof};
use crate::protocol::{
    BROWSER_BROKER_IDENTITY_CHALLENGE_PATH, BROWSER_BROKER_PROTOCOL_VERSION,
    BROWSER_BROKER_SCHEMA_VERSION, BrokerError, BrokerErrorCode, BrokerIdentityChallenge,
    BrowserTarget, DiscoveryResponse, EndpointRecord, ExtensionHeartbeat, ExtensionResponse,
    ExtensionStreamMessage, MAX_CONTROL_MESSAGE_BYTES, MAX_HTTP_BODY_BYTES,
    MAX_WEBSOCKET_CONNECTIONS, MAX_WEBSOCKET_MESSAGE_BYTES, NetworkBatch, OperationRequest,
    PreviewFrameMetadata,
};

const EXTENSION_FRAME_WS_PATH: &str = "/extension/frames";
const TSH1_MAGIC: &[u8; 4] = b"TSH1";
const MAX_PREVIEW_META_BYTES: usize = 65_536;
const MAX_EVENT_RESPONSE_WAIT: Duration = Duration::from_secs(30);
const WS_WRITE_BUFFER_BYTES: usize = 64 * 1024;
const WS_MAX_WRITE_BUFFER_BYTES: usize = MAX_WEBSOCKET_MESSAGE_BYTES;
const HTTP_CONCURRENCY: usize = 64;
const MAX_REQUEST_HEADER_BYTES: usize = 4 * 1024;
const MAX_REQUEST_HEADER_COUNT: usize = 64;
const MAX_EXTENSION_STREAMS: usize = 8;
const MAX_EVENT_QUEUE_CAPACITY: usize = 1024;
const MAX_PUBLICATION_QUEUE_CAPACITY: usize = 64;
const MAX_BROKER_FEATURES: usize = 64;
const EXTENSION_OUTBOUND_QUEUE_CAPACITY: usize = 16;
const MAX_CLIENT_IN_FLIGHT_OPERATIONS: usize = 64;
struct BrokerOutgoingMessage {
    message: Message,
    _budget: tokio::sync::OwnedSemaphorePermit,
}
struct ClientOutgoingMessage {
    message: Message,
    _budget: Option<tokio::sync::OwnedSemaphorePermit>,
}
type ExtensionStreamSender = mpsc::Sender<BrokerOutgoingMessage>;
type StreamRegistry = Arc<RwLock<HashMap<String, (u64, ExtensionStreamSender)>>>;

/// Settings for one in-process broker server. Production callers use loopback and
/// port 17373; tests may bind port zero to avoid touching a user's running broker.
#[derive(Clone)]
pub struct BrokerServerConfig {
    /// Must be a loopback address; non-loopback binds are rejected.
    pub discovery_addr: SocketAddr,
    /// Exact Chrome extension Origins explicitly paired with this OS user.
    pub trusted_extension_origins: Vec<String>,
    /// Cryptographically random per-broker bearer secret.
    pub token: String,
    /// Broker start identity used to reject stale endpoint/process metadata.
    pub broker_start_id: String,
    /// Features served by this broker generation.
    pub broker_features: Vec<String>,
    /// Bound for body parsing, message buffering and per-socket write buffering.
    pub max_websocket_message_bytes: usize,
    /// Upper bound on concurrent upgraded WebSocket connections.
    pub max_websocket_connections: usize,
    /// Upper bound on active Chrome Profile extension streams.
    pub max_extension_streams: usize,
    /// Bounded handoff queue to the state machine.
    pub event_queue_capacity: usize,
    /// Bytes retained by parsed events until the state owner drops them.
    pub queued_event_bytes: usize,
    /// Deadline for state-machine replies and extension acknowledgements.
    pub event_response_timeout: Duration,
}

impl BrokerServerConfig {
    /// Construct the standard per-user loopback listener configuration.
    pub fn new(trusted_extension_origin: impl Into<String>) -> Self {
        Self::with_trusted_extension_origins(vec![trusted_extension_origin.into()])
    }

    /// Construct a configuration for a bounded set of explicitly paired IDs.
    pub fn with_trusted_extension_origins(mut trusted_extension_origins: Vec<String>) -> Self {
        trusted_extension_origins.sort();
        Self {
            discovery_addr: SocketAddr::new(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                crate::protocol::CHROME_DISCOVERY_PORT,
            ),
            trusted_extension_origins,
            token: generate_broker_token(),
            broker_start_id: Uuid::new_v4().simple().to_string(),
            // Features are enabled only by the runtime after their state machine
            // has registered. The transport alone must not promise control.
            broker_features: Vec::new(),
            max_websocket_message_bytes: MAX_WEBSOCKET_MESSAGE_BYTES,
            max_websocket_connections: MAX_WEBSOCKET_CONNECTIONS,
            max_extension_streams: MAX_EXTENSION_STREAMS,
            event_queue_capacity: 256,
            queued_event_bytes: crate::protocol::MAX_QUEUED_EVENT_BYTES,
            event_response_timeout: MAX_EVENT_RESPONSE_WAIT,
        }
    }

    fn validate(&self) -> Result<(), BrokerError> {
        if !self.discovery_addr.ip().is_loopback() {
            return Err(BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "Chrome broker listeners must bind to loopback",
            ));
        }
        if self.trusted_extension_origins.is_empty()
            || self.trusted_extension_origins.len() > crate::protocol::MAX_TRUSTED_EXTENSION_ORIGINS
            || self
                .trusted_extension_origins
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self
                .trusted_extension_origins
                .iter()
                .any(|origin| !valid_extension_origin(origin))
        {
            return Err(BrokerError::new(
                BrokerErrorCode::BrokerOriginDenied,
                "trusted extension origins must be unique exact Chrome extension IDs within the configured limit",
            ));
        }
        if self.token.len() < 32 {
            return Err(BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "broker token is shorter than the minimum credential length",
            ));
        }
        if self.broker_start_id.is_empty()
            || self.broker_start_id.len() > 128
            || self.event_queue_capacity == 0
            || self.event_queue_capacity > MAX_EVENT_QUEUE_CAPACITY
            || self.max_websocket_connections == 0
            || self.max_websocket_connections > MAX_WEBSOCKET_CONNECTIONS
            || self.max_extension_streams == 0
            || self.max_extension_streams > MAX_EXTENSION_STREAMS
            || self.broker_features.len() > MAX_BROKER_FEATURES
            || self
                .broker_features
                .iter()
                .any(|feature| feature.len() > 128)
            || self.token.len() > 512
            || self.max_websocket_message_bytes < 1024
            || self.max_websocket_message_bytes > MAX_WEBSOCKET_MESSAGE_BYTES
            || self.queued_event_bytes < self.max_websocket_message_bytes
            || self.queued_event_bytes > 512 * 1024 * 1024
            || self.event_response_timeout.is_zero()
            || self.event_response_timeout > Duration::from_secs(300)
        {
            return Err(BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "broker limits or start identity are invalid",
            ));
        }
        Ok(())
    }
}

/// Transport events consumed by the broker state machine. Network and filesystem
/// work belongs to the consumer; the transport never holds a registry lock over I/O.
pub enum BrokerEvent {
    Heartbeat {
        payload: ExtensionHeartbeat,
        reply: oneshot::Sender<Value>,
        _budget: tokio::sync::OwnedSemaphorePermit,
    },
    Operation {
        request: OperationRequest,
        reply: oneshot::Sender<Result<Value, BrokerError>>,
        _budget: tokio::sync::OwnedSemaphorePermit,
    },
    ExtensionResponse {
        extension_instance_id: String,
        generation: Option<u64>,
        response: ExtensionResponse,
        reply: Option<oneshot::Sender<Value>>,
        _budget: tokio::sync::OwnedSemaphorePermit,
    },
    ExtensionHttpMessage {
        path: String,
        extension_instance_id: Option<String>,
        payload: Value,
        reply: oneshot::Sender<Value>,
        _budget: tokio::sync::OwnedSemaphorePermit,
    },
    ExtensionConnected {
        hello: ExtensionStreamMessage,
        generation: u64,
        reply: oneshot::Sender<Value>,
        _budget: tokio::sync::OwnedSemaphorePermit,
    },
    ExtensionDisconnected {
        extension_instance_id: String,
        generation: u64,
    },
    NetworkBatch {
        batch: NetworkBatch,
        reply: oneshot::Sender<Value>,
        _budget: tokio::sync::OwnedSemaphorePermit,
    },
    PreviewFrame {
        target: BrowserTarget,
        seq: u64,
        url: String,
        jpeg: Bytes,
        _budget: tokio::sync::OwnedSemaphorePermit,
    },
    FrameError {
        extension_instance_id: String,
        error: String,
    },
    Subscribe {
        extension_instance_id: String,
        request_id: String,
        reply: oneshot::Sender<Result<Value, BrokerError>>,
    },
}

/// A target-scoped update to a CLI/desktop/daemon browser subscriber.
#[derive(Debug, Clone)]
pub struct BrokerPublication {
    pub extension_instance_id: String,
    pub payload: Value,
}

#[derive(Clone)]
struct ServerState {
    trusted_extension_origins: Arc<Vec<Arc<str>>>,
    token: Arc<str>,
    start_id: Arc<str>,
    broker_features: Arc<Vec<String>>,
    discovery_addr: SocketAddr,
    websocket_addr: SocketAddr,
    websocket_base_url: Arc<str>,
    discovery_base_url: Arc<str>,
    events: mpsc::Sender<BrokerEvent>,
    event_byte_budget: Arc<Semaphore>,
    websocket_slots: Arc<Semaphore>,
    extension_stream_slots: Arc<Semaphore>,
    streams: StreamRegistry,
    publications: broadcast::Sender<BrokerPublication>,
    next_stream_generation: Arc<std::sync::atomic::AtomicU64>,
    max_websocket_message_bytes: usize,
    response_timeout: Duration,
}

/// Running loopback broker listeners and the event queue used by the state owner.
pub struct BrokerRuntime {
    state: ServerState,
    events: mpsc::Receiver<BrokerEvent>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl BrokerRuntime {
    /// Bind both loopback listeners, refusing an occupied discovery port without
    /// signalling, killing, or sending any data to the process that owns it.
    pub async fn start(config: BrokerServerConfig) -> Result<Self, BrokerError> {
        config.validate()?;

        // Bind fixed discovery first so an unrelated existing listener is reported
        // before any dynamic socket is allocated.
        let discovery_listener =
            TcpListener::bind(config.discovery_addr)
                .await
                .map_err(|error| {
                    BrokerError::new(
                        BrokerErrorCode::BrowserUnavailable,
                        format!("cannot bind loopback broker discovery listener: {error}"),
                    )
                })?;
        let discovery_addr = discovery_listener.local_addr().map_err(|error| {
            BrokerError::new(
                BrokerErrorCode::BrowserUnavailable,
                format!("cannot inspect discovery listener: {error}"),
            )
        })?;
        let websocket_listener =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
                .await
                .map_err(|error| {
                    BrokerError::new(
                        BrokerErrorCode::BrowserUnavailable,
                        format!("cannot bind dynamic loopback WebSocket listener: {error}"),
                    )
                })?;
        let websocket_addr = websocket_listener.local_addr().map_err(|error| {
            BrokerError::new(
                BrokerErrorCode::BrowserUnavailable,
                format!("cannot inspect WebSocket listener: {error}"),
            )
        })?;

        let (event_tx, event_rx) = mpsc::channel(config.event_queue_capacity);
        let (publication_tx, _) = broadcast::channel(MAX_PUBLICATION_QUEUE_CAPACITY);
        let state = ServerState {
            trusted_extension_origins: Arc::new(
                config
                    .trusted_extension_origins
                    .into_iter()
                    .map(Arc::<str>::from)
                    .collect(),
            ),
            token: Arc::from(config.token),
            start_id: Arc::from(config.broker_start_id),
            broker_features: Arc::new(config.broker_features),
            discovery_addr,
            websocket_addr,
            websocket_base_url: Arc::from(format!("ws://127.0.0.1:{}", websocket_addr.port())),
            discovery_base_url: Arc::from(format!(
                "http://127.0.0.1:{}/v1/bridge",
                discovery_addr.port()
            )),
            events: event_tx,
            event_byte_budget: Arc::new(Semaphore::new(config.queued_event_bytes)),
            websocket_slots: Arc::new(Semaphore::new(config.max_websocket_connections)),
            extension_stream_slots: Arc::new(Semaphore::new(config.max_extension_streams)),
            streams: Arc::new(RwLock::new(HashMap::new())),
            publications: publication_tx,
            next_stream_generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            max_websocket_message_bytes: config.max_websocket_message_bytes,
            response_timeout: config.event_response_timeout,
        };

        let (shutdown, shutdown_rx) = watch::channel(false);
        let discovery_app = discovery_router(state.clone());
        let websocket_app = websocket_router(state.clone());
        let tasks = vec![
            spawn_server(discovery_listener, discovery_app, shutdown_rx.clone()),
            spawn_server(websocket_listener, websocket_app, shutdown_rx),
        ];

        Ok(Self {
            state,
            events: event_rx,
            shutdown,
            tasks,
        })
    }

    /// Receive the next bounded transport event for state-machine processing.
    pub async fn next_event(&mut self) -> Option<BrokerEvent> {
        self.events.recv().await
    }

    /// Subscribe to target-scoped browser updates published by the state owner.
    pub fn subscribe_publications(&self) -> broadcast::Receiver<BrokerPublication> {
        self.state.publications.subscribe()
    }

    /// Publish one already-authorized target event to browser UI clients.
    pub fn publish(&self, publication: BrokerPublication) -> Result<(), BrokerError> {
        json_to_bounded_text(&publication.payload, MAX_CONTROL_MESSAGE_BYTES)?;
        self.state
            .publications
            .send(publication)
            .map(|_| ())
            .map_err(|_| {
                BrokerError::new(
                    BrokerErrorCode::BrowserUnavailable,
                    "no browser UI subscriber is currently connected",
                )
            })
    }

    /// Send a correlated command directly to the currently connected Profile stream.
    pub async fn send_extension_command(
        &self,
        extension_instance_id: &str,
        command: Value,
    ) -> Result<(), BrokerError> {
        let sender = self
            .state
            .streams
            .read()
            .await
            .get(extension_instance_id)
            .map(|(_, sender)| sender.clone())
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::BrowserSessionDisconnected,
                    "extension preview/control stream is disconnected",
                )
            })?;
        let payload = json!({"type":"direct_command", "command":command});
        let text = json_to_bounded_text(&payload, MAX_CONTROL_MESSAGE_BYTES)?;
        let budget = reserve_event_bytes(&self.state, text.len())?;
        match sender.try_send(BrokerOutgoingMessage {
            message: Message::Text(text.into()),
            _budget: budget,
        }) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "extension preview/control stream queue is full; command remains on heartbeat fallback",
            )),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(BrokerError::new(
                BrokerErrorCode::BrowserSessionDisconnected,
                "extension preview/control stream closed before dispatch",
            )),
        }
    }

    /// Endpoint written to the per-project compatibility pointer. It contains no
    /// authentication token and no other project's filesystem path.
    pub fn endpoint_record(&self) -> EndpointRecord {
        let discovery = self.discovery_response(false);
        EndpointRecord {
            schema_version: discovery.schema_version,
            protocol_version: discovery.protocol_version,
            mode: discovery.mode,
            ws_url: discovery.ws_url,
            discovery_url: discovery.discovery_url,
            extension_frame_ws_url: discovery.extension_frame_ws_url,
            broker_pid: discovery.broker_pid,
            broker_start_id: discovery.broker_start_id,
            broker_features: discovery.broker_features,
            bridge: "rust".into(),
        }
    }

    /// Access the bearer secret for private per-user credential persistence.
    /// Callers must never put this value in logs or project files.
    pub fn credential(&self) -> &str {
        &self.state.token
    }

    /// Persist the secret only in the per-user private store, bound to this
    /// listener's public PID/start generation.
    pub fn persist_private_credential(
        &self,
        store: &PrivateCredentialStore,
    ) -> Result<(), BrokerError> {
        let credential = PrivateBrokerCredential::for_endpoint(
            &self.endpoint_record(),
            self.credential(),
            self.state
                .trusted_extension_origins
                .iter()
                .map(|origin| origin.to_string())
                .collect(),
        )?;
        store.write(&credential)
    }

    /// Stop both listeners and wait for their graceful shutdown.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
    }

    /// Run the single broker state owner until Ctrl+C or listener shutdown.
    pub async fn run_state_machine(mut self) {
        let mut state = crate::state::BrokerState::new();
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        let mut expiry_tick = interval(Duration::from_millis(250));
        expiry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = &mut ctrl_c => break,
                _ = expiry_tick.tick() => state.tick(),
                event = self.events.recv() => {
                    let Some(event) = event else { break };
                    state.handle(event, &self).await;
                }
            }
        }
        self.shutdown().await;
    }

    fn discovery_response(&self, include_token: bool) -> DiscoveryResponse {
        build_discovery_response(&self.state, include_token)
    }
}

impl Drop for BrokerRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        for task in &self.tasks {
            task.abort();
        }
    }
}

fn discovery_router(state: ServerState) -> Router {
    Router::new()
        .route("/v1/bridge", get(discovery).options(preflight))
        .route(
            BROWSER_BROKER_IDENTITY_CHALLENGE_PATH,
            post(identity_challenge).options(preflight),
        )
        .route("/v1/bridge/heartbeat", post(heartbeat).options(preflight))
        .route(
            "/v1/bridge/response",
            post(extension_response).options(preflight),
        )
        .route(
            "/v1/bridge/activate_tab",
            post(extension_http_message).options(preflight),
        )
        .route(
            "/v1/bridge/capture_now",
            post(extension_http_message).options(preflight),
        )
        .layer(DefaultBodyLimit::max(MAX_HTTP_BODY_BYTES))
        .layer(ServiceBuilder::new().layer(ConcurrencyLimitLayer::new(HTTP_CONCURRENCY)))
        .layer(middleware::from_fn(enforce_request_header_bounds))
        .with_state(state)
}

fn websocket_router(state: ServerState) -> Router {
    Router::new()
        .route("/", get(client_websocket))
        .route(EXTENSION_FRAME_WS_PATH, get(extension_websocket))
        .layer(middleware::from_fn(enforce_request_header_bounds))
        .with_state(state)
}

async fn enforce_request_header_bounds(request: Request<Body>, next: Next) -> Response {
    let headers = request.headers();
    let bytes = headers.iter().fold(0usize, |total, (name, value)| {
        total
            .saturating_add(name.as_str().len())
            .saturating_add(value.as_bytes().len())
            .saturating_add(4)
    });
    if headers.len() > MAX_REQUEST_HEADER_COUNT || bytes > MAX_REQUEST_HEADER_BYTES {
        return (
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            Json(error_value(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "request headers exceed the broker limits",
            ))),
        )
            .into_response();
    }
    next.run(request).await
}

fn spawn_server(
    listener: TcpListener,
    app: Router,
    mut shutdown_rx: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let graceful = async move {
            if !*shutdown_rx.borrow() {
                let _ = shutdown_rx.changed().await;
            }
        };
        if let Err(error) = axum::serve(listener, app)
            .with_graceful_shutdown(graceful)
            .await
        {
            tracing::error!(%error, "Chrome broker listener exited");
        }
    })
}

async fn discovery(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    if !host_is_loopback(&headers, state.discovery_addr.port()) {
        return error_response(BrokerError::new(
            BrokerErrorCode::BrokerOriginDenied,
            "broker Host must identify its loopback listener",
        ));
    }
    let origin = header_text(&headers, ORIGIN);
    let include_token = match origin.as_deref() {
        None => false,
        Some(value) if is_trusted_extension_origin(&state, value) => true,
        Some(_) => {
            return cors_error(
                &headers,
                &state,
                BrokerError::new(
                    BrokerErrorCode::BrokerOriginDenied,
                    "discovery is unavailable to this browser origin",
                ),
            );
        }
    };
    let payload = build_discovery_response(&state, include_token);
    let mut response = Json(payload).into_response();
    add_cors_if_trusted(response.headers_mut(), &headers, &state);
    response
}

async fn identity_challenge(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(challenge): Json<BrokerIdentityChallenge>,
) -> Response {
    if !host_is_loopback(&headers, state.discovery_addr.port()) {
        return error_response(BrokerError::new(
            BrokerErrorCode::BrokerOriginDenied,
            "identity proof Host must identify its loopback listener",
        ));
    }
    if let Some(origin) = header_text(&headers, ORIGIN)
        && !is_trusted_extension_origin(&state, &origin)
    {
        return cors_error(
            &headers,
            &state,
            BrokerError::new(
                BrokerErrorCode::BrokerOriginDenied,
                "identity proof is unavailable to this browser origin",
            ),
        );
    }
    match create_identity_proof(
        &state.token,
        BROWSER_BROKER_SCHEMA_VERSION,
        BROWSER_BROKER_PROTOCOL_VERSION,
        std::process::id(),
        &state.start_id,
        &challenge.nonce,
    ) {
        Ok(proof) => {
            let mut response = Json(proof).into_response();
            add_cors_if_trusted(response.headers_mut(), &headers, &state);
            response
        }
        Err(error) => error_response(error),
    }
}

async fn preflight(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    if !host_is_loopback(&headers, state.discovery_addr.port()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(origin) = header_text(&headers, ORIGIN) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !is_trusted_extension_origin(&state, &origin) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Ok(origin) = HeaderValue::from_str(&origin) {
        response
            .headers_mut()
            .insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    response.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, X-Teshi-Broker-Token"),
    );
    response
        .headers_mut()
        .insert(VARY, HeaderValue::from_static("Origin"));
    response
}

async fn heartbeat(
    State(state): State<ServerState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize_http(&state, &headers, &uri) {
        return cors_error(&headers, &state, error);
    }
    if !content_type_is_json(&headers) {
        return cors_error(
            &headers,
            &state,
            BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "heartbeat requires application/json",
            ),
        );
    }
    let _budget = match reserve_event_bytes(&state, body.len()) {
        Ok(budget) => budget,
        Err(error) => return cors_error(&headers, &state, error),
    };
    let payload = match serde_json::from_slice::<ExtensionHeartbeat>(&body) {
        Ok(payload) => payload,
        Err(_) => {
            return cors_error(
                &headers,
                &state,
                BrokerError::new(
                    BrokerErrorCode::BrokerProtocolError,
                    "heartbeat body is not a valid protocol message",
                ),
            );
        }
    };
    if payload
        .schema_version
        .is_some_and(|version| version != BROWSER_BROKER_SCHEMA_VERSION)
        || payload
            .protocol_version
            .is_some_and(|version| version > BROWSER_BROKER_PROTOCOL_VERSION)
    {
        return cors_error(
            &headers,
            &state,
            BrokerError::new(
                BrokerErrorCode::IncompatibleBrowserSession,
                "extension protocol version is newer than the broker",
            ),
        );
    }
    let (reply, receiver) = oneshot::channel();
    if state
        .events
        .try_send(BrokerEvent::Heartbeat {
            payload,
            reply,
            _budget,
        })
        .is_err()
    {
        return cors_error(
            &headers,
            &state,
            BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "broker event queue is full; heartbeat was not accepted",
            ),
        );
    }
    let body = await_value(receiver, state.response_timeout).await;
    add_cors_if_trusted_to_value(body, &headers, &state)
}

async fn extension_response(
    State(state): State<ServerState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize_http(&state, &headers, &uri) {
        return cors_error(&headers, &state, error);
    }
    if !content_type_is_json(&headers) {
        return cors_error(
            &headers,
            &state,
            BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "extension response requires application/json",
            ),
        );
    }
    let _budget = match reserve_event_bytes(&state, body.len()) {
        Ok(budget) => budget,
        Err(error) => return cors_error(&headers, &state, error),
    };
    let value: Value = match serde_json::from_slice::<Value>(&body) {
        Ok(value) if value.is_object() => value,
        _ => {
            return cors_error(
                &headers,
                &state,
                BrokerError::new(
                    BrokerErrorCode::BrokerProtocolError,
                    "extension response body must be a JSON object",
                ),
            );
        }
    };
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind == "response" {
        let response = match serde_json::from_value::<ExtensionResponse>(value) {
            Ok(response) if response.validate().is_ok() => response,
            _ => {
                return cors_error(
                    &headers,
                    &state,
                    BrokerError::new(
                        BrokerErrorCode::BrokerProtocolError,
                        "extension response is missing required correlation fields",
                    ),
                );
            }
        };
        let instance_id = response
            .extension_instance_id
            .clone()
            .or_else(|| {
                response
                    .target
                    .as_ref()
                    .map(|target| target.extension_instance_id.clone())
            })
            .unwrap_or_default();
        if instance_id.is_empty() {
            return cors_error(
                &headers,
                &state,
                BrokerError::new(
                    BrokerErrorCode::BrokerProtocolError,
                    "extension response is missing its Profile identity",
                ),
            );
        }
        let (reply, receiver) = oneshot::channel();
        return queue_response_reply(
            &state,
            &headers,
            BrokerEvent::ExtensionResponse {
                extension_instance_id: instance_id,
                generation: None,
                response,
                reply: Some(reply),
                _budget,
            },
            receiver,
        )
        .await;
    }
    if matches!(kind, "frame_error" | "console_event") {
        let extension_instance_id = value
            .get("extension_instance_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (reply, receiver) = oneshot::channel();
        let event = BrokerEvent::ExtensionHttpMessage {
            path: uri.path().to_owned(),
            extension_instance_id,
            payload: value,
            reply,
            _budget,
        };
        return queue_http_reply(&state, &headers, event, receiver).await;
    }
    cors_error(
        &headers,
        &state,
        BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "unsupported extension HTTP message type",
        ),
    )
}

async fn extension_http_message(
    State(state): State<ServerState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize_http(&state, &headers, &uri) {
        return cors_error(&headers, &state, error);
    }
    if !content_type_is_json(&headers) {
        return cors_error(
            &headers,
            &state,
            BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "extension message requires application/json",
            ),
        );
    }
    let _budget = match reserve_event_bytes(&state, body.len()) {
        Ok(budget) => budget,
        Err(error) => return cors_error(&headers, &state, error),
    };
    let value: Value = match serde_json::from_slice::<Value>(&body) {
        Ok(value) if value.is_object() => value,
        _ => {
            return cors_error(
                &headers,
                &state,
                BrokerError::new(
                    BrokerErrorCode::BrokerProtocolError,
                    "extension HTTP message must be a JSON object",
                ),
            );
        }
    };
    let (reply, receiver) = oneshot::channel();
    let event = BrokerEvent::ExtensionHttpMessage {
        path: uri.path().to_owned(),
        extension_instance_id: value
            .get("extension_instance_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        payload: value,
        reply,
        _budget,
    };
    queue_http_reply(&state, &headers, event, receiver).await
}

async fn queue_response_reply(
    state: &ServerState,
    headers: &HeaderMap,
    event: BrokerEvent,
    receiver: oneshot::Receiver<Value>,
) -> Response {
    if state.events.try_send(event).is_err() {
        return cors_error(
            headers,
            state,
            BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "broker event queue is full; extension response was not accepted",
            ),
        );
    }
    let body = await_value(receiver, state.response_timeout).await;
    add_cors_if_trusted_to_value(body, headers, state)
}

async fn queue_http_reply(
    state: &ServerState,
    headers: &HeaderMap,
    event: BrokerEvent,
    receiver: oneshot::Receiver<Value>,
) -> Response {
    if state.events.try_send(event).is_err() {
        return cors_error(
            headers,
            state,
            BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "broker event queue is full; extension message was not accepted",
            ),
        );
    }
    let body = await_value(receiver, state.response_timeout).await;
    add_cors_if_trusted_to_value(body, headers, state)
}

async fn await_value(receiver: oneshot::Receiver<Value>, wait: Duration) -> Value {
    match timeout(wait, receiver).await {
        Ok(Ok(value)) => value,
        _ => error_value(BrokerError::new(
            BrokerErrorCode::BrowserOperationTimeout,
            "broker state handler did not respond before the transport deadline",
        )),
    }
}

fn reserve_event_bytes(
    state: &ServerState,
    bytes: usize,
) -> Result<tokio::sync::OwnedSemaphorePermit, BrokerError> {
    let permits = u32::try_from(bytes.max(1)).map_err(|_| {
        BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "broker event exceeds the byte-budget counter",
        )
    })?;
    Arc::clone(&state.event_byte_budget)
        .try_acquire_many_owned(permits)
        .map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "broker retained-message byte budget is full",
            )
        })
}

struct BoundedJsonWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedJsonWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "serialized JSON exceeded the transport limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json_to_bounded_text(value: &Value, limit: usize) -> Result<String, BrokerError> {
    let mut writer = BoundedJsonWriter {
        bytes: Vec::with_capacity(limit.min(64 * 1024)),
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|error| {
        if error.io_error_kind() == Some(io::ErrorKind::FileTooLarge) {
            BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "serialized JSON exceeds the transport message limit",
            )
        } else {
            BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "broker could not serialize its JSON response",
            )
        }
    })?;
    String::from_utf8(writer.bytes).map_err(|_| {
        BrokerError::new(
            BrokerErrorCode::BrokerProtocolError,
            "serialized broker response was not UTF-8",
        )
    })
}

async fn client_websocket(
    State(state): State<ServerState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    websocket: WebSocketUpgrade,
) -> Response {
    if !host_is_loopback(&headers, state.websocket_addr.port()) {
        return error_response(BrokerError::new(
            BrokerErrorCode::BrokerOriginDenied,
            "WebSocket Host must identify its loopback listener",
        ));
    }
    if let Err(error) = authorize_websocket(&state, &headers, &uri, false) {
        return error_response(error);
    }
    let permit = match Arc::clone(&state.websocket_slots).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return error_response(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "broker WebSocket connection limit reached",
            ));
        }
    };
    websocket
        .max_message_size(crate::protocol::MAX_CONTROL_MESSAGE_BYTES)
        .max_frame_size(crate::protocol::MAX_CONTROL_MESSAGE_BYTES)
        .write_buffer_size(WS_WRITE_BUFFER_BYTES)
        .max_write_buffer_size(WS_MAX_WRITE_BUFFER_BYTES)
        .on_upgrade(move |socket| client_socket(socket, state, permit))
}

async fn extension_websocket(
    State(state): State<ServerState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    websocket: WebSocketUpgrade,
) -> Response {
    if !host_is_loopback(&headers, state.websocket_addr.port()) {
        return error_response(BrokerError::new(
            BrokerErrorCode::BrokerOriginDenied,
            "extension WebSocket Host must identify its loopback listener",
        ));
    }
    if let Err(error) = authorize_websocket(&state, &headers, &uri, true) {
        return error_response(error);
    }
    let permit = match Arc::clone(&state.websocket_slots).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return error_response(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "broker WebSocket connection limit reached",
            ));
        }
    };
    let extension_stream_permit =
        match Arc::clone(&state.extension_stream_slots).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                return error_response(BrokerError::new(
                    BrokerErrorCode::BrowserResourceLimit,
                    "broker extension stream limit reached",
                ));
            }
        };
    let max_size = state.max_websocket_message_bytes;
    websocket
        .max_message_size(max_size)
        .max_frame_size(max_size)
        .write_buffer_size(WS_WRITE_BUFFER_BYTES)
        .max_write_buffer_size(WS_MAX_WRITE_BUFFER_BYTES)
        .on_upgrade(move |socket| extension_socket(socket, state, permit, extension_stream_permit))
}

async fn client_socket(
    mut socket: WebSocket,
    state: ServerState,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let mut subscription: Option<String> = None;
    let mut publications = state.publications.subscribe();
    let (outgoing_tx, mut outgoing_rx) =
        mpsc::channel::<ClientOutgoingMessage>(MAX_CLIENT_IN_FLIGHT_OPERATIONS);
    let in_flight = Arc::new(Semaphore::new(MAX_CLIENT_IN_FLIGHT_OPERATIONS));
    let mut operation_tasks = JoinSet::new();
    loop {
        tokio::select! {
            incoming = socket.next() => {
                let Some(Ok(message)) = incoming else { break };
                let Message::Text(text) = message else {
                    if matches!(message, Message::Close(_)) { break; }
                    if socket.send(Message::Close(None)).await.is_err() { break; }
                    break;
                };
                let value: Value = match serde_json::from_str::<Value>(text.as_str()) {
                    Ok(value) if value.is_object() => value,
                    _ => {
                        let _ = socket.send(Message::Text(error_value(BrokerError::new(
                            BrokerErrorCode::BrokerProtocolError,
                            "client WebSocket message must be a JSON object",
                        )).to_string().into())).await;
                        continue;
                    }
                };
                if value.get("cmd").and_then(Value::as_str) == Some("subscribe_browser_session") {
                    let extension_instance_id = value.get("extension_instance_id").and_then(Value::as_str).unwrap_or_default().to_owned();
                    let request_id = value.get("request_id").and_then(Value::as_str).unwrap_or_default().to_owned();
                    if extension_instance_id.is_empty() || request_id.is_empty() {
                        let _ = socket.send(Message::Text(error_value(BrokerError::new(
                            BrokerErrorCode::InvalidBrowserOperation,
                            "subscription requires extension_instance_id and request_id",
                        )).to_string().into())).await;
                        continue;
                    }
                    let (reply, receiver) = oneshot::channel();
                    if state.events.try_send(BrokerEvent::Subscribe { extension_instance_id: extension_instance_id.clone(), request_id, reply }).is_err() {
                        let _ = socket.send(Message::Text(error_value(BrokerError::new(
                            BrokerErrorCode::BrowserResourceLimit,
                            "broker event queue is full; subscription was not accepted",
                        )).to_string().into())).await;
                        continue;
                    }
                    match timeout(state.response_timeout, receiver).await {
                        Ok(Ok(Ok(_))) => subscription = Some(extension_instance_id),
                        Ok(Ok(Err(error))) => {
                            let _ = socket.send(Message::Text(error_value(error).to_string().into())).await;
                            continue;
                        }
                        _ => {
                            let _ = socket.send(Message::Text(error_value(BrokerError::new(
                                BrokerErrorCode::BrowserOperationTimeout,
                                "browser subscription did not complete before its deadline",
                            )).to_string().into())).await;
                            continue;
                        }
                    }
                } else {
                    let request: OperationRequest = match serde_json::from_value(value) {
                        Ok(request) => request,
                        Err(_) => {
                            let _ = socket.send(Message::Text(error_value(BrokerError::new(
                                BrokerErrorCode::BrokerProtocolError,
                                "client operation does not match the protocol envelope",
                            )).to_string().into())).await;
                            continue;
                        }
                    };
                    if let Err(error) = request.validate() {
                        let _ = socket.send(Message::Text(operation_error_value(&request, error).to_string().into())).await;
                        continue;
                    }
                    let operation_permit = match Arc::clone(&in_flight).try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            let _ = socket.send(Message::Text(operation_error_value(&request, BrokerError::new(
                                BrokerErrorCode::BrowserResourceLimit,
                                "client WebSocket has too many in-flight operations",
                            )).to_string().into())).await;
                            continue;
                        }
                    };
                    let _budget = match reserve_event_bytes(&state, text.len()) {
                        Ok(budget) => budget,
                        Err(error) => {
                            drop(operation_permit);
                            let _ = socket.send(Message::Text(operation_error_value(&request, error).to_string().into())).await;
                            continue;
                        }
                    };
                    let request_timeout = request.timeout_ms.map(Duration::from_millis).unwrap_or(state.response_timeout).min(state.response_timeout);
                    let (reply, receiver) = oneshot::channel();
                    if state.events.try_send(BrokerEvent::Operation { request: request.clone(), reply, _budget }).is_err() {
                        drop(operation_permit);
                        let _ = socket.send(Message::Text(operation_error_value(&request, BrokerError::new(
                            BrokerErrorCode::BrowserResourceLimit,
                            "broker event queue is full; operation was not dispatched",
                        )).to_string().into())).await;
                        continue;
                    }
                    let operation_state = state.clone();
                    let operation_outgoing = outgoing_tx.clone();
                    operation_tasks.spawn(async move {
                        let _in_flight = operation_permit;
                        let result = match timeout(request_timeout, receiver).await {
                            Ok(Ok(Ok(value))) => value,
                            Ok(Ok(Err(error))) => operation_error_value(&request, error),
                            _ => operation_error_value(&request, BrokerError::new(
                                BrokerErrorCode::BrowserOperationTimeout,
                                "broker operation did not complete before its deadline",
                            )),
                        };
                        let (text, response_budget) =
                            operation_response_text(&operation_state, &request, &result);
                        let _ = operation_outgoing
                            .send(ClientOutgoingMessage {
                                message: Message::Text(text.into()),
                                _budget: response_budget,
                            })
                            .await;
                    });
                }
            }
            outgoing = outgoing_rx.recv() => {
                let Some(outgoing) = outgoing else { break };
                if socket.send(outgoing.message).await.is_err() { break; }
            }
            completed = operation_tasks.join_next(), if !operation_tasks.is_empty() => {
                if completed.is_none() {
                    break;
                }
            }
            publication = publications.recv(), if subscription.is_some() => {
                match publication {
                    Ok(publication) if Some(publication.extension_instance_id.as_str()) == subscription.as_deref() => {
                        if socket.send(Message::Text(publication.payload.to_string().into())).await.is_err() { break; }
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    operation_tasks.abort_all();
    while operation_tasks.join_next().await.is_some() {}
}

async fn extension_socket(
    mut socket: WebSocket,
    state: ServerState,
    _permit: tokio::sync::OwnedSemaphorePermit,
    _extension_stream_permit: tokio::sync::OwnedSemaphorePermit,
) {
    let Some(Ok(Message::Text(first))) = socket.next().await else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };
    if first.len() > 64 * 1024 {
        let _ = socket
            .send(Message::Text(
                error_value(BrokerError::new(
                    BrokerErrorCode::BrowserResourceLimit,
                    "extension stream hello exceeds the metadata limit",
                ))
                .to_string()
                .into(),
            ))
            .await;
        let _ = socket.send(Message::Close(None)).await;
        return;
    }
    let _hello_budget = match reserve_event_bytes(&state, first.len()) {
        Ok(budget) => budget,
        Err(error) => {
            let _ = socket
                .send(Message::Text(error_value(error).to_string().into()))
                .await;
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    let hello = match serde_json::from_str::<ExtensionStreamMessage>(first.as_str()) {
        Ok(
            hello @ ExtensionStreamMessage::StreamHello {
                protocol_version, ..
            },
        ) if protocol_version == BROWSER_BROKER_PROTOCOL_VERSION => hello,
        _ => {
            let _ = socket
                .send(Message::Text(
                    error_value(BrokerError::new(
                        BrokerErrorCode::IncompatibleBrowserSession,
                        "extension stream must begin with a supported stream_hello",
                    ))
                    .to_string()
                    .into(),
                ))
                .await;
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    let extension_instance_id = match &hello {
        ExtensionStreamMessage::StreamHello {
            extension_instance_id,
            ..
        } => extension_instance_id.clone(),
    };
    if extension_instance_id.is_empty() || extension_instance_id.len() > 128 {
        let _ = socket.send(Message::Close(None)).await;
        return;
    }

    let generation = state
        .next_stream_generation
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (outgoing, mut outgoing_rx) =
        mpsc::channel::<BrokerOutgoingMessage>(EXTENSION_OUTBOUND_QUEUE_CAPACITY);
    let event_permit = match timeout(state.response_timeout, state.events.reserve()).await {
        Ok(Ok(permit)) => permit,
        _ => {
            let _ = socket
                .send(Message::Text(
                    error_value(BrokerError::new(
                        BrokerErrorCode::BrowserResourceLimit,
                        "broker event queue stayed full while registering the extension stream",
                    ))
                    .to_string()
                    .into(),
                ))
                .await;
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    let previous = state.streams.write().await.insert(
        extension_instance_id.clone(),
        (generation, outgoing.clone()),
    );
    if let Some((_, old_sender)) = previous {
        close_replaced_stream(&state, old_sender).await;
    }
    let (reply, reply_rx) = oneshot::channel();
    event_permit.send(BrokerEvent::ExtensionConnected {
        hello,
        generation,
        reply,
        _budget: _hello_budget,
    });
    let ack = match timeout(state.response_timeout, reply_rx).await {
        Ok(Ok(value)) => value,
        _ => {
            let _ = socket.send(Message::Close(None)).await;
            unregister_stream(&state, &extension_instance_id, generation, true).await;
            return;
        }
    };
    let connected = ack.get("ok").and_then(Value::as_bool) == Some(true);
    if !connected {
        let _ = socket.send(Message::Text(ack.to_string().into())).await;
        let _ = socket.send(Message::Close(None)).await;
        unregister_stream(&state, &extension_instance_id, generation, true).await;
        return;
    }
    if socket
        .send(Message::Text(ack.to_string().into()))
        .await
        .is_err()
    {
        unregister_stream(&state, &extension_instance_id, generation, true).await;
        return;
    }

    loop {
        tokio::select! {
            incoming = socket.next() => {
                let Some(Ok(message)) = incoming else { break };
                if !stream_generation_is_current(&state, &extension_instance_id, generation).await {
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
                match message {
                    Message::Text(text) => {
                        if !handle_extension_text(
                            &state,
                            &extension_instance_id,
                            generation,
                            text.as_str(),
                        )
                        .await
                        {
                            let _ = socket.send(Message::Close(None)).await;
                            break;
                        }
                    }
                    Message::Binary(packet) => {
                        let frame_budget = match reserve_event_bytes(&state, packet.len()) {
                            Ok(budget) => budget,
                            Err(_) => {
                                let _ = socket.send(Message::Close(None)).await;
                                break;
                            }
                        };
                        match parse_tsh1_frame(packet) {
                            Ok((metadata, jpeg)) => {
                                if metadata.extension_instance_id != extension_instance_id {
                                    let _ = socket.send(Message::Close(None)).await;
                                    break;
                                }
                                let target = BrowserTarget {
                                    extension_instance_id: metadata.extension_instance_id,
                                    window_id: metadata.window_id,
                                    tab_id: metadata.tab_id,
                                };
                                let event = BrokerEvent::PreviewFrame { target, seq: metadata.seq, url: metadata.url, jpeg, _budget: frame_budget };
                                if state.events.try_send(event).is_err() {
                                    let _ = socket.send(Message::Close(None)).await;
                                    break;
                                }
                            }
                            Err(_) => {
                                let _ = socket.send(Message::Close(None)).await;
                                break;
                            }
                        }
                    }
                    Message::Ping(payload) => {
                        if socket.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    Message::Pong(_) => {}
                    Message::Close(_) => break,
                }
            }
            outgoing = outgoing_rx.recv() => {
                let Some(outgoing) = outgoing else { break };
                if !stream_generation_is_current(&state, &extension_instance_id, generation).await {
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
                if socket.send(outgoing.message).await.is_err() { break; }
            }
        }
    }

    unregister_stream(&state, &extension_instance_id, generation, true).await;
}

async fn close_replaced_stream(state: &ServerState, sender: ExtensionStreamSender) {
    let Ok(budget) = reserve_event_bytes(state, 1) else {
        return;
    };
    let close = sender.send(BrokerOutgoingMessage {
        message: Message::Close(None),
        _budget: budget,
    });
    let _ = timeout(Duration::from_secs(1), close).await;
}

async fn unregister_stream(
    state: &ServerState,
    extension_instance_id: &str,
    generation: u64,
    notify: bool,
) {
    let removed = {
        let mut streams = state.streams.write().await;
        if streams
            .get(extension_instance_id)
            .is_some_and(|(current, _)| *current == generation)
        {
            streams.remove(extension_instance_id);
            true
        } else {
            false
        }
    };
    if removed && notify {
        let event = BrokerEvent::ExtensionDisconnected {
            extension_instance_id: extension_instance_id.to_owned(),
            generation,
        };
        let _ = timeout(state.response_timeout, state.events.send(event)).await;
    }
}

async fn stream_generation_is_current(
    state: &ServerState,
    instance_id: &str,
    generation: u64,
) -> bool {
    state
        .streams
        .read()
        .await
        .get(instance_id)
        .is_some_and(|(current, _)| *current == generation)
}

async fn handle_extension_text(
    state: &ServerState,
    instance_id: &str,
    generation: u64,
    text: &str,
) -> bool {
    let _budget = match reserve_event_bytes(state, text.len()) {
        Ok(budget) => budget,
        Err(_) => return false,
    };
    let value: Value = match serde_json::from_str::<Value>(text) {
        Ok(value) if value.is_object() => value,
        _ => return false,
    };
    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "response" => {
            let mut response: ExtensionResponse = match serde_json::from_value(value) {
                Ok(response) => response,
                Err(_) => return false,
            };
            if response.validate().is_err() {
                return false;
            }
            if response
                .extension_instance_id
                .as_deref()
                .is_some_and(|id| id != instance_id)
                || response
                    .target
                    .as_ref()
                    .is_some_and(|target| target.extension_instance_id != instance_id)
            {
                return false;
            }
            response.extension_instance_id = Some(instance_id.to_owned());
            state
                .events
                .try_send(BrokerEvent::ExtensionResponse {
                    extension_instance_id: instance_id.to_owned(),
                    generation: Some(generation),
                    response,
                    reply: None,
                    _budget,
                })
                .is_ok()
        }
        "network_batch" => {
            let batch: NetworkBatch = match serde_json::from_value(value) {
                Ok(batch) => batch,
                Err(_) => return false,
            };
            if batch.validate().is_err() || batch.extension_instance_id != instance_id {
                return false;
            }
            let (reply, receiver) = oneshot::channel();
            if state
                .events
                .try_send(BrokerEvent::NetworkBatch {
                    batch,
                    reply,
                    _budget,
                })
                .is_err()
            {
                return false;
            }
            let ack = match timeout(state.response_timeout, receiver).await {
                Ok(Ok(value)) => value,
                _ => return false,
            };
            let sender = {
                state.streams.read().await.get(instance_id).and_then(
                    |(current_generation, sender)| {
                        (*current_generation == generation).then(|| sender.clone())
                    },
                )
            };
            if let Some(sender) = sender {
                match json_to_bounded_text(&ack, 64 * 1024) {
                    Ok(text) => match reserve_event_bytes(state, text.len()) {
                        Ok(budget) => sender
                            .send(BrokerOutgoingMessage {
                                message: Message::Text(text.into()),
                                _budget: budget,
                            })
                            .await
                            .is_ok(),
                        Err(_) => false,
                    },
                    Err(_) => false,
                }
            } else {
                false
            }
        }
        "frame_error" | "console_event" => {
            let error = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("extension diagnostic event")
                .chars()
                .take(2048)
                .collect::<String>();
            let event = if value.get("type").and_then(Value::as_str) == Some("frame_error") {
                BrokerEvent::FrameError {
                    extension_instance_id: instance_id.to_owned(),
                    error,
                }
            } else {
                let (reply, receiver) = oneshot::channel();
                if state
                    .events
                    .try_send(BrokerEvent::ExtensionHttpMessage {
                        path: "/v1/bridge/response".into(),
                        extension_instance_id: Some(instance_id.to_owned()),
                        payload: value,
                        reply,
                        _budget,
                    })
                    .is_err()
                {
                    return false;
                }
                let _ = timeout(state.response_timeout, receiver).await;
                return true;
            };
            state.events.try_send(event).is_ok()
        }
        _ => false,
    }
}

fn build_discovery_response(state: &ServerState, include_token: bool) -> DiscoveryResponse {
    let token = if include_token {
        format!("?token={}", state.token)
    } else {
        String::new()
    };
    let ws_base = state.websocket_base_url.as_ref();
    DiscoveryResponse {
        schema_version: BROWSER_BROKER_SCHEMA_VERSION,
        protocol_version: BROWSER_BROKER_PROTOCOL_VERSION,
        mode: "chrome".into(),
        transport: "http-heartbeat+ws-screencast".into(),
        command_transport: "direct-ws+heartbeat-fallback".into(),
        ws_url: format!("{ws_base}/{token}"),
        extension_frame_ws_url: format!("{ws_base}{EXTENSION_FRAME_WS_PATH}{token}"),
        discovery_url: state.discovery_base_url.to_string(),
        broker_pid: std::process::id(),
        broker_scope: "user_session".into(),
        broker_start_id: state.start_id.to_string(),
        broker_features: state.broker_features.as_ref().clone(),
        extension_connected: false,
    }
}

fn authorize_http(
    state: &ServerState,
    headers: &HeaderMap,
    uri: &axum::http::Uri,
) -> Result<(), BrokerError> {
    if !host_is_loopback(headers, state.discovery_addr.port()) {
        return Err(BrokerError::new(
            BrokerErrorCode::BrokerOriginDenied,
            "HTTP Host must identify the loopback discovery listener",
        ));
    }
    if let Some(origin) = header_text(headers, ORIGIN)
        && !is_trusted_extension_origin(state, &origin)
    {
        return Err(BrokerError::new(
            BrokerErrorCode::BrokerOriginDenied,
            "HTTP mutation is not accepted from this browser origin",
        ));
    }
    if !token_matches(state, headers, uri.query()) {
        return Err(BrokerError::new(
            BrokerErrorCode::BrokerAuthenticationFailed,
            "broker authentication failed",
        ));
    }
    Ok(())
}

fn authorize_websocket(
    state: &ServerState,
    headers: &HeaderMap,
    uri: &axum::http::Uri,
    extension_stream: bool,
) -> Result<(), BrokerError> {
    let origin = header_text(headers, ORIGIN);
    if extension_stream
        && !origin
            .as_deref()
            .is_some_and(|value| is_trusted_extension_origin(state, value))
    {
        return Err(BrokerError::new(
            BrokerErrorCode::BrokerOriginDenied,
            "extension stream Origin is not trusted",
        ));
    }
    if let Some(origin) = origin
        && !is_trusted_extension_origin(state, &origin)
    {
        return Err(BrokerError::new(
            BrokerErrorCode::BrokerOriginDenied,
            "WebSocket Origin is not trusted",
        ));
    }
    if !token_matches(state, headers, uri.query()) {
        return Err(BrokerError::new(
            BrokerErrorCode::BrokerAuthenticationFailed,
            "broker authentication failed",
        ));
    }
    Ok(())
}

fn token_matches(state: &ServerState, headers: &HeaderMap, query: Option<&str>) -> bool {
    let header_token = headers
        .get("x-teshi-broker-token")
        .and_then(|value| value.to_str().ok());
    let bearer_token = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let supplied_header = header_token.or(bearer_token);
    let supplied_query = query.and_then(|query| {
        let mut tokens = query
            .split('&')
            .filter_map(|entry| entry.split_once('='))
            .filter(|(key, _)| *key == "token")
            .map(|(_, value)| value);
        let first = tokens.next()?;
        if tokens.next().is_some() {
            return None;
        }
        Some(first)
    });
    match (supplied_header, supplied_query) {
        (Some(header), Some(query)) => {
            constant_time_equal(header, &state.token) && constant_time_equal(query, &state.token)
        }
        (Some(header), None) => constant_time_equal(header, &state.token),
        (None, Some(query)) => constant_time_equal(query, &state.token),
        (None, None) => false,
    }
}

fn constant_time_equal(candidate: &str, expected: &str) -> bool {
    if candidate.len() != expected.len() {
        return false;
    }
    bool::from(candidate.as_bytes().ct_eq(expected.as_bytes()))
}

fn valid_extension_origin(origin: &str) -> bool {
    let Some(id) = origin.strip_prefix("chrome-extension://") else {
        return false;
    };
    id.len() == 32 && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte))
}

fn is_trusted_extension_origin(state: &ServerState, origin: &str) -> bool {
    state
        .trusted_extension_origins
        .iter()
        .any(|trusted| trusted.as_ref() == origin)
}

fn host_is_loopback(headers: &HeaderMap, port: u16) -> bool {
    let Some(host) = headers.get(HOST).and_then(|host| host.to_str().ok()) else {
        return false;
    };
    host == format!("127.0.0.1:{port}")
}

fn content_type_is_json(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

fn header_text(headers: &HeaderMap, key: axum::http::header::HeaderName) -> Option<String> {
    headers
        .get(key)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn add_cors_if_trusted(headers: &mut HeaderMap, request_headers: &HeaderMap, state: &ServerState) {
    let Some(origin) = header_text(request_headers, ORIGIN) else {
        return;
    };
    if !is_trusted_extension_origin(state, &origin) {
        return;
    }
    if let Ok(origin) = HeaderValue::from_str(&origin) {
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(VARY, HeaderValue::from_static("Origin"));
    }
}

fn add_cors_if_trusted_to_value(
    value: Value,
    headers: &HeaderMap,
    state: &ServerState,
) -> Response {
    let mut response = Json(value).into_response();
    add_cors_if_trusted(response.headers_mut(), headers, state);
    response
}

fn cors_error(headers: &HeaderMap, state: &ServerState, error: BrokerError) -> Response {
    let mut response = error_response(error);
    add_cors_if_trusted(response.headers_mut(), headers, state);
    response
}

fn error_status(code: BrokerErrorCode) -> StatusCode {
    match code {
        BrokerErrorCode::BrokerAuthenticationFailed | BrokerErrorCode::BrokerOriginDenied => {
            StatusCode::FORBIDDEN
        }
        BrokerErrorCode::BrowserResourceLimit => StatusCode::SERVICE_UNAVAILABLE,
        BrokerErrorCode::BrowserOperationTimeout => StatusCode::GATEWAY_TIMEOUT,
        BrokerErrorCode::BrowserUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        BrokerErrorCode::IncompatibleBrowserSession => StatusCode::UPGRADE_REQUIRED,
        _ => StatusCode::BAD_REQUEST,
    }
}

fn error_value(error: BrokerError) -> Value {
    json!({
        "ok": false,
        "code": error.code.as_str(),
        "error": error.message,
        "recovery": error.recovery,
    })
}

fn operation_error_value(request: &OperationRequest, error: BrokerError) -> Value {
    json!({
        "ok": false,
        "request_id": request.request_id,
        "operation": request.operation,
        "code": error.code.as_str(),
        "error": error.message,
        "recovery": error.recovery,
    })
}

fn operation_response_text(
    state: &ServerState,
    request: &OperationRequest,
    response: &Value,
) -> (String, Option<tokio::sync::OwnedSemaphorePermit>) {
    let mut text = json_to_bounded_text(response, MAX_WEBSOCKET_MESSAGE_BYTES)
        .unwrap_or_else(|error| operation_error_value(request, error).to_string());
    match reserve_event_bytes(state, text.len()) {
        Ok(budget) => (text, Some(budget)),
        Err(error) => {
            text = operation_error_value(request, error).to_string();
            let budget = reserve_event_bytes(state, text.len()).ok();
            (text, budget)
        }
    }
}

fn error_response(error: BrokerError) -> Response {
    let status = error_status(error.code);
    (status, Json(error_value(error))).into_response()
}

fn parse_tsh1_frame(packet: Bytes) -> Result<(PreviewFrameMetadata, Bytes), BrokerError> {
    if packet.len() < 8 || &packet[..4] != TSH1_MAGIC {
        return Err(BrokerError::new(
            BrokerErrorCode::BrokerProtocolError,
            "preview frame is not a TSH1 packet",
        ));
    }
    let metadata_len = u32::from_le_bytes(packet[4..8].try_into().unwrap()) as usize;
    if metadata_len == 0 || metadata_len > MAX_PREVIEW_META_BYTES {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "preview frame metadata is empty or oversized",
        ));
    }
    let metadata_end = 8usize.checked_add(metadata_len).ok_or_else(|| {
        BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "preview frame metadata length overflowed",
        )
    })?;
    if metadata_end >= packet.len() {
        return Err(BrokerError::new(
            BrokerErrorCode::BrokerProtocolError,
            "preview frame metadata or image payload is truncated",
        ));
    }
    let metadata: PreviewFrameMetadata =
        serde_json::from_slice(&packet[8..metadata_end]).map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "preview frame metadata is malformed",
            )
        })?;
    if metadata.extension_instance_id.trim().is_empty()
        || metadata.extension_instance_id.len() > 128
        || metadata.window_id <= 0
        || metadata.tab_id <= 0
        || metadata.seq == 0
        || metadata.url.len() > 16 * 1024
    {
        return Err(BrokerError::new(
            BrokerErrorCode::BrokerProtocolError,
            "preview frame metadata is outside protocol bounds",
        ));
    }
    let jpeg = packet.slice(metadata_end..);
    if jpeg.len() > 50 * 1024 * 1024
        || jpeg.len() < 4
        || jpeg[..2] != [0xff, 0xd8]
        || jpeg[jpeg.len() - 2..] != [0xff, 0xd9]
    {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "preview JPEG is empty, malformed, or exceeds the frame limit",
        ));
    }
    Ok((metadata, jpeg))
}

/// Generate a URL-safe bearer value with 244 CSPRNG bits without logging or
/// writing it to a project endpoint. The two UUID v4 values are OS-random.
pub fn generate_broker_token() -> String {
    let first = Uuid::new_v4().into_bytes();
    let second = Uuid::new_v4().into_bytes();
    let mut material = [0u8; 32];
    material[..16].copy_from_slice(&first);
    material[16..].copy_from_slice(&second);
    URL_SAFE_NO_PAD.encode(material)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::BrokerIdentityProof;
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue as WsHeaderValue;
    use tokio_tungstenite::tungstenite::http::header::ORIGIN as WS_ORIGIN;
    use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

    const EXTENSION_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn test_config() -> BrokerServerConfig {
        let mut config = BrokerServerConfig::new(format!("chrome-extension://{EXTENSION_ID}"));
        config.discovery_addr = "127.0.0.1:0".parse().unwrap();
        config
    }

    fn request_ws_url(runtime: &BrokerRuntime) -> String {
        format!(
            "{}?token={}",
            runtime.endpoint_record().ws_url,
            runtime.credential()
        )
    }

    fn request_extension_ws_url(runtime: &BrokerRuntime) -> String {
        format!(
            "{}?token={}",
            runtime.endpoint_record().extension_frame_ws_url,
            runtime.credential()
        )
    }

    async fn read_json(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Value {
        let message = socket.next().await.unwrap().unwrap();
        serde_json::from_str(message.into_text().unwrap().as_str()).unwrap()
    }

    #[test]
    fn extension_origin_is_exact_and_uses_chrome_id_alphabet() {
        assert!(valid_extension_origin(&format!(
            "chrome-extension://{}",
            "a".repeat(32)
        )));
        assert!(!valid_extension_origin("https://example.test"));
        assert!(!valid_extension_origin(&format!(
            "chrome-extension://{}",
            "q".repeat(32)
        )));
        assert!(!valid_extension_origin(&format!(
            "chrome-extension://{}",
            "a".repeat(31)
        )));
    }

    #[test]
    fn tsh1_parser_preserves_frame_bytes_without_reencoding() {
        let metadata = serde_json::to_vec(&PreviewFrameMetadata {
            extension_instance_id: "profile-a".into(),
            window_id: 7,
            tab_id: 42,
            url: "https://example.test/".into(),
            seq: 1,
        })
        .unwrap();
        let jpeg = [0xff, 0xd8, 0xff, 0xd9];
        let mut packet = Vec::new();
        packet.extend_from_slice(TSH1_MAGIC);
        packet.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
        packet.extend_from_slice(&metadata);
        packet.extend_from_slice(&jpeg);
        let (parsed, bytes) = parse_tsh1_frame(Bytes::from(packet)).unwrap();
        assert_eq!(parsed.extension_instance_id, "profile-a");
        assert_eq!(parsed.seq, 1);
        assert_eq!(&bytes[..], &jpeg);
    }

    #[test]
    fn broker_rejects_non_loopback_and_unpinned_extension_origin() {
        let mut config = BrokerServerConfig::new(format!("chrome-extension://{}", "a".repeat(32)));
        config.discovery_addr = "0.0.0.0:17373".parse().unwrap();
        assert_eq!(
            config.validate().unwrap_err().code,
            BrokerErrorCode::BrokerProtocolError
        );
        config.discovery_addr = "127.0.0.1:17373".parse().unwrap();
        config.trusted_extension_origins = vec!["chrome-extension://other".into()];
        assert_eq!(
            config.validate().unwrap_err().code,
            BrokerErrorCode::BrokerOriginDenied
        );
    }

    #[tokio::test]
    async fn occupied_discovery_listener_is_never_terminated_or_reused() {
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = occupied.local_addr().unwrap();
        let mut config = test_config();
        config.discovery_addr = address;

        let error = BrokerRuntime::start(config).await.err().unwrap();
        assert_eq!(error.code, BrokerErrorCode::BrowserUnavailable);
        let connection = tokio::net::TcpStream::connect(address).await.unwrap();
        let (accepted, _) = occupied.accept().await.unwrap();
        drop(connection);
        drop(accepted);
    }

    #[tokio::test]
    async fn discovery_never_gives_credentials_or_project_paths_to_local_clients() {
        let runtime = BrokerRuntime::start(test_config()).await.unwrap();
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap();
        let discovery_url = runtime.endpoint_record().discovery_url;

        let local = client.get(&discovery_url).send().await.unwrap();
        assert_eq!(local.status(), StatusCode::OK);
        let local: Value = local.json().await.unwrap();
        assert_eq!(local["mode"], "chrome");
        assert_eq!(local["broker_scope"], "user_session");
        assert!(!local["ws_url"].as_str().unwrap().contains("token="));
        assert!(
            !local["extension_frame_ws_url"]
                .as_str()
                .unwrap()
                .contains("token=")
        );
        assert!(local.get("project_root").is_none());
        assert!(local.get("endpoint").is_none());
        let public_ws_port = local["ws_url"]
            .as_str()
            .unwrap()
            .split(':')
            .nth(2)
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .split('?')
            .next()
            .unwrap()
            .parse::<u16>()
            .unwrap();
        assert_ne!(public_ws_port, crate::protocol::CHROME_DISCOVERY_PORT);

        let mut bad_host = tokio::net::TcpStream::connect(runtime.state.discovery_addr)
            .await
            .unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        bad_host
            .write_all(
                b"GET /v1/bridge HTTP/1.1\r\nHost: attacker.example\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let mut bad_host_response = Vec::new();
        bad_host.read_to_end(&mut bad_host_response).await.unwrap();
        assert!(bad_host_response.starts_with(b"HTTP/1.1 403"));

        let hostile = client
            .get(&discovery_url)
            .header("Origin", "https://attacker.example")
            .send()
            .await
            .unwrap();
        assert_eq!(hostile.status(), StatusCode::FORBIDDEN);
        assert!(
            hostile
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
        let _ = hostile.bytes().await.unwrap();

        let trusted_origin = format!("chrome-extension://{EXTENSION_ID}");
        let extension = client
            .get(&discovery_url)
            .header("Origin", &trusted_origin)
            .send()
            .await
            .unwrap();
        assert_eq!(extension.status(), StatusCode::OK);
        assert_eq!(
            extension
                .headers()
                .get("access-control-allow-origin")
                .unwrap(),
            trusted_origin.as_str()
        );
        let extension: Value = extension.json().await.unwrap();
        assert!(extension["ws_url"].as_str().unwrap().contains("token="));
        assert!(
            extension["extension_frame_ws_url"]
                .as_str()
                .unwrap()
                .contains("token=")
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn identity_challenge_proves_generation_without_sending_token() {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;

        let runtime = BrokerRuntime::start(test_config()).await.unwrap();
        let endpoint = runtime.endpoint_record();
        let token = runtime.credential().to_owned();
        let credential = PrivateBrokerCredential::for_endpoint(
            &endpoint,
            token.clone(),
            vec![format!("chrome-extension://{EXTENSION_ID}")],
        )
        .unwrap();
        let challenge = BrokerIdentityChallenge {
            nonce: URL_SAFE_NO_PAD.encode([7u8; 32]),
        };
        let response = reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}{}",
                runtime.state.discovery_addr.port(),
                BROWSER_BROKER_IDENTITY_CHALLENGE_PATH
            ))
            .json(&challenge)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let proof: BrokerIdentityProof = response.json().await.unwrap();
        credential
            .verify_identity_proof(&challenge, &proof, &endpoint)
            .unwrap();
        assert!(!serde_json::to_string(&proof).unwrap().contains(&token));

        let denied = reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}{}",
                runtime.state.discovery_addr.port(),
                BROWSER_BROKER_IDENTITY_CHALLENGE_PATH
            ))
            .header("Origin", "https://attacker.example")
            .json(&challenge)
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn runtime_persists_a_credential_bound_to_its_process_generation() {
        let runtime = BrokerRuntime::start(test_config()).await.unwrap();
        let temp = tempfile::tempdir().unwrap();
        let store = PrivateCredentialStore::new(temp.path().join("private-state"));
        runtime.persist_private_credential(&store).unwrap();
        let endpoint = runtime.endpoint_record();
        let credential = store.read_for_endpoint(&endpoint).unwrap();
        assert_eq!(credential.token(), runtime.credential());

        let mut stale_endpoint = endpoint;
        stale_endpoint.broker_start_id.push_str("-stale");
        assert_eq!(
            store.read_for_endpoint(&stale_endpoint).unwrap_err().code,
            BrokerErrorCode::IncompatibleBrowserSession
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn http_mutations_require_exact_origin_and_token_before_state_events() {
        let mut runtime = BrokerRuntime::start(test_config()).await.unwrap();
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap();
        let endpoint = runtime.endpoint_record();
        let origin = format!("chrome-extension://{EXTENSION_ID}");
        let heartbeat = serde_json::json!({
            "schema_version": 1,
            "protocol_version": 1,
            "extension_instance_id": "profile-a",
            "project_root": "C:/private/project",
            "tabs": []
        });

        let missing = client
            .post(format!("{}/heartbeat", endpoint.discovery_url))
            .header("Content-Type", "application/json")
            .header("Origin", &origin)
            .json(&heartbeat)
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::FORBIDDEN);
        let _ = missing.bytes().await.unwrap();

        let hostile = client
            .post(format!(
                "{}/heartbeat?token={}",
                endpoint.discovery_url,
                runtime.credential()
            ))
            .header("Content-Type", "application/json")
            .header("Origin", "https://attacker.example")
            .json(&heartbeat)
            .send()
            .await
            .unwrap();
        assert_eq!(hostile.status(), StatusCode::FORBIDDEN);
        let _ = hostile.bytes().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), runtime.events.recv())
                .await
                .is_err()
        );

        let credential = runtime.credential().to_owned();
        let request = tokio::spawn(async move {
            client
                .post(format!(
                    "{}/heartbeat?token={credential}",
                    endpoint.discovery_url
                ))
                .header("Content-Type", "application/json")
                .header("X-Teshi-Broker-Token", &credential)
                .header("Origin", origin)
                .json(&heartbeat)
                .send()
                .await
                .unwrap()
        });
        let BrokerEvent::Heartbeat { payload, reply, .. } = runtime.next_event().await.unwrap()
        else {
            panic!("expected authenticated heartbeat event")
        };
        assert_eq!(payload.extension_instance_id.as_deref(), Some("profile-a"));
        assert_eq!(payload.project_root.as_deref(), Some("C:/private/project"));
        reply
            .send(serde_json::json!({"ok": true, "compatible": true}))
            .unwrap();
        let response = request.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.json::<Value>().await.unwrap()["ok"], true);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn extension_stream_requires_pinned_origin_and_acknowledges_network_batches() {
        let mut runtime = BrokerRuntime::start(test_config()).await.unwrap();
        let url = request_extension_ws_url(&runtime);
        let mut request = url.into_client_request().unwrap();
        request.headers_mut().insert(
            WS_ORIGIN,
            WsHeaderValue::from_str(&format!("chrome-extension://{EXTENSION_ID}")).unwrap(),
        );
        let (mut socket, _) = connect_async(request).await.unwrap();
        socket
            .send(WsMessage::Text(
                serde_json::json!({
                    "type": "stream_hello",
                    "protocol_version": 1,
                    "extension_instance_id": "profile-a",
                    "project_root": "C:/stale/first-project",
                    "extension_version": "0.7.10"
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let BrokerEvent::ExtensionConnected {
            hello,
            generation,
            reply,
            ..
        } = runtime.next_event().await.unwrap()
        else {
            panic!("expected extension connection event")
        };
        assert!(matches!(hello, ExtensionStreamMessage::StreamHello { .. }));
        assert_eq!(generation, 1);
        reply
            .send(serde_json::json!({
                "type": "stream_hello_ack",
                "schema_version": 1,
                "protocol_version": 1,
                "ok": true
            }))
            .unwrap();
        assert_eq!(read_json(&mut socket).await["type"], "stream_hello_ack");

        runtime
            .send_extension_command(
                "profile-a",
                serde_json::json!({"request_id": "action-1", "cmd": "get_page_snapshot"}),
            )
            .await
            .unwrap();
        let command = read_json(&mut socket).await;
        assert_eq!(command["type"], "direct_command");
        assert_eq!(command["command"]["request_id"], "action-1");

        socket
            .send(WsMessage::Text(
                serde_json::json!({
                    "type": "network_batch",
                    "extension_instance_id": "profile-a",
                    "capture_id": "capture-a",
                    "target": {"extension_instance_id": "profile-a", "window_id": 9, "tab_id": 4},
                    "events": [{"seq": 1, "request_id": "network-1"}],
                    "first_seq": 1,
                    "last_seq": 1,
                    "dropped_events": 0,
                    "dropped_bytes": 0,
                    "dropped_events_total": 0,
                    "dropped_bytes_total": 0,
                    "diagnostics": {"retries": 0}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let BrokerEvent::NetworkBatch { batch, reply, .. } = runtime.next_event().await.unwrap()
        else {
            panic!("expected network capture batch event")
        };
        assert_eq!(batch.target.extension_instance_id, "profile-a");
        assert_eq!(batch.events[0].seq, 1);
        reply
            .send(serde_json::json!({
                "type": "network_ack",
                "capture_id": "capture-a",
                "target": {"extension_instance_id": "profile-a", "window_id": 9, "tab_id": 4},
                "ack_seq": 1,
                "accepted": true
            }))
            .unwrap();
        let ack = read_json(&mut socket).await;
        assert_eq!(ack["type"], "network_ack");
        assert_eq!(ack["ack_seq"], 1);

        let mut reconnect_request = request_extension_ws_url(&runtime)
            .into_client_request()
            .unwrap();
        reconnect_request.headers_mut().insert(
            WS_ORIGIN,
            WsHeaderValue::from_str(&format!("chrome-extension://{EXTENSION_ID}")).unwrap(),
        );
        let (mut reconnected, _) = connect_async(reconnect_request).await.unwrap();
        reconnected
            .send(WsMessage::Text(
                serde_json::json!({
                    "type": "stream_hello",
                    "protocol_version": 1,
                    "extension_instance_id": "profile-a",
                    "project_root": "C:/different/project",
                    "extension_version": "0.7.10"
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let BrokerEvent::ExtensionConnected {
            generation: next_generation,
            reply,
            ..
        } = runtime.next_event().await.unwrap()
        else {
            panic!("expected replacement extension connection event")
        };
        assert_eq!(next_generation, generation + 1);
        reply
            .send(serde_json::json!({
                "type": "stream_hello_ack",
                "schema_version": 1,
                "protocol_version": 1,
                "ok": true
            }))
            .unwrap();
        assert_eq!(
            read_json(&mut reconnected).await["type"],
            "stream_hello_ack"
        );
        assert!(matches!(
            socket.next().await.unwrap().unwrap(),
            WsMessage::Close(_)
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(30), runtime.events.recv())
                .await
                .is_err()
        );
        runtime
            .send_extension_command(
                "profile-a",
                serde_json::json!({"request_id": "reconnected-action", "cmd": "get_page_snapshot"}),
            )
            .await
            .unwrap();
        assert_eq!(
            read_json(&mut reconnected).await["command"]["request_id"],
            "reconnected-action"
        );

        reconnected.close(None).await.unwrap();
        let BrokerEvent::ExtensionDisconnected {
            extension_instance_id,
            generation: disconnected_generation,
        } = runtime.next_event().await.unwrap()
        else {
            panic!("expected extension disconnect event")
        };
        assert_eq!(extension_instance_id, "profile-a");
        assert_eq!(disconnected_generation, next_generation);
        drop(socket);
        drop(reconnected);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn direct_websocket_rejects_unknown_operations_and_correlates_supported_requests() {
        let mut runtime = BrokerRuntime::start(test_config()).await.unwrap();
        let (mut socket, _) = connect_async(request_ws_url(&runtime)).await.unwrap();
        socket
            .send(WsMessage::Text(
                serde_json::json!({"request_id": "bad-1", "cmd": "delete_all_files"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let rejected = read_json(&mut socket).await;
        assert_eq!(rejected["code"], "invalid_browser_operation");
        assert!(
            tokio::time::timeout(Duration::from_millis(30), runtime.events.recv())
                .await
                .is_err()
        );

        socket
            .send(WsMessage::Text(
                serde_json::json!({
                    "schema_version": 1,
                    "request_id": "snapshot-1",
                    "cmd": "get_page_snapshot",
                    "target": {"extension_instance_id": "profile-a", "window_id": 2, "tab_id": 3}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let BrokerEvent::Operation { request, reply, .. } = runtime.next_event().await.unwrap()
        else {
            panic!("expected supported browser operation")
        };
        assert_eq!(request.request_id, "snapshot-1");
        reply
            .send(Ok(serde_json::json!({"ok": true, "revision": "r1"})))
            .unwrap();
        assert_eq!(read_json(&mut socket).await["revision"], "r1");
        socket.close(None).await.unwrap();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn same_client_websocket_can_receive_cancel_while_operation_is_pending() {
        let mut runtime = BrokerRuntime::start(test_config()).await.unwrap();
        let (mut socket, _) = connect_async(request_ws_url(&runtime)).await.unwrap();
        socket
            .send(WsMessage::Text(
                serde_json::json!({
                    "schema_version": 1,
                    "request_id": "pending-1",
                    "cmd": "get_page_snapshot",
                    "target": {"extension_instance_id": "profile-a", "window_id": 2, "tab_id": 3}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let BrokerEvent::Operation {
            request: pending_request,
            reply: pending_reply,
            ..
        } = runtime.next_event().await.unwrap()
        else {
            panic!("expected the pending browser operation")
        };

        socket
            .send(WsMessage::Text(
                serde_json::json!({
                    "schema_version": 1,
                    "request_id": "cancel-1",
                    "caller_label": "caller-a",
                    "project_root": "C:/project-a",
                    "cmd": "cancel_browser_request",
                    "cancel_request_id": pending_request.request_id.clone()
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let BrokerEvent::Operation {
            request: cancel_request,
            reply: cancel_reply,
            ..
        } = tokio::time::timeout(Duration::from_millis(250), runtime.next_event())
            .await
            .expect("same WebSocket stopped reading the cancellation")
            .unwrap()
        else {
            panic!("expected the cancellation operation")
        };
        assert_eq!(cancel_request.operation, "cancel_browser_request");

        cancel_reply
            .send(Ok(serde_json::json!({
                "type": "response",
                "request_id": "cancel-1",
                "operation": "cancel_browser_request",
                "ok": true,
                "cancelled": true,
                "cancel_request_id": "pending-1"
            })))
            .unwrap();
        pending_reply
            .send(Err(BrokerError::new(
                BrokerErrorCode::BrowserOperationCancelled,
                "browser operation was explicitly cancelled",
            )))
            .unwrap();

        let first = read_json(&mut socket).await;
        let second = read_json(&mut socket).await;
        let responses = [first, second];
        let cancelled = responses
            .iter()
            .find(|response| response["request_id"] == "cancel-1")
            .unwrap();
        let pending = responses
            .iter()
            .find(|response| response["request_id"] == "pending-1")
            .unwrap();
        assert_eq!(cancelled["cancelled"], true);
        assert_eq!(pending["code"], "browser_operation_cancelled");
        socket.close(None).await.unwrap();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn direct_response_budget_exhaustion_returns_resource_error() {
        let mut runtime = BrokerRuntime::start(test_config()).await.unwrap();
        let (mut socket, _) = connect_async(request_ws_url(&runtime)).await.unwrap();
        socket
            .send(WsMessage::Text(
                serde_json::json!({
                    "schema_version": 1,
                    "request_id": "response-budget-1",
                    "cmd": "get_page_snapshot",
                    "target": {"extension_instance_id": "profile-a", "window_id": 2, "tab_id": 3}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let BrokerEvent::Operation { request, reply, .. } = runtime.next_event().await.unwrap()
        else {
            panic!("expected browser operation")
        };
        let available = runtime.state.event_byte_budget.available_permits();
        let _all_remaining = Arc::clone(&runtime.state.event_byte_budget)
            .try_acquire_many_owned(available as u32)
            .unwrap();
        reply.send(Ok(serde_json::json!({"ok": true}))).unwrap();

        let rejected = read_json(&mut socket).await;
        assert_eq!(rejected["request_id"], request.request_id);
        assert_eq!(rejected["code"], "browser_resource_limit");
        socket.close(None).await.unwrap();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn full_event_queue_rejects_without_dispatching_a_second_operation() {
        let mut config = test_config();
        config.event_queue_capacity = 1;
        let mut runtime = BrokerRuntime::start(config).await.unwrap();
        let (mut first, _) = connect_async(request_ws_url(&runtime)).await.unwrap();
        let (mut second, _) = connect_async(request_ws_url(&runtime)).await.unwrap();
        let snapshot = |request_id: &str| {
            serde_json::json!({
                "schema_version": 1,
                "request_id": request_id,
                "cmd": "get_page_snapshot",
                "target": {"extension_instance_id": "profile-a", "window_id": 2, "tab_id": 3}
            })
            .to_string()
        };
        first
            .send(WsMessage::Text(snapshot("queued-1").into()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while runtime.events.capacity() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        second
            .send(WsMessage::Text(snapshot("rejected-2").into()))
            .await
            .unwrap();
        let rejected = read_json(&mut second).await;
        assert_eq!(rejected["code"], "browser_resource_limit");

        let BrokerEvent::Operation { request, reply, .. } = runtime.next_event().await.unwrap()
        else {
            panic!("expected the first queued operation only")
        };
        assert_eq!(request.request_id, "queued-1");
        reply.send(Ok(serde_json::json!({"ok": true}))).unwrap();
        assert_eq!(read_json(&mut first).await["ok"], true);
        first.close(None).await.unwrap();
        second.close(None).await.unwrap();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn response_deadline_is_explicit_and_late_reply_is_not_delivered() {
        let mut config = test_config();
        config.event_response_timeout = Duration::from_millis(100);
        let mut runtime = BrokerRuntime::start(config).await.unwrap();
        let (mut socket, _) = connect_async(request_ws_url(&runtime)).await.unwrap();
        socket
            .send(WsMessage::Text(
                serde_json::json!({
                    "schema_version": 1,
                    "request_id": "deadline-1",
                    "cmd": "get_page_snapshot",
                    "target": {"extension_instance_id": "profile-a", "window_id": 2, "tab_id": 3}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let BrokerEvent::Operation { request, reply, .. } = runtime.next_event().await.unwrap()
        else {
            panic!("expected the timed operation")
        };
        let response = read_json(&mut socket).await;
        assert_eq!(response["request_id"], "deadline-1");
        assert_eq!(response["code"], "browser_operation_timeout");
        assert!(reply.send(Ok(serde_json::json!({"ok": true}))).is_err());
        assert_eq!(request.request_id, "deadline-1");
        socket.close(None).await.unwrap();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn full_profile_outbound_queue_returns_without_blocking_state_transport() {
        let runtime = BrokerRuntime::start(test_config()).await.unwrap();
        let (sender, _receiver) = mpsc::channel(EXTENSION_OUTBOUND_QUEUE_CAPACITY);
        let (other_sender, mut other_receiver) = mpsc::channel(EXTENSION_OUTBOUND_QUEUE_CAPACITY);
        runtime.state.streams.write().await.extend([
            ("profile-a".into(), (1, sender)),
            ("profile-b".into(), (1, other_sender)),
        ]);

        for index in 0..EXTENSION_OUTBOUND_QUEUE_CAPACITY {
            runtime
                .send_extension_command(
                    "profile-a",
                    serde_json::json!({"request_id": format!("queued-{index}"), "cmd": "get_page_snapshot"}),
                )
                .await
                .unwrap();
        }

        let result = tokio::time::timeout(
            Duration::from_millis(100),
            runtime.send_extension_command(
                "profile-a",
                serde_json::json!({"request_id": "overflow", "cmd": "get_page_snapshot"}),
            ),
        )
        .await
        .expect("a full Profile queue blocked the broker event loop")
        .unwrap_err();
        assert_eq!(result.code, BrokerErrorCode::BrowserResourceLimit);
        tokio::time::timeout(
            Duration::from_millis(100),
            runtime.send_extension_command(
                "profile-b",
                serde_json::json!({"request_id": "profile-b-command", "cmd": "heartbeat"}),
            ),
        )
        .await
        .expect("Profile B command was blocked by Profile A")
        .unwrap();
        assert!(other_receiver.try_recv().is_ok());
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn extension_websocket_rejects_untrusted_origins_and_stale_tokens() {
        let mut runtime = BrokerRuntime::start(test_config()).await.unwrap();
        let endpoint = runtime.endpoint_record();
        let valid_token = runtime.credential().to_owned();

        for (origin, token) in [
            ("https://attacker.example", valid_token.clone()),
            (
                "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "stale-token-that-is-long-enough-to-look-valid".to_owned(),
            ),
        ] {
            let mut request = format!("{}?token={token}", endpoint.extension_frame_ws_url)
                .into_client_request()
                .unwrap();
            request
                .headers_mut()
                .insert(WS_ORIGIN, WsHeaderValue::from_str(origin).unwrap());
            let error = connect_async(request).await.unwrap_err();
            let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
                panic!("expected handshake rejection, got {error}")
            };
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(30), runtime.events.recv())
                .await
                .is_err()
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn control_http_body_and_websocket_connection_limits_are_enforced() {
        let mut config = test_config();
        config.max_websocket_connections = 1;
        let runtime = BrokerRuntime::start(config).await.unwrap();
        let endpoint = runtime.endpoint_record();
        let client = reqwest::Client::new();
        let origin = format!("chrome-extension://{EXTENSION_ID}");
        let response = client
            .post(format!(
                "{}/heartbeat?token={}",
                endpoint.discovery_url,
                runtime.credential()
            ))
            .header("X-Teshi-Broker-Token", runtime.credential())
            .header("Origin", &origin)
            .header("Content-Type", "application/json")
            .body(vec![b' '; MAX_HTTP_BODY_BYTES + 1])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let oversized_headers = client
            .get(&endpoint.discovery_url)
            .header("X-Teshi-Test", "h".repeat(MAX_REQUEST_HEADER_BYTES + 512))
            .send()
            .await
            .unwrap();
        assert_eq!(
            oversized_headers.status(),
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
        );

        let (mut first, _) = connect_async(request_ws_url(&runtime)).await.unwrap();
        let error = connect_async(request_ws_url(&runtime)).await.unwrap_err();
        let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
            panic!("expected socket limit rejection, got {error}")
        };
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        first.close(None).await.unwrap();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn extension_stream_limit_rejects_excess_profiles_without_replacing_first() {
        let mut config = test_config();
        config.max_extension_streams = 1;
        let mut runtime = BrokerRuntime::start(config).await.unwrap();
        let endpoint = runtime.endpoint_record();
        let mut first_request = request_extension_ws_url(&runtime)
            .into_client_request()
            .unwrap();
        first_request.headers_mut().insert(
            WS_ORIGIN,
            WsHeaderValue::from_str(&format!("chrome-extension://{EXTENSION_ID}")).unwrap(),
        );
        let (mut first, _) = connect_async(first_request).await.unwrap();

        let mut second_request = endpoint.extension_frame_ws_url + "?token=";
        second_request.push_str(runtime.credential());
        let mut second_request = second_request.into_client_request().unwrap();
        second_request.headers_mut().insert(
            WS_ORIGIN,
            WsHeaderValue::from_str(&format!("chrome-extension://{EXTENSION_ID}")).unwrap(),
        );
        let error = connect_async(second_request).await.unwrap_err();
        let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
            panic!("expected extension stream limit rejection, got {error}")
        };
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        first
            .send(WsMessage::Text(
                serde_json::json!({
                    "type": "stream_hello",
                    "protocol_version": 1,
                    "extension_instance_id": "profile-a",
                    "extension_version": "0.7.10"
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let BrokerEvent::ExtensionConnected { reply, .. } = runtime.next_event().await.unwrap()
        else {
            panic!("first extension stream was displaced")
        };
        reply.send(serde_json::json!({"ok": true})).unwrap();
        assert_eq!(read_json(&mut first).await["ok"], true);

        first.close(None).await.unwrap();
        runtime.shutdown().await;
    }

    #[test]
    fn queued_event_byte_budget_fails_closed_when_exhausted() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut config = test_config();
            config.queued_event_bytes = MAX_WEBSOCKET_MESSAGE_BYTES;
            let runtime = BrokerRuntime::start(config).await.unwrap();
            let all = Arc::clone(&runtime.state.event_byte_budget)
                .try_acquire_many_owned(MAX_WEBSOCKET_MESSAGE_BYTES as u32)
                .unwrap();
            assert_eq!(
                reserve_event_bytes(&runtime.state, 1).unwrap_err().code,
                BrokerErrorCode::BrowserResourceLimit
            );
            drop(all);
            assert!(reserve_event_bytes(&runtime.state, 1).is_ok());
            runtime.shutdown().await;
        });
    }
}
