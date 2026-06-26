//! Daemon manifest and project root resolution.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::fs_util;

/// Written by the daemon; read by all clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonManifest {
    pub pid: u32,
    pub port: u16,
    pub started: DateTime<Utc>,
}

impl DaemonManifest {
    /// Returns the path to `daemon.json` within the project's `.teshi/` directory.
    pub fn path(project_root: &Path) -> PathBuf {
        project_root.join(".teshi").join("daemon.json")
    }

    /// Load the manifest if it exists and is valid JSON.
    pub fn load(project_root: &Path) -> Option<Self> {
        let path = Self::path(project_root);
        if !path.exists() {
            return None;
        }
        fs_util::read_locked(&path).ok()
    }

    /// Persist the manifest atomically.
    pub fn save(&self, project_root: &Path) -> Result<()> {
        let path = Self::path(project_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        fs_util::write_atomic(&path, self)
    }

    /// Check whether the daemon is still alive (pid exists + port is listening).
    pub fn is_alive(&self) -> bool {
        // Quick TCP probe — if the port accepts a connection, the daemon is alive.
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
    let path = DaemonManifest::path(project_root);
    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&path.with_extension("lock")).ok();
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
