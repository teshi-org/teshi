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

/// Persist a flat update onto the active model profile.
///
/// Creates a `Default` profile when none exists. Also mirrors to legacy
/// `llm-config.json` for older tooling.
pub fn save_stored_llm_config(update: &LlmConfigWrite) -> Result<StoredLlmConfig> {
    crate::model_profile::ensure_migrated()?;
    let mut profile = match crate::model_profile::load_active_profile()? {
        Some(p) => p,
        None => {
            let mut p = crate::model_profile::ModelProfile::new("Default");
            let saved = crate::model_profile::save_profile(&mut p)?;
            crate::model_profile::load_profile(&saved.id)?
        }
    };
    profile.base_url = update.base_url.clone();
    profile.model_id = update.model.clone();
    if !update.api_key.is_empty() {
        profile.api_key = update.api_key.clone();
    }
    crate::model_profile::save_profile(&mut profile)?;

    let stored = StoredLlmConfig {
        base_url: profile.base_url.clone(),
        model: profile.model_id.clone(),
        api_key: profile.api_key.clone(),
    };
    let path = store_path()?;
    write_legacy_mirror(&path, &stored)?;
    Ok(stored)
}

fn write_legacy_mirror(path: &std::path::Path, stored: &StoredLlmConfig) -> Result<()> {
    let json = serde_json::to_string_pretty(stored)?;
    fs::write(path, json).with_context(|| format!("write {}", path.display()))
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

/// Load the public flat projection of the **active** model profile.
///
/// Falls back to legacy `llm-config.json` when no profiles exist yet.
pub fn load_llm_config_public() -> Result<LlmConfigPublic> {
    crate::model_profile::ensure_migrated()?;
    if let Some(profile) = crate::model_profile::load_active_profile()? {
        return Ok(to_public_flat_from_profile(&profile));
    }
    Ok(to_public(&load_stored_llm_config()?))
}

fn to_public_flat_from_profile(profile: &crate::model_profile::ModelProfile) -> LlmConfigPublic {
    let configured = !profile.api_key.is_empty();
    LlmConfigPublic {
        base_url: profile.base_url.clone(),
        model: profile.model_id.clone(),
        api_key_configured: configured,
        api_key_masked: if configured {
            mask_api_key(&profile.api_key)
        } else {
            String::new()
        },
    }
}

/// Effective config for engine LLM calls: active model profile, else `TESHI_LLM_*` env.
///
/// Reads the shared on-disk profile store on every call so updates made by
/// another desktop or daemon process become effective without restarting.
/// When no active profile has a usable API key, falls back to env vars.
pub fn effective_llm_config() -> Result<crate::llm::LlmConfig> {
    let active = crate::model_profile::load_active_profile()?;
    effective_llm_config_from_profile(active, crate::llm::LlmConfig::from_env)
}

fn effective_llm_config_from_profile<F>(
    active: Option<crate::model_profile::ModelProfile>,
    env_config: F,
) -> Result<crate::llm::LlmConfig>
where
    F: FnOnce() -> Result<crate::llm::LlmConfig>,
{
    if let Some(profile) = active {
        if !profile.api_key.is_empty() {
            return crate::model_profile::profile_to_llm_config(&profile);
        }
    }
    env_config()
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

    #[test]
    fn test_flat_projection_from_profile_fields() {
        let profile = crate::model_profile::ModelProfile {
            id: "x".into(),
            name: "Default".into(),
            provider: crate::model_profile::PROVIDER_OPENAI.into(),
            api_style: crate::model_profile::ApiStyle::ChatCompletions,
            model_id: "proj-model".into(),
            max_context_tokens: None,
            max_output_tokens: 1024,
            base_url: "https://proj.example/v1".into(),
            api_key: "sk-projkey".into(),
            stream: true,
            http_headers: Default::default(),
            chat_options: Default::default(),
        };
        let public = to_public_flat_from_profile(&profile);
        assert_eq!(public.base_url, "https://proj.example/v1");
        assert_eq!(public.model, "proj-model");
        assert!(public.api_key_configured);
        assert!(!public.api_key_masked.contains("sk-projkey"));
    }

    #[test]
    fn profile_without_key_uses_environment_fallback_not_legacy() {
        let profile = crate::model_profile::ModelProfile {
            id: "active".into(),
            name: "Anthropic".into(),
            provider: crate::model_profile::PROVIDER_ANTHROPIC.into(),
            api_style: crate::model_profile::ApiStyle::ChatCompletions,
            model_id: "claude-test".into(),
            max_context_tokens: None,
            max_output_tokens: 200,
            base_url: String::new(),
            api_key: String::new(),
            stream: false,
            http_headers: Default::default(),
            chat_options: Default::default(),
        };
        let resolved = effective_llm_config_from_profile(Some(profile), || {
            Ok(crate::llm::LlmConfig {
                api_key: "env-key".into(),
                base_url: "https://env.example/v1".into(),
                model: "env-model".into(),
                max_tokens: 99,
                temperature: 0.1,
                context_window: None,
                provider: crate::model_profile::PROVIDER_OPENAI.into(),
                api_style: crate::model_profile::ApiStyle::ChatCompletions,
                stream: true,
                http_headers: Default::default(),
                chat_options: Default::default(),
            })
        })
        .unwrap();
        assert_eq!(resolved.api_key, "env-key");
        assert_eq!(resolved.model, "env-model");
    }

    #[test]
    fn legacy_mirror_write_failures_are_propagated() {
        let tmp = tempfile::TempDir::new().unwrap();
        let stored = StoredLlmConfig {
            base_url: "https://example.test/v1".into(),
            model: "test".into(),
            api_key: "secret".into(),
        };
        let err = write_legacy_mirror(tmp.path(), &stored).unwrap_err();
        assert!(err.to_string().contains("write"));
    }
}
