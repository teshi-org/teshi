//! Named LLM model profiles for GPUI/daemon (app-data `model-profiles/`).
//!
//! Profiles are the source of truth for runtime LLM configuration. Legacy
//! `llm-config.json` is imported once into a `Default` profile when needed.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app_data::app_data_dir;
use crate::legacy_tui_import::{ensure_tui_legacy_imported_at, legacy_tui_config_dir};
use crate::llm_config_store::mask_api_key;

/// Built-in OpenAI provider id.
pub const PROVIDER_OPENAI: &str = "openai";
/// Built-in Anthropic provider id.
pub const PROVIDER_ANTHROPIC: &str = "anthropic";
/// Built-in DeepSeek OpenAI-compatible provider id.
pub const PROVIDER_DEEPSEEK_OPENAI: &str = "deepseek-openai";

/// Default base URL for [`PROVIDER_OPENAI`].
pub const DEFAULT_BASE_URL_OPENAI: &str = "https://api.openai.com/v1";
/// Default base URL for [`PROVIDER_ANTHROPIC`].
pub const DEFAULT_BASE_URL_ANTHROPIC: &str = "https://api.anthropic.com";
/// Default base URL for [`PROVIDER_DEEPSEEK_OPENAI`].
pub const DEFAULT_BASE_URL_DEEPSEEK: &str = "https://api.deepseek.com";

const MIGRATION_MARKER: &str = ".migrated-from-llm-config";
const ACTIVE_POINTER: &str = "active";
const DEFAULT_PROFILE_NAME: &str = "Default";

/// Which OpenAI-family request shape to use.
///
/// Meaningful for `openai` only; other providers force chat-completions
/// semantics at effective-config time (Anthropic uses Messages transport).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApiStyle {
    /// OpenAI-compatible `/chat/completions`.
    #[default]
    ChatCompletions,
    /// OpenAI `/responses`.
    Responses,
}

/// A named model profile persisted under app data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    /// Stable unique identifier.
    pub id: String,
    /// Human-readable label.
    pub name: String,
    /// Built-in provider id (`openai`, `anthropic`, `deepseek-openai`).
    pub provider: String,
    /// API style; only honored for `openai`.
    #[serde(default)]
    pub api_style: ApiStyle,
    /// Provider model identifier.
    #[serde(default)]
    pub model_id: String,
    /// Soft context window hint (tokens).
    #[serde(default)]
    pub max_context_tokens: Option<u32>,
    /// Maximum generation tokens.
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
    /// API base URL; empty resolves to the provider default at effective time.
    #[serde(default)]
    pub base_url: String,
    /// API key plaintext (local trust model; never log).
    #[serde(default)]
    pub api_key: String,
    /// When true, use the provider streaming protocol.
    #[serde(default = "default_stream")]
    pub stream: bool,
    /// Extra HTTP headers merged into outbound requests.
    #[serde(default)]
    pub http_headers: HashMap<String, String>,
    /// Extra JSON body fields shallow-merged into the request (core fields win).
    #[serde(default)]
    pub chat_options: HashMap<String, Value>,
}

fn default_max_output_tokens() -> u32 {
    1024
}

fn default_stream() -> bool {
    true
}

impl Default for ModelProfile {
    fn default() -> Self {
        Self {
            id: generate_id(),
            name: "New Profile".into(),
            provider: PROVIDER_OPENAI.into(),
            api_style: ApiStyle::ChatCompletions,
            model_id: "gpt-4o-mini".into(),
            max_context_tokens: None,
            max_output_tokens: default_max_output_tokens(),
            base_url: String::new(),
            api_key: String::new(),
            stream: true,
            http_headers: HashMap::new(),
            chat_options: HashMap::new(),
        }
    }
}

impl ModelProfile {
    /// Create a new profile with a fresh id and the given display name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: generate_id(),
            name: name.into(),
            ..Self::default()
        }
    }
}

/// Public profile snapshot with a masked API key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfilePublic {
    /// Stable unique identifier.
    pub id: String,
    /// Human-readable label.
    pub name: String,
    /// Built-in provider id.
    pub provider: String,
    /// API style.
    pub api_style: ApiStyle,
    /// Provider model identifier.
    pub model_id: String,
    /// Soft context window hint (tokens).
    pub max_context_tokens: Option<u32>,
    /// Maximum generation tokens.
    pub max_output_tokens: u32,
    /// API base URL (may be empty).
    pub base_url: String,
    /// Whether an API key is stored.
    pub api_key_configured: bool,
    /// Masked API key for display (empty when not configured).
    pub api_key_masked: String,
    /// Streaming flag.
    pub stream: bool,
    /// Extra HTTP headers.
    pub http_headers: HashMap<String, String>,
    /// Extra chat options.
    pub chat_options: HashMap<String, Value>,
    /// Whether this profile is the active one.
    pub active: bool,
}

/// List + active summary returned by list APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfileList {
    /// All profiles (masked keys).
    pub profiles: Vec<ModelProfilePublic>,
    /// Active profile id, if any.
    pub active_id: Option<String>,
}

/// Returns whether `provider` is a built-in id.
pub fn is_builtin_provider(provider: &str) -> bool {
    matches!(
        provider,
        PROVIDER_OPENAI | PROVIDER_ANTHROPIC | PROVIDER_DEEPSEEK_OPENAI
    )
}

/// Default base URL for a built-in provider.
///
/// # Errors
///
/// Returns an error when `provider` is not a built-in id.
pub fn default_base_url_for_provider(provider: &str) -> Result<&'static str> {
    match provider {
        PROVIDER_OPENAI => Ok(DEFAULT_BASE_URL_OPENAI),
        PROVIDER_ANTHROPIC => Ok(DEFAULT_BASE_URL_ANTHROPIC),
        PROVIDER_DEEPSEEK_OPENAI => Ok(DEFAULT_BASE_URL_DEEPSEEK),
        other => bail!("unknown provider: {other}"),
    }
}

/// Resolve empty `base_url` to the provider default.
///
/// # Errors
///
/// Returns an error when `provider` is not a built-in id.
pub fn resolve_base_url(provider: &str, base_url: &str) -> Result<String> {
    if base_url.trim().is_empty() {
        Ok(default_base_url_for_provider(provider)?.to_string())
    } else {
        Ok(base_url.trim_end_matches('/').to_string())
    }
}

/// Effective API style for transport routing.
///
/// Non-`openai` providers always use chat-completions semantics.
pub fn effective_api_style(provider: &str, stored: ApiStyle) -> ApiStyle {
    if provider == PROVIDER_OPENAI {
        stored
    } else {
        ApiStyle::ChatCompletions
    }
}

/// Validate provider and coerce non-openai api_style for storage consistency.
///
/// # Errors
///
/// Returns an error when the provider is not built-in or required fields fail checks.
pub fn validate_profile(profile: &mut ModelProfile) -> Result<()> {
    if !is_builtin_provider(&profile.provider) {
        bail!("unknown provider: {}", profile.provider);
    }
    if profile.name.trim().is_empty() {
        bail!("profile name must not be empty");
    }
    validate_profile_id(&profile.id)?;
    // Anthropic / DeepSeek ignore stored style; normalize so public reads are consistent.
    if profile.provider != PROVIDER_OPENAI {
        profile.api_style = ApiStyle::ChatCompletions;
    }
    Ok(())
}

/// Validate a profile id before it is used as a filename.
///
/// Profile ids are deliberately limited to a short ASCII filename token so
/// absolute paths, parent components, separators, and platform-specific path
/// syntax can never escape the profile store.
///
/// # Errors
///
/// Returns an error when `id` is empty, too long, or contains anything other
/// than ASCII letters, digits, `-`, or `_`.
pub fn validate_profile_id(id: &str) -> Result<()> {
    if id.is_empty() {
        bail!("profile id must not be empty");
    }
    if id.len() > 128 {
        bail!("profile id must not exceed 128 bytes");
    }
    if !id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("profile id must contain only ASCII letters, digits, '-' or '_'");
    }
    Ok(())
}

fn is_sensitive_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie" | "api-key"
    ) || name.ends_with("-api-key")
        || name.ends_with("-auth-token")
        || name.ends_with("-access-token")
}

fn redact_http_headers(headers: &HashMap<String, String>) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let value = if is_sensitive_header(name) && !value.is_empty() {
                mask_api_key(value)
            } else {
                value.clone()
            };
            (name.clone(), value)
        })
        .collect()
}

fn preserve_masked_sensitive_headers(
    incoming: &mut HashMap<String, String>,
    existing: &HashMap<String, String>,
) {
    for (existing_name, existing_value) in existing {
        if !is_sensitive_header(existing_name) || existing_value.is_empty() {
            continue;
        }
        let Some(incoming_name) = incoming
            .keys()
            .find(|name| name.eq_ignore_ascii_case(existing_name))
            .cloned()
        else {
            continue;
        };
        let should_preserve = incoming.get(&incoming_name).is_some_and(|value| {
            value.is_empty() || value.as_str() == mask_api_key(existing_value)
        });
        if should_preserve {
            incoming.insert(incoming_name, existing_value.clone());
        }
    }
}

/// Convert a stored profile to a public (masked) snapshot.
pub fn to_public_profile(profile: &ModelProfile, active: bool) -> ModelProfilePublic {
    let configured = !profile.api_key.is_empty();
    ModelProfilePublic {
        id: profile.id.clone(),
        name: profile.name.clone(),
        provider: profile.provider.clone(),
        api_style: profile.api_style,
        model_id: profile.model_id.clone(),
        max_context_tokens: profile.max_context_tokens,
        max_output_tokens: profile.max_output_tokens,
        base_url: profile.base_url.clone(),
        api_key_configured: configured,
        api_key_masked: if configured {
            mask_api_key(&profile.api_key)
        } else {
            String::new()
        },
        stream: profile.stream,
        http_headers: redact_http_headers(&profile.http_headers),
        chat_options: profile.chat_options.clone(),
        active,
    }
}

/// Directory containing profile JSON files and the active pointer.
pub fn model_profiles_dir() -> Result<PathBuf> {
    let dir = app_data_dir()?.join("model-profiles");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir)
}

fn profile_path(dir: &Path, id: &str) -> Result<PathBuf> {
    validate_profile_id(id)?;
    Ok(dir.join(format!("{id}.json")))
}

fn active_path(dir: &Path) -> PathBuf {
    dir.join(ACTIVE_POINTER)
}

fn migration_marker_path(dir: &Path) -> PathBuf {
    dir.join(MIGRATION_MARKER)
}

/// Generate a unique-ish hex id (timestamp nanos + pid).
pub fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    format!("{nanos:016x}{pid:08x}")
}

/// Ensure one-time migration from legacy `llm-config.json` has run.
///
/// # Errors
///
/// Returns an error when directory or file I/O fails.
pub fn ensure_migrated() -> Result<()> {
    ensure_migrated_at(&app_data_dir()?)
}

fn ensure_migrated_at(app_root: &Path) -> Result<()> {
    let dir = app_root.join("model-profiles");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    if !migration_marker_path(&dir).exists() {
        let existing = list_raw_profiles_in(&dir)?;
        let has_any_keyed = existing.iter().any(|p| !p.api_key.trim().is_empty());
        if has_any_keyed {
            // At least one keyed profile already present — assume store is configured.
            write_migration_marker(&dir)?;
        } else {
            // No keyed profiles — attempt to import from legacy llm-config.json.
            // This covers both the empty-store case and the case where a prior migration
            // or desktop copy left only keyless placeholder profiles.
            let legacy_path = app_root.join("llm-config.json");
            let legacy = if legacy_path.exists() {
                let content = fs::read_to_string(&legacy_path)
                    .with_context(|| format!("read {}", legacy_path.display()))?;
                serde_json::from_str(&content)
                    .with_context(|| format!("parse {}", legacy_path.display()))?
            } else {
                crate::llm_config_store::StoredLlmConfig::default()
            };
            let usable = !legacy.api_key.is_empty()
                || !legacy.base_url.trim().is_empty()
                || !legacy.model.trim().is_empty();
            if usable {
                // Reuse an existing keyless Default profile when available so that any
                // previously created placeholder is updated rather than duplicated.
                let default_profile = existing
                    .iter()
                    .find(|p| p.name == DEFAULT_PROFILE_NAME)
                    .cloned();
                let mut profile =
                    default_profile.unwrap_or_else(|| ModelProfile::new(DEFAULT_PROFILE_NAME));
                profile.base_url = legacy.base_url;
                profile.model_id = if legacy.model.is_empty() {
                    "gpt-4o-mini".into()
                } else {
                    legacy.model
                };
                profile.api_key = legacy.api_key;
                profile.provider = PROVIDER_OPENAI.into();
                save_profile_in(&dir, &mut profile)?;
                set_active_id_in(&dir, &profile.id)?;
            }
            write_migration_marker(&dir)?;
        }
    }

    // After llm-config migration, import empty stores from legacy TUI paths.
    // Skip when TESHI_APP_DATA_DIR is overridden (tests / custom installs) so we
    // do not pull the developer's real config_dir into a temp store.
    let using_override = std::env::var("TESHI_APP_DATA_DIR")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    if !using_override {
        let tui_dir = legacy_tui_config_dir();
        ensure_tui_legacy_imported_at(&dir, tui_dir.as_deref())?;
    }
    Ok(())
}

fn write_migration_marker(dir: &Path) -> Result<()> {
    fs::write(migration_marker_path(dir), b"1")
        .with_context(|| format!("write migration marker in {}", dir.display()))
}

fn list_raw_profiles_in(dir: &Path) -> Result<Vec<ModelProfile>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut profiles = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let profile: ModelProfile =
            serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
        profiles.push(profile);
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    Ok(profiles)
}

/// Read the active profile id, if the pointer file exists.
pub fn read_active_id() -> Result<Option<String>> {
    read_active_id_in(&model_profiles_dir()?)
}

fn read_active_id_in(dir: &Path) -> Result<Option<String>> {
    let path = active_path(dir);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let id = raw.trim().to_string();
    if id.is_empty() {
        Ok(None)
    } else {
        Ok(Some(id))
    }
}

/// Persist the active-profile pointer.
///
/// # Errors
///
/// Returns an error when the profile id does not exist or I/O fails.
pub fn set_active_id(id: &str) -> Result<()> {
    ensure_migrated()?;
    set_active_id_in(&model_profiles_dir()?, id)
}

pub(crate) fn set_active_id_in(dir: &Path, id: &str) -> Result<()> {
    let path = profile_path(dir, id)?;
    if !path.exists() {
        bail!("profile not found: {id}");
    }
    fs::write(active_path(dir), id)
        .with_context(|| format!("write active pointer in {}", dir.display()))?;
    Ok(())
}

/// Load a profile by id (full key).
///
/// # Errors
///
/// Returns an error when the profile is missing or cannot be parsed.
pub fn load_profile(id: &str) -> Result<ModelProfile> {
    ensure_migrated()?;
    load_profile_in(&model_profiles_dir()?, id)
}

fn load_profile_in(dir: &Path, id: &str) -> Result<ModelProfile> {
    let path = profile_path(dir, id)?;
    if !path.exists() {
        bail!("profile not found: {id}");
    }
    let content = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))
}

/// List all profiles with masked keys and active flag.
///
/// # Errors
///
/// Returns an error on I/O or parse failure.
pub fn list_profiles() -> Result<ModelProfileList> {
    ensure_migrated()?;
    list_profiles_in(&model_profiles_dir()?)
}

fn list_profiles_in(dir: &Path) -> Result<ModelProfileList> {
    let profiles = list_raw_profiles_in(dir)?;
    let active_id = read_active_id_in(dir)?;
    // If pointer is missing but profiles exist, activate the first.
    let active_id = match active_id {
        Some(id) if profiles.iter().any(|p| p.id == id) => Some(id),
        _ => {
            if let Some(first) = profiles.first() {
                set_active_id_in(dir, &first.id)?;
                Some(first.id.clone())
            } else {
                None
            }
        }
    };
    let public: Vec<_> = profiles
        .iter()
        .map(|p| {
            let is_active = active_id.as_deref() == Some(p.id.as_str());
            to_public_profile(p, is_active)
        })
        .collect();
    Ok(ModelProfileList {
        profiles: public,
        active_id,
    })
}

/// Get a single public profile by id.
///
/// # Errors
///
/// Returns an error when the profile is missing.
pub fn get_profile_public(id: &str) -> Result<ModelProfilePublic> {
    ensure_migrated()?;
    let dir = model_profiles_dir()?;
    let profile = load_profile_in(&dir, id)?;
    let active = read_active_id_in(&dir)?.as_deref() == Some(id);
    Ok(to_public_profile(&profile, active))
}

/// Create or update a profile.
///
/// When `api_key` on `profile` is empty and an existing profile has a key,
/// the previous key is preserved.
///
/// # Errors
///
/// Returns a validation or I/O error.
pub fn save_profile(profile: &mut ModelProfile) -> Result<ModelProfilePublic> {
    ensure_migrated()?;
    save_profile_in(&model_profiles_dir()?, profile)
}

pub(crate) fn save_profile_in(
    dir: &Path,
    profile: &mut ModelProfile,
) -> Result<ModelProfilePublic> {
    validate_profile(profile)?;
    let path = profile_path(dir, &profile.id)?;
    if path.exists() {
        let existing = load_profile_in(dir, &profile.id)?;
        if profile.api_key.is_empty() && !existing.api_key.is_empty() {
            profile.api_key = existing.api_key;
        }
        preserve_masked_sensitive_headers(&mut profile.http_headers, &existing.http_headers);
    }
    let json = serde_json::to_string_pretty(profile)?;
    fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;

    // First profile becomes active automatically.
    let active = match read_active_id_in(dir)? {
        Some(id) if id == profile.id => true,
        Some(_) => false,
        None => {
            set_active_id_in(dir, &profile.id)?;
            true
        }
    };
    Ok(to_public_profile(profile, active))
}

/// Delete a profile by id.
///
/// Refuses to delete the last remaining profile. When deleting the active
/// profile and others remain, another profile is activated.
///
/// # Errors
///
/// Returns an error when the profile is missing, is the last one, or I/O fails.
pub fn delete_profile(id: &str) -> Result<()> {
    ensure_migrated()?;
    delete_profile_in(&model_profiles_dir()?, id)
}

fn delete_profile_in(dir: &Path, id: &str) -> Result<()> {
    let profiles = list_raw_profiles_in(dir)?;
    if !profiles.iter().any(|p| p.id == id) {
        bail!("profile not found: {id}");
    }
    if profiles.len() <= 1 {
        bail!("cannot delete the last remaining profile");
    }
    let path = profile_path(dir, id)?;
    fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;

    let active = read_active_id_in(dir)?;
    if active.as_deref() == Some(id) {
        let remaining = list_raw_profiles_in(dir)?;
        if let Some(next) = remaining.first() {
            set_active_id_in(dir, &next.id)?;
        }
    }
    Ok(())
}

/// Load the active profile (full key), if any.
///
/// # Errors
///
/// Returns an error on I/O failure (missing active is `Ok(None)`).
pub fn load_active_profile() -> Result<Option<ModelProfile>> {
    ensure_migrated()?;
    let dir = model_profiles_dir()?;
    let Some(id) = read_active_id_in(&dir)? else {
        return Ok(None);
    };
    match load_profile_in(&dir, &id) {
        Ok(p) => Ok(Some(p)),
        Err(_) => Ok(None),
    }
}

/// Map a profile into runtime [`crate::llm::LlmConfig`].
///
/// # Errors
///
/// Returns an error when the provider is unknown.
pub fn profile_to_llm_config(profile: &ModelProfile) -> Result<crate::llm::LlmConfig> {
    let base_url = resolve_base_url(&profile.provider, &profile.base_url)?;
    let model = if profile.model_id.is_empty() {
        "gpt-4o-mini".into()
    } else {
        profile.model_id.clone()
    };
    // Prefer an explicit chat_options temperature so TUI/Desktop round-trips preserve it.
    let temperature = profile
        .chat_options
        .get("temperature")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(0.7);
    Ok(crate::llm::LlmConfig {
        api_key: profile.api_key.clone(),
        base_url,
        model,
        max_tokens: profile.max_output_tokens,
        temperature,
        context_window: profile.max_context_tokens,
        provider: profile.provider.clone(),
        api_style: effective_api_style(&profile.provider, profile.api_style),
        stream: profile.stream,
        http_headers: profile.http_headers.clone(),
        chat_options: profile.chat_options.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_store() -> TempDir {
        TempDir::new().expect("tempdir")
    }

    #[test]
    fn test_default_base_url_for_builtin_providers() {
        assert_eq!(
            default_base_url_for_provider(PROVIDER_OPENAI).unwrap(),
            DEFAULT_BASE_URL_OPENAI
        );
        assert_eq!(
            default_base_url_for_provider(PROVIDER_ANTHROPIC).unwrap(),
            DEFAULT_BASE_URL_ANTHROPIC
        );
        assert_eq!(
            default_base_url_for_provider(PROVIDER_DEEPSEEK_OPENAI).unwrap(),
            DEFAULT_BASE_URL_DEEPSEEK
        );
        assert!(default_base_url_for_provider("glm").is_err());
    }

    #[test]
    fn test_empty_base_url_resolves_to_provider_default() {
        let url = resolve_base_url(PROVIDER_DEEPSEEK_OPENAI, "").unwrap();
        assert_eq!(url, DEFAULT_BASE_URL_DEEPSEEK);
    }

    #[test]
    fn test_stream_defaults_to_true() {
        let p = ModelProfile::new("x");
        assert!(p.stream);
    }

    #[test]
    fn test_profile_to_llm_config_prefers_chat_options_temperature() {
        let mut p = ModelProfile::new("Temp");
        p.provider = PROVIDER_OPENAI.into();
        p.model_id = "gpt-4o-mini".into();
        p.api_key = "sk-test".into();
        p.chat_options
            .insert("temperature".into(), serde_json::json!(0.25));
        let cfg = profile_to_llm_config(&p).unwrap();
        assert!((cfg.temperature - 0.25).abs() < 1e-5);
        assert_eq!(cfg.provider, PROVIDER_OPENAI);
    }

    #[test]
    fn test_validate_rejects_unknown_provider() {
        let mut p = ModelProfile::new("x");
        p.provider = "glm".into();
        assert!(validate_profile(&mut p).is_err());
    }

    #[test]
    fn test_profile_id_rejects_path_components() {
        for invalid in [
            "",
            ".",
            "..",
            "../escape",
            "nested/profile",
            "/absolute",
            "a\\b",
        ] {
            assert!(
                validate_profile_id(invalid).is_err(),
                "accepted invalid profile id {invalid:?}"
            );
        }
        for valid in ["default", "profile-01", "profile_01", "ABC123"] {
            validate_profile_id(valid).unwrap();
        }

        let tmp = temp_store();
        let dir = tmp.path().join("model-profiles");
        fs::create_dir(&dir).unwrap();
        let mut profile = ModelProfile::new("Escape");
        profile.id = "../escape".into();
        assert!(save_profile_in(&dir, &mut profile).is_err());
        assert!(!tmp.path().join("escape.json").exists());
    }

    #[test]
    fn test_public_headers_are_redacted_and_masked_values_preserve_secrets() {
        let tmp = temp_store();
        let dir = tmp.path();
        write_migration_marker(dir).unwrap();

        let mut profile = ModelProfile::new("Headers");
        profile
            .http_headers
            .insert("Authorization".into(), "Bearer credential-secret".into());
        profile
            .http_headers
            .insert("api-key".into(), "header-secret".into());
        profile
            .http_headers
            .insert("X-Region".into(), "us-east".into());

        let public = save_profile_in(dir, &mut profile).unwrap();
        assert_eq!(public.http_headers["X-Region"], "us-east");
        assert!(!public.http_headers["Authorization"].contains("credential-secret"));
        assert!(!public.http_headers["api-key"].contains("header-secret"));

        profile.http_headers = public.http_headers;
        save_profile_in(dir, &mut profile).unwrap();
        let stored = load_profile_in(dir, &profile.id).unwrap();
        assert_eq!(
            stored.http_headers["Authorization"],
            "Bearer credential-secret"
        );
        assert_eq!(stored.http_headers["api-key"], "header-secret");
    }

    #[test]
    fn test_crud_save_list_activate() {
        let tmp = temp_store();
        let dir = tmp.path();
        write_migration_marker(dir).unwrap();

        let mut a = ModelProfile::new("Alpha");
        a.provider = PROVIDER_OPENAI.into();
        a.api_key = "sk-aaaa".into();
        save_profile_in(dir, &mut a).unwrap();

        let mut b = ModelProfile::new("Beta");
        b.provider = PROVIDER_ANTHROPIC.into();
        b.api_key = "sk-bbbb".into();
        save_profile_in(dir, &mut b).unwrap();

        set_active_id_in(dir, &b.id).unwrap();
        let list = list_profiles_in(dir).unwrap();
        assert_eq!(list.profiles.len(), 2);
        assert_eq!(list.active_id.as_deref(), Some(b.id.as_str()));
        let active = list.profiles.iter().find(|p| p.active).unwrap();
        assert_eq!(active.id, b.id);
        assert!(active.api_key_masked.contains('…'));
        assert!(!active.api_key_masked.contains("sk-bbbb"));
    }

    #[test]
    fn test_empty_api_key_on_save_preserves_stored() {
        let tmp = temp_store();
        let dir = tmp.path();
        write_migration_marker(dir).unwrap();

        let mut p = ModelProfile::new("Keep");
        p.api_key = "sk-secret".into();
        save_profile_in(dir, &mut p).unwrap();

        p.api_key.clear();
        p.name = "Renamed".into();
        save_profile_in(dir, &mut p).unwrap();

        let loaded = load_profile_in(dir, &p.id).unwrap();
        assert_eq!(loaded.api_key, "sk-secret");
        assert_eq!(loaded.name, "Renamed");
    }

    #[test]
    fn test_delete_last_profile_rejected() {
        let tmp = temp_store();
        let dir = tmp.path();
        write_migration_marker(dir).unwrap();

        let mut p = ModelProfile::new("Only");
        save_profile_in(dir, &mut p).unwrap();
        let err = delete_profile_in(dir, &p.id).unwrap_err();
        assert!(err.to_string().contains("last remaining"));
    }

    #[test]
    fn test_delete_active_activates_another() {
        let tmp = temp_store();
        let dir = tmp.path();
        write_migration_marker(dir).unwrap();

        let mut a = ModelProfile::new("A");
        let mut b = ModelProfile::new("B");
        save_profile_in(dir, &mut a).unwrap();
        save_profile_in(dir, &mut b).unwrap();
        set_active_id_in(dir, &a.id).unwrap();

        delete_profile_in(dir, &a.id).unwrap();
        let list = list_profiles_in(dir).unwrap();
        assert_eq!(list.profiles.len(), 1);
        assert_eq!(list.active_id.as_deref(), Some(b.id.as_str()));
    }

    #[test]
    fn test_migration_from_legacy_is_idempotent() {
        let tmp = temp_store();
        let app = tmp.path();

        let legacy = crate::llm_config_store::StoredLlmConfig {
            base_url: "https://example.com/v1".into(),
            model: "gpt-test".into(),
            api_key: "sk-legacy".into(),
        };
        fs::write(
            app.join("llm-config.json"),
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();

        ensure_migrated_at(app).unwrap();
        let dir = app.join("model-profiles");
        let list = list_profiles_in(&dir).unwrap();
        assert_eq!(list.profiles.len(), 1);
        assert_eq!(list.profiles[0].name, DEFAULT_PROFILE_NAME);
        assert!(list.profiles[0].api_key_configured);
        assert_eq!(list.profiles[0].model_id, "gpt-test");

        // Second ensure must not duplicate.
        ensure_migrated_at(app).unwrap();
        let list2 = list_profiles_in(&dir).unwrap();
        assert_eq!(list2.profiles.len(), 1);
    }

    #[test]
    fn test_keyless_profiles_do_not_block_llm_config_import() {
        // When only keyless profiles exist (e.g. from desktop migration) and
        // llm-config.json has a usable api_key, the key must be imported into
        // the Default profile rather than being silently skipped.
        let tmp = temp_store();
        let app = tmp.path();
        let dir = app.join("model-profiles");
        fs::create_dir_all(&dir).unwrap();

        // Prevent TUI import from running against the real developer config dir.
        fs::write(
            dir.join(crate::legacy_tui_import::TUI_IMPORT_MARKER_FOR_TEST),
            b"1",
        )
        .unwrap();

        // Pre-existing keyless profile (e.g. written by desktop copy migration).
        let keyless = serde_json::json!({
            "id": "default",
            "name": DEFAULT_PROFILE_NAME,
            "provider": "openai",
            "model_id": "gpt-4o-mini",
            "api_key": ""
        });
        fs::write(dir.join("default.json"), keyless.to_string()).unwrap();

        // llm-config.json with a usable api_key.
        let legacy = crate::llm_config_store::StoredLlmConfig {
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o".into(),
            api_key: "sk-from-llm-config".into(),
        };
        fs::write(
            app.join("llm-config.json"),
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();

        ensure_migrated_at(app).unwrap();

        let profiles = list_raw_profiles_in(&dir).unwrap();
        assert_eq!(profiles.len(), 1, "must not duplicate the Default profile");
        assert_eq!(
            profiles[0].api_key, "sk-from-llm-config",
            "keyless Default must be filled with llm-config api_key"
        );
        assert_eq!(profiles[0].model_id, "gpt-4o");
        assert!(dir.join(MIGRATION_MARKER).is_file());
    }

    #[test]
    fn test_effective_api_style_forced_for_non_openai() {
        assert_eq!(
            effective_api_style(PROVIDER_ANTHROPIC, ApiStyle::Responses),
            ApiStyle::ChatCompletions
        );
        assert_eq!(
            effective_api_style(PROVIDER_OPENAI, ApiStyle::Responses),
            ApiStyle::Responses
        );
    }
}
