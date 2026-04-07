use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use cucumber::{World, given, then, when};

use crate::SCENARIO_MOVE_SELECTION_DOWN;
use crate::driver::TuiDriver;

const STARTUP_TIMEOUT: Duration = Duration::from_millis(5000);
const CHANGE_TIMEOUT: Duration = Duration::from_millis(3000);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Default, cucumber::World)]
pub struct TuiWorld {
    teshi_bin: Option<PathBuf>,
    repo_root: Option<PathBuf>,
    tui: Option<TuiDriver>,
    before_snapshot: Option<String>,
}

impl fmt::Debug for TuiWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TuiWorld")
            .field("teshi_bin", &self.teshi_bin)
            .field("repo_root", &self.repo_root)
            .field("has_tui", &self.tui.is_some())
            .field("has_before_snapshot", &self.before_snapshot.is_some())
            .finish()
    }
}

impl Drop for TuiWorld {
    fn drop(&mut self) {
        if let Some(tui) = self.tui.take() {
            let _ = tui.shutdown(Duration::from_millis(500));
        }
    }
}

#[given("the MindMap tree is displayed")]
async fn mindmap_tree_is_displayed(world: &mut TuiWorld) -> Result<()> {
    if world.tui.is_some() {
        return Ok(());
    }

    let teshi_bin = locate_teshi_bin()?;
    let repo_root = crate::infer_repo_root(&teshi_bin)?;
    let mut tui = TuiDriver::spawn(&teshi_bin, &["tests/features/"], &repo_root)?;

    tui.wait_for_output(STARTUP_TIMEOUT)?;
    tui.send_text("2")?;
    tui.wait_for_contains("MindMap", STARTUP_TIMEOUT)?;

    world.teshi_bin = Some(teshi_bin);
    world.repo_root = Some(repo_root);
    world.tui = Some(tui);
    Ok(())
}

#[when("I press the down arrow key")]
async fn press_down(world: &mut TuiWorld) -> Result<()> {
    let tui = world
        .tui
        .as_mut()
        .context("TUI not initialized; missing Given step")?;
    let before = tui.snapshot();
    tui.send_key_down()?;
    world.before_snapshot = Some(before);
    Ok(())
}

#[then("the selection moves to the next visible tree node")]
async fn selection_moves(world: &mut TuiWorld) -> Result<()> {
    let before = world
        .before_snapshot
        .as_deref()
        .context("missing snapshot before move")?;
    if let Some(tui) = world.tui.as_mut() {
        tui.wait_for_change(before, CHANGE_TIMEOUT)?;
    } else {
        anyhow::bail!("TUI not initialized; missing Given step");
    }

    if let Some(tui) = world.tui.take() {
        tui.shutdown(SHUTDOWN_TIMEOUT)?;
    }
    Ok(())
}

pub async fn run_move_selection_down() {
    if !crate::tui_e2e_host_supported() {
        eprintln!("Skipping: TUI E2E tests run on Linux only");
        return;
    }
    let feature_path = feature_path("mindmap.feature");
    let _ = TuiWorld::cucumber()
        .filter_run_and_exit(feature_path, |_, _, scenario| {
            scenario.name == SCENARIO_MOVE_SELECTION_DOWN
        })
        .await;
}

fn feature_path(file: &str) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .ancestors()
        .nth(3)
        .expect("failed to locate repo root");
    root.join("tests").join("features").join(file)
}

fn locate_teshi_bin() -> Result<PathBuf> {
    if let Ok(bin) = std::env::var("TESHI_BIN") {
        let path = PathBuf::from(bin);
        if path.exists() {
            return Ok(path.canonicalize().unwrap_or(path));
        }
        anyhow::bail!("TESHI_BIN points to missing path: {}", path.display());
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
    anyhow::bail!(
        "teshi binary not found at {}; run `cargo build` or set TESHI_BIN",
        candidate.display()
    );
}
