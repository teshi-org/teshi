//! Native application updates, shared by CLI and desktop without UI dependencies.

pub mod download;
pub mod exe;
pub mod github;
pub mod install;
pub mod lifecycle;
pub mod manager;
pub mod manifest;
pub mod policy;
pub mod release;
pub mod storage;
pub mod transaction;

use serde::{Deserialize, Serialize};

/// Stable error category for automation and UI presentation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Invalid or inconsistent release metadata.
    InvalidManifest,
    /// Network or GitHub API failure.
    Network,
    /// GitHub throttled the client.
    RateLimited,
    /// Artifact checksum does not match.
    Verification,
    /// Installation ownership or target is unsupported.
    Unsupported,
    /// Another operation or application holds the install.
    Busy,
    /// Filesystem operation failed.
    Io,
    /// User cancelled or confirmation was not supplied.
    Cancelled,
    /// Installer/helper operation failed.
    Installation,
}

/// Serializable update error without non-serializable anyhow internals.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct UpdateError {
    /// Programmatic category.
    pub code: ErrorCode,
    /// Actionable explanation.
    pub message: String,
}

impl UpdateError {
    /// Creates an error suitable for both CLI and UI.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<std::io::Error> for UpdateError {
    fn from(error: std::io::Error) -> Self {
        Self::new(ErrorCode::Io, error.to_string())
    }
}

impl From<serde_json::Error> for UpdateError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(ErrorCode::InvalidManifest, error.to_string())
    }
}

/// Result used throughout the native updater.
pub type Result<T> = std::result::Result<T, UpdateError>;

/// Phase names shared by persistent results and native views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    /// No requested operation.
    Idle,
    /// Fetching release metadata.
    Checking,
    /// No newer compatible release.
    UpToDate,
    /// A candidate is available.
    UpdateAvailable,
    /// Fetching a payload.
    Downloading,
    /// Validating downloaded data.
    Verifying,
    /// Payload validated; installed files remain unchanged.
    Staged,
    /// Helper accepted work and is waiting for participants.
    WaitingForExit,
    /// Replacement in progress.
    Installing,
    /// Replacement succeeded; application restart remains.
    ReadyToRestart,
    /// Installation finished.
    Completed,
    /// Operation cannot proceed under current conditions.
    Blocked,
    /// Operation failed.
    Errored,
}

/// Progress event; unknown total download size is represented by None.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateEvent {
    /// Current phase.
    pub status: UpdateStatus,
    /// Fraction in [0, 1], when known.
    pub progress: Option<f32>,
    /// Human-readable progress or error detail.
    pub detail: String,
}

pub(crate) fn invalid(message: impl Into<String>) -> UpdateError {
    UpdateError::new(ErrorCode::InvalidManifest, message)
}
