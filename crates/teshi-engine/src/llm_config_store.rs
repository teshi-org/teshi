//! User-level LLM configuration store (`llm-config.json` under app data).

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::app_data::app_data_dir;

/// On-disk LLM settings (full API key; never log this struct).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoredLlmConfig {
    /// OpenAI-compatible API base URL.
    #[serde(default)]
    pub base_url: String,
    /// Model id.
    #[serde(default)]
    pub model: String,
    /// API key plaintext for local daemon trust model.
    #[serde(default)]
    pub api_key: String,
}

/// Public snapshot with a masked API key.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfigPublic {
    pub base_url: String,
    pub model: String,
    pub api_key_configured: bool,
    #[serde(default)]
    pub api_key_masked: String,
}

/// Update payload from the GPUI shell / HTTP API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfigWrite {
    pub base_url: String,
    pub model: String,
    /// When empty and a key already exists, the previous key is preserved.
    #[serde(default)]
    pub api_key: String,
}

fn store_path() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("llm-config.json"))
}

/// Mask an API key for UI/API responses (last up to 4 characters).
pub fn mask_api_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let suffix: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("…{suffix}")
}

/// Load the stored config, or defaults when the file is missing.
pub fn load_stored_llm_config() -> Result<StoredLlmConfig> {
    let path = store_path()?;
    if !path.exists() {
        return Ok(StoredLlmConfig::default());
    }
    let content = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))
}

/// Persist LLM config to disk.
pub fn save_stored_llm_config(update: &LlmConfigWrite) -> Result<StoredLlmConfig> {
    let mut stored = load_stored_llm_config()?;
    stored.base_url = update.base_url.clone();
    stored.model = update.model.clone();
    if !update.api_key.is_empty() {
        stored.api_key = update.api_key.clone();
    }
    let path = store_path()?;
    let json = serde_json::to_string_pretty(&stored)?;
    fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(stored)
}

/// Convert stored config to a public (masked) snapshot.
pub fn to_public(stored: &StoredLlmConfig) -> LlmConfigPublic {
    let configured = !stored.api_key.is_empty();
    LlmConfigPublic {
        base_url: stored.base_url.clone(),
        model: stored.model.clone(),
        api_key_configured: configured,
        api_key_masked: if configured {
            mask_api_key(&stored.api_key)
        } else {
            String::new()
        },
    }
}

/// Load and return the public snapshot.
pub fn load_llm_config_public() -> Result<LlmConfigPublic> {
    Ok(to_public(&load_stored_llm_config()?))
}

/// Effective config for engine LLM calls: file store, else `TESHI_LLM_*` env.
///
/// Read the shared on-disk store on every call so updates made by another
/// desktop or daemon process become effective without restarting this process.
pub fn effective_llm_config() -> Result<crate::llm::LlmConfig> {
    let stored = load_stored_llm_config()?;
    if !stored.api_key.is_empty() {
        return Ok(stored_to_llm_config(&stored));
    }
    crate::llm::LlmConfig::from_env()
}

fn stored_to_llm_config(stored: &StoredLlmConfig) -> crate::llm::LlmConfig {
    crate::llm::LlmConfig {
        api_key: stored.api_key.clone(),
        base_url: if stored.base_url.is_empty() {
            "https://api.openai.com/v1".into()
        } else {
            stored.base_url.clone()
        },
        model: if stored.model.is_empty() {
            "gpt-4o-mini".into()
        } else {
            stored.model.clone()
        },
        max_tokens: 1024,
        temperature: 0.7,
        context_window: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_api_key_shows_suffix() {
        assert_eq!(mask_api_key(""), "");
        assert_eq!(mask_api_key("sk-abcdefgh"), "…efgh");
        assert_eq!(mask_api_key("ab"), "…ab");
    }
}
