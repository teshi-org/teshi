//! Isolated diagnostic for idle ACP shutdown latency in a real terminal.
use std::time::Instant;
use teshi_tui_runner::{Launch, Terminal};

fn main() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let project = root.path().join("project");
    std::fs::create_dir(&project)?;
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../teshi-agent-runtime/tests/fixtures/fake_acp_agent.py")
        .canonicalize()?;
    for acp in [false, true] {
        let mut env = Vec::new();
        for (key, directory) in [
            ("APPDATA", "appdata"),
            ("LOCALAPPDATA", "localdata"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("TESHI_SESSION_DIR", "sessions"),
            ("TESHI_ACP_STORE_DIR", "acp"),
        ] {
            env.push((
                key.into(),
                root.path().join(directory).to_string_lossy().into_owned(),
            ));
        }
        env.push((
            "TESHI_AGENT_BACKEND".into(),
            if acp { "acp" } else { "native" }.into(),
        ));
        env.push(("TESHI_ACP_PROGRAM".into(), "python".into()));
        env.push((
            "TESHI_ACP_ARGS_JSON".into(),
            serde_json::to_string(&[fixture.to_string_lossy().as_ref(), "activity"])?,
        ));
        let terminal = Terminal::launch(Launch {
            program: std::env::var("TESHI_SELF_TEST_BIN")?,
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
            cols: 120,
            rows: 40,
        })?;
        terminal.expect_text("Explore", true, 15000)?;
        if acp {
            terminal.key("3")?;
            terminal.expect_text("Welcome to AI Chat!", true, 15000)?;
            terminal.key("Enter")?;
            terminal.write("hello")?;
            terminal.key("Enter")?;
            terminal.expect_text("Inspection finished", true, 15000)?;
            terminal.key("Escape")?;
        }
        // Allow the local fixture to finish its initialization before measuring.
        std::thread::sleep(std::time::Duration::from_secs(1));
        let start = Instant::now();
        terminal.key("q")?;
        terminal.expect_exit(0, 15000)?;
        println!(
            "backend={} quit_ms={}",
            if acp { "acp" } else { "native" },
            start.elapsed().as_millis()
        );
    }
    Ok(())
}
