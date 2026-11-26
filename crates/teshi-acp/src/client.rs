use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use agent_client_protocol::schema::v1::{
    CloseSessionRequest, DeleteSessionRequest, ListSessionsRequest, LoadSessionRequest,
    NewSessionRequest, ResumeSessionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    process::Child,
    sync::{Mutex, mpsc, oneshot, watch},
    task::JoinHandle,
};

use crate::{
    command::AcpAgentCommand,
    error::{AcpError, AcpResult},
    jsonrpc::{self, InboundMessage, JsonRpcMessage, JsonRpcPeer},
    permission::{PermissionDecision, PermissionHost, PermissionOption, PermissionPolicy},
};

/// Longest wait for `session/cancel` to reach an agent that may not be reading.
const CANCEL_DELIVERY_TIMEOUT: Duration = Duration::from_secs(2);

/// Per-route outbound event buffer capacity so one slow consumer cannot stall
/// the shared JSON-RPC dispatcher or reader loop.
const ROUTE_EVENT_BUFFER: usize = 4096;

/// Bounded wait when flushing `AcpEvent::Completed` through a route forwarder so
/// callers that drain a small bounded `events` receiver after `prompt` returns
/// cannot deadlock on a full channel.
const ROUTE_FLUSH_TIMEOUT: Duration = Duration::from_millis(100);

/// Bounded wait for the dispatcher fence so trailing notifications arrive before
/// `AcpEvent::Completed`.
const DISPATCHER_FENCE_TIMEOUT: Duration = Duration::from_secs(2);

/// Upper bound on `session/list` pagination pages to guard against faulty servers.
const MAX_LIST_SESSIONS_PAGES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpLifecycle {
    Created,
    Starting,
    Initialized,
    Authenticated,
    Ready,
    ShuttingDown,
    Exited,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

impl Default for ClientInfo {
    fn default() -> Self {
        Self {
            name: "teshi".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthMethodInfo {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSessionInfo {
    pub session_id: String,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(default)]
    pub protocol_version: Option<u32>,
    #[serde(default)]
    pub agent_info: Value,
    #[serde(default)]
    pub auth_methods: Vec<String>,
    #[serde(default)]
    pub auth_method_details: Vec<AuthMethodInfo>,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(default)]
    pub raw: Value,
}

impl InitializeResult {
    /// Whether the agent accepts image blocks in `session/prompt`.
    ///
    /// ACP makes image prompts opt-in through
    /// `agentCapabilities.promptCapabilities.image`; anything else means text only.
    pub fn accepts_images(&self) -> bool {
        self.capabilities
            .pointer("/promptCapabilities/image")
            .and_then(Value::as_bool)
            == Some(true)
    }

    /// Whether the agent advertises top-level `loadSession` for `session/load`.
    pub fn supports_load_session(&self) -> bool {
        self.capabilities
            .get("loadSession")
            .and_then(Value::as_bool)
            == Some(true)
    }

    /// Whether the agent advertises `sessionCapabilities.resume` for `session/resume`.
    pub fn supports_resume_session(&self) -> bool {
        self.capabilities
            .pointer("/sessionCapabilities/resume")
            .is_some_and(Value::is_object)
    }

    /// Whether the agent advertises `sessionCapabilities.list` for `session/list`.
    pub fn supports_list_sessions(&self) -> bool {
        self.capabilities
            .pointer("/sessionCapabilities/list")
            .is_some_and(Value::is_object)
    }

    /// Whether the agent advertises `sessionCapabilities.close` for `session/close`.
    pub fn supports_close_session(&self) -> bool {
        self.capabilities
            .pointer("/sessionCapabilities/close")
            .is_some_and(Value::is_object)
    }

    /// Whether the agent advertises `sessionCapabilities.delete` for `session/delete`.
    pub fn supports_delete_session(&self) -> bool {
        self.capabilities
            .pointer("/sessionCapabilities/delete")
            .is_some_and(Value::is_object)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcpSession {
    pub id: String,
    pub cwd: PathBuf,
    /// Settings discovered while creating or loading this session.
    #[serde(default)]
    pub config: crate::session_config::SessionConfig,
}

/// Result of one `session/prompt` turn.
///
/// `stop_reason` is the protocol value. `detail` is a user-facing explanation
/// some agents attach beside an abnormal stop, for example a quota error that
/// they still report as `refusal`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptCompletion {
    /// `stopReason` from the prompt result, when the agent included one.
    pub stop_reason: Option<String>,
    /// Short explanation extracted from the prompt result, when present.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AcpEvent {
    AgentMessageChunk(String),
    ToolCall(Value),
    ToolCallUpdate(Value),
    PermissionRequest(Value),
    SessionUpdate(Value),
    ExtensionNotification {
        method: String,
        params: Value,
    },
    Completed {
        stop_reason: Option<String>,
        /// Agent-supplied explanation for this stop, when the result carried one.
        detail: Option<String>,
    },
    Error(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct AcpClientConfig {
    pub initialize_timeout: Duration,
    pub authentication_timeout: Duration,
    pub request_timeout: Duration,
    /// Optional limit for a complete agent turn. By default prompts have no
    /// client-side timeout because ACP turns may legitimately be long-running.
    pub prompt_timeout: Option<Duration>,
    pub shutdown_timeout: Duration,
    pub client_info: ClientInfo,
    pub protocol_version: u32,
}

impl Default for AcpClientConfig {
    fn default() -> Self {
        Self {
            initialize_timeout: Duration::from_secs(20),
            authentication_timeout: Duration::from_secs(20),
            request_timeout: Duration::from_secs(30),
            prompt_timeout: None,
            shutdown_timeout: Duration::from_secs(5),
            client_info: ClientInfo::default(),
            protocol_version: 1,
        }
    }
}

struct RouteEventEnvelope {
    event: Option<AcpEvent>,
    ack: Option<oneshot::Sender<()>>,
}

struct SessionRoute {
    events: mpsc::Sender<RouteEventEnvelope>,
    permission_policy: Arc<Mutex<PermissionPolicy>>,
    permission_host: Option<Arc<dyn PermissionHost>>,
    prompt_running: Arc<AtomicBool>,
    pending_permissions: Arc<StdMutex<HashMap<jsonrpc::JsonRpcId, oneshot::Sender<()>>>>,
}

impl SessionRoute {
    fn new(
        caller_events: mpsc::Sender<AcpEvent>,
        permission_policy: PermissionPolicy,
        permission_host: Option<Arc<dyn PermissionHost>>,
    ) -> Arc<Self> {
        let (tx, mut rx) = mpsc::channel::<RouteEventEnvelope>(ROUTE_EVENT_BUFFER);
        tokio::spawn(async move {
            while let Some(envelope) = rx.recv().await {
                let delivered = match envelope.event {
                    Some(event) => caller_events.send(event).await.is_ok(),
                    None => !caller_events.is_closed(),
                };
                if let Some(ack) = envelope.ack {
                    let _ = ack.send(());
                }
                if !delivered {
                    break;
                }
            }
        });
        Arc::new(Self {
            events: tx,
            permission_policy: Arc::new(Mutex::new(permission_policy)),
            permission_host,
            prompt_running: Arc::new(AtomicBool::new(false)),
            pending_permissions: Arc::new(StdMutex::new(HashMap::new())),
        })
    }

    fn send_nonblocking(&self, event: AcpEvent) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.events.try_send(RouteEventEnvelope {
            event: Some(event),
            ack: None,
        }) {
            tracing::warn!(
                target: "teshi_acp::client",
                "dropping ACP event for slow session consumer to prevent dispatcher deadlock"
            );
        }
    }

    async fn send_and_flush(&self, event: AcpEvent) {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = tokio::time::timeout(ROUTE_FLUSH_TIMEOUT, async {
            if self
                .events
                .send(RouteEventEnvelope {
                    event: Some(event),
                    ack: Some(ack_tx),
                })
                .await
                .is_ok()
            {
                let _ = ack_rx.await;
            }
        })
        .await;
    }

    async fn flush_barrier(&self, timeout: Duration) {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = tokio::time::timeout(timeout, async {
            if self
                .events
                .send(RouteEventEnvelope {
                    event: None,
                    ack: Some(ack_tx),
                })
                .await
                .is_ok()
            {
                let _ = ack_rx.await;
            }
        })
        .await;
    }
}

#[derive(Default)]
struct RouteTable {
    routes: HashMap<String, Arc<SessionRoute>>,
    pending: Option<Arc<SessionRoute>>,
    routed_mode: bool,
    unregistered_sessions: HashSet<String>,
}

struct PendingRouteGuard {
    route_table: Arc<StdMutex<RouteTable>>,
    armed: bool,
}

impl PendingRouteGuard {
    fn install(route_table: &Arc<StdMutex<RouteTable>>, route: Arc<SessionRoute>) -> Self {
        {
            let mut guard = route_table.lock().unwrap_or_else(|e| e.into_inner());
            guard.routed_mode = true;
            guard.pending = Some(route);
        }
        Self {
            route_table: Arc::clone(route_table),
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingRouteGuard {
    fn drop(&mut self) {
        if self.armed {
            let pending = {
                let mut guard = self.route_table.lock().unwrap_or_else(|e| e.into_inner());
                guard.pending.take()
            };
            if let Some(route) = pending {
                cancel_pending_permissions(&route.pending_permissions);
            }
        }
    }
}

#[derive(Default)]
struct ClientRuntime {
    reader_task: Option<JoinHandle<()>>,
    dispatcher_task: Option<JoinHandle<()>>,
    stderr_task: Option<JoinHandle<()>>,
    child: Option<Child>,
    process_tree: Option<crate::process_tree::ProcessTree>,
}

pub struct AcpClient<W> {
    peer: JsonRpcPeer<W>,
    inbound_tx: mpsc::Sender<InboundMessage>,
    inbound: Mutex<Option<mpsc::Receiver<InboundMessage>>>,
    default_route: Arc<SessionRoute>,
    route_table: Arc<StdMutex<RouteTable>>,
    session_create_lock: Arc<Mutex<()>>,
    active_prompts: Arc<AtomicUsize>,
    lifecycle: Arc<Mutex<AcpLifecycle>>,
    config: AcpClientConfig,
    initialized: Arc<Mutex<Option<InitializeResult>>>,
    alive: Arc<AtomicBool>,
    shutting_down: Arc<AtomicBool>,
    exit_tx: watch::Sender<bool>,
    child_pid: Option<u32>,
    runtime: Mutex<ClientRuntime>,
}

impl AcpClient<tokio::process::ChildStdin> {
    pub async fn spawn(
        command: AcpAgentCommand,
        config: AcpClientConfig,
        events: mpsc::Sender<AcpEvent>,
        permission_policy: PermissionPolicy,
        permission_host: Option<Arc<dyn PermissionHost>>,
    ) -> AcpResult<Self> {
        let mut process_command = command.to_tokio_command();
        crate::process_tree::ProcessTree::configure(&mut process_command);
        let mut child = process_command
            .spawn()
            .map_err(|source| AcpError::SpawnFailed {
                program: command.program.display().to_string(),
                source,
            })?;
        let process_tree = match crate::process_tree::ProcessTree::attach(&child) {
            Ok(tree) => tree,
            Err(source) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(AcpError::SpawnFailed {
                    program: command.program.display().to_string(),
                    source,
                });
            }
        };
        let pid = child.id();
        let stdin = child.stdin.take().ok_or_else(|| AcpError::SpawnFailed {
            program: command.program.display().to_string(),
            source: std::io::Error::other("missing stdin"),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| AcpError::SpawnFailed {
            program: command.program.display().to_string(),
            source: std::io::Error::other("missing stdout"),
        })?;
        let stderr = child.stderr.take();
        let mut client = Self::from_io(
            stdout,
            stdin,
            config,
            events,
            permission_policy,
            permission_host,
        );
        client.child_pid = pid;
        let runtime = client.runtime.get_mut();
        runtime.child = Some(child);
        runtime.process_tree = Some(process_tree);
        if let Some(stderr) = stderr {
            runtime.stderr_task = Some(tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(target: "teshi_acp::stderr", line = %crate::error::redact_secret(&line), "ACP agent stderr");
                }
            }));
        }
        Ok(client)
    }
}

impl<W: AsyncWrite + Unpin + Send + 'static> AcpClient<W> {
    pub fn from_io<R: AsyncRead + Unpin + Send + 'static>(
        reader: R,
        writer: W,
        config: AcpClientConfig,
        events: mpsc::Sender<AcpEvent>,
        permission_policy: PermissionPolicy,
        permission_host: Option<Arc<dyn PermissionHost>>,
    ) -> Self {
        let peer = JsonRpcPeer::new(writer);
        let (tx, rx) = mpsc::channel(128);
        let alive = Arc::new(AtomicBool::new(true));
        let (exit_tx, _) = watch::channel(false);
        let reader_task = tokio::spawn(jsonrpc::reader_loop_with_liveness(
            reader,
            peer.clone(),
            tx.clone(),
            Some((Arc::clone(&alive), exit_tx.clone())),
        ));
        Self {
            peer,
            inbound_tx: tx,
            inbound: Mutex::new(Some(rx)),
            default_route: SessionRoute::new(events, permission_policy, permission_host),
            route_table: Arc::new(StdMutex::new(RouteTable::default())),
            session_create_lock: Arc::new(Mutex::new(())),
            active_prompts: Arc::new(AtomicUsize::new(0)),
            lifecycle: Arc::new(Mutex::new(AcpLifecycle::Created)),
            config,
            initialized: Arc::new(Mutex::new(None)),
            alive,
            shutting_down: Arc::new(AtomicBool::new(false)),
            exit_tx,
            child_pid: None,
            runtime: Mutex::new(ClientRuntime {
                reader_task: Some(reader_task),
                dispatcher_task: None,
                stderr_task: None,
                child: None,
                process_tree: None,
            }),
        }
    }

    /// OS process id of the spawned ACP child, when backed by an external process.
    pub fn process_id(&self) -> Option<u32> {
        self.child_pid
    }

    /// Whether the underlying transport/process is still believed alive.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && !self.shutting_down.load(Ordering::SeqCst)
    }

    /// Whether the client has no registered session routes, no in-flight session
    /// creation, and no running prompt turn.
    pub fn is_idle_clean(&self) -> bool {
        let table = self.route_table.lock().unwrap_or_else(|e| e.into_inner());
        table.routes.is_empty()
            && table.pending.is_none()
            && self.active_prompts.load(Ordering::SeqCst) == 0
    }

    /// Check liveness, including non-blocking child exit status when backed by a child process.
    pub async fn check_alive(&self) -> bool {
        if !self.is_alive() {
            return false;
        }
        let exited = {
            let mut runtime = self.runtime.lock().await;
            runtime
                .child
                .as_mut()
                .is_some_and(|child| matches!(child.try_wait(), Ok(Some(_)) | Err(_)))
        };
        if exited {
            self.alive.store(false, Ordering::SeqCst);
            let _ = self.exit_tx.send(true);
            return false;
        }
        true
    }

    /// Subscribe to transport/process exit notifications (`true` once exited).
    pub fn subscribe_exit(&self) -> watch::Receiver<bool> {
        self.exit_tx.subscribe()
    }

    /// Cached initialization payload after [`Self::initialize`].
    pub async fn initialized_result(&self) -> Option<InitializeResult> {
        self.initialized.lock().await.clone()
    }

    /// Register a per-session route for `session/update` and `session/request_permission`.
    pub async fn register_session(
        &self,
        session_id: impl Into<String>,
        events: mpsc::Sender<AcpEvent>,
        permission_policy: PermissionPolicy,
        permission_host: Option<Arc<dyn PermissionHost>>,
    ) {
        let session_id = session_id.into();
        let route = SessionRoute::new(events, permission_policy, permission_host);
        let replaced = {
            let mut table = self.route_table.lock().unwrap_or_else(|e| e.into_inner());
            table.routed_mode = true;
            table.unregistered_sessions.remove(&session_id);
            table.routes.insert(session_id, route)
        };
        if let Some(old_route) = replaced {
            cancel_pending_permissions(&old_route.pending_permissions);
        }
    }

    /// Remove a previously registered session route and cancel any pending permissions on it.
    pub async fn unregister_session(&self, session_id: &str) {
        let removed = {
            let mut table = self.route_table.lock().unwrap_or_else(|e| e.into_inner());
            let removed = table.routes.remove(session_id);
            if removed.is_some() || table.routed_mode {
                table.unregistered_sessions.insert(session_id.to_string());
            }
            removed
        };
        if let Some(route) = removed {
            cancel_pending_permissions(&route.pending_permissions);
        }
    }

    /// Update the permission policy for a registered session (or the default route if unregistered).
    pub async fn set_session_permission_policy(&self, session_id: &str, policy: PermissionPolicy) {
        let route = self.route_for(session_id);
        *route.permission_policy.lock().await = policy;
    }

    pub async fn lifecycle(&self) -> AcpLifecycle {
        *self.lifecycle.lock().await
    }

    /// Set the policy for future permission requests on the default route; pending requests keep their decision path.
    pub async fn set_permission_policy(&self, policy: PermissionPolicy) {
        *self.default_route.permission_policy.lock().await = policy;
    }

    fn route_for(&self, session_id: &str) -> Arc<SessionRoute> {
        let table = self.route_table.lock().unwrap_or_else(|e| e.into_inner());
        table
            .routes
            .get(session_id)
            .cloned()
            .unwrap_or_else(|| Arc::clone(&self.default_route))
    }

    async fn set_state(&self, to: AcpLifecycle) {
        *self.lifecycle.lock().await = to;
    }

    async fn set_state_if_active(&self, to: AcpLifecycle) {
        let mut state = self.lifecycle.lock().await;
        if !matches!(*state, AcpLifecycle::ShuttingDown | AcpLifecycle::Exited) {
            *state = to;
        }
    }

    async fn transition_state(
        &self,
        allowed: &[AcpLifecycle],
        to: AcpLifecycle,
        action: &'static str,
    ) -> AcpResult<()> {
        let mut state = self.lifecycle.lock().await;
        if allowed.contains(&*state) {
            *state = to;
            Ok(())
        } else {
            Err(AcpError::InvalidState {
                from: *state,
                to: action,
            })
        }
    }

    async fn ensure_state(&self, allowed: &[AcpLifecycle], to: &'static str) -> AcpResult<()> {
        let state = *self.lifecycle.lock().await;
        if allowed.contains(&state) {
            Ok(())
        } else {
            Err(AcpError::InvalidState { from: state, to })
        }
    }

    pub async fn initialize(&mut self) -> AcpResult<InitializeResult> {
        self.initialize_shared().await
    }

    /// Initialize an `Arc`-shared client without requiring `&mut self`.
    pub async fn initialize_shared(&self) -> AcpResult<InitializeResult> {
        self.transition_state(
            &[AcpLifecycle::Created],
            AcpLifecycle::Starting,
            "initialize",
        )
        .await?;
        self.start_dispatcher().await;
        let params = json!({
            "protocolVersion": self.config.protocol_version,
            "clientCapabilities": { "filesystem": false, "terminal": false },
            "clientInfo": { "name": self.config.client_info.name, "version": self.config.client_info.version }
        });
        let result = match self
            .request_with_timeout("initialize", params, self.config.initialize_timeout)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                self.set_state_if_active(AcpLifecycle::Failed).await;
                return Err(AcpError::InitializeFailed(e.to_string()));
            }
        };
        let protocol_version = result
            .get("protocolVersion")
            .and_then(Value::as_u64)
            .map(|v| v as u32);
        if let Some(v) = protocol_version
            && v != self.config.protocol_version
        {
            self.set_state_if_active(AcpLifecycle::Failed).await;
            return Err(AcpError::UnsupportedProtocolVersion(v));
        }
        let auth_method_details = result
            .get("authMethods")
            .or_else(|| result.pointer("/authentication/methods"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| {
                        if let Some(id) = v.as_str() {
                            Some(AuthMethodInfo {
                                id: id.to_string(),
                                name: id.to_string(),
                                description: None,
                            })
                        } else {
                            let id = v.get("id").and_then(Value::as_str)?;
                            let name = v.get("name").and_then(Value::as_str).unwrap_or(id);
                            let description = v
                                .get("description")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                            Some(AuthMethodInfo {
                                id: id.to_string(),
                                name: name.to_string(),
                                description,
                            })
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let auth_methods = auth_method_details
            .iter()
            .map(|method| method.id.clone())
            .collect();
        let initialized = InitializeResult {
            protocol_version,
            agent_info: result.get("agentInfo").cloned().unwrap_or(Value::Null),
            auth_methods,
            auth_method_details,
            capabilities: result
                .get("agentCapabilities")
                .or_else(|| result.get("capabilities"))
                .cloned()
                .unwrap_or(Value::Null),
            raw: result,
        };
        *self.initialized.lock().await = Some(initialized.clone());
        self.set_state_if_active(AcpLifecycle::Initialized).await;
        Ok(initialized)
    }

    pub async fn authenticate(&self, method_id: &str) -> AcpResult<()> {
        self.ensure_state(
            &[
                AcpLifecycle::Initialized,
                AcpLifecycle::Authenticated,
                AcpLifecycle::Ready,
            ],
            "authenticate",
        )
        .await?;
        let init = self
            .initialized
            .lock()
            .await
            .clone()
            .ok_or_else(|| AcpError::InitializeFailed("missing initialize result".into()))?;
        if !init.auth_methods.iter().any(|method| method == method_id) {
            return Err(AcpError::AuthenticationFailed(format!(
                "agent did not advertise authentication method `{method_id}`"
            )));
        }
        let params = json!({ "methodId": method_id });
        match self
            .request_with_timeout("authenticate", params, self.config.authentication_timeout)
            .await
        {
            Ok(_) => {
                let mut state = self.lifecycle.lock().await;
                if *state == AcpLifecycle::Initialized {
                    *state = AcpLifecycle::Authenticated;
                }
                Ok(())
            }
            Err(
                err @ (AcpError::Timeout(_)
                | AcpError::ProcessExited
                | AcpError::Cancelled
                | AcpError::Shutdown(_)),
            ) => Err(err),
            Err(e) => Err(AcpError::AuthenticationFailed(e.to_string())),
        }
    }

    /// Create a session without client-provided MCP servers.
    ///
    /// # Errors
    /// Returns lifecycle, serialization, transport, or remote session errors.
    pub async fn new_session(&self, cwd: PathBuf) -> AcpResult<AcpSession> {
        self.new_session_with_mcp(cwd, Vec::new()).await
    }

    /// Create a session with client-provided MCP tool servers.
    ///
    /// # Errors
    /// Returns lifecycle, serialization, transport, or remote session errors.
    pub async fn new_session_with_mcp(
        &self,
        cwd: PathBuf,
        servers: Vec<crate::McpServer>,
    ) -> AcpResult<AcpSession> {
        self.new_session_with_mcp_inner(cwd, servers, None).await
    }

    /// Create a session and atomically register its dedicated event/permission route.
    ///
    /// # Errors
    /// Returns lifecycle, serialization, transport, or remote session errors.
    pub async fn new_session_with_mcp_routed(
        &self,
        cwd: PathBuf,
        servers: Vec<crate::McpServer>,
        events: mpsc::Sender<AcpEvent>,
        permission_policy: PermissionPolicy,
        permission_host: Option<Arc<dyn PermissionHost>>,
    ) -> AcpResult<AcpSession> {
        let route = SessionRoute::new(events, permission_policy, permission_host);
        self.new_session_with_mcp_inner(cwd, servers, Some(route))
            .await
    }

    async fn new_session_with_mcp_inner(
        &self,
        cwd: PathBuf,
        servers: Vec<crate::McpServer>,
        route: Option<Arc<SessionRoute>>,
    ) -> AcpResult<AcpSession> {
        self.ensure_state(
            &[
                AcpLifecycle::Initialized,
                AcpLifecycle::Authenticated,
                AcpLifecycle::Ready,
            ],
            "session/new",
        )
        .await?;
        let params = serialize_params(NewSessionRequest::new(cwd.clone()).mcp_servers(servers))?;
        let _create_guard = self.session_create_lock.lock().await;
        let mut pending_guard = route
            .as_ref()
            .map(|r| PendingRouteGuard::install(&self.route_table, Arc::clone(r)));
        let result = self
            .request_with_timeout("session/new", params, self.config.request_timeout)
            .await?;
        let id = result
            .get("sessionId")
            .or_else(|| result.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| AcpError::SessionFailed("session/new missing sessionId".into()))?
            .to_string();
        let replaced = {
            let mut table = self.route_table.lock().unwrap_or_else(|e| e.into_inner());
            let replaced = if let Some(route) = route {
                table.routed_mode = true;
                table.unregistered_sessions.remove(&id);
                table.routes.insert(id.clone(), route)
            } else {
                None
            };
            table.pending = None;
            replaced
        };
        if let Some(old_route) = replaced {
            cancel_pending_permissions(&old_route.pending_permissions);
        }
        if let Some(guard) = pending_guard.as_mut() {
            guard.disarm();
        }
        self.set_state_if_active(AcpLifecycle::Ready).await;
        Ok(AcpSession {
            id,
            cwd,
            config: crate::session_config::SessionConfig::from_response(&result),
        })
    }

    pub async fn list_sessions(&self) -> AcpResult<Value> {
        self.ensure_state(
            &[
                AcpLifecycle::Initialized,
                AcpLifecycle::Authenticated,
                AcpLifecycle::Ready,
            ],
            "session/list",
        )
        .await?;
        let init = self
            .initialized
            .lock()
            .await
            .clone()
            .ok_or_else(|| AcpError::InitializeFailed("missing initialize result".into()))?;
        if !init.supports_list_sessions() {
            return Err(AcpError::CapabilityUnsupported("session/list"));
        }
        self.request_with_timeout(
            "session/list",
            serialize_params(ListSessionsRequest::new())?,
            self.config.request_timeout,
        )
        .await
    }

    /// List remote ACP sessions, optionally filtered to a working directory.
    ///
    /// Follows `nextCursor` pagination across pages up to a safe bound.
    ///
    /// # Errors
    /// Returns capability, lifecycle, serialization, transport, or remote protocol errors.
    pub async fn list_sessions_for_cwd(
        &self,
        cwd: Option<PathBuf>,
    ) -> AcpResult<Vec<RemoteSessionInfo>> {
        self.ensure_state(
            &[
                AcpLifecycle::Initialized,
                AcpLifecycle::Authenticated,
                AcpLifecycle::Ready,
            ],
            "session/list",
        )
        .await?;
        let init = self
            .initialized
            .lock()
            .await
            .clone()
            .ok_or_else(|| AcpError::InitializeFailed("missing initialize result".into()))?;
        if !init.supports_list_sessions() {
            return Err(AcpError::CapabilityUnsupported("session/list"));
        }
        let mut sessions = Vec::new();
        let mut seen_cursors = HashSet::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_SESSIONS_PAGES {
            let params = serialize_params(
                ListSessionsRequest::new()
                    .cwd(cwd.clone())
                    .cursor(cursor.clone()),
            )?;
            let response = self
                .request_with_timeout("session/list", params, self.config.request_timeout)
                .await?;
            sessions.extend(parse_remote_sessions(&response));
            let next_cursor = response
                .get("nextCursor")
                .or_else(|| response.get("next_cursor"))
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            match next_cursor {
                Some(next) if seen_cursors.insert(next.clone()) => {
                    cursor = Some(next);
                }
                _ => break,
            }
        }
        Ok(sessions)
    }

    /// Close a remote ACP session and unregister its local route.
    ///
    /// Requires `agentCapabilities.sessionCapabilities.close`.
    ///
    /// # Errors
    /// Returns capability, lifecycle, serialization, transport, or remote session errors.
    pub async fn close_session(&self, id: &str) -> AcpResult<()> {
        self.ensure_state(
            &[
                AcpLifecycle::Initialized,
                AcpLifecycle::Authenticated,
                AcpLifecycle::Ready,
            ],
            "session/close",
        )
        .await?;
        let init = self
            .initialized
            .lock()
            .await
            .clone()
            .ok_or_else(|| AcpError::InitializeFailed("missing initialize result".into()))?;
        if !init.supports_close_session() {
            return Err(AcpError::CapabilityUnsupported("session/close"));
        }
        let route = {
            let table = self.route_table.lock().unwrap_or_else(|e| e.into_inner());
            table.routes.get(id).cloned()
        };
        if let Some(route) = route {
            cancel_pending_permissions(&route.pending_permissions);
        }
        let params = serialize_params(CloseSessionRequest::new(id.to_string()))?;
        let result = self
            .request_with_timeout("session/close", params, self.config.request_timeout)
            .await;
        self.unregister_session(id).await;
        result.map(|_| ())
    }

    /// Permanently delete a remote ACP session and unregister its local route.
    ///
    /// Requires `agentCapabilities.sessionCapabilities.delete`.
    ///
    /// # Errors
    /// Returns capability, lifecycle, serialization, transport, or remote session errors.
    pub async fn delete_session(&self, id: &str) -> AcpResult<()> {
        self.ensure_state(
            &[
                AcpLifecycle::Initialized,
                AcpLifecycle::Authenticated,
                AcpLifecycle::Ready,
            ],
            "session/delete",
        )
        .await?;
        let init = self
            .initialized
            .lock()
            .await
            .clone()
            .ok_or_else(|| AcpError::InitializeFailed("missing initialize result".into()))?;
        if !init.supports_delete_session() {
            return Err(AcpError::CapabilityUnsupported("session/delete"));
        }
        let route = {
            let table = self.route_table.lock().unwrap_or_else(|e| e.into_inner());
            table.routes.get(id).cloned()
        };
        if let Some(route) = route {
            cancel_pending_permissions(&route.pending_permissions);
        }
        let params = serialize_params(DeleteSessionRequest::new(id.to_string()))?;
        let result = self
            .request_with_timeout("session/delete", params, self.config.request_timeout)
            .await;
        self.unregister_session(id).await;
        result.map(|_| ())
    }

    /// Load a previously persisted ACP session without MCP servers.
    ///
    /// # Errors
    /// Returns capability, lifecycle, serialization, or remote session errors.
    pub async fn load_session(&self, id: &str, cwd: PathBuf) -> AcpResult<AcpSession> {
        self.load_session_with_mcp(id, cwd, Vec::new()).await
    }

    /// Load a previously persisted ACP session and attach MCP servers.
    ///
    /// Requires `agentCapabilities.loadSession`. The agent may replay history
    /// through `session/update` before answering; callers that already own a
    /// local transcript should ignore those updates for UI purposes.
    ///
    /// # Errors
    /// Returns capability, lifecycle, serialization, or remote session errors.
    pub async fn load_session_with_mcp(
        &self,
        id: &str,
        cwd: PathBuf,
        servers: Vec<crate::McpServer>,
    ) -> AcpResult<AcpSession> {
        self.ensure_state(
            &[
                AcpLifecycle::Initialized,
                AcpLifecycle::Authenticated,
                AcpLifecycle::Ready,
            ],
            "session/load",
        )
        .await?;
        let init = self
            .initialized
            .lock()
            .await
            .clone()
            .ok_or_else(|| AcpError::InitializeFailed("missing initialize result".into()))?;
        if !init.supports_load_session() {
            return Err(AcpError::CapabilityUnsupported("session/load"));
        }
        let params = serialize_params(
            LoadSessionRequest::new(id.to_string(), cwd.clone()).mcp_servers(servers),
        )?;
        let result = self
            .request_with_timeout("session/load", params, self.config.request_timeout)
            .await?;
        let session_id = result
            .get("sessionId")
            .or_else(|| result.get("id"))
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        self.set_state_if_active(AcpLifecycle::Ready).await;
        Ok(AcpSession {
            id: session_id,
            cwd,
            config: crate::session_config::SessionConfig::from_response(&result),
        })
    }

    /// Resume an ACP session without replaying history or attaching MCP servers.
    ///
    /// # Errors
    /// Returns capability, lifecycle, serialization, or remote session errors.
    pub async fn resume_session(&self, id: &str, cwd: PathBuf) -> AcpResult<AcpSession> {
        self.resume_session_with_mcp(id, cwd, Vec::new()).await
    }

    /// Resume an ACP session without replaying history, attaching MCP servers.
    ///
    /// Requires `agentCapabilities.sessionCapabilities.resume`. Prefer this over
    /// [`Self::load_session_with_mcp`] when the client already displays a local
    /// transcript and only needs the agent's remote context restored.
    ///
    /// # Errors
    /// Returns capability, lifecycle, serialization, or remote session errors.
    pub async fn resume_session_with_mcp(
        &self,
        id: &str,
        cwd: PathBuf,
        servers: Vec<crate::McpServer>,
    ) -> AcpResult<AcpSession> {
        self.ensure_state(
            &[
                AcpLifecycle::Initialized,
                AcpLifecycle::Authenticated,
                AcpLifecycle::Ready,
            ],
            "session/resume",
        )
        .await?;
        let init = self
            .initialized
            .lock()
            .await
            .clone()
            .ok_or_else(|| AcpError::InitializeFailed("missing initialize result".into()))?;
        if !init.supports_resume_session() {
            return Err(AcpError::CapabilityUnsupported("session/resume"));
        }
        let params = serialize_params(
            ResumeSessionRequest::new(id.to_string(), cwd.clone()).mcp_servers(servers),
        )?;
        let result = self
            .request_with_timeout("session/resume", params, self.config.request_timeout)
            .await?;
        let session_id = result
            .get("sessionId")
            .or_else(|| result.get("id"))
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        self.set_state_if_active(AcpLifecycle::Ready).await;
        Ok(AcpSession {
            id: session_id,
            cwd,
            config: crate::session_config::SessionConfig::from_response(&result),
        })
    }

    /// Change an advertised setting on the existing session.
    ///
    /// # Errors
    /// Rejects unsupported values, active turns, and remote protocol errors.
    pub async fn set_session_option(
        &self,
        session: &AcpSession,
        option: &crate::session_config::ConfigOption,
        value: Value,
    ) -> AcpResult<Value> {
        use crate::session_config::ConfigRoute;
        self.ensure_state(&[AcpLifecycle::Ready], "set session option")
            .await?;
        let route = self.route_for(&session.id);
        if route.prompt_running.load(Ordering::SeqCst) {
            return Err(AcpError::PromptAlreadyRunning);
        }
        if !option.accepts(&value) {
            return Err(AcpError::ProtocolViolation(
                "unsupported configuration value".into(),
            ));
        }
        let (method, params) = match option.route {
            ConfigRoute::Config => {
                use agent_client_protocol::schema::v1::{
                    SessionConfigOptionValue, SetSessionConfigOptionRequest,
                };
                let value = match value {
                    Value::Bool(value) => SessionConfigOptionValue::boolean(value),
                    Value::String(value) => SessionConfigOptionValue::value_id(value),
                    _ => {
                        return Err(AcpError::ProtocolViolation(
                            "unsupported configuration type".into(),
                        ));
                    }
                };
                (
                    "session/set_config_option",
                    serialize_params(SetSessionConfigOptionRequest::new(
                        session.id.clone(),
                        option.id.clone(),
                        value,
                    ))?,
                )
            }
            ConfigRoute::Mode => (
                "session/set_mode",
                json!({"sessionId":session.id,"modeId":value}),
            ),
            ConfigRoute::Model => (
                "session/set_model",
                json!({"sessionId":session.id,"modelId":value}),
            ),
        };
        self.request_with_timeout(method, params, self.config.request_timeout)
            .await
    }

    pub async fn prompt(&self, session: &AcpSession, prompt: &str) -> AcpResult<Option<String>> {
        Ok(self
            .prompt_blocks(session, &[crate::PromptBlock::Text(prompt.to_string())])
            .await?
            .stop_reason)
    }

    /// Run one turn whose prompt is a sequence of content blocks.
    ///
    /// Callers passing a small bounded `events` channel should drain it
    /// concurrently during the turn so backpressure does not delay event
    /// forwarding; `Completed` flush uses a bounded wait to avoid deadlocking
    /// callers that drain after `prompt_blocks` returns.
    ///
    /// # Errors
    /// Rejects image blocks when the agent did not advertise
    /// `promptCapabilities.image`, as well as lifecycle, transport, and remote errors.
    pub async fn prompt_blocks(
        &self,
        session: &AcpSession,
        blocks: &[crate::PromptBlock],
    ) -> AcpResult<PromptCompletion> {
        self.ensure_state(&[AcpLifecycle::Ready], "session/prompt")
            .await?;
        if blocks.iter().any(crate::PromptBlock::is_image)
            && !self
                .initialized
                .lock()
                .await
                .as_ref()
                .is_some_and(InitializeResult::accepts_images)
        {
            return Err(AcpError::CapabilityUnsupported("image prompts"));
        }
        let route = self.route_for(&session.id);
        PromptSlotGuard::try_acquire(&route.prompt_running, &self.active_prompts)?;
        let peer = self.peer.clone();
        let slot = Arc::clone(&route.prompt_running);
        let active_prompts = Arc::clone(&self.active_prompts);
        let params = json!({
            "sessionId": session.id,
            "prompt": blocks.iter().map(crate::PromptBlock::to_json).collect::<Vec<_>>()
        });
        let (result_tx, result_rx) = oneshot::channel();
        let (sent_tx, sent_rx) = watch::channel(None);
        tokio::spawn(async move {
            let result = match peer.send_request("session/prompt", params).await {
                Ok((_id, response_rx)) => {
                    let _ = sent_tx.send(Some(true));
                    response_rx.await.unwrap_or(Err(AcpError::ProcessExited))
                }
                Err(error) => {
                    let _ = sent_tx.send(Some(false));
                    Err(error)
                }
            };
            slot.store(false, Ordering::SeqCst);
            active_prompts.fetch_sub(1, Ordering::SeqCst);
            let _ = result_tx.send(result);
        });

        let cancel_action =
            self.prompt_cancel_action(session.id.clone(), Arc::clone(&route), sent_rx);
        let mut cancel_on_drop = CancelOnDrop::new(cancel_action);
        let result = match self.config.prompt_timeout {
            Some(timeout) => match tokio::time::timeout(timeout, result_rx).await {
                Ok(result) => {
                    cancel_on_drop.disarm();
                    result.unwrap_or(Err(AcpError::ProcessExited))
                }
                Err(_) => Err(AcpError::Timeout(timeout)),
            },
            None => {
                let result = result_rx.await.unwrap_or(Err(AcpError::ProcessExited));
                cancel_on_drop.disarm();
                result
            }
        };
        let result = result?;
        let completion = prompt_completion_from_result(&result);
        self.flush_dispatcher().await;
        route
            .send_and_flush(AcpEvent::Completed {
                stop_reason: completion.stop_reason.clone(),
                detail: completion.detail.clone(),
            })
            .await;
        Ok(completion)
    }

    /// Ask the agent to stop the running turn.
    ///
    /// Delivery is bounded: a wedged agent that stops reading stdin leaves an
    /// earlier write parked on the shared writer, and the caller is usually the
    /// only consumer of the event channel. Waiting without a limit would park
    /// the caller for as long as the agent stays wedged.
    ///
    /// # Errors
    /// Returns `Timeout` when the notification cannot be delivered in time, or
    /// `ProcessExited` when the agent's input is already closed.
    pub async fn cancel(&self, session: &AcpSession) -> AcpResult<()> {
        let route = self.route_for(&session.id);
        if !route.prompt_running.load(Ordering::SeqCst) {
            return Ok(());
        }
        tokio::time::timeout(CANCEL_DELIVERY_TIMEOUT, async {
            let notify_result = self
                .peer
                .notify("session/cancel", json!({ "sessionId": session.id }))
                .await;
            cancel_pending_permissions(&route.pending_permissions);
            route.send_nonblocking(AcpEvent::Cancelled);
            notify_result
        })
        .await
        .map_err(|_| AcpError::Timeout(CANCEL_DELIVERY_TIMEOUT))?
    }

    pub async fn shutdown(&mut self) -> AcpResult<()> {
        self.shutdown_shared().await
    }

    /// Shut down an `Arc`-shared client, terminating its external process and container.
    ///
    /// A failed process-tree shutdown remains retryable rather than marking `Exited`.
    pub async fn shutdown_shared(&self) -> AcpResult<()> {
        {
            let mut state = self.lifecycle.lock().await;
            if matches!(*state, AcpLifecycle::Exited) {
                return Ok(());
            }
            *state = AcpLifecycle::ShuttingDown;
        }
        self.shutting_down.store(true, Ordering::SeqCst);
        self.alive.store(false, Ordering::SeqCst);
        let _ = self.exit_tx.send(true);
        self.peer.fail_all(AcpError::Cancelled).await;
        cancel_pending_permissions(&self.default_route.pending_permissions);
        let drained_routes = {
            let mut table = self.route_table.lock().unwrap_or_else(|e| e.into_inner());
            let mut routes = table
                .routes
                .drain()
                .map(|(_, route)| route)
                .collect::<Vec<_>>();
            if let Some(pending) = table.pending.take() {
                routes.push(pending);
            }
            routes
        };
        for route in drained_routes {
            cancel_pending_permissions(&route.pending_permissions);
        }
        // Closing needs the writer, which a write blocked on a wedged agent
        // holds until the agent dies. Kill instead of waiting for that agent.
        let closed = tokio::time::timeout(self.config.shutdown_timeout, self.peer.close())
            .await
            .is_ok();
        let mut runtime = self.runtime.lock().await;
        if self.lifecycle().await == AcpLifecycle::Exited {
            return Ok(());
        }
        if let Some(child) = &mut runtime.child {
            if !closed {
                terminate_child(child).await;
            } else {
                match tokio::time::timeout(self.config.shutdown_timeout, child.wait()).await {
                    Ok(Ok(_)) => {}
                    _ => terminate_child(child).await,
                }
            }
        }
        // The container outlives the primary process, so a graceful agent exit
        // cannot leave its tools writing into the old project after a switch.
        if let Some(tree) = &mut runtime.process_tree {
            tree.shutdown()
                .await
                .map_err(|error| AcpError::Shutdown(error.to_string()))?;
        }
        if let Some(t) = runtime.reader_task.take() {
            t.abort();
        }
        if let Some(t) = runtime.dispatcher_task.take() {
            t.abort();
        }
        if let Some(t) = runtime.stderr_task.take() {
            t.abort();
        }
        drop(runtime);
        self.set_state(AcpLifecycle::Exited).await;
        Ok(())
    }

    async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> AcpResult<Value> {
        self.peer
            .request_with_timeout(method, params, timeout)
            .await
    }

    async fn flush_dispatcher(&self) {
        let (tx, rx) = oneshot::channel();
        let _ = tokio::time::timeout(DISPATCHER_FENCE_TIMEOUT, async {
            if self
                .inbound_tx
                .send(InboundMessage::Fence(tx))
                .await
                .is_ok()
            {
                let _ = rx.await;
            }
        })
        .await;
    }

    /// Flush all already-read inbound notifications through the dispatcher and
    /// the target session's route forwarder (`None` targets the default route).
    ///
    /// Callers with bounded event channels should drain their `events` receiver
    /// concurrently (for example inside `tokio::select!`) while awaiting this
    /// barrier so the route forwarder cannot stall on a full receiver.
    pub async fn flush_session_events(&self, session_id: Option<&str>) {
        self.flush_dispatcher().await;
        let route = match session_id {
            Some(id) => {
                let table = self.route_table.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(route) = table.routes.get(id) {
                    Some(Arc::clone(route))
                } else if table.routed_mode {
                    None
                } else {
                    Some(Arc::clone(&self.default_route))
                }
            }
            None => Some(Arc::clone(&self.default_route)),
        };
        if let Some(route) = route {
            route.flush_barrier(DISPATCHER_FENCE_TIMEOUT).await;
        }
    }

    async fn start_dispatcher(&self) {
        let Some(mut inbound) = self.inbound.lock().await.take() else {
            return;
        };
        let peer = self.peer.clone();
        let default_route = Arc::clone(&self.default_route);
        let route_table = Arc::clone(&self.route_table);
        let alive = Arc::clone(&self.alive);
        let shutting_down = Arc::clone(&self.shutting_down);
        let exit_tx = self.exit_tx.clone();
        let task = tokio::spawn(async move {
            while let Some(item) = inbound.recv().await {
                let msg = match item {
                    InboundMessage::JsonRpc(msg) => msg,
                    InboundMessage::Fence(ack) => {
                        let _ = ack.send(());
                        continue;
                    }
                    InboundMessage::Closed => break,
                };
                match msg {
                    JsonRpcMessage::Notification { method, params }
                        if method == "session/update" =>
                    {
                        if let Some(route) =
                            resolve_dispatch_route(&params, &route_table, &default_route, true)
                        {
                            dispatch_update(&route, params);
                        }
                    }
                    JsonRpcMessage::Notification { method, params }
                        if method.starts_with("cursor/") =>
                    {
                        if let Some(route) =
                            resolve_dispatch_route(&params, &route_table, &default_route, true)
                        {
                            route.send_nonblocking(AcpEvent::ExtensionNotification {
                                method,
                                params,
                            });
                        }
                    }
                    JsonRpcMessage::Notification { .. } => {}
                    JsonRpcMessage::Request { id, method, params }
                        if method == "session/request_permission" =>
                    {
                        let Some(route) =
                            resolve_dispatch_route(&params, &route_table, &default_route, false)
                        else {
                            let _ = peer.respond(id, cancelled_permission()).await;
                            continue;
                        };
                        let options: Vec<PermissionOption> = serde_json::from_value(
                            params
                                .get("options")
                                .cloned()
                                .unwrap_or(Value::Array(vec![])),
                        )
                        .unwrap_or_default();
                        let peer = peer.clone();
                        let host = route.permission_host.clone();
                        let policy = *route.permission_policy.lock().await;
                        let pending = Arc::clone(&route.pending_permissions);
                        let (cancel_tx, cancel_rx) = oneshot::channel();
                        pending
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(id.clone(), cancel_tx);
                        route.send_nonblocking(AcpEvent::PermissionRequest(params.clone()));
                        tokio::spawn(async move {
                            let decision = async {
                                match policy {
                                    PermissionPolicy::Manual => match &host {
                                        Some(h) => h.request_permission(&options).await,
                                        None => Err(AcpError::PermissionFailed(
                                            "manual permission has no host".into(),
                                        )),
                                    },
                                    _ => policy.choose(&options),
                                }
                            };
                            tokio::pin!(decision);
                            let (outcome, was_cancelled) = tokio::select! {
                                biased;
                                _ = cancel_rx => (Ok(PermissionDecision::Rejected), true),
                                decision = &mut decision => (decision, false),
                            };
                            let was_pending = pending
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .remove(&id)
                                .is_some();
                            if was_cancelled || !was_pending {
                                let _ = peer.respond(id, cancelled_permission()).await;
                                return;
                            }
                            match outcome {
                                Ok(PermissionDecision::Selected { option_id }) => {
                                    let _ = peer
                                        .respond(
                                            id,
                                            json!({"outcome": {
                                                "outcome": "selected", "optionId": option_id
                                            }}),
                                        )
                                        .await;
                                }
                                Ok(PermissionDecision::Rejected) => {
                                    let _ = peer.respond(id, cancelled_permission()).await;
                                }
                                Err(e) => {
                                    let _ =
                                        peer.respond_error(Some(id), -32001, e.to_string()).await;
                                }
                            }
                        });
                    }
                    JsonRpcMessage::Request { id, method, .. }
                        if method == "cursor/ask_question" || method == "cursor/create_plan" =>
                    {
                        let _ = peer
                            .respond_error(
                                Some(id),
                                -32601,
                                format!("unsupported Cursor extension request: {method}"),
                            )
                            .await;
                    }
                    JsonRpcMessage::Request { id, method, .. } => {
                        let _ = peer
                            .respond_error(Some(id), -32601, format!("unknown method: {method}"))
                            .await;
                    }
                    JsonRpcMessage::Error { error, .. } => {
                        let (routes, routed_mode) = {
                            let table = route_table.lock().unwrap_or_else(|e| e.into_inner());
                            (
                                table.routes.values().cloned().collect::<Vec<_>>(),
                                table.routed_mode,
                            )
                        };
                        if routes.is_empty() {
                            if !routed_mode {
                                default_route.send_nonblocking(AcpEvent::Error(error.message));
                            }
                        } else {
                            for route in routes {
                                route.send_nonblocking(AcpEvent::Error(error.message.clone()));
                            }
                        }
                    }
                    JsonRpcMessage::Response { .. } => {}
                }
            }
            // Drop the receiver before cleanup so any concurrent or future
            // `flush_dispatcher()` sees a closed channel immediately instead of
            // waiting for `DISPATCHER_FENCE_TIMEOUT`.
            drop(inbound);
            if !shutting_down.load(Ordering::SeqCst) {
                alive.store(false, Ordering::SeqCst);
                let _ = exit_tx.send(true);
                let routes = {
                    let table = route_table.lock().unwrap_or_else(|e| e.into_inner());
                    let mut routes = table.routes.values().cloned().collect::<Vec<_>>();
                    if let Some(pending) = table.pending.clone() {
                        routes.push(pending);
                    }
                    routes
                };
                cancel_pending_permissions(&default_route.pending_permissions);
                for route in routes {
                    cancel_pending_permissions(&route.pending_permissions);
                }
            }
        });
        self.runtime.lock().await.dispatcher_task = Some(task);
    }
}

fn resolve_dispatch_route(
    params: &Value,
    route_table: &StdMutex<RouteTable>,
    default_route: &Arc<SessionRoute>,
    allow_pending: bool,
) -> Option<Arc<SessionRoute>> {
    let session_id = params
        .get("sessionId")
        .or_else(|| params.get("session_id"))
        .and_then(Value::as_str);
    let table = route_table.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(session_id) = session_id {
        if let Some(route) = table.routes.get(session_id) {
            return Some(Arc::clone(route));
        }
        if table.unregistered_sessions.contains(session_id) {
            return None;
        }
        if allow_pending && let Some(pending) = &table.pending {
            return Some(Arc::clone(pending));
        }
        if table.routed_mode || !table.routes.is_empty() {
            return None;
        }
        return Some(Arc::clone(default_route));
    }
    if allow_pending && let Some(pending) = &table.pending {
        return Some(Arc::clone(pending));
    }
    if table.routed_mode || !table.routes.is_empty() {
        return None;
    }
    Some(Arc::clone(default_route))
}

fn parse_remote_sessions(response: &Value) -> Vec<RemoteSessionInfo> {
    response
        .get("sessions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let session_id = item
                        .get("sessionId")
                        .or_else(|| item.get("session_id"))
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)?
                        .to_string();
                    let cwd = item
                        .get("cwd")
                        .and_then(Value::as_str)
                        .map(PathBuf::from)
                        .unwrap_or_default();
                    let title = item
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    let updated_at = item
                        .get("updatedAt")
                        .or_else(|| item.get("updated_at"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    Some(RemoteSessionInfo {
                        session_id,
                        cwd,
                        title,
                        updated_at,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn terminate_child(child: &mut Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let status = tokio::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if status.is_ok_and(|status| status.success()) {
            let _ = child.wait().await;
            return;
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn serialize_params<T: Serialize>(params: T) -> AcpResult<Value> {
    serde_json::to_value(params).map_err(|e| AcpError::ProtocolDecode(e.to_string()))
}

fn cancelled_permission() -> Value {
    json!({ "outcome": { "outcome": "cancelled" } })
}

fn cancel_pending_permissions(
    pending: &StdMutex<HashMap<jsonrpc::JsonRpcId, oneshot::Sender<()>>>,
) {
    let senders = pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain()
        .map(|(_, tx)| tx)
        .collect::<Vec<_>>();
    for sender in senders {
        let _ = sender.send(());
    }
}

fn dispatch_update(route: &SessionRoute, params: Value) {
    let update = params.get("update").unwrap_or(&params);
    if let Some(text) = update_text(update).or_else(|| {
        params
            .get("text")
            .or_else(|| params.get("chunk"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }) {
        route.send_nonblocking(AcpEvent::AgentMessageChunk(text));
        return;
    }
    if update.get("sessionUpdate").and_then(Value::as_str) == Some("tool_call")
        || params.get("toolCall").is_some()
    {
        route.send_nonblocking(AcpEvent::ToolCall(params));
        return;
    }
    if update.get("sessionUpdate").and_then(Value::as_str) == Some("tool_call_update")
        || params.get("toolCallUpdate").is_some()
    {
        route.send_nonblocking(AcpEvent::ToolCallUpdate(params));
        return;
    }
    route.send_nonblocking(AcpEvent::SessionUpdate(params));
}

struct PromptSlotGuard;

impl PromptSlotGuard {
    fn try_acquire(slot: &AtomicBool, active_prompts: &AtomicUsize) -> AcpResult<Self> {
        if slot.swap(true, Ordering::SeqCst) {
            Err(AcpError::PromptAlreadyRunning)
        } else {
            active_prompts.fetch_add(1, Ordering::SeqCst);
            Ok(Self)
        }
    }
}

impl<W: AsyncWrite + Unpin + Send + 'static> AcpClient<W> {
    fn prompt_cancel_action(
        &self,
        session_id: String,
        route: Arc<SessionRoute>,
        mut sent: watch::Receiver<Option<bool>>,
    ) -> impl FnOnce() + Send + 'static {
        let peer = self.peer.clone();
        move || {
            tokio::spawn(async move {
                while sent.borrow().is_none() && sent.changed().await.is_ok() {}
                if *sent.borrow() == Some(true) {
                    let _ = peer
                        .notify("session/cancel", json!({ "sessionId": session_id }))
                        .await;
                }
                cancel_pending_permissions(&route.pending_permissions);
                route.send_nonblocking(AcpEvent::Cancelled);
            });
        }
    }
}

struct CancelOnDrop(Option<Box<dyn FnOnce() + Send>>);

impl CancelOnDrop {
    fn new(action: impl FnOnce() + Send + 'static) -> Self {
        Self(Some(Box::new(action)))
    }

    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(action) = self.0.take() {
            action();
        }
    }
}

fn update_text(update: &Value) -> Option<String> {
    if update.get("sessionUpdate").and_then(Value::as_str) != Some("agent_message_chunk") {
        return None;
    }
    update
        .pointer("/content/content/text")
        .or_else(|| update.pointer("/content/text"))
        .or_else(|| update.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Maximum characters kept from an agent-supplied stop explanation.
const PROMPT_DETAIL_MAX_CHARS: usize = 400;

fn prompt_completion_from_result(result: &Value) -> PromptCompletion {
    let stop_reason = result
        .get("stopReason")
        .and_then(Value::as_str)
        .map(str::to_string);
    let detail = result
        .get("_meta")
        .and_then(|meta| {
            meta.get("codebuddy.ai/errorMessage")
                .or_else(|| meta.get("errorMessage"))
        })
        .or_else(|| result.get("errorMessage"))
        .and_then(detail_from_meta_value);
    PromptCompletion {
        stop_reason,
        detail,
    }
}

fn detail_from_meta_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            // Agents such as CodeBuddy stringify a JSON-RPC error object and
            // still report the turn as `refusal`. Prefer its message over the
            // raw envelope.
            if let Ok(parsed) = serde_json::from_str::<Value>(trimmed)
                && parsed.is_object()
            {
                return detail_from_error_payload(&parsed);
            }
            bounded_detail(trimmed)
        }
        Value::Object(_) => detail_from_error_payload(value),
        _ => None,
    }
}

fn detail_from_error_payload(value: &Value) -> Option<String> {
    value
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/error/message").and_then(Value::as_str))
        .and_then(bounded_detail)
}

fn bounded_detail(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut end = text.len();
    for (count, (index, _)) in text.char_indices().enumerate() {
        if count == PROMPT_DETAIL_MAX_CHARS {
            end = index;
            break;
        }
    }
    let mut owned = text[..end].trim_end().to_string();
    if end < text.len() {
        owned.push('…');
    }
    Some(owned)
}

impl<W> Drop for AcpClient<W> {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = self.exit_tx.send(true);
        let runtime = self.runtime.get_mut();
        if let Some(t) = runtime.reader_task.take() {
            t.abort();
        }
        if let Some(t) = runtime.dispatcher_task.take() {
            t.abort();
        }
        if let Some(t) = runtime.stderr_task.take() {
            t.abort();
        }
        if let Some(child) = &mut runtime.child {
            let _ = child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_completion_reads_stringified_quota_error() {
        let message = "429 额度已用尽，请访问以下链接，购买加量包以获取更多额度：https://www.codebuddy.cn/profile/usage";
        let envelope = json!({
            "code": -32003,
            "message": message,
            "data": {"statusCode": 429, "code": 14018, "category": "quota"}
        });
        let completion = prompt_completion_from_result(&json!({
            "stopReason": "refusal",
            "_meta": {"codebuddy.ai/errorMessage": envelope.to_string()}
        }));
        assert_eq!(completion.stop_reason.as_deref(), Some("refusal"));
        assert_eq!(completion.detail.as_deref(), Some(message));
    }

    #[test]
    fn prompt_completion_reads_plain_error_message() {
        let completion = prompt_completion_from_result(&json!({
            "stopReason": "refusal",
            "errorMessage": "model quota exhausted"
        }));
        assert_eq!(completion.detail.as_deref(), Some("model quota exhausted"));
    }

    #[test]
    fn prompt_completion_ignores_empty_and_missing_detail() {
        assert!(
            prompt_completion_from_result(&json!({"stopReason": "end_turn"}))
                .detail
                .is_none()
        );
        assert!(
            prompt_completion_from_result(&json!({
                "stopReason": "refusal",
                "_meta": {"codebuddy.ai/errorMessage": "  "}
            }))
            .detail
            .is_none()
        );
    }
}
