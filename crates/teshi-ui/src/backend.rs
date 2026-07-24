//! Backend port for LLM configuration load/save.

use std::rc::Rc;

/// Public LLM settings returned to the UI (API key never fully exposed).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
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

/// Values submitted when the user saves LLM settings.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LlmConfigUpdate {
    /// OpenAI-compatible API base URL.
    pub base_url: String,
    /// Model id.
    pub model: String,
    /// Full API key to persist.
    pub api_key: String,
}

/// Platform-specific load/save for LLM configuration.
pub trait LlmConfigBackend {
    /// Load the current snapshot (masked key).
    ///
    /// # Errors
    ///
    /// Returns an error string when the store or network call fails.
    fn get_llm_config(&self) -> Result<LlmConfigSnapshot, String>;

    /// Persist an update.
    ///
    /// # Errors
    ///
    /// Returns an error string when the store or network call fails.
    fn set_llm_config(&self, update: LlmConfigUpdate) -> Result<(), String>;
}

/// Shared backend handle used by [`crate::LlmConfigView`].
pub type SharedLlmBackend = Rc<dyn LlmConfigBackend>;
