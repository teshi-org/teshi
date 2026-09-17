//! Configuration adapter for the TUI's native LLM profile selection.
//!
//! HTTP and SSE transport are owned by `teshi-engine`. This module resolves
//! runtime config from the shared
//! model-profile store (with `TESHI_LLM_*` fallback).

use anyhow::Result;

pub use teshi_engine::llm::{ChatMessage, LlmConfig};
#[cfg(test)]
pub use teshi_engine::llm::{LlmEvent, ToolCall};

/// Resolve effective LLM config from the shared profile store, else env.
pub fn effective_config() -> Result<LlmConfig> {
    teshi_engine::effective_llm_config()
}

/// Whether a usable API key is available via active profile or env.
pub fn is_configured() -> bool {
    match effective_config() {
        Ok(cfg) => !cfg.api_key.trim().is_empty(),
        Err(_) => LlmConfig::is_configured(),
    }
}
