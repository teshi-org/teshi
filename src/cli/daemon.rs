use std::path::Path;

use anyhow::{Context, Result};
use teshi_runtime::{find_project_root, spawn_daemon_background, DaemonManifest};

use super::DaemonCommand;

/// Handles `teshi daemon start` and `teshi daemon stop`.
pub fn handle_daemon_command(action: &DaemonCommand) -> Result<()> {
    let project_root = find_project_root(None)
        .context("no .teshi/ directory found; run `teshi web`, `teshi desktop`, or `teshi daemon start --project PATH` from a project directory")?;
    match action {
        DaemonCommand::Start => daemon_start(&project_root),
        DaemonCommand::Stop => daemon_stop(&project_root),
    }
}

fn daemon_start(project_root: &Path) -> Result<()> {
    let manifest_path = project_root.join(".teshi").join("daemon.json");
    if let Ok(data) = std::fs::read_to_string(&manifest_path) {
        if let Ok(m) = serde_json::from_str::<DaemonManifest>(&data) {
            if m.is_alive() {
                eprintln!("daemon already running (pid {}, port {})", m.pid, m.port);
                return Ok(());
            }
        }
    }

    let port = teshi_runtime::pick_free_port()?;
    spawn_daemon_background(project_root, port, None)?;
    eprintln!("daemon spawning on port {port}");
    Ok(())
}

fn daemon_stop(project_root: &Path) -> Result<()> {
    let manifest = DaemonManifest::load(project_root)
        .context("daemon not running (no daemon.json found)")?;

    if !manifest.is_alive() {
        eprintln!("daemon was not running; cleaning up stale manifest");
        teshi_runtime::remove_daemon_manifest(project_root);
        return Ok(());
    }

    // Try graceful shutdown via HTTP
    let url = format!("http://127.0.0.1:{}/api/v1/daemon/shutdown", manifest.port);
    let http_ok = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()
        .and_then(|client| client.post(&url).send().ok())
        .is_some();

    if !http_ok {
        // Fallback: force-kill the daemon process
        eprintln!("graceful shutdown failed, killing daemon (pid {})", manifest.pid);
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &manifest.pid.to_string(), "/F"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        #[cfg(not(windows))]
        {
            let _ = std::process::Command::new("kill")
                .args(["-9", &manifest.pid.to_string()])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        // Brief wait for process to die
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    // Clean up manifest
    teshi_runtime::remove_daemon_manifest(project_root);
    println!("daemon stopped (was pid {}, port {})", manifest.pid, manifest.port);
    Ok(())
}
