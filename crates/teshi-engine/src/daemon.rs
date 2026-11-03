//! Daemon manifest and project root resolution.
//! Pure DTO lives in `teshi-core::daemon`.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::fs_util;

pub use teshi_core::daemon::DaemonManifest;

/// Extension trait providing I/O methods for [`DaemonManifest`].
/// Import this trait to call `.load()`, `.save()`, `.is_alive()`, and
/// `DaemonManifest::path()` from outside `teshi-engine`.
pub trait DaemonManifestExt {
    /// Returns the path to `daemon.json` within the project's `.teshi/` directory.
    fn manifest_path(project_root: &Path) -> PathBuf;
    /// Load the manifest if it exists and is valid JSON.
    fn load_manifest(project_root: &Path) -> Option<DaemonManifest>;
    /// Persist the manifest atomically.
    fn save_manifest(&self, project_root: &Path) -> Result<()>;
    /// Check whether the daemon is still alive (pid exists + port is listening).
    fn is_daemon_alive(&self) -> bool;
}

impl DaemonManifestExt for DaemonManifest {
    fn manifest_path(project_root: &Path) -> PathBuf {
        project_root.join(".teshi").join("daemon.json")
    }

    fn load_manifest(project_root: &Path) -> Option<DaemonManifest> {
        let path = Self::manifest_path(project_root);
        if !path.exists() {
            return None;
        }
        fs_util::read_locked(&path).ok()
    }

    fn save_manifest(&self, project_root: &Path) -> Result<()> {
        let path = Self::manifest_path(project_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        fs_util::write_atomic(&path, self)
    }

    fn is_daemon_alive(&self) -> bool {
        let addr = format!("127.0.0.1:{}", self.port);
        TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(200)).is_ok()
    }
}

/// Walk up from `start_dir` (or CWD) until a `.teshi/` directory is found.
pub fn find_project_root(start_dir: Option<&Path>) -> Option<PathBuf> {
    let mut current = start_dir
        .map(|p| p.to_path_buf())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    // Canonicalize if possible for a cleaner walk
    if let Ok(canonical) = current.canonicalize() {
        current = canonical;
    }

    loop {
        if current.join(".teshi").is_dir() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Returns a free TCP port on loopback by binding to port 0.
pub fn pick_free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").context("find free loopback port")?;
    Ok(listener.local_addr()?.port())
}

/// Remove the daemon manifest (called on daemon shutdown).
pub fn remove_daemon_manifest(project_root: &Path) {
    let path = DaemonManifest::manifest_path(project_root);
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(path.with_extension("lock")).ok();
}

/// Removes a stopped daemon's manifest only if no replacement has published one.
/// Keeps the advisory lock file so concurrent writers retain one lock identity.
///
/// # Errors
/// Returns lock, manifest-read or deletion errors.
pub fn remove_daemon_manifest_if_matches(
    project_root: &Path,
    expected: &DaemonManifest,
) -> Result<()> {
    let path = DaemonManifest::manifest_path(project_root);
    fs_util::with_exclusive_lock(&path, || {
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let actual: DaemonManifest = serde_json::from_slice(&data)?;
        if actual.pid == expected.pid
            && actual.port == expected.port
            && actual.started == expected.started
            && actual.process_start == expected.process_start
        {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    })
}

/// Spawn the daemon as a detached background process using the current executable.
///
/// On Windows this delegates to PowerShell `Start-Process` to avoid a self-spawn
/// deadlock (the parent process hangs if it creates a direct child of itself).
///
/// `dist` is optional — pass `Some(...)` when the frontend dist directory is known
/// (e.g. `teshi web`), or `None` when the daemon should resolve it itself.
pub fn spawn_daemon_background(
    project_root: &Path,
    port: u16,
    host: &str,
    dist: Option<&Path>,
) -> Result<()> {
    let exe = std::env::current_exe().context("resolve current executable")?;

    let mut args: Vec<String> = vec![
        "--daemon-internal".to_string(),
        "--project-root".to_string(),
        project_root.to_string_lossy().to_string(),
        "--port".to_string(),
        port.to_string(),
        "--host".to_string(),
        host.to_string(),
    ];
    if let Some(d) = dist {
        args.push("--dist".to_string());
        args.push(d.to_string_lossy().to_string());
    }

    #[cfg(windows)]
    {
        // Delegate to PowerShell Start-Process to avoid self-spawn deadlock.
        // NOTE: -ArgumentList must be a single string (not comma-separated)
        // because comma-separated args don't handle spaces in paths correctly.
        let arg_str = args
            .iter()
            .map(|a| {
                if a.contains(' ') {
                    // Double-quote paths with spaces
                    format!("\"{}\"", a.replace('"', "\\\""))
                } else {
                    a.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let ps_cmd = format!(
            "Start-Process -FilePath '{}' -ArgumentList '{}' -WindowStyle Hidden",
            exe.display(),
            arg_str.replace('\'', "''"),
        );
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("spawn daemon via powershell")?;
    }

    #[cfg(not(windows))]
    {
        let mut cmd = std::process::Command::new(&exe);
        cmd.args(&args);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        cmd.spawn().context("spawn daemon process")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_daemon_cleanup_preserves_a_replacement_manifest_and_lock() {
        let root = tempfile::tempdir().unwrap();
        let old = DaemonManifest {
            pid: 100,
            port: 200,
            started: chrono::Utc::now(),
            process_start: Some("old".into()),
        };
        old.save_manifest(root.path()).unwrap();
        let replacement = DaemonManifest {
            pid: 101,
            process_start: Some("new".into()),
            ..old.clone()
        };
        replacement.save_manifest(root.path()).unwrap();
        remove_daemon_manifest_if_matches(root.path(), &old).unwrap();
        assert_eq!(DaemonManifest::load_manifest(root.path()).unwrap().pid, 101);
        remove_daemon_manifest_if_matches(root.path(), &replacement).unwrap();
        assert!(!DaemonManifest::manifest_path(root.path()).exists());
        assert!(DaemonManifest::manifest_path(root.path())
            .with_extension("lock")
            .exists());
    }
}
