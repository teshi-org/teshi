//! Per-project settings DTOs (pure data).
//! File I/O lives in `teshi-engine`.

use serde::{Deserialize, Serialize};

/// Default auto-confirm delay for locator proposals (seconds); `0` disables.
pub const DEFAULT_LOCATOR_AUTO_CONFIRM_SEC: u64 = 60;

/// Default DOM attribute recognized by Playwright's `getByTestId` contract.
pub const DEFAULT_PLAYWRIGHT_TEST_ID_ATTRIBUTE: &str = "data-testid";

/// Project-local teshi settings (`.teshi/settings.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// Seconds to wait before auto-confirming a pending locator; `0` = manual only.
    #[serde(default = "default_locator_auto_confirm_sec")]
    pub locator_auto_confirm_sec: u64,
    /// DOM attribute names considered stable project test identifiers.
    #[serde(default = "default_playwright_test_id_attributes")]
    pub playwright_test_id_attributes: Vec<String>,
}

fn default_locator_auto_confirm_sec() -> u64 {
    DEFAULT_LOCATOR_AUTO_CONFIRM_SEC
}

fn default_playwright_test_id_attributes() -> Vec<String> {
    vec![DEFAULT_PLAYWRIGHT_TEST_ID_ATTRIBUTE.to_string()]
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            locator_auto_confirm_sec: DEFAULT_LOCATOR_AUTO_CONFIRM_SEC,
            playwright_test_id_attributes: default_playwright_test_id_attributes(),
        }
    }
}
