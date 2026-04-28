//! LLM integration module — streaming chat completions over HTTP.
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
//! # DeepSeek V4 thinking mode (reasoning_content)
//!
//! The `reasoning_content` field is captured from streaming deltas and must be
//! passed back in subsequent requests when tool calls are involved.

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
        let model = std::env::var("TESHI_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
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
    /// DeepSeek V4 thinking chain — must be preserved across tool-call turns.
    pub reasoning_content: Option<String>,
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
    Chunk { content: String },
    /// The completion finished successfully with a text response.
    Done {
        full_text: String,
        /// DeepSeek V4 thinking chain for this assistant message.
        reasoning_content: Option<String>,
        /// Input + output token usage, if reported.
        #[allow(dead_code)]
        input_tokens: Option<u32>,
        #[allow(dead_code)]
        output_tokens: Option<u32>,
        model: String,
    },
    /// The model requested one or more tool calls instead of a text response.
    ToolCallRequest {
        tool_calls: Vec<ToolCall>,
        /// DeepSeek V4 thinking chain — must be preserved in the assistant
        /// message sent back in follow-up requests.
        reasoning_content: Option<String>,
    },
    /// A non-recoverable error occurred.
    Error { message: String },
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

fn run_llm_worker(config: LlmConfig, req_rx: Receiver<LlmRequest>, evt_tx: Sender<LlmEvent>) {
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

// ── Request body builder ─────────────────────────────────────────────────────

/// Build the JSON request body for a streaming chat completion.
fn build_request_body(
    config: &LlmConfig,
    system: Option<String>,
    messages: &[ChatMessage],
    tools: Option<&[ToolDefinition]>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": config.model,
        "messages": [],
        "max_tokens": config.max_tokens,
        "temperature": config.temperature,
        "stream": true,
    });

    let mut json_messages: Vec<serde_json::Value> = Vec::new();

    if let Some(sys) = system {
        json_messages.push(serde_json::json!({
            "role": "system",
            "content": sys,
        }));
    }

    for msg in messages {
        let mut j = serde_json::json!({
            "role": msg.role,
        });

        // Include content if non-empty, otherwise null
        if msg.content.is_empty() && (msg.tool_calls.is_some() || msg.role == "assistant") {
            j["content"] = serde_json::Value::Null;
        } else {
            j["content"] = serde_json::json!(msg.content);
        }

        if let Some(ref tcs) = msg.tool_calls {
            let tool_calls_json: Vec<serde_json::Value> = tcs
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": tc.arguments,
                        },
                    })
                })
                .collect();
            j["tool_calls"] = serde_json::Value::Array(tool_calls_json);
        }

        if let Some(ref tci) = msg.tool_call_id {
            j["tool_call_id"] = serde_json::json!(tci);
        }

        // DeepSeek V4: preserve reasoning_content in assistant messages
        if let Some(ref rc) = msg.reasoning_content {
            j["reasoning_content"] = serde_json::json!(rc);
        }

        json_messages.push(j);
    }

    body["messages"] = serde_json::Value::Array(json_messages);

    if let Some(tool_defs) = tools {
        let tools_json: Vec<serde_json::Value> = tool_defs
            .iter()
            .map(|td| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": td.name,
                        "description": td.description,
                        "parameters": td.parameters,
                    },
                })
            })
            .collect();
        body["tools"] = serde_json::Value::Array(tools_json);
    }

    body
}

// ── Streaming chat completion ────────────────────────────────────────────────

async fn chat_completion(
    config: &LlmConfig,
    system: Option<String>,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<ToolDefinition>>,
    evt_tx: &Sender<LlmEvent>,
) {
    let request_body = build_request_body(config, system, &messages, tools.as_deref());

    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));

    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(e) => {
            let _ = evt_tx.send(LlmEvent::Error {
                message: format!("failed to build HTTP client: {e}"),
            });
            return;
        }
    };

    let response = match client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .json(&request_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = evt_tx.send(LlmEvent::Error {
                message: format!("HTTP request failed: {e}"),
            });
            return;
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let _ = evt_tx.send(LlmEvent::Error {
            message: format!("API returned {status}: {body}"),
        });
        return;
    }

    // Stream SSE chunks
    let mut full_text = String::new();
    let mut full_reasoning = String::new();
    let mut model_name = String::new();
    let mut input_tokens: Option<u32> = None;
    let mut output_tokens: Option<u32> = None;
    let mut tool_call_chunks: HashMap<u32, (Option<String>, Option<String>, String)> =
        HashMap::new();

    let mut stream = response.bytes_stream();
    let mut buf = String::new();

    use futures::StreamExt;
    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                let _ = evt_tx.send(LlmEvent::Error {
                    message: format!("stream read error: {e}"),
                });
                return;
            }
        };

        buf.push_str(&String::from_utf8_lossy(&chunk));

        // Split by double-newline to get complete SSE events
        while let Some(pos) = buf.find("\n\n") {
            let event = buf[..pos].to_string();
            buf = buf[pos + 2..].to_string();

            // Process this SSE event
            for line in event.lines() {
                let line = line.trim();
                if line.is_empty() || !line.starts_with("data: ") {
                    continue;
                }
                let data = &line[6..]; // strip "data: "
                if data == "[DONE]" {
                    break;
                }
                let v: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if model_name.is_empty()
                    && let Some(m) = v["model"].as_str()
                {
                    model_name = m.to_string();
                }

                if let Some(usage) = v.get("usage") {
                    input_tokens = usage["prompt_tokens"].as_u64().map(|n| n as u32);
                    output_tokens = usage["completion_tokens"].as_u64().map(|n| n as u32);
                }

                for choice in v["choices"].as_array().into_iter().flatten() {
                    let delta = &choice["delta"];

                    // Text content
                    if let Some(text) = delta["content"].as_str() {
                        full_text.push_str(text);
                        let _ = evt_tx.send(LlmEvent::Chunk {
                            content: text.to_string(),
                        });
                    }

                    // DeepSeek V4 reasoning_content — accumulate and preserve
                    if let Some(rc) = delta["reasoning_content"].as_str() {
                        full_reasoning.push_str(rc);
                    }

                    // Tool calls
                    if let Some(tc_array) = delta["tool_calls"].as_array() {
                        for tc in tc_array {
                            let index = tc["index"].as_u64().unwrap_or(0) as u32;
                            let entry = tool_call_chunks.entry(index).or_insert((
                                None,
                                None,
                                String::new(),
                            ));
                            if let Some(id) = tc["id"].as_str() {
                                entry.0 = Some(id.to_string());
                            }
                            if let Some(func) = tc.get("function") {
                                if let Some(name) = func["name"].as_str() {
                                    entry.1 = Some(name.to_string());
                                }
                                if let Some(args) = func["arguments"].as_str() {
                                    entry.2.push_str(args);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Build extracted reasoning_content
    let reasoning: Option<String> = if full_reasoning.is_empty() {
        None
    } else {
        Some(full_reasoning)
    };

    // Emit tool calls if the model requested any
    if !tool_call_chunks.is_empty() {
        let mut sorted: Vec<_> = tool_call_chunks.into_iter().collect();
        sorted.sort_by_key(|(idx, _)| *idx);
        let tool_calls: Vec<ToolCall> = sorted
            .into_iter()
            .map(|(_, (id, name, args))| ToolCall {
                id: id.unwrap_or_default(),
                name: name.unwrap_or_default(),
                arguments: args,
            })
            .collect();

        let _ = evt_tx.send(LlmEvent::ToolCallRequest {
            tool_calls,
            reasoning_content: reasoning,
        });

        // Also emit Done if there was text content before the tool call
        if !full_text.is_empty() {
            let _ = evt_tx.send(LlmEvent::Done {
                full_text,
                reasoning_content: None,
                input_tokens,
                output_tokens,
                model: model_name,
            });
        }
    } else {
        let _ = evt_tx.send(LlmEvent::Done {
            full_text,
            reasoning_content: reasoning,
            input_tokens,
            output_tokens,
            model: model_name,
        });
    }
}
