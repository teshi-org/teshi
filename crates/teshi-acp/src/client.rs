use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use agent_client_protocol::schema::v1::{
    ListSessionsRequest, LoadSessionRequest, NewSessionRequest,
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
    jsonrpc::{self, JsonRpcMessage, JsonRpcPeer},
    permission::{PermissionDecision, PermissionHost, PermissionOption, PermissionPolicy},
};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(default)]
    pub protocol_version: Option<u32>,
    #[serde(default)]
    pub agent_info: Value,
    #[serde(default)]
    pub auth_methods: Vec<String>,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(default)]
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcpSession {
    pub id: String,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AcpEvent {
    AgentMessageChunk(String),
    ToolCall(Value),
    ToolCallUpdate(Value),
    PermissionRequest(Value),
    SessionUpdate(Value),
    ExtensionNotification { method: String, params: Value },
    Completed { stop_reason: Option<String> },
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

pub struct AcpClient<W> {
    peer: JsonRpcPeer<W>,
    inbound: Option<mpsc::Receiver<JsonRpcMessage>>,
    events: mpsc::Sender<AcpEvent>,
    lifecycle: Arc<Mutex<AcpLifecycle>>,
    config: AcpClientConfig,
    permission_policy: PermissionPolicy,
    permission_host: Option<Arc<dyn PermissionHost>>,
    initialized: Arc<Mutex<Option<InitializeResult>>>,
    prompt_running: Arc<AtomicBool>,
    pending_permissions: Arc<Mutex<HashMap<jsonrpc::JsonRpcId, oneshot::Sender<()>>>>,
    reader_task: Option<JoinHandle<()>>,
    stderr_task: Option<JoinHandle<()>>,
    child: Option<Child>,
}

impl AcpClient<tokio::process::ChildStdin> {
    pub async fn spawn(
        command: AcpAgentCommand,
        config: AcpClientConfig,
        events: mpsc::Sender<AcpEvent>,
        permission_policy: PermissionPolicy,
        permission_host: Option<Arc<dyn PermissionHost>>,
    ) -> AcpResult<Self> {
        let mut child =
            command
                .to_tokio_command()
                .spawn()
                .map_err(|source| AcpError::SpawnFailed {
                    program: command.program.display().to_string(),
                    source,
                })?;
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
        client.child = Some(child);
        if let Some(stderr) = stderr {
            client.stderr_task = Some(tokio::spawn(async move {
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
        let reader_task = tokio::spawn(jsonrpc::reader_loop(reader, peer.clone(), tx));
        Self {
            peer,
            inbound: Some(rx),
            events,
            lifecycle: Arc::new(Mutex::new(AcpLifecycle::Created)),
            config,
            permission_policy,
            permission_host,
            initialized: Arc::new(Mutex::new(None)),
            prompt_running: Arc::new(AtomicBool::new(false)),
            pending_permissions: Arc::new(Mutex::new(HashMap::new())),
            reader_task: Some(reader_task),
            stderr_task: None,
            child: None,
        }
    }

    pub async fn lifecycle(&self) -> AcpLifecycle {
        *self.lifecycle.lock().await
    }

    async fn set_state(&self, to: AcpLifecycle) {
        *self.lifecycle.lock().await = to;
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
        self.ensure_state(&[AcpLifecycle::Created], "initialize")
            .await?;
        self.set_state(AcpLifecycle::Starting).await;
        self.start_dispatcher();
        let params = json!({
            "protocolVersion": self.config.protocol_version,
            "clientCapabilities": { "filesystem": false, "terminal": false },
            "clientInfo": { "name": self.config.client_info.name, "version": self.config.client_info.version }
        });
        let result = self
            .request_with_timeout("initialize", params, self.config.initialize_timeout)
            .await
            .map_err(|e| AcpError::InitializeFailed(e.to_string()))?;
        let protocol_version = result
            .get("protocolVersion")
            .and_then(Value::as_u64)
            .map(|v| v as u32);
        if let Some(v) = protocol_version
            && v != self.config.protocol_version
        {
            self.set_state(AcpLifecycle::Failed).await;
            return Err(AcpError::UnsupportedProtocolVersion(v));
        }
        let auth_methods = result
            .get("authMethods")
            .or_else(|| result.pointer("/authentication/methods"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| {
                        v.as_str()
                            .or_else(|| v.get("id").and_then(Value::as_str))
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let initialized = InitializeResult {
            protocol_version,
            agent_info: result.get("agentInfo").cloned().unwrap_or(Value::Null),
            auth_methods,
            capabilities: result
                .get("agentCapabilities")
                .or_else(|| result.get("capabilities"))
                .cloned()
                .unwrap_or(Value::Null),
            raw: result,
        };
        *self.initialized.lock().await = Some(initialized.clone());
        self.set_state(AcpLifecycle::Initialized).await;
        Ok(initialized)
    }

    pub async fn authenticate(&self, method_id: &str) -> AcpResult<()> {
        self.ensure_state(&[AcpLifecycle::Initialized], "authenticate")
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
                self.set_state(AcpLifecycle::Authenticated).await;
                Ok(())
            }
            Err(e) => Err(AcpError::AuthenticationFailed(e.to_string())),
        }
    }

    pub async fn new_session(&self, cwd: PathBuf) -> AcpResult<AcpSession> {
        self.ensure_state(
            &[
                AcpLifecycle::Initialized,
                AcpLifecycle::Authenticated,
                AcpLifecycle::Ready,
            ],
            "session/new",
        )
        .await?;
        let params = serialize_params(NewSessionRequest::new(cwd.clone()))?;
        let result = self
            .request_with_timeout("session/new", params, self.config.request_timeout)
            .await?;
        let id = result
            .get("sessionId")
            .or_else(|| result.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| AcpError::SessionFailed("session/new missing sessionId".into()))?
            .to_string();
        self.set_state(AcpLifecycle::Ready).await;
        Ok(AcpSession { id, cwd })
    }

    pub async fn list_sessions(&self) -> AcpResult<Value> {
        let init = self
            .initialized
            .lock()
            .await
            .clone()
            .ok_or_else(|| AcpError::InitializeFailed("missing initialize result".into()))?;
        if !init
            .capabilities
            .pointer("/sessionCapabilities/list")
            .is_some_and(Value::is_object)
        {
            return Err(AcpError::CapabilityUnsupported("session/list"));
        }
        self.request_with_timeout(
            "session/list",
            serialize_params(ListSessionsRequest::new())?,
            self.config.request_timeout,
        )
        .await
    }

    pub async fn load_session(&self, id: &str, cwd: PathBuf) -> AcpResult<AcpSession> {
        let init = self
            .initialized
            .lock()
            .await
            .clone()
            .ok_or_else(|| AcpError::InitializeFailed("missing initialize result".into()))?;
        if init
            .capabilities
            .get("loadSession")
            .and_then(Value::as_bool)
            != Some(true)
        {
            return Err(AcpError::CapabilityUnsupported("session/load"));
        }
        let params = serialize_params(LoadSessionRequest::new(id.to_string(), cwd.clone()))?;
        let result = self
            .request_with_timeout("session/load", params, self.config.request_timeout)
            .await?;
        let session_id = result
            .get("sessionId")
            .or_else(|| result.get("id"))
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        Ok(AcpSession {
            id: session_id,
            cwd,
        })
    }

    pub async fn prompt(&self, session: &AcpSession, prompt: &str) -> AcpResult<Option<String>> {
        self.ensure_state(&[AcpLifecycle::Ready], "session/prompt")
            .await?;
        PromptSlotGuard::try_acquire(&self.prompt_running)?;
        let peer = self.peer.clone();
        let slot = Arc::clone(&self.prompt_running);
        let params = json!({
            "sessionId": session.id,
            "prompt": [{ "type": "text", "text": prompt }]
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
            let _ = result_tx.send(result);
        });

        let cancel_action = self.prompt_cancel_action(session.id.clone(), sent_rx);
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
        let stop_reason = result
            .get("stopReason")
            .and_then(Value::as_str)
            .map(str::to_string);
        let _ = self
            .events
            .send(AcpEvent::Completed {
                stop_reason: stop_reason.clone(),
            })
            .await;
        Ok(stop_reason)
    }

    pub async fn cancel(&self, session: &AcpSession) -> AcpResult<()> {
        if !self.prompt_running.load(Ordering::SeqCst) {
            return Ok(());
        }
        let notify_result = self
            .peer
            .notify("session/cancel", json!({ "sessionId": session.id }))
            .await;
        cancel_pending_permissions(&self.pending_permissions).await;
        let _ = self.events.send(AcpEvent::Cancelled).await;
        notify_result
    }

    pub async fn shutdown(&mut self) -> AcpResult<()> {
        let state = self.lifecycle().await;
        if matches!(state, AcpLifecycle::ShuttingDown | AcpLifecycle::Exited) {
            return Ok(());
        }
        self.set_state(AcpLifecycle::ShuttingDown).await;
        self.peer.fail_all(AcpError::Cancelled).await;
        cancel_pending_permissions(&self.pending_permissions).await;
        let _ = self.peer.close().await;
        if let Some(child) = &mut self.child {
            match tokio::time::timeout(self.config.shutdown_timeout, child.wait()).await {
                Ok(Ok(_)) => {}
                _ => terminate_child(child).await,
            }
        }
        if let Some(t) = self.reader_task.take() {
            t.abort();
        }
        if let Some(t) = self.stderr_task.take() {
            t.abort();
        }
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

    fn start_dispatcher(&mut self) {
        let Some(mut inbound) = self.inbound.take() else {
            return;
        };
        let peer = self.peer.clone();
        let events = self.events.clone();
        let policy = self.permission_policy;
        let host = self.permission_host.clone();
        let pending_permissions = Arc::clone(&self.pending_permissions);
        tokio::spawn(async move {
            while let Some(msg) = inbound.recv().await {
                match msg {
                    JsonRpcMessage::Notification { method, params }
                        if method == "session/update" =>
                    {
                        dispatch_update(&events, params).await;
                    }
                    JsonRpcMessage::Notification { method, params }
                        if method.starts_with("cursor/") =>
                    {
                        let _ = events
                            .send(AcpEvent::ExtensionNotification { method, params })
                            .await;
                    }
                    JsonRpcMessage::Notification { .. } => {}
                    JsonRpcMessage::Request { id, method, params }
                        if method == "session/request_permission" =>
                    {
                        let options: Vec<PermissionOption> = serde_json::from_value(
                            params
                                .get("options")
                                .cloned()
                                .unwrap_or(Value::Array(vec![])),
                        )
                        .unwrap_or_default();
                        let peer = peer.clone();
                        let host = host.clone();
                        let pending = Arc::clone(&pending_permissions);
                        let (cancel_tx, cancel_rx) = oneshot::channel();
                        pending.lock().await.insert(id.clone(), cancel_tx);
                        let _ = events
                            .send(AcpEvent::PermissionRequest(params.clone()))
                            .await;
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
                            let was_pending = pending.lock().await.remove(&id).is_some();
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
                        let _ = events.send(AcpEvent::Error(error.message)).await;
                    }
                    JsonRpcMessage::Response { .. } => {}
                }
            }
        });
    }
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

async fn cancel_pending_permissions(
    pending: &Mutex<HashMap<jsonrpc::JsonRpcId, oneshot::Sender<()>>>,
) {
    let senders = pending
        .lock()
        .await
        .drain()
        .map(|(_, tx)| tx)
        .collect::<Vec<_>>();
    for sender in senders {
        let _ = sender.send(());
    }
}

async fn dispatch_update(events: &mpsc::Sender<AcpEvent>, params: Value) {
    let update = params.get("update").unwrap_or(&params);
    if let Some(text) = update_text(update).or_else(|| {
        params
            .get("text")
            .or_else(|| params.get("chunk"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }) {
        let _ = events.send(AcpEvent::AgentMessageChunk(text)).await;
        return;
    }
    if update.get("sessionUpdate").and_then(Value::as_str) == Some("tool_call")
        || params.get("toolCall").is_some()
    {
        let _ = events.send(AcpEvent::ToolCall(params)).await;
        return;
    }
    if update.get("sessionUpdate").and_then(Value::as_str) == Some("tool_call_update")
        || params.get("toolCallUpdate").is_some()
    {
        let _ = events.send(AcpEvent::ToolCallUpdate(params)).await;
        return;
    }
    let _ = events.send(AcpEvent::SessionUpdate(params)).await;
}

struct PromptSlotGuard;

impl PromptSlotGuard {
    fn try_acquire(slot: &AtomicBool) -> AcpResult<Self> {
        if slot.swap(true, Ordering::SeqCst) {
            Err(AcpError::PromptAlreadyRunning)
        } else {
            Ok(Self)
        }
    }
}

impl<W: AsyncWrite + Unpin + Send + 'static> AcpClient<W> {
    fn prompt_cancel_action(
        &self,
        session_id: String,
        mut sent: watch::Receiver<Option<bool>>,
    ) -> impl FnOnce() + Send + 'static {
        let peer = self.peer.clone();
        let pending = Arc::clone(&self.pending_permissions);
        let events = self.events.clone();
        move || {
            tokio::spawn(async move {
                while sent.borrow().is_none() && sent.changed().await.is_ok() {}
                if *sent.borrow() == Some(true) {
                    let _ = peer
                        .notify("session/cancel", json!({ "sessionId": session_id }))
                        .await;
                }
                cancel_pending_permissions(&pending).await;
                let _ = events.send(AcpEvent::Cancelled).await;
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

impl<W> Drop for AcpClient<W> {
    fn drop(&mut self) {
        if let Some(t) = self.reader_task.take() {
            t.abort();
        }
        if let Some(t) = self.stderr_task.take() {
            t.abort();
        }
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
    }
}
