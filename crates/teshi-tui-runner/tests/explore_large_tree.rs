//! Large-catalog interaction regression in a real PTY.
use std::{fs, time::Instant};
use teshi_tui_runner::{Launch, Terminal};

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN"]
fn search_scroll_and_click_large_catalog() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    fs::create_dir_all(&project).unwrap();
    for i in 0..10_000 {
        fs::write(
            project.join(format!("file{i:05}.feature")),
            format!("Feature: Item {i:05}\n  Scenario: Selected {i:05}\n    Given ready\n"),
        )
        .unwrap();
    }
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );
    let config = Launch {
        program: fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap())
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        args: vec![
            project.to_string_lossy().into_owned(),
            "--requirements-root".into(),
            root.path()
                .join("requirements")
                .to_string_lossy()
                .into_owned(),
        ],
        cwd: Some(project.to_string_lossy().into_owned()),
        env: [
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "TESHI_APP_DATA_DIR",
            "TESHI_SESSION_DIR",
            "TESHI_ACP_STORE_DIR",
        ]
        .into_iter()
        .map(|name| {
            (
                name.into(),
                root.path().join(name).to_string_lossy().into_owned(),
            )
        })
        .collect(),
        cols: 140,
        rows: 30,
    };
    let terminal = Terminal::launch(config).unwrap();
    terminal.expect_text("Selected 00000", true, 20000).unwrap();
    let start = Instant::now();
    terminal.write("/file09999").unwrap();
    terminal.expect_text("Selected 09999", true, 5000).unwrap();
    eprintln!("10k search including debounce: {:?}", start.elapsed());
    terminal.key("Enter").unwrap();
    terminal.key("Escape").unwrap();
    terminal.expect_text("file09998", true, 5000).unwrap();
    let start = Instant::now();
    terminal.click_text("file09998", 5000).unwrap();
    terminal.expect_text("Selected 09998", true, 5000).unwrap();
    eprintln!("10k bottom-row click: {:?}", start.elapsed());
    terminal.write("gg").unwrap();
    terminal.expect_text("Selected 00000", true, 5000).unwrap();
    let start = Instant::now();
    for _ in 0..10 {
        terminal.key("PageDown").unwrap();
    }
    terminal.expect_text("Selected 00100", true, 5000).unwrap();
    eprintln!("10k page scrolling: {:?}", start.elapsed());
    terminal.scroll_at(8, 8, false, false).unwrap();
    terminal.write("/file05000").unwrap();
    terminal.expect_text("Selected 05000", true, 5000).unwrap();
    terminal.close().unwrap();
}
