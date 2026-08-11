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

/// Non-sensitive browser-extension identity shown by the shared GPUI shell.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSessionIdentitySnapshot {
    /// Stable opaque identifier scoped to one extension installation/profile.
    pub extension_instance_id: String,
    /// Optional user-provided display label; never used for routing.
    #[serde(default)]
    pub profile_label: Option<String>,
    /// Installed extension version.
    #[serde(default)]
    pub extension_version: String,
    /// Broker protocol version spoken by this extension.
    #[serde(default)]
    pub protocol_version: u32,
}

/// Browser product metadata reported by an extension session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserMetadataSnapshot {
    /// Browser product name.
    #[serde(default)]
    pub name: String,
    /// Browser product version.
    #[serde(default)]
    pub version: String,
    /// Browser-reported platform.
    #[serde(default)]
    pub platform: Option<String>,
}

/// Public lease state. The secret lease token is intentionally absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserLeaseSnapshot {
    /// Display-only owner label.
    #[serde(default)]
    pub owner_label: String,
    /// Wall-clock expiry as Unix milliseconds.
    #[serde(default)]
    pub expires_at_ms: i64,
}

/// One browser tab reported by an extension session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserTabSnapshot {
    /// Browser-local tab id.
    pub id: i64,
    /// Browser-local window id. Old extensions may omit it on the tab itself.
    #[serde(default)]
    pub window_id: Option<i64>,
    /// Current page title.
    #[serde(default)]
    pub title: String,
    /// Current page URL.
    #[serde(default)]
    pub url: String,
    /// Whether this is the window's active tab.
    #[serde(default)]
    pub active: bool,
    /// Whether the extension can attach the debugger to this page.
    #[serde(default = "default_true")]
    pub debuggable: bool,
}

/// One browser window and its tabs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserWindowSnapshot {
    /// Browser-local window id.
    pub id: i64,
    /// Whether Chrome reports this window focused.
    #[serde(default)]
    pub focused: bool,
    /// Tabs belonging to the window.
    #[serde(default)]
    pub tabs: Vec<BrowserTabSnapshot>,
}

/// Public discovery record for one browser-extension instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSessionSnapshot {
    /// Versioned extension identity.
    pub identity: BrowserSessionIdentitySnapshot,
    /// Browser product metadata.
    #[serde(default)]
    pub browser: BrowserMetadataSnapshot,
    /// Stable health state (`ready`, `stale`, `disconnected`, ...).
    #[serde(default)]
    pub health: String,
    /// Age of the last extension heartbeat.
    #[serde(default)]
    pub last_heartbeat_age_ms: u64,
    /// Current window/tab inventory.
    #[serde(default)]
    pub windows: Vec<BrowserWindowSnapshot>,
    /// Public lease summary when another local actor owns this session.
    #[serde(default)]
    pub lease: Option<BrowserLeaseSnapshot>,
}

impl BrowserSessionSnapshot {
    /// Return whether this session is eligible for compatibility auto-selection.
    pub fn is_eligible(&self) -> bool {
        self.health == "ready"
            && self
                .windows
                .iter()
                .flat_map(|window| &window.tabs)
                .any(|tab| tab.debuggable)
    }
}

/// Discovery response returned by the loopback broker or daemon adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSessionListSnapshot {
    /// Whether at least one compatible extension session is live.
    #[serde(default)]
    pub extension_connected: bool,
    /// Whether legacy implicit targeting would be ambiguous.
    #[serde(default)]
    pub ambiguous_browser_target: bool,
    /// All retained session records, including recently disconnected ones.
    #[serde(default)]
    pub sessions: Vec<BrowserSessionSnapshot>,
}

/// Composite target used when the GPUI shell activates a browser tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserTabTarget {
    /// Stable extension/profile identity.
    pub extension_instance_id: String,
    /// Browser-local window id.
    pub window_id: i64,
    /// Browser-local tab id.
    pub tab_id: i64,
}

/// Platform I/O required by the shared browser-session panel.
pub trait BrowserSessionsBackend {
    /// Ensure the local Chrome broker is running.
    ///
    /// # Errors
    ///
    /// Returns an actionable error when this host cannot start the broker.
    fn start_browser_bridge(&self) -> Result<(), String>;

    /// Read the latest browser-extension session inventory.
    ///
    /// # Errors
    ///
    /// Returns an actionable error when the broker is unavailable or malformed.
    fn list_browser_sessions(&self) -> Result<BrowserSessionListSnapshot, String>;

    /// Activate one explicitly selected tab.
    ///
    /// # Errors
    ///
    /// Returns an error when the target disappeared, is busy, or cannot be debugged.
    fn activate_browser_tab(&self, target: &BrowserTabTarget) -> Result<(), String>;
}

/// Shared backend handle used by [`crate::BrowserSessionsView`].
pub type SharedBrowserSessionsBackend = Rc<dyn BrowserSessionsBackend>;
