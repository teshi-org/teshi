//! Real terminal acceptance of process folding, live reasoning, rates and queued drafts.

use std::{fs, path::Path, time::Duration};
use teshi_tui_runner::{Launch, Terminal};

fn launch(root: &Path) -> Terminal {
    launch_backend(root, None)
}

fn launch_backend(root: &Path, native_url: Option<&str>) -> Terminal {
    let project = root.join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("sample.feature"), "Feature: Sample\n").unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/chat_activity_acp.py")
        .canonicalize()
        .unwrap();
    let mut env: Vec<_> = [
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "TESHI_APP_DATA_DIR",
        "TESHI_SESSION_DIR",
        "TESHI_ACP_STORE_DIR",
    ]
    .into_iter()
    .map(|key| (key.into(), root.join(key).to_string_lossy().into_owned()))
    .collect();
    env.extend([
        (
            "TESHI_AGENT_BACKEND".into(),
            if native_url.is_some() {
                "native"
            } else {
                "acp"
            }
            .into(),
        ),
        (
            "TESHI_ACP_PROGRAM".into(),
            if cfg!(windows) { "python" } else { "python3" }.into(),
        ),
        (
            "TESHI_ACP_ARGS_JSON".into(),
            serde_json::to_string(&[fixture.to_string_lossy().as_ref()]).unwrap(),
        ),
        (
            "TESHI_CHAT_ACTIVITY_GATE".into(),
            root.join("gate").to_string_lossy().into_owned(),
        ),
        (
            "TESHI_CHAT_ACTIVITY_TRACE".into(),
            root.join("trace.jsonl").to_string_lossy().into_owned(),
        ),
        (
            "TESHI_TEST_FRAME_PATH".into(),
            root.join("frame.json").to_string_lossy().into_owned(),
        ),
    ]);
    if let Some(url) = native_url {
        let profiles = root.join("TESHI_APP_DATA_DIR/model-profiles");
        fs::create_dir_all(&profiles).unwrap();
        fs::write(profiles.join("Fixture.json"), serde_json::to_vec(&serde_json::json!({
            "id":"Fixture", "name":"Fixture", "provider":"openai", "model_id":"fixture", "api_key":"synthetic",
            "base_url":url, "max_context_tokens":1000
        })).unwrap()).unwrap();
        fs::write(profiles.join("active"), "Fixture").unwrap();
    }
    let terminal = Terminal::launch(Launch {
        program: fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap())
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        args: vec![
            project.to_string_lossy().into_owned(),
            "--requirements-root".into(),
            root.join("requirements").to_string_lossy().into_owned(),
        ],
        cwd: Some(root.to_string_lossy().into_owned()),
        env,
        cols: 110,
        rows: 32,
    })
    .unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.key("3").unwrap();
    terminal.key("Enter").unwrap();
    terminal
}

fn gate(root: &Path, name: &str) {
    fs::write(root.join(format!("gate.{name}")), "continue").unwrap();
}

fn capture(terminal: &Terminal, name: &str) {
    if let Some(dir) = std::env::var_os("TESHI_CHAT_ACTIVITY_ARTIFACTS") {
        let dir = std::path::PathBuf::from(dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{name}.txt")), terminal.screen().unwrap()).unwrap();
        terminal
            .screenshot(&dir.join(format!("{name}.svg")))
            .unwrap();
    }
}

#[test]
#[ignore = "requires a built Teshi CLI and Python; set TESHI_SELF_TEST_BIN"]
fn live_thinking_speed_inline_wait_and_independent_process_folding() {
    let root = tempfile::tempdir().unwrap();
    let terminal = launch(root.path());
    terminal.write("Inspect this fixture").unwrap();
    terminal.click_text("Send ↵", 10000).unwrap();
    terminal
        .expect_text("Waiting for agent", true, 10000)
        .unwrap();
    capture(&terminal, "waiting");
    gate(root.path(), "thinking");
    terminal.expect_text("Thinking:", true, 10000).unwrap();
    terminal.expect_text("tok/s", true, 10000).unwrap();
    let screen = terminal.screen().unwrap();
    assert_eq!(
        screen
            .lines()
            .filter(|line| line.contains("Thinking:"))
            .count(),
        1,
        "{screen}"
    );
    capture(&terminal, "thinking");
    terminal.click_text("Thinking:", 10000).unwrap();
    terminal.expect_text("▾ Thinking", true, 10000).unwrap();
    terminal.write("FOCUS_SURVIVED").unwrap();
    terminal.expect_text("FOCUS_SURVIVED", true, 10000).unwrap();
    gate(root.path(), "tools");
    terminal.expect_text("1 failed", true, 10000).unwrap();
    terminal
        .expect_text("Waiting for agent", true, 10000)
        .unwrap();
    terminal.expect_text("tok/s", false, 10000).unwrap();
    capture(&terminal, "tools");
    gate(root.path(), "finish");
    terminal
        .expect_text("ACTIVITY_DONE_1", true, 10000)
        .unwrap();
    terminal.expect_text("2 tool calls:", true, 10000).unwrap();
    terminal.expect_text("THINKING_LINE", false, 10000).unwrap();
    terminal
        .expect_text("src/chat_fixture.rs", false, 10000)
        .unwrap();
    terminal.expect_text("FOCUS_SURVIVED", true, 10000).unwrap();
    capture(&terminal, "folded");
    terminal.click_text("2 tool calls:", 10000).unwrap();
    terminal
        .expect_text("▾ 2 tool calls:", true, 10000)
        .unwrap();
    terminal.click_text("▸ 2 tool calls:", 10000).unwrap();
    terminal
        .expect_text("Read src/chat_fixture.rs", true, 10000)
        .unwrap();
    terminal
        .expect_text("ACTIVITY_DONE_1", true, 10000)
        .unwrap();
    terminal.resize(48, 32).unwrap();
    terminal.expect_text("1 failed", true, 10000).unwrap();
    capture(&terminal, "expanded-narrow");
}

#[test]
#[ignore = "requires a built Teshi CLI and Python; set TESHI_SELF_TEST_BIN"]
fn messages_queued_while_running_send_in_submission_order() {
    let root = tempfile::tempdir().unwrap();
    let terminal = launch(root.path());
    terminal.write("Inspect this fixture").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("Waiting for agent", true, 10000)
        .unwrap();
    for text in ["QUEUED_SECOND", "QUEUED_THIRD"] {
        terminal.write(text).unwrap();
        terminal.key("Enter").unwrap();
    }
    terminal.expect_text("Queued 2:", true, 10000).unwrap();
    capture(&terminal, "queued");
    for stage in ["thinking", "tools", "finish"] {
        gate(root.path(), stage);
    }
    terminal
        .expect_text("ACTIVITY_DONE_3", true, 15000)
        .unwrap();
    terminal.expect_text("Queued 2:", false, 10000).unwrap();
    let trace = fs::read_to_string(root.path().join("trace.jsonl")).unwrap();
    let calls: Vec<serde_json::Value> = trace
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(calls.len(), 3, "{trace}");
    assert!(
        calls[1]["text"]
            .as_str()
            .unwrap()
            .contains("user: QUEUED_SECOND\n"),
        "{trace}"
    );
    assert!(
        calls[2]["text"]
            .as_str()
            .unwrap()
            .contains("user: QUEUED_THIRD\n"),
        "{trace}"
    );
    capture(&terminal, "queue-completed");
}

#[test]
#[ignore = "requires a built Teshi CLI and Python; set TESHI_SELF_TEST_BIN"]
fn a_failed_turn_preserves_and_pauses_queued_messages() {
    let root = tempfile::tempdir().unwrap();
    let terminal = launch(root.path());
    terminal.write("FAIL_TURN").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("Waiting for agent", true, 10000)
        .unwrap();
    terminal.write("KEEP_THIS_FOLLOWUP").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("Queued 1:", true, 10000).unwrap();
    gate(root.path(), "failure");
    terminal
        .expect_text("ACTIVITY_FIXTURE_FAILURE", true, 10000)
        .unwrap();
    terminal
        .expect_text("send a message to resume", true, 10000)
        .unwrap();
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(
        fs::read_to_string(root.path().join("trace.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    capture(&terminal, "queue-paused-on-failure");
    terminal.write("RESUME_WITH_THIS").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("ACTIVITY_DONE_3", true, 15000)
        .unwrap();
    terminal.expect_text("Queued ", false, 10000).unwrap();
    let trace = fs::read_to_string(root.path().join("trace.jsonl")).unwrap();
    let prompts: Vec<String> = trace
        .lines()
        .map(|line| {
            let call: serde_json::Value = serde_json::from_str(line).unwrap();
            call["text"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(prompts.len(), 3, "{trace}");
    assert!(
        prompts[1].contains("KEEP_THIS_FOLLOWUP") && !prompts[1].contains("RESUME_WITH_THIS"),
        "the paused follow-up must be sent before the resuming message: {trace}"
    );
    assert!(prompts[2].contains("RESUME_WITH_THIS"), "{trace}");
}

#[test]
#[ignore = "requires a built Teshi CLI and Python; set TESHI_SELF_TEST_BIN"]
fn cancelling_a_turn_keeps_its_followup_available_for_editing() {
    let root = tempfile::tempdir().unwrap();
    let terminal = launch(root.path());
    terminal.write("Inspect this fixture").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("Waiting for agent", true, 10000)
        .unwrap();
    terminal.write("RECLAIM_AFTER_CANCEL").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("Queued 1:", true, 10000).unwrap();
    // Esc withdraws the queued follow-up into the composer and keeps the run
    // going. The first press only blurs a focused composer, so press until the
    // withdrawal reports itself.
    for _ in 0..2 {
        terminal.key("Escape").unwrap();
        if terminal
            .expect_text("Queued message restored for editing", true, 2000)
            .is_ok()
        {
            break;
        }
    }
    terminal
        .expect_text("RECLAIM_AFTER_CANCEL", true, 10000)
        .unwrap();
    terminal
        .expect_text("Enter queues it", false, 10000)
        .unwrap();
    // The withdrawal focused the composer, so blur it and then ask to stop.
    terminal.key("Escape").unwrap();
    terminal.key("Escape").unwrap();
    terminal
        .expect_text("Interrupt the current run?", true, 10000)
        .unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("cancelled", true, 10000).unwrap();
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(
        fs::read_to_string(root.path().join("trace.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    // The withdrawn follow-up is still the draft, so it can be edited and sent.
    terminal
        .expect_text("RECLAIM_AFTER_CANCEL", true, 10000)
        .unwrap();
    terminal.expect_text("Queued 1:", false, 10000).unwrap();
    capture(&terminal, "queue-reclaimed-after-cancel");
}

#[test]
#[ignore = "requires a built Teshi CLI and Python; set TESHI_SELF_TEST_BIN"]
fn refined_dock_keeps_model_controls_metrics_and_narrow_composers_accessible() {
    let root = tempfile::tempdir().unwrap();
    let terminal = launch(root.path());
    terminal.expect_text("Model Quality", true, 10000).unwrap();
    terminal.click_text("Model Quality", 10000).unwrap();
    terminal.expect_text("Fast", true, 10000).unwrap();
    terminal.click_text("Fast", 10000).unwrap();
    terminal.expect_text("Model Fast", true, 10000).unwrap();
    let screen = terminal.screen().unwrap();
    let lines: Vec<_> = screen.lines().collect();
    let metadata = lines
        .iter()
        .position(|line| line.contains("Model Fast") && line.contains("ctx unknown"))
        .unwrap();
    let send = lines
        .iter()
        .position(|line| line.contains("Send ↵"))
        .unwrap();
    assert_eq!(
        metadata + 2,
        send,
        "metrics sit above the border, not on it: {screen}"
    );
    assert!(!lines[send - 1].contains("ctx"), "{screen}");
    capture(&terminal, "refined-config-and-input");
    for width in [150, 88, 48, 32, 110] {
        terminal.resize(width, 32).unwrap();
        terminal.expect_text("Chats", width >= 90, 10000).unwrap();
        terminal.expect_text("ctx", true, 10000).unwrap();
        terminal.expect_text("Send ↵", true, 10000).unwrap();
        capture(&terminal, &format!("refined-width-{width}"));
    }
    terminal.expect_text("Model Fast", true, 10000).unwrap();
    assert!(
        !root.path().join("trace.jsonl").exists(),
        "presentation and model selection must not submit a prompt"
    );
}

#[test]
#[ignore = "requires a built Teshi CLI and Python; set TESHI_SELF_TEST_BIN"]
fn native_streaming_keeps_the_composer_active_and_sends_the_next_turn_once() {
    struct Provider(std::process::Child);
    impl Drop for Provider {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = tempfile::tempdir().unwrap();
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native_chat_activity.py");
    let _provider = Provider(
        std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
            .arg(script)
            .arg(root.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let port = loop {
        if let Ok(port) = fs::read_to_string(root.path().join("native-port"))
            && !port.trim().is_empty()
        {
            break port;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "provider did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let terminal = launch_backend(root.path(), Some(&format!("http://127.0.0.1:{port}/v1")));
    terminal.write("Inspect native chat").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("NATIVE_REASONING", true, 10000)
        .unwrap();
    terminal.expect_text("tok/s", true, 10000).unwrap();
    terminal.write("NATIVE_QUEUED_PROMPT").unwrap();
    terminal.click_text("Queue ↵", 10000).unwrap();
    terminal.expect_text("Queued 1:", true, 10000).unwrap();
    terminal.expect_text("NATIVE_DONE_2", false, 1000).unwrap();
    capture(&terminal, "native-queued");
    fs::write(root.path().join("native.finish"), "finish").unwrap();
    terminal.expect_text("NATIVE_DONE_2", true, 10000).unwrap();
    terminal.expect_text("Queued 1:", false, 10000).unwrap();
    terminal.expect_text("tok/s", false, 10000).unwrap();
    let trace = fs::read_to_string(root.path().join("native-trace.jsonl")).unwrap();
    let requests: Vec<serde_json::Value> = trace
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(requests.len(), 2, "{trace}");
    assert_eq!(
        requests[1]["messages"].as_array().unwrap().last().unwrap()["content"],
        "NATIVE_QUEUED_PROMPT"
    );
    capture(&terminal, "native-completed");
}
