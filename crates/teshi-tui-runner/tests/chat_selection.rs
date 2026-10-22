//! Real PTY acceptance for ACP chat copying and selection.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use arboard::Clipboard;
use teshi_tui_runner::{Launch, Terminal};

/// Clipboard tests run serially because the desktop clipboard is global.
static CLIPBOARD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct ClipboardRestore {
    clipboard: Clipboard,
    previous: Option<String>,
}

impl ClipboardRestore {
    fn new(mut clipboard: Clipboard) -> Self {
        let previous = clipboard.get_text().ok();
        Self {
            clipboard,
            previous,
        }
    }

    fn get_text(&mut self) -> Result<String, arboard::Error> {
        self.clipboard.get_text()
    }

    fn set_text(&mut self, text: &str) -> Result<(), arboard::Error> {
        self.clipboard.set_text(text)
    }
}

impl Drop for ClipboardRestore {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            let _ = self.clipboard.set_text(previous);
        }
    }
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI, Python, and a desktop clipboard"]
fn acp_chat_copy_command_and_selection_round_trip() {
    let _serial = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let binary = std::fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wrapped_chat_acp.py")
        .canonicalize()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("sample.feature"),
        "Feature: Chat selection fixture\n  Scenario: Ready\n    Given a sample step\n",
    )
    .unwrap();

    let clipboard =
        Clipboard::new().expect("desktop clipboard is required for the chat copy acceptance");
    let mut clipboard = ClipboardRestore::new(clipboard);
    clipboard
        .set_text("CHAT_SELECTION_CLIPBOARD_SENTINEL")
        .unwrap();
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
                "TESHI_APP_DATA_DIR".into(),
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
            ("TESHI_AGENT_BACKEND".into(), "acp".into()),
            ("TESHI_ACP_PROGRAM".into(), python()),
            (
                "TESHI_ACP_ARGS_JSON".into(),
                serde_json::to_string(&vec![fixture]).unwrap(),
            ),
        ],
        cols: 100,
        rows: 30,
    })
    .unwrap();

    terminal.expect_text("Explore", true, 15000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat!", true, 15000)
        .unwrap();
    terminal.key("Enter").unwrap();

    terminal.write("first response").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("WRAPPED_REPLY_01", true, 15000)
        .unwrap();
    terminal.write("second response").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("WRAPPED_REPLY_02", true, 15000)
        .unwrap();

    terminal.write("/copy 2").unwrap();
    terminal.expect_text("/copy 2", true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("/copy 2", false, 5000).unwrap();
    assert_clipboard(
        &mut clipboard,
        "WRAPPED_REPLY_01\n\n---\n\nWRAPPED_REPLY_02",
    );

    terminal.drag_text("WRAPPED_REPLY_01", 5000).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));
    let selection_screenshot = root.path().join("chat-selection.svg");
    terminal.screenshot(&selection_screenshot).unwrap();
    let screenshot = std::fs::read_to_string(&selection_screenshot).unwrap();
    assert!(screenshot.contains("WRAPPED_REPLY_01"));
    save_viewport(&terminal, root.path(), "acp-original-selection");
    terminal.expect_text("Focus [i/Enter]", true, 5000).unwrap();
    clipboard
        .set_text("CHAT_SELECTION_AFTER_DRAG_SENTINEL")
        .unwrap();
    terminal.right_click_text("WRAPPED_REPLY_01", 5000).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !matches!(clipboard.get_text().as_deref(), Ok("WRAPPED_REPLY_01"))
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert_clipboard(&mut clipboard, "WRAPPED_REPLY_01");

    terminal.key("Escape").unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

struct NativeFixture {
    child: Child,
    trace: PathBuf,
}

impl Drop for NativeFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI, Python, and a desktop clipboard"]
fn native_chat_copy_command_reads_completed_responses() {
    let _serial = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let binary = std::fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/agent_acceptance_fixture.py")
        .canonicalize()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("sample.feature"),
        "Feature: Native chat fixture\n  Scenario: Ready\n    Given a sample step\n",
    )
    .unwrap();

    let mut clipboard = ClipboardRestore::new(
        Clipboard::new().expect("desktop clipboard is required for native acceptance"),
    );
    clipboard
        .set_text("NATIVE_CHAT_CLIPBOARD_SENTINEL")
        .unwrap();
    let native = start_native_fixture(&fixture, root.path());
    let port = std::fs::read_to_string(native.trace.join("native-port"))
        .unwrap()
        .trim()
        .to_owned();
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
                "TESHI_APP_DATA_DIR".into(),
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
            ("TESHI_AGENT_BACKEND".into(), "native".into()),
            ("TESHI_LLM_API_KEY".into(), "native-test-key".into()),
            (
                "TESHI_LLM_BASE_URL".into(),
                format!("http://127.0.0.1:{port}/v1"),
            ),
            ("TESHI_LLM_MODEL".into(), "acceptance".into()),
            ("NO_PROXY".into(), "127.0.0.1,localhost,::1".into()),
        ],
        cols: 100,
        rows: 30,
    })
    .unwrap();

    terminal.expect_text("Explore", true, 15000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat!", true, 15000)
        .unwrap();
    terminal.key("Enter").unwrap();

    terminal.write("native first response").unwrap();
    terminal.key("Enter").unwrap();
    let first_turn = wait_for_native_turn(&native.trace, 0).unwrap_or_else(|| {
        panic!(
            "native fixture did not receive turn 0\n{}",
            terminal.screen().unwrap()
        )
    });
    write_native_events(
        &native.trace,
        &first_turn,
        &[
            json_event("reply", "NATIVE_REPLY_01"),
            json_event("done", ""),
        ],
    );
    terminal
        .expect_text("NATIVE_REPLY_01", true, 15000)
        .unwrap();
    // A visible streamed chunk precedes completion; wait for the rendered
    // idle indicator before submitting another turn.
    terminal.expect_text("○ native first", true, 15000).unwrap();

    terminal.write("native second response").unwrap();
    terminal.key("Enter").unwrap();
    let second_turn = wait_for_native_turn(&native.trace, 1).unwrap_or_else(|| {
        panic!(
            "native fixture did not receive turn 1\n{}",
            terminal.screen().unwrap()
        )
    });
    write_native_events(
        &native.trace,
        &second_turn,
        &[
            json_event("reply", "NATIVE_REPLY_02"),
            json_event("done", ""),
        ],
    );
    terminal
        .expect_text("NATIVE_REPLY_02", true, 15000)
        .unwrap();
    terminal.expect_text("○ native first", true, 15000).unwrap();

    terminal.write("/copy 2").unwrap();
    terminal.expect_text("/copy 2", true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("/copy 2", false, 5000).unwrap();
    assert_clipboard(&mut clipboard, "NATIVE_REPLY_01\n\n---\n\nNATIVE_REPLY_02");
    terminal
        .expect_stable_text("NATIVE_REPLY_02", true, 5000, 250)
        .unwrap();
    assert_clipboard(&mut clipboard, "NATIVE_REPLY_01\n\n---\n\nNATIVE_REPLY_02");
    save_viewport(&terminal, root.path(), "native-original-copy");

    // Replacing another application's clipboard value must also keep ownership
    // alive across later event-loop frames on X11 without a clipboard manager.
    clipboard.set_text("NATIVE_SECOND_COPY_SENTINEL").unwrap();
    terminal.write("/copy 1").unwrap();
    terminal.expect_text("/copy 1", true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("/copy 1", false, 5000).unwrap();
    terminal
        .expect_stable_text("NATIVE_REPLY_02", true, 5000, 250)
        .unwrap();
    assert_clipboard(&mut clipboard, "NATIVE_REPLY_02");
    save_viewport(&terminal, root.path(), "native-second-copy");
    terminal.drag_text("NATIVE_REPLY_01", 5000).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    save_viewport(&terminal, root.path(), "native-response-selection");
    clipboard.set_text("NATIVE_SELECTION_SENTINEL").unwrap();
    terminal.right_click_text("NATIVE_REPLY_01", 5000).unwrap();
    assert_clipboard(&mut clipboard, "NATIVE_REPLY_01");
    terminal
        .expect_stable_text("NATIVE_REPLY_02", true, 5000, 250)
        .unwrap();
    assert_clipboard(&mut clipboard, "NATIVE_REPLY_01");

    terminal.key("Escape").unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

fn start_native_fixture(fixture: &Path, root: &Path) -> NativeFixture {
    let trace = root.join("native-trace");
    std::fs::create_dir(&trace).unwrap();
    let python = std::env::var("TESHI_TEST_PYTHON").unwrap_or_else(|_| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    });
    let child = Command::new(python)
        .args([
            fixture.to_string_lossy().as_ref(),
            "native",
            trace.to_string_lossy().as_ref(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let port_file = trace.join("native-port");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !port_file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(port_file.exists(), "native fixture did not publish a port");
    NativeFixture { child, trace }
}

fn wait_for_native_turn(trace: &Path, expected_index: usize) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(trace.join("trace.jsonl")) {
            let turns = content
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .filter(|record| {
                    record.get("kind").and_then(serde_json::Value::as_str) == Some("native")
                })
                .collect::<Vec<_>>();
            if let Some(turn) = turns.get(expected_index)
                && let Some(id) = turn.get("turn").and_then(serde_json::Value::as_str)
            {
                return Some(id.to_owned());
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    None
}

fn json_event(kind: &str, text: &str) -> serde_json::Value {
    serde_json::json!({"kind": kind, "text": text})
}

fn write_native_events(trace: &Path, turn: &str, events: &[serde_json::Value]) {
    let body = events
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(trace.join(format!("{turn}.jsonl")), format!("{body}\n")).unwrap();
}

fn python() -> String {
    std::env::var("TESHI_TEST_PYTHON").unwrap_or_else(|_| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    })
}

struct ChatCase {
    root: tempfile::TempDir,
    project: PathBuf,
    name: &'static str,
}

impl ChatCase {
    fn new(name: &'static str, messages: &[serde_json::Value]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(
            project.join("sample.feature"),
            "Feature: Chat selection acceptance\n  Scenario: Ready\n    Given a sample step\n",
        )
        .unwrap();
        let sessions = root.path().join("sessions");
        std::fs::create_dir(&sessions).unwrap();
        if !messages.is_empty() {
            let session = serde_json::json!({
                "id": "chat-projection-fixture",
                "created_at": "1",
                "updated_at": "9999999999999",
                "model_label": null,
                "message_count": messages.len(),
                "messages": messages,
                "project_root": project,
                "backend_kind": "acp",
                "acp_agent_id": null
            });
            std::fs::write(
                sessions.join("chat-projection-fixture.json"),
                serde_json::to_vec_pretty(&session).unwrap(),
            )
            .unwrap();
        }
        Self {
            root,
            project,
            name,
        }
    }

    fn launch(&self, cols: u16, rows: u16) -> Terminal {
        self.launch_agent(cols, rows, false)
    }

    fn launch_agent(&self, cols: u16, rows: u16, controlled: bool) -> Terminal {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(if controlled {
                "tests/fixtures/agent_acceptance_fixture.py"
            } else {
                "tests/fixtures/wrapped_chat_acp.py"
            })
            .canonicalize()
            .unwrap();
        let mut agent_args = vec![fixture.to_string_lossy().into_owned()];
        if controlled {
            agent_args.extend([
                "acp".into(),
                self.root
                    .path()
                    .join("controlled-trace")
                    .to_string_lossy()
                    .into_owned(),
                "p3-chat".into(),
                "controlled".into(),
            ]);
        }
        let mut env = vec![
            ("TESHI_AGENT_BACKEND".into(), "acp".into()),
            ("TESHI_ACP_PROGRAM".into(), python()),
            (
                "TESHI_ACP_ARGS_JSON".into(),
                serde_json::to_string(&agent_args).unwrap(),
            ),
            ("NO_COLOR".into(), String::new()),
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
        ];
        for (key, subdir) in [
            ("APPDATA", "appdata"),
            ("TESHI_APP_DATA_DIR", "appdata"),
            ("LOCALAPPDATA", "localdata"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("TESHI_SESSION_DIR", "sessions"),
            ("TESHI_ACP_STORE_DIR", "acp"),
        ] {
            env.push((
                key.into(),
                self.root.path().join(subdir).to_string_lossy().into_owned(),
            ));
        }
        let terminal = Terminal::launch(Launch {
            program: std::env::var("TESHI_SELF_TEST_BIN").unwrap(),
            args: vec![
                self.project.to_string_lossy().into_owned(),
                "--requirements-root".into(),
                self.root
                    .path()
                    .join("requirements")
                    .to_string_lossy()
                    .into_owned(),
            ],
            cwd: Some(self.root.path().to_string_lossy().into_owned()),
            env: {
                env.push((
                    "TESHI_DIAG_PATH".into(),
                    self.root
                        .path()
                        .join("input-diag.log")
                        .to_string_lossy()
                        .into_owned(),
                ));
                env
            },
            cols,
            rows,
        })
        .unwrap();
        terminal.expect_text("Explore", true, 15000).unwrap();
        terminal.key("3").unwrap();
        terminal
            .expect_text("Welcome to AI Chat!", true, 15000)
            .unwrap();
        terminal.key("Enter").unwrap();
        terminal
    }

    fn save(&self, terminal: &Terminal, stage: &str) {
        save_viewport(
            terminal,
            self.root.path(),
            &format!("{}-{stage}", self.name),
        );
        if let Some(output) = std::env::var_os("TESHI_CHAT_ARTIFACTS") {
            let diagnostic = self.root.path().join("input-diag.log");
            let probe = diagnostic.with_extension("clipboard-probe");
            if probe.exists() {
                std::fs::copy(
                    probe,
                    PathBuf::from(&output)
                        .join(format!("{}-{stage}-clipboard-probe.txt", self.name)),
                )
                .unwrap();
            }
            if diagnostic.exists() {
                std::fs::copy(
                    diagnostic,
                    PathBuf::from(&output).join(format!("{}-{stage}-input.log", self.name)),
                )
                .unwrap();
            }
            let trace = self.root.path().join("controlled-trace/trace.jsonl");
            if trace.exists() {
                std::fs::copy(
                    trace,
                    PathBuf::from(output).join(format!("{}-trace.jsonl", self.name)),
                )
                .unwrap();
            }
        }
    }
}

fn message(role: &str, content: &str, source: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "role": role,
        "content": content,
        "source": source,
        "reasoning_content": null,
        "tool_calls": null,
        "tool_call_id": null
    })
}

fn command(terminal: &Terminal, text: &str) {
    // A bare /copy completes to /copy on the first Enter. A trailing space
    // dismisses that suggestion so this helper submits the default command.
    let input = if text == "/copy" {
        format!("{text} ")
    } else {
        text.into()
    };
    terminal.write(&input).unwrap();
    terminal.expect_text(text, true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text(text, false, 5000).unwrap();
}

fn close_chat(terminal: &Terminal) {
    terminal.key("Escape").unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

fn assert_clipboard(clipboard: &mut ClipboardRestore, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(actual) = clipboard.get_text()
            && actual == expected
        {
            if let Some(output) = std::env::var_os("TESHI_CHAT_ARTIFACTS") {
                use std::io::Write;
                std::fs::create_dir_all(&output).unwrap();
                let mut records = std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(PathBuf::from(output).join("clipboard-readbacks.jsonl"))
                    .unwrap();
                writeln!(
                    records,
                    "{}",
                    serde_json::json!({
                        "test": std::thread::current().name(),
                        "expected": expected,
                        "observed": actual
                    })
                )
                .unwrap();
            }
            return;
        }
        assert!(
            Instant::now() < deadline,
            "clipboard mismatch: expected {expected:?}, received {:?}",
            clipboard.get_text()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn save_viewport(terminal: &Terminal, temporary: &Path, stage: &str) {
    let output = std::env::var_os("TESHI_CHAT_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| temporary.join("screenshots"));
    std::fs::create_dir_all(&output).unwrap();
    terminal
        .screenshot(&output.join(format!("{stage}.svg")))
        .unwrap();
    std::fs::write(
        output.join(format!("{stage}.screen.txt")),
        terminal.screen().unwrap(),
    )
    .unwrap();
}

fn assert_selection_highlight(temporary: &Path, stage: &str, needle: &str) {
    let output = std::env::var_os("TESHI_CHAT_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| temporary.join("screenshots"));
    let svg = std::fs::read_to_string(output.join(format!("{stage}.svg"))).unwrap();
    assert!(
        svg.split("<text ").any(|part| {
            part.split_once('>').is_some_and(|(attributes, content)| {
                attributes.contains("fill=\"#303030\"")
                    && attributes.contains("font-weight=\"bold\"")
                    && content.split("</text>").next().unwrap().contains(needle)
            })
        }),
        "selected text {needle:?} does not use the highlighted foreground"
    );
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and a desktop clipboard"]
fn copy_command_rejects_empty_invalid_and_nonanswer_history_in_real_terminal() {
    let _serial = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut clipboard = ClipboardRestore::new(Clipboard::new().unwrap());
    let case = ChatCase::new("command-errors", &[]);
    let terminal = case.launch(120, 35);
    clipboard.set_text("COPY_ERROR_SENTINEL").unwrap();
    for (input, status) in [
        ("/copy", "No completed assistant responses to copy"),
        ("/copy 0", "Usage: /copy [N] where N is a positive integer"),
        (
            "/copy invalid",
            "Usage: /copy [N] where N is a positive integer",
        ),
    ] {
        command(&terminal, input);
        terminal.expect_text(status, true, 5000).unwrap();
        assert_clipboard(&mut clipboard, "COPY_ERROR_SENTINEL");
    }
    case.save(&terminal, "invalid-count");
    close_chat(&terminal);

    let mut activity = message("Assistant", "TOOL_TITLE_ONLY", Some("acp_activity"));
    activity["reasoning_content"] = "completed".into();
    let mut reasoning = message("Assistant", "", Some("acp_reasoning"));
    reasoning["reasoning_content"] = "REASONING_ONLY".into();
    let case = ChatCase::new(
        "answer-filter",
        &[
            message("Assistant", "FIRST_COMPLETED", None),
            activity,
            reasoning,
            message("Assistant", "PARTIAL_EXCLUDED", Some("agent_progress")),
            message("Assistant", "SECOND_COMPLETED", None),
            message("Assistant", "  ", None),
        ],
    );
    let terminal = case.launch(120, 45);
    command(&terminal, "/resume");
    terminal
        .expect_text("SECOND_COMPLETED", true, 5000)
        .unwrap();
    command(&terminal, "/copy");
    assert_clipboard(&mut clipboard, "SECOND_COMPLETED");
    command(&terminal, "/copy 50");
    assert_clipboard(&mut clipboard, "FIRST_COMPLETED\n\n---\n\nSECOND_COMPLETED");
    case.save(&terminal, "completed-only");
    close_chat(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and a desktop clipboard"]
fn chat_selection_preserves_wrapped_code_projection_across_resize_and_mindmap() {
    let _serial = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut clipboard = ClipboardRestore::new(Clipboard::new().unwrap());
    let body = "SOURCE_START a long line of words that must wrap within the narrow chat viewport while retaining its original source newline boundary SOURCE_END\n\n```rust\n    let value = 42;\n    println!(\"done\");\n```\nCODE_END";
    let case = ChatCase::new("wrapped-source", &[message("Assistant", body, None)]);
    let terminal = case.launch(70, 36);
    command(&terminal, "/resume");
    terminal.expect_text("CODE_END", true, 5000).unwrap();
    drag_between_text(&terminal, "SOURCE_START", "CODE_END", false);
    case.save(&terminal, "selected-narrow");
    assert_selection_highlight(
        case.root.path(),
        "wrapped-source-selected-narrow",
        "SOURCE_START",
    );
    terminal.resize(120, 40).unwrap();
    terminal
        .expect_stable_text("CODE_END", true, 5000, 100)
        .unwrap();
    case.save(&terminal, "selected-wide");
    assert_selection_highlight(
        case.root.path(),
        "wrapped-source-selected-wide",
        "SOURCE_START",
    );
    clipboard.set_text("SOURCE_RESIZE_SENTINEL").unwrap();
    terminal.right_click_text("CODE_END", 5000).unwrap();
    assert_clipboard(
        &mut clipboard,
        "SOURCE_START a long line of words that must wrap within the narrow chat viewport while retaining its original source newline boundary SOURCE_END\n\n  rust\n      let value = 42;\n      println!(\"done\");\n  \n\nCODE_END",
    );

    terminal.key("2").unwrap();
    terminal
        .expect_stable_text("╭MindMap", true, 5000, 100)
        .unwrap();
    terminal.expect_text("CODE_END", true, 5000).unwrap();
    drag_between_text(&terminal, "SOURCE_START", "CODE_END", true);
    case.save(&terminal, "mindmap-reverse-selection");
    assert_selection_highlight(
        case.root.path(),
        "wrapped-source-mindmap-reverse-selection",
        "SOURCE_START",
    );
    clipboard.set_text("MINDMAP_SENTINEL").unwrap();
    terminal.right_click_text("CODE_END", 5000).unwrap();
    assert_clipboard(
        &mut clipboard,
        "SOURCE_START a long line of words that must wrap within the narrow chat viewport while retaining its original source newline boundary SOURCE_END\n\n  rust\n      let value = 42;\n      println!(\"done\");\n  \n\nCODE_END",
    );
    // Leave the embedded AI pane before using the MindMap tab shortcuts.
    terminal.key("Escape").unwrap();
    terminal.key("3").unwrap();
    terminal.expect_text("╭ Agents", true, 5000).unwrap();
    terminal.expect_text("CODE_END", true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    close_chat(&terminal);
}

fn locate_text(terminal: &Terminal, needle: &str) -> (usize, usize) {
    let screen = terminal.screen().unwrap();
    screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find(needle)
                .map(|byte| (line[..byte].chars().count() + 1, row + 1))
        })
        .unwrap_or_else(|| panic!("text {needle:?} not visible\n{screen}"))
}

fn drag_between_text(terminal: &Terminal, first: &str, last: &str, reverse: bool) {
    terminal.expect_text(first, true, 5000).unwrap();
    terminal.expect_text(last, true, 5000).unwrap();
    let start = locate_text(terminal, first);
    let (last_col, last_row) = locate_text(terminal, last);
    let end = (last_col + last.chars().count(), last_row);
    let (start, end) = if reverse { (end, start) } else { (start, end) };
    terminal
        .write(&format!(
            "\x1b[<0;{};{}M\x1b[<32;{};{}M\x1b[<0;{};{}m",
            start.0, start.1, end.0, end.1, end.0, end.1
        ))
        .unwrap();
    std::thread::sleep(Duration::from_millis(80));
}

#[cfg(windows)]
struct LockedClipboard(*mut std::ffi::c_void);

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn OpenClipboard(owner: *mut std::ffi::c_void) -> i32;
    fn CloseClipboard() -> i32;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: *mut std::ffi::c_void,
        menu: *mut std::ffi::c_void,
        instance: *mut std::ffi::c_void,
        param: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn DestroyWindow(window: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
impl LockedClipboard {
    fn acquire() -> Self {
        // A real hidden owner window makes clipboard contention observable
        // across processes; a NULL owner did not reliably hold it in ConPTY.
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert!(!window.is_null());
        assert_ne!(unsafe { OpenClipboard(window) }, 0);
        let guard = Self(window);
        let probe = Command::new(python()).args(["-c", "import ctypes; u=ctypes.WinDLL('user32'); opened=u.OpenClipboard(None); print(opened); u.CloseClipboard() if opened else None"]).output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&probe.stdout).trim(),
            "0",
            "clipboard fault was not held across processes"
        );
        guard
    }
}

#[cfg(windows)]
impl Drop for LockedClipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
            DestroyWindow(self.0);
        }
    }
}

#[test]
#[cfg(windows)]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and a desktop clipboard"]
fn clipboard_failure_keeps_chat_selection_and_allows_retry() {
    let _serial = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut clipboard = ClipboardRestore::new(Clipboard::new().unwrap());
    let case = ChatCase::new(
        "clipboard-failure",
        &[message("Assistant", "COPY_RETRY_SELECTION", None)],
    );
    let terminal = case.launch(120, 35);
    command(&terminal, "/resume");
    terminal
        .expect_text("COPY_RETRY_SELECTION", true, 5000)
        .unwrap();
    terminal.drag_text("COPY_RETRY_SELECTION", 5000).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    case.save(&terminal, "before-failed-write");
    clipboard.set_text("LOCKED_CLIPBOARD_SENTINEL").unwrap();
    {
        let _lock = LockedClipboard::acquire();
        let (col, row) = locate_text(&terminal, "COPY_RETRY_SELECTION");
        terminal
            .write(&format!("\x1b[<2;{col};{row}M\x1b[<2;{col};{row}m"))
            .unwrap();
        // ConPTY can postpone repaint while the desktop clipboard is held.
        // Release the injected fault before observing the error and retry UI.
        std::thread::sleep(Duration::from_millis(200));
    }
    terminal
        .expect_text("Could not copy to clipboard:", true, 10000)
        .unwrap();
    case.save(&terminal, "failed-write");
    assert_clipboard(&mut clipboard, "LOCKED_CLIPBOARD_SENTINEL");
    terminal
        .right_click_text("COPY_RETRY_SELECTION", 5000)
        .unwrap();
    assert_clipboard(&mut clipboard, "COPY_RETRY_SELECTION");
    case.save(&terminal, "retried-selection");
    terminal.key("Enter").unwrap();
    terminal.write("/copy ").unwrap();
    terminal.expect_text("/copy", true, 5000).unwrap();
    {
        let _lock = LockedClipboard::acquire();
        terminal.key("Enter").unwrap();
        std::thread::sleep(Duration::from_millis(200));
    }
    terminal
        .expect_text("Could not copy to clipboard:", true, 10000)
        .unwrap();
    case.save(&terminal, "command-failed-write");
    assert_clipboard(&mut clipboard, "COPY_RETRY_SELECTION");
    clipboard.set_text("COMMAND_RETRY_SENTINEL").unwrap();
    command(&terminal, "/copy");
    assert_clipboard(&mut clipboard, "COPY_RETRY_SELECTION");
    case.save(&terminal, "retried-command");
    close_chat(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and a desktop clipboard"]
fn chat_selection_copies_unicode_across_messages_and_completed_tool_title() {
    let _serial = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut clipboard = ClipboardRestore::new(Clipboard::new().unwrap());
    let mut activity = message("Assistant", "READ_PROJECT_TOOL", Some("acp_activity"));
    activity["reasoning_content"] = "completed".into();
    activity["tool_call_id"] = "fixture-read".into();
    let case = ChatCase::new(
        "unicode-tool",
        &[
            message("Assistant", "UNICODE_START\n中文 e\u{301} 😀", None),
            message("Assistant", "UNICODE_END", None),
            activity,
        ],
    );
    let terminal = case.launch(120, 40);
    command(&terminal, "/resume");
    terminal
        .expect_text("READ_PROJECT_TOOL", true, 5000)
        .unwrap();
    drag_between_text(&terminal, "UNICODE_START", "UNICODE_END", true);
    case.save(&terminal, "reverse-cross-message-selection");
    terminal.right_click_text("UNICODE_END", 5000).unwrap();
    assert_clipboard(
        &mut clipboard,
        "UNICODE_START\n中文 e\u{301} 😀\n\nUNICODE_END",
    );
    terminal.drag_text("READ_PROJECT_TOOL", 5000).unwrap();
    case.save(&terminal, "completed-tool-selection");
    terminal
        .right_click_text("READ_PROJECT_TOOL", 5000)
        .unwrap();
    assert_clipboard(&mut clipboard, "READ_PROJECT_TOOL");
    terminal.key("Enter").unwrap();
    command(&terminal, "/copy 2");
    assert_clipboard(
        &mut clipboard,
        "UNICODE_START\n中文 e\u{301} 😀\n\n---\n\nUNICODE_END",
    );
    close_chat(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and a desktop clipboard"]
fn chat_selection_survives_scrolling_out_of_view_and_new_session_clears_it() {
    let _serial = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut clipboard = ClipboardRestore::new(Clipboard::new().unwrap());
    let body = (0..80)
        .map(|index| format!("SCROLL_LINE_{index:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let case = ChatCase::new("scroll-session", &[message("Assistant", &body, None)]);
    let terminal = case.launch(120, 32);
    command(&terminal, "/resume");
    terminal.expect_text("SCROLL_LINE_79", true, 5000).unwrap();
    terminal.drag_text("SCROLL_LINE_78", 5000).unwrap();
    case.save(&terminal, "before-scroll");
    let (col, row) = locate_text(&terminal, "SCROLL_LINE_78");
    for _ in 0..12 {
        terminal
            .scroll_at((col - 1) as u16, (row - 1) as u16, true, false)
            .unwrap();
    }
    terminal.expect_text("SCROLL_LINE_78", false, 5000).unwrap();
    case.save(&terminal, "selection-out-of-view");
    terminal
        .write(&format!("\x1b[<2;{col};{row}M\x1b[<2;{col};{row}m"))
        .unwrap();
    assert_clipboard(&mut clipboard, "SCROLL_LINE_78");

    // Restore the lower viewport, select a message, then replace its session.
    terminal.key("Enter").unwrap();
    terminal.key("Control+End").unwrap();
    terminal.expect_text("SCROLL_LINE_79", true, 5000).unwrap();
    terminal.drag_text("SCROLL_LINE_78", 5000).unwrap();
    terminal.key("Enter").unwrap();
    command(&terminal, "/new");
    terminal
        .expect_text("Welcome to AI Chat!", true, 5000)
        .unwrap();
    clipboard.set_text("SESSION_REPLACEMENT_SENTINEL").unwrap();
    terminal
        .right_click_text("Welcome to AI Chat!", 5000)
        .unwrap();
    terminal
        .expect_text("No selection to copy", true, 5000)
        .unwrap();
    assert_clipboard(&mut clipboard, "SESSION_REPLACEMENT_SENTINEL");
    case.save(&terminal, "new-session-cleared-selection");
    command(&terminal, "/resume");
    terminal.expect_text("SCROLL_LINE_79", true, 5000).unwrap();
    terminal.right_click_text("SCROLL_LINE_79", 5000).unwrap();
    terminal
        .expect_text("No selection to copy", true, 5000)
        .unwrap();
    assert_clipboard(&mut clipboard, "SESSION_REPLACEMENT_SENTINEL");
    case.save(&terminal, "resume-cleared-selection");
    close_chat(&terminal);
    drop(terminal);

    let terminal = case.launch(120, 32);
    command(&terminal, "/resume");
    terminal.expect_text("SCROLL_LINE_79", true, 5000).unwrap();
    terminal.right_click_text("SCROLL_LINE_79", 5000).unwrap();
    terminal
        .expect_text("No selection to copy", true, 5000)
        .unwrap();
    assert_clipboard(&mut clipboard, "SESSION_REPLACEMENT_SENTINEL");
    case.save(&terminal, "restart-cleared-selection");
    close_chat(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI, Python, and a desktop clipboard"]
fn chat_selection_invalidates_changed_stream_and_copy_excludes_live_partial() {
    let _serial = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut clipboard = ClipboardRestore::new(Clipboard::new().unwrap());
    let case = ChatCase::new("stream-selection", &[]);
    let terminal = case.launch_agent(120, 40, true);
    terminal.write("stream selection acceptance").unwrap();
    terminal.key("Enter").unwrap();
    let trace = case.root.path().join("controlled-trace");
    let deadline = Instant::now() + Duration::from_secs(10);
    let turn = loop {
        let prompt = std::fs::read_to_string(trace.join("trace.jsonl"))
            .ok()
            .into_iter()
            .flat_map(|text| {
                text.lines()
                    .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                    .collect::<Vec<_>>()
            })
            .find(|value| value["kind"] == "prompt");
        if let Some(prompt) = prompt {
            break prompt["turn"].as_str().unwrap().to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "ACP fixture did not receive a prompt"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    write_native_events(&trace, &turn, &[json_event("reply", "STREAM_SELECTED")]);
    terminal.expect_text("STREAM_SELECTED", true, 5000).unwrap();
    clipboard.set_text("STREAM_PARTIAL_SENTINEL").unwrap();
    command(&terminal, "/copy");
    terminal
        .expect_text("No completed assistant responses to copy", true, 5000)
        .unwrap();
    assert_clipboard(&mut clipboard, "STREAM_PARTIAL_SENTINEL");

    terminal.drag_text("STREAM_SELECTED", 5000).unwrap();
    case.save(&terminal, "selected-partial");
    write_native_events(
        &trace,
        &turn,
        &[
            json_event("reply", "STREAM_SELECTED"),
            json_event("reply", "_UPDATED"),
        ],
    );
    terminal
        .expect_text("STREAM_SELECTED_UPDATED", true, 5000)
        .unwrap();
    terminal
        .right_click_text("STREAM_SELECTED_UPDATED", 5000)
        .unwrap();
    terminal
        .expect_text("No selection to copy", true, 5000)
        .unwrap();
    assert_clipboard(&mut clipboard, "STREAM_PARTIAL_SENTINEL");
    case.save(&terminal, "changed-partial-cleared-selection");

    write_native_events(
        &trace,
        &turn,
        &[
            json_event("reply", "STREAM_SELECTED"),
            json_event("reply", "_UPDATED"),
            json_event("done", ""),
        ],
    );
    terminal
        .expect_stable_text("STREAM_SELECTED_UPDATED", true, 5000, 100)
        .unwrap();
    terminal.key("Enter").unwrap();
    command(&terminal, "/copy");
    assert_clipboard(&mut clipboard, "STREAM_SELECTED_UPDATED");
    case.save(&terminal, "completed-stream-copy");
    close_chat(&terminal);
}
