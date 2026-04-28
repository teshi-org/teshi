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
//! # Usage
//!
//! ```ignore
//! let config = LlmConfig::from_env().unwrap();
//! let (handle, rx) = spawn_llm(config);
//! handle.send(LlmRequest::Chat {
//!     system: Some("You are a helpful assistant.".into()),
//!     messages: vec!["What is BDD?".into()],
//! })?;
//!
//! while let Ok(event) = rx.recv() {
//!     match event {
//!         LlmEvent::Chunk { content, .. } => { /* stream partial */ }
//!         LlmEvent::Done { full_text, .. } => { /* done */ }
//!         LlmEvent::Error { message } => { /* handle */ }
//!     }
//! }
//! ```

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

// ── Request / Event types ────────────────────────────────────────────────────

/// A request sent into the LLM background thread.
#[derive(Debug, Clone)]
pub enum LlmRequest {
    /// A simple chat completion.
    Chat {
        /// Optional system prompt.
        system: Option<String>,
        /// User messages, each sent as a separate user turn.
        messages: Vec<String>,
    },
}

/// An event emitted by the LLM background thread.
#[derive(Debug, Clone)]
pub enum LlmEvent {
    /// A streaming text chunk.
    Chunk {
        content: String,
    },
    /// The completion finished successfully.
    Done {
        full_text: String,
        /// Input + output token usage, if reported.
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        model: String,
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
            LlmRequest::Chat { system, messages } => {
                rt.block_on(process_chat_request(&config, system, messages, &evt_tx));
            }
        }
    }
}

/// Process a chat request with a 120-second timeout.
async fn process_chat_request(
    config: &LlmConfig,
    system: Option<String>,
    messages: Vec<String>,
    evt_tx: &Sender<LlmEvent>,
) {
    let timeout_dur = std::time::Duration::from_secs(120);
    match tokio::time::timeout(
        timeout_dur,
        chat_completion(config, system, messages, evt_tx),
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

// ── Streaming chat completion ────────────────────────────────────────────────

async fn chat_completion(
    config: &LlmConfig,
    system: Option<String>,
    messages: Vec<String>,
    evt_tx: &Sender<LlmEvent>,
) {
    use async_openai::Client;
    use async_openai::config::OpenAIConfig;
    use async_openai::types::chat::{
        ChatCompletionRequestMessage, CreateChatCompletionRequestArgs,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    };
    use futures::StreamExt;

    let openai_config = OpenAIConfig::default()
        .with_api_key(&config.api_key)
        .with_api_base(&config.base_url);

    let client = Client::with_config(openai_config);

    let mut req_messages: Vec<ChatCompletionRequestMessage> = Vec::new();

    if let Some(sys) = system {
        if let Ok(msg) = ChatCompletionRequestSystemMessageArgs::default()
            .content(sys)
            .build()
            .map(|m| ChatCompletionRequestMessage::System(m))
        {
            req_messages.push(msg);
        }
    }

    for msg_text in &messages {
        if let Ok(msg) = ChatCompletionRequestUserMessageArgs::default()
            .content(msg_text.clone())
            .build()
            .map(|m| ChatCompletionRequestMessage::User(m))
        {
            req_messages.push(msg);
        }
    }

    let request = match CreateChatCompletionRequestArgs::default()
        .model(&config.model)
        .messages(req_messages)
        .max_tokens(config.max_tokens)
        .temperature(config.temperature)
        .build()
    {
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

    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                if model_name.is_empty() {
                    model_name = chunk.model.clone();
                }
                // Capture usage from the final chunk if present
                if let Some(usage) = chunk.usage {
                    input_tokens = Some(usage.prompt_tokens as u32);
                    output_tokens = Some(usage.completion_tokens as u32);
                }
                for choice in &chunk.choices {
                    if let Some(ref delta) = choice.delta.content {
                        full_text.push_str(delta);
                        let _ = evt_tx.send(LlmEvent::Chunk {
                            content: delta.clone(),
                        });
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

    let _ = evt_tx.send(LlmEvent::Done {
        full_text,
        input_tokens,
        output_tokens,
        model: model_name,
    });
}
