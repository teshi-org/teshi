//! Shared application update use cases, independent of UI and command parsing.

use crate::{
    ErrorCode, Result, UpdateError, UpdateEvent, UpdateStatus, exe,
    github::{Candidate, GithubHttp, GithubSource, Http, HttpResponse, SystemClock},
    install::Installation,
    manifest::{BUNDLE_MANIFEST, InstallKind},
    payload::Payload,
    storage::{install_key, state_directory, write_json},
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

/// Explicit process-close and restart choices for one installation attempt.
#[derive(Default)]
pub struct InstallOptions {
    /// Allow verified processes from this installation to be force-closed after
    /// graceful shutdown times out. This can discard unsaved work.
    pub force_close: bool,
    /// Native desktop restart context, when approved by its caller.
    pub restart: Option<crate::transaction::Restart>,
}

/// Resolves release metadata and persists failures for local status inspection.
///
/// # Errors
/// Returns ownership, network and metadata validation failures.
pub fn check(
    executable: &Path,
    current: BuildIdentity,
    channel: Option<ReleaseChannel>,
) -> Result<CheckResult> {
    let key = diagnostic_key(executable, &current);
    check_inner(executable, current, channel)
        .map_err(|error| persist_failure(&key, "metadata", error, None, false))
}

fn diagnostic_key(executable: &Path, current: &BuildIdentity) -> std::path::PathBuf {
    Installation::detect(executable, current)
        .ok()
        .and_then(|install| install.root)
        .unwrap_or_else(|| {
            std::fs::canonicalize(executable).unwrap_or_else(|_| executable.to_path_buf())
        })
}

fn persist_failure(
    root: &Path,
    stage: &str,
    mut error: UpdateError,
    candidate: Option<&Candidate>,
    cached: bool,
) -> UpdateError {
    // A competing invocation must not overwrite the active updater's diagnostics.
    if error.code != ErrorCode::Busy
        && let Err(diagnostic) = crate::diagnostics::record(root, stage, &error, candidate, cached)
    {
        error.message.push_str(&format!(
            " (could not persist update diagnostics: {diagnostic})"
        ));
    }
    error
}

/// Resolves an update using the production GitHub source without payload mutations.
///
/// # Errors
/// Returns ownership, network and metadata validation failures.
fn check_inner(
    executable: &Path,
    current: BuildIdentity,
    channel: Option<ReleaseChannel>,
) -> Result<CheckResult> {
    let installation = Installation::detect(executable, &current)?;
    if current.channel == ReleaseChannel::Local && channel.is_none() {
        return Err(UpdateError::new(
            ErrorCode::Unsupported,
            "Local builds do not follow a published update stream; rebuild locally or explicitly select --channel stable or --channel nightly",
        ));
    }
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
    install_with_options(
        check,
        cancel,
        emit,
        InstallOptions {
            restart,
            ..Default::default()
        },
    )
}

/// Installs using process-close permissions scoped to this one attempt.
///
/// # Errors
/// Returns the same validation and handoff errors as [`install`].
pub fn install_with_options(
    check: &CheckResult,
    cancel: &AtomicBool,
    emit: &mut impl FnMut(UpdateEvent),
    options: InstallOptions,
) -> Result<TransactionResult> {
    install_impl(
        check,
        cancel,
        emit,
        options,
        false,
        &GithubHttp::new()?,
        &state_directory()?,
    )
}

fn retry_path(cache: &Path, root: &Path) -> std::path::PathBuf {
    cache.join(format!("retry-{}.json", install_key(root)))
}

struct Offline;
impl Http for Offline {
    fn get(&self, _: &str, _: Option<&str>) -> Result<HttpResponse> {
        Err(crate::invalid(
            "Offline installation must never request a download",
        ))
    }
}

/// Revalidates and installs the last prepared candidate without network access.
/// The current installation must still match the snapshot used to cache it.
///
/// # Errors
/// Returns ownership, stale snapshot, missing cache or installation failures.
pub fn retry_install(
    executable: &Path,
    current: BuildIdentity,
    cancel: &AtomicBool,
    emit: &mut impl FnMut(UpdateEvent),
) -> Result<(CheckResult, TransactionResult)> {
    retry_install_with_close(executable, current, cancel, emit, false)
}

/// Retries offline with explicit, non-persisted permission to force-close processes.
///
/// # Errors
/// Returns the same validation and installation errors as [`retry_install`].
pub fn retry_install_with_close(
    executable: &Path,
    current: BuildIdentity,
    cancel: &AtomicBool,
    emit: &mut impl FnMut(UpdateEvent),
    force_close: bool,
) -> Result<(CheckResult, TransactionResult)> {
    let key = diagnostic_key(executable, &current);
    let cache = state_directory()?;
    let report = load_retry(executable, current, &cache)
        .map_err(|error| persist_failure(&key, "verification", error, None, false))?;
    let transaction = install_impl(
        &report,
        cancel,
        emit,
        InstallOptions {
            force_close,
            ..Default::default()
        },
        true,
        &Offline,
        &cache,
    )?;
    Ok((report, transaction))
}

fn load_retry(executable: &Path, current: BuildIdentity, cache: &Path) -> Result<CheckResult> {
    let installation = Installation::detect(executable, &current)?;
    let root = installation
        .root
        .as_deref()
        .ok_or_else(|| crate::invalid("No updateable installation"))?;
    let path = retry_path(cache, root);
    crate::install::reject_links(
        cache,
        path.strip_prefix(cache)
            .map_err(|_| crate::invalid("Invalid retry path"))?,
    )?;
    let saved = std::fs::read(&path).map_err(|error| {
        UpdateError::new(
            ErrorCode::Verification,
            format!("No cached installation snapshot; run teshi update first: {error}"),
        )
    })?;
    if saved.len() > 8 * 1024 * 1024 {
        return Err(crate::invalid("Retry snapshot is too large"));
    }
    let snapshot: CheckResult = serde_json::from_slice(&saved)?;
    if snapshot.current != current || snapshot.installation.root != installation.root {
        return Err(crate::invalid(
            "Installation changed since the cached attempt; run teshi update to select a new release",
        ));
    }
    Ok(CheckResult {
        current,
        installation,
        ..snapshot
    })
}

fn install_impl(
    check: &CheckResult,
    cancel: &AtomicBool,
    emit: &mut impl FnMut(UpdateEvent),
    options: InstallOptions,
    offline: bool,
    http: &impl Http,
    cache: &Path,
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
    let mut phase = "verification";
    let mut cached = false;
    let result = (|| {
        crate::payload::validate_candidate(candidate)?;
        exe::require_exe_payload(candidate)?;
        if old.kind != InstallKind::Exe
            || candidate.asset.target != old.target
            || candidate.asset.target != BUILD_TARGET
        {
            return Err(crate::invalid(
                "Payload does not match the installed kind/target",
            ));
        }
        if !crate::release::is_update(&check.current, &candidate.manifest.identity, true)? {
            return Err(crate::invalid(
                "Cached candidate cannot replace the current build; run teshi update to select a new release",
            ));
        }
        // Reserve the install before any cache/network mutation. A second updater
        // cannot download or replace the offline retry snapshot concurrently.
        phase = "reservation";
        let preparation = Preparation::new(root)?;
        let payload = Payload::reserve(cache, candidate)?;
        phase = "download";
        let package = payload.obtain(http, candidate, offline, cancel, &mut |event| {
            phase = if event.status == UpdateStatus::Verifying {
                "verification"
            } else {
                "download"
            };
            emit(event);
        })?;
        cached = true;
        phase = "verification";
        if !offline {
            write_json(&retry_path(cache, root), check)?;
        }
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(UpdateError::new(ErrorCode::Cancelled, "Update cancelled"));
        }
        // Keep the persistent cache separate from the executable handed to setup.
        // Copy and reverify inside the existing same-volume transaction boundary.
        let executable = preparation.directory.path().join(&candidate.asset.name);
        std::fs::copy(&package, &executable)?;
        if crate::download::file_hash(&executable)? != candidate.asset.sha256 {
            return Err(UpdateError::new(
                ErrorCode::Verification,
                "Payload changed while staging setup.exe",
            ));
        }
        let stage = preparation.directory.path().join("stage");
        let new = stage_setup(&executable, root, &stage, candidate, cancel, &mut phase)?;
        emit(UpdateEvent {
            status: UpdateStatus::Staged,
            progress: None,
            detail: candidate.manifest.tag.clone(),
        });
        phase = "handoff";
        crate::diagnostics::clear(root)?;
        // The helper reads the pinned identity only for failure diagnostics.
        write_json(
            &preparation.directory.path().join("candidate.json"),
            candidate,
        )?;
        preparation.handoff_with_close(root, old, &new, options.restart, options.force_close)
    })();
    result.map_err(|error| {
        let phase = if phase == "download" && error.code == ErrorCode::Verification {
            "verification"
        } else {
            phase
        };
        persist_failure(root, phase, error, Some(candidate), cached)
    })
}

fn stage_setup(
    package: &Path,
    root: &Path,
    stage: &Path,
    candidate: &Candidate,
    cancel: &AtomicBool,
    phase: &mut &'static str,
) -> Result<crate::manifest::BundleManifest> {
    let result = (|| {
        *phase = "staging";
        exe::clear_staged_payload(root)?;
        *phase = "setup";
        exe::run_silent_setup(package, root)?;
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(UpdateError::new(ErrorCode::Cancelled, "Update cancelled"));
        }
        *phase = "staging";
        exe::copy_tree(&exe::staged_payload(root), stage)?;
        *phase = "verification";
        let new: crate::manifest::BundleManifest =
            serde_json::from_slice(&std::fs::read(stage.join(BUNDLE_MANIFEST))?)?;
        if new.kind != InstallKind::Exe
            || new.identity != candidate.manifest.identity
            || new.target != candidate.asset.target
        {
            return Err(crate::invalid(
                "Staged setup payload does not match the selected release identity/target",
            ));
        }
        crate::download::verify_bundle(stage, &new)?;
        Ok(new)
    })();
    // Cleanup happens before releasing Preparation's reservation. It cannot
    // remove another updater's staging tree or touch a pending helper journal.
    result.map_err(|mut error: UpdateError| {
        if let Err(cleanup) = exe::clear_staged_payload(root) {
            error
                .message
                .push_str(&format!(" (could not clean setup staging: {cleanup})"));
        }
        error
    })
}

#[cfg(all(test, windows))]
#[path = "manager_acceptance.rs"]
mod acceptance;
