//! One-time import of legacy TUI LLM config into the shared model-profile store.
//!
//! Sources (OS config dir `teshi/`):
//! - `models/*.toml` — TUI model panel profiles
//! - `config.toml` `[providers.*]` + `auth.json` — provider table + credential store
//!
//! Only runs when the destination `model-profiles/` directory has no profiles.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use crate::model_profile::{
    generate_id, save_profile_in, set_active_id_in, ApiStyle, ModelProfile, PROVIDER_ANTHROPIC,
    PROVIDER_DEEPSEEK_OPENAI, PROVIDER_OPENAI,
};

const TUI_IMPORT_MARKER: &str = ".migrated-from-tui-config";

#[derive(Debug, Deserialize)]
struct LegacyTomlProfile {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default = "default_max_tokens")]
    max_tokens: u32,
    #[serde(default = "default_temperature")]
    temperature: f32,
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_temperature() -> f32 {
    0.7
}

#[derive(Debug, Default, Deserialize)]
struct LegacyAppConfig {
    #[serde(default)]
    default_provider: Option<String>,
    #[serde(default)]
    providers: HashMap<String, LegacyProviderConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyProviderConfig {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    context_window: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct CredentialEntry {
    #[serde(default)]
    key: String,
}

/// Map a legacy free-form provider name to a built-in engine provider id.
pub fn map_legacy_provider_id(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "openai" => PROVIDER_OPENAI.into(),
        "anthropic" | "claude" => PROVIDER_ANTHROPIC.into(),
        "deepseek" | "deepseek-openai" => PROVIDER_DEEPSEEK_OPENAI.into(),
        // Ollama and other OpenAI-compatible endpoints use the openai transport.
        _ => PROVIDER_OPENAI.into(),
    }
}

/// Default config directory used by the legacy TUI (`dirs::config_dir()/teshi`).
pub fn legacy_tui_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join("teshi"))
}

/// Import legacy TUI LLM settings into `profiles_dir` when empty.
///
/// # Errors
///
/// Returns an error when directory or file I/O fails.
pub fn ensure_tui_legacy_imported_at(
    profiles_dir: &Path,
    tui_config_dir: Option<&Path>,
) -> Result<()> {
    fs::create_dir_all(profiles_dir)
        .with_context(|| format!("create {}", profiles_dir.display()))?;
    let marker = profiles_dir.join(TUI_IMPORT_MARKER);
    if marker.exists() {
        return Ok(());
    }

    let existing = list_json_profile_count(profiles_dir)?;
    if existing > 0 {
        write_marker(&marker)?;
        return Ok(());
    }

    let Some(config_dir) = tui_config_dir else {
        write_marker(&marker)?;
        return Ok(());
    };
    if !config_dir.is_dir() {
        write_marker(&marker)?;
        return Ok(());
    }

    let mut imported = import_toml_models(profiles_dir, &config_dir.join("models"))?;
    if imported.is_empty() {
        imported = import_from_providers_and_auth(profiles_dir, config_dir)?;
    }

    // Prefer the legacy active pointer when it matches an imported id.
    if let Some(active) = read_legacy_active_id(config_dir) {
        if imported.iter().any(|id| id == &active) {
            set_active_id_in(profiles_dir, &active)?;
        }
    } else if let Some(first) = imported.first() {
        set_active_id_in(profiles_dir, first)?;
    }

    write_marker(&marker)?;
    Ok(())
}

fn write_marker(path: &Path) -> Result<()> {
    fs::write(path, b"1").with_context(|| format!("write {}", path.display()))
}

fn list_json_profile_count(dir: &Path) -> Result<usize> {
    if !dir.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
            count += 1;
        }
    }
    Ok(count)
}

fn read_legacy_active_id(config_dir: &Path) -> Option<String> {
    let path = config_dir.join("model_profile");
    let raw = fs::read_to_string(path).ok()?;
    let id = raw.trim().to_string();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

fn import_toml_models(profiles_dir: &Path, models_dir: &Path) -> Result<Vec<String>> {
    if !models_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::new();
    let mut paths: Vec<_> = fs::read_dir(models_dir)
        .with_context(|| format!("read {}", models_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
        .collect();
    paths.sort();
    for path in paths {
        let content =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let legacy: LegacyTomlProfile =
            toml::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
        if legacy.model.trim().is_empty() && legacy.api_key.trim().is_empty() {
            continue;
        }
        let mut profile = toml_to_engine_profile(legacy);
        let id = profile.id.clone();
        save_profile_in(profiles_dir, &mut profile)?;
        ids.push(id);
    }
    Ok(ids)
}

fn toml_to_engine_profile(legacy: LegacyTomlProfile) -> ModelProfile {
    let provider = map_legacy_provider_id(&legacy.provider);
    let id = if legacy.id.trim().is_empty() {
        generate_id()
    } else {
        legacy.id.trim().to_string()
    };
    let name = if legacy.name.trim().is_empty() {
        format!("{} ({})", legacy.model, provider)
    } else {
        legacy.name
    };
    let mut chat_options = HashMap::new();
    chat_options.insert("temperature".into(), json!(legacy.temperature));
    ModelProfile {
        id,
        name,
        provider,
        api_style: ApiStyle::ChatCompletions,
        model_id: legacy.model,
        max_context_tokens: None,
        max_output_tokens: legacy.max_tokens,
        base_url: legacy.base_url,
        api_key: legacy.api_key,
        stream: true,
        http_headers: HashMap::new(),
        chat_options,
    }
}

fn import_from_providers_and_auth(profiles_dir: &Path, config_dir: &Path) -> Result<Vec<String>> {
    let auth = load_auth_keys(&config_dir.join("auth.json"))?;
    let config = load_app_config(&config_dir.join("config.toml"))?;

    let mut ids = Vec::new();
    let mut default_id: Option<String> = None;

    for (name, provider) in &config.providers {
        let mut api_key = provider
            .api_key
            .clone()
            .unwrap_or_default()
            .trim()
            .to_string();
        // Strip unresolved placeholders; prefer auth.json.
        if api_key.starts_with("${") || api_key.is_empty() {
            api_key = auth.get(name).cloned().unwrap_or_default();
        }
        if api_key.is_empty() {
            continue;
        }
        let engine_provider = map_legacy_provider_id(name);
        let mut profile = ModelProfile::new(name.clone());
        profile.provider = engine_provider;
        profile.model_id = provider
            .model
            .clone()
            .unwrap_or_else(|| "gpt-4o-mini".into());
        profile.base_url = provider.base_url.clone().unwrap_or_default();
        profile.api_key = api_key;
        profile.max_output_tokens = provider.max_tokens.unwrap_or(4096);
        profile.max_context_tokens = provider.context_window;
        if let Some(temp) = provider.temperature {
            profile
                .chat_options
                .insert("temperature".into(), json!(temp));
        }
        let id = profile.id.clone();
        save_profile_in(profiles_dir, &mut profile)?;
        if config.default_provider.as_deref() == Some(name.as_str()) {
            default_id = Some(id.clone());
        }
        ids.push(id);
    }

    // auth.json-only keys with no matching provider entry.
    for (name, key) in &auth {
        if config.providers.contains_key(name) || key.trim().is_empty() {
            continue;
        }
        let mut profile = ModelProfile::new(name.clone());
        profile.provider = map_legacy_provider_id(name);
        profile.api_key = key.clone();
        let id = profile.id.clone();
        save_profile_in(profiles_dir, &mut profile)?;
        ids.push(id);
    }

    if let Some(active) = default_id {
        set_active_id_in(profiles_dir, &active)?;
    }
    Ok(ids)
}

fn load_auth_keys(path: &Path) -> Result<HashMap<String, String>> {
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let parsed: HashMap<String, CredentialEntry> =
        serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
    Ok(parsed
        .into_iter()
        .map(|(k, v)| (k, v.key))
        .filter(|(_, v)| !v.trim().is_empty())
        .collect())
}

fn load_app_config(path: &Path) -> Result<LegacyAppConfig> {
    if !path.is_file() {
        return Ok(LegacyAppConfig::default());
    }
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    Ok(toml::from_str(&content).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_map_legacy_provider_ids() {
        assert_eq!(map_legacy_provider_id("deepseek"), PROVIDER_DEEPSEEK_OPENAI);
        assert_eq!(map_legacy_provider_id("openai"), PROVIDER_OPENAI);
        assert_eq!(map_legacy_provider_id("ollama"), PROVIDER_OPENAI);
        assert_eq!(map_legacy_provider_id("anthropic"), PROVIDER_ANTHROPIC);
    }

    #[test]
    fn test_import_toml_models_when_store_empty() {
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        let config = tmp.path().join("teshi");
        let models = config.join("models");
        fs::create_dir_all(&models).unwrap();
        fs::write(
            models.join("p1.toml"),
            r#"
id = "p1"
name = "DeepSeek"
provider = "deepseek"
model = "deepseek-chat"
base_url = "https://api.deepseek.com"
api_key = "sk-ds"
max_tokens = 2048
temperature = 0.2
"#,
        )
        .unwrap();
        fs::write(config.join("model_profile"), "p1").unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&config)).unwrap();
        let content = fs::read_to_string(profiles.join("p1.json")).unwrap();
        let profile: ModelProfile = serde_json::from_str(&content).unwrap();
        assert_eq!(profile.provider, PROVIDER_DEEPSEEK_OPENAI);
        assert_eq!(profile.model_id, "deepseek-chat");
        assert_eq!(profile.api_key, "sk-ds");
        let temp = profile
            .chat_options
            .get("temperature")
            .and_then(|v| v.as_f64())
            .expect("temperature");
        assert!((temp - 0.2).abs() < 1e-5, "temperature={temp}");
        assert_eq!(
            fs::read_to_string(profiles.join("active")).unwrap().trim(),
            "p1"
        );
        assert!(profiles.join(TUI_IMPORT_MARKER).is_file());

        // Idempotent: second call does not duplicate.
        fs::write(
            models.join("p2.toml"),
            r#"
id = "p2"
name = "Other"
provider = "openai"
model = "gpt-4o"
api_key = "sk-x"
"#,
        )
        .unwrap();
        ensure_tui_legacy_imported_at(&profiles, Some(&config)).unwrap();
        assert!(!profiles.join("p2.json").exists());
    }

    #[test]
    fn test_import_from_auth_and_config_toml() {
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        let config = tmp.path().join("teshi");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("config.toml"),
            r#"
default_provider = "openai"

[providers.openai]
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key = "${auth:openai}"
"#,
        )
        .unwrap();
        fs::write(
            config.join("auth.json"),
            r#"{"openai":{"credential_type":"api_key","key":"sk-from-auth"}}"#,
        )
        .unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&config)).unwrap();
        let mut found = false;
        for entry in fs::read_dir(&profiles).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let profile: ModelProfile =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            if profile.api_key == "sk-from-auth" {
                found = true;
                assert_eq!(profile.provider, PROVIDER_OPENAI);
                assert_eq!(profile.model_id, "gpt-4o");
            }
        }
        assert!(found);
    }

    #[test]
    fn test_existing_profiles_skip_import() {
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        fs::create_dir_all(&profiles).unwrap();
        fs::write(
            profiles.join("keep.json"),
            r#"{"id":"keep","name":"Keep","provider":"openai","model_id":"x","api_key":"k"}"#,
        )
        .unwrap();
        let config = tmp.path().join("teshi");
        let models = config.join("models");
        fs::create_dir_all(&models).unwrap();
        fs::write(
            models.join("p1.toml"),
            r#"
id = "p1"
name = "ShouldNotImport"
provider = "openai"
model = "gpt-4o"
api_key = "sk-x"
"#,
        )
        .unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&config)).unwrap();
        assert!(!profiles.join("p1.json").exists());
        assert!(profiles.join(TUI_IMPORT_MARKER).is_file());
    }
}
