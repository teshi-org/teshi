//! Per-project settings DTOs (pure data).
//! File I/O lives in `teshi-engine`.

use serde::{Deserialize, Serialize};

/// Default auto-confirm delay for locator proposals (seconds); `0` disables.
pub const DEFAULT_LOCATOR_AUTO_CONFIRM_SEC: u64 = 60;

/// Project-local teshi settings (`.teshi/settings.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// Seconds to wait before auto-confirming a pending locator; `0` = manual only.
    #[serde(default = "default_locator_auto_confirm_sec")]
    pub locator_auto_confirm_sec: u64,
}

fn default_locator_auto_confirm_sec() -> u64 {
    DEFAULT_LOCATOR_AUTO_CONFIRM_SEC
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            locator_auto_confirm_sec: DEFAULT_LOCATOR_AUTO_CONFIRM_SEC,
        }
    }
}
