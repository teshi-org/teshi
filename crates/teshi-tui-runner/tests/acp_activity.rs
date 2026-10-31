//! Real PTY coverage for the compact ACP transcript and details command.

use teshi_tui_runner::{Launch, Terminal};

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn acp_reply_color_and_event_order_in_real_terminal() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let gate = root.path().join("gate");
    let output = std::env::var_os("TESHI_ACTIVITY_ARTIFACTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.path().join("screenshots"));
    std::fs::create_dir_all(&output).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../teshi-agent-runtime/tests/fixtures/fake_acp_agent.py")
        .canonicalize()
        .unwrap();
    let launch = Launch {
        program: std::env::var("TESHI_SELF_TEST_BIN").unwrap(),
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
                "TESHI_APP_DATA_DIR".into(),
                root.path().join("app-data").to_string_lossy().into_owned(),
            ),
            // The host agent shell disables color; this regression measures it.
            ("NO_COLOR".into(), String::new()),
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
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
            (
                "TESHI_SESSION_DIR".into(),
                root.path().join("sessions").to_string_lossy().into_owned(),
            ),
            (
                "TESHI_ACP_STORE_DIR".into(),
                root.path().join("acp").to_string_lossy().into_owned(),
            ),
            ("TESHI_AGENT_BACKEND".into(), "acp".into()),
            (
                "TESHI_ACP_PROGRAM".into(),
                if cfg!(windows) { "python" } else { "python3" }.into(),
            ),
            (
                "TESHI_ACP_ARGS_JSON".into(),
                serde_json::to_string(&[fixture.to_string_lossy().as_ref(), "visual_success"])
                    .unwrap(),
            ),
            (
                "TESHI_ACTIVITY_GATE".into(),
                gate.to_string_lossy().into_owned(),
            ),
        ],
        cols: 120,
        rows: 40,
    };
    let terminal = Terminal::launch(launch.clone()).unwrap();
    terminal.expect_text("Explore", true, 15000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat!", true, 15000)
        .unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("Inspect the project").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("I will inspect the project", true, 15000)
        .unwrap();
    assert_svg_headers(&terminal, &output.join("streaming.svg"), 1, 0);
    std::fs::write(gate.with_extension("tools"), "continue").unwrap();
    terminal
        .expect_text("Inspection finished", true, 15000)
        .unwrap();
    assert_svg_headers(&terminal, &output.join("before-completion.svg"), 2, 0);
    let screen = terminal.screen().unwrap();
    let progress = screen.find("I will inspect the project").unwrap();
    let tool = screen.find("Done  Read project").unwrap();
    let final_reply = screen.find("Inspection finished").unwrap();
    assert!(progress < tool && tool < final_reply, "{screen}");
    assert_eq!(screen.matches("Read project").count(), 1);
    std::fs::write(gate.with_extension("finish"), "continue").unwrap();
    assert_svg_headers(&terminal, &output.join("completed.svg"), 1, 1);
    terminal.key("Escape").unwrap();
    terminal.key("ctrl+q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
    drop(terminal);

    // A fresh isolated storage root avoids selecting the completed conversation.
    let mut recovery_launch = launch;
    for (key, value) in &mut recovery_launch.env {
        if key == "TESHI_SESSION_DIR" {
            *value = root
                .path()
                .join("recovery-sessions")
                .to_string_lossy()
                .into_owned();
        } else if key == "TESHI_ACTIVITY_GATE" {
            *value = root
                .path()
                .join("recovery-gate")
                .to_string_lossy()
                .into_owned();
        }
    }
    let terminal = Terminal::launch(recovery_launch.clone()).unwrap();
    terminal.expect_text("Explore", true, 15000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat!", true, 15000)
        .unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("Inspect this project").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("I will inspect the project", true, 15000)
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let checkpoint = std::fs::read_dir(root.path().join("recovery-sessions"))
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .any(|entry| {
                std::fs::read_to_string(entry.path()).is_ok_and(|text| {
                    text.contains("I will inspect the project") && text.contains("agent_progress")
                })
            });
        if checkpoint {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "unfinished checkpoint was not saved"
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    // Dropping the owned PTY terminates the process without a graceful save.
    drop(terminal);
    let terminal = Terminal::launch(recovery_launch).unwrap();
    terminal.expect_text("Explore", true, 15000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat!", true, 15000)
        .unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("/resume").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("I will inspect the project", true, 15000)
        .unwrap();
    assert_svg_headers(&terminal, &output.join("recovered.svg"), 1, 0);
    terminal.key("Escape").unwrap();
    terminal.key("ctrl+q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
    println!("Color and order evidence: {}", output.display());
}

fn assert_svg_headers(terminal: &Terminal, path: &std::path::Path, muted: usize, green: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        terminal.screenshot(path).unwrap();
        let svg = std::fs::read_to_string(path).unwrap();
        let headers: Vec<_> = svg
            .split("<text ")
            .filter_map(|part| {
                let (attributes, rest) = part.split_once('>')?;
                let text = rest.split_once("</text>")?.0;
                (text.trim().ends_with("Teshi") && !text.contains("Welcome")).then_some(attributes)
            })
            .collect();
        if headers
            .iter()
            .filter(|s| s.contains("fill=\"#949494\""))
            .count()
            == muted
            && headers
                .iter()
                .filter(|s| s.contains("fill=\"#5fff87\""))
                .count()
                == green
            && headers.len() == muted + green
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "wrong header colors: {headers:?}; {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

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
                "TESHI_APP_DATA_DIR".into(),
                root.path().join("app-data").to_string_lossy().into_owned(),
            ),
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
            (
                "TESHI_SESSION_DIR".into(),
                root.path().join("sessions").to_string_lossy().into_owned(),
            ),
            (
                "TESHI_ACP_STORE_DIR".into(),
                root.path().join("acp").to_string_lossy().into_owned(),
            ),
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
    terminal.key("ctrl+q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
    println!("Screenshots: {}", output.display());
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn interrupted_acp_task_can_continue_in_real_terminal() {
    for use_button in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let gate = root.path().join("gate");
        let output = std::env::var_os("TESHI_ACTIVITY_ARTIFACTS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.path().join("screenshots"));
        let output = output.join(if use_button { "button" } else { "slash" });
        std::fs::create_dir_all(&output).unwrap();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../teshi-agent-runtime/tests/fixtures/fake_acp_agent.py")
            .canonicalize()
            .unwrap();
        let launch = Launch {
            program: std::env::var("TESHI_SELF_TEST_BIN").unwrap(),
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
                    "TESHI_APP_DATA_DIR".into(),
                    root.path().join("app-data").to_string_lossy().into_owned(),
                ),
                // The host agent shell disables color; this regression measures it.
                ("NO_COLOR".into(), String::new()),
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
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
                (
                    "TESHI_SESSION_DIR".into(),
                    root.path().join("sessions").to_string_lossy().into_owned(),
                ),
                (
                    "TESHI_ACP_STORE_DIR".into(),
                    root.path().join("acp").to_string_lossy().into_owned(),
                ),
                ("TESHI_AGENT_BACKEND".into(), "acp".into()),
                (
                    "TESHI_ACP_PROGRAM".into(),
                    if cfg!(windows) { "python" } else { "python3" }.into(),
                ),
                (
                    "TESHI_ACP_ARGS_JSON".into(),
                    serde_json::to_string(&[fixture.to_string_lossy().as_ref(), "refusal_once"])
                        .unwrap(),
                ),
                (
                    "TESHI_ACTIVITY_GATE".into(),
                    gate.to_string_lossy().into_owned(),
                ),
            ],
            cols: 120,
            rows: 40,
        };
        let terminal = Terminal::launch(launch).unwrap();
        terminal.expect_text("Explore", true, 15000).unwrap();
        terminal.key("3").unwrap();
        terminal
            .expect_text("Welcome to AI Chat!", true, 15000)
            .unwrap();
        terminal.key("Enter").unwrap();
        terminal.write("Inspect the project").unwrap();
        terminal.key("Enter").unwrap();
        terminal
            .expect_text("Task interrupted", true, 15000)
            .unwrap();
        terminal
            .expect_text("[Continue task]", true, 15000)
            .unwrap();
        terminal
            .screenshot(&output.join("interrupted.svg"))
            .unwrap();
        std::fs::write(output.join("interrupted.txt"), terminal.screen().unwrap()).unwrap();
        if use_button {
            terminal.click_text("[Continue task]", 15000).unwrap();
        } else {
            terminal.write("/retry").unwrap();
            terminal.key("Enter").unwrap();
        }
        terminal
            .expect_text("Continued in original session", true, 15000)
            .unwrap();
        terminal
            .expect_text("[Continue task]", false, 2000)
            .unwrap();
        terminal.screenshot(&output.join("continued.svg")).unwrap();
        terminal.key("Escape").unwrap();
        terminal.key("ctrl+q").unwrap();
        terminal.expect_exit(0, 10000).unwrap();
        let sessions = std::fs::read_dir(root.path().join("sessions")).unwrap();
        let transcript = sessions
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(transcript.contains("agent returned refusal"));
        assert!(transcript.contains("Continued in original session"));
    }
}
