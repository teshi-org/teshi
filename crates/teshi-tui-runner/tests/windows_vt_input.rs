//! Exercise the product's console reader through a real Windows ConPTY.
#![cfg(windows)]

use std::fs;
use std::path::PathBuf;

use teshi_tui_runner::{Launch, Terminal};

#[test]
#[ignore = "set TESHI_VT_INPUT_TEST_BIN to the built teshi-tui unit-test executable"]
fn console_input_releases_reader_restores_mode_and_restarts() {
    let binary = fs::canonicalize(std::env::var_os("TESHI_VT_INPUT_TEST_BIN").unwrap()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let terminal = Terminal::launch(Launch {
        program: binary.to_string_lossy().into_owned(),
        args: vec![
            "--exact".into(),
            "input::windows_vt::tests::console_lifecycle_fixture".into(),
            "--ignored".into(),
            "--nocapture".into(),
        ],
        cwd: Some(root.path().to_string_lossy().into_owned()),
        env: vec![
            ("ZELLIJ".into(), "teshi-owned-console-acceptance".into()),
            ("TESHI_VT_INPUT_FIXTURE".into(), "1".into()),
        ],
        cols: 100,
        rows: 25,
    })
    .unwrap();
    for cycle in 0..3 {
        terminal
            .expect_text(&format!("vt-ready-{cycle}"), true, 10000)
            .unwrap();
        terminal.write("甲🙂尾|").unwrap();
        terminal
            .expect_text(&format!("vt-released-{cycle}"), true, 5000)
            .unwrap();
    }
    terminal.expect_text("stdin-ready", true, 5000).unwrap();
    terminal.write("after-vt\r").unwrap();
    terminal.expect_text("stdin-restored", true, 5000).unwrap();
    terminal.expect_exit(0, 10000).unwrap();
    if let Some(directory) = std::env::var_os("TESHI_VT_INPUT_ARTIFACTS") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("console-lifecycle.txt"),
            terminal.screen().unwrap(),
        )
        .unwrap();
        terminal
            .screenshot(&directory.join("console-lifecycle.svg"))
            .unwrap();
    }
}
