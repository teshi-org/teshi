//! Real PTY coverage for restored ACP discovery and loading feedback.
use std::{fs, path::PathBuf, time::Duration};
use teshi_tui_runner::{Launch, Terminal};

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN and TESHI_ACP_DISCOVERY_ARTIFACTS"]
fn restored_agent_loads_on_ai_entry() {
    for cols in [100, 40] {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let store = root.path().join("acp");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&store).unwrap();
        fs::write(project.join("sample.feature"), "Feature: Discovery\n").unwrap();
        let provider = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../teshi-agent-runtime/tests/fixtures/fake_acp_agent.py")
            .canonicalize()
            .unwrap();
        let wrapper = root.path().join("slow_agent.py");
        fs::write(&wrapper, format!(
            "import runpy,sys,time\ntime.sleep(2)\nsys.argv=[{},'settings']\nrunpy.run_path(sys.argv[0],run_name='__main__')\n",
            serde_json::to_string(&provider.to_string_lossy()).unwrap()
        )).unwrap();
        fs::write(store.join("installed.json"), serde_json::to_vec(&serde_json::json!([
            {"id":"fixture", "name":"Fixture", "program": if cfg!(windows) {"python"} else {"python3"}, "args":[wrapper], "env":{}}
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
        terminal.expect_text("sample", true, 10000).unwrap();
        terminal.key("3").unwrap();
        terminal.expect_text("Loading", true, 5000).unwrap();
        let output = PathBuf::from(std::env::var("TESHI_ACP_DISCOVERY_ARTIFACTS").unwrap())
            .join(cols.to_string());
        fs::create_dir_all(&output).unwrap();
        let first = terminal.screen().unwrap();
        fs::write(output.join("loading-first.txt"), &first).unwrap();
        std::thread::sleep(Duration::from_millis(170));
        let second = terminal.screen().unwrap();
        fs::write(output.join("loading-second.txt"), &second).unwrap();
        assert_ne!(first, second, "loading feedback must animate");
        terminal.expect_text("Settings", true, 10000).unwrap();
        terminal.click_text("Settings", 1500).unwrap();
        terminal.expect_text("Model: Alpha", true, 3000).unwrap();
        fs::write(output.join("settings.txt"), terminal.screen().unwrap()).unwrap();
        terminal.key("Enter").unwrap();
        terminal.expect_text("Beta", true, 3000).unwrap();
        fs::write(output.join("model-picker.txt"), terminal.screen().unwrap()).unwrap();
        terminal.close().unwrap();
    }
}
