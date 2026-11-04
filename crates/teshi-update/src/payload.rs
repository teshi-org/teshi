//! Persistent payloads, independent of disposable installation transactions.

use crate::{
    ErrorCode, Result, UpdateError, UpdateEvent, UpdateStatus, download,
    github::{Candidate, Http},
    install::reject_links,
    storage::{StateLock, private_directory},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

/// Validates cached metadata using the same release contract as online discovery.
///
/// # Errors
/// Rejects mismatched selected assets, release identities and targets.
pub fn validate_candidate(candidate: &Candidate) -> Result<()> {
    candidate.manifest.validate()?;
    let selected = serde_json::to_value(&candidate.asset)?;
    if !candidate
        .manifest
        .assets
        .iter()
        .any(|asset| serde_json::to_value(asset).is_ok_and(|value| value == selected))
    {
        return Err(crate::invalid(
            "Selected payload does not match release manifest",
        ));
    }
    Ok(())
}

/// A payload reservation retained through setup and helper handoff.
pub struct Payload {
    directory: PathBuf,
    key: String,
    _lock: StateLock,
}

impl Payload {
    /// Reserves a persistent entry keyed by release, asset and checksum.
    ///
    /// # Errors
    /// Returns manifest, filesystem or Busy errors; refuses linked cache paths.
    pub fn reserve(cache: &Path, candidate: &Candidate) -> Result<Self> {
        validate_candidate(candidate)?;
        let key = format!(
            "{}-{}-{}",
            candidate.release_id, candidate.asset_id, candidate.asset.sha256
        );
        let relative = PathBuf::from("payloads").join(&key);
        reject_links(cache, &relative)?;
        private_directory(cache)?;
        let directory = cache.join(relative);
        private_directory(&directory)?;
        reject_links(&directory, Path::new("payload.lock"))?;
        let lock = StateLock::acquire(&directory.join("payload.lock"))?;
        Ok(Self {
            directory,
            key,
            _lock: lock,
        })
    }

    /// Obtains and revalidates a payload. Offline mode never calls the transport.
    /// Network failures have bounded retries and leave resumable bytes on disk.
    ///
    /// # Errors
    /// Returns verification, cancellation, network or filesystem failures.
    pub fn obtain(
        &self,
        http: &impl Http,
        candidate: &Candidate,
        offline: bool,
        cancel: &AtomicBool,
        emit: &mut impl FnMut(UpdateEvent),
    ) -> Result<PathBuf> {
        validate_candidate(candidate)?;
        if self.key
            != format!(
                "{}-{}-{}",
                candidate.release_id, candidate.asset_id, candidate.asset.sha256
            )
        {
            return Err(crate::invalid(
                "Payload does not match its cache reservation",
            ));
        }
        let package = self.directory.join(&candidate.asset.name);
        reject_links(&self.directory, Path::new(&candidate.asset.name))?;
        if package.exists() {
            emit(UpdateEvent {
                status: UpdateStatus::Verifying,
                progress: None,
                detail: "Revalidating cached payload".into(),
            });
            if package.metadata()?.len() == candidate.asset.size
                && download::file_hash(&package)? == candidate.asset.sha256
            {
                return Ok(package);
            }
            if offline {
                return Err(UpdateError::new(
                    ErrorCode::Verification,
                    "Cached payload failed size/SHA256 validation; run teshi update to download it again",
                ));
            }
            fs::remove_file(&package)?;
            emit(UpdateEvent {
                status: UpdateStatus::Verifying,
                progress: None,
                detail: "Cached payload failed validation; downloading a fresh copy".into(),
            });
        }
        if offline {
            return Err(UpdateError::new(
                ErrorCode::Verification,
                "No verified cached payload; run teshi update to download it first",
            ));
        }
        let partial = self.directory.join("payload.part");
        let mut attempt = 0;
        loop {
            match download::download(http, candidate, &partial, cancel, emit) {
                Ok(()) => {
                    fs::rename(&partial, &package)?;
                    return Ok(package);
                }
                Err(error)
                    if attempt < 2
                        && matches!(error.code, ErrorCode::Network | ErrorCode::Verification) =>
                {
                    emit(UpdateEvent {
                        status: UpdateStatus::Downloading,
                        progress: None,
                        detail: format!("{error}; retrying transfer ({}/3)", attempt + 2),
                    });
                    std::thread::sleep(Duration::from_millis(200 * (attempt + 1)));
                    attempt += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }
}
