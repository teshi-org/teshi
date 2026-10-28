//! Requirement renaming through keyboard input in the actual Teshi TUI.

#[path = "support/documents.rs"]
mod documents;

use std::fs;

use documents::document_path;
use teshi_tui_runner::{Launch, Terminal};

fn assert_tree_title(terminal: &Terminal, title: &str) {
    let screen = terminal.screen().unwrap();
    assert!(
        screen.lines().any(|line| line
            .split('│')
            .nth(1)
            .is_some_and(|pane| pane.contains(title))),
        "requirement tree did not show {title:?}:\n{screen}"
    );
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn requirement_title_tracks_keyboard_save_and_survives_restart() {
    let binary = fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let store = root.path().join("requirements");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("sample.feature"), "Feature: Sample\n").unwrap();
    let config = Launch {
        program: binary.to_string_lossy().into_owned(),
        args: vec![
            "--recursive".into(),
            project.to_string_lossy().into_owned(),
            "--requirements-root".into(),
            store.to_string_lossy().into_owned(),
        ],
        cwd: Some(root.path().to_string_lossy().into_owned()),
        env: [
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "TESHI_APP_DATA_DIR",
        ]
        .into_iter()
        .map(|name| {
            (
                name.into(),
                root.path().join(name).to_string_lossy().into_owned(),
            )
        })
        .collect(),
        cols: 150,
        rows: 30,
    };
    let terminal = Terminal::launch(config.clone()).unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.key("4").unwrap();
    terminal.expect_text("Requirements", true, 10000).unwrap();
    terminal.key("Ctrl+n").unwrap();
    terminal.expect_text("# Document 1", true, 10000).unwrap();
    terminal.key("Right").unwrap();
    terminal.key("i").unwrap();
    terminal.expect_text("[INSERT]", true, 10000).unwrap();
    terminal.key("End").unwrap();
    for _ in "Document 1".chars() {
        terminal.key("Backspace").unwrap();
    }
    let title = "用户登录需求";
    terminal.write(title).unwrap();
    terminal.key("Ctrl+s").unwrap();
    terminal
        .expect_text("Saved requirement document", true, 10000)
        .unwrap();
    assert_tree_title(&terminal, title);
    let document = document_path(&store, "doc-1").expect("doc-1 is indexed");
    let markdown = fs::read(&document).unwrap();
    let index = fs::read(store.join("_teshi.json")).unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&index).unwrap();
    assert_eq!(metadata["documents"][0]["id"], "doc-1");
    let recorded = metadata["documents"][0]["path"].as_str().unwrap();
    assert!(
        recorded.ends_with(".md") && recorded != "doc-1.md",
        "new bodies carry timestamp names: {recorded}"
    );
    assert_eq!(metadata["documents"][0]["title"], title);
    assert_eq!(
        String::from_utf8(markdown.clone()).unwrap(),
        format!("# {title}\n\n")
    );

    for _ in title.chars() {
        terminal.key("Backspace").unwrap();
    }
    terminal.key("Ctrl+s").unwrap();
    terminal
        .expect_text("Requirement title cannot be empty", true, 10000)
        .unwrap();
    assert_eq!(fs::read(&document).unwrap(), markdown);
    assert_eq!(fs::read(store.join("_teshi.json")).unwrap(), index);
    terminal.write(title).unwrap();
    terminal.key("Home").unwrap();
    terminal.key("Delete").unwrap();
    terminal.key("Delete").unwrap();
    terminal.key("Ctrl+s").unwrap();
    terminal
        .expect_text("Requirement must start with an H1 title", true, 10000)
        .unwrap();
    assert_eq!(fs::read(&document).unwrap(), markdown);
    assert_eq!(fs::read(store.join("_teshi.json")).unwrap(), index);
    terminal.write("# ").unwrap();
    terminal.key("Ctrl+s").unwrap();
    terminal
        .expect_text("Saved requirement document", true, 10000)
        .unwrap();
    terminal.key("Escape").unwrap();
    terminal.expect_text("[BROWSE]", true, 10000).unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
    drop(terminal);

    let reopened = Terminal::launch(config).unwrap();
    reopened.expect_text("sample", true, 10000).unwrap();
    reopened.key("4").unwrap();
    reopened
        .expect_text(&format!("# {title}"), true, 10000)
        .unwrap();
    assert_tree_title(&reopened, title);
    reopened.key("q").unwrap();
    reopened.expect_exit(0, 10000).unwrap();
}
