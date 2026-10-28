//! Real terminal regression for explicit ACP selection persistence.

use teshi_tui_runner::{Launch, Terminal};

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn explicit_acp_selection_survives_new_and_restart() {
    let binary = std::fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wrapped_chat_acp.py")
        .canonicalize()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let store = root.path().join("acp");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(
        store.join("installed.json"),
        serde_json::to_vec(&serde_json::json!([{
            "id": "fake-alpha", "name": "Fake Alpha", "program": "python",
            "args": [fixture], "env": {}, "env_from": {}
        }]))
        .unwrap(),
    )
    .unwrap();
    let launch = || {
        Terminal::launch(Launch {
            program: binary.to_string_lossy().into_owned(),
            args: vec![
                "--recursive".into(),
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
                    "TESHI_ACP_STORE_DIR".into(),
                    store.to_string_lossy().into_owned(),
                ),
                (
                    "APPDATA".into(),
                    root.path().join("appdata").to_string_lossy().into_owned(),
                ),
                (
                    "LOCALAPPDATA".into(),
                    root.path().join("localdata").to_string_lossy().into_owned(),
                ),
                (
                    "XDG_CONFIG_HOME".into(),
                    root.path().join("config").to_string_lossy().into_owned(),
                ),
                (
                    "XDG_DATA_HOME".into(),
                    root.path().join("data").to_string_lossy().into_owned(),
                ),
            ],
            cols: 120,
            rows: 35,
        })
        .unwrap()
    };
    let terminal = launch();
    terminal.expect_text("Explore", true, 15000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat!", true, 15000)
        .unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("/agent").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("Fake Alpha", true, 15000).unwrap();
    terminal.key("Down").unwrap();
    terminal.key("Enter").unwrap();
    let send = |terminal: &Terminal, text: &str| {
        terminal.write(text).unwrap();
        terminal.key("Enter").unwrap();
        terminal
            .expect_text("WRAPPED_REPLY_01", true, 20000)
            .unwrap();
    };
    send(&terminal, "original conversation");
    terminal.key("Escape").unwrap();
    terminal.expect_text("Send [Enter]", false, 5000).unwrap();
    terminal.key("a").unwrap();
    terminal
        .expect_text("WRAPPED_REPLY_01", false, 5000)
        .unwrap();
    send(&terminal, "new agent conversation");
    terminal.write("/new").unwrap();
    terminal
        .expect_text("Start a new session", true, 5000)
        .unwrap();
    terminal.key("Tab").unwrap();
    terminal
        .expect_text("WRAPPED_REPLY_01", false, 20000)
        .unwrap();
    send(&terminal, "fresh conversation");
    terminal.write("/exit").unwrap();
    terminal.expect_text("/exit", true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_exit(0, 15000).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(store.join("last-selection.json")).unwrap()).unwrap();
    assert_eq!(saved["agent_id"], "fake-alpha");
    let terminal = launch();
    terminal.expect_text("Explore", true, 15000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat!", true, 15000)
        .unwrap();
    terminal.key("Enter").unwrap();
    send(&terminal, "after restart");
    terminal.write("/exit").unwrap();
    terminal.expect_text("/exit", true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_exit(0, 15000).unwrap();
}
