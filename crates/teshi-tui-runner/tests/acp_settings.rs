//! ACP settings through a real terminal, including SGR mouse input.
use teshi_tui_runner::{Launch, Terminal};

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn acp_settings_support_mouse_keyboard_and_overflow() {
    let binary = std::fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("sample.feature"),
        "Feature: Settings fixture\n",
    )
    .unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../teshi-agent-runtime/tests/fixtures/fake_acp_agent.py");
    let mut env: Vec<_> = [
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
    ]
    .into_iter()
    .map(|key| {
        (
            key.into(),
            root.path().join(key).to_string_lossy().into_owned(),
        )
    })
    .collect();
    env.extend([
        ("TESHI_AGENT_BACKEND".into(), "acp".into()),
        (
            "TESHI_ACP_PROGRAM".into(),
            if cfg!(windows) { "python" } else { "python3" }.into(),
        ),
        (
            "TESHI_ACP_ARGS_JSON".into(),
            serde_json::to_string(&[script.to_string_lossy().as_ref(), "settings"]).unwrap(),
        ),
    ]);
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
        env,
        cols: 140,
        rows: 32,
    })
    .unwrap();
    terminal.expect_text("sample", true, 15000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat!", true, 10000)
        .unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("/settings").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("Model: Alpha", true, 10000).unwrap();
    terminal.key("Escape").unwrap();
    terminal.expect_text("[More", true, 10000).unwrap();
    let screen = terminal.screen().unwrap();
    let (row, col) = screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find("Model: Alpha")
                .map(|index| (row + 1, line[..index].chars().count() + 1))
        })
        .unwrap();
    terminal
        .write(&format!("\x1b[<0;{col};{row}M\x1b[<0;{col};{row}m"))
        .unwrap();
    terminal.expect_text("Beta", true, 10000).unwrap();
    terminal.key("Down").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("Model: Beta", true, 10000).unwrap();
    terminal.expect_text("Thinking: Low", true, 10000).unwrap();
    terminal.resize(64, 24).unwrap();
    terminal.expect_text("[More", true, 10000).unwrap();
    terminal.write("/settings").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("Fast: Off", true, 10000).unwrap();
    terminal.key("Down").unwrap();
    terminal.key("Down").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("On", true, 10000).unwrap();
    terminal.key("Down").unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("/settings").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("Fast: On", true, 10000).unwrap();
    println!("{}", terminal.screen().unwrap());
}
