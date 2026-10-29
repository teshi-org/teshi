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
                "--recursive".into(),
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
    // The launch Feature is parsed in the background; navigation needs its steps.
    terminal.expect_text("scenario 01", true, 10000).unwrap();
    prefix(&terminal, "Right");
    terminal.key("G").unwrap();
    terminal.expect_text("Given step 30", true, 10000).unwrap();
    terminal.write("gg").unwrap();
    terminal.expect_text("Given step 01", true, 10000).unwrap();
    terminal.key("PageDown").unwrap();
    terminal.expect_text("Given step 11", true, 10000).unwrap();
    terminal.key("Ctrl+b").unwrap();
    terminal.expect_text("Given step 01", true, 10000).unwrap();
    capture(&terminal, "vim-explore-pages");

    terminal.key("2").unwrap();
    terminal.expect_text("Preview", true, 10000).unwrap();
    prefix(&terminal, "Right");
    terminal.key("i").unwrap();
    terminal.write("hjkl q12345").unwrap();
    terminal.expect_text("hjkl q12345", true, 10000).unwrap();
    capture(&terminal, "vim-mindmap-input");
    terminal.key("Escape").unwrap();
    terminal.expect_text("[INPUT]", false, 10000).unwrap();
    prefix(&terminal, "Left");

    terminal.key("4").unwrap();
    terminal.expect_text("# Req 01", true, 10000).unwrap();
    terminal.key("End").unwrap();
    terminal.expect_text("# Req 35", true, 10000).unwrap();
    terminal.key("PageUp").unwrap();
    terminal.expect_text("# Req 25", true, 10000).unwrap();
    prefix(&terminal, "Right");
    terminal.key("G").unwrap();
    terminal.expect_text("body 55", true, 10000).unwrap();
    terminal.write("gg").unwrap();
    terminal.expect_text("# Req 25", true, 10000).unwrap();
    capture(&terminal, "vim-requirement-boundaries");

    terminal.key("3").unwrap();
    terminal.key("i").unwrap();
    terminal.expect_text("[INPUT]", true, 10000).unwrap();
    terminal.write("hjkl q12345").unwrap();
    terminal.expect_text("hjkl q12345", true, 10000).unwrap();
    terminal.key("Escape").unwrap();
    terminal.expect_text("[NORMAL]", true, 10000).unwrap();
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
        let focused_body = match tab {
            "1" => "sample",
            "4" => "Req 01",
            "5" => "Test points (",
            _ => body,
        };
        terminal.expect_text(focused_body, true, 10000).unwrap();
        if tab == "1" {
            prefix(&terminal, "Right");
            terminal.expect_text("Scenarios (30)", true, 3000).unwrap();
            prefix(&terminal, "Left");
        }
        if tab == "4" {
            prefix(&terminal, "Right");
            terminal.expect_text("# Req 01", true, 3000).unwrap();
            prefix(&terminal, "Left");
        }
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
    terminal.expect_text("Open path", false, 3000).unwrap();
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
    let tree_border = terminal
        .screen()
        .unwrap()
        .lines()
        .nth(2)
        .unwrap()
        .chars()
        .position(|ch| ch == '╮')
        .unwrap()
        + 1;
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
    terminal
        .write(&format!("\x1b[<0;{tree_border};4M"))
        .unwrap();
    capture(&terminal, "requirements-tree-scrollbar-press-top");
    assert!(
        tree_contains(&terminal.screen().unwrap(), "Req 01"),
        "scrollbar press did not move to the top"
    );
    terminal
        .write(&format!(
            "\x1b[<32;{tree_border};28M\x1b[<0;{tree_border};28m"
        ))
        .unwrap();
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
    prefix(&terminal, "Right");
    terminal.expect_text("body 04", true, 3000).unwrap();
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
    // One notch per event: a loaded runner may coalesce bursts, so keep
    // scrolling until the list reaches its last scenario.
    let wheel_deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let visible = terminal.screen().unwrap();
        if visible.contains("scenario 30") {
            break;
        }
        assert!(
            std::time::Instant::now() < wheel_deadline,
            "scenario wheel did not reach the bottom: {visible}"
        );
        terminal.scroll_at(45, 5, false, false).unwrap();
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
    terminal.scroll_at(45, 5, false, true).unwrap();
    capture(&terminal, "explore-scenario-horizontal-wheel");
    terminal.expect_text("Given step 30", true, 1000).unwrap();
    fixture.unchanged();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn external_feature_additions_are_discovered_in_recursive_project() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    let project = fixture.source.parent().unwrap();
    let nested = project.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(
        project.join("added_root.feature"),
        "Feature: Added root\n  Scenario: root\n    Given root\n",
    )
    .unwrap();
    fs::write(
        nested.join("added_nested.feature"),
        "Feature: Added nested\n  Scenario: nested\n    Given nested\n",
    )
    .unwrap();
    terminal.expect_text("added_root", true, 10000).unwrap();
    // Directory rows start collapsed, so the recursive scope is verified through
    // file search rather than by expanding every discovered directory.
    terminal.key("/").unwrap();
    terminal.write("added_nested").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("added_nested", true, 10000).unwrap();
    capture(&terminal, "external-additions");
    fixture.unchanged();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn external_clean_changes_and_deletion_refresh_without_reopening() {
    let fixture = Fixture::new();
    let removed = fixture.source.parent().unwrap().join("remove_me.feature");
    let other = fixture.source.parent().unwrap().join("z_other.feature");
    fs::write(
        &removed,
        "Feature: Remove me\n  Scenario: old\n    Given old\n",
    )
    .unwrap();
    fs::write(
        &other,
        "Feature: Other\n  Scenario: old other\n    Given old\n",
    )
    .unwrap();
    let terminal = fixture.launch();
    terminal.expect_text("remove_me", true, 10000).unwrap();
    fs::write(
        &fixture.source,
        "Feature: Pointer acceptance\n  Scenario: updated externally\n    Given updated\n",
    )
    .unwrap();
    fs::write(
        &other,
        "Feature: Other\n  Scenario: updated second\n    Given updated\n",
    )
    .unwrap();
    fs::remove_file(&removed).unwrap();
    terminal
        .expect_text("updated externally", true, 10000)
        .unwrap();
    terminal
        .expect_stable_text("remove_me", false, 10000, 500)
        .unwrap();
    capture(&terminal, "external-changes-and-deletion");
    terminal.click_text("z_other", 10000).unwrap();
    terminal.expect_text("updated second", true, 10000).unwrap();
    capture(&terminal, "external-second-change");
    assert!(!removed.exists());
    assert!(
        fs::read_to_string(&fixture.source)
            .unwrap()
            .contains("updated externally")
    );
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn externally_deleted_dirty_feature_can_keep_local_edits() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    terminal.click_text("Edit", 10000).unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("LOCAL_DELETE_SENTINEL").unwrap();
    terminal.key("Enter").unwrap();
    terminal.expect_text("Edit committed", true, 10000).unwrap();
    fs::remove_file(&fixture.source).unwrap();
    terminal
        .expect_text("Feature changed on disk", true, 10000)
        .unwrap();
    capture(&terminal, "external-delete-dirty-prompt");
    terminal.click_text("Keep", 10000).unwrap();
    terminal
        .expect_text("Feature changed on disk", false, 5000)
        .unwrap();
    terminal
        .expect_text("LOCAL_DELETE_SENTINEL", true, 5000)
        .unwrap();
    assert!(!fixture.source.exists());
    capture(&terminal, "external-delete-kept-local");
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn external_unreadable_feature_reports_error_and_preserves_project() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    let project = fixture.source.parent().unwrap();
    // Invalid UTF-8 is a deterministic read failure on Windows without
    // changing ACLs or moving an open project directory.
    let invalid = project.join("unreadable.feature");
    fs::write(&invalid, [0xff, 0xfe]).unwrap();
    // Discovery only collects paths, so the refresh still succeeds and lists the
    // unreadable file; the read error surfaces when it is selected.
    terminal.key("F6").unwrap();
    terminal.expect_text("unreadable", true, 10000).unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    terminal.click_text("unreadable", 10000).unwrap();
    terminal.expect_text("stream did not", true, 10000).unwrap();
    terminal.expect_text("sample", true, 10000).unwrap();
    capture(&terminal, "external-scan-failure");
    fs::remove_file(invalid).unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI and Python; set TESHI_SELF_TEST_BIN"]
fn external_changes_wait_until_real_feature_run_finishes() {
    let fixture = Fixture::new();
    let runner = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/external_sync_runner.py")
        .canonicalize()
        .unwrap();
    let evidence = fixture._root.path().join("runner-evidence");
    fs::create_dir(&evidence).unwrap();
    let python = if cfg!(windows) { "python" } else { "python3" };
    let args =
        serde_json::to_string(&[runner.to_string_lossy(), evidence.to_string_lossy()]).unwrap();
    fs::write(
        fixture.source.parent().unwrap().join("teshi.toml"),
        format!("[runner]\ncmd = \"{python}\"\nargs = {args}\n"),
    )
    .unwrap();
    let terminal = fixture.launch();
    terminal.key("r").unwrap();
    let ready = evidence.join("runner-ready");
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !ready.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "runner did not start: {}",
            terminal.screen().unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let request: serde_json::Value =
        serde_json::from_slice(&fs::read(evidence.join("run-request.json")).unwrap()).unwrap();
    let cases = request["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 30);
    assert!(
        cases[0]["scenario"]
            .as_str()
            .unwrap()
            .contains("scenario 01"),
        "{request}"
    );
    assert!(
        cases[29]["scenario"]
            .as_str()
            .unwrap()
            .contains("scenario 30"),
        "{request}"
    );
    fs::write(
        &fixture.source,
        "Feature: Pointer acceptance\n  Scenario: changed after run\n    Given new\n",
    )
    .unwrap();
    terminal.key("F6").unwrap();
    terminal
        .expect_stable_text("scenario 01", true, 5000, 1500)
        .unwrap();
    assert!(!terminal.screen().unwrap().contains("changed after run"));
    capture(&terminal, "external-run-deferred");
    fs::write(evidence.join("runner-release"), "finish").unwrap();
    terminal
        .expect_text("changed after run", true, 10000)
        .unwrap();
    capture(&terminal, "external-run-resumed");
    if let Some(dir) = std::env::var_os("TESHI_P4_P5_ARTIFACTS") {
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir).unwrap();
        fs::copy(
            evidence.join("run-request.json"),
            dir.join("run-request.json"),
        )
        .unwrap();
    }
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn pointer_hover_changes_visible_action_style_and_clears_on_exit() {
    let mut fixture = Fixture::new();
    fixture.config.env.extend([
        ("NO_COLOR".into(), String::new()),
        ("TERM".into(), "xterm-256color".into()),
        ("COLORTERM".into(), "truecolor".into()),
    ]);
    let terminal = fixture.launch();
    terminal.key("4").unwrap();
    terminal.expect_text("Save [Ctrl+s]", true, 10000).unwrap();
    let screen = terminal.screen().unwrap();
    let (row, col) = screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find("Save [Ctrl+s]")
                .map(|byte| (row + 1, line[..byte].chars().count() + 1))
        })
        .unwrap();
    let base = fixture._root.path().join("base.svg");
    let hover = fixture._root.path().join("hover.svg");
    let after = fixture._root.path().join("after.svg");
    terminal.screenshot(&base).unwrap();
    capture(&terminal, "footer-hover-base");
    terminal.write(&format!("\x1b[<35;{col};{row}M")).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    terminal.screenshot(&hover).unwrap();
    capture(&terminal, "footer-hover");
    terminal.write("\x1b[<35;149;15M").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    terminal.screenshot(&after).unwrap();
    capture(&terminal, "footer-hover-exit");
    assert!(
        fs::read(&base).unwrap() != fs::read(&hover).unwrap(),
        "hover did not change styled viewport"
    );
    assert!(
        fs::read(&base).unwrap() == fs::read(&after).unwrap(),
        "hover style did not clear"
    );
    fixture.unchanged();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn explore_long_step_horizontal_scroll_and_resize_stays_bounded() {
    let mut fixture = Fixture::new();
    fixture.config.env.extend([
        ("NO_COLOR".into(), String::new()),
        ("TERM".into(), "xterm-256color".into()),
        ("COLORTERM".into(), "truecolor".into()),
    ]);
    let body = format!(
        "Feature: Long step\n  Scenario: long step\n    Given START {} END_MARKER\n",
        "abcdefghij ".repeat(24)
    );
    fs::write(&fixture.source, &body).unwrap();
    let terminal = fixture.launch();
    terminal.expect_text("START", true, 10000).unwrap();
    prefix(&terminal, "Right");
    prefix(&terminal, "Right");
    assert!(!terminal.screen().unwrap().contains("END_MARKER"));
    let focus_path = fixture._root.path().join("focus.svg");
    let focused_border = |svg: &str| {
        let segments: Vec<_> = svg.split("<text ").collect();
        let title = segments
            .iter()
            .position(|part| part.contains(">Steps</text>"))
            .unwrap();
        segments[title - 1].contains("fill=\"#af87ff\"")
            && segments[title - 1].contains(">╭</text>")
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        terminal.screenshot(&focus_path).unwrap();
        if focused_border(&fs::read_to_string(&focus_path).unwrap()) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Step pane was not focused"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    capture(&terminal, "horizontal-start");
    for _ in 0..90 {
        terminal.key("Shift+Right").unwrap();
    }
    terminal.expect_text("END_MARKER", true, 10000).unwrap();
    terminal.screenshot(&focus_path).unwrap();
    assert!(
        focused_border(&fs::read_to_string(&focus_path).unwrap()),
        "scrolling changed Step pane focus"
    );
    capture(&terminal, "horizontal-end");
    let at_end = terminal.screen().unwrap();
    assert!(
        at_end
            .lines()
            .any(|line| line.contains('━') && line.trim_end().ends_with("━│")),
        "scrollbar is not at right edge: {at_end}"
    );
    for _ in 0..12 {
        terminal.key("Shift+Right").unwrap();
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        terminal.screen().unwrap(),
        at_end,
        "scroll moved beyond content"
    );
    terminal.resize(180, 30).unwrap();
    terminal.expect_text("END_MARKER", true, 10000).unwrap();
    capture(&terminal, "horizontal-widened");
    let widened = terminal.screen().unwrap();
    assert!(
        widened
            .lines()
            .any(|line| line.contains('━') && line.trim_end().ends_with("━│")),
        "widened scrollbar is not clamped: {widened}"
    );
    assert_eq!(fs::read_to_string(&fixture.source).unwrap(), body);
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn normal_tabs_place_navigation_left_and_details_vertically_right() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    for (tab, nav, upper, lower) in [
        ("1", "Features", "Scenarios (30)", "Steps"),
        (
            "4",
            "Requirements ·",
            "Req 01 extended",
            "Linked test points",
        ),
        (
            "5",
            "Test points (all)",
            "Intent & review",
            "Requirement excerpts",
        ),
    ] {
        terminal.key(tab).unwrap();
        terminal.expect_text(lower, true, 10000).unwrap();
        let screen = terminal.screen().unwrap();
        let position = |needle: &str| {
            screen
                .lines()
                .enumerate()
                .find_map(|(y, line)| {
                    line.find(needle)
                        .map(|byte| (y, unicode_width::UnicodeWidthStr::width(&line[..byte])))
                })
                .unwrap_or_else(|| panic!("missing {needle}: {screen}"))
        };
        let (nav_y, nav_x) = position(nav);
        let (upper_y, upper_x) = position(upper);
        let (lower_y, lower_x) = position(lower);
        assert_eq!(
            nav_y, upper_y,
            "navigation and upper pane differ in top alignment: {screen}"
        );
        assert!(
            nav_x < upper_x && upper_y < lower_y,
            "not two-column stacked details: {screen}"
        );
        assert_eq!(
            upper_x, lower_x,
            "detail panes do not share a column: {screen}"
        );
        capture(&terminal, &format!("layout-normal-tab-{tab}"));
    }
    fixture.unchanged();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn narrow_requirements_panes_remain_reachable_with_selection_and_edit() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    terminal.resize(55, 16).unwrap();
    terminal.key("4").unwrap();
    terminal.expect_text("Req 01", true, 10000).unwrap();
    terminal.key("j").unwrap();
    prefix(&terminal, "Right");
    terminal.expect_text("# Req 02", true, 10000).unwrap();
    terminal.key("i").unwrap();
    terminal.write("LOCAL_DRAFT").unwrap();
    terminal.key("Escape").unwrap();
    terminal.expect_text("LOCAL_DRAFT", true, 10000).unwrap();
    capture(&terminal, "compact-editor-draft");
    prefix(&terminal, "Right");
    terminal
        .expect_text("Linked test points", true, 10000)
        .unwrap();
    capture(&terminal, "compact-linked-pane");
    prefix(&terminal, "Left");
    terminal.expect_text("LOCAL_DRAFT", true, 10000).unwrap();
    prefix(&terminal, "Left");
    terminal.expect_text("Req 02", true, 10000).unwrap();
    capture(&terminal, "compact-tree-return");
    prefix(&terminal, "Right");
    terminal.expect_text("LOCAL_DRAFT", true, 10000).unwrap();
    terminal.expect_text("Req 02", true, 10000).unwrap();
    assert!(
        !fs::read_to_string(fixture._root.path().join("requirements/doc-02.md"))
            .unwrap()
            .contains("LOCAL_DRAFT")
    );
    capture(&terminal, "compact-editor-return");
}
