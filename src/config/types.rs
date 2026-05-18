use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Top-level application configuration.
///
/// Loaded from layered sources (hardcoded defaults → user config →
/// project config → environment variables) and resolved through
/// `${auth:provider}` / `${env:VAR}` placeholder expansion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// The provider to use when none is specified explicitly.
    #[serde(default)]
    pub default_provider: Option<String>,
    /// Provider-specific configuration keyed by provider name.
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

/// Configuration for a single LLM / API provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Base URL for the API endpoint (e.g. `https://api.openai.com/v1`).
    #[serde(default)]
    pub base_url: Option<String>,
    /// Model identifier (e.g. `gpt-4o`, `deepseek-chat`).
    #[serde(default)]
    pub model: Option<String>,
    /// API key. Supports `${auth:name}`, `${env:VAR}`, or plaintext.
    ///
    /// Placeholders are resolved during config loading.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Maximum tokens per request.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Maximum context window size in tokens.
    #[serde(default)]
    pub context_window: Option<u32>,
}

impl AppConfig {
    /// Returns the provider config for `name`, or `None`.
    #[allow(dead_code)]
    pub fn provider(&self, name: &str) -> Option<&ProviderConfig> {
        self.providers.get(name)
    }

    /// Returns the default provider name and config, or `None`.
    pub fn default_provider_config(&self) -> Option<(&str, &ProviderConfig)> {
        let name = self.default_provider.as_deref()?;
        let cfg = self.providers.get(name)?;
        Some((name, cfg))
    }
}
