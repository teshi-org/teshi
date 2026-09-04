//! Atomic file read/write with advisory locking for `.teshi/` state files.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use fd_lock::RwLock;
use serde::{de::DeserializeOwned, Serialize};

fn lock_file_path(path: &Path) -> std::path::PathBuf {
    path.with_extension("lock")
}

/// Runs `f` while holding an exclusive advisory lock for `path`.
///
/// The lock file is `path` with its extension replaced by `lock` (for example
/// `_teshi.json` → `_teshi.lock`). Nested calls on the same path deadlock.
///
/// # Errors
///
/// Returns an error when the lock file cannot be created or the exclusive lock
/// cannot be acquired, or when `f` fails.
pub(crate) fn with_exclusive_lock<T>(path: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    let lock_path = lock_file_path(path);
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .context("open lock file")?;
    let mut lock = RwLock::new(lock_file);
    let _guard = lock.write().context("acquire exclusive lock")?;
    let result = f();
    drop(_guard);
    fs::remove_file(&lock_path).ok();
    result
}

/// Writes serializable JSON to `path` via temp file + rename without taking a lock.
///
/// Callers that already hold [`with_exclusive_lock`] on `path` must use this
/// instead of [`write_atomic`] to avoid deadlock.
///
/// # Errors
///
/// Returns an error when serialization, the temp write, or the rename fails.
pub(crate) fn write_json_unlocked<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create parent directory")?;
    }

    let tmp = path.with_extension("tmp");
    let data = serde_json::to_string_pretty(value).context("serialize json")?;
    fs::write(&tmp, &data).context("write temp file")?;
    fs::rename(&tmp, path).context("rename temp to target")?;
    Ok(())
}

/// Atomically write serializable data to `path` with an exclusive lock.
///
/// Writes to a temp file (`path.tmp`), acquires an exclusive lock on `path.lock`,
/// then renames the temp file into place.  The same-directory rename is atomic on
/// all major operating systems.
pub fn write_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    with_exclusive_lock(path, || write_json_unlocked(path, value))
}

/// Read and deserialize a file with a shared lock.
pub fn read_locked<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let lock_path = path.with_extension("lock");
    let lock_file = if lock_path.exists() {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .context("open lock file")?
    } else {
        // No lock file exists yet — create one for the read
        fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .context("create lock file")?
    };
    let lock = RwLock::new(lock_file);
    let _guard = lock.read().context("acquire shared lock")?;

    let data = fs::read_to_string(path).context("read file")?;
    serde_json::from_str(&data).context("parse json")
}
