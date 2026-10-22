//! Real terminal pointer coverage for MindMap navigation and common pickers.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use teshi_tui_runner::{Launch, Terminal};

struct Fixture {
    root: tempfile::TempDir,
    launch: Launch,
    source: PathBuf,
    original: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("pointer-project");
        fs::create_dir(&project).unwrap();
        let source = project.join("navigation.feature");
        let mut text = String::from("Feature: Pointer navigation\n");
        for scenario in ["Location Alpha", "Location Beta"] {
            text.push_str(&format!("  Scenario: {scenario}\n    Given SHARED_ENTRY\n"));
            for number in 0..36 {
                text.push_str(&format!("    And PREVIEW_LINE_{number:02}\n"));
            }
            text.push_str(&format!("    Then {}_END\n", scenario.replace(' ', "_")));
        }
        fs::write(&source, &text).unwrap();

        let sessions = root.path().join("TESHI_SESSION_DIR");
        fs::create_dir(&sessions).unwrap();
        for (name, updated, content) in [
            (
                "Alpha",
                "9999999999999",
                (0..65)
                    .map(|index| format!("HISTORY_LINE_{index:02}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            ("Beta", "9999999999998", "SESSION_BETA_BODY".into()),
        ] {
            fs::write(
                sessions.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "id": name,
                    "created_at": name,
                    "updated_at": updated,
                    "model_label": format!("Session {name}"),
                    "message_count": 1,
                    "messages": [{
                        "role": "Assistant", "content": content, "source": null,
                        "tool_calls": null, "tool_call_id": null, "reasoning_content": null
                    }],
                    "project_root": project,
                    "backend_kind": "acp",
                    "acp_agent_id": null
                }))
                .unwrap(),
            )
            .unwrap();
        }

        let profiles = root.path().join("TESHI_APP_DATA_DIR/model-profiles");
        fs::create_dir_all(&profiles).unwrap();
        for name in ["Alpha", "Beta"] {
            fs::write(
                profiles.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "id": name, "name": format!("Pointer {name}"), "provider": "openai",
                    "model_id": format!("pointer-{name}"), "api_key": "",
                    "base_url": "http://127.0.0.1:1/v1"
                }))
                .unwrap(),
            )
            .unwrap();
        }
        fs::write(profiles.join("active"), "Alpha").unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/wrapped_chat_acp.py")
            .canonicalize()
            .unwrap();
        let mut env = [
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "TESHI_APP_DATA_DIR",
            "TESHI_SESSION_DIR",
            "TESHI_ACP_STORE_DIR",
        ]
        .into_iter()
        .map(|name| {
            (
                name.into(),
                root.path().join(name).to_string_lossy().into_owned(),
            )
        })
        .collect::<Vec<_>>();
        env.extend([
            ("TESHI_AGENT_BACKEND".into(), "acp".into()),
            (
                "TESHI_ACP_PROGRAM".into(),
                if cfg!(windows) { "python" } else { "python3" }.into(),
            ),
            (
                "TESHI_ACP_ARGS_JSON".into(),
                serde_json::to_string(&[fixture]).unwrap(),
            ),
            ("TESHI_LLM_API_KEY".into(), String::new()),
            ("NO_COLOR".into(), String::new()),
            ("TERM".into(), "xterm-256color".into()),
        ]);
        let launch = Launch {
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
            cwd: Some(root.path().to_string_lossy().into_owned()),
            env,
            cols: 140,
            rows: 40,
        };
        Self {
            root,
            launch,
            source,
            original: text.into_bytes(),
        }
    }

    fn launch(&self) -> Terminal {
        let terminal = Terminal::launch(self.launch.clone()).unwrap();
        terminal.expect_text("navigation", true, 15000).unwrap();
        terminal
    }

    fn unchanged(&self) {
        assert_eq!(fs::read(&self.source).unwrap(), self.original);
    }

    fn active_profile(&self) -> String {
        fs::read_to_string(
            self.root
                .path()
                .join("TESHI_APP_DATA_DIR/model-profiles/active"),
        )
        .unwrap()
    }
}

fn capture(terminal: &Terminal, name: &str) {
    std::thread::sleep(Duration::from_millis(100));
    let output = std::env::var_os("TESHI_NAVIGATION_ARTIFACTS")
        .map(PathBuf::from)
        .expect("set TESHI_NAVIGATION_ARTIFACTS to retain real terminal evidence");
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join(format!("{name}.txt")),
        terminal.screen().unwrap(),
    )
    .unwrap();
    terminal
        .screenshot(&output.join(format!("{name}.svg")))
        .unwrap();
}

fn command(terminal: &Terminal, command: &str) {
    terminal.write(command).unwrap();
    terminal.expect_text(command, true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text(&format!("│{command}"), false, 5000)
        .unwrap();
}

fn mindmap(terminal: &Terminal) {
    terminal.key("2").unwrap();
    terminal
        .expect_stable_text("╭MindMap", true, 5000, 100)
        .unwrap();
    terminal.click_text("▶ pointer-project", 5000).unwrap();
    terminal.expect_text("SHARED_ENTRY", true, 5000).unwrap();
    terminal.click_text("SHARED_ENTRY", 5000).unwrap();
    terminal
        .expect_text("Scenario: Location Alpha", true, 5000)
        .unwrap();
}

fn close(terminal: &Terminal) {
    terminal.key("Escape").unwrap();
    terminal.key("Escape").unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

fn locate(terminal: &Terminal, text: &str) -> (u16, u16) {
    terminal
        .screen()
        .unwrap()
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find(text)
                .map(|col| (line[..col].chars().count() as u16, row as u16))
        })
        .unwrap_or_else(|| panic!("{text:?} is not in the current viewport"))
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn mindmap_tree_click_preview_wheel_and_panel_toggle_keep_source() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    mindmap(&terminal);
    capture(&terminal, "mindmap-tree-selection");
    let (col, row) = locate(&terminal, "PREVIEW_LINE_03");
    for _ in 0..14 {
        terminal.scroll_at(col, row, false, false).unwrap();
    }
    capture(&terminal, "mindmap-preview-after-wheel");
    terminal
        .expect_text("PREVIEW_LINE_03", false, 1000)
        .unwrap();
    terminal.expect_text("PREVIEW_LINE_15", true, 1000).unwrap();
    terminal.expect_text("SHARED_ENTRY", true, 1000).unwrap();
    terminal.key("p").unwrap();
    terminal.expect_text("AI Chat", false, 5000).unwrap();
    capture(&terminal, "mindmap-panel-hidden");
    terminal.key("p").unwrap();
    terminal.expect_text("AI Chat", true, 5000).unwrap();
    capture(&terminal, "mindmap-panel-restored");
    fixture.unchanged();
    close(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn mindmap_location_dropdown_pointer_selects_same_location_as_keyboard() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    mindmap(&terminal);
    terminal.key("o").unwrap();
    terminal.expect_text("Open Scenario", true, 5000).unwrap();
    capture(&terminal, "mindmap-location-open");
    terminal
        .click_text("Scenario: Location Beta", 5000)
        .unwrap();
    capture(&terminal, "mindmap-location-pointer-selected");
    terminal.key("Enter").unwrap();
    terminal.expect_text("Open Scenario", false, 5000).unwrap();
    capture(&terminal, "mindmap-location-pointer-committed");
    terminal
        .expect_text("Scenario: Location Beta", true, 1000)
        .unwrap();
    terminal.key("[").unwrap();
    terminal
        .expect_text("Scenario: Location Alpha", true, 5000)
        .unwrap();
    fixture.unchanged();
    close(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn model_modal_blocks_background_tab_click_and_pointer_selects_profile() {
    let mut fixture = Fixture::new();
    fixture
        .launch
        .env
        .iter_mut()
        .find(|(name, _)| name == "TESHI_AGENT_BACKEND")
        .unwrap()
        .1 = "native".into();
    let terminal = fixture.launch();
    terminal.key("2").unwrap();
    terminal.expect_text("╭MindMap", true, 5000).unwrap();
    terminal.key("m").unwrap();
    terminal.expect_text("Model Profiles", true, 5000).unwrap();
    terminal.click_text("AI [3]", 5000).unwrap();
    terminal.scroll_at(2, 5, false, false).unwrap();
    capture(&terminal, "model-modal-background-click");
    terminal.key("Escape").unwrap();
    terminal.expect_text("Model Profiles", false, 5000).unwrap();
    capture(&terminal, "model-modal-after-close");
    terminal.expect_text("╭MindMap", true, 1000).unwrap();

    terminal.key("m").unwrap();
    terminal.expect_text("Pointer Beta", true, 5000).unwrap();
    terminal.click_text("Pointer Beta", 5000).unwrap();
    capture(&terminal, "model-profile-pointer-selected");
    terminal.key("Enter").unwrap();
    terminal.expect_text("Model Profiles", false, 5000).unwrap();
    assert_eq!(fixture.active_profile().trim(), "Beta");
    capture(&terminal, "model-profile-pointer-activated");
    fixture.unchanged();
    close(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn session_modal_blocks_background_pointer_and_selects_target_history() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    terminal.key("3").unwrap();
    terminal.expect_text("AI Chat", true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    command(&terminal, "/resume");
    terminal.expect_text("HISTORY_LINE_64", true, 5000).unwrap();
    command(&terminal, "/sessions");
    terminal.expect_text("Load [Enter]", true, 5000).unwrap();
    terminal.click_text("Requirements [4]", 5000).unwrap();
    for _ in 0..12 {
        terminal.scroll_at(120, 4, true, false).unwrap();
    }
    capture(&terminal, "session-modal-background-pointer");
    terminal.key("Escape").unwrap();
    terminal.expect_text("Load [Enter]", false, 5000).unwrap();
    capture(&terminal, "session-modal-after-close");
    terminal.expect_text("HISTORY_LINE_64", true, 1000).unwrap();
    command(&terminal, "/sessions");
    terminal
        .expect_text("Model: Session Beta", true, 5000)
        .unwrap();
    terminal.click_text("Model: Session Beta", 5000).unwrap();
    capture(&terminal, "session-pointer-selected-beta");
    terminal.key("Enter").unwrap();
    terminal.expect_text("Load [Enter]", false, 5000).unwrap();
    terminal
        .expect_text("SESSION_BETA_BODY", true, 5000)
        .unwrap();
    capture(&terminal, "session-pointer-loaded-beta");
    fixture.unchanged();
    close(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn model_profile_pointer_row_selects_same_profile_as_keyboard() {
    let mut fixture = Fixture::new();
    fixture
        .launch
        .env
        .iter_mut()
        .find(|(name, _)| name == "TESHI_AGENT_BACKEND")
        .unwrap()
        .1 = "native".into();
    let terminal = fixture.launch();
    terminal.resize(55, 16).unwrap();
    terminal.key("m").unwrap();
    terminal.expect_text("Pointer Beta", true, 5000).unwrap();
    terminal.click_text("Pointer Beta", 5000).unwrap();
    capture(&terminal, "model-row-pointer-selected");
    terminal.key("Enter").unwrap();
    terminal.expect_text("Model Profiles", false, 5000).unwrap();
    capture(&terminal, "model-row-pointer-activated");
    assert_eq!(fixture.active_profile().trim(), "Beta");
    fixture.unchanged();
    close(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn session_pointer_row_selects_target_history() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    terminal.resize(55, 16).unwrap();
    terminal.key("3").unwrap();
    terminal.expect_text("AI Chat", true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    command(&terminal, "/sessions");
    terminal
        .expect_text("Model: Session Beta", true, 5000)
        .unwrap();
    terminal.click_text("Model: Session Beta", 5000).unwrap();
    capture(&terminal, "session-row-selected-beta");
    terminal.key("Enter").unwrap();
    terminal.expect_text("Load [Enter]", false, 5000).unwrap();
    capture(&terminal, "session-row-loaded-beta");
    terminal
        .expect_text("SESSION_BETA_BODY", true, 1000)
        .unwrap();
    fixture.unchanged();
    close(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn mindmap_tree_pointer_takes_keyboard_focus_from_ai_pane() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    mindmap(&terminal);
    terminal.key("Enter").unwrap();
    terminal
        .expect_text("Return Tree [Esc]", true, 5000)
        .unwrap();
    terminal.click_text("SHARED_ENTRY", 5000).unwrap();
    capture(&terminal, "mindmap-tree-click-from-ai-focus");
    terminal.key("p").unwrap();
    capture(&terminal, "mindmap-tree-focus-panel-toggle");
    terminal.expect_text("AI Chat", false, 1000).unwrap();
    fixture.unchanged();
    close(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn approval_modal_owns_background_pointer_and_its_option_rows() {
    let mut fixture = Fixture::new();
    fixture
        .launch
        .env
        .iter_mut()
        .find(|(name, _)| name == "TESHI_AGENT_BACKEND")
        .unwrap()
        .1 = "native".into();
    let terminal = fixture.launch();
    terminal.key("3").unwrap();
    terminal.expect_text("AI Chat", true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    command(&terminal, "/approval");
    terminal.expect_text("Approval Mode", true, 5000).unwrap();
    terminal.click_text("Requirements [4]", 5000).unwrap();
    terminal.scroll_at(120, 4, false, false).unwrap();
    capture(&terminal, "approval-modal-background-pointer");
    terminal.key("Escape").unwrap();
    terminal.expect_text("Approval Mode", false, 5000).unwrap();
    capture(&terminal, "approval-modal-after-close");
    terminal.expect_text("╭ Agents", true, 1000).unwrap();
    command(&terminal, "/approval");
    terminal.expect_text("Approval Mode", true, 5000).unwrap();
    terminal.click_text("Auto", 5000).unwrap();
    terminal.expect_text("Approval Mode", false, 5000).unwrap();
    terminal.expect_text(" Auto ", true, 5000).unwrap();
    capture(&terminal, "approval-pointer-auto-selected");
    fixture.unchanged();
    close(&terminal);
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn ai_input_pointer_click_takes_text_focus() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    terminal.key("3").unwrap();
    terminal.expect_text("AI Chat", true, 5000).unwrap();
    terminal.expect_text("Focus [i/Enter]", true, 5000).unwrap();
    let (col, row) = locate(&terminal, "╭────────────────");
    terminal
        .write(&format!(
            "\x1b[<0;{};{}M\x1b[<0;{};{}m",
            col + 5,
            row + 2,
            col + 5,
            row + 2
        ))
        .unwrap();
    capture(&terminal, "ai-input-pointer-focus");
    terminal.expect_text("Send [Enter]", true, 1000).unwrap();
    terminal.write("POINTER_INPUT_TEXT").unwrap();
    terminal
        .expect_text("POINTER_INPUT_TEXT", true, 5000)
        .unwrap();
    capture(&terminal, "ai-input-pointer-text");
    fixture.unchanged();
    close(&terminal);
}
