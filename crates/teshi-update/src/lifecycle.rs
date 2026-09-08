//! Shared desktop/CLI rules for when an update may start and how handoff is shown.

use crate::{UpdateStatus, transaction::TransactionResult};

/// Message shown when Settings still has an unsaved draft.
pub const UNSAVED_INSTALL_MESSAGE: &str =
    "Save your changes in Settings before installing the update.";

/// Message shown when the helper is waiting and the shell must stay open.
pub const WAITING_FOR_EXIT_MESSAGE: &str =
    "Update is waiting. Save settings and close Teshi; if it times out, retry installation.";

/// Native desktop action after the helper accepts (or fails) a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopHandoff {
    /// No unsaved work; quit so the helper can replace files.
    QuitForHelper,
    /// Helper is waiting; keep the window open and do not claim success.
    KeepOpen,
    /// Replacement actually finished (restart may still have failed).
    Installed,
    /// Any other status, including errors and blocked.
    Report,
}

/// Installation must not start while the native shell has unsaved settings.
pub fn may_start_install(unsaved_work: bool) -> bool {
    !unsaved_work
}

/// True only after managed files were replaced. Pending/staged must not use this.
pub fn replacement_committed(status: UpdateStatus) -> bool {
    matches!(
        status,
        UpdateStatus::Completed | UpdateStatus::ReadyToRestart
    )
}

/// Decides how the native UI presents a helper or installer result.
pub fn desktop_handoff(result: &TransactionResult, can_exit: bool) -> DesktopHandoff {
    match result.status {
        UpdateStatus::WaitingForExit if can_exit => DesktopHandoff::QuitForHelper,
        UpdateStatus::WaitingForExit => DesktopHandoff::KeepOpen,
        UpdateStatus::Completed | UpdateStatus::ReadyToRestart => DesktopHandoff::Installed,
        _ => DesktopHandoff::Report,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UpdateStatus;
    use crate::transaction::TransactionResult;

    fn result(status: UpdateStatus) -> TransactionResult {
        TransactionResult {
            transaction_id: "transaction-test".into(),
            status,
            detail: "helper accepted".into(),
            reboot_required: false,
        }
    }

    #[test]
    fn unsaved_settings_block_install_and_pending_is_never_installed() {
        assert!(!may_start_install(true));
        assert!(may_start_install(false));
        let pending = result(UpdateStatus::WaitingForExit);
        assert_eq!(desktop_handoff(&pending, false), DesktopHandoff::KeepOpen);
        assert_eq!(
            desktop_handoff(&pending, true),
            DesktopHandoff::QuitForHelper
        );
        assert!(!replacement_committed(pending.status));
        assert!(replacement_committed(UpdateStatus::Completed));
        assert_ne!(
            desktop_handoff(&result(UpdateStatus::Completed), false),
            DesktopHandoff::QuitForHelper
        );
    }
}
