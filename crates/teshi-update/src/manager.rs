//! Shared application update use cases, independent of UI and command parsing.

use crate::{
    ErrorCode, Result, UpdateError, UpdateEvent, UpdateStatus, exe,
    github::{Candidate, GithubHttp, GithubSource, SystemClock},
    install::Installation,
    manifest::{BUNDLE_MANIFEST, InstallKind},
    storage::state_directory,
    transaction::{Preparation, TransactionResult, last_result},
};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::atomic::AtomicBool};
use teshi_core::version::{BUILD_TARGET, BuildIdentity, ReleaseChannel};

/// Complete check-only result for automation and native UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    /// Available or up-to-date state.
    pub status: UpdateStatus,
    /// Running build.
    pub current: BuildIdentity,
    /// Validated target, when available.
    pub candidate: Option<Candidate>,
    /// Resolved ownership and eligibility explanation.
    pub installation: Installation,
    /// Outcome of a previous helper transaction.
    pub previous_transaction: Option<TransactionResult>,
}

/// Resolves an update using the production GitHub source without payload mutations.
///
/// # Errors
/// Returns ownership, network and metadata validation failures.
pub fn check(
    executable: &Path,
    current: BuildIdentity,
    channel: Option<ReleaseChannel>,
) -> Result<CheckResult> {
    let installation = Installation::detect(executable, &current)?;
    let selected = channel.unwrap_or(match current.channel {
        ReleaseChannel::Dev => ReleaseChannel::Stable,
        other => other,
    });
    let source = GithubSource {
        http: GithubHttp::new()?,
        clock: SystemClock,
        cache_dir: state_directory()?,
    };
    let kind = match installation.kind {
        InstallKind::Exe => InstallKind::Exe,
        InstallKind::Msi => InstallKind::Msi,
        _ => InstallKind::Portable,
    };
    let candidate = source.check(&current, selected, BUILD_TARGET, kind, channel.is_some())?;
    let previous_transaction = installation.root.as_deref().and_then(last_result);
    Ok(CheckResult {
        status: if candidate.is_some() {
            UpdateStatus::UpdateAvailable
        } else {
            UpdateStatus::UpToDate
        },
        current,
        candidate,
        installation,
        previous_transaction,
    })
}

/// Downloads the Windows setup.exe, stages it with Inno, and hands off to the helper.
///
/// # Errors
/// Refuses ineligible installations and propagates validation/handoff failures.
pub fn install(
    check: &CheckResult,
    cancel: &AtomicBool,
    emit: &mut impl FnMut(UpdateEvent),
) -> Result<TransactionResult> {
    install_with_restart(check, cancel, emit, None)
}

/// Installs with an explicitly approved native desktop restart context.
///
/// # Errors
/// Returns the same eligibility, download and handoff errors as [`install`].
pub fn install_with_restart(
    check: &CheckResult,
    cancel: &AtomicBool,
    emit: &mut impl FnMut(UpdateEvent),
    restart: Option<crate::transaction::Restart>,
) -> Result<TransactionResult> {
    if !check.installation.can_install() {
        return Err(UpdateError::new(
            ErrorCode::Unsupported,
            check
                .installation
                .explanation
                .clone()
                .unwrap_or_else(|| "Installation is not updateable".into()),
        ));
    }
    let candidate = check
        .candidate
        .as_ref()
        .ok_or_else(|| crate::invalid("No update candidate"))?;
    exe::require_exe_payload(candidate)?;
    let root = check
        .installation
        .root
        .as_deref()
        .ok_or_else(|| crate::invalid("Missing installation root"))?;
    let old = check
        .installation
        .bundle
        .as_ref()
        .ok_or_else(|| crate::invalid("Missing installation inventory"))?;
    if old.kind != InstallKind::Exe {
        return Err(UpdateError::new(
            ErrorCode::Unsupported,
            "Only Windows setup.exe installations can apply in-app updates",
        ));
    }
    let preparation = Preparation::new(root)?;
    let package = preparation.directory.path().join(&candidate.asset.name);
    crate::download::download(&GithubHttp::new()?, candidate, &package, cancel, emit)?;
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(UpdateError::new(ErrorCode::Cancelled, "Update cancelled"));
    }
    exe::run_silent_setup(&package, root)?;
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(UpdateError::new(ErrorCode::Cancelled, "Update cancelled"));
    }
    let stage = preparation.directory.path().join("stage");
    exe::copy_tree(&exe::staged_payload(root), &stage)?;
    let new: crate::manifest::BundleManifest =
        serde_json::from_slice(&std::fs::read(stage.join(BUNDLE_MANIFEST))?)?;
    if new.kind != InstallKind::Exe {
        return Err(crate::invalid(
            "Staged setup payload is not a Windows EXE installation",
        ));
    }
    emit(UpdateEvent {
        status: UpdateStatus::Staged,
        progress: None,
        detail: candidate.manifest.tag.clone(),
    });
    preparation.handoff(root, old, &new, restart)
}
