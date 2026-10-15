//! Synchronous frontend adapter for the asynchronous teshi-acp client.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

use teshi_acp::{
    AcpClient, AcpEvent,
    client::AcpClientConfig,
    command::AcpAgentCommand,
    permission::{PermissionDecision, PermissionHost, PermissionPolicy},
};
use teshi_agent::backend::{AgentBackendEvent, AgentBackendStatus, PermissionChoice};
use teshi_core::llm::{ChatMessage, ContentBlock, MessageContent};
use tokio::sync::{mpsc as async_mpsc, oneshot};

use crate::{AiChatMessage, AiRole};

#[derive(Debug, Clone)]
pub struct AcpBackendConfig {
    pub command: AcpAgentCommand,
    pub permission_policy: PermissionPolicy,
    /// An Agent-advertised ACP authentication method, if the caller selected one.
    pub auth_method: Option<String>,
    pub client: AcpClientConfig,
    /// Interactive permission host for `PermissionPolicy::Manual`. Without a
    /// host every `session/request_permission` fails immediately and tool-using
    /// agents abort their turn after the first streamed chunk.
    pub permission_host: Option<Arc<ChannelPermissionHost>>,
}

impl AcpBackendConfig {
    pub fn new(command: AcpAgentCommand) -> Self {
        Self {
            command,
            permission_policy: PermissionPolicy::Manual,
            auth_method: None,
            client: AcpClientConfig::default(),
            permission_host: None,
        }
    }
}

enum WorkerCommand {
    Prompt(String),
    Configure {
        option: Box<teshi_acp::session_config::ConfigOption>,
        value: serde_json::Value,
        revision: u64,
    },
    Policy(PermissionPolicy),
    Cancel,
    Shutdown,
}

/// Confirmed settings and asynchronous operation state for one backend instance.
#[derive(Debug, Clone, Default)]
pub struct AcpControls {
    /// Remote session identity, absent while connecting.
    pub session_id: Option<String>,
    /// Last confirmed settings.
    pub config: teshi_acp::session_config::SessionConfig,
    /// Whether configuration has been discovered.
    pub ready: bool,
    /// One configuration mutation is awaiting confirmation.
    pub pending: bool,
    /// Recoverable settings error, or reconciliation requirement.
    pub error: Option<String>,
    /// Monotonic snapshot revision for stale picker rejection.
    pub revision: u64,
    /// Effective client approval policy.
    pub policy: Option<PermissionPolicy>,
}

/// Bridges an ACP agent's `session/request_permission` to a synchronous
/// frontend. The async [`PermissionHost`] side is driven by the teshi-acp
/// dispatcher inside the worker runtime; the frontend polls
/// [`ChannelPermissionHost::try_recv_request`] and answers with
/// [`ChannelPermissionHost::resolve`].
pub struct ChannelPermissionHost {
    pending: Mutex<HashMap<u64, oneshot::Sender<PermissionDecision>>>,
    /// `Sender` is not `Sync`; the mutex keeps the host shareable as
    /// `Arc<dyn PermissionHost>`.
    requests_tx: Mutex<mpsc::Sender<(u64, Vec<PermissionChoice>)>>,
    requests_rx: Mutex<mpsc::Receiver<(u64, Vec<PermissionChoice>)>>,
    next_id: AtomicU64,
}

impl std::fmt::Debug for ChannelPermissionHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelPermissionHost")
            .field(
                "pending",
                &self.pending.lock().map(|p| p.len()).unwrap_or(0),
            )
            .finish_non_exhaustive()
    }
}

impl ChannelPermissionHost {
    pub fn new() -> Arc<Self> {
        let (tx, rx) = mpsc::channel();
        Arc::new(Self {
            pending: Mutex::new(HashMap::new()),
            requests_tx: Mutex::new(tx),
            requests_rx: Mutex::new(rx),
            next_id: AtomicU64::new(1),
        })
    }

    /// Next pending `(request_id, options)` for the frontend to render.
    pub fn try_recv_request(&self) -> Option<(u64, Vec<PermissionChoice>)> {
        self.requests_rx.lock().ok()?.try_recv().ok()
    }

    /// Answer a previously delivered request. `None` rejects. Returns false
    /// when the request is unknown or already resolved.
    pub fn resolve(&self, request_id: u64, option_id: Option<String>) -> bool {
        let decision = match option_id {
            Some(option_id) => PermissionDecision::Selected { option_id },
            None => PermissionDecision::Rejected,
        };
        self.pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&request_id))
            .is_some_and(|tx| tx.send(decision).is_ok())
    }
}

impl PermissionHost for ChannelPermissionHost {
    fn request_permission<'a>(
        &'a self,
        options: &'a [teshi_acp::PermissionOption],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = teshi_acp::AcpResult<PermissionDecision>> + Send + 'a>,
    > {
        Box::pin(async move {
            let request_id = self.next_id.fetch_add(1, Ordering::SeqCst);
            let (tx, rx) = oneshot::channel();
            {
                let mut pending = self.pending.lock().map_err(|_| {
                    teshi_acp::AcpError::PermissionFailed("permission lock poisoned".into())
                })?;
                pending.insert(request_id, tx);
            }
            let choices = options
                .iter()
                .map(|option| PermissionChoice {
                    id: option.id.clone(),
                    label: option.label.clone(),
                    kind: option.kind.clone(),
                })
                .collect();
            self.requests_tx
                .lock()
                .map_err(|_| {
                    teshi_acp::AcpError::PermissionFailed("permission lock poisoned".into())
                })?
                .send((request_id, choices))
                .map_err(|_| {
                    teshi_acp::AcpError::PermissionFailed("permission channel closed".into())
                })?;
            match rx.await {
                Ok(decision) => Ok(decision),
                Err(_) => {
                    if let Ok(mut pending) = self.pending.lock() {
                        pending.remove(&request_id);
                    }
                    Err(teshi_acp::AcpError::PermissionFailed(
                        "permission request was cancelled".into(),
                    ))
                }
            }
        })
    }
}

pub struct AcpAgentBackend {
    controls: Arc<Mutex<AcpControls>>,
    pub messages: Vec<AiChatMessage>,
    pub partial_response: String,
    config: Option<AcpBackendConfig>,
    commands: Option<async_mpsc::UnboundedSender<WorkerCommand>>,
    events: Option<mpsc::Receiver<AgentBackendEvent>>,
    worker: Option<thread::JoinHandle<()>>,
    state: AgentBackendStatus,
    /// Shared with the ACP client inside the worker; delivers and answers
    /// `session/request_permission` prompts while the turn stays parked.
    permission_host: Option<Arc<ChannelPermissionHost>>,
}

impl std::fmt::Debug for AcpAgentBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpAgentBackend")
            .field("state", &self.state)
            .field("configured", &self.config.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for AcpAgentBackend {
    fn default() -> Self {
        Self {
            controls: Arc::new(Mutex::new(AcpControls::default())),
            messages: Vec::new(),
            partial_response: String::new(),
            config: None,
            commands: None,
            events: None,
            worker: None,
            state: AgentBackendStatus::Unavailable,
            permission_host: None,
        }
    }
}

impl AcpAgentBackend {
    pub fn configure(&mut self, config: AcpBackendConfig) {
        self.stop();
        self.controls = Arc::new(Mutex::new(AcpControls {
            policy: Some(config.permission_policy),
            ..Default::default()
        }));
        self.permission_host = config.permission_host.clone();
        self.config = Some(config);
        self.state = AgentBackendStatus::Idle;
    }

    pub fn status(&self) -> AgentBackendStatus {
        self.state
    }

    pub fn is_connected(&self) -> bool {
        self.commands.is_some()
    }

    /// Connect and discover settings without starting a prompt.
    ///
    /// # Errors
    /// Returns an error if no command is configured or the worker cannot start.
    pub fn connect(&mut self) -> anyhow::Result<()> {
        let config = self.config.clone().ok_or_else(|| {
            anyhow::anyhow!("ACP backend is not configured; supply an AcpAgentCommand")
        })?;
        if matches!(
            self.state,
            AgentBackendStatus::Failed | AgentBackendStatus::Cancelled
        ) || self
            .controls
            .lock()
            .is_ok_and(|s| !s.ready && s.error.is_some())
            || self
                .worker
                .as_ref()
                .is_some_and(thread::JoinHandle::is_finished)
        {
            self.stop();
        }
        if self.worker.is_none() {
            let revision = self.controls().revision.saturating_add(1);
            self.controls = Arc::new(Mutex::new(AcpControls {
                policy: Some(config.permission_policy),
                revision,
                ..Default::default()
            }));
            let controls = Arc::clone(&self.controls);
            let (commands_tx, commands_rx) = async_mpsc::unbounded_channel();
            let (events_tx, events_rx) = mpsc::channel();
            let worker = thread::Builder::new()
                .name("teshi-acp-backend".into())
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build();
                    match runtime {
                        Ok(runtime) => {
                            runtime.block_on(run_worker(config, commands_rx, events_tx, controls))
                        }
                        Err(error) => {
                            let _ = events_tx.send(AgentBackendEvent::Failed(format!(
                                "ACP runtime initialization failed: {error}"
                            )));
                        }
                    }
                })?;
            self.commands = Some(commands_tx);
            self.events = Some(events_rx);
            self.worker = Some(worker);
            self.state = AgentBackendStatus::Idle;
        }
        Ok(())
    }

    /// Start a prompt using the discovered session.
    ///
    /// # Errors
    /// Fails when connecting fails or a setting change is pending.
    pub fn start(&mut self) -> anyhow::Result<()> {
        if self.controls().pending {
            anyhow::bail!("ACP settings change is pending");
        }
        self.connect()?;
        self.partial_response.clear();
        self.state = AgentBackendStatus::Running;
        Ok(())
    }

    /// Read a snapshot without exposing transport state to the frontend.
    pub fn controls(&self) -> AcpControls {
        self.controls
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default()
    }

    /// Queue a validated setting change for this conversation.
    ///
    /// # Errors
    /// Rejects stale selections, busy sessions, and unavailable workers.
    pub fn set_option(
        &mut self,
        id: &str,
        value: serde_json::Value,
        revision: u64,
    ) -> anyhow::Result<()> {
        if matches!(
            self.state,
            AgentBackendStatus::Running | AgentBackendStatus::WaitingForPermission
        ) {
            anyhow::bail!("Wait for the current turn to finish");
        }
        let mut state = self
            .controls
            .lock()
            .map_err(|_| anyhow::anyhow!("ACP settings lock poisoned"))?;
        if !state.ready || state.pending || state.revision != revision {
            anyhow::bail!("ACP settings changed; reopen the picker");
        }
        let option = state
            .config
            .options
            .iter()
            .find(|o| o.id == id && o.accepts(&value))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("ACP setting is no longer available"))?;
        self.commands
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ACP worker unavailable"))?
            .send(WorkerCommand::Configure {
                option: Box::new(option),
                value,
                revision,
            })
            .map_err(|_| anyhow::anyhow!("ACP worker exited"))?;
        state.pending = true;
        state.error = None;
        Ok(())
    }

    /// Update future permission requests without resolving any pending prompt.
    ///
    /// # Errors
    /// Returns an error if the connected worker has exited.
    pub fn set_policy(&mut self, policy: PermissionPolicy) -> anyhow::Result<()> {
        if let Some(config) = &mut self.config {
            config.permission_policy = policy;
        }
        if let Some(commands) = &self.commands {
            commands
                .send(WorkerCommand::Policy(policy))
                .map_err(|_| anyhow::anyhow!("ACP worker exited"))?;
        } else if let Ok(mut state) = self.controls.lock() {
            state.policy = Some(policy);
        }
        Ok(())
    }

    pub fn submit_turn(
        &mut self,
        system: Option<String>,
        messages: &[ChatMessage],
    ) -> anyhow::Result<()> {
        if self.state != AgentBackendStatus::Running {
            anyhow::bail!("ACP backend has no active turn");
        }
        let prompt = prompt_text(system.as_deref(), messages);
        self.commands
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ACP worker is unavailable"))?
            .send(WorkerCommand::Prompt(prompt))
            .map_err(|_| anyhow::anyhow!("ACP worker exited"))
    }

    pub fn cancel(&mut self) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(WorkerCommand::Cancel);
        }
        self.state = AgentBackendStatus::Cancelled;
        self.partial_response.clear();
    }

    pub fn try_next_event(&mut self) -> Result<Option<AgentBackendEvent>, mpsc::TryRecvError> {
        // Permission prompts are delivered ahead of the turn's own events so a
        // frontend can answer while the worker stays parked on the turn future.
        if let Some(host) = &self.permission_host
            && let Some((request_id, options)) = host.try_recv_request()
        {
            self.state = AgentBackendStatus::WaitingForPermission;
            return Ok(Some(AgentBackendEvent::PermissionRequest {
                request_id,
                options,
            }));
        }
        let Some(events) = &self.events else {
            return Ok(None);
        };
        match events.try_recv() {
            Ok(event) => {
                match &event {
                    AgentBackendEvent::MessageChunk { content } => {
                        self.partial_response.push_str(content)
                    }
                    AgentBackendEvent::Completed { .. } => {
                        let content = std::mem::take(&mut self.partial_response);
                        if !content.is_empty() {
                            self.messages.push(AiChatMessage {
                                role: AiRole::Assistant,
                                content,
                                tool_calls: None,
                                tool_call_id: None,
                                reasoning_content: None,
                                source: None,
                            });
                        }
                        self.state = AgentBackendStatus::Completed;
                    }
                    AgentBackendEvent::Failed(_) => {
                        self.partial_response.clear();
                        self.state = AgentBackendStatus::Failed;
                    }
                    _ => {}
                }
                Ok(Some(event))
            }
            Err(mpsc::TryRecvError::Disconnected) if self.state != AgentBackendStatus::Running => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn stop(&mut self) {
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(WorkerCommand::Shutdown);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.events = None;
    }

    /// Answer a pending ACP permission prompt. `None` rejects the request.
    /// Returns false when the prompt is unknown or already answered.
    pub fn respond_permission(&mut self, request_id: u64, option_id: Option<String>) -> bool {
        let resolved = self
            .permission_host
            .as_ref()
            .is_some_and(|host| host.resolve(request_id, option_id));
        if resolved && self.state == AgentBackendStatus::WaitingForPermission {
            self.state = AgentBackendStatus::Running;
        }
        resolved
    }
}

impl Drop for AcpAgentBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

fn prompt_text(system: Option<&str>, messages: &[ChatMessage]) -> String {
    let mut parts = Vec::new();
    if let Some(system) = system.filter(|s| !s.is_empty()) {
        parts.push(format!("Teshi task instructions:\n{system}"));
    }
    for message in messages.iter().rev().take(1) {
        let content = match &message.content {
            MessageContent::Text(text) => text.clone(),
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::Image { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        };
        parts.push(format!("{}: {content}", message.role));
    }
    parts.join("\n\n")
}

async fn run_worker(
    config: AcpBackendConfig,
    mut commands: async_mpsc::UnboundedReceiver<WorkerCommand>,
    output: mpsc::Sender<AgentBackendEvent>,
    controls: Arc<Mutex<AcpControls>>,
) {
    let (updates_tx, mut updates_rx) = async_mpsc::channel(128);
    let cwd: PathBuf = config.command.cwd.clone();
    let auth_method = config.auth_method.clone();
    let permission_host: Option<Arc<dyn PermissionHost>> = config
        .permission_host
        .clone()
        .map(|host| host as Arc<dyn PermissionHost>);
    let mut client = match AcpClient::spawn(
        config.command,
        config.client,
        updates_tx,
        config.permission_policy,
        permission_host,
    )
    .await
    {
        Ok(client) => client,
        Err(error) => {
            if let Ok(mut state) = controls.lock() {
                state.error = Some(format!("ACP process launch failed: {error}"));
            }
            let _ = output.send(AgentBackendEvent::Failed(format!(
                "ACP process launch failed: {error}"
            )));
            return;
        }
    };
    let result = async {
        let initialized = client
            .initialize()
            .await
            .map_err(|e| format!("ACP initialize failed: {e}"))?;
        if let Some(method) = auth_method {
            client
                .authenticate(&method)
                .await
                .map_err(|e| format!("ACP authentication failed: {e}"))?;
        }
        let model = initialized
            .agent_info
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("ACP agent")
            .to_string();
        let session = client
            .new_session(cwd)
            .await
            .map_err(|e| format!("ACP session creation failed: {e}"))?;
        Ok::<_, String>((session, model))
    }
    .await;
    let (session, model) = match result {
        Ok(value) => value,
        Err(error) => {
            if let Ok(mut state) = controls.lock() {
                state.error = Some(error.clone());
            }
            let _ = output.send(AgentBackendEvent::Failed(error));
            let _ = client.shutdown().await;
            return;
        }
    };
    if let Ok(mut state) = controls.lock() {
        state.config = session.config.clone();
        state.session_id = Some(session.id.clone());
        state.ready = true;
        state.revision += 1;
    }
    loop {
        let command = tokio::select! {
            command = commands.recv() => match command { Some(command) => command, None => break },
            Some(update) = updates_rx.recv() => {
                apply_control_update(&update, &session.id, &controls);
                if forward_update(update, &output) { break; }
                continue;
            }
        };
        match command {
            WorkerCommand::Policy(policy) => {
                client.set_permission_policy(policy).await;
                if let Ok(mut state) = controls.lock() {
                    state.policy = Some(policy);
                }
            }
            WorkerCommand::Configure {
                option,
                value,
                revision,
            } => {
                let valid = controls
                    .lock()
                    .is_ok_and(|state| state.revision == revision && state.ready);
                let result = if valid {
                    client
                        .set_session_option(&session, &option, value.clone())
                        .await
                } else {
                    Err(teshi_acp::AcpError::ProtocolViolation(
                        "ACP settings changed; reopen the picker".into(),
                    ))
                };
                let uncertain = matches!(&result, Err(teshi_acp::AcpError::Timeout(_)));
                if let Ok(mut state) = controls.lock() {
                    state.pending = false;
                    match result {
                        Ok(response) => {
                            if response.get("configOptions").is_some() {
                                state.config.replace_modern(response.get("configOptions"));
                            } else if let Some(current) =
                                state.config.options.iter_mut().find(|o| o.id == option.id)
                            {
                                current.current = value;
                            }
                            state.error = None;
                            state.revision += 1;
                        }
                        Err(error) => {
                            state.error = Some(format!("ACP setting change failed: {error}"));
                        }
                    }
                    if uncertain {
                        state.ready = false;
                        state.error = Some(
                            "ACP setting timed out; reconnect to discover confirmed values".into(),
                        );
                    }
                }
                if uncertain {
                    break;
                }
            }
            WorkerCommand::Prompt(prompt) => {
                let mut turn = Box::pin(client.prompt(&session, &prompt));
                let mut cancelled = false;
                let mut protocol_failed = false;
                let mut turn_failed = false;
                loop {
                    tokio::select! {
                        result = &mut turn => {
                            while let Ok(update) = updates_rx.try_recv() {
                                apply_control_update(&update, &session.id, &controls);
                                protocol_failed |= forward_update(update, &output);
                            }
                            if !cancelled && !protocol_failed {
                                match result {
                                    Ok(reason) => { let _ = output.send(AgentBackendEvent::Completed {
                                        model: model.clone(), input_tokens: None, output_tokens: None, finish_reason: reason,
                                    }); }
                                    Err(error) => {
                                        turn_failed = true;
                                        let _ = output.send(AgentBackendEvent::Failed(format!("ACP prompt failed: {error}")));
                                    }
                                }
                            }
                            break;
                        }
                        Some(update) = updates_rx.recv() => {
                            apply_control_update(&update, &session.id, &controls);
                            protocol_failed |= forward_update(update, &output);
                        }
                        Some(command) = commands.recv() => match command {
                            WorkerCommand::Policy(policy) => {
                                client.set_permission_policy(policy).await;
                                if let Ok(mut state) = controls.lock() { state.policy = Some(policy); }
                            }
                            WorkerCommand::Configure { .. } => {
                                if let Ok(mut state) = controls.lock() { state.pending = false; state.error = Some("Wait for the current turn to finish".into()); }
                            }
                            WorkerCommand::Cancel | WorkerCommand::Shutdown => {
                                cancelled = true;
                                drop(turn);
                                let _ = client.cancel(&session).await;
                                break;
                            }
                            WorkerCommand::Prompt(_) => { let _ = output.send(AgentBackendEvent::Failed("ACP prompt already running".into())); }
                        }
                    }
                }
                if cancelled || protocol_failed || turn_failed {
                    break;
                }
            }
            WorkerCommand::Cancel | WorkerCommand::Shutdown => break,
        }
    }
    if let Ok(mut state) = controls.lock() {
        state.ready = false;
        state.pending = false;
    }
    let _ = client.shutdown().await;
}

fn apply_control_update(update: &AcpEvent, session: &str, controls: &Mutex<AcpControls>) {
    if let AcpEvent::SessionUpdate(params) = update
        && let Ok(mut state) = controls.lock()
        && state.config.apply_update(session, params)
    {
        state.revision += 1;
    }
}

fn forward_update(update: AcpEvent, output: &mpsc::Sender<AgentBackendEvent>) -> bool {
    let failed = matches!(&update, AcpEvent::Error(_));
    let event = match update {
        AcpEvent::AgentMessageChunk(content) => Some(AgentBackendEvent::MessageChunk { content }),
        AcpEvent::ToolCall(value) | AcpEvent::ToolCallUpdate(value) => {
            let update = value.get("update").unwrap_or(&value);
            let title = update
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("ACP agent tool")
                .to_string();
            let status = update
                .get("status")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            Some(AgentBackendEvent::ExternalToolActivity { title, status })
        }
        AcpEvent::Error(error) => Some(AgentBackendEvent::Failed(format!(
            "ACP protocol error: {error}"
        ))),
        _ => None,
    };
    if let Some(event) = event {
        let _ = output.send(event);
    }
    failed
}
