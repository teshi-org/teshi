//! Real ACP task timing through controlled tool activity.
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use teshi_tui_runner::{Launch, Terminal};

fn capture(terminal: &Terminal, name: &str) {
    if let Some(path) = std::env::var_os("TESHI_CHAT_RUNTIME_ARTIFACTS") {
        let dir = PathBuf::from(path);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{name}.txt")), terminal.screen().unwrap()).unwrap();
        terminal
            .screenshot(&dir.join(format!("{name}.svg")))
            .unwrap();
    }
}

fn timer_line(terminal: &Terminal) -> String {
    terminal
        .screen()
        .unwrap()
        .lines()
        .find(|line| line.contains("ctx unknown"))
        .unwrap()
        .to_owned()
}

#[test]
#[ignore = "requires a built Teshi CLI and Python; set TESHI_SELF_TEST_BIN"]
fn elapsed_time_advances_through_acp_tool_activity_and_remains_after_completion() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("sample.feature"), "Feature: Sample\n").unwrap();
    let gate = root.path().join("gate");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../teshi-agent-runtime/tests/fixtures/fake_acp_agent.py")
        .canonicalize()
        .unwrap();
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
    env.extend([
        ("TESHI_AGENT_BACKEND".into(), "acp".into()),
        ("NO_COLOR".into(), String::new()),
        ("TERM".into(), "xterm-256color".into()),
        ("COLORTERM".into(), "truecolor".into()),
        (
            "TESHI_ACP_PROGRAM".into(),
            if cfg!(windows) { "python" } else { "python3" }.into(),
        ),
        (
            "TESHI_ACP_ARGS_JSON".into(),
            serde_json::to_string(&[fixture.to_string_lossy().as_ref(), "visual_success"]).unwrap(),
        ),
        (
            "TESHI_ACTIVITY_GATE".into(),
            gate.to_string_lossy().into_owned(),
        ),
    ]);
    let terminal = Terminal::launch(Launch {
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
        cwd: Some(root.path().to_string_lossy().into_owned()),
        env,
        cols: 150,
        rows: 30,
    })
    .unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.key("3").unwrap();
    terminal.expect_text("ctx unknown", true, 10000).unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("Inspect the project").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("I will inspect the project", true, 15000)
        .unwrap();
    let screen = terminal.screen().unwrap();
    let status = screen.lines().find(|line| line.contains("thinking") && line.contains("ACP") && line.contains("Settings"))
        .unwrap_or_else(|| panic!("activity and controls not on one row: {screen}"));
    assert!(status.contains('●'), "waiting animation not visible: {status}");
    let initial = timer_line(&terminal);
    capture(&terminal, "elapsed-initial");
    std::thread::sleep(Duration::from_millis(2200));
    let running = timer_line(&terminal);
    assert_ne!(initial, running, "elapsed display did not advance");
    fs::write(gate.with_extension("tools"), "continue").unwrap();
    terminal.expect_text("Read project", true, 15000).unwrap();
    terminal
        .expect_text("Inspection finished", true, 15000)
        .unwrap();
    capture(&terminal, "elapsed-after-tool");
    fs::write(gate.with_extension("finish"), "continue").unwrap();
    let completion_svg = root.path().join("completion.svg");
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        terminal.screenshot(&completion_svg).unwrap();
        let svg = fs::read_to_string(&completion_svg).unwrap();
        if svg
            .split("<text ")
            .filter_map(|part| part.split_once('>'))
            .any(|(attrs, text)| {
                attrs.contains("fill=\"#5fff87\"")
                    && text
                        .split_once("</text>")
                        .is_some_and(|(body, _)| body.trim().ends_with("Teshi"))
            })
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "ACP completion indicator did not appear"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
    let completed = timer_line(&terminal);
    capture(&terminal, "elapsed-completed");
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        timer_line(&terminal),
        completed,
        "elapsed display changed after completion"
    );
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn reported_context_usage_above_threshold_is_visible() {
    use std::io::{Read, Write};
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("sample.feature"), "Feature: Sample\n").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "local provider was not called"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => panic!("provider accept: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 8192];
        let header_end = loop {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0, "provider request ended before headers");
            request.extend_from_slice(&chunk[..count]);
            if let Some(pos) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        assert!(headers.starts_with("POST "));
        let length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(key, value)| {
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
            })
            .unwrap_or(0);
        while request.len() - header_end < length {
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&chunk[..count]);
        }
        let body = "data: {\"model\":\"fixture\",\"choices\":[{\"delta\":{\"content\":\"Usage fixture reply\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":701,\"completion_tokens\":3}}\n\ndata: [DONE]\n\n";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let profiles = root.path().join("TESHI_APP_DATA_DIR/model-profiles");
    fs::create_dir_all(&profiles).unwrap();
    fs::write(profiles.join("Fixture.json"), serde_json::to_vec(&serde_json::json!({"id":"Fixture", "name":"Fixture", "provider":"openai", "model_id":"fixture", "api_key":"synthetic", "base_url":format!("http://{address}/v1"), "max_context_tokens":1000})).unwrap()).unwrap();
    fs::write(profiles.join("active"), "Fixture").unwrap();
    let env = [
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
    .chain([("TESHI_AGENT_BACKEND".into(), "native".into())])
    .collect();
    let terminal = Terminal::launch(Launch {
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
        cwd: Some(root.path().to_string_lossy().into_owned()),
        env,
        cols: 150,
        rows: 30,
    })
    .unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.key("3").unwrap();
    terminal.expect_text("AI Chat", true, 10000).unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("Inspect usage").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("Usage fixture reply", true, 15000)
        .unwrap();
    terminal.expect_text("ctx 701/1000 !", true, 15000).unwrap();
    capture(&terminal, "reported-context-warning");
    server.join().unwrap();
}
