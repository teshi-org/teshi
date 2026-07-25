//! Shared LLM transport — chat completions (and later Responses / Anthropic).
//!
//! This module provides a minimal, non-blocking interface for sending prompts
//! to configured LLM providers and receiving completions as [`LlmEvent`]s. It
//! follows the same background-thread + channel pattern as `runner.rs` so the
//! synchronous TUI event loop never blocks on network I/O.
//!
//! Streaming is the default; when `LlmConfig::stream` is false, a one-shot JSON
//! response is synthesized into the same `Chunk` / `ToolCallRequest` / `Done`
//! event sequence.
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
//!
//! Pure DTOs (`ChatMessage`, `ToolDefinition`, `ToolCall`) live in `teshi-core::llm`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

pub use teshi_core::llm::{ChatMessage, ToolCall, ToolDefinition};

use crate::model_profile::{
    ApiStyle, PROVIDER_ANTHROPIC, PROVIDER_DEEPSEEK_OPENAI, PROVIDER_OPENAI,
};

pub fn llm_config_from_env() -> Result<(String, String, String), String> {
    let api_key = std::env::var("TESHI_LLM_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .map_err(|_| {
            "TESHI_LLM_API_KEY or OPENAI_API_KEY environment variable not set".to_string()
        })?;
    let base_url = std::env::var("TESHI_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let model = std::env::var("TESHI_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    Ok((api_key, base_url, model))
}

#[allow(clippy::too_many_arguments)]
pub async fn call_llm_with_tool(
    api_key: &str,
    base_url: &str,
    model: &str,
    system: &str,
    user: &str,
    tool_name: &str,
    tool_description: &str,
    tool_parameters: Value,
) -> Result<Value, String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ],
        "tools": [{"type": "function", "function": {
            "name": tool_name,
            "description": tool_description,
            "parameters": tool_parameters
        }}],
        "tool_choice": {"type": "function", "function": {"name": tool_name}},
        "temperature": 0.2,
        "max_tokens": 16384
    });
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("LLM HTTP request failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("LLM API returned status {status}: {error_text}"));
    }
    let response: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse LLM response: {e}"))?;
    let message = response["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .map(|choice| &choice["message"])
        .ok_or_else(|| "LLM response missing a choice message".to_string())?;
    let arguments = message["tool_calls"]
        .as_array()
        .and_then(|calls| calls.first())
        .and_then(|call| call["function"]["arguments"].as_str())
        .ok_or_else(|| "LLM response missing tool call arguments".to_string())?;
    serde_json::from_str(arguments).map_err(|e| format!("Failed to parse tool call arguments: {e}"))
}

/// Call the provider-aware transport and return the selected tool's arguments.
///
/// This is intended for one-shot daemon operations that require a structured
/// tool result while still honoring the active profile's provider, API style,
/// streaming mode, extra headers, and body options.
#[allow(clippy::too_many_arguments)]
pub async fn call_llm_with_tool_config(
    mut config: LlmConfig,
    system: &str,
    user: &str,
    tool_name: &str,
    tool_description: &str,
    tool_parameters: Value,
) -> Result<Value, String> {
    let tool_choice = if config.provider == PROVIDER_ANTHROPIC {
        json!({"type": "tool", "name": tool_name})
    } else if config.provider == PROVIDER_OPENAI && config.api_style == ApiStyle::Responses {
        json!({"type": "function", "name": tool_name})
    } else {
        json!({"type": "function", "function": {"name": tool_name}})
    };
    config
        .chat_options
        .insert("tool_choice".into(), tool_choice);

    let request = LlmRequest::Chat {
        system: Some(system.to_string()),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: user.to_string(),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        }],
        tools: Some(vec![ToolDefinition {
            name: tool_name.to_string(),
            description: tool_description.to_string(),
            parameters: tool_parameters,
        }]),
    };
    let expected_tool_name = tool_name.to_string();

    tokio::task::spawn_blocking(move || {
        let (handle, events) = spawn_llm(config);
        handle
            .send(request)
            .map_err(|e| format!("failed to submit LLM request: {e:#}"))?;

        for event in events {
            match event {
                LlmEvent::Chunk { .. } => {}
                LlmEvent::ToolCallRequest { tool_calls, .. } => {
                    let tool_call = tool_calls
                        .into_iter()
                        .find(|call| call.name == expected_tool_name)
                        .ok_or_else(|| {
                            format!("LLM response did not call tool '{expected_tool_name}'")
                        })?;
                    return serde_json::from_str(&tool_call.arguments)
                        .map_err(|e| format!("Failed to parse tool call arguments as JSON: {e}"));
                }
                LlmEvent::Done { .. } => {
                    return Err(format!(
                        "LLM response completed without calling tool '{expected_tool_name}'"
                    ));
                }
                LlmEvent::Error { message } => return Err(message),
            }
        }
        Err("LLM worker exited without a terminal event".to_string())
    })
    .await
    .map_err(|e| format!("LLM worker task failed: {e}"))?
}

// ── Configuration ────────────────────────────────────────────────────────────

/// LLM client configuration from a model profile or environment variables.
///
/// | Variable | Required | Default |
/// |---|---|---|
/// | `TESHI_LLM_API_KEY` | Yes (env path) | — |
/// | `TESHI_LLM_BASE_URL` | No | `https://api.openai.com/v1` |
/// | `TESHI_LLM_MODEL` | No | `gpt-4o-mini` |
/// | `TESHI_LLM_MAX_TOKENS` | No | `1024` |
/// | `TESHI_LLM_TEMPERATURE` | No | `0.7` |
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// API key for the configured provider.
    pub api_key: String,
    /// Resolved API base URL (provider default applied when profile base was empty).
    pub base_url: String,
    /// Model identifier.
    pub model: String,
    /// Maximum generation tokens.
    pub max_tokens: u32,
    /// Sampling temperature (chat completions).
    pub temperature: f32,
    /// Optional context window hint.
    #[allow(dead_code)]
    pub context_window: Option<u32>,
    /// Built-in provider id (`openai`, `anthropic`, `deepseek-openai`).
    pub provider: String,
    /// Effective API style for routing.
    pub api_style: ApiStyle,
    /// When true, use the provider streaming protocol.
    pub stream: bool,
    /// Extra HTTP headers merged into outbound requests.
    pub http_headers: HashMap<String, String>,
    /// Extra JSON body fields shallow-merged into the request (core fields win).
    pub chat_options: HashMap<String, Value>,
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
        let context_window = std::env::var("TESHI_LLM_CONTEXT_WINDOW")
            .ok()
            .and_then(|v| v.parse().ok());
        Ok(Self {
            api_key,
            base_url,
            model,
            max_tokens,
            temperature,
            context_window,
            provider: PROVIDER_OPENAI.into(),
            api_style: ApiStyle::ChatCompletions,
            stream: true,
            http_headers: HashMap::new(),
            chat_options: HashMap::new(),
        })
    }

    /// Check whether the required env-var is present (without returning config).
    pub fn is_configured() -> bool {
        std::env::var("TESHI_LLM_API_KEY").is_ok()
    }
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
    cancel: Arc<AtomicBool>,
}

impl LlmHandle {
    /// Send a request to the LLM background thread.
    pub fn send(&self, request: LlmRequest) -> Result<()> {
        self.tx
            .send(request)
            .context("LLM background thread has exited")
    }

    /// Request cancellation of the currently in-progress request.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
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
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    thread::Builder::new()
        .name("teshi-llm".into())
        .spawn(move || run_llm_worker(config, req_rx, evt_tx, cancel_clone))
        .expect("failed to spawn LLM worker thread");

    (LlmHandle { tx: req_tx, cancel }, evt_rx)
}

// ── Background worker ────────────────────────────────────────────────────────

fn run_llm_worker(
    config: LlmConfig,
    req_rx: Receiver<LlmRequest>,
    evt_tx: Sender<LlmEvent>,
    cancel: Arc<AtomicBool>,
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
        // Reset cancel flag for new request
        cancel.store(false, Ordering::SeqCst);
        match request {
            LlmRequest::Chat {
                system,
                messages,
                tools,
            } => {
                rt.block_on(process_chat_request(
                    &config, system, messages, tools, &evt_tx, &cancel,
                ));
            }
        }
    }
}

/// Process a chat request with automatic retry on transient errors.
///
/// Retries up to 3 times with exponential backoff for transient errors
/// (connection failures, timeouts, 5xx). Non-transient errors (4xx,
/// client build failure, malformed provider payload) are not retried and are
/// reported here as `LlmEvent::Error` if the transport returned an error.
async fn process_chat_request(
    config: &LlmConfig,
    system: Option<String>,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<ToolDefinition>>,
    evt_tx: &Sender<LlmEvent>,
    cancel: &Arc<AtomicBool>,
) {
    const MAX_RETRIES: u32 = 5;
    const BACKOFF_MS: [u64; 5] = [3_000, 7_000, 15_000, 30_000, 60_000];
    let timeout_dur = std::time::Duration::from_secs(120);

    let mut last_error: Option<String> = None;

    for attempt in 0..MAX_RETRIES {
        if attempt > 0 {
            // Check cancel before sleeping
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            // Interruptible sleep with deterministic jitter (±20%)
            let base = BACKOFF_MS[(attempt - 1) as usize];
            let delay = jitter_ms(base, attempt);
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(delay);
            loop {
                if cancel.load(Ordering::SeqCst) {
                    return;
                }
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200).min(remaining)).await;
            }
        }

        let result = tokio::time::timeout(
            timeout_dur,
            chat_completion(
                config,
                system.clone(),
                messages.clone(),
                tools.clone(),
                evt_tx,
                cancel,
            ),
        )
        .await;

        match result {
            Ok(Ok(())) => {
                // Success
                return;
            }
            Ok(Err(err)) => {
                // chat_completion returned an error
                let err_msg = format!("{err:#}");
                if !is_transient_error(&err_msg) {
                    let _ = evt_tx.send(LlmEvent::Error { message: err_msg });
                    return;
                }
                last_error = Some(err_msg);
            }
            Err(_elapsed) => {
                // Timeout is transient
                last_error = Some(format!(
                    "API call timed out after {}s (attempt {}/{})",
                    timeout_dur.as_secs(),
                    attempt + 1,
                    MAX_RETRIES
                ));
            }
        }
    }

    // All retries exhausted
    let final_err = last_error.unwrap_or_else(|| "unknown error".to_string());
    let _ = evt_tx.send(LlmEvent::Error {
        message: format!("All {MAX_RETRIES} retries failed. Last error: {final_err}"),
    });
}

/// Returns true if the error message indicates a transient (retryable) failure.
fn is_transient_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();

    // Connection/network errors
    if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("refused")
        || lower.contains("reset by peer")
        || lower.contains("broken pipe")
        || lower.contains("eof")
        || lower.contains("stream end")
        || lower.contains("channel closed")
        || lower.contains("incomplete chunked")
    {
        return true;
    }

    // HTTP status codes
    if lower.contains("429") {
        return true;
    } // rate limit
    if lower.contains("500") {
        return true;
    }
    if lower.contains("502") {
        return true;
    }
    if lower.contains("503") {
        return true;
    }
    if lower.contains("504") {
        return true;
    }
    if lower.contains("529") {
        return true;
    } // overloaded

    // SDK-specific error messages
    if lower.contains("rate limit") {
        return true;
    }
    if lower.contains("overloaded") {
        return true;
    }
    if lower.contains("service unavailable") {
        return true;
    }
    if lower.contains("internal server error") {
        return true;
    }

    false
}

/// Deterministic jitter (±20%) for retry backoff.
///
/// Produces a value in [base - offset, base + offset] where
/// offset = base / 5, using a simple pseudo-random function
/// keyed by attempt number.
fn jitter_ms(base: u64, attempt: u32) -> u64 {
    let offset = base / 5;
    let pseudo = (attempt as u64 * 7 + 13) % (offset * 2 + 1);
    let jitter = if pseudo > offset { offset } else { pseudo };
    base + jitter - offset / 2
}

// ── Request body builder ─────────────────────────────────────────────────────

/// Shallow-merge `chat_options` under `body`, then re-apply `core` so required
/// fields always win over conflicting option keys.
pub(crate) fn merge_chat_options(
    body: &mut Value,
    chat_options: &HashMap<String, Value>,
    core: &Map<String, Value>,
) {
    if let Some(obj) = body.as_object_mut() {
        for (k, v) in chat_options {
            obj.insert(k.clone(), v.clone());
        }
        for (k, v) in core {
            obj.insert(k.clone(), v.clone());
        }
    }
}

/// Apply profile HTTP extras onto a reqwest request builder.
pub(crate) fn apply_extra_headers(
    mut builder: reqwest::RequestBuilder,
    headers: &HashMap<String, String>,
) -> reqwest::RequestBuilder {
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder
}

/// Build the JSON request body for a chat completion (stream or not).
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
        "stream": config.stream,
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

    // Core fields must win over chat_options keys such as `model` / `stream`.
    let mut core = Map::new();
    core.insert("model".into(), Value::String(config.model.clone()));
    core.insert("messages".into(), body["messages"].clone());
    core.insert("max_tokens".into(), json!(config.max_tokens));
    core.insert("stream".into(), Value::Bool(config.stream));
    if body.get("tools").is_some() {
        core.insert("tools".into(), body["tools"].clone());
    }
    merge_chat_options(&mut body, &config.chat_options, &core);

    body
}

/// Whether this config should use OpenAI-compatible `/chat/completions`.
fn uses_chat_completions(config: &LlmConfig) -> bool {
    match config.provider.as_str() {
        PROVIDER_DEEPSEEK_OPENAI => true,
        PROVIDER_OPENAI => config.api_style == ApiStyle::ChatCompletions,
        PROVIDER_ANTHROPIC => false,
        _ => config.api_style == ApiStyle::ChatCompletions,
    }
}

// ── Streaming chat completion ────────────────────────────────────────────────

/// Execute a chat request for the configured provider/style.
///
/// On transient errors (connection failure, stream read error, timeout, 5xx),
/// returns `Err(...)` so the caller can retry. On non-transient errors
/// (HTTP client build failure, 4xx), an `LlmEvent::Error` is emitted and
/// `Ok(())` is returned to signal that the caller should not retry.
async fn chat_completion(
    config: &LlmConfig,
    system: Option<String>,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<ToolDefinition>>,
    evt_tx: &Sender<LlmEvent>,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    if config.provider == PROVIDER_ANTHROPIC {
        return crate::llm_anthropic::anthropic_messages_request(
            config, system, messages, tools, evt_tx, cancel,
        )
        .await;
    }
    if config.provider == PROVIDER_OPENAI && config.api_style == ApiStyle::Responses {
        return crate::llm_responses::responses_request(
            config, system, messages, tools, evt_tx, cancel,
        )
        .await;
    }
    if uses_chat_completions(config) {
        return chat_completions_request(config, system, messages, tools, evt_tx, cancel).await;
    }

    let _ = evt_tx.send(LlmEvent::Error {
        message: format!(
            "LLM transport for provider '{}' / api_style {:?} is not supported",
            config.provider, config.api_style
        ),
    });
    Ok(())
}

async fn chat_completions_request(
    config: &LlmConfig,
    system: Option<String>,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<ToolDefinition>>,
    evt_tx: &Sender<LlmEvent>,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let request_body = build_request_body(config, system, &messages, tools.as_deref());
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));

    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(e) => {
            let _ = evt_tx.send(LlmEvent::Error {
                message: format!("failed to build HTTP client: {e}"),
            });
            return Ok(());
        }
    };

    let mut builder = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json");
    if config.stream {
        builder = builder.header("Accept", "text/event-stream");
    } else {
        builder = builder.header("Accept", "application/json");
    }
    builder = apply_extra_headers(builder, &config.http_headers);

    let response = match builder.json(&request_body).send().await {
        Ok(r) => r,
        Err(e) => {
            return Err(anyhow::anyhow!("HTTP request failed: {e}"));
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let err_msg = format!("API returned {status}: {body}");
        if status.is_server_error() {
            return Err(anyhow::anyhow!("{err_msg}"));
        }
        let _ = evt_tx.send(LlmEvent::Error { message: err_msg });
        return Ok(());
    }

    if config.stream {
        read_chat_completions_sse(response, evt_tx, cancel).await
    } else {
        let body: Value = response
            .json()
            .await
            .context("parse non-streaming chat completions JSON")?;
        emit_chat_completions_json(&body, evt_tx);
        Ok(())
    }
}

/// Emit `LlmEvent`s from a non-streaming chat-completions JSON body.
pub(crate) fn emit_chat_completions_json(body: &Value, evt_tx: &Sender<LlmEvent>) {
    let model_name = body["model"].as_str().unwrap_or("").to_string();
    let input_tokens = body["usage"]["prompt_tokens"].as_u64().map(|n| n as u32);
    let output_tokens = body["usage"]["completion_tokens"]
        .as_u64()
        .map(|n| n as u32);

    let message = body["choices"]
        .as_array()
        .and_then(|c| c.first())
        .map(|c| &c["message"]);

    let Some(message) = message else {
        let _ = evt_tx.send(LlmEvent::Error {
            message: "chat completions response missing choices[0].message".into(),
        });
        return;
    };

    let reasoning = message["reasoning_content"]
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());

    if let Some(tc_array) = message["tool_calls"].as_array() {
        if !tc_array.is_empty() {
            let tool_calls: Vec<ToolCall> = tc_array
                .iter()
                .map(|tc| ToolCall {
                    id: tc["id"].as_str().unwrap_or("").to_string(),
                    name: tc["function"]["name"].as_str().unwrap_or("").to_string(),
                    arguments: tc["function"]["arguments"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    execution_duration_ms: None,
                })
                .collect();
            let _ = evt_tx.send(LlmEvent::ToolCallRequest {
                tool_calls,
                reasoning_content: reasoning,
            });
            return;
        }
    }

    let full_text = message["content"].as_str().unwrap_or("").to_string();
    if !full_text.is_empty() {
        let _ = evt_tx.send(LlmEvent::Chunk {
            content: full_text.clone(),
        });
    }
    let _ = evt_tx.send(LlmEvent::Done {
        full_text,
        reasoning_content: reasoning,
        input_tokens,
        output_tokens,
        model: model_name,
    });
}

async fn read_chat_completions_sse(
    response: reqwest::Response,
    evt_tx: &Sender<LlmEvent>,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
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
    const CHUNK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

    loop {
        // Check for cancellation
        if cancel.load(Ordering::SeqCst) {
            let _ = evt_tx.send(LlmEvent::Error {
                message: "Request cancelled by user".to_string(),
            });
            return Ok(());
        }

        let next_chunk = tokio::time::timeout(CHUNK_TIMEOUT, stream.next()).await;

        let chunk = match next_chunk {
            Ok(Some(Ok(c))) => c,
            Ok(Some(Err(e))) => {
                return Err(anyhow::anyhow!("stream read error: {e}"));
            }
            Ok(None) => break, // Stream ended normally
            Err(_elapsed) => {
                return Err(anyhow::anyhow!(
                    "response stream stalled: no data received for {}s",
                    CHUNK_TIMEOUT.as_secs()
                ));
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

                if model_name.is_empty() {
                    if let Some(m) = v["model"].as_str() {
                        model_name = m.to_string();
                    }
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
                execution_duration_ms: None,
            })
            .collect();

        let _ = evt_tx.send(LlmEvent::ToolCallRequest {
            tool_calls,
            reasoning_content: reasoning,
        });

        // Text content before the tool call was already sent as Chunk
        // events and stored in ai_partial_response — no separate Done needed.
    } else {
        let _ = evt_tx.send(LlmEvent::Done {
            full_text,
            reasoning_content: reasoning,
            input_tokens,
            output_tokens,
            model: model_name,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn base_config() -> LlmConfig {
        LlmConfig {
            api_key: "sk-test".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o-mini".into(),
            max_tokens: 100,
            temperature: 0.5,
            context_window: None,
            provider: PROVIDER_OPENAI.into(),
            api_style: ApiStyle::ChatCompletions,
            stream: true,
            http_headers: HashMap::new(),
            chat_options: HashMap::new(),
        }
    }

    #[test]
    fn test_chat_options_do_not_override_model() {
        let mut config = base_config();
        config
            .chat_options
            .insert("model".into(), Value::String("should-not-win".into()));
        config.chat_options.insert("top_p".into(), json!(0.9));
        let body = build_request_body(&config, None, &[], None);
        assert_eq!(body["model"], "gpt-4o-mini");
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn test_stream_false_in_request_body() {
        let mut config = base_config();
        config.stream = false;
        let body = build_request_body(&config, None, &[], None);
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn test_deepseek_uses_chat_completions() {
        let mut config = base_config();
        config.provider = PROVIDER_DEEPSEEK_OPENAI.into();
        config.api_style = ApiStyle::Responses;
        assert!(uses_chat_completions(&config));
    }

    #[test]
    fn test_emit_non_stream_done_with_reasoning() {
        let (tx, rx) = mpsc::channel();
        let body = json!({
            "model": "deepseek-chat",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "hello",
                    "reasoning_content": "think"
                }
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 2 }
        });
        emit_chat_completions_json(&body, &tx);
        let events: Vec<_> = rx.try_iter().collect();
        assert!(matches!(events[0], LlmEvent::Chunk { .. }));
        match &events[1] {
            LlmEvent::Done {
                full_text,
                reasoning_content,
                model,
                ..
            } => {
                assert_eq!(full_text, "hello");
                assert_eq!(reasoning_content.as_deref(), Some("think"));
                assert_eq!(model, "deepseek-chat");
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn test_emit_non_stream_tool_calls() {
        let (tx, rx) = mpsc::channel();
        let body = json!({
            "model": "gpt-4o-mini",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "search", "arguments": "{\"q\":1}" }
                    }]
                }
            }]
        });
        emit_chat_completions_json(&body, &tx);
        let events: Vec<_> = rx.try_iter().collect();
        match &events[0] {
            LlmEvent::ToolCallRequest { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name, "search");
            }
            other => panic!("expected ToolCallRequest, got {other:?}"),
        }
    }

    #[test]
    fn test_merge_chat_options_core_wins() {
        let mut body = json!({ "model": "a", "stream": true });
        let mut opts = HashMap::new();
        opts.insert("model".into(), Value::String("b".into()));
        opts.insert("foo".into(), json!(1));
        let mut core = Map::new();
        core.insert("model".into(), Value::String("a".into()));
        core.insert("stream".into(), Value::Bool(true));
        merge_chat_options(&mut body, &opts, &core);
        assert_eq!(body["model"], "a");
        assert_eq!(body["foo"], 1);
    }

    #[tokio::test]
    async fn test_extra_http_headers_injected() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.expect("read");
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = br#"{"id":"1","model":"gpt-4o-mini","choices":[{"message":{"role":"assistant","content":"ok"}}]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                String::from_utf8_lossy(body)
            );
            socket.write_all(resp.as_bytes()).await.expect("write");
            req
        });

        let mut config = base_config();
        config.base_url = format!("http://{addr}/v1");
        config.stream = false;
        config.http_headers.insert("X-Test".into(), "1".into());

        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        chat_completions_request(&config, None, vec![], None, &tx, &cancel)
            .await
            .expect("request");

        let captured = server.await.expect("join");
        assert!(
            captured.to_lowercase().contains("x-test: 1"),
            "missing X-Test header in:\n{captured}"
        );
        let events: Vec<_> = rx.try_iter().collect();
        assert!(events.iter().any(|e| matches!(e, LlmEvent::Done { .. })));
    }

    #[tokio::test]
    async fn provider_aware_tool_call_uses_responses_transport() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.expect("read");
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = br#"{"id":"resp_1","model":"gpt-4.1","output":[{"type":"function_call","call_id":"call_1","name":"generate","arguments":"{\"ok\":true}"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                String::from_utf8_lossy(body)
            );
            socket.write_all(response.as_bytes()).await.expect("write");
            request
        });

        let mut config = base_config();
        config.base_url = format!("http://{addr}/v1");
        config.api_style = ApiStyle::Responses;
        config.stream = false;
        config
            .http_headers
            .insert("X-Profile".into(), "active".into());
        let result = call_llm_with_tool_config(
            config,
            "system",
            "user",
            "generate",
            "Generate data",
            json!({"type": "object"}),
        )
        .await
        .expect("tool result");
        assert_eq!(result["ok"], true);

        let request = server.await.expect("join");
        assert!(
            request.contains("POST /v1/responses"),
            "request:\n{request}"
        );
        assert!(
            request.to_ascii_lowercase().contains("x-profile: active"),
            "request:\n{request}"
        );
    }

    #[tokio::test]
    async fn malformed_non_stream_payloads_emit_errors_for_all_transports() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for (provider, style) in [
            (PROVIDER_OPENAI, ApiStyle::ChatCompletions),
            (PROVIDER_OPENAI, ApiStyle::Responses),
            (PROVIDER_ANTHROPIC, ApiStyle::ChatCompletions),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.expect("accept");
                let mut request = vec![0u8; 8192];
                let _ = socket.read(&mut request).await.expect("read");
                let body = "not-json";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.expect("write");
            });

            let mut config = base_config();
            config.provider = provider.into();
            config.api_style = style;
            config.base_url = format!("http://{addr}");
            config.stream = false;
            let (tx, rx) = mpsc::channel();
            let cancel = Arc::new(AtomicBool::new(false));
            process_chat_request(&config, None, vec![], None, &tx, &cancel).await;
            server.await.expect("join");

            let events: Vec<_> = rx.try_iter().collect();
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, LlmEvent::Error { .. })),
                "{provider}/{style:?} emitted no error: {events:?}"
            );
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, LlmEvent::Done { .. })),
                "{provider}/{style:?} reported malformed JSON as done"
            );
        }
    }
}
