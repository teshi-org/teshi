use std::path::Path;

use anyhow::{Context, Result};
use teshi_engine::{DaemonManifest, DaemonManifestExt, find_project_root, spawn_daemon_background};

use super::DaemonCommand;

/// Handles `teshi daemon start` and `teshi daemon stop`.
pub fn handle_daemon_command(action: &DaemonCommand) -> Result<()> {
    let project_root = find_project_root(None)
        .context("no .teshi/ directory found; run `teshi web`, `teshi desktop`, or `teshi daemon start --project PATH` from a project directory")?;
    match action {
        DaemonCommand::Start => daemon_start(&project_root),
        DaemonCommand::Stop { force } => daemon_stop(&project_root, *force),
    }
}

fn daemon_start(project_root: &Path) -> Result<()> {
    let manifest_path = project_root.join(".teshi").join("daemon.json");
    if let Ok(data) = std::fs::read_to_string(&manifest_path)
        && let Ok(m) = serde_json::from_str::<DaemonManifest>(&data)
        && m.is_daemon_alive()
    {
        eprintln!("daemon already running (pid {}, port {})", m.pid, m.port);
        return Ok(());
    }

    let port = teshi_engine::pick_free_port()?;
    spawn_daemon_background(project_root, port, "0.0.0.0", None)?;
    eprintln!("daemon spawning on port {port}");
    Ok(())
}

fn daemon_stop(project_root: &Path, force: bool) -> Result<()> {
    use std::time::Duration;
    use teshi_update::processes::{self, DaemonEndpoint};
    let manifest = DaemonManifest::load_manifest(project_root)
        .context("daemon not running (no daemon.json found)")?;
    let Some(process) = processes::inspect(manifest.pid)? else {
        teshi_engine::remove_daemon_manifest_if_matches(project_root, &manifest)?;
        println!("Daemon was not running; stale manifest removed");
        return Ok(());
    };
    let executable = std::fs::canonicalize(std::env::current_exe()?)?;
    anyhow::ensure!(
        processes::same_image(&process.executable, &executable),
        "Daemon executable does not belong to this Teshi installation; refusing shutdown"
    );
    if let Some(expected) = &manifest.process_start {
        anyhow::ensure!(
            expected == &process.started,
            "Daemon PID was reused; refusing shutdown and retaining manifest"
        );
    }
    let endpoint = DaemonEndpoint {
        process: process.clone(),
        port: manifest.port,
        project_root: Some(std::fs::canonicalize(project_root)?),
    };
    let graceful = processes::request_daemon_shutdown(&endpoint);
    // A failed request is never evidence of exit. Force is explicit and requires
    // the recorded process generation even when the endpoint cannot respond.
    if !processes::is_running(&process)?
        || (graceful.is_ok()
            && processes::wait_for_exit(
                &process,
                if force {
                    Duration::from_secs(3)
                } else {
                    Duration::from_secs(10)
                },
            )?)
    {
        teshi_engine::remove_daemon_manifest_if_matches(project_root, &manifest)?;
        println!(
            "Daemon stopped (pid {}, port {}); process exit confirmed",
            manifest.pid, manifest.port
        );
        return Ok(());
    }
    if !force && let Err(error) = &graceful {
        return Err(error.clone().into());
    }
    anyhow::ensure!(
        force,
        "Daemon did not exit; manifest retained. Retry with --force to close this verified process"
    );
    anyhow::ensure!(
        manifest.process_start.as_deref() == Some(process.started.as_str()),
        "No matching process creation identity in daemon manifest; refusing unsafe force close"
    );
    processes::terminate(&process)?;
    teshi_engine::remove_daemon_manifest_if_matches(project_root, &manifest)?;
    println!(
        "Daemon force-closed (pid {}, port {}); process exit confirmed",
        manifest.pid, manifest.port
    );
    Ok(())
}
