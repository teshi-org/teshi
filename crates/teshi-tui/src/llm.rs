//! Compatibility adapter for the TUI's channel-based LLM API.
//!
//! HTTP and SSE transport are owned by `teshi-engine`; this module only
//! resolves TUI provider configuration and re-exports the stable handle/event
//! interface consumed by `App`.

use anyhow::Result;

pub use teshi_engine::llm::{ChatMessage, LlmEvent, LlmHandle, LlmRequest, ToolCall};

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub context_window: Option<u32>,
}

impl LlmConfig {
    pub fn from_env() -> Result<Self> {
        let config = teshi_engine::llm::LlmConfig::from_env()?;
        Ok(config.into())
    }

    pub fn is_configured() -> bool {
        teshi_engine::llm::LlmConfig::is_configured()
    }

    pub fn from_provider_config(
        provider_name: &str,
        cfg: &crate::config::ProviderConfig,
    ) -> Result<Self> {
        config_from_provider(provider_name, cfg)
    }
}

impl From<teshi_engine::llm::LlmConfig> for LlmConfig {
    fn from(value: teshi_engine::llm::LlmConfig) -> Self {
        Self {
            api_key: value.api_key,
            base_url: value.base_url,
            model: value.model,
            max_tokens: value.max_tokens,
            temperature: value.temperature,
            context_window: value.context_window,
        }
    }
}

impl From<LlmConfig> for teshi_engine::llm::LlmConfig {
    fn from(value: LlmConfig) -> Self {
        Self {
            api_key: value.api_key,
            base_url: value.base_url,
            model: value.model,
            max_tokens: value.max_tokens,
            temperature: value.temperature,
            context_window: value.context_window,
        }
    }
}

pub fn spawn_llm(config: LlmConfig) -> (LlmHandle, std::sync::mpsc::Receiver<LlmEvent>) {
    teshi_engine::llm::spawn_llm(config.into())
}

pub fn config_from_provider(
    provider_name: &str,
    cfg: &crate::config::ProviderConfig,
) -> Result<LlmConfig> {
    let api_key = cfg.api_key.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "no API key configured for provider '{}'. Run 'teshi auth login --provider {}'.",
            provider_name,
            provider_name
        )
    })?;
    Ok(LlmConfig {
        api_key,
        base_url: cfg
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1".into()),
        model: cfg.model.clone().unwrap_or_else(|| "gpt-4o-mini".into()),
        max_tokens: cfg.max_tokens.unwrap_or(1024),
        temperature: cfg.temperature.unwrap_or(0.7),
        context_window: cfg.context_window,
    })
}
