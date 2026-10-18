//! Real PTY acceptance for Native and ACP completed-answer copying.

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
fn acp_chat_copy_command_round_trip() {
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
