pub mod types;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow};

pub use types::{AppConfig, ProviderConfig};

/// Resolves an `api_key` value which may be a placeholder.
///
/// Supported formats:
/// - `${auth:provider}` — look up in `auth.json`
/// - `${env:VAR}` — read from environment variable
/// - Plaintext — returned as-is
/// - `None` — returned as `None`
pub fn resolve_api_key(api_key: Option<&str>) -> Option<String> {
    let key = api_key?;
    if let Some(rest) = key.strip_prefix("${auth:") {
        let provider = rest.strip_suffix('}')?;
        let mgr = crate::auth::CredentialManager::new().ok()?;
        let creds = mgr.load().ok()?;
        creds.get(provider).map(|e| e.key.clone())
    } else if let Some(rest) = key.strip_prefix("${env:") {
        let var = rest.strip_suffix('}')?;
        std::env::var(var).ok()
    } else {
        Some(key.to_string())
    }
}

/// Detects whether a string looks like an unresolved placeholder.
#[allow(dead_code)]
pub fn is_placeholder(value: Option<&str>) -> bool {
    value.map(|v| v.starts_with("${")).unwrap_or(false)
}

/// Hardcoded defaults for known providers.
fn hardcoded_defaults() -> AppConfig {
    let mut providers = HashMap::new();

    providers.insert(
        "deepseek".into(),
        ProviderConfig {
            base_url: Some("https://api.deepseek.com".into()),
            model: Some("deepseek-chat".into()),
            api_key: Some("${auth:deepseek}".into()),
            max_tokens: Some(1024),
            temperature: Some(0.7),
            context_window: Some(65536),
        },
    );

    providers.insert(
        "openai".into(),
        ProviderConfig {
            base_url: Some("https://api.openai.com/v1".into()),
            model: Some("gpt-4o".into()),
            api_key: Some("${auth:openai}".into()),
            max_tokens: Some(1024),
            temperature: Some(0.7),
            context_window: Some(128000),
        },
    );

    AppConfig {
        default_provider: Some("deepseek".into()),
        providers,
    }
}

/// Loads and deserializes a TOML config file, returning `None` if the file
/// does not exist (i.e. it should be silently skipped).
fn load_toml_file(path: &Path) -> Result<Option<AppConfig>> {
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let cfg: AppConfig =
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(cfg))
}

/// Merges `overlay` into `base`: for each provider in `overlay`, each `Some`
/// field overwrites the corresponding field in `base`. New providers are added.
fn merge_config(base: &mut AppConfig, overlay: &AppConfig) {
    if overlay.default_provider.is_some() {
        base.default_provider = overlay.default_provider.clone();
    }
    for (name, provider) in &overlay.providers {
        let entry = base
            .providers
            .entry(name.clone())
            .or_insert(ProviderConfig {
                base_url: None,
                model: None,
                api_key: None,
                max_tokens: None,
                temperature: None,
                context_window: None,
            });
        if provider.base_url.is_some() {
            entry.base_url = provider.base_url.clone();
        }
        if provider.model.is_some() {
            entry.model = provider.model.clone();
        }
        if provider.api_key.is_some() {
            entry.api_key = provider.api_key.clone();
        }
        if provider.max_tokens.is_some() {
            entry.max_tokens = provider.max_tokens;
        }
        if provider.temperature.is_some() {
            entry.temperature = provider.temperature;
        }
        if provider.context_window.is_some() {
            entry.context_window = provider.context_window;
        }
    }
}

/// Applies environment variable overrides (highest priority).
///
/// Supported variables:
/// - `TESHI_DEFAULT_PROVIDER` → `default_provider`
fn apply_env_overrides(config: &mut AppConfig) {
    if let Ok(val) = std::env::var("TESHI_DEFAULT_PROVIDER") {
        config.default_provider = Some(val);
    }
}

/// Scans the project-level config for plaintext (non-placeholder) API keys
/// and prints a warning if any are found.
fn warn_plaintext_keys(_config_path: &Path, cfg: &AppConfig) {
    for (name, provider) in &cfg.providers {
        if let Some(ref key) = provider.api_key
            && !key.starts_with("${")
            && key.len() > 4
        {
            eprintln!(
                "warning: provider '{}' has a plaintext API key in config. \
                 Consider using teshi auth login to store it securely.",
                name
            );
        }
    }
}

/// Resolves all `${auth:*}` and `${env:*}` placeholders in the config's
/// `api_key` fields against the credential store and environment.
fn resolve_all_placeholders(config: &mut AppConfig) {
    let resolved: Vec<(String, Option<String>)> = config
        .providers
        .iter()
        .filter_map(|(name, provider)| {
            provider
                .api_key
                .as_ref()
                .map(|key| (name.clone(), resolve_api_key(Some(key))))
        })
        .collect();
    for (name, key) in resolved {
        if let Some(provider) = config.providers.get_mut(&name) {
            provider.api_key = key;
        }
    }
}

/// Resolves a single provider's API key using the credential store and env.
///
/// Returns `Ok(Some(key))` if resolved, `Ok(None)` if no key is configured,
/// or an error if a placeholder couldn't be resolved.
#[allow(dead_code)]
pub fn resolve_provider_key(config: &AppConfig, name: &str) -> Result<Option<String>> {
    let provider = config.providers.get(name);
    let api_key = provider.and_then(|p| p.api_key.as_deref());
    let Some(key) = api_key else {
        return Ok(None);
    };

    if let Some(rest) = key.strip_prefix("${auth:") {
        let provider_name = rest.strip_suffix('}').ok_or_else(|| {
            anyhow!(
                "invalid placeholder syntax in api_key for provider '{}': {}",
                name,
                key
            )
        })?;
        let mgr = crate::auth::CredentialManager::new()?;
        let creds = mgr.load()?;
        let entry = creds.get(provider_name).ok_or_else(|| {
            anyhow!(
                "no credentials stored for provider '{}'. Run 'teshi auth login --provider {}' to configure.",
                provider_name,
                provider_name
            )
        })?;
        Ok(Some(entry.key.clone()))
    } else if let Some(rest) = key.strip_prefix("${env:") {
        let var = rest.strip_suffix('}').ok_or_else(|| {
            anyhow!(
                "invalid placeholder syntax in api_key for provider '{}': {}",
                name,
                key
            )
        })?;
        let val = std::env::var(var).map_err(|_| {
            anyhow!(
                "environment variable '{}' is not set (required by provider '{}').",
                var,
                name
            )
        })?;
        Ok(Some(val))
    } else {
        Ok(Some(key.to_string()))
    }
}

/// Loads the complete configuration using the layering strategy:
///
/// 1. Hardcoded defaults
/// 2. `~/.teshi/config.toml` (user-level)
/// 3. `.teshi/config.toml` (project-level, relative to cwd)
/// 4. Environment variable overrides (highest priority)
///
/// All `${auth:*}` / `${env:*}` placeholders are resolved after merging.
pub fn load_config() -> Result<AppConfig> {
    let mut config = hardcoded_defaults();

    let user_config_path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("teshi")
        .join("config.toml");

    if let Some(user_cfg) = load_toml_file(&user_config_path)? {
        merge_config(&mut config, &user_cfg);
    }

    let project_config_path = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".teshi")
        .join("config.toml");

    if let Some(project_cfg) = load_toml_file(&project_config_path)? {
        warn_plaintext_keys(&project_config_path, &project_cfg);
        merge_config(&mut config, &project_cfg);
    }

    apply_env_overrides(&mut config);
    resolve_all_placeholders(&mut config);

    Ok(config)
}
