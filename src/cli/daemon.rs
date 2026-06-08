use std::path::Path;

use anyhow::{Context, Result};
use teshi_runtime::{find_project_root, DaemonManifest};

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
    let exe = std::env::current_exe().context("resolve current executable")?;

    // On Windows, delegate to PowerShell Start-Process to fully detach.
    // Calling CreateProcess on our own exe from any thread in the same
    // process causes the parent to hang on exit.  PowerShell's Start-Process
    // runs in a separate powershell.exe process, avoiding the deadlock.
    #[cfg(windows)]
    {
        let ps_cmd = format!(
            "Start-Process -FilePath '{}' -ArgumentList '--daemon-internal','--project-root','{}','--port','{}' -WindowStyle Hidden",
            exe.display(),
            project_root.display(),
            port
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
        let args: Vec<String> = vec![
            "--daemon-internal".to_string(),
            "--project-root".to_string(),
            project_root.to_string_lossy().to_string(),
            "--port".to_string(),
            port.to_string(),
        ];
        let mut cmd = std::process::Command::new(&exe);
        cmd.args(&args);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        cmd.spawn().context("spawn daemon process")?;
    }

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
