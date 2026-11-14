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

use base64::Engine as _;
use teshi_acp::{
    AcpClient, AcpEvent, PromptBlock,
    client::AcpClientConfig,
    command::AcpAgentCommand,
    permission::{PermissionDecision, PermissionHost, PermissionPolicy},
};
use teshi_agent::backend::{AgentBackendEvent, AgentBackendStatus, PermissionChoice};
use teshi_core::llm::{ChatMessage, ContentBlock, ImageSource, MessageContent};
use tokio::sync::{mpsc as async_mpsc, oneshot};

use crate::acp_retry::{self, TurnRetryPolicy};
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
    /// Optional per-agent file for the last successfully selected model.
    pub model_preference_path: Option<PathBuf>,
    /// Optional per-agent file for the other confirmed session options (mode,
    /// thinking level, agent-advertised approval), keyed by option id.
    pub option_preferences_path: Option<PathBuf>,
    /// Optional live-host MCP transport supplied by an interactive frontend.
    pub tool_bridge: Option<crate::tool_bridge::ToolBridgeConfig>,
    /// Automatic retries for turns that a transient network failure ended.
    pub turn_retry: TurnRetryPolicy,
}

impl AcpBackendConfig {
    pub fn new(command: AcpAgentCommand) -> Self {
        Self {
            command,
            permission_policy: PermissionPolicy::Manual,
            auth_method: None,
            client: AcpClientConfig::default(),
            permission_host: None,
            model_preference_path: None,
            option_preferences_path: None,
            tool_bridge: None,
            turn_retry: TurnRetryPolicy::default(),
        }
    }
}

/// How long the worker keeps retrying ACP process-container shutdown.
///
/// An unresponsive agent must not keep the frontend's worker join alive
/// forever: that join is the last blocking step of an exit, and retrying
/// without a bound is what makes quitting look like a hang. Dropping the
/// client after the deadline still closes the job object, and
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` reaps the container.
const SHUTDOWN_RETRY_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

/// How long a cancelled turn waits for the agent to acknowledge `session/cancel`.
///
/// Cancelling ends the turn, not the conversation: the worker keeps the ACP
/// process and session alive. An agent that ignores the notification must not
/// park the worker forever, so the budget turns that case into a real
/// teardown; the next turn then reconnects and re-seeds the saved transcript.
const CANCEL_ACK_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);

enum WorkerCommand {
    Prompt(Vec<PromptBlock>),
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
    /// Whether the agent advertised `promptCapabilities.image`; only
    /// meaningful once `ready` is set.
    pub image_prompts: bool,
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

    /// Drop prompts queued by a turn that was cancelled. The ACP client already
    /// answers those requests as `cancelled`, so a stopped turn must not raise a
    /// dialog that no longer belongs to any turn.
    pub fn discard_requests(&self) {
        if let Ok(requests) = self.requests_rx.lock() {
            while requests.try_recv().is_ok() {}
        }
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
    tool_bridge: Option<crate::tool_bridge::ToolBridge>,
    controls: Arc<Mutex<AcpControls>>,
    pub messages: Vec<AiChatMessage>,
    pub partial_response: String,
    config: Option<AcpBackendConfig>,
    commands: Option<async_mpsc::UnboundedSender<WorkerCommand>>,
    events: Option<mpsc::Receiver<AgentBackendEvent>>,
    worker: Option<thread::JoinHandle<()>>,
    state: AgentBackendStatus,
    submitted_prompt: bool,
    shutting_down: bool,
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
            tool_bridge: None,
            controls: Arc::new(Mutex::new(AcpControls::default())),
            messages: Vec::new(),
            partial_response: String::new(),
            config: None,
            commands: None,
            events: None,
            worker: None,
            state: AgentBackendStatus::Unavailable,
            submitted_prompt: false,
            shutting_down: false,
            permission_host: None,
        }
    }
}

impl AcpAgentBackend {
    pub fn configure(&mut self, config: AcpBackendConfig) {
        self.stop();
        self.submitted_prompt = false;
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
        if self.shutting_down {
            self.poll_shutdown();
            anyhow::ensure!(
                !self.shutdown_pending(),
                "ACP backend is still stopping; retry after exit"
            );
        }
        let config = self.config.clone().ok_or_else(|| {
            anyhow::anyhow!("ACP backend is not configured; supply an AcpAgentCommand")
        })?;
        if matches!(self.state, AgentBackendStatus::Failed)
            || self
                .controls
                .lock()
                .is_ok_and(|s| !s.ready && s.error.is_some())
            || self
                .worker
                .as_ref()
                .is_some_and(thread::JoinHandle::is_finished)
        {
            // A failed transport may still be unwinding an RPC. Keep its
            // handle until acknowledgement instead of joining on the UI path.
            self.request_shutdown();
            self.poll_shutdown();
            anyhow::ensure!(
                !self.shutdown_pending(),
                "ACP backend is still stopping; retry after exit"
            );
        }
        if self.worker.is_none() {
            // A new ACP process starts without remote history, so the next turn
            // must carry the saved transcript even when the previous one was
            // already submitted to an older process.
            self.submitted_prompt = false;
            self.tool_bridge = config
                .tool_bridge
                .as_ref()
                .map(crate::tool_bridge::ToolBridge::start)
                .transpose()?;
            let servers = self
                .tool_bridge
                .as_ref()
                .map(|bridge| vec![bridge.server()])
                .unwrap_or_default();
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
                            runtime.block_on(run_worker(
                                config,
                                commands_rx,
                                events_tx,
                                controls,
                                servers,
                            ));
                            // Windows child pipes do I/O on blocking threads. A
                            // pipe still held by an escaped descendant must not
                            // keep this worker, and the conversation, stopping.
                            runtime.shutdown_timeout(SHUTDOWN_RETRY_BUDGET);
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
        let controls = self.controls();
        if controls.pending {
            anyhow::bail!("ACP settings change is pending");
        }
        if !controls.ready
            && let Some(error) = controls.error
        {
            anyhow::bail!("ACP connection unavailable: {error}; reselect the Agent to reconnect");
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
        let images = prompt_images(messages);
        if !images.is_empty() {
            let controls = self.controls();
            // Before the session is ready the capability is unknown; the
            // worker's client re-checks it, so the turn fails rather than
            // silently dropping the images.
            if controls.ready && !controls.image_prompts {
                anyhow::bail!("This agent does not accept images; remove them and send again");
            }
        }
        let mut prompt = prompt_text(system.as_deref(), messages);
        if self.tool_bridge.is_some() {
            prompt.push_str("\n\nTeshi tools are available through the teshi-authoring MCP server. Your MCP client may prefix tool names with the server name. Use its submit_requirements, propose_test_points, generate_plan, create_feature_file, insert_scenario, and validate_feature tools for this workflow. Do not replace human review with chat approval. After proposing test points, stop and wait for the user to review them in Teshi and continue generation.");
        }
        // A new ACP process has no remote history. Supply the saved transcript
        // once, then let that process maintain its own conversation.
        if !self.submitted_prompt && messages.len() > 1 {
            let history = messages[..messages.len() - 1]
                .iter()
                .map(|message| {
                    format!(
                        "{}: {}",
                        message.role,
                        message.content.text().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            prompt =
                format!("[Restored conversation history]\n{history}\n\n[Current turn]\n{prompt}");
        }
        let mut blocks = vec![PromptBlock::Text(prompt)];
        blocks.extend(images);
        self.commands
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ACP worker is unavailable"))?
            .send(WorkerCommand::Prompt(blocks))
            .map_err(|_| anyhow::anyhow!("ACP worker exited"))?;
        self.submitted_prompt = true;
        Ok(())
    }

    /// End the running turn while keeping the ACP process and session.
    ///
    /// The worker sends `session/cancel`, waits for the agent to finish the turn,
    /// and stays connected, so the next prompt continues the same conversation.
    pub fn cancel(&mut self) {
        if let Some(bridge) = &mut self.tool_bridge {
            bridge.cancel_pending();
        }
        if let Some(commands) = &self.commands {
            let _ = commands.send(WorkerCommand::Cancel);
        }
        if let Some(host) = &self.permission_host {
            host.discard_requests();
        }
        self.state = AgentBackendStatus::Cancelled;
        self.partial_response.clear();
    }

    /// Stop this project's process and MCP bridge without blocking the frontend.
    pub fn request_shutdown(&mut self) {
        self.shutting_down = true;
        if let Some(bridge) = &mut self.tool_bridge {
            bridge.request_shutdown();
        }
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(WorkerCommand::Shutdown);
        }
        // Old updates and permission requests cannot mutate the retained A
        // context or any later B context while shutdown is in progress.
        self.events = None;
        self.permission_host = None;
        self.state = AgentBackendStatus::Cancelled;
        self.partial_response.clear();
    }

    /// Whether owned resources still need shutdown acknowledgement.
    pub fn shutdown_pending(&self) -> bool {
        self.shutting_down && (self.worker.is_some() || self.tool_bridge.is_some())
    }

    /// Reap only resources that have already exited; retain live handles on timeout.
    pub fn poll_shutdown(&mut self) {
        if !self.shutting_down {
            return;
        }
        if self
            .worker
            .as_ref()
            .is_some_and(thread::JoinHandle::is_finished)
            && let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
        if self
            .tool_bridge
            .as_mut()
            .is_some_and(|bridge| bridge.poll_shutdown())
        {
            self.tool_bridge = None;
        }
        if self.worker.is_none() && self.tool_bridge.is_none() {
            self.shutting_down = false;
        }
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
                // Seal preceding text before activity so the transcript follows
                // event order. Only text remaining at turn completion is final;
                // a retry's output starts a new record after the interrupted one.
                if matches!(
                    &event,
                    AgentBackendEvent::ReasoningChunk { .. }
                        | AgentBackendEvent::ExternalToolActivity { .. }
                        | AgentBackendEvent::Retrying { .. }
                ) && !self.partial_response.is_empty()
                {
                    self.messages.push(AiChatMessage {
                        role: AiRole::Assistant,
                        content: std::mem::take(&mut self.partial_response),
                        reasoning_content: None,
                        tool_calls: None,
                        tool_call_id: None,
                        source: Some("agent_progress".into()),
                        activity: None,
                    });
                }
                match &event {
                    AgentBackendEvent::MessageChunk { content } => {
                        self.partial_response.push_str(content)
                    }
                    AgentBackendEvent::ReasoningChunk { content } => {
                        if let Some(last) = self.messages.last_mut()
                            && last.source.as_deref() == Some("acp_reasoning")
                        {
                            last.reasoning_content
                                .get_or_insert_default()
                                .push_str(content);
                        } else {
                            self.messages.push(AiChatMessage {
                                role: AiRole::Assistant,
                                content: String::new(),
                                reasoning_content: Some(content.clone()),
                                tool_calls: None,
                                tool_call_id: None,
                                source: Some("acp_reasoning".into()),
                                activity: None,
                            });
                        }
                    }
                    AgentBackendEvent::ExternalToolActivity {
                        tool_call_id,
                        title,
                        status,
                        details,
                    } => {
                        let existing = tool_call_id.as_ref().and_then(|id| {
                            self.messages.iter_mut().rev().find(|m| {
                                m.source.as_deref() == Some("acp_activity")
                                    && m.tool_call_id.as_ref() == Some(id)
                            })
                        });
                        if let Some(message) = existing {
                            // ACP updates may omit the original title.
                            if title != "ACP agent tool" {
                                message.content = title.clone();
                            }
                            if let Some(status) = status {
                                message.reasoning_content = Some(status.clone());
                            }
                            let saved = message.activity.get_or_insert_default();
                            if details.kind.is_some() {
                                saved.kind.clone_from(&details.kind);
                            }
                            if details.input.is_some() {
                                saved.input.clone_from(&details.input);
                            }
                            if !details.locations.is_empty() {
                                saved.locations.clone_from(&details.locations);
                            }
                        } else {
                            self.messages.push(AiChatMessage {
                                role: AiRole::Assistant,
                                content: title.clone(),
                                reasoning_content: status.clone(),
                                tool_calls: None,
                                tool_call_id: tool_call_id.clone(),
                                source: Some("acp_activity".into()),
                                activity: Some(details.clone()),
                            });
                        }
                    }
                    AgentBackendEvent::Completed { finish_reason, .. } => {
                        let content = std::mem::take(&mut self.partial_response);
                        if !content.is_empty() {
                            self.messages.push(AiChatMessage {
                                role: AiRole::Assistant,
                                content,
                                tool_calls: None,
                                tool_call_id: None,
                                reasoning_content: None,
                                source: matches!(finish_reason.as_deref(), Some("cancelled"))
                                    .then(|| "agent_progress".into()),
                                activity: None,
                            });
                        }
                        self.state = AgentBackendStatus::Completed;
                    }
                    AgentBackendEvent::Failed(_) => {
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
        self.tool_bridge = None;
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(WorkerCommand::Shutdown);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.events = None;
        self.shutting_down = false;
    }

    pub(crate) fn host_tool_request(&self) -> Option<crate::tool_bridge::HostToolRequest> {
        self.tool_bridge.as_ref()?.try_recv()
    }

    pub(crate) fn defer_host_tool(&mut self, request: crate::tool_bridge::HostToolRequest) {
        if let Some(bridge) = &mut self.tool_bridge {
            bridge.defer(request);
        }
    }

    pub(crate) fn resolve_host_tool(&mut self, id: &str, result: Result<String, String>) -> bool {
        self.tool_bridge
            .as_mut()
            .is_some_and(|bridge| bridge.resolve(id, result))
    }

    pub(crate) fn host_tool_is_pending(&self, id: &str) -> bool {
        self.tool_bridge
            .as_ref()
            .is_some_and(|bridge| bridge.is_pending(id))
    }

    pub(crate) fn set_host_tools(&self, tools: Vec<teshi_core::llm::ToolDefinition>) {
        if let Some(bridge) = &self.tool_bridge {
            bridge.set_tools(tools);
        }
    }

    pub(crate) fn reconnect_host_tools(
        &mut self,
        tools: crate::tool_bridge::ToolBridgeConfig,
    ) -> anyhow::Result<()> {
        let config = self
            .config
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("ACP backend is not configured"))?;
        config.tool_bridge = Some(tools);
        self.stop();
        // A fresh remote session needs the saved transcript on its first turn.
        self.submitted_prompt = false;
        self.state = AgentBackendStatus::Idle;
        self.connect()
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

/// Inline images of the turn's message, in order, as ACP image blocks.
///
/// Only the newest message is sent to a live ACP session; earlier images
/// already belong to the agent's own history, and a restored transcript is
/// replayed as text.
fn prompt_images(messages: &[ChatMessage]) -> Vec<PromptBlock> {
    let Some(MessageContent::Blocks(blocks)) = messages.last().map(|message| &message.content)
    else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Image {
                source: ImageSource::Data { media_type, data },
            } => Some(PromptBlock::Image {
                media_type: media_type.clone(),
                data: base64::engine::general_purpose::STANDARD.encode(data),
            }),
            // ACP image blocks carry bytes; a remote URL has nothing to send.
            _ => None,
        })
        .collect()
}

fn is_model_option(option: &teshi_acp::session_config::ConfigOption) -> bool {
    option.category.as_deref() == Some("model") || option.id == "model"
}

/// Whether a confirmed value belongs in the per-agent option preferences.
///
/// The model keeps its own preference file; read-only types cannot be reapplied.
fn remembers_option(option: &teshi_acp::session_config::ConfigOption) -> bool {
    !is_model_option(option) && matches!(option.kind.as_str(), "select" | "boolean")
}

/// Merge a successful mutation response into the confirmed settings.
fn apply_option_response(
    config: &mut teshi_acp::session_config::SessionConfig,
    id: &str,
    value: serde_json::Value,
    response: &serde_json::Value,
) {
    if response.get("configOptions").is_some() {
        config.replace_modern(response.get("configOptions"));
    } else if let Some(current) = config.options.iter_mut().find(|o| o.id == id) {
        current.current = value;
    }
}

fn append_notice(target: &mut Option<String>, notice: String) {
    *target = Some(match target.take() {
        Some(existing) => format!("{existing}; {notice}"),
        None => notice,
    });
}

/// Read the saved option values; a missing file means nothing is remembered.
fn read_option_preferences(
    path: &std::path::Path,
) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error.into()),
    }
}

/// Record one confirmed value while keeping the agent's other saved options.
///
/// A corrupt file is reported instead of replaced so the user's other values
/// are not silently lost.
fn save_option_preference(
    path: &std::path::Path,
    id: &str,
    value: &serde_json::Value,
) -> anyhow::Result<()> {
    let mut saved = read_option_preferences(path)?;
    saved.insert(id.to_owned(), value.clone());
    teshi_engine::write_atomic(path, &saved)
}

/// Reapply remembered non-model options to a new session before its first prompt.
///
/// Returns user-facing notices for values that fell back to the agent default
/// because the agent no longer offers them or refused them. A timeout is an
/// error: the remote value is then unknown.
async fn restore_option_preferences(
    client: &AcpClient<tokio::process::ChildStdin>,
    session: &mut teshi_acp::AcpSession,
    path: &std::path::Path,
) -> anyhow::Result<Vec<String>> {
    let order: Vec<String> = session
        .config
        .options
        .iter()
        .map(|o| o.id.clone())
        .collect();
    let mut saved: Vec<_> = read_option_preferences(path)?.into_iter().collect();
    // Follow the agent's advertised order so dependent options see their
    // controlling option's restored value first.
    saved.sort_by_key(|(id, _)| order.iter().position(|o| o == id).unwrap_or(usize::MAX));
    let mut notices = Vec::new();
    for (id, value) in saved {
        // Each mutation may replace the advertised list, so resolve against
        // the latest confirmed settings rather than the initial snapshot.
        let Some(option) = session
            .config
            .options
            .iter()
            .find(|o| o.id == id && remembers_option(o))
            .cloned()
        else {
            notices.push(format!(
                "Saved setting {id} is no longer offered; using the agent default"
            ));
            continue;
        };
        if !option.accepts(&value) {
            notices.push(format!(
                "Saved {} is no longer available; using the agent default",
                option.name
            ));
            continue;
        }
        if option.current == value {
            continue;
        }
        match client
            .set_session_option(session, &option, value.clone())
            .await
        {
            Ok(response) => {
                apply_option_response(&mut session.config, &option.id, value, &response)
            }
            Err(error @ teshi_acp::AcpError::Timeout(_)) => return Err(error.into()),
            Err(error) => notices.push(format!(
                "Saved {} could not be restored: {error}; using the agent default",
                option.name
            )),
        }
    }
    Ok(notices)
}

async fn run_worker(
    config: AcpBackendConfig,
    mut commands: async_mpsc::UnboundedReceiver<WorkerCommand>,
    output: mpsc::Sender<AgentBackendEvent>,
    controls: Arc<Mutex<AcpControls>>,
    servers: Vec<teshi_acp::McpServer>,
) {
    let (updates_tx, mut updates_rx) = async_mpsc::channel(128);
    let cwd: PathBuf = config.command.cwd.clone();
    let model_preference_path = config.model_preference_path.clone();
    let option_preferences_path = config.option_preferences_path.clone();
    let turn_retry = config.turn_retry.clone();
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
            .new_session_with_mcp(cwd, servers)
            .await
            .map_err(|e| format!("ACP session creation failed: {e}"))?;
        Ok::<_, String>((session, model, initialized.accepts_images()))
    }
    .await;
    let (mut session, model, image_prompts) = match result {
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
    let mut preference_error = None;
    if let Some(path) = &model_preference_path {
        let restored = async {
            let bytes = match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(anyhow::Error::from(error)),
            };
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            let option = session
                .config
                .options
                .iter()
                .find(|o| is_model_option(o))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Agent has no model setting"))?;
            anyhow::ensure!(option.accepts(&value), "Saved model is no longer available");
            if option.current != value {
                let response = client
                    .set_session_option(&session, &option, value.clone())
                    .await?;
                apply_option_response(&mut session.config, &option.id, value, &response);
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(error) = restored {
            preference_error = Some(format!(
                "ACP model preference could not be restored: {error}"
            ));
            // A timed-out mutation leaves the remote model uncertain. Do not
            // send prompts on a session whose configuration is unknown.
            if error
                .downcast_ref::<teshi_acp::AcpError>()
                .is_some_and(|e| matches!(e, teshi_acp::AcpError::Timeout(_)))
            {
                if let Ok(mut state) = controls.lock() {
                    state.error = preference_error;
                }
                let _ = client.shutdown().await;
                return;
            }
        }
    }
    // Restored after the model, whose choice can change which values the agent
    // advertises for dependent options such as thinking levels.
    if let Some(path) = &option_preferences_path {
        match restore_option_preferences(&client, &mut session, path).await {
            Ok(notices) => {
                for notice in notices {
                    append_notice(&mut preference_error, notice);
                }
            }
            Err(error) => {
                append_notice(
                    &mut preference_error,
                    format!("ACP session options could not be restored: {error}"),
                );
                // Same contract as the model: an unconfirmed remote value must
                // not receive prompts.
                if error
                    .downcast_ref::<teshi_acp::AcpError>()
                    .is_some_and(|e| matches!(e, teshi_acp::AcpError::Timeout(_)))
                {
                    if let Ok(mut state) = controls.lock() {
                        state.error = preference_error;
                    }
                    let _ = client.shutdown().await;
                    return;
                }
            }
        }
    }
    if let Ok(mut state) = controls.lock() {
        state.config = session.config.clone();
        state.error = preference_error;
        state.session_id = Some(session.id.clone());
        state.image_prompts = image_prompts;
        state.ready = true;
        state.revision += 1;
    }
    let mut shutdown = false;
    loop {
        let command = tokio::select! {
            command = commands.recv() => match command { Some(command) => command, None => break },
            Some(update) = updates_rx.recv() => {
                apply_control_update(&update, &session.id, &controls);
                if forward_update(update, &output, false) { break; }
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
                            apply_option_response(&mut state.config, &option.id, value, &response);
                            state.error = None;
                            // Persist the agent-confirmed value, which may differ
                            // from the request when the agent normalizes it.
                            let confirmed = state
                                .config
                                .options
                                .iter()
                                .find(|o| o.id == option.id)
                                .map(|o| o.current.clone());
                            if let Some(confirmed) = confirmed {
                                let saved = if is_model_option(&option) {
                                    model_preference_path.as_ref().map(|path| {
                                        teshi_engine::write_atomic(path, &confirmed)
                                            .map_err(|e| format!("Model changed, but saving its preference failed: {e}"))
                                    })
                                } else if remembers_option(&option) {
                                    option_preferences_path.as_ref().map(|path| {
                                        save_option_preference(path, &option.id, &confirmed)
                                            .map_err(|e| format!("{} changed, but saving its preference failed: {e}", option.name))
                                    })
                                } else {
                                    None
                                };
                                if let Some(Err(error)) = saved {
                                    state.error = Some(error);
                                }
                            }
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
                let mut prompt = prompt;
                // Retries spent on the current user turn, and whether any of its
                // attempts showed output or ran tools. Once one has, a retry asks
                // the agent to continue instead of resending the request.
                let mut retries = 0_usize;
                let mut progressed = false;
                loop {
                    // Scoped so the turn future's borrow of `prompt` ends before
                    // a deferred prompt or a retry replaces it.
                    let (stop, deferred_prompt, retry_reason) = {
                        let mut turn = Box::pin(client.prompt_blocks(&session, &prompt));
                        let mut cancelled = false;
                        // Re-armed on every cancel so an agent that ignores
                        // `session/cancel` degrades to a real teardown instead of
                        // parking the worker forever.
                        let mut cancel_deadline = tokio::time::Instant::now() + CANCEL_ACK_BUDGET;
                        let mut deferred_prompt: Option<Vec<PromptBlock>> = None;
                        let mut protocol_failed = false;
                        let mut turn_failed = false;
                        let mut attempt = TurnAttempt::default();
                        let mut retry_reason = None;
                        loop {
                            tokio::select! {
                                result = &mut turn => {
                                    while let Ok(update) = updates_rx.try_recv() {
                                        apply_control_update(&update, &session.id, &controls);
                                        protocol_failed |= attempt.route(update, &output, cancelled);
                                    }
                                    if !cancelled && !protocol_failed {
                                        let transient = match &result {
                                            Ok(_) => attempt.notice.take().map(|notice| notice.trim().to_string()),
                                            Err(error) if acp_retry::is_transient_prompt_error(error) => Some(error.to_string()),
                                            Err(_) => None,
                                        };
                                        match (transient, result) {
                                            (Some(reason), _) => retry_reason = Some(reason),
                                            (None, Ok(completion)) => { let _ = output.send(AgentBackendEvent::Completed {
                                                model: model.clone(),
                                                input_tokens: None,
                                                output_tokens: None,
                                                finish_reason: completion.stop_reason,
                                                detail: completion.detail,
                                            }); }
                                            (None, Err(error)) => {
                                                turn_failed = true;
                                                let _ = output.send(AgentBackendEvent::Failed(format!("ACP prompt failed: {error}")));
                                            }
                                        }
                                    }
                                    break;
                                }
                                Some(update) = updates_rx.recv() => {
                                    apply_control_update(&update, &session.id, &controls);
                                    protocol_failed |= attempt.route(update, &output, cancelled);
                                }
                                Some(command) = commands.recv() => match command {
                                    WorkerCommand::Policy(policy) => {
                                        client.set_permission_policy(policy).await;
                                        if let Ok(mut state) = controls.lock() { state.policy = Some(policy); }
                                    }
                                    WorkerCommand::Configure { .. } => {
                                        if let Ok(mut state) = controls.lock() { state.pending = false; state.error = Some("Wait for the current turn to finish".into()); }
                                    }
                                    // Cancelling ends this turn, not the session: keep the
                                    // process, keep polling the turn, and drop the events the
                                    // stopped turn still produces.
                                    WorkerCommand::Cancel => {
                                        if !cancelled {
                                            cancelled = true;
                                            cancel_deadline = tokio::time::Instant::now() + CANCEL_ACK_BUDGET;
                                            let _ = client.cancel(&session).await;
                                        }
                                    }
                                    WorkerCommand::Shutdown => {
                                        shutdown = true;
                                        drop(turn);
                                        let _ = client.cancel(&session).await;
                                        break;
                                    }
                                    WorkerCommand::Prompt(next) => {
                                        if cancelled && deferred_prompt.is_none() {
                                            deferred_prompt = Some(next);
                                        } else {
                                            let _ = output.send(AgentBackendEvent::Failed("ACP prompt already running".into()));
                                        }
                                    }
                                },
                                _ = tokio::time::sleep_until(cancel_deadline), if cancelled => {
                                    turn_failed = true;
                                    let _ = output.send(AgentBackendEvent::Failed(
                                        "ACP agent did not acknowledge the cancellation; reconnect required".into(),
                                    ));
                                    drop(turn);
                                    break;
                                }
                            }
                        }
                        progressed |= attempt.progressed;
                        (
                            shutdown || protocol_failed || turn_failed,
                            deferred_prompt,
                            retry_reason,
                        )
                    };
                    if stop {
                        break;
                    }
                    if let Some(reason) = retry_reason {
                        let Some(delay) = turn_retry.delay(retries) else {
                            let _ = output.send(AgentBackendEvent::Failed(
                                exhausted_retry_message(retries, &reason),
                            ));
                            break;
                        };
                        retries += 1;
                        let _ = output.send(AgentBackendEvent::Retrying {
                            attempt: retries as u32,
                            max_attempts: turn_retry.max_retries() as u32,
                            delay,
                            reason,
                        });
                        match wait_before_retry(
                            delay,
                            &mut commands,
                            &mut updates_rx,
                            &client,
                            &session.id,
                            &controls,
                            &output,
                        )
                        .await
                        {
                            RetryWait::Elapsed => {
                                if progressed {
                                    prompt = vec![PromptBlock::Text(
                                        acp_retry::CONTINUE_AFTER_INTERRUPTION.into(),
                                    )];
                                }
                                continue;
                            }
                            RetryWait::Stopped => break,
                            RetryWait::Shutdown => {
                                shutdown = true;
                                break;
                            }
                        }
                    }
                    match deferred_prompt {
                        Some(next) => {
                            prompt = next;
                            retries = 0;
                            progressed = false;
                        }
                        None => break,
                    }
                }
                if shutdown {
                    break;
                }
            }
            // A cancel received between turns keeps the session for the next prompt.
            WorkerCommand::Cancel => {}
            WorkerCommand::Shutdown => break,
        }
    }
    if let Ok(mut state) = controls.lock() {
        state.ready = false;
        state.pending = false;
    }
    // Keep the worker owned until the process container has actually stopped.
    // The frontend's bounded switch wait can time out without losing this handle.
    let deadline = tokio::time::Instant::now() + SHUTDOWN_RETRY_BUDGET;
    while let Err(error) = client.shutdown().await {
        tracing::error!(error = %error, "ACP shutdown has not completed");
        if tokio::time::Instant::now() >= deadline {
            tracing::error!(
                "Abandoning the ACP process container after repeated shutdown failures"
            );
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn apply_control_update(update: &AcpEvent, session: &str, controls: &Mutex<AcpControls>) {
    if let AcpEvent::SessionUpdate(params) = update
        && let Ok(mut state) = controls.lock()
        && state.config.apply_update(session, params)
    {
        state.revision += 1;
    }
}

fn forward_update(update: AcpEvent, output: &mpsc::Sender<AgentBackendEvent>, muted: bool) -> bool {
    let failed = matches!(&update, AcpEvent::Error(_));
    if muted && !failed {
        // A cancelled turn keeps polling so the agent can acknowledge the
        // cancel, but its remaining chunks must not land after `[cancelled]`.
        // Protocol errors still surface: they end the worker.
        return false;
    }
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
            let tool_call_id = update
                .get("toolCallId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            Some(AgentBackendEvent::ExternalToolActivity {
                tool_call_id,
                title,
                status,
                details: teshi_agent::backend::ToolActivityInfo {
                    kind: update
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                    input: update.get("rawInput").cloned(),
                    locations: update
                        .get("locations")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.get("path").and_then(|p| p.as_str()).map(str::to_owned))
                        .collect(),
                },
            })
        }
        AcpEvent::SessionUpdate(value)
            if value
                .get("update")
                .unwrap_or(&value)
                .get("sessionUpdate")
                .and_then(serde_json::Value::as_str)
                == Some("agent_thought_chunk") =>
        {
            value
                .get("update")
                .unwrap_or(&value)
                .get("content")
                .and_then(|v| v.get("text"))
                .and_then(serde_json::Value::as_str)
                .map(|content| AgentBackendEvent::ReasoningChunk {
                    content: content.into(),
                })
        }
        AcpEvent::SessionUpdate(value)
            if value
                .get("update")
                .unwrap_or(&value)
                .get("sessionUpdate")
                .and_then(serde_json::Value::as_str)
                == Some("session_info_update") =>
        {
            // Only a present `title` speaks about the label: an update that
            // merely refreshes `updatedAt` must leave it alone. A null title
            // clears the agent label so the client's own title can return.
            let update = value.get("update").unwrap_or(&value);
            match update.get("title") {
                Some(serde_json::Value::String(title)) => {
                    let title = title.trim();
                    (!title.is_empty()).then(|| AgentBackendEvent::SessionTitle {
                        title: Some(title.to_string()),
                    })
                }
                Some(serde_json::Value::Null) => {
                    Some(AgentBackendEvent::SessionTitle { title: None })
                }
                _ => None,
            }
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

/// Routes one attempt's updates and records what the attempt did, so a
/// transient failure can be retried without repeating visible work.
#[derive(Default)]
struct TurnAttempt {
    /// Transient-failure notice held back from the transcript, as streamed.
    notice: Option<String>,
    /// Whether the attempt streamed answer text or reported tool activity.
    progressed: bool,
}

impl TurnAttempt {
    /// Forwards `update` like [`forward_update`], except that a
    /// transient-failure notice waits for the turn's outcome.
    fn route(
        &mut self,
        update: AcpEvent,
        output: &mpsc::Sender<AgentBackendEvent>,
        muted: bool,
    ) -> bool {
        if !muted {
            match &update {
                AcpEvent::AgentMessageChunk(content)
                    if acp_retry::transient_notice(content).is_some() =>
                {
                    self.notice = Some(content.clone());
                    return false;
                }
                AcpEvent::AgentMessageChunk(content) if !content.trim().is_empty() => {
                    self.record_progress(output);
                }
                AcpEvent::ToolCall(_) | AcpEvent::ToolCallUpdate(_) => self.record_progress(output),
                _ => {}
            }
        }
        forward_update(update, output, muted)
    }

    /// Work after a notice means the agent recovered by itself: the notice
    /// did not end the turn, so it is shown in its original position.
    fn record_progress(&mut self, output: &mpsc::Sender<AgentBackendEvent>) {
        self.progressed = true;
        if let Some(content) = self.notice.take() {
            let _ = output.send(AgentBackendEvent::MessageChunk { content });
        }
    }
}

/// How the pause before an automatic retry ended.
enum RetryWait {
    Elapsed,
    /// The user cancelled the turn, or the session failed during the pause.
    Stopped,
    Shutdown,
}

/// Waits out a retry backoff without parking the worker: settings and policy
/// commands keep working, and a cancel or shutdown ends the turn at once.
async fn wait_before_retry<W: tokio::io::AsyncWrite + Unpin + Send + 'static>(
    delay: std::time::Duration,
    commands: &mut async_mpsc::UnboundedReceiver<WorkerCommand>,
    updates_rx: &mut async_mpsc::Receiver<AcpEvent>,
    client: &AcpClient<W>,
    session_id: &str,
    controls: &Mutex<AcpControls>,
    output: &mpsc::Sender<AgentBackendEvent>,
) -> RetryWait {
    let sleep = tokio::time::sleep(delay);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            () = &mut sleep => return RetryWait::Elapsed,
            Some(update) = updates_rx.recv() => {
                apply_control_update(&update, session_id, controls);
                // The failed attempt is over, so its stragglers stay muted;
                // a protocol error still surfaces and ends the turn.
                if forward_update(update, output, true) {
                    return RetryWait::Stopped;
                }
            }
            command = commands.recv() => match command {
                Some(WorkerCommand::Policy(policy)) => {
                    client.set_permission_policy(policy).await;
                    if let Ok(mut state) = controls.lock() {
                        state.policy = Some(policy);
                    }
                }
                Some(WorkerCommand::Configure { .. }) => {
                    if let Ok(mut state) = controls.lock() {
                        state.pending = false;
                        state.error = Some("Wait for the current turn to finish".into());
                    }
                }
                Some(WorkerCommand::Prompt(_)) => {
                    let _ = output.send(AgentBackendEvent::Failed("ACP prompt already running".into()));
                }
                Some(WorkerCommand::Cancel) => return RetryWait::Stopped,
                Some(WorkerCommand::Shutdown) | None => return RetryWait::Shutdown,
            }
        }
    }
}

/// Terminal failure text once a turn's retry budget is spent.
fn exhausted_retry_message(retries: usize, reason: &str) -> String {
    match retries {
        0 => format!("ACP prompt failed: {reason}"),
        1 => format!("ACP prompt failed after 1 automatic retry: {reason}"),
        n => format!("ACP prompt failed after {n} automatic retries: {reason}"),
    }
}
