//! Persistent, per-installation automatic check policy.

use crate::{
    Result,
    install::Installation,
    storage::{StateLock, install_key, private_directory, write_json},
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use teshi_core::version::ReleaseChannel;

/// User preferences; checks never grant installation permission.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    /// Check-and-notify for supported installed release builds.
    pub auto_check: bool,
    /// Explicit channel preference used by manual checks and installs.
    pub channel: Option<ReleaseChannel>,
}
impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            auto_check: true,
            channel: None,
        }
    }
}

/// Loads update preferences, using defaults when no file exists.
///
/// # Errors
/// Returns invalid JSON or filesystem errors rather than overwriting preferences.
pub fn load_settings(state: &Path) -> Result<UpdateSettings> {
    match std::fs::read(state.join("settings.json")) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(UpdateSettings::default()),
        Err(e) => Err(e.into()),
    }
}

/// Persists preferences without changing installed build/channel identity.
///
/// # Errors
/// Returns filesystem or serialization failures.
pub fn save_settings(state: &Path, settings: &UpdateSettings) -> Result<()> {
    private_directory(state)?;
    write_json(&state.join("settings.json"), settings)
}

/// Claims a due background check across desktop processes sharing an installation.
///
/// # Errors
/// Returns a Busy error for a competing policy writer or other storage failures.
pub fn claim_check(
    state: &Path,
    installation: &Installation,
    settings: &UpdateSettings,
    now: u64,
) -> Result<bool> {
    let Some(bundle) = installation.bundle.as_ref() else {
        return Ok(false);
    };
    if !settings.auto_check || !installation.can_install() {
        return Ok(false);
    }
    let interval = match bundle.identity.channel {
        ReleaseChannel::Stable => 3600,
        ReleaseChannel::Nightly => 900,
        ReleaseChannel::Dev => return Ok(false),
    };
    let Some(root) = installation.root.as_deref() else {
        return Ok(false);
    };
    private_directory(state)?;
    let key = install_key(root);
    let _lock = StateLock::acquire(&state.join(format!("poll-{key}.lock")))?;
    let path = state.join(format!("poll-{key}.json"));
    let next: u64 = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(0);
    if now < next {
        return Ok(false);
    }
    // Deterministic per-install jitter avoids synchronized checks after launch.
    let jitter = u64::from_str_radix(&key[..4], 16).unwrap_or(0) % 60;
    write_json(&path, &(now.saturating_add(interval + jitter)))?;
    Ok(true)
}
