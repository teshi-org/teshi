//! Persistent failures from metadata lookup through helper commit.

use crate::{
    Result, UpdateError,
    github::Candidate,
    storage::{install_key, private_directory, state_directory, write_json},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Last failed phase and instructions for the next attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Failure {
    /// Unix time of the failure.
    pub timestamp: i64,
    /// Metadata, download, verification, setup, staging, handoff or commit.
    pub stage: String,
    /// Stable error code and readable cause.
    pub error: UpdateError,
    /// Pinned payload identity, when metadata lookup succeeded.
    pub payload: Option<Candidate>,
    /// Whether a verified download was retained when this failure occurred.
    pub cached_payload: bool,
    /// Expected behavior when retrying, subject to integrity revalidation.
    pub next_attempt: String,
}

fn path(installation: &Path) -> Result<PathBuf> {
    Ok(state_directory()?.join(format!("failure-{}.json", install_key(installation))))
}

/// Reads the last failure for an installation or unmarked executable.
pub fn last_failure(installation: &Path) -> Option<Failure> {
    fs::read(path(installation).ok()?)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
}

/// Records a failure without touching installation files or pending journals.
///
/// # Errors
/// Returns filesystem or serialization errors.
pub fn record(
    installation: &Path,
    stage: &str,
    error: &UpdateError,
    candidate: Option<&Candidate>,
    cached: bool,
) -> Result<()> {
    let path = path(installation)?;
    private_directory(
        path.parent()
            .ok_or_else(|| crate::invalid("Missing diagnostic directory"))?,
    )?;
    write_json(
        &path,
        &Failure {
            timestamp: chrono::Utc::now().timestamp(),
            stage: stage.into(),
            error: error.clone(),
            payload: candidate.cloned(),
            cached_payload: cached,
            next_attempt: if cached {
                "Cached payload retained. --retry-install works offline after revalidation; teshi update also reuses valid cache.".into()
            } else if stage == "download" {
                "Partial bytes retained when safe. teshi update resumes with Range, or restarts if unsupported; --retry-install requires a complete verified payload.".into()
            } else {
                "Run teshi update to resolve metadata and download missing or invalid payloads."
                    .into()
            },
        },
    )
}

/// Clears a prior failure after a successful new operation.
///
/// # Errors
/// Returns filesystem errors other than an absent diagnostic file.
pub fn clear(installation: &Path) -> Result<()> {
    match fs::remove_file(path(installation)?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
