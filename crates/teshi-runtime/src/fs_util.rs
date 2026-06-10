//! Atomic file read/write with advisory locking for `.teshi/` state files.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use fd_lock::RwLock;
use serde::{de::DeserializeOwned, Serialize};

/// Atomically write serializable data to `path` with an exclusive lock.
///
/// Writes to a temp file (`path.tmp`), acquires an exclusive lock on `path.lock`,
/// then renames the temp file into place.  The same-directory rename is atomic on
/// all major operating systems.
pub fn write_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create parent directory")?;
    }

    let tmp = path.with_extension("tmp");
    let data = serde_json::to_string_pretty(value).context("serialize json")?;
    fs::write(&tmp, &data).context("write temp file")?;

    let lock_path = path.with_extension("lock");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .context("open lock file")?;
    let mut lock = RwLock::new(lock_file);
    let _guard = lock.write().context("acquire exclusive lock")?;

    fs::rename(&tmp, path).context("rename temp to target")?;

    // Drop lock guard, then remove lock file
    drop(_guard);
    fs::remove_file(&lock_path).ok();

    Ok(())
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
            .read(true)
            .write(true)
            .open(&lock_path)
            .context("create lock file")?
    };
    let lock = RwLock::new(lock_file);
    let _guard = lock.read().context("acquire shared lock")?;

    let data = fs::read_to_string(path).context("read file")?;
    Ok(serde_json::from_str(&data).context("parse json")?)
}
