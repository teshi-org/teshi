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
    terminal
        .expect_text("No requirement artifacts", true, 10000)
        .unwrap();

    for number in 1..=29 {
        terminal.key("Ctrl+n").unwrap();
        terminal
            .expect_text(&format!("# Document {number}"), true, 10000)
            .unwrap();
        if number <= 2 {
            let (tree_is_synchronized, screen) =
                observe_tree_title(&terminal, &format!("Document {number}"), 3000);
            assert!(
                tree_is_synchronized,
                "requirement tree did not show Document {number} before save:\n{screen}"
            );
            assert!(
                screen.contains(&format!("# Document {number}")),
                "editor did not show Document {number} before save:\n{screen}"
            );
        }
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
    // The old success message may still be visible from document 29. Await the
    // persistence boundary rather than accepting that stale status as an ACK.
    let deadline = Instant::now() + Duration::from_secs(5);
    let index = loop {
        let index: serde_json::Value =
            serde_json::from_slice(&fs::read(store.join("_teshi.json")).unwrap()).unwrap();
        if index["documents"].as_array().unwrap().len() == 30 {
            break index;
        }
        assert!(
            Instant::now() < deadline,
            "document 30 was not persisted: {index}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    if let Some(path) = artifact_path("saved-index.json") {
        fs::write(path, serde_json::to_vec_pretty(&index).unwrap()).unwrap();
    }
    let documents = index["documents"].as_array().unwrap();
    assert_eq!(documents.len(), 30);
    for document in documents {
        let path = document["path"].as_str().unwrap();
        assert_eq!(path.len(), 26);
        assert_eq!(&path[8..9], "T");
        assert!(path.ends_with("Z.md"));
        assert_eq!(
            fs::read_to_string(store.join(path)).unwrap(),
            format!("# {}\n\n", document["title"].as_str().unwrap())
        );
    }
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn requirements_footer_save_and_keyboard_both_persist_documents() {
    let binary = fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let store = root.path().join("requirements");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("sample.feature"), "Feature: Sample\n").unwrap();
    let terminal = Terminal::launch(launch_config(&binary, &project, &store, root.path())).unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.key("4").unwrap();
    terminal.key("Ctrl+n").unwrap();
    terminal.expect_text("# Document 1", true, 10000).unwrap();
    terminal.expect_text("Save [Ctrl+s]", true, 10000).unwrap();
    terminal.click_text("Save", 10000).unwrap();
    let wait_count = |count: usize| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(bytes) = fs::read(store.join("_teshi.json")) {
                if let Ok(index) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    if index["documents"]
                        .as_array()
                        .is_some_and(|items| items.len() == count)
                    {
                        break index;
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "save did not persist {count} documents"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let first = wait_count(1);
    let doc = &first["documents"][0];
    assert_eq!(doc["title"], "Document 1");
    assert_eq!(
        fs::read_to_string(store.join(doc["path"].as_str().unwrap())).unwrap(),
        "# Document 1\n\n"
    );
    terminal.key("Ctrl+n").unwrap();
    terminal.expect_text("# Document 2", true, 10000).unwrap();
    terminal.key("Ctrl+s").unwrap();
    let second = wait_count(2);
    let doc = &second["documents"][1];
    assert_eq!(doc["title"], "Document 2");
    assert_eq!(
        fs::read_to_string(store.join(doc["path"].as_str().unwrap())).unwrap(),
        "# Document 2\n\n"
    );
    if let Some(path) = artifact_path("footer-save-index.json") {
        fs::write(path, serde_json::to_vec_pretty(&second).unwrap()).unwrap();
    }
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn clipped_footer_blank_space_does_not_trigger_hidden_action() {
    let binary = fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let store = root.path().join("requirements");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("sample.feature"), "Feature: Sample\n").unwrap();
    let mut config = launch_config(&binary, &project, &store, root.path());
    config.cols = 45;
    config.rows = 16;
    let terminal = Terminal::launch(config).unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.key("4").unwrap();
    terminal.key("Ctrl+n").unwrap();
    terminal.expect_text("Document 1", true, 10000).unwrap();
    let screen = terminal.screen().unwrap();
    let footer = screen.lines().nth(15).unwrap();
    let blank = footer
        .chars()
        .collect::<Vec<_>>()
        .iter()
        .rposition(|c| *c != ' ')
        .map(|i| i + 1)
        .unwrap_or(0);
    assert!(blank < 45, "no unused footer cell: {screen}");
    let before = fs::read(store.join("_teshi.json")).ok();
    terminal
        .write(&format!(
            "\x1b[<0;{};16M\x1b[<0;{};16m",
            blank + 1,
            blank + 1
        ))
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        fs::read(store.join("_teshi.json")).ok(),
        before,
        "blank click changed persistence"
    );
    terminal.expect_text("Document 1", true, 10000).unwrap();
    assert_eq!(
        terminal.screen().unwrap(),
        screen,
        "blank click changed the UI"
    );
    if let Some(path) = artifact_path("clipped-footer.txt") {
        fs::write(path, terminal.screen().unwrap()).unwrap();
    }
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn new_requirement_creation_dirty_guard_supports_cancel_and_discard() {
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
    terminal.key("Ctrl+n").unwrap();
    terminal.expect_text("# Document 1", true, 10000).unwrap();
    terminal.key("Right").unwrap();
    terminal.key("Ctrl+s").unwrap();
    terminal
        .expect_text("Saved requirement document", true, 10000)
        .unwrap();

    terminal.key("i").unwrap();
    terminal.write(" dirty").unwrap();
    terminal.key("Escape").unwrap();
    terminal.expect_text("[BROWSE]", true, 10000).unwrap();
    terminal.key("Ctrl+n").unwrap();
    terminal
        .expect_text("Unsaved requirement", true, 10000)
        .unwrap();
    terminal.key("Escape").unwrap();
    terminal.expect_text("# Document 1", true, 10000).unwrap();
    terminal
        .expect_text("Unsaved requirement", false, 10000)
        .unwrap();
    terminal.expect_text("# Document 2", false, 1000).unwrap();

    terminal.key("Ctrl+n").unwrap();
    terminal
        .expect_text("Unsaved requirement", true, 10000)
        .unwrap();
    terminal.key("D").unwrap();
    terminal.expect_text("# Document 2", true, 10000).unwrap();
    let screen = terminal.screen().unwrap();
    assert!(
        tree_contains_title(&screen, "Document 2"),
        "discarding the dirty document did not select the new document in the tree:\n{screen}"
    );

    terminal.key("Ctrl+s").unwrap();
    terminal
        .expect_text("Saved requirement document", true, 10000)
        .unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn requirements_tree_wheel_routes_under_pointer_and_keeps_editor_selection() {
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

    for number in 1..=18 {
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

    let before = terminal.screen().unwrap();
    assert!(tree_contains_title(&before, "Document 18"), "{before}");
    let tree_row = before
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.split('│')
                .nth(1)
                .filter(|pane| pane.contains("Document 18"))
                .map(|_| row as u16)
        })
        .expect("selected document should be visible in the tree");

    for _ in 0..8 {
        terminal.scroll_at(2, tree_row, true, false).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }

    let after = terminal.screen().unwrap();
    assert!(
        tree_contains_title(&after, "Document 1"),
        "tree wheel did not scroll the owning pane to the first document:\n{after}"
    );
    assert!(
        after.contains("# Document 18"),
        "scrolling the tree changed the editor selection:\n{after}"
    );
    if let Some(path) = artifact_path("wheel-tree-after.txt") {
        fs::write(&path, &after).unwrap();
        terminal
            .screenshot(&path.with_file_name("wheel-tree-after.svg"))
            .unwrap();
        println!("Requirement wheel viewport evidence: {}", path.display());
    }

    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}
