//! Real PTY coverage for the compact ACP transcript and details command.

use teshi_tui_runner::{Launch, Terminal};

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn fake_acp_activity_compact_expanded_and_narrow() {
    let binary = std::fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../teshi-agent-runtime/tests/fixtures/fake_acp_agent.py")
        .canonicalize()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let output = std::env::var_os("TESHI_ACTIVITY_ARTIFACTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.path().join("screenshots"));
    std::fs::create_dir_all(&output).unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let terminal = Terminal::launch(Launch {
        program: binary.to_string_lossy().into_owned(),
        args: vec![
            project.to_string_lossy().into_owned(),
            "--requirements-root".into(),
            root.path()
                .join("requirements")
                .to_string_lossy()
                .into_owned(),
        ],
        cwd: Some(root.path().to_string_lossy().into_owned()),
        env: vec![
            (
                "APPDATA".into(),
                root.path().join("appdata").to_string_lossy().into_owned(),
            ),
            (
                "LOCALAPPDATA".into(),
                root.path()
                    .join("localappdata")
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "XDG_CONFIG_HOME".into(),
                root.path().join("config").to_string_lossy().into_owned(),
            ),
            (
                "XDG_DATA_HOME".into(),
                root.path().join("data").to_string_lossy().into_owned(),
            ),
            ("TESHI_AGENT_BACKEND".into(), "acp".into()),
            ("TESHI_ACP_PROGRAM".into(), "python".into()),
            (
                "TESHI_ACP_ARGS_JSON".into(),
                serde_json::to_string(&vec![fixture.to_string_lossy().as_ref(), "activity"])
                    .unwrap(),
            ),
        ],
        cols: 100,
        rows: 32,
    })
    .unwrap();
    terminal.expect_text("Explore", true, 15000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat!", true, 15000)
        .unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("Inspect the project").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("hello", true, 15000).unwrap();
    terminal
        .expect_text("Done  Read project", true, 15000)
        .unwrap();
    terminal.expect_text("acp_reasoning", false, 1000).unwrap();
    terminal.expect_text("Unconfirmed", true, 15000).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    terminal.screenshot(&output.join("compact.svg")).unwrap();
    terminal.write("/details").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("to collapse", true, 15000).unwrap();
    terminal.screenshot(&output.join("expanded.svg")).unwrap();
    terminal.write("/details").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("to expand", true, 15000).unwrap();
    terminal.resize(48, 28).unwrap();
    terminal.expect_text("Done", true, 15000).unwrap();
    terminal.screenshot(&output.join("narrow.svg")).unwrap();
    std::fs::write(output.join("narrow.txt"), terminal.screen().unwrap()).unwrap();
    terminal.key("Escape").unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
    println!("Screenshots: {}", output.display());
}
