//! Real PTY acceptance for project identity, picker input, and switch isolation.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use teshi_tui_runner::{Launch, Terminal};

fn launch_config(binary: &Path, project: &Path, store: &Path, app_data: &Path) -> Launch {
    fs::create_dir_all(app_data).unwrap();
    Launch {
        program: binary.to_string_lossy().into_owned(),
        args: vec![
            "--recursive".into(),
            project.to_string_lossy().into_owned(),
            "--requirements-root".into(),
            store.to_string_lossy().into_owned(),
        ],
        cwd: Some(app_data.to_string_lossy().into_owned()),
        env: [
            ("APPDATA", app_data.join("appdata")),
            ("LOCALAPPDATA", app_data.join("localdata")),
            ("XDG_CONFIG_HOME", app_data.join("config")),
            ("XDG_DATA_HOME", app_data.join("data")),
            ("TESHI_APP_DATA_DIR", app_data.to_path_buf()),
            ("TESHI_SESSION_DIR", app_data.join("sessions")),
            ("TESHI_ACP_STORE_DIR", app_data.join("acp")),
        ]
        .into_iter()
        .map(|(name, value)| (name.into(), value.to_string_lossy().into_owned()))
        .chain(std::iter::once((
            "TESHI_AGENT_BACKEND".into(),
            "native".into(),
        )))
        .collect(),
        cols: 120,
        rows: 35,
    }
}

fn write_project(root: &Path, name: &str, title: &str, runner_cmd: &str) -> PathBuf {
    let project = root.join(name);
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("sample.feature"),
        format!("Feature: {title}\n  Scenario: Ready\n    Given the {name} step\n"),
    )
    .unwrap();
    fs::write(
        project.join("teshi.toml"),
        format!("[runner]\ncmd = \"{runner_cmd}\"\n"),
    )
    .unwrap();
    project
}

fn binary() -> PathBuf {
    fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap()
}

fn write_slow(terminal: &Terminal, text: &str) {
    for character in text.chars() {
        terminal.write(&character.to_string()).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn keyboard_project_switch_reloads_project_feature_and_runner_config() {
    let binary = binary();
    let root = tempfile::tempdir().unwrap();
    let store = root.path().join("requirements");
    let app_data = root.path().join("app-data");
    let project_a = write_project(root.path(), "project-a", "Project A", "runner-a");
    let project_b = write_project(root.path(), "project-b", "Project B", "runner-b");
    let terminal = Terminal::launch(launch_config(&binary, &project_a, &store, &app_data)).unwrap();

    terminal
        .expect_text("Project: project-a", true, 15000)
        .unwrap();
    terminal.expect_text("project-a step", true, 15000).unwrap();
    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat!", true, 15000)
        .unwrap();
    terminal.key("Enter").unwrap();
    write_slow(&terminal, &format!("/project {}", project_b.display()));
    terminal.key("Enter").unwrap();

    terminal
        .expect_text("Project: project-b", true, 15000)
        .unwrap();
    terminal.key("Escape").unwrap();
    terminal.key("1").unwrap();
    terminal.expect_text("project-b step", true, 15000).unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 15000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn project_picker_pointer_and_open_path_switch_projects() {
    let binary = binary();
    let root = tempfile::tempdir().unwrap();
    let store = root.path().join("requirements");
    let app_data = root.path().join("app-data");
    let project_a = write_project(root.path(), "project-a", "Project A", "runner-a");
    let project_b = write_project(root.path(), "project-b", "Project B", "runner-b");
    fs::create_dir_all(&app_data).unwrap();
    fs::write(
        app_data.join("recent.json"),
        serde_json::to_vec_pretty(&json!({
            "paths": [],
            "projects": [{
                "root": project_b,
                "display_name": "project-b",
                "last_opened_ms": 1
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let terminal = Terminal::launch(launch_config(&binary, &project_a, &store, &app_data)).unwrap();
    terminal
        .expect_text("Project: project-a", true, 15000)
        .unwrap();

    terminal.key("3").unwrap();
    terminal
        .expect_text("Welcome to AI Chat!", true, 15000)
        .unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("/project").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("Open project", true, 10000).unwrap();
    terminal.click_text("project-b", 10000).unwrap();
    terminal
        .expect_text("Project: project-b", true, 15000)
        .unwrap();

    terminal.click_text("Project: project-b", 10000).unwrap();
    terminal.expect_text("Open project", true, 10000).unwrap();
    terminal.click_text("[O] Open path", 10000).unwrap();
    terminal
        .expect_text("Open project path", true, 10000)
        .unwrap();
    write_slow(&terminal, &project_a.to_string_lossy());
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("Project: project-a", true, 15000)
        .unwrap();
    terminal.key("Escape").unwrap();
    terminal.key("1").unwrap();
    terminal.expect_text("project-a step", true, 15000).unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 15000).unwrap();
}
