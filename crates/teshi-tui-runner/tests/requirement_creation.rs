//! New requirement creation keeps the tree and editor aligned in a real PTY.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use teshi_tui_runner::{Launch, Terminal};

fn tree_contains_title(screen: &str, title: &str) -> bool {
    screen.lines().any(|line| {
        line.split('│')
            .nth(1)
            .is_some_and(|pane| pane.contains(title))
    })
}

fn observe_tree_title(terminal: &Terminal, title: &str, timeout_ms: u64) -> (bool, String) {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let screen = terminal.screen().unwrap();
        if tree_contains_title(&screen, title) {
            return (true, screen);
        }
        if Instant::now() >= deadline {
            return (false, screen);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn artifact_path(name: &str) -> Option<PathBuf> {
    let directory = std::env::var_os("TESHI_REQUIREMENT_CREATION_ARTIFACTS").map(PathBuf::from)?;
    fs::create_dir_all(&directory).unwrap();
    Some(directory.join(name))
}

fn launch_config(binary: &Path, project: &Path, store: &Path, root: &Path) -> Launch {
    Launch {
        program: binary.to_string_lossy().into_owned(),
        args: vec![
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
fn new_requirement_creation_keeps_tree_and_editor_in_sync() {
    let binary = fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let store = root.path().join("requirements");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("sample.feature"), "Feature: Sample\n").unwrap();

    let terminal = Terminal::launch(launch_config(&binary, &project, &store, root.path())).unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.key("4").unwrap();
    terminal.expect_text("Requirements", true, 10000).unwrap();

    for number in 1..=29 {
        terminal.key("Ctrl+n").unwrap();
        terminal
            .expect_text(&format!("# Document {number}"), true, 10000)
            .unwrap();
        if number == 1 {
            terminal.key("Right").unwrap();
        }
        terminal.key("Ctrl+s").unwrap();
        terminal
            .expect_text("Saved requirement document", true, 10000)
            .unwrap();
    }

    terminal.key("Ctrl+n").unwrap();
    terminal.expect_text("# Document 30", true, 10000).unwrap();
    let (tree_is_synchronized, screen) = observe_tree_title(&terminal, "Document 30", 3000);
    if let Some(path) = artifact_path("after-create.txt") {
        fs::write(&path, &screen).unwrap();
        terminal
            .screenshot(&path.with_file_name("after-create.svg"))
            .unwrap();
        println!("Requirement creation viewport evidence: {}", path.display());
    }
    assert!(
        tree_is_synchronized,
        "requirement tree did not show the new document before save:\n{screen}"
    );
    assert!(
        screen.contains("# Document 30"),
        "editor did not show the new body:\n{screen}"
    );

    terminal.key("Ctrl+s").unwrap();
    terminal
        .expect_text("Saved requirement document", true, 10000)
        .unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}
