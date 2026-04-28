//! LLM integration module using the async-openai crate.
//!
//! This module provides a minimal, non-blocking interface for sending prompts
//! to an OpenAI-compatible API and receiving streaming completions. It follows
//! the same background-thread + channel pattern as `runner.rs` so the
//! synchronous TUI event loop never blocks on network I/O.
//!
//! Streaming is the only mode: partial text chunks are delivered via
//! `LlmEvent::Chunk` so the UI can render the response in real time.
//!
//! # Tool calling
//!
//! When `LlmRequest::Chat` includes `tools`, the model may respond with
//! `LlmEvent::ToolCallRequest` instead of text. The caller should execute the
//! requested tools and feed results back as `ChatMessage` with `role: "tool"`.
//!
//! # Usage
//!
//! ```ignore
//! let config = LlmConfig::from_env().unwrap();
//! let (handle, rx) = spawn_llm(config);
//! handle.send(LlmRequest::Chat {
//!     system: Some("You are a helpful assistant.".into()),
//!     messages: vec![ChatMessage {
//!         role: "user".into(),
//!         content: "What is BDD?".into(),
//!         tool_calls: None,
//!         tool_call_id: None,
//!     }],
//!     tools: None,
//! })?;
//!
//! while let Ok(event) = rx.recv() {
//!     match event {
//!         LlmEvent::Chunk { content, .. } => { /* stream partial */ }
//!         LlmEvent::Done { full_text, .. } => { /* done */ }
//!         LlmEvent::ToolCallRequest { tool_calls } => { /* execute tools */ }
//!         LlmEvent::Error { message } => { /* handle */ }
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use anyhow::{Context, Result};

// ── Configuration ────────────────────────────────────────────────────────────

/// LLM client configuration loaded from environment variables.
///
/// | Variable | Required | Default |
/// |---|---|---|
/// | `TESHI_LLM_API_KEY` | Yes | — |
/// | `TESHI_LLM_BASE_URL` | No | `https://api.openai.com/v1` |
/// | `TESHI_LLM_MODEL` | No | `gpt-4o-mini` |
/// | `TESHI_LLM_MAX_TOKENS` | No | `1024` |
/// | `TESHI_LLM_TEMPERATURE` | No | `0.7` |
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl LlmConfig {
    /// Build config from environment variables, returning an error if
    /// `TESHI_LLM_API_KEY` is missing.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("TESHI_LLM_API_KEY")
            .map_err(|_| anyhow::anyhow!("TESHI_LLM_API_KEY must be set"))?;
        let base_url = std::env::var("TESHI_LLM_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".into());
        let model = std::env::var("TESHI_LLM_MODEL")
            .unwrap_or_else(|_| "gpt-4o-mini".into());
        let max_tokens = std::env::var("TESHI_LLM_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1024);
        let temperature = std::env::var("TESHI_LLM_TEMPERATURE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.7);
        Ok(Self {
            api_key,
            base_url,
            model,
            max_tokens,
            temperature,
        })
    }

    /// Check whether the required env-var is present (without returning config).
    pub fn is_configured() -> bool {
        std::env::var("TESHI_LLM_API_KEY").is_ok()
    }
}

// ── Tool types ───────────────────────────────────────────────────────────────

/// A tool definition conforming to OpenAI's function-calling JSON Schema format.
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    /// The name of the function (a-z, A-Z, 0-9, underscores, dashes).
    pub name: String,
    /// A description of what the function does.
    pub description: String,
    /// JSON Schema for the function parameters.
    pub parameters: serde_json::Value,
}

/// A tool call request returned by the LLM.
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// Unique identifier for this tool call.
    pub id: String,
    /// The name of the function to call.
    pub name: String,
    /// JSON-encoded arguments for the function.
    pub arguments: String,
}

// ── Message types ────────────────────────────────────────────────────────────

/// A structured chat message with role, content, and optional tool fields.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// One of `"user"`, `"assistant"`, `"system"`, or `"tool"`.
    pub role: String,
    /// The message content (may be empty for assistant messages that only
    /// contain tool calls).
    pub content: String,
    /// Tool calls included in an assistant message.
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The tool call ID this message responds to (for `role: "tool"`).
    pub tool_call_id: Option<String>,
}

// ── Request / Event types ────────────────────────────────────────────────────

/// A request sent into the LLM background thread.
#[derive(Debug, Clone)]
pub enum LlmRequest {
    /// A chat completion request with optional tools.
    Chat {
        /// Optional system prompt.
        system: Option<String>,
        /// Messages in conversation order.
        messages: Vec<ChatMessage>,
        /// Optional tool definitions for function calling.
        tools: Option<Vec<ToolDefinition>>,
    },
}

/// An event emitted by the LLM background thread.
#[derive(Debug, Clone)]
pub enum LlmEvent {
    /// A streaming text chunk.
    Chunk {
        content: String,
    },
    /// The completion finished successfully with a text response.
    Done {
        full_text: String,
        /// Input + output token usage, if reported.
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        model: String,
    },
    /// The model requested one or more tool calls instead of a text response.
    ToolCallRequest {
        tool_calls: Vec<ToolCall>,
    },
    /// A non-recoverable error occurred.
    Error {
        message: String,
    },
}

// ── Handle ───────────────────────────────────────────────────────────────────

/// A handle that can send requests to the background LLM thread.
#[derive(Debug)]
pub struct LlmHandle {
    tx: Sender<LlmRequest>,
}

impl LlmHandle {
    /// Send a request to the LLM background thread.
    pub fn send(&self, request: LlmRequest) -> Result<()> {
        self.tx
            .send(request)
            .context("LLM background thread has exited")
    }
}

// ── Spawn ────────────────────────────────────────────────────────────────────

/// Spawn a background thread that runs a tokio runtime and services LLM
/// requests with streaming completions.
///
/// Returns a `(LlmHandle, Receiver<LlmEvent>)` pair. Drop the handle when you
/// no longer need to send requests; the thread will shut down once the channel
/// is closed and the current request finishes.
pub fn spawn_llm(config: LlmConfig) -> (LlmHandle, Receiver<LlmEvent>) {
    let (req_tx, req_rx) = mpsc::channel::<LlmRequest>();
    let (evt_tx, evt_rx) = mpsc::channel::<LlmEvent>();

    thread::Builder::new()
        .name("teshi-llm".into())
        .spawn(move || run_llm_worker(config, req_rx, evt_tx))
        .expect("failed to spawn LLM worker thread");

    (LlmHandle { tx: req_tx }, evt_rx)
}

// ── Background worker ────────────────────────────────────────────────────────

fn run_llm_worker(
    config: LlmConfig,
    req_rx: Receiver<LlmRequest>,
    evt_tx: Sender<LlmEvent>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = evt_tx.send(LlmEvent::Error {
                message: format!("failed to build tokio runtime: {e}"),
            });
            return;
        }
    };

    while let Ok(request) = req_rx.recv() {
        match request {
            LlmRequest::Chat {
                system,
                messages,
                tools,
            } => {
                rt.block_on(process_chat_request(
                    &config, system, messages, tools, &evt_tx,
                ));
            }
        }
    }
}

/// Process a chat request with a 120-second timeout.
async fn process_chat_request(
    config: &LlmConfig,
    system: Option<String>,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<ToolDefinition>>,
    evt_tx: &Sender<LlmEvent>,
) {
    let timeout_dur = std::time::Duration::from_secs(120);
    match tokio::time::timeout(
        timeout_dur,
        chat_completion(config, system, messages, tools, evt_tx),
    )
    .await
    {
        Ok(()) => {}
        Err(_elapsed) => {
            let _ = evt_tx.send(LlmEvent::Error {
                message: "API call timed out after 120 seconds".to_string(),
            });
        }
    }
}

// ── Message conversion helpers ───────────────────────────────────────────────

use async_openai::types::chat::{
    ChatCompletionMessageToolCall as OpenAiMessageToolCall,
    ChatCompletionMessageToolCalls, ChatCompletionRequestAssistantMessageArgs,
    ChatCompletionRequestAssistantMessageContent, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestSystemMessageContent,
    ChatCompletionRequestToolMessageArgs, ChatCompletionRequestToolMessageContent,
    ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
    FunctionCall,
};

fn build_openai_messages(
    system: Option<String>,
    messages: &[ChatMessage],
) -> Vec<ChatCompletionRequestMessage> {
    let mut req_messages: Vec<ChatCompletionRequestMessage> = Vec::new();

    if let Some(sys) = system {
        if let Ok(msg) = ChatCompletionRequestSystemMessageArgs::default()
            .content(ChatCompletionRequestSystemMessageContent::Text(sys))
            .build()
            .map(ChatCompletionRequestMessage::System)
        {
            req_messages.push(msg);
        }
    }

    for msg in messages {
        let openai_msg = match msg.role.as_str() {
            "user" => {
                let Ok(m) = ChatCompletionRequestUserMessageArgs::default()
                    .content(ChatCompletionRequestUserMessageContent::Text(
                        msg.content.clone(),
                    ))
                    .build()
                    .map(ChatCompletionRequestMessage::User)
                else {
                    continue;
                };
                m
            }
            "assistant" => {
                let mut builder = ChatCompletionRequestAssistantMessageArgs::default();
                if let Some(ref tool_calls) = msg.tool_calls {
                    let oai_tool_calls: Vec<ChatCompletionMessageToolCalls> = tool_calls
                        .iter()
                        .map(|tc| {
                            ChatCompletionMessageToolCalls::Function(
                                OpenAiMessageToolCall {
                                    id: tc.id.clone(),
                                    function: FunctionCall {
                                        name: tc.name.clone(),
                                        arguments: tc.arguments.clone(),
                                    },
                                },
                            )
                        })
                        .collect();
                    builder.tool_calls(oai_tool_calls);
                }
                if !msg.content.is_empty() {
                    builder.content(
                        ChatCompletionRequestAssistantMessageContent::Text(
                            msg.content.clone(),
                        ),
                    );
                }
                let Ok(m) = builder
                    .build()
                    .map(ChatCompletionRequestMessage::Assistant)
                else {
                    continue;
                };
                m
            }
            "tool" => {
                let Ok(m) = ChatCompletionRequestToolMessageArgs::default()
                    .content(ChatCompletionRequestToolMessageContent::Text(
                        msg.content.clone(),
                    ))
                    .tool_call_id(msg.tool_call_id.clone().unwrap_or_default())
                    .build()
                    .map(ChatCompletionRequestMessage::Tool)
                else {
                    continue;
                };
                m
            }
            _ => continue,
        };
        req_messages.push(openai_msg);
    }

    req_messages
}

fn build_openai_tools(
    tools: &[ToolDefinition],
) -> Vec<async_openai::types::chat::ChatCompletionTools> {
    use async_openai::types::chat::{
        ChatCompletionTool, ChatCompletionTools, FunctionObject,
    };

    tools
        .iter()
        .map(|td| {
            let func = FunctionObject {
                name: td.name.clone(),
                description: Some(td.description.clone()),
                parameters: Some(td.parameters.clone()),
                strict: None,
            };
            ChatCompletionTools::Function(ChatCompletionTool { function: func })
        })
        .collect()
}

// ── Streaming chat completion ────────────────────────────────────────────────

async fn chat_completion(
    config: &LlmConfig,
    system: Option<String>,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<ToolDefinition>>,
    evt_tx: &Sender<LlmEvent>,
) {
    use async_openai::Client;
    use async_openai::config::OpenAIConfig;
    use async_openai::types::chat::CreateChatCompletionRequestArgs;
    use futures::StreamExt;

    let openai_config = OpenAIConfig::default()
        .with_api_key(&config.api_key)
        .with_api_base(&config.base_url);

    let client = Client::with_config(openai_config);

    let req_messages = build_openai_messages(system, &messages);

    let mut request_builder = CreateChatCompletionRequestArgs::default();
    request_builder
        .model(&config.model)
        .messages(req_messages)
        .max_tokens(config.max_tokens)
        .temperature(config.temperature);

    if let Some(ref tool_defs) = tools {
        let oai_tools = build_openai_tools(tool_defs);
        request_builder.tools(oai_tools);
    }

    let request = match request_builder.build() {
        Ok(req) => req,
        Err(e) => {
            let _ = evt_tx.send(LlmEvent::Error {
                message: format!("failed to build request: {e}"),
            });
            return;
        }
    };

    let mut stream = match client.chat().create_stream(request).await {
        Ok(s) => s,
        Err(e) => {
            let _ = evt_tx.send(LlmEvent::Error {
                message: format!("API call failed: {e}"),
            });
            return;
        }
    };

    let mut full_text = String::new();
    let mut model_name = String::new();
    let mut input_tokens: Option<u32> = None;
    let mut output_tokens: Option<u32> = None;

    // Accumulate streaming tool call chunks keyed by index.
    // (id, name, accumulated_arguments_json)
    let mut tool_call_chunks: HashMap<u32, (Option<String>, Option<String>, String)> =
        HashMap::new();

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                if model_name.is_empty() {
                    model_name = chunk.model.clone();
                }
                if let Some(usage) = chunk.usage {
                    input_tokens = Some(usage.prompt_tokens as u32);
                    output_tokens = Some(usage.completion_tokens as u32);
                }
                for choice in &chunk.choices {
                    // Handle text content deltas
                    if let Some(ref delta) = choice.delta.content {
                        full_text.push_str(delta);
                        let _ = evt_tx.send(LlmEvent::Chunk {
                            content: delta.clone(),
                        });
                    }
                    // Handle tool call deltas
                    if let Some(ref tc_chunks) = choice.delta.tool_calls {
                        for tc in tc_chunks {
                            let entry = tool_call_chunks
                                .entry(tc.index)
                                .or_insert((None, None, String::new()));
                            if let Some(ref id) = tc.id {
                                entry.0 = Some(id.clone());
                            }
                            if let Some(ref func) = tc.function {
                                if let Some(ref name) = func.name {
                                    entry.1 = Some(name.clone());
                                }
                                if let Some(ref args) = func.arguments {
                                    entry.2.push_str(args);
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let _ = evt_tx.send(LlmEvent::Error {
                    message: format!("stream error: {e}"),
                });
                return;
            }
        }
    }

    // Emit tool calls if the model requested any
    if !tool_call_chunks.is_empty() {
        let mut sorted: Vec<_> = tool_call_chunks.into_iter().collect();
        // Sort by index to preserve the model's intended order
        sorted.sort_by_key(|(idx, _)| *idx);
        let tool_calls: Vec<ToolCall> = sorted
            .into_iter()
            .map(|(_, (id, name, args))| ToolCall {
                id: id.unwrap_or_default(),
                name: name.unwrap_or_default(),
                arguments: args,
            })
            .collect();

        let _ = evt_tx.send(LlmEvent::ToolCallRequest { tool_calls });

        // Also emit Done if there was text content before the tool call
        if !full_text.is_empty() {
            let _ = evt_tx.send(LlmEvent::Done {
                full_text,
                input_tokens,
                output_tokens,
                model: model_name,
            });
        }
    } else {
        let _ = evt_tx.send(LlmEvent::Done {
            full_text,
            input_tokens,
            output_tokens,
            model: model_name,
        });
    }
}
