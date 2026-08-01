//! One-time import of legacy TUI LLM config into the shared model-profile store.
//!
//! Sources (OS config dir `teshi/`):
//! - `models/*.toml` — TUI model panel profiles
//! - `config.toml` `[providers.*]` + `auth.json` — provider table + credential store
//!
//! A project-level `config.toml` may supplement the user config with additional
//! provider entries; auth keys still resolve via the user `auth.json`.
//!
//! The migration marker `.migrated-from-tui-config` is written **only** when
//! `tui_config_dir` exists and is a directory. When the directory is absent the
//! marker is withheld so a later-created legacy config can still be imported.

use std::collections::{HashMap, HashSet};
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

/// Exported for use in tests that need to pre-seed the TUI import marker.
#[cfg(test)]
pub(crate) const TUI_IMPORT_MARKER_FOR_TEST: &str = TUI_IMPORT_MARKER;

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

/// Import legacy TUI LLM settings into `profiles_dir` when no migration marker exists.
///
/// Both TOML model files (`tui_config_dir/models/*.toml`) and provider entries from
/// `tui_config_dir/config.toml` (and optionally `project_config_dir/config.toml`) are
/// always attempted in a single pass. Auth keys still resolve via `tui_config_dir/auth.json`.
///
/// The marker `.migrated-from-tui-config` is written **only** when `tui_config_dir` exists
/// and is a directory, so a later-created legacy config can still be imported when the
/// directory is currently absent.
///
/// An existing valid active pointer in the shared store is never overwritten.
///
/// # Errors
///
/// Returns an error when directory or file I/O fails.
pub fn ensure_tui_legacy_imported_at(
    profiles_dir: &Path,
    tui_config_dir: Option<&Path>,
    project_config_dir: Option<&Path>,
) -> Result<()> {
    fs::create_dir_all(profiles_dir)
        .with_context(|| format!("create {}", profiles_dir.display()))?;
    let marker = profiles_dir.join(TUI_IMPORT_MARKER);
    if marker.exists() {
        return Ok(());
    }

    let Some(config_dir) = tui_config_dir else {
        // No config dir — withhold marker so a later-created dir can still import.
        return Ok(());
    };
    if !config_dir.is_dir() {
        // Config dir not yet a directory — withhold marker so it can import once created.
        return Ok(());
    }

    // Always attempt both TOML models and provider/auth sources.
    let mut imported = import_toml_models(profiles_dir, &config_dir.join("models"))?;

    // Reload snapshot after TOML phase so provider dedup sees newly-written profiles.
    let existing_after_toml = load_all_profiles(profiles_dir);
    let provider_ids = import_from_providers_and_auth(
        profiles_dir,
        config_dir,
        project_config_dir,
        &existing_after_toml,
    )?;
    imported.extend(provider_ids);

    // Only set active when there is no existing valid active pointer in the store.
    // This prevents overwriting a desktop-migrated or user-set active profile.
    if !active_pointer_is_valid(profiles_dir) {
        if let Some(active) = read_legacy_active_id(config_dir) {
            if imported.iter().any(|id| id == &active) {
                set_active_id_in(profiles_dir, &active)?;
            }
        } else if let Some(first) = imported.first() {
            set_active_id_in(profiles_dir, first)?;
        }
    }

    // Write marker after a fully successful pass (config dir existed; both sources processed).
    // On error (propagated via `?` above) the marker is NOT written, allowing retry.
    write_marker(&marker)?;
    Ok(())
}

/// Returns `true` when `profiles_dir/active` exists and points to a present profile file.
fn active_pointer_is_valid(profiles_dir: &Path) -> bool {
    let ap = profiles_dir.join("active");
    if !ap.is_file() {
        return false;
    }
    match fs::read_to_string(&ap) {
        Ok(s) => {
            let id = s.trim().to_string();
            !id.is_empty() && profiles_dir.join(format!("{id}.json")).exists()
        }
        Err(_) => false,
    }
}

fn write_marker(path: &Path) -> Result<()> {
    fs::write(path, b"1").with_context(|| format!("write {}", path.display()))
}

/// Load all parseable [`ModelProfile`] entries from `dir`, silently skipping
/// non-JSON files and files that fail to parse.
fn load_all_profiles(dir: &Path) -> Vec<ModelProfile> {
    if !dir.exists() {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|ex| ex.to_str()) == Some("json"))
        .filter_map(|e| {
            fs::read_to_string(e.path())
                .ok()
                .and_then(|c| serde_json::from_str::<ModelProfile>(&c).ok())
        })
        .collect()
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
        // Skip profiles already written by a prior partial-import attempt.
        if profiles_dir.join(format!("{id}.json")).exists() {
            ids.push(id);
            continue;
        }
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

/// Import profiles from `config_dir/config.toml` (merged with optional `project_config_dir`)
/// and `config_dir/auth.json`.
///
/// Provider entries are deduplicated by `(engine_provider, normalized_base_url)` against
/// both the `existing` snapshot and entries already created in the same pass, so that two
/// config entries with identical provider+base_url never produce duplicate profiles.
fn import_from_providers_and_auth(
    profiles_dir: &Path,
    config_dir: &Path,
    project_config_dir: Option<&Path>,
    existing: &[ModelProfile],
) -> Result<Vec<String>> {
    let auth = load_auth_keys(&config_dir.join("auth.json"))?;
    let user_config = load_app_config(&config_dir.join("config.toml"))?;

    // Merge providers from the optional project config; auth keys still come from user auth.json.
    let project_config = project_config_dir
        .map(|d| load_app_config(&d.join("config.toml")))
        .transpose()?
        .unwrap_or_default();

    // User default_provider takes precedence; fall back to project config.
    let default_provider = user_config
        .default_provider
        .or(project_config.default_provider);
    // User provider entries take precedence; project entries fill gaps.
    let mut all_providers = user_config.providers;
    for (name, prov) in project_config.providers {
        all_providers.entry(name).or_insert(prov);
    }

    let mut ids = Vec::new();
    let mut default_id: Option<String> = None;

    // Seed the dedup set from the pre-import snapshot; extend it within the loop to catch
    // within-pass duplicates (e.g. two config entries sharing the same provider+base_url).
    let mut seen: HashSet<(String, String)> = existing
        .iter()
        .filter(|p| !p.api_key.trim().is_empty())
        .map(|p| {
            let base = p.base_url.trim_end_matches('/').to_lowercase();
            (p.provider.clone(), base)
        })
        .collect();

    for (name, provider) in &all_providers {
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
        let new_base = provider
            .base_url
            .as_deref()
            .unwrap_or("")
            .trim_end_matches('/')
            .to_lowercase();
        let dedup_key = (engine_provider.clone(), new_base.clone());
        // Skip if a keyed profile for the same provider+base_url already exists, or was
        // already created earlier in this same pass.
        if seen.contains(&dedup_key) {
            continue;
        }
        seen.insert(dedup_key);

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
        if default_provider.as_deref() == Some(name.as_str()) {
            default_id = Some(id.clone());
        }
        ids.push(id);
    }

    // auth.json-only keys with no matching provider entry.
    for (name, key) in &auth {
        if all_providers.contains_key(name) || key.trim().is_empty() {
            continue;
        }
        let engine_provider = map_legacy_provider_id(name);
        // Auth-only entries have no custom base_url; dedup against empty-base_url keyed profiles.
        let dedup_key = (engine_provider.clone(), String::new());
        if seen.contains(&dedup_key) {
            continue;
        }
        seen.insert(dedup_key);

        let mut profile = ModelProfile::new(name.clone());
        profile.provider = engine_provider;
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

        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();
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
        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();
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

        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();
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
    fn test_existing_keyed_profiles_do_not_block_toml_import() {
        // Existing keyed profiles must NOT block TOML import of different-id profiles.
        // Only the marker is the signal that a full pass has completed.
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
name = "Additional"
provider = "openai"
model = "gpt-4o"
api_key = "sk-x"
"#,
        )
        .unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();
        // p1 is a different id — it should be imported alongside keep.
        assert!(profiles.join("p1.json").exists());
        // keep.json must not be removed or overwritten.
        assert!(profiles.join("keep.json").is_file());
        assert!(profiles.join(TUI_IMPORT_MARKER).is_file());
    }

    #[test]
    fn test_keyless_profile_does_not_block_tui_import() {
        // A keyless Default profile (from llm-config migration) must not prevent
        // importing credentials from TUI config sources.
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        fs::create_dir_all(&profiles).unwrap();
        fs::write(
            profiles.join("default.json"),
            r#"{"id":"default","name":"Default","provider":"openai","model_id":"gpt-4o-mini","api_key":""}"#,
        )
        .unwrap();

        let config = tmp.path().join("teshi");
        let models = config.join("models");
        fs::create_dir_all(&models).unwrap();
        fs::write(
            models.join("real.toml"),
            r#"
id = "real"
name = "Real"
provider = "openai"
model = "gpt-4o"
api_key = "sk-real"
"#,
        )
        .unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();
        assert!(
            profiles.join("real.json").exists(),
            "import must proceed past keyless placeholder"
        );
        assert!(profiles.join(TUI_IMPORT_MARKER).is_file());
    }

    #[test]
    fn test_partial_keyless_profiles_without_marker_import_continues() {
        // Simulate: keyless profiles exist (no marker), e.g. from a failed prior import
        // or llm-config migration. Import must continue and add the keyed TOML profile.
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        fs::create_dir_all(&profiles).unwrap();
        // Partial state: a keyless profile from a previous step, no marker.
        fs::write(
            profiles.join("partial.json"),
            r#"{"id":"partial","name":"Partial","provider":"openai","model_id":"","api_key":""}"#,
        )
        .unwrap();

        let config = tmp.path().join("teshi");
        let models = config.join("models");
        fs::create_dir_all(&models).unwrap();
        fs::write(
            models.join("newprofile.toml"),
            r#"
id = "newprofile"
name = "New"
provider = "anthropic"
model = "claude-3-haiku-20240307"
api_key = "sk-ant-test"
"#,
        )
        .unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();
        assert!(
            profiles.join("newprofile.json").exists(),
            "import must continue past partial keyless state"
        );
        assert!(profiles.join(TUI_IMPORT_MARKER).is_file());
    }

    #[test]
    fn test_different_base_urls_produce_separate_profiles() {
        // Two OpenAI-compatible provider entries with different base_urls must each
        // produce their own profile rather than the second being dropped by dedup.
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        let config = tmp.path().join("teshi");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("config.toml"),
            r#"
[providers.openai]
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key = "sk-openai"

[providers.ollama]
base_url = "http://localhost:11434/v1"
model = "llama3"
api_key = "ollama-key"
"#,
        )
        .unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();

        let mut found_openai = false;
        let mut found_ollama = false;
        for entry in fs::read_dir(&profiles).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let profile: ModelProfile =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            if profile.api_key == "sk-openai" {
                found_openai = true;
            }
            if profile.api_key == "ollama-key" {
                found_ollama = true;
            }
        }
        assert!(found_openai, "openai profile must be imported");
        assert!(
            found_ollama,
            "ollama profile (different base_url) must be imported separately"
        );
        assert!(profiles.join(TUI_IMPORT_MARKER).is_file());
    }

    #[test]
    fn test_same_provider_and_base_url_not_duplicated_on_retry() {
        // On retry, a keyed profile with the same provider+base_url must not be duplicated.
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        fs::create_dir_all(&profiles).unwrap();
        fs::write(
            profiles.join("existing.json"),
            r#"{"id":"existing","name":"Existing","provider":"openai","model_id":"gpt-4o","api_key":"sk-openai","base_url":"https://api.openai.com/v1"}"#,
        )
        .unwrap();

        let config = tmp.path().join("teshi");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("config.toml"),
            r#"
[providers.openai]
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key = "sk-openai"
"#,
        )
        .unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();

        let count = fs::read_dir(&profiles)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .ok()
                    .and_then(|e| e.path().extension().map(|x| x == "json"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(
            count, 1,
            "no duplicate profile must be created for same provider+base_url"
        );
        assert!(profiles.join(TUI_IMPORT_MARKER).is_file());
    }

    // ── Fix 7: marker withheld when config dir absent ──────────────────────────

    #[test]
    fn test_no_marker_when_tui_config_dir_is_none() {
        // When no config dir is provided the marker must NOT be written so a future
        // call can still run once the dir appears.
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");

        ensure_tui_legacy_imported_at(&profiles, None, None).unwrap();
        assert!(
            !profiles.join(TUI_IMPORT_MARKER).is_file(),
            "marker must not be written when tui_config_dir is None"
        );

        // After the dir is created the next call must complete and write the marker.
        let config = tmp.path().join("teshi");
        fs::create_dir_all(&config).unwrap();
        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();
        assert!(
            profiles.join(TUI_IMPORT_MARKER).is_file(),
            "marker must be written after config dir appears"
        );
    }

    #[test]
    fn test_no_marker_when_tui_config_dir_does_not_exist() {
        // A non-existent config dir path must also withhold the marker.
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        let nonexistent = tmp.path().join("does-not-exist");

        ensure_tui_legacy_imported_at(&profiles, Some(&nonexistent), None).unwrap();
        assert!(
            !profiles.join(TUI_IMPORT_MARKER).is_file(),
            "marker must not be written when config dir does not exist"
        );
    }

    // ── Fix 2: active pointer not overwritten ─────────────────────────────────

    #[test]
    fn test_import_does_not_overwrite_valid_active_when_no_legacy_pointer() {
        // A pre-existing valid active pointer (e.g. from desktop migration) must not be
        // replaced when the legacy config has no `model_profile` pointer.
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        fs::create_dir_all(&profiles).unwrap();

        // Pre-existing active profile from desktop migration.
        fs::write(
            profiles.join("desktop.json"),
            r#"{"id":"desktop","name":"Desktop","provider":"openai","model_id":"gpt-4o","api_key":"sk-desk"}"#,
        )
        .unwrap();
        fs::write(profiles.join("active"), "desktop").unwrap();

        // TUI config has a model but NO model_profile pointer file.
        let config = tmp.path().join("teshi");
        let models = config.join("models");
        fs::create_dir_all(&models).unwrap();
        fs::write(
            models.join("tui.toml"),
            r#"
id = "tui"
name = "TUI"
provider = "openai"
model = "gpt-4o"
api_key = "sk-tui"
"#,
        )
        .unwrap();
        // Intentionally no `model_profile` file → read_legacy_active_id returns None.

        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();

        let active = fs::read_to_string(profiles.join("active")).unwrap();
        assert_eq!(
            active.trim(),
            "desktop",
            "pre-existing active must not be overwritten"
        );
        assert!(
            profiles.join("tui.json").is_file(),
            "tui profile must still be imported"
        );
    }

    // ── Fix 3: both TOML and providers always run ──────────────────────────────

    #[test]
    fn test_both_toml_and_providers_imported_when_both_present() {
        // Provider entries must be imported even when TOML models were also found.
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        let config = tmp.path().join("teshi");
        let models = config.join("models");
        fs::create_dir_all(&models).unwrap();

        fs::write(
            models.join("p1.toml"),
            r#"
id = "p1"
name = "TOML Profile"
provider = "openai"
model = "gpt-4o"
api_key = "sk-toml"
"#,
        )
        .unwrap();
        fs::write(
            config.join("config.toml"),
            r#"
[providers.anthropic]
model = "claude-3-haiku-20240307"
api_key = "sk-anthropic"
"#,
        )
        .unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();

        assert!(
            profiles.join("p1.json").is_file(),
            "TOML profile must be imported"
        );
        let mut found_anthropic = false;
        for entry in fs::read_dir(&profiles).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let profile: ModelProfile =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            if profile.api_key == "sk-anthropic" {
                found_anthropic = true;
            }
        }
        assert!(
            found_anthropic,
            "provider profile must be imported even when TOML had models"
        );
        assert!(profiles.join(TUI_IMPORT_MARKER).is_file());
    }

    // ── Fix 4: project config.toml providers imported ─────────────────────────

    #[test]
    fn test_project_config_providers_are_imported() {
        // Providers from the project-level config.toml must be imported when no user
        // config entry exists for the same provider.
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        let user_config = tmp.path().join("teshi");
        let project_config = tmp.path().join("project-teshi");
        // User config dir exists but has no providers.
        fs::create_dir_all(&user_config).unwrap();
        fs::create_dir_all(&project_config).unwrap();

        fs::write(
            project_config.join("config.toml"),
            r#"
[providers.anthropic]
model = "claude-3-haiku-20240307"
api_key = "sk-proj-anthropic"
"#,
        )
        .unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&user_config), Some(&project_config))
            .unwrap();

        let mut found = false;
        for entry in fs::read_dir(&profiles).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let profile: ModelProfile =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            if profile.api_key == "sk-proj-anthropic" {
                found = true;
                assert_eq!(profile.provider, PROVIDER_ANTHROPIC);
            }
        }
        assert!(found, "project config providers must be imported");
        assert!(profiles.join(TUI_IMPORT_MARKER).is_file());
    }

    #[test]
    fn test_user_config_provider_takes_precedence_over_project_config() {
        // When the same provider name appears in both user and project configs,
        // the user config entry wins.
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        let user_config = tmp.path().join("teshi");
        let project_config = tmp.path().join("project-teshi");
        fs::create_dir_all(&user_config).unwrap();
        fs::create_dir_all(&project_config).unwrap();

        fs::write(
            user_config.join("config.toml"),
            r#"
[providers.openai]
model = "gpt-4o"
api_key = "sk-user"
"#,
        )
        .unwrap();
        fs::write(
            project_config.join("config.toml"),
            r#"
[providers.openai]
model = "gpt-3.5-turbo"
api_key = "sk-project"
"#,
        )
        .unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&user_config), Some(&project_config))
            .unwrap();

        let mut found_user = false;
        for entry in fs::read_dir(&profiles).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let profile: ModelProfile =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            if profile.api_key == "sk-user" {
                found_user = true;
            }
            assert_ne!(
                profile.api_key, "sk-project",
                "project config must not override user config"
            );
        }
        assert!(found_user, "user config provider must be imported");
    }

    // ── Fix 6: within-pass provider dedup ─────────────────────────────────────

    #[test]
    fn test_same_provider_base_url_within_same_config_not_duplicated() {
        // Two provider entries in the same config.toml that map to the same
        // engine_provider+base_url must produce only one profile.
        let tmp = TempDir::new().unwrap();
        let profiles = tmp.path().join("model-profiles");
        let config = tmp.path().join("teshi");
        fs::create_dir_all(&config).unwrap();
        // "openai" and "openai-alias" both resolve to PROVIDER_OPENAI + same base_url.
        fs::write(
            config.join("config.toml"),
            r#"
[providers.openai]
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key = "sk-first"

[providers.openai-alias]
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key = "sk-second"
"#,
        )
        .unwrap();

        ensure_tui_legacy_imported_at(&profiles, Some(&config), None).unwrap();

        let count = fs::read_dir(&profiles)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .ok()
                    .and_then(|e| e.path().extension().map(|x| x == "json"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(
            count, 1,
            "same provider+base_url within one config must not create two profiles"
        );
        assert!(profiles.join(TUI_IMPORT_MARKER).is_file());
    }
}
