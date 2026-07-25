//! Backend port for LLM configuration and model-profile CRUD.

use std::collections::HashMap;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Public LLM settings returned to the UI (API key never fully exposed).
///
/// Flat projection of the **active** model profile for compatibility.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfigSnapshot {
    /// OpenAI-compatible API base URL.
    pub base_url: String,
    /// Model id.
    pub model: String,
    /// Whether an API key is stored.
    pub api_key_configured: bool,
    /// Masked key preview (for example `…abcd`); empty when not configured.
    #[serde(default)]
    pub api_key_masked: String,
}

/// Values submitted when the user saves LLM settings via the flat compat API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfigUpdate {
    /// OpenAI-compatible API base URL.
    pub base_url: String,
    /// Model id.
    pub model: String,
    /// Full API key to persist (empty preserves the stored key).
    pub api_key: String,
}

/// Which OpenAI-family request shape to use (meaningful for `openai` only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApiStyleDto {
    /// OpenAI-compatible `/chat/completions`.
    #[default]
    ChatCompletions,
    /// OpenAI `/responses`.
    Responses,
}

/// Public model profile snapshot (masked API key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfileSnapshot {
    /// Stable unique identifier.
    pub id: String,
    /// Human-readable label.
    pub name: String,
    /// Built-in provider id.
    pub provider: String,
    /// API style.
    pub api_style: ApiStyleDto,
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
    /// Masked API key for display.
    #[serde(default)]
    pub api_key_masked: String,
    /// Streaming flag.
    pub stream: bool,
    /// Extra HTTP headers.
    #[serde(default)]
    pub http_headers: HashMap<String, String>,
    /// Extra chat options.
    #[serde(default)]
    pub chat_options: HashMap<String, Value>,
    /// Whether this profile is the active one.
    pub active: bool,
}

/// List of profiles plus the active id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfileListSnapshot {
    /// All profiles (masked keys).
    pub profiles: Vec<ModelProfileSnapshot>,
    /// Active profile id, if any.
    pub active_id: Option<String>,
}

/// Create/update payload for a model profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfileUpdate {
    /// Stable unique identifier.
    pub id: String,
    /// Human-readable label.
    pub name: String,
    /// Built-in provider id.
    pub provider: String,
    /// API style.
    #[serde(default)]
    pub api_style: ApiStyleDto,
    /// Provider model identifier.
    #[serde(default)]
    pub model_id: String,
    /// Soft context window hint (tokens).
    #[serde(default)]
    pub max_context_tokens: Option<u32>,
    /// Maximum generation tokens.
    #[serde(default = "default_max_output")]
    pub max_output_tokens: u32,
    /// API base URL (may be empty).
    #[serde(default)]
    pub base_url: String,
    /// Full API key (empty preserves the stored key on update).
    #[serde(default)]
    pub api_key: String,
    /// Streaming flag.
    #[serde(default = "default_true")]
    pub stream: bool,
    /// Extra HTTP headers.
    #[serde(default)]
    pub http_headers: HashMap<String, String>,
    /// Extra chat options.
    #[serde(default)]
    pub chat_options: HashMap<String, Value>,
}

fn default_max_output() -> u32 {
    1024
}

fn default_true() -> bool {
    true
}

impl Default for ModelProfileUpdate {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "New Profile".into(),
            provider: "openai".into(),
            api_style: ApiStyleDto::ChatCompletions,
            model_id: "gpt-4o-mini".into(),
            max_context_tokens: None,
            max_output_tokens: 1024,
            base_url: String::new(),
            api_key: String::new(),
            stream: true,
            http_headers: HashMap::new(),
            chat_options: HashMap::new(),
        }
    }
}

/// Platform-specific load/save for LLM configuration and model profiles.
pub trait LlmConfigBackend {
    /// Load the flat active-profile projection (masked key).
    ///
    /// # Errors
    ///
    /// Returns an error string when the store or network call fails.
    fn get_llm_config(&self) -> Result<LlmConfigSnapshot, String>;

    /// Persist a flat update onto the active profile.
    ///
    /// # Errors
    ///
    /// Returns an error string when the store or network call fails.
    fn set_llm_config(&self, update: LlmConfigUpdate) -> Result<(), String>;

    /// List all model profiles (masked keys).
    ///
    /// # Errors
    ///
    /// Returns an error string when the store or network call fails.
    fn list_profiles(&self) -> Result<ModelProfileListSnapshot, String>;

    /// Get one profile by id (masked key).
    ///
    /// # Errors
    ///
    /// Returns an error string when the profile is missing or I/O fails.
    fn get_profile(&self, id: &str) -> Result<ModelProfileSnapshot, String>;

    /// Create or update a profile.
    ///
    /// # Errors
    ///
    /// Returns an error string on validation or I/O failure.
    fn save_profile(&self, update: ModelProfileUpdate) -> Result<ModelProfileSnapshot, String>;

    /// Delete a profile by id.
    ///
    /// # Errors
    ///
    /// Returns an error string when delete is rejected or I/O fails.
    fn delete_profile(&self, id: &str) -> Result<(), String>;

    /// Activate a profile by id.
    ///
    /// # Errors
    ///
    /// Returns an error string when the profile is missing or I/O fails.
    fn activate_profile(&self, id: &str) -> Result<(), String>;
}

/// Shared backend handle used by [`crate::AppShell`] / [`crate::LlmConfigView`].
pub type SharedLlmBackend = Rc<dyn LlmConfigBackend>;
