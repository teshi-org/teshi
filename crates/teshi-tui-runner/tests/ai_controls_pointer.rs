//! Real pointer acceptance for AI conversations and explicit human review.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use teshi_tui_runner::{Launch, Terminal};

const ORIGINAL: &str = "Feature: AI pointer fixture\n  Scenario: Ready\n    Given a sample step\n";

fn trace_event(path: &std::path::Path, event: serde_json::Value) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{event}").unwrap();
}

fn expect_wrapped_marker(terminal: &Terminal, marker: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let visible = terminal
            .screen()
            .unwrap()
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect::<String>();
        if visible.contains(marker) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "missing rendered marker {marker}: {visible}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct Fixture {
    root: tempfile::TempDir,
    project: PathBuf,
    source: PathBuf,
    artifacts: PathBuf,
    narrow: bool,
}

impl Fixture {
    fn new(name: &str, narrow: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        fs::create_dir(&project).unwrap();
        let source = project.join("sample.feature");
        fs::write(&source, ORIGINAL).unwrap();
        let artifacts = PathBuf::from(std::env::var_os("TESHI_AI_POINTER_ARTIFACTS").unwrap())
            .join(format!(
                "{name}-{}",
                if narrow { "narrow" } else { "normal" }
            ));
        fs::create_dir_all(&artifacts).unwrap();
        Self {
            root,
            project,
            source,
            artifacts,
            narrow,
        }
    }

    fn launch(&self, provider: Option<&OwnedProvider>, permission: bool) -> Terminal {
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
                self.root.path().join(key).to_string_lossy().into_owned(),
            )
        })
        .collect();
        env.push(("TERM".into(), "xterm-256color".into()));
        if permission {
            let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/permission_prompt_acp.py")
                .canonicalize()
                .unwrap();
            env.extend([
                ("TESHI_AGENT_BACKEND".into(), "acp".into()),
                (
                    "TESHI_ACP_PROGRAM".into(),
                    std::env::var("TESHI_TEST_PYTHON").unwrap_or_else(|_| {
                        if cfg!(windows) {
                            "python".into()
                        } else {
                            "python3".into()
                        }
                    }),
                ),
                (
                    "TESHI_ACP_ARGS_JSON".into(),
                    serde_json::to_string(&vec![fixture]).unwrap(),
                ),
            ]);
        } else {
            env.push(("TESHI_AGENT_BACKEND".into(), "native".into()));
            let directory = self.root.path().join("TESHI_APP_DATA_DIR/model-profiles");
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("Pointer.json"), serde_json::to_vec_pretty(&serde_json::json!({
                "id":"Pointer", "name":"Pointer local", "provider":"openai", "model_id":"pointer", "api_key":"POINTER-SYNTHETIC-LOCAL", "base_url":format!("http://{}/v1",provider.unwrap().address)
            })).unwrap()).unwrap();
            fs::write(directory.join("active"), "Pointer").unwrap();
        }
        let terminal = Terminal::launch(Launch {
            program: fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            args: vec![
                "--recursive".into(),
                self.project.to_string_lossy().into_owned(),
                "--requirements-root".into(),
                self.root
                    .path()
                    .join("requirements")
                    .to_string_lossy()
                    .into_owned(),
            ],
            cwd: Some(self.root.path().to_string_lossy().into_owned()),
            env,
            cols: if self.narrow { 55 } else { 150 },
            rows: if self.narrow { 16 } else { 30 },
        })
        .unwrap();
        terminal.expect_text("sample", true, 10000).unwrap();
        self.click(&terminal, "AI [3]", "ai-tab");
        terminal
    }

    fn capture(&self, terminal: &Terminal, name: &str) {
        trace_event(
            &self.artifacts.join("input-trace.jsonl"),
            serde_json::json!({"capture": name, "time": format!("{:?}", std::time::SystemTime::now())}),
        );
        std::thread::sleep(Duration::from_millis(100));
        fs::write(
            self.artifacts.join(format!("{name}.txt")),
            terminal.screen().unwrap(),
        )
        .unwrap();
        terminal
            .screenshot(&self.artifacts.join(format!("{name}.svg")))
            .unwrap();
    }

    fn click(&self, terminal: &Terminal, label: &str, name: &str) {
        self.capture(terminal, &format!("{name}-before"));
        terminal.click_text(label, 2000).unwrap();
        self.capture(terminal, &format!("{name}-after"));
    }

    fn background(&self, terminal: &Terminal) {
        terminal
            .write("\x1b[<0;2;1M\x1b[<0;2;1m\x1b[<65;2;1M")
            .unwrap();
        self.capture(terminal, "background-blocked");
    }

    fn finish(&self, terminal: Terminal) {
        terminal.key("Escape").unwrap();
        terminal.key("Escape").unwrap();
        terminal.key("ctrl+q").unwrap();
        terminal.expect_exit(0, 10000).unwrap();
    }
}

struct OwnedProvider {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl OwnedProvider {
    fn new(source: Option<PathBuf>, trace: PathBuf) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            let started = Instant::now();
            let mut turn = 0;
            let mut connection_id = 0;
            while !stop_worker.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut connection, _)) => {
                        connection_id += 1;
                        trace_event(
                            &trace,
                            serde_json::json!({"event":"accepted", "connection":connection_id, "elapsed_ms":started.elapsed().as_millis()}),
                        );
                        // Accepted sockets can inherit the nonblocking listener mode on
                        // Windows. A temporary lack of bytes is not an HTTP request end.
                        connection.set_nonblocking(false).unwrap();
                        connection
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut bytes = Vec::new();
                        let mut buffer = [0u8; 4096];
                        let mut complete = false;
                        loop {
                            let count = match connection.read(&mut buffer) {
                                Ok(count) => count,
                                Err(error) => {
                                    trace_event(
                                        &trace,
                                        serde_json::json!({"event":"read_error", "connection":connection_id, "received_bytes":bytes.len(), "kind":format!("{:?}", error.kind()), "os_error":error.raw_os_error(), "elapsed_ms":started.elapsed().as_millis()}),
                                    );
                                    break;
                                }
                            };
                            if count == 0 {
                                break;
                            }
                            bytes.extend_from_slice(&buffer[..count]);
                            if let Some(end) =
                                bytes.windows(4).position(|window| window == b"\r\n\r\n")
                            {
                                let headers = String::from_utf8_lossy(&bytes[..end]);
                                let length = headers
                                    .lines()
                                    .find_map(|line| {
                                        line.to_ascii_lowercase()
                                            .strip_prefix("content-length:")
                                            .map(|value| value.trim().parse::<usize>().unwrap())
                                    })
                                    .unwrap_or(0);
                                if bytes.len() >= end + 4 + length {
                                    complete = true;
                                    break;
                                }
                            }
                        }
                        if !complete {
                            trace_event(
                                &trace,
                                serde_json::json!({"event":"incomplete_request", "connection":connection_id, "received_bytes":bytes.len(), "elapsed_ms":started.elapsed().as_millis()}),
                            );
                            continue;
                        }
                        turn += 1;
                        trace_event(
                            &trace,
                            serde_json::json!({"event":"response", "connection":connection_id, "turn":turn, "request_bytes":bytes.len(), "tool_call":turn == 1 && source.is_some(), "elapsed_ms":started.elapsed().as_millis()}),
                        );
                        let (delta, finish) = if turn == 1
                            && let Some(source) = source.as_ref()
                        {
                            (
                                serde_json::json!({"tool_calls":[{"index":0,"id":"pointer-change","type":"function","function":{"name":"update_step","arguments":serde_json::json!({"file_path":source,"scenario_name":"Ready","step_index":0,"new_text":"POINTER_ACCEPTED_STEP"}).to_string()}}]}),
                                "tool_calls",
                            )
                        } else {
                            (
                                serde_json::json!({"content":"POINTER_REPLY_COMPLETE"}),
                                "stop",
                            )
                        };
                        let chunks = [
                            serde_json::json!({"id":"pointer","choices":[{"index":0,"delta":delta,"finish_reason":null}]}),
                            serde_json::json!({"id":"pointer","choices":[{"index":0,"delta":{},"finish_reason":finish}]}),
                        ];
                        let body = format!(
                            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                            chunks[0], chunks[1]
                        );
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let result = connection.write_all(response.as_bytes());
                        trace_event(
                            &trace,
                            serde_json::json!({"event":"write_result", "connection":connection_id, "error":result.err().map(|error| format!("{error:?}")), "elapsed_ms":started.elapsed().as_millis()}),
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("owned provider failed: {error}"),
                }
            }
        });
        Self {
            address,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for OwnedProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn provider_waits_for_complete_request_before_responding() {
    let root = tempfile::tempdir().unwrap();
    let provider = OwnedProvider::new(
        Some(root.path().join("sample.feature")),
        root.path().join("provider.jsonl"),
    );
    let mut connection = std::net::TcpStream::connect(provider.address).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    connection
        .write_all(b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 2\r\n\r\n{")
        .unwrap();
    let mut byte = [0];
    let result = connection.read(&mut byte);
    assert!(
        matches!(&result, Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)),
        "provider responded before the request body was complete: {result:?}"
    );
    connection.write_all(b"}").unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut response = String::new();
    connection.read_to_string(&mut response).unwrap();
    assert!(response.contains("pointer-change"), "{response}");
}

#[test]
fn provider_does_not_consume_a_turn_for_a_closed_connection() {
    let root = tempfile::tempdir().unwrap();
    let trace = root.path().join("provider.jsonl");
    let provider = OwnedProvider::new(Some(root.path().join("sample.feature")), trace.clone());
    drop(std::net::TcpStream::connect(provider.address).unwrap());
    let mut connection = std::net::TcpStream::connect(provider.address).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    connection
        .write_all(b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}")
        .unwrap();
    let mut response = String::new();
    connection.read_to_string(&mut response).unwrap();
    assert!(
        response.contains("pointer-change"),
        "an empty connection consumed the first tool-call response: {response}; {}",
        fs::read_to_string(trace).unwrap()
    );
}

#[test]
fn provider_does_not_respond_to_a_truncated_body() {
    let root = tempfile::tempdir().unwrap();
    let provider = OwnedProvider::new(
        Some(root.path().join("sample.feature")),
        root.path().join("provider.jsonl"),
    );
    let mut connection = std::net::TcpStream::connect(provider.address).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    connection
        .write_all(b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 2\r\n\r\n{")
        .unwrap();
    connection.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = String::new();
    connection.read_to_string(&mut response).unwrap();
    assert!(
        response.is_empty(),
        "responded to a truncated body: {response}"
    );
    let mut connection = std::net::TcpStream::connect(provider.address).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    connection
        .write_all(b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}")
        .unwrap();
    connection.read_to_string(&mut response).unwrap();
    assert!(response.contains("pointer-change"), "{response}");
}

fn footer_and_threads(narrow: bool) {
    let fixture = Fixture::new("threads-footer", narrow);
    let provider = OwnedProvider::new(None, fixture.artifacts.join("provider-trace.jsonl"));
    let terminal = fixture.launch(Some(&provider), false);
    fixture.click(&terminal, "Focus", "focus");
    terminal.write("THREAD_ORIGINAL").unwrap();
    if narrow {
        // doc-31/doc-36: below 70 columns the sidebar is hidden, so agent
        // management and switching move to the keyboard and the transcript is
        // the only per-agent evidence.
        let trace = fixture.artifacts.join("provider-trace.jsonl");
        let accepted = |path: &std::path::Path| {
            fs::read_to_string(path)
                .map(|text| text.matches("\"accepted\"").count())
                .unwrap_or(0)
        };
        let before = accepted(&trace);
        assert!(!terminal.screen().unwrap().contains("New [a]"));
        fixture.click(&terminal, "Send", "send-first");
        terminal
            .expect_text("POINTER_REPLY_COMPLETE", true, 10000)
            .unwrap();
        terminal.key("Escape").unwrap();
        terminal.key("a").unwrap();
        terminal.write("THREAD_SECOND").unwrap();
        fixture.click(&terminal, "Send", "send-second");
        std::thread::sleep(Duration::from_millis(800));
        terminal.key("Escape").unwrap();
        terminal.key("[").unwrap();
        terminal.key("]").unwrap();
        terminal.key("x").unwrap();
        fixture.capture(&terminal, "narrow-after-close");
        assert_eq!(accepted(&trace), before + 2, "expected one turn per thread");
    } else {
        fixture.click(&terminal, "New [a]", "new-first");
        assert_eq!(
            terminal
                .screen()
                .unwrap()
                .lines()
                .filter(|line| line.contains("Agent 1"))
                .count(),
            2
        );
        terminal.write("THREAD_SECOND").unwrap();
        fixture.click(&terminal, "New [a]", "new-second");
        terminal.expect_text("Agent 2", true, 5000).unwrap();
        let screen = terminal.screen().unwrap();
        let rows = screen
            .lines()
            .enumerate()
            .filter_map(|(row, line)| {
                line.find("Agent 1")
                    .map(|column| (row + 1, line[..column].chars().count() + 1))
            })
            .collect::<Vec<_>>();
        assert!(
            rows.len() >= 2,
            "missing two distinct thread rows: {screen}"
        );
        for (index, input) in ["THREAD_ORIGINAL", "THREAD_SECOND"].into_iter().enumerate() {
            let (row, column) = rows[index];
            terminal
                .write(&format!("\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m"))
                .unwrap();
            terminal.expect_text(input, true, 5000).unwrap();
            fixture.capture(&terminal, &format!("thread-row-{index}-input"));
        }
        fixture.click(&terminal, "Agent 2", "select-third");
        fixture.click(&terminal, "Close [x]", "close-third");
        terminal.expect_text("Agent 2", false, 5000).unwrap();
    }
    fixture.click(&terminal, "Blur", "blur");
    fixture.click(&terminal, "Model", "model");
    terminal.expect_text("Model", true, 5000).unwrap();
    fixture.capture(&terminal, "model-visible");
    terminal.key("Escape").unwrap();
    fixture.click(&terminal, "Focus", "focus-before-send");
    fixture.click(&terminal, "Send", "send");
    expect_wrapped_marker(&terminal, "POINTER_REPLY_COMPLETE");
    fixture.capture(&terminal, "reply-visible");
    fixture.click(&terminal, "Blur", "blur-before-quit");
    if fixture.narrow {
        // The pinned exit control starts at 60 columns; narrow terminals keep
        // the chord as their exit affordance.
        terminal.key("ctrl+q").unwrap();
    } else {
        fixture.click(&terminal, "^q", "quit");
    }
    terminal.expect_exit(0, 10000).unwrap();
    assert_eq!(fs::read_to_string(&fixture.source).unwrap(), ORIGINAL);
    let sessions = fixture.root.path().join("TESHI_SESSION_DIR");
    let stored = fs::read_dir(sessions)
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(stored.contains("THREAD_SECOND") && stored.contains("POINTER_REPLY_COMPLETE"));
    fs::write(fixture.artifacts.join("sessions.json.txt"), stored).unwrap();
}

fn slash_rows(narrow: bool) {
    let fixture = Fixture::new("slash", narrow);
    let provider = OwnedProvider::new(None, fixture.artifacts.join("provider-trace.jsonl"));
    let terminal = fixture.launch(Some(&provider), false);
    fixture.click(&terminal, "Focus", "focus");
    terminal.write("/model").unwrap();
    terminal.expect_text("/models", true, 5000).unwrap();
    fixture.click(&terminal, "/models", "choose-models");
    fixture.click(&terminal, "Select", "select-command");
    terminal.expect_text("Model Profiles", true, 5000).unwrap();
    terminal.key("Escape").unwrap();
    fixture.click(&terminal, "Blur", "blur-after-model");
    fixture.click(&terminal, "Focus", "focus-dismiss");
    terminal.write("/copy").unwrap();
    fixture.click(&terminal, "Dismiss", "dismiss");
    assert!(!terminal.screen().unwrap().contains(" Select"));
    assert!(terminal.screen().unwrap().contains("/copy"));
    fixture.finish(terminal);
}

fn permission(narrow: bool, allow: bool) {
    let fixture = Fixture::new(
        if allow {
            "permission-allow"
        } else {
            "permission-reject"
        },
        narrow,
    );
    let terminal = fixture.launch(None, true);
    fixture.click(&terminal, "Focus", "focus");
    terminal.write("Inspect owned project").unwrap();
    fixture.click(&terminal, "Send", "send");
    terminal.expect_text("Allow once", true, 5000).unwrap();
    fixture.background(&terminal);
    assert!(terminal.screen().unwrap().contains("Allow once"));
    fixture.click(&terminal, "Allow always", "choose-always");
    assert!(!terminal.screen().unwrap().contains("PERMISSION_AFTER"));
    fixture.click(
        &terminal,
        if allow {
            "Allow [Enter]"
        } else {
            "Reject [Esc]"
        },
        "decision",
    );
    if allow {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let visible = terminal
                .screen()
                .unwrap()
                .chars()
                .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect::<String>();
            if visible.contains("PERMISSION_AFTER_allow_always") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "wrong rendered permission choice: {visible}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    } else {
        terminal
            .expect_text("ACP permission requested", false, 5000)
            .unwrap();
        assert!(!terminal.screen().unwrap().contains("PERMISSION_AFTER"));
    }
    fixture.capture(&terminal, "decision-result");
    assert_eq!(fs::read_to_string(&fixture.source).unwrap(), ORIGINAL);
    fixture.finish(terminal);
}

fn pending_change(narrow: bool, accept: bool, inspect_diff: bool) {
    let fixture = Fixture::new(
        if inspect_diff {
            "change-diff"
        } else if accept {
            "change-accept"
        } else {
            "change-reject"
        },
        narrow,
    );
    let provider = OwnedProvider::new(
        Some(fixture.source.clone()),
        fixture.artifacts.join("provider-trace.jsonl"),
    );
    let terminal = fixture.launch(Some(&provider), false);
    fixture.click(&terminal, "Focus", "focus");
    terminal
        .write("Change the step in the owned project")
        .unwrap();
    fixture.click(&terminal, "Send", "send");
    if let Err(error) = terminal.expect_text("Change Summary", true, 10000) {
        fixture.capture(&terminal, "summary-timeout");
        panic!(
            "{error}; provider trace: {}",
            fixture.artifacts.join("provider-trace.jsonl").display()
        );
    }
    fixture.capture(&terminal, "summary-before-review");
    fixture.click(&terminal, "Close", "close-summary");
    terminal
        .expect_text("Pending Agent Change", true, 5000)
        .unwrap();
    fixture.background(&terminal);
    assert!(terminal.screen().unwrap().contains("Pending Agent Change"));
    assert_eq!(fs::read_to_string(&fixture.source).unwrap(), ORIGINAL);
    fixture.click(&terminal, "Summary", "open-summary-from-review");
    terminal.expect_text("Change Summary", true, 5000).unwrap();
    fixture.click(&terminal, "Close", "close-summary-again");
    if inspect_diff {
        fixture.click(&terminal, "Diff", "open-diff");
        terminal.expect_text("Steps (diff)", true, 5000).unwrap();
        fixture.capture(&terminal, "diff-visible");
        terminal.key("d").unwrap();
        terminal.expect_text("Steps (diff)", false, 5000).unwrap();
        terminal
            .expect_text("Pending Agent Change", true, 5000)
            .unwrap();
        terminal.key("d").unwrap();
        terminal.expect_text("Steps (diff)", true, 5000).unwrap();
        fixture.capture(&terminal, "diff-reopened-with-keyboard");
    }
    fixture.click(
        &terminal,
        if accept { "Accept" } else { "Reject" },
        "decision",
    );
    if accept {
        // The Summary view leaves MindMap in its normal navigation mode,
        // whose documented save key is `s`.
        terminal.key("s").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            fs::read_to_string(&fixture.source)
                .unwrap()
                .contains("Given POINTER_ACCEPTED_STEP")
        );
    } else {
        assert_eq!(fs::read_to_string(&fixture.source).unwrap(), ORIGINAL);
    }
    fixture.click(&terminal, "AI [3]", "return-ai");
    expect_wrapped_marker(&terminal, "POINTER_REPLY_COMPLETE");
    fs::write(
        fixture.artifacts.join("source-after.feature.txt"),
        fs::read_to_string(&fixture.source).unwrap(),
    )
    .unwrap();
    fixture.capture(&terminal, "decision-result");
    fixture.finish(terminal);
}

macro_rules! acceptance {
    ($name:ident, $body:expr) => {
        #[test]
        #[ignore = "requires a checkout-built Teshi binary and real PTY"]
        fn $name() {
            $body
        }
    };
}

acceptance!(threads_footer_normal, footer_and_threads(false));
acceptance!(threads_footer_narrow, footer_and_threads(true));
acceptance!(slash_rows_normal, slash_rows(false));
acceptance!(slash_rows_narrow, slash_rows(true));
acceptance!(permission_allow_normal, permission(false, true));
acceptance!(permission_allow_narrow, permission(true, true));
acceptance!(permission_reject_normal, permission(false, false));
acceptance!(permission_reject_narrow, permission(true, false));
acceptance!(
    pending_change_accept_normal,
    pending_change(false, true, false)
);
acceptance!(
    pending_change_accept_narrow,
    pending_change(true, true, false)
);
acceptance!(
    pending_change_reject_normal,
    pending_change(false, false, false)
);
acceptance!(
    pending_change_reject_narrow,
    pending_change(true, false, false)
);
acceptance!(
    pending_change_diff_normal,
    pending_change(false, false, true)
);
acceptance!(
    pending_change_diff_narrow,
    pending_change(true, false, true)
);
