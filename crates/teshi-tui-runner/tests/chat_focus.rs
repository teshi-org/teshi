//! Real PTY chat focus and newline/send coverage with an isolated ACP fixture.
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
use teshi_tui_runner::{Launch, Terminal};

fn setup(acp: bool) -> (tempfile::TempDir, Terminal) {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("sample.feature"), "Feature: Sample\n").unwrap();
    let trace = root.path().join("trace");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agent_acceptance_fixture.py");
    let mut env: Vec<(String, String)> = [
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "TESHI_APP_DATA_DIR",
        "TESHI_SESSION_DIR",
        "TESHI_ACP_STORE_DIR",
    ]
    .into_iter()
    .map(|key| {
        (
            key.into(),
            root.path().join(key).to_string_lossy().into_owned(),
        )
    })
    .collect();
    env.push((
        "TESHI_AGENT_BACKEND".into(),
        if acp { "acp" } else { "native" }.into(),
    ));
    if acp {
        env.push((
            "TESHI_ACP_PROGRAM".into(),
            if cfg!(windows) { "python" } else { "python3" }.into(),
        ));
        env.push((
            "TESHI_ACP_ARGS_JSON".into(),
            serde_json::to_string(&[
                fixture.to_string_lossy().as_ref(),
                "acp",
                trace.to_string_lossy().as_ref(),
                "Focus Agent",
            ])
            .unwrap(),
        ));
    }
    let terminal = Terminal::launch(Launch {
        program: fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap())
            .unwrap()
            .to_string_lossy()
            .into_owned(),
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
        env,
        cols: 150,
        rows: 30,
    })
    .unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.key("3").unwrap();
    terminal.expect_text("ctx unknown", true, 10000).unwrap();
    (root, terminal)
}

fn capture(terminal: &Terminal, name: &str) {
    if let Some(path) = std::env::var_os("TESHI_CHAT_FOCUS_ARTIFACTS") {
        let dir = std::path::PathBuf::from(path);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{name}.txt")), terminal.screen().unwrap()).unwrap();
        terminal
            .screenshot(&dir.join(format!("{name}.svg")))
            .unwrap();
    }
}

fn input_title(terminal: &Terminal) -> String {
    terminal
        .screen()
        .unwrap()
        .lines()
        .find(|line| line.contains("ctx unknown"))
        .unwrap()
        .to_owned()
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn repeated_focus_keeps_input_title_and_context_statistics() {
    let (root, terminal) = setup(false);
    let before = input_title(&terminal);
    let before_svg = root.path().join("blur.svg");
    terminal.screenshot(&before_svg).unwrap();
    capture(&terminal, "blurred");
    for _ in 0..2 {
        terminal.key("Enter").unwrap();
        terminal.expect_text("Send [Enter]", true, 5000).unwrap();
        assert_eq!(input_title(&terminal), before);
        let focused_svg = root.path().join("focused.svg");
        terminal.screenshot(&focused_svg).unwrap();
        assert_ne!(
            fs::read(&before_svg).unwrap(),
            fs::read(&focused_svg).unwrap(),
            "focus did not change rendered styling"
        );
        capture(&terminal, "focused");
        terminal.key("Escape").unwrap();
        terminal.expect_text("Focus [i/Enter]", true, 5000).unwrap();
        assert_eq!(input_title(&terminal), before);
    }
}

#[test]
#[ignore = "requires a built Teshi CLI and Python; set TESHI_SELF_TEST_BIN"]
fn message_is_sent_once_with_exact_body() {
    let (root, terminal) = setup(true);
    terminal.key("Enter").unwrap();
    // The legacy PTY encoding this harness speaks cannot express Shift+Enter or
    // Ctrl+J as anything but Enter, so a literal newline can never reach the
    // buffer here; AiNewline is covered by a unit test instead. This keeps the
    // end-to-end contract that one Enter sends one prompt with its exact body.
    terminal.write("first line").unwrap();
    terminal.key("Enter").unwrap();
    let trace = root.path().join("trace/trace.jsonl");
    let deadline = Instant::now() + Duration::from_secs(15);
    let prompts = loop {
        let prompts: Vec<serde_json::Value> = fs::read_to_string(&trace)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|event| event["kind"] == "prompt")
            .collect();
        if !prompts.is_empty() {
            break prompts;
        }
        assert!(
            Instant::now() < deadline,
            "ACP fixture did not receive a prompt"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(prompts.len(), 1);
    let params = &prompts[0]["params"];
    if let Some(path) = std::env::var_os("TESHI_CHAT_FOCUS_ARTIFACTS") {
        let dir = std::path::PathBuf::from(path);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("prompt.json"),
            serde_json::to_vec_pretty(params).unwrap(),
        )
        .unwrap();
        capture(&terminal, "newline-before-assertion");
    }
    assert!(params.to_string().contains("first line\\n\\n"), "{params}");
    terminal
        .expect_text("Focus Agent response", true, 15000)
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let trace_text = fs::read_to_string(trace).unwrap();
    assert_eq!(
        trace_text
            .lines()
            .filter(|line| line.contains("\"kind\": \"prompt\""))
            .count(),
        1
    );
    capture(&terminal, "newline-sent");
}

#[test]
#[ignore = "requires a built Teshi CLI and Python; set TESHI_SELF_TEST_BIN"]
fn acp_without_reported_usage_shows_unknown_after_response() {
    let (_root, terminal) = setup(true);
    terminal.key("Enter").unwrap();
    terminal.write("Inspect context usage").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("Focus Agent response", true, 15000)
        .unwrap();
    let line = input_title(&terminal);
    assert!(line.contains("ctx unknown"), "{line}");
    capture(&terminal, "unknown-usage-after-response");
}
