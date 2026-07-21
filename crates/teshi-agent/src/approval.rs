//! Approval mode definitions — controls how agent tool call approvals work.
//!
//! Inspired by Chrys's ApprovalMode system, which determines whether a tool
//! call needs user approval before execution.

use serde::{Deserialize, Serialize};

/// Approval mode for agent tool calls.
///
/// Controls how file-modifying tool calls are handled:
///
/// - **Manual** (default): tool changes are queued and the user must press
///   Y/N to accept or reject each change before the agent continues.
/// - **Auto**: tool changes are queued and then automatically accepted
///   without manual intervention. The agent loop continues immediately.
/// - **Bypass**: tool changes are executed and applied directly without
///   queuing or approval. Use with caution — the agent has full write access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ApprovalMode {
    /// Queue changes and wait for user Y/N confirmation.
    #[default]
    Manual,
    /// Queue changes but auto-accept them without user intervention.
    Auto,
    /// Execute changes directly without queuing (full auto-pilot).
    Bypass,
}

impl ApprovalMode {
    /// All available modes in display order.
    pub const ALL: &'static [ApprovalMode] = &[
        ApprovalMode::Manual,
        ApprovalMode::Auto,
        ApprovalMode::Bypass,
    ];

    /// Human-readable display name for the mode.
    pub fn display_name(&self) -> &'static str {
        match self {
            ApprovalMode::Manual => "Manual",
            ApprovalMode::Auto => "Auto",
            ApprovalMode::Bypass => "Bypass",
        }
    }

    /// Short description of the mode.
    pub fn description(&self) -> &'static str {
        match self {
            ApprovalMode::Manual => "Queue changes and wait for user approval (Y/N)",
            ApprovalMode::Auto => "Auto-accept all changes without manual intervention",
            ApprovalMode::Bypass => "Bypass approval entirely — changes applied immediately",
        }
    }

    /// Whether this mode requires user to press Y/N.
    pub fn requires_manual_approval(&self) -> bool {
        matches!(self, ApprovalMode::Manual)
    }

    /// Whether this mode auto-accepts queued changes.
    pub fn auto_accepts(&self) -> bool {
        matches!(self, ApprovalMode::Auto)
    }

    /// Whether this mode bypasses the queue entirely.
    #[allow(dead_code)]
    pub fn bypasses_queue(&self) -> bool {
        matches!(self, ApprovalMode::Bypass)
    }
}

impl std::fmt::Display for ApprovalMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_approval_mode_is_manual() {
        assert_eq!(ApprovalMode::default(), ApprovalMode::Manual);
    }

    #[test]
    fn approval_mode_properties() {
        assert!(ApprovalMode::Manual.requires_manual_approval());
        assert!(!ApprovalMode::Auto.requires_manual_approval());
        assert!(!ApprovalMode::Bypass.requires_manual_approval());

        assert!(!ApprovalMode::Manual.auto_accepts());
        assert!(ApprovalMode::Auto.auto_accepts());
        assert!(!ApprovalMode::Bypass.auto_accepts());

        assert!(!ApprovalMode::Manual.bypasses_queue());
        assert!(!ApprovalMode::Auto.bypasses_queue());
        assert!(ApprovalMode::Bypass.bypasses_queue());
    }

    #[test]
    fn all_modes_have_display_names_and_descriptions() {
        for mode in ApprovalMode::ALL {
            assert!(!mode.display_name().is_empty());
            assert!(!mode.description().is_empty());
        }
    }

    #[test]
    fn cycle_through_modes() {
        assert_eq!(ApprovalMode::ALL.len(), 3);
        assert_eq!(ApprovalMode::ALL[0].display_name(), "Manual");
        assert_eq!(ApprovalMode::ALL[1].display_name(), "Auto");
        assert_eq!(ApprovalMode::ALL[2].display_name(), "Bypass");
    }

    #[test]
    fn display_impl() {
        assert_eq!(format!("{}", ApprovalMode::Manual), "Manual");
        assert_eq!(format!("{}", ApprovalMode::Auto), "Auto");
        assert_eq!(format!("{}", ApprovalMode::Bypass), "Bypass");
    }
}
