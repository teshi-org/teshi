//! Real PTY coverage of built-in and project-owned chat profile titles.
use std::{fs, path::Path, time::Duration};
use teshi_tui_runner::{Launch, Terminal};

fn launch(project: &Path, root: &Path) -> Terminal {
    let binary = fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
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
    .map(|key| (key.into(), root.join(key).to_string_lossy().into_owned()))
    .chain([("TESHI_AGENT_BACKEND".into(), "native".into())])
    .collect();
    let terminal = Terminal::launch(Launch {
        program: binary.to_string_lossy().into_owned(),
        args: vec![
            project.to_string_lossy().into_owned(),
            "--requirements-root".into(),
            root.join("requirements").to_string_lossy().into_owned(),
        ],
        cwd: Some(root.to_string_lossy().into_owned()),
        env,
        cols: 150,
        rows: 30,
    })
    .unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat", true, 10000)
        .unwrap();
    terminal
}

fn title(terminal: &Terminal) -> String {
    terminal
        .screen()
        .unwrap()
        .lines()
        .find(|line| line.contains("AI Chat") && line.contains('╭'))
        .expect("chat title row")
        .to_owned()
}

fn capture(terminal: &Terminal, name: &str) {
    if let Some(dir) = std::env::var_os("TESHI_CHAT_TITLE_ARTIFACTS") {
        let dir = std::path::PathBuf::from(dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{name}.txt")), terminal.screen().unwrap()).unwrap();
        terminal
            .screenshot(&dir.join(format!("{name}.svg")))
            .unwrap();
    }
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("sample.feature"), "Feature: Sample\n").unwrap();
    (root, project)
}

fn profile(project: &Path, id: &str, name: &str) {
    let dir = project.join(".teshi/agents").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("agent.yaml"), format!("id: {id}\nname: {name}\ndescription: test profile\nsystem_prompt: system.md\nmodel_ref: ''\n")).unwrap();
    fs::write(dir.join("system.md"), "Test profile.").unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn builtin_default_title_has_no_profile_suffix() {
    let (root, project) = fixture();
    let terminal = launch(&project, root.path());
    let line = title(&terminal);
    assert!(!line.contains("Agent:"), "{line}");
    capture(&terminal, "builtin-default");
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn custom_default_profile_is_identified_in_title() {
    let (root, project) = fixture();
    profile(&project, "custom-default", "Default");
    let terminal = launch(&project, root.path());
    assert!(title(&terminal).contains("Agent: Default"));
    capture(&terminal, "custom-default");
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn changing_custom_profile_updates_title() {
    let (root, project) = fixture();
    profile(&project, "alpha", "Alpha");
    profile(&project, "beta", "Beta");
    let terminal = launch(&project, root.path());
    let initial = title(&terminal);
    let target = if initial.contains("Agent: Alpha") {
        "Beta"
    } else {
        assert!(initial.contains("Agent: Beta"), "{initial}");
        "Alpha"
    };
    capture(&terminal, "custom-initial");
    terminal.key("Enter").unwrap();
    terminal.write("/agent-profile").unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("Agent Configuration", true, 10000)
        .unwrap();
    terminal.click_text(target, 10000).unwrap();
    terminal
        .expect_text(&format!("Name: {target}"), true, 10000)
        .unwrap();
    terminal.click_text("Apply [Enter]", 10000).unwrap();
    terminal
        .expect_text("Agent Configuration", false, 10000)
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        title(&terminal).contains(&format!("Agent: {target}")),
        "{}",
        terminal.screen().unwrap()
    );
    capture(&terminal, "custom-switched");
}
