//! Model profile schema — data model for an LLM profile.
//!
//! Each profile is stored as an individual TOML file under
//! `~/.config/teshi/models/{id}.toml`. The active profile ID is
//! persisted in `~/.config/teshi/model_profile`.

use serde::{Deserialize, Serialize};

/// A named model profile with provider, model ID, and connection settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    /// Unique identifier (hex string).
    pub id: String,
    /// Human-readable label shown in the UI.
    pub name: String,
    /// Provider name (e.g. `"openai"`, `"deepseek"`).
    pub provider: String,
    /// Model identifier (e.g. `"gpt-4o"`, `"deepseek-chat"`).
    pub model: String,
    /// API base URL (e.g. `"https://api.openai.com/v1"`).
    pub base_url: String,
    /// API key.
    #[serde(default)]
    pub api_key: String,
    /// Maximum generation tokens.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Sampling temperature.
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

fn default_max_tokens() -> u32 {
    4096
}
fn default_temperature() -> f32 {
    0.7
}

impl ModelProfile {
    /// Create a new profile with a fresh unique ID.
    #[allow(dead_code)]
    pub fn new(name: &str, provider: &str, model: &str, base_url: &str) -> Self {
        Self {
            id: generate_id(),
            name: name.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            base_url: base_url.to_string(),
            api_key: String::new(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
        }
    }

    /// Directory where profile TOML files live.
    pub fn storage_dir() -> std::path::PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
            .join("teshi")
            .join("models")
    }

    /// Path to the active-profile pointer file.
    pub fn active_profile_path() -> std::path::PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
            .join("teshi")
            .join("model_profile")
    }

    /// File path for this profile's TOML.
    #[allow(dead_code)]
    pub fn file_path(&self) -> std::path::PathBuf {
        Self::storage_dir().join(format!("{}.toml", self.id))
    }

    /// Save this profile to its TOML file (creates parent dir if needed).
    #[allow(dead_code)]
    pub fn save_to_disk(&self) -> Result<(), Box<dyn std::error::Error>> {
        let dir = Self::storage_dir();
        std::fs::create_dir_all(&dir)?;
        std::fs::write(self.file_path(), toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Delete this profile's TOML file from disk.
    #[allow(dead_code)]
    pub fn delete_from_disk(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = self.file_path();
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Load a profile from a TOML file path.
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let toml_str = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&toml_str)?)
    }

    /// Read the active profile ID from the pointer file.
    pub fn read_active_id() -> Option<String> {
        let path = Self::active_profile_path();
        if path.exists() {
            std::fs::read_to_string(path)
                .ok()
                .map(|s| s.trim().to_string())
        } else {
            None
        }
    }

    /// Write the active profile ID to the pointer file.
    pub fn write_active_id(id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::active_profile_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, id)?;
        Ok(())
    }

    /// List all profile TOML files on disk, sorted.
    pub fn list_profile_paths() -> Vec<std::path::PathBuf> {
        let dir = Self::storage_dir();
        if !dir.exists() {
            return vec![];
        }
        let mut paths: Vec<_> = std::fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|e| e == "toml").unwrap_or(false))
            .collect();
        paths.sort();
        paths
    }

    /// Load all profiles from disk.
    pub fn load_all() -> Vec<Self> {
        Self::list_profile_paths()
            .iter()
            .filter_map(|p| Self::load_from_file(p).ok())
            .collect()
    }
}

/// Generate a unique-ish hex ID (timestamp + pid).
#[allow(dead_code)]
fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    format!("{:016x}{:08x}", nanos, pid)
}
