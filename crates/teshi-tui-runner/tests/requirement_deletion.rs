//! Global requirement deletion keeps the real PTY viewport and store aligned.

use std::fs;
use std::path::{Path, PathBuf};

use teshi_tui_runner::{Launch, Terminal};

fn artifact_path(name: &str) -> Option<PathBuf> {
    let directory = std::env::var_os("TESHI_REQUIREMENT_DELETION_ARTIFACTS").map(PathBuf::from)?;
    fs::create_dir_all(&directory).unwrap();
    Some(directory.join(name))
}

/// Requirement bodies carry timestamp names, so resolve the path through the
/// index instead of assuming a `doc-N.md` layout.
fn document_path(store: &Path, id: &str) -> Option<PathBuf> {
    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(store.join("_teshi.json")).ok()?).ok()?;
    index["documents"]
        .as_array()?
        .iter()
        .find(|document| document["id"] == id)
        .and_then(|document| document["path"].as_str())
        .map(|path| store.join(path))
}

fn launch_config(binary: &Path, project: &Path, store: &Path, root: &Path) -> Launch {
    Launch {
        program: binary.to_string_lossy().into_owned(),
        args: vec![
            "--recursive".into(),
            project.to_string_lossy().into_owned(),
            "--requirements-root".into(),
            store.to_string_lossy().into_owned(),
        ],
        cwd: Some(root.to_string_lossy().into_owned()),
        env: [
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "TESHI_APP_DATA_DIR",
        ]
        .into_iter()
        .map(|name| (name.into(), root.join(name).to_string_lossy().into_owned()))
        .collect(),
        cols: 150,
        rows: 30,
    }
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn global_requirement_delete_confirms_cancels_and_updates_viewport() -> anyhow::Result<()> {
    let binary = fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN")?).unwrap();
    let root = tempfile::tempdir()?;
    let project = root.path().join("project");
    let store = root.path().join("requirements");
    fs::create_dir(&project)?;
    fs::write(project.join("sample.feature"), "Feature: Sample\n")?;

    let terminal = Terminal::launch(launch_config(&binary, &project, &store, root.path()))?;
    terminal.expect_text("sample", true, 10000)?;
    terminal.key("4")?;
    terminal.expect_text("Requirements", true, 10000)?;

    for number in 1..=3 {
        terminal.key("Ctrl+n")?;
        terminal.expect_text(&format!("# Document {number}"), true, 10000)?;
        if number == 1 {
            terminal.key("Right")?;
        }
        terminal.key("Ctrl+s")?;
        terminal.expect_text("Saved requirement document", true, 10000)?;
    }

    // The third document is selected after creation. Ctrl-W h moves back to
    // the tree, then Up selects the middle document for the delete flow.
    terminal.key("Ctrl+w")?;
    terminal.key("h")?;
    terminal.key("Up")?;
    let before = terminal.screen()?;
    assert!(
        before.contains("Document 2"),
        "middle document was not selected:\n{before}"
    );
    if let Some(path) = artifact_path("before-delete.txt") {
        fs::write(&path, &before)?;
        terminal.screenshot(&path.with_file_name("before-delete.svg"))?;
    }

    terminal.click_text("Delete [d/Delete]", 5000)?;
    terminal.expect_text("global requirement library", true, 5000)?;
    if let Some(path) = artifact_path("delete-confirmation.txt") {
        fs::write(&path, terminal.screen()?)?;
        terminal.screenshot(&path.with_file_name("delete-confirmation.svg"))?;
    }

    terminal.key("Escape")?;
    terminal.expect_text("global requirement library", false, 5000)?;
    let doc2 = document_path(&store, "doc-2").expect("doc-2 is indexed");
    assert!(doc2.is_file(), "cancel removed {}", doc2.display());

    terminal.click_text("Delete [d/Delete]", 5000)?;
    terminal.expect_text("global requirement library", true, 5000)?;
    terminal.key("Enter")?;
    terminal.expect_text("# Document 3", true, 5000)?;
    terminal.expect_text("Document 2", false, 5000)?;
    let after = terminal.screen()?;
    assert!(
        !after.contains("Document 2"),
        "deleted document stayed visible:\n{after}"
    );
    if let Some(path) = artifact_path("after-delete.txt") {
        fs::write(&path, &after)?;
        terminal.screenshot(&path.with_file_name("after-delete.svg"))?;
    }

    assert!(!doc2.exists());
    let trash = fs::read_to_string(store.join("_teshi-trash.json"))?;
    assert!(trash.contains("doc-2"));
    let index = fs::read_to_string(store.join("_teshi.json"))?;
    assert!(!index.contains("doc-2"));

    terminal.key("q")?;
    terminal.expect_exit(0, 10000)?;
    Ok(())
}
