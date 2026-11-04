//! Real PTY coverage for passive chat and explicitly requested ACP startup.
use std::{fs, path::PathBuf, time::Duration};
use teshi_tui_runner::{Launch, Terminal};

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN and TESHI_ACP_DISCOVERY_ARTIFACTS"]
fn restored_agent_stays_disconnected_until_explicit_startup() {
    for cols in [100, 64, 40] {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let store = root.path().join("acp");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&store).unwrap();
        fs::write(project.join("sample.feature"), "Feature: Discovery\n").unwrap();
        let provider = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/lazy_start_acp.py")
            .canonicalize()
            .unwrap();
        let trace = root.path().join("acp-trace.jsonl");
        fs::write(store.join("installed.json"), serde_json::to_vec(&serde_json::json!([
            {"id":"fixture", "name":"Fixture", "program": if cfg!(windows) {"python"} else {"python3"}, "args":[provider,trace,"ok"], "env":{}}
        ])).unwrap()).unwrap();
        fs::write(
            store.join("last-selection.json"),
            r#"{"backend":"acp","agent_id":"fixture"}"#,
        )
        .unwrap();
        let mut env: Vec<(String, String)> = [
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "TESHI_APP_DATA_DIR",
            "TESHI_SESSION_DIR",
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
            "TESHI_ACP_STORE_DIR".into(),
            store.to_string_lossy().into_owned(),
        ));
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
            cwd: Some(project.to_string_lossy().into_owned()),
            env,
            cols,
            rows: 24,
        })
        .unwrap();
        // Toolbars narrower than 32 cells collapse the connected Settings chip to an ellipsis.
        let settings = if cols == 40 { "⋯" } else { "Settings" };
        terminal.expect_text("sample", true, 10000).unwrap();
        terminal.key("3").unwrap();
        terminal.expect_text("Not connected", true, 5000).unwrap();
        let output = PathBuf::from(std::env::var("TESHI_ACP_DISCOVERY_ARTIFACTS").unwrap())
            .join(cols.to_string());
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("disconnected.txt"), terminal.screen().unwrap()).unwrap();
        for _ in 0..3 {
            terminal.key("1").unwrap();
            terminal.key("3").unwrap();
            terminal.resize(cols + 2, 25).unwrap();
            terminal.resize(cols, 24).unwrap();
        }
        std::thread::sleep(Duration::from_millis(400));
        assert!(!trace.exists(), "browsing must not spawn an ACP process");
        if cols == 64 {
            // The composer's prompt glyph; the usage text now sits on the toolbar above it.
            terminal.click_text("›", 2000).unwrap();
        } else {
            terminal
                .key(if cols == 40 { "i" } else { "Enter" })
                .unwrap();
        }
        terminal.expect_text("Loading", true, 5000).unwrap();
        let first = terminal.screen().unwrap();
        fs::write(output.join("loading-first.txt"), &first).unwrap();
        std::thread::sleep(Duration::from_millis(170));
        let second = terminal.screen().unwrap();
        fs::write(output.join("loading-second.txt"), &second).unwrap();
        assert_ne!(first, second, "loading feedback must animate");
        terminal.expect_text(settings, true, 10000).unwrap();
        assert_eq!(count_method(&trace, "spawn"), 1);
        assert_eq!(count_method(&trace, "session/new"), 1);
        assert_eq!(
            count_method(&trace, "session/prompt"),
            0,
            "focus must not send a prompt"
        );
        for _ in 0..2 {
            terminal.key("Escape").unwrap();
            terminal.key("Enter").unwrap();
        }
        assert_eq!(count_method(&trace, "spawn"), 1);
        assert_eq!(count_method(&trace, "session/prompt"), 0);
        terminal.write("first explicit prompt").unwrap();
        terminal.key("Enter").unwrap();
        terminal.expect_text("LAZY_REPLY_1", true, 10000).unwrap();
        terminal.key("Escape").unwrap();
        terminal.click_text(settings, 1500).unwrap();
        terminal.expect_text("Model: Alpha", true, 3000).unwrap();
        fs::write(output.join("settings.txt"), terminal.screen().unwrap()).unwrap();
        terminal.key("Enter").unwrap();
        terminal.expect_text("Beta", true, 3000).unwrap();
        fs::write(output.join("model-picker.txt"), terminal.screen().unwrap()).unwrap();
        terminal.key("Escape").unwrap();
        terminal.key("Escape").unwrap();
        terminal.key("1").unwrap();
        terminal.key("3").unwrap();
        assert_eq!(count_method(&trace, "spawn"), 1);
        assert_eq!(count_method(&trace, "session/new"), 1);
        assert_eq!(count_method(&trace, "session/prompt"), 1);
        terminal.key("Enter").unwrap();
        terminal.write("/new").unwrap();
        terminal.key("Tab").unwrap();
        terminal.expect_text("LAZY_REPLY_1", false, 10000).unwrap();
        terminal.expect_text(settings, true, 10000).unwrap();
        terminal.write("/resume").unwrap();
        terminal.key("Enter").unwrap();
        terminal.expect_text("LAZY_REPLY_1", true, 10000).unwrap();
        terminal.expect_text("Not connected", true, 5000).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(
            count_method(&trace, "spawn"),
            2,
            "history restore is passive"
        );
        fs::write(output.join("restored.txt"), terminal.screen().unwrap()).unwrap();
        terminal.key("Enter").unwrap();
        terminal.expect_text(settings, true, 10000).unwrap();
        assert_eq!(count_method(&trace, "spawn"), 3);
        assert_eq!(count_method(&trace, "session/new"), 3);
        assert_eq!(count_method(&trace, "session/prompt"), 1);
        fs::copy(&trace, output.join("acp-trace.jsonl")).unwrap();
        terminal.close().unwrap();
    }
}

fn count_method(path: &std::path::Path, method: &str) -> usize {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["method"] == method)
        .count()
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN and TESHI_ACP_DISCOVERY_ARTIFACTS"]
fn failed_explicit_selection_does_not_retry_while_browsing_and_can_be_reselected() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let store = root.path().join("TESHI_ACP_STORE_DIR");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::write(project.join("sample.feature"), "Feature: Startup failure\n").unwrap();
    let provider = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lazy_start_acp.py")
        .canonicalize()
        .unwrap();
    let trace = root.path().join("acp-trace.jsonl");
    let configure = |mode: &str| {
        fs::write(store.join("installed.json"), serde_json::to_vec(&serde_json::json!([
            {"id":"fixture", "name":"Fixture", "program": if cfg!(windows) {"python"} else {"python3"}, "args":[provider,trace,mode], "env":{}}
        ])).unwrap()).unwrap();
    };
    configure("fail");
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
        cwd: Some(project.to_string_lossy().into_owned()),
        env: [
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
        .collect(),
        cols: 64,
        rows: 24,
    })
    .unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("No agent selected", true, 5000)
        .unwrap();
    assert!(!trace.exists());
    let select = || {
        terminal.key("Enter").unwrap();
        terminal.write("/agent").unwrap();
        terminal.key("Enter").unwrap();
        terminal.expect_text("Fixture", true, 5000).unwrap();
        terminal.key("Down").unwrap();
        terminal.key("Enter").unwrap();
    };
    select();
    terminal.expect_text("Error", true, 10000).unwrap();
    terminal.key("Escape").unwrap();
    terminal.click_text("Error", 1500).unwrap();
    // The transcript guide narrows the body, so match a phrase that stays on one row.
    terminal
        .expect_text("fixture rejected", true, 5000)
        .unwrap();
    let output =
        PathBuf::from(std::env::var("TESHI_ACP_DISCOVERY_ARTIFACTS").unwrap()).join("failure");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("startup-error.txt"), terminal.screen().unwrap()).unwrap();
    terminal.key("Escape").unwrap();
    for _ in 0..3 {
        terminal.key("1").unwrap();
        terminal.key("3").unwrap();
        terminal.key("Enter").unwrap();
        terminal.key("Escape").unwrap();
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(count_method(&trace, "spawn"), 1);
    assert_eq!(count_method(&trace, "session/prompt"), 0);
    configure("ok");
    select();
    terminal.expect_text("Settings", true, 10000).unwrap();
    assert_eq!(count_method(&trace, "spawn"), 2);
    assert_eq!(count_method(&trace, "session/prompt"), 0);
    terminal.write("after explicit retry").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("LAZY_REPLY_1", true, 10000).unwrap();
    assert_eq!(count_method(&trace, "spawn"), 2);
    assert_eq!(count_method(&trace, "session/prompt"), 1);
    fs::copy(&trace, output.join("acp-trace.jsonl")).unwrap();
    terminal.close().unwrap();
}
