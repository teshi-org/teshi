//! Per-user update state and cross-process locks.

use crate::{ErrorCode, Result, UpdateError};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// Resolves updater state independently of projects and credential stores.
///
/// # Errors
/// Returns an error if the OS user directory is unavailable.
pub fn state_directory() -> Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")));
    base.map(|p| p.join("teshi/updates")).ok_or_else(|| {
        UpdateError::new(ErrorCode::Io, "Cannot resolve user update state directory")
    })
}

/// Creates a state directory with owner-only Unix permissions.
///
/// # Errors
/// Propagates filesystem failures or existing symbolic links.
pub fn private_directory(path: &Path) -> Result<()> {
    if path
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(UpdateError::new(
            ErrorCode::Io,
            "Update state directory is a link",
        ));
    }
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Hashes a canonical installation root for per-install state naming.
pub fn install_key(root: &Path) -> String {
    let path = root.to_string_lossy();
    #[cfg(windows)]
    let path = path.to_lowercase();
    format!("{:x}", Sha256::digest(path.as_bytes()))
}

/// An exclusive OS lock held until drop, including during errors.
pub struct StateLock {
    _file: File,
}

impl StateLock {
    /// Acquires an exclusive lock without waiting.
    ///
    /// # Errors
    /// Returns Busy if another process holds the lock, or an I/O failure.
    pub fn acquire(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|error| {
            UpdateError::new(ErrorCode::Busy, format!("Update state is in use: {error}"))
        })?;
        Ok(Self { _file: file })
    }
}

/// Atomically writes and flushes JSON using a temporary file in the same directory.
///
/// # Errors
/// Returns serialization or filesystem errors; leaves the previous file intact.
pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| UpdateError::new(ErrorCode::Io, "Missing state parent"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| UpdateError::from(e.error))?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}
