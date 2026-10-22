//! Rendered keyboard, pointer, wheel, drag, and resize acceptance for the TUI.

#[path = "support/requirement_project.rs"]
mod requirement_project;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use teshi_core::authoring::{DocumentRevision, RequirementDocumentIndex, RequirementDocumentMeta};
use teshi_tui_runner::{Launch, Terminal};

struct Fixture {
    _root: tempfile::TempDir,
    config: Launch,
    source: PathBuf,
    source_before: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let store = root.path().join("requirements");
        fs::create_dir(&project).unwrap();
        fs::create_dir(&store).unwrap();
        let app_data = root.path().join("TESHI_APP_DATA_DIR");
        fs::create_dir(&app_data).unwrap();
        fs::write(
            app_data.join("recent.json"),
            serde_json::to_vec(&serde_json::json!({
                "paths": [],
                "projects": [{"root": project, "display_name": "project", "last_opened_ms": 1}]
            }))
            .unwrap(),
        )
        .unwrap();
        let source = project.join("sample.feature");
        let text = format!("Feature: Pointer acceptance\n{}", (1..=30).map(|number| format!("  Scenario: scenario {number:02} with a deliberately long descriptive title for horizontal overflow\n    Given step {number:02}\n")).collect::<String>());
        fs::write(&source, &text).unwrap();
        let mut index = RequirementDocumentIndex::initialize_empty_store();
        for number in 1..=35 {
            let title =
                format!("Req {number:02} extended Chinese 中文 title for horizontal scrolling");
            let body = format!("# {title}\n{}", (1..=55).map(|line| format!("body {line:02} 甲乙🙂 some wrapped descriptive content for requirement {number:02}\n")).collect::<String>());
            let path = format!("doc-{number:02}.md");
            fs::write(store.join(&path), body).unwrap();
            index.documents.push(RequirementDocumentMeta::new(
                format!("doc-{number:02}"),
                path,
                title,
                DocumentRevision::new(format!("fixture-{number}")),
            ));
        }
        requirement_project::associate(&project, &mut index);
        fs::write(
            store.join("_teshi.json"),
            serde_json::to_vec_pretty(&index).unwrap(),
        )
        .unwrap();
        let binary = fs::canonicalize(std::env::var("TESHI_SELF_TEST_BIN").unwrap()).unwrap();
        let config = Launch {
            program: binary.to_string_lossy().into_owned(),
            args: vec![
                project.to_string_lossy().into_owned(),
                "--requirements-root".into(),
                store.to_string_lossy().into_owned(),
            ],
            cwd: Some(root.path().to_string_lossy().into_owned()),
            env: [
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
            .collect(),
            cols: 150,
            rows: 30,
        };
        Self {
            _root: root,
            config,
            source,
            source_before: text.into_bytes(),
        }
    }

    fn launch(&self) -> Terminal {
        let terminal = Terminal::launch(self.config.clone()).unwrap();
        terminal.expect_text("sample", true, 10000).unwrap();
        terminal
    }

    fn unchanged(&self) {
        assert_eq!(fs::read(&self.source).unwrap(), self.source_before);
    }
}

fn capture(terminal: &Terminal, name: &str) {
    std::thread::sleep(Duration::from_millis(100));
    if let Some(directory) = std::env::var_os("TESHI_P4_P5_ARTIFACTS") {
        let directory = PathBuf::from(directory).join("interaction");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(format!("{name}.txt")),
            terminal.screen().unwrap(),
        )
        .unwrap();
        terminal
            .screenshot(&directory.join(format!("{name}.svg")))
            .unwrap();
    }
}

fn prefix(terminal: &Terminal, key: &str) {
    terminal.key("Ctrl+w").unwrap();
    terminal.key(key).unwrap();
}

fn tree_contains(screen: &str, text: &str) -> bool {
    screen.lines().any(|line| {
        line.split('│')
            .nth(1)
            .is_some_and(|pane| pane.contains(text))
    })
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn vim_boundaries_pages_and_arrow_pane_chords_work_across_tabs() {
    let mut fixture = Fixture::new();
    fixture
        .config
        .env
        .push(("TESHI_AGENT_BACKEND".into(), "native".into()));
    fixture
        .config
        .env
        .push(("TESHI_LLM_API_KEY".into(), String::new()));
    let terminal = fixture.launch();
    prefix(&terminal, "Right");
    terminal.key("G").unwrap();
    terminal.expect_text("Given step 30", true, 3000).unwrap();
    terminal.write("gg").unwrap();
    terminal.expect_text("Given step 01", true, 3000).unwrap();
    terminal.key("PageDown").unwrap();
    terminal.expect_text("Given step 11", true, 3000).unwrap();
    terminal.key("Ctrl+b").unwrap();
    terminal.expect_text("Given step 01", true, 3000).unwrap();
    capture(&terminal, "vim-explore-pages");

    terminal.key("2").unwrap();
    terminal.expect_text("Preview", true, 3000).unwrap();
    prefix(&terminal, "Right");
    terminal.key("i").unwrap();
    terminal.write("hjkl q12345").unwrap();
    terminal.expect_text("hjkl q12345", true, 3000).unwrap();
    capture(&terminal, "vim-mindmap-input");
    terminal.key("Escape").unwrap();
    prefix(&terminal, "Left");

    terminal.key("4").unwrap();
    terminal.expect_text("# Req 01", true, 3000).unwrap();
    terminal.key("End").unwrap();
    terminal.expect_text("# Req 35", true, 3000).unwrap();
    terminal.key("PageUp").unwrap();
    terminal.expect_text("# Req 25", true, 3000).unwrap();
    prefix(&terminal, "Right");
    terminal.key("G").unwrap();
    terminal.expect_text("body 55", true, 3000).unwrap();
    terminal.write("gg").unwrap();
    terminal.expect_text("# Req 25", true, 3000).unwrap();
    capture(&terminal, "vim-requirement-boundaries");

    terminal.key("3").unwrap();
    terminal.key("i").unwrap();
    terminal.expect_text("[INPUT]", true, 3000).unwrap();
    terminal.write("hjkl q12345").unwrap();
    terminal.expect_text("hjkl q12345", true, 3000).unwrap();
    terminal.key("Escape").unwrap();
    terminal.expect_text("[NORMAL]", true, 3000).unwrap();
    capture(&terminal, "vim-ai-modes");
    fixture.unchanged();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn five_tabs_render_at_normal_and_narrow_sizes_and_modal_owns_pointer() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    for (tab, label, body) in [
        ("1", "Explore [1]", "Scenario"),
        ("2", "MindMap [2]", "Preview"),
        ("3", "AI [3]", "AI Chat"),
        ("4", "Requirements [4]", "# Req 01"),
        ("5", "Test Points [5]", "Intent & review"),
    ] {
        terminal.resize(150, 30).unwrap();
        terminal.click_text(label, 10000).unwrap();
        terminal.expect_text(body, true, 10000).unwrap();
        prefix(&terminal, "l");
        prefix(&terminal, "h");
        capture(&terminal, &format!("tab-{tab}-normal"));
        terminal.resize(55, 16).unwrap();
        let compact_label = match tab {
            "1" => "Explore [1]",
            "2" => "Map [2]",
            "3" => "AI [3]",
            "4" => "Req [4]",
            _ => "Points [5]",
        };
        terminal.click_text(compact_label, 10000).unwrap();
        capture(&terminal, &format!("tab-{tab}-narrow"));
        terminal.expect_text(body, true, 10000).unwrap();
    }
    terminal.resize(150, 30).unwrap();
    terminal.key("1").unwrap();
    terminal.expect_text("Scenarios (30)", true, 10000).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    terminal.click_text("Project: project", 10000).unwrap();
    terminal.expect_text("Open path", true, 10000).unwrap();
    terminal.click_text("Requirements [4]", 10000).unwrap();
    terminal.scroll_at(2, 6, false, false).unwrap();
    terminal.expect_text("Open path", true, 1000).unwrap();
    capture(&terminal, "project-modal-pointer-owned");
    terminal.key("Escape").unwrap();
    terminal.expect_text("Open path", false, 10000).unwrap();
    terminal.expect_text("Scenario", true, 1000).unwrap();
    fixture.unchanged();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn project_header_pointer_opens_picker_on_explore() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    terminal.click_text("Project: project", 10000).unwrap();
    capture(&terminal, "explore-project-header-click");
    terminal.expect_text("Open path", true, 1000).unwrap();
    terminal.key("Escape").unwrap();
    fixture.unchanged();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn pane_prefix_cancel_keeps_mindmap_preview_visible() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    terminal.key("2").unwrap();
    terminal.expect_text("Preview", true, 10000).unwrap();
    prefix(&terminal, "Escape");
    capture(&terminal, "mindmap-prefix-cancel");
    terminal.expect_text("Preview", true, 1000).unwrap();
    fixture.unchanged();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn requirements_pointer_wheel_scrollbar_and_resize_keep_independent_panes() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    terminal.key("4").unwrap();
    terminal.expect_text("# Req 01", true, 10000).unwrap();
    capture(&terminal, "requirements-initial");
    for _ in 0..9 {
        terminal.scroll_at(2, 5, false, false).unwrap();
    }
    std::thread::sleep(Duration::from_millis(150));
    let after_tree = terminal.screen().unwrap();
    capture(&terminal, "requirements-tree-wheel");
    assert!(
        after_tree.contains("# Req 01"),
        "tree wheel replaced editor: {after_tree}"
    );
    assert!(
        !tree_contains(&after_tree, "Req 01"),
        "tree did not scroll: {after_tree}"
    );
    terminal.scroll_at(2, 5, false, true).unwrap();
    capture(&terminal, "requirements-tree-horizontal-wheel");
    let horizontal = terminal.screen().unwrap();
    assert_ne!(
        after_tree, horizontal,
        "Shift+wheel did not scroll tree horizontally"
    );
    terminal.scroll_at(45, 5, false, false).unwrap();
    capture(&terminal, "requirements-editor-wheel");
    terminal.expect_text("# Req 01", false, 1000).unwrap();

    // SGR press, motion with the left button held, and release on the current
    // tree border. These are PTY mouse events, rather than direct handler calls.
    terminal.scroll_at(2, 5, true, true).unwrap();
    terminal.write("\x1b[<0;30;4M").unwrap();
    capture(&terminal, "requirements-tree-scrollbar-press-top");
    assert!(
        tree_contains(&terminal.screen().unwrap(), "Req 01"),
        "scrollbar press did not move to the top"
    );
    terminal.write("\x1b[<32;30;28M\x1b[<0;30;28m").unwrap();
    capture(&terminal, "requirements-tree-scrollbar-drag");
    let dragged = terminal.screen().unwrap();
    assert!(
        tree_contains(&dragged, "Req 35"),
        "scrollbar drag did not reach the end: {dragged}"
    );
    assert!(
        !tree_contains(&dragged, "Req 01"),
        "scrollbar drag did not leave the top: {dragged}"
    );
    terminal.expect_text("body 04", true, 1000).unwrap();
    terminal.resize(55, 16).unwrap();
    capture(&terminal, "requirements-after-resize");
    terminal.write("\x1b[<0;20;5M\x1b[<0;20;5m").unwrap();
    terminal.key("i").unwrap();
    terminal.expect_text("[INSERT]", true, 1000).unwrap();
    terminal.key("Escape").unwrap();
    capture(&terminal, "requirements-narrow-pointer-editor");
    fixture.unchanged();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn explore_pointer_selection_and_scenario_wheel_leave_feature_and_step_independent() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    terminal.click_text("scenario 02", 10000).unwrap();
    terminal.expect_text("Given step 02", true, 10000).unwrap();
    capture(&terminal, "explore-pointer-selected-scenario");
    for _ in 0..9 {
        terminal.scroll_at(40, 5, false, false).unwrap();
    }
    capture(&terminal, "explore-scenario-wheel");
    let scrolled = terminal.screen().unwrap();
    assert!(
        scrolled.contains("Given step 02"),
        "scenario wheel changed the selected step: {scrolled}"
    );
    assert!(
        scrolled.contains("sample"),
        "scenario wheel changed the selected feature: {scrolled}"
    );
    assert!(
        scrolled.contains("scenario 30"),
        "scenario wheel did not reach the bottom: {scrolled}"
    );
    terminal.click_text("scenario 30", 10000).unwrap();
    terminal.expect_text("Given step 30", true, 10000).unwrap();
    terminal.scroll_at(40, 5, false, true).unwrap();
    capture(&terminal, "explore-scenario-horizontal-wheel");
    terminal.expect_text("Given step 30", true, 1000).unwrap();
    fixture.unchanged();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}
