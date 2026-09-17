//! Synchronous frontend adapter for the asynchronous teshi-acp client.
use std::{path::PathBuf, sync::mpsc, thread};

use teshi_acp::{
    AcpClient, AcpEvent, client::AcpClientConfig, command::AcpAgentCommand,
    permission::PermissionPolicy,
};
use teshi_agent::backend::{AgentBackendEvent, AgentBackendStatus};
use teshi_core::llm::{ChatMessage, ContentBlock, MessageContent};
use tokio::sync::mpsc as async_mpsc;

use crate::{AiChatMessage, AiRole};

#[derive(Debug, Clone)]
pub struct AcpBackendConfig {
    pub command: AcpAgentCommand,
    pub permission_policy: PermissionPolicy,
    /// An Agent-advertised ACP authentication method, if the caller selected one.
    pub auth_method: Option<String>,
    pub client: AcpClientConfig,
}

impl AcpBackendConfig {
    pub fn new(command: AcpAgentCommand) -> Self {
        Self {
            command,
            permission_policy: PermissionPolicy::Manual,
            auth_method: None,
            client: AcpClientConfig::default(),
        }
    }
}

enum WorkerCommand {
    Prompt(String),
    Cancel,
    Shutdown,
}

pub struct AcpAgentBackend {
    pub messages: Vec<AiChatMessage>,
    pub partial_response: String,
    config: Option<AcpBackendConfig>,
    commands: Option<async_mpsc::UnboundedSender<WorkerCommand>>,
    events: Option<mpsc::Receiver<AgentBackendEvent>>,
    worker: Option<thread::JoinHandle<()>>,
    state: AgentBackendStatus,
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
            messages: Vec::new(),
            partial_response: String::new(),
            config: None,
            commands: None,
            events: None,
            worker: None,
            state: AgentBackendStatus::Unavailable,
        }
    }
}

impl AcpAgentBackend {
    pub fn configure(&mut self, config: AcpBackendConfig) {
        self.stop();
        self.config = Some(config);
        self.state = AgentBackendStatus::Idle;
    }

    pub fn status(&self) -> AgentBackendStatus {
        self.state
    }

    pub fn is_connected(&self) -> bool {
        self.commands.is_some()
    }

    pub fn start(&mut self) -> anyhow::Result<()> {
        let config = self.config.clone().ok_or_else(|| {
            anyhow::anyhow!("ACP backend is not configured; supply an AcpAgentCommand")
        })?;
        if matches!(
            self.state,
            AgentBackendStatus::Running
                | AgentBackendStatus::Failed
                | AgentBackendStatus::Cancelled
        ) || self
            .worker
            .as_ref()
            .is_some_and(thread::JoinHandle::is_finished)
        {
            self.stop();
        }
        if self.worker.is_none() {
            let (commands_tx, commands_rx) = async_mpsc::unbounded_channel();
            let (events_tx, events_rx) = mpsc::channel();
            let worker = thread::Builder::new()
                .name("teshi-acp-backend".into())
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build();
                    match runtime {
                        Ok(runtime) => runtime.block_on(run_worker(config, commands_rx, events_tx)),
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
        }
        self.partial_response.clear();
        self.state = AgentBackendStatus::Running;
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
) {
    let (updates_tx, mut updates_rx) = async_mpsc::channel(128);
    let cwd: PathBuf = config.command.cwd.clone();
    let auth_method = config.auth_method.clone();
    let mut client = match AcpClient::spawn(
        config.command,
        config.client,
        updates_tx,
        config.permission_policy,
        None,
    )
    .await
    {
        Ok(client) => client,
        Err(error) => {
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
            let _ = output.send(AgentBackendEvent::Failed(error));
            let _ = client.shutdown().await;
            return;
        }
    };
    while let Some(command) = commands.recv().await {
        match command {
            WorkerCommand::Prompt(prompt) => {
                let mut turn = Box::pin(client.prompt(&session, &prompt));
                let mut cancelled = false;
                let mut protocol_failed = false;
                let mut turn_failed = false;
                loop {
                    tokio::select! {
                        result = &mut turn => {
                            while let Ok(update) = updates_rx.try_recv() {
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
                            protocol_failed |= forward_update(update, &output);
                        }
                        Some(command) = commands.recv() => match command {
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
    let _ = client.shutdown().await;
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
