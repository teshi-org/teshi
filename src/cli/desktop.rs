use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// Spawns the `teshi-desktop` binary with the same project flags as this CLI.
pub fn spawn_desktop(project: Option<&str>, path: Option<&str>) -> Result<()> {
    let binary = resolve_desktop_binary()?;
    let mut cmd = Command::new(&binary);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if let Some(project) = project {
        cmd.arg("--project").arg(project);
    } else if let Some(path) = path {
        cmd.arg(path);
    }

    let status = cmd
        .status()
        .with_context(|| format!("failed to spawn {}", binary.display()))?;
    if !status.success() {
        bail!(
            "{} exited with {}",
            binary.display(),
            status
                .code()
                .map_or_else(|| "signal".to_string(), |c| c.to_string())
        );
    }
    Ok(())
}

/// Resolves `teshi-desktop` next to the current executable, then on `PATH`.
fn resolve_desktop_binary() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let name = if cfg!(windows) {
            "teshi-desktop.exe"
        } else {
            "teshi-desktop"
        };
        let sibling = dir.join(name);
        if sibling.is_file() {
            return Ok(sibling);
        }
    }

    Ok(PathBuf::from(if cfg!(windows) {
        "teshi-desktop.exe"
    } else {
        "teshi-desktop"
    }))
}
