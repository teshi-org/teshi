use std::io::{BufRead, Write};

use teshi_tui_runner::{Launch, Terminal};

#[cfg(windows)]
struct ProcessProbe(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl ProcessProbe {
    fn open(pid: u32) -> Self {
        // Keep a handle to this exact owned process, avoiding PID reuse ambiguity.
        let handle =
            unsafe { windows_sys::Win32::System::Threading::OpenProcess(0x00100000, 0, pid) };
        assert!(!handle.is_null(), "could not observe owned fixture process");
        Self(handle)
    }

    fn running(&self) -> bool {
        let result =
            unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(self.0, 0) };
        assert_ne!(result, windows_sys::Win32::Foundation::WAIT_FAILED);
        result == windows_sys::Win32::Foundation::WAIT_TIMEOUT
    }
}

#[cfg(windows)]
impl Drop for ProcessProbe {
    fn drop(&mut self) {
        // The probe owns only this observation handle, never process termination.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(unix)]
struct ProcessProbe(libc::pid_t);

#[cfg(unix)]
impl ProcessProbe {
    fn open(pid: u32) -> Self {
        Self(pid.try_into().unwrap())
    }

    fn running(&self) -> bool {
        // Signal zero observes the fixture without sending a signal to it.
        let result = unsafe { libc::kill(self.0, 0) };
        if result == 0 {
            return true;
        }
        let error = std::io::Error::last_os_error();
        assert_eq!(
            error.raw_os_error(),
            Some(libc::ESRCH),
            "fixture process probe failed"
        );
        false
    }
}

fn assert_terminated(probe: &ProcessProbe) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while probe.running() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        !probe.running(),
        "owned fixture process survived PTY cleanup"
    );
}

#[test]
fn child_fixture() {
    if std::env::var_os("TESHI_TUI_RUNNER_FIXTURE").is_none() {
        return;
    }
    let mut out = std::io::stdout();
    write!(out, "\x1b[2J\x1b[Hfixture-ready\r\n").unwrap();
    out.flush().unwrap();
    for line in std::io::stdin().lock().lines() {
        match line.unwrap().trim() {
            "replace" => {
                write!(out, "\x1b[2J\x1b[Hreplacement-screen\r\n").unwrap();
                out.flush().unwrap();
            }
            "quit" => return,
            _ => {}
        }
    }
}

fn fixture() -> Launch {
    Launch {
        program: std::env::current_exe().unwrap().to_string_lossy().into(),
        args: vec![
            "--exact".into(),
            "child_fixture".into(),
            "--nocapture".into(),
        ],
        cwd: None,
        env: vec![("TESHI_TUI_RUNNER_FIXTURE".into(), "1".into())],
        cols: 80,
        rows: 24,
    }
}

#[test]
fn real_pty_input_repaint_resize_and_exit() {
    let terminal = Terminal::launch(fixture()).unwrap();
    terminal.expect_text("fixture-ready", true, 5000).unwrap();
    terminal.write("replace").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("replacement-screen", true, 5000)
        .unwrap();
    terminal.expect_text("fixture-ready", false, 1000).unwrap();
    terminal.resize(100, 30).unwrap();
    assert!(terminal.resize(0, 30).is_err());
    terminal.write("quit").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_exit(0, 5000).unwrap();
    terminal.close().unwrap();
}

#[test]
fn timeout_contains_current_screen_and_drop_closes_session() {
    let terminal = Terminal::launch(fixture()).unwrap();
    terminal.expect_text("fixture-ready", true, 5000).unwrap();
    let probe = ProcessProbe::open(terminal.process_id().unwrap());
    assert!(probe.running());
    let error = terminal
        .expect_text("never-rendered", true, 50)
        .unwrap_err();
    assert!(error.to_string().contains("fixture-ready"));
    assert!(terminal.expect_text("", true, 50).is_err());
    assert!(terminal.expect_exit(0, 0).is_err());
    assert!(terminal.expect_text("ready", true, u64::MAX).is_err());
    assert!(terminal.key("not-a-real-key").is_err());
    drop(terminal);
    assert_terminated(&probe);
    // A fresh session must be independent of the dropped application.
    let next = Terminal::launch(fixture()).unwrap();
    next.expect_text("fixture-ready", true, 5000).unwrap();
    let next_probe = ProcessProbe::open(next.process_id().unwrap());
    next.close().unwrap();
    assert_terminated(&next_probe);
}

#[test]
fn rejects_invalid_launch_before_spawning() {
    let mut config = fixture();
    config.cols = 0;
    assert!(Terminal::launch(config).is_err());
    let mut config = fixture();
    config.program.clear();
    assert!(Terminal::launch(config).is_err());
}

#[test]
fn canonical_working_directory_starts_a_visible_application() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = fixture();
    config.cwd = Some(
        directory
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    );
    let terminal = Terminal::launch(config).unwrap();
    terminal.expect_text("fixture-ready", true, 5000).unwrap();
}

#[test]
fn scenario_protocol_replays_background_and_preserves_failure_screen() {
    use std::collections::HashMap;
    use teshi_tui_runner::scenario::{Action, Binding, Bindings, Case, Request, run};

    let directory = tempfile::tempdir().unwrap();
    let feature = directory.path().join("terminal.feature");
    std::fs::write(&feature, "Feature: Terminal\n  Background:\n    Given the application is running\n  Scenario: Replace screen\n    When I replace the screen\n    Then the replacement appears\n  Scenario: Missing content\n    Then missing content appears\n").unwrap();
    let binding = |line, text: &str, actions| Binding {
        line,
        text: text.into(),
        confirmed: true,
        actions,
    };
    let bindings = Bindings {
        version: 1,
        mode: "tui".into(),
        steps: vec![
            binding(
                3,
                "the application is running",
                vec![
                    Action::Launch { config: fixture() },
                    Action::ExpectText {
                        text: "fixture-ready".into(),
                        visible: true,
                        timeout_ms: 5000,
                    },
                ],
            ),
            binding(
                5,
                "I replace the screen",
                vec![
                    Action::Write {
                        text: "replace".into(),
                    },
                    Action::Key {
                        key: "Enter".into(),
                    },
                ],
            ),
            binding(
                6,
                "the replacement appears",
                vec![Action::ExpectText {
                    text: "replacement-screen".into(),
                    visible: true,
                    timeout_ms: 5000,
                }],
            ),
            binding(
                8,
                "missing content appears",
                vec![Action::ExpectText {
                    text: "missing-marker".into(),
                    visible: true,
                    timeout_ms: 50,
                }],
            ),
        ],
    };
    std::fs::write(
        feature.with_extension("tui.json"),
        serde_json::to_vec(&bindings).unwrap(),
    )
    .unwrap();
    let request = Request {
        command: "run".into(),
        cases: vec![
            Case {
                id: "pass".into(),
                feature_path: "terminal.feature".into(),
                scenario: "Replace screen".into(),
                line_number: Some(4),
                until_line: None,
            },
            Case {
                id: "fail".into(),
                feature_path: "terminal.feature".into(),
                scenario: "Missing content".into(),
                line_number: Some(7),
                until_line: None,
            },
        ],
        meta: HashMap::from([(
            "project_root".into(),
            directory.path().to_string_lossy().into_owned(),
        )]),
    };
    let mut output = Vec::new();
    assert_eq!(run(request, &mut output).unwrap(), 1);
    let events: Vec<serde_json::Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "case_passed" && e["case_id"] == "pass")
    );
    let failure = events.iter().find(|e| e["type"] == "case_failed").unwrap();
    assert!(
        failure["error"]["message"]
            .as_str()
            .unwrap()
            .contains("line 8")
    );
    let evidence = failure["error"]["attachments"][0]["path"].as_str().unwrap();
    assert!(
        std::fs::read_to_string(evidence)
            .unwrap()
            .contains("fixture-ready")
    );
    assert_eq!(events.last().unwrap()["failed"], 1);
}
