use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

pub mod bdd;
pub mod driver;

pub const SCENARIO_MOVE_SELECTION_DOWN: &str = "Move selection down in tree";

/// Returns `true` when this host can run TUI end-to-end tests.
///
/// The harness uses a Unix PTY (`portable-pty`); Windows is intentionally unsupported so CI and
/// local runs use Linux (or WSL) only.
pub fn tui_e2e_host_supported() -> bool {
    cfg!(target_os = "linux")
}

/// Returns `true` if `scenario` is implemented by the TUI E2E step crate.
pub fn is_tui_scenario(scenario: &str) -> bool {
    scenario == SCENARIO_MOVE_SELECTION_DOWN
}

/// Returns `true` if the scenario should be executed on this host.
pub fn supports_scenario(scenario: &str) -> bool {
    tui_e2e_host_supported() && is_tui_scenario(scenario)
}

pub fn run_scenario(scenario: &str, teshi_bin: &Path) -> Result<()> {
    match scenario {
        SCENARIO_MOVE_SELECTION_DOWN => run_move_selection_down(teshi_bin),
        _ => bail!("unsupported scenario: {scenario}"),
    }
}

fn run_move_selection_down(teshi_bin: &Path) -> Result<()> {
    let repo_root = infer_repo_root(teshi_bin)?;
    let args = ["tests/features/"];
    let mut tui = driver::TuiDriver::spawn(teshi_bin, &args, &repo_root)?;

    tui.wait_for_output(Duration::from_millis(5000))?;
    tui.send_text("2")?;
    tui.wait_for_contains("MindMap", Duration::from_millis(5000))?;

    let before = tui.snapshot();
    tui.send_key_down()?;
    tui.wait_for_change(&before, Duration::from_millis(3000))?;

    tui.shutdown(Duration::from_millis(1500))?;
    Ok(())
}

fn infer_repo_root(teshi_bin: &Path) -> Result<PathBuf> {
    if let Some(root) = teshi_bin
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
    {
        return Ok(root.to_path_buf());
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .ancestors()
        .nth(3)
        .context("failed to infer repo root")?;
    Ok(root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn e2e_move_selection_down() -> Result<()> {
        let teshi_bin = locate_teshi_bin()?;
        run_scenario(SCENARIO_MOVE_SELECTION_DOWN, &teshi_bin)
    }

    fn locate_teshi_bin() -> Result<PathBuf> {
        if let Ok(bin) = std::env::var("TESHI_BIN") {
            let path = PathBuf::from(bin);
            if path.exists() {
                return Ok(path.canonicalize().unwrap_or(path));
            }
            bail!("TESHI_BIN points to missing path: {}", path.display());
        }
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .ancestors()
            .nth(3)
            .context("failed to infer repo root")?;
        let candidate = root.join("target").join("debug").join("teshi");
        if candidate.exists() {
            return Ok(candidate);
        }
        bail!(
            "teshi binary not found at {}; run `cargo build` or set TESHI_BIN",
            candidate.display()
        );
    }
}
