//! Real PTY acceptance for session, approval, and location footer actions.

#[path = "support/requirement_project.rs"]
mod requirement_project;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use teshi_tui_runner::{Launch, Terminal};

struct Fixture {
    root: tempfile::TempDir,
    config: Launch,
    source: PathBuf,
    original: Vec<u8>,
}

impl Fixture {
    fn new(cols: u16, rows: u16) -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("overlay-project");
        fs::create_dir(&project).unwrap();
        let source = project.join("location.feature");
        let original = b"Feature: Overlay pointers\n  Scenario: Alpha\n    Given SHARED\n    Then ALPHA_END\n  Scenario: Beta\n    Given SHARED\n    Then BETA_END\n".to_vec();
        fs::write(&source, &original).unwrap();
        let sessions = root.path().join("TESHI_SESSION_DIR");
        fs::create_dir(&sessions).unwrap();
        for (name, updated) in [("Alpha", "9999999999999"), ("Beta", "9999999999998")] {
            fs::write(
                sessions.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "id": name, "created_at": name, "updated_at": updated,
                    "model_label": format!("Session {name}"), "message_count": 1,
                    "project_root": project, "backend_kind": "native", "acp_agent_id": null,
                    "messages": [{"role": "Assistant", "content": format!("SESSION_{name}_BODY"),
                        "source": null, "tool_calls": null, "tool_call_id": null,
                        "reasoning_content": null}]
                }))
                .unwrap(),
            )
            .unwrap();
        }
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
        .map(|key| {
            (
                key.into(),
                root.path().join(key).to_string_lossy().into_owned(),
            )
        })
        .collect::<Vec<_>>();
        env.extend([
            ("TESHI_AGENT_BACKEND".into(), "native".into()),
            ("TESHI_LLM_API_KEY".into(), String::new()),
            ("TESHI_LLM_BASE_URL".into(), "http://127.0.0.1:1/v1".into()),
            ("NO_COLOR".into(), String::new()),
            ("TERM".into(), "xterm-256color".into()),
        ]);
        let config = Launch {
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
            cols,
            rows,
        };
        Self {
            root,
            config,
            source,
            original,
        }
    }

    fn launch(&self) -> Terminal {
        let terminal = Terminal::launch(self.config.clone()).unwrap();
        terminal.expect_text("location", true, 10000).unwrap();
        terminal
    }

    fn session_path(&self, name: &str) -> PathBuf {
        self.root
            .path()
            .join("TESHI_SESSION_DIR")
            .join(format!("{name}.json"))
    }

    fn unchanged(&self) {
        assert_eq!(fs::read(&self.source).unwrap(), self.original);
    }
}

fn capture(terminal: &Terminal, stage: &str) {
    std::thread::sleep(Duration::from_millis(100));
    let output = PathBuf::from(std::env::var_os("TESHI_OVERLAY_ARTIFACTS").unwrap());
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join(format!("{stage}.txt")),
        terminal.screen().unwrap(),
    )
    .unwrap();
    terminal
        .screenshot(&output.join(format!("{stage}.svg")))
        .unwrap();
}

fn command(terminal: &Terminal, input: &str) {
    terminal.write(input).unwrap();
    terminal.expect_text(input, true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    terminal
        .expect_text(&format!("│{input}"), false, 5000)
        .unwrap();
}

fn click_footer(terminal: &Terminal, label: &str) {
    terminal.expect_text(label, true, 5000).unwrap();
    let screen = terminal.screen().unwrap();
    // Distinguish a modal Close action from the still-visible background agent toolbar.
    let label = if label == "Close" && screen.contains("Close [Esc]") {
        "Close [Esc]"
    } else {
        label
    };
    let (col, row) = screen
        .lines()
        .enumerate()
        .filter_map(|(row, line)| {
            line.find(label)
                .map(|byte| (line[..byte].chars().count() + 1, row + 1))
        })
        .last()
        .unwrap_or_else(|| panic!("footer {label:?} was not visible\n{screen}"));
    terminal
        .write(&format!("\x1b[<0;{col};{row}M\x1b[<0;{col};{row}m"))
        .unwrap();
}

fn close_app(terminal: &Terminal) {
    terminal.key("Escape").unwrap();
    terminal.key("Escape").unwrap();
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

fn session_footer_flow(cols: u16, rows: u16, stage: &str) {
    let fixture = Fixture::new(cols, rows);
    let terminal = fixture.launch();
    terminal.key("3").unwrap();
    terminal.expect_text("AI Chat", true, 5000).unwrap();
    terminal.key("Enter").unwrap();
    command(&terminal, "/sessions");
    terminal.expect_text("Load [Enter]", true, 5000).unwrap();
    capture(&terminal, &format!("{stage}-session-open"));
    terminal.click_text("Model: Session Beta", 5000).unwrap();
    click_footer(&terminal, "Load");
    terminal.expect_text("Load [Enter]", false, 5000).unwrap();
    terminal
        .expect_text("SESSION_Beta_BODY", true, 5000)
        .unwrap();
    capture(&terminal, &format!("{stage}-session-loaded-beta"));
    command(&terminal, "/sessions");
    terminal
        .expect_text("Model: Session Alpha", true, 5000)
        .unwrap();
    terminal.click_text("Model: Session Alpha", 5000).unwrap();
    click_footer(&terminal, "Delete");
    terminal
        .expect_text("Confirm deletion", true, 5000)
        .unwrap();
    click_footer(&terminal, "Delete [Enter/y]");
    terminal
        .expect_text("Model: Session Alpha", false, 5000)
        .unwrap();
    assert!(
        !fixture.session_path("Alpha").exists(),
        "inactive session was not deleted"
    );
    assert!(
        fixture.session_path("Beta").exists(),
        "deleting Alpha removed Beta"
    );
    capture(&terminal, &format!("{stage}-session-deleted-alpha"));
    click_footer(&terminal, "Close");
    terminal.expect_text("Load [Enter]", false, 5000).unwrap();
    terminal
        .expect_text("SESSION_Beta_BODY", true, 5000)
        .unwrap();
    capture(
        &terminal,
        &format!("{stage}-session-close-restored-history"),
    );
    fixture.unchanged();
    close_app(&terminal);
    assert!(
        !fixture.session_path("Alpha").exists(),
        "deleted inactive history reappeared on exit"
    );
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn session_load_delete_and_close_footer_actions_in_normal_terminal() {
    session_footer_flow(150, 30, "normal");
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn session_load_delete_and_close_footer_actions_in_narrow_terminal() {
    session_footer_flow(55, 16, "narrow");
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn approval_close_footer_returns_to_unchanged_chat() {
    for (cols, rows, stage) in [(150, 30, "normal"), (55, 16, "narrow")] {
        let fixture = Fixture::new(cols, rows);
        let terminal = fixture.launch();
        terminal.key("3").unwrap();
        terminal.expect_text("AI Chat", true, 5000).unwrap();
        terminal.key("Enter").unwrap();
        command(&terminal, "/resume");
        terminal
            .expect_text("SESSION_Alpha_BODY", true, 5000)
            .unwrap();
        command(&terminal, "/approval");
        terminal.expect_text("Approval Mode", true, 5000).unwrap();
        capture(&terminal, &format!("{stage}-approval-open"));
        click_footer(&terminal, "Close");
        terminal.expect_text("Approval Mode", false, 5000).unwrap();
        terminal
            .expect_text("SESSION_Alpha_BODY", true, 5000)
            .unwrap();
        capture(
            &terminal,
            &format!("{stage}-approval-close-restored-history"),
        );
        fixture.unchanged();
        close_app(&terminal);
    }
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn mindmap_location_open_and_close_footer_actions() {
    for (cols, rows, stage) in [(150, 30, "normal"), (55, 16, "narrow")] {
        let fixture = Fixture::new(cols, rows);
        let terminal = fixture.launch();
        terminal.key("2").unwrap();
        terminal.expect_text("MindMap", true, 5000).unwrap();
        terminal.click_text("▶ overlay-project", 5000).unwrap();
        terminal.expect_text("SHARED", true, 5000).unwrap();
        terminal.click_text("SHARED", 5000).unwrap();
        terminal.expect_text("Scenario: Alpha", true, 5000).unwrap();
        terminal.key("o").unwrap();
        terminal.expect_text("Open Scenario", true, 5000).unwrap();
        capture(&terminal, &format!("{stage}-location-open"));
        terminal.key("Down").unwrap();
        click_footer(&terminal, "Close");
        terminal.expect_text("Open Scenario", false, 5000).unwrap();
        terminal.expect_text("Scenario: Alpha", true, 5000).unwrap();
        capture(
            &terminal,
            &format!("{stage}-location-close-preserved-alpha"),
        );
        terminal.key("o").unwrap();
        terminal.expect_text("Open Scenario", true, 5000).unwrap();
        terminal.key("Down").unwrap();
        click_footer(&terminal, "Open");
        terminal.expect_text("Open Scenario", false, 5000).unwrap();
        terminal.expect_text("Scenario: Beta", true, 5000).unwrap();
        capture(&terminal, &format!("{stage}-location-open-committed-beta"));
        fixture.unchanged();
        close_app(&terminal);
    }
}

fn assert_background_owned(terminal: &Terminal, title: &str, cols: u16) {
    // A tab click and wheel event outside the modal must leave its active draft intact.
    terminal.write("\x1b[<0;2;1M\x1b[<0;2;1m").unwrap();
    terminal.scroll_at(cols - 2, 2, false, false).unwrap();
    terminal.expect_text(title, true, 5000).unwrap();
}

fn save_evidence(name: &str, bytes: &[u8]) {
    let output = PathBuf::from(std::env::var_os("TESHI_OVERLAY_ARTIFACTS").unwrap());
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join(name), bytes).unwrap();
}

fn wait_for_saved_source(terminal: &Terminal, path: &Path, expected: &str) -> String {
    // PTY key submission returns before the application processes the save.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let actual = fs::read_to_string(path);
        if let Ok(body) = &actual
            && body == expected
        {
            return body.clone();
        }
        assert!(
            Instant::now() < deadline,
            "saved source did not match: expected {expected:?}, found {actual:?}\n{}",
            terminal.screen().unwrap()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn generation_scope_iteration_cancel_and_confirm_preserve_sources() {
    use teshi_core::authoring::{
        DocumentRevision, RequirementDocumentIndex, RequirementDocumentMeta,
    };
    for (cols, rows, stage) in [(150, 30, "normal"), (55, 16, "narrow")] {
        let fixture = Fixture::new(cols, rows);
        let store = fixture.root.path().join("requirements");
        fs::create_dir(&store).unwrap();
        let requirement = b"# Scope source\nSCOPE_REQUIREMENT_SENTINEL\n";
        fs::write(store.join("scope.md"), requirement).unwrap();
        let mut index = RequirementDocumentIndex::initialize_empty_store();
        let mut meta = RequirementDocumentMeta::new(
            "scope",
            "scope.md",
            "Scope source",
            DocumentRevision::new("scope-revision"),
        );
        meta.iteration = Some("Sprint Scope".into());
        index.documents.push(meta);
        requirement_project::associate(fixture.source.parent().unwrap(), &mut index);
        let index_bytes = serde_json::to_vec_pretty(&index).unwrap();
        fs::write(store.join("_teshi.json"), &index_bytes).unwrap();
        let state_path = fixture
            .source
            .parent()
            .unwrap()
            .join(".teshi/generation-state.json");
        let terminal = fixture.launch();
        terminal.key("3").unwrap();
        terminal.expect_text("AI Chat", true, 5000).unwrap();
        terminal.key("Enter").unwrap();
        // Keep the command visible on one row even in the narrow input pane.
        command(&terminal, "/generate scope");
        terminal
            .expect_text("Confirm generation source scope", true, 5000)
            .unwrap();
        assert_background_owned(&terminal, "Confirm generation source scope", cols);
        capture(&terminal, &format!("{stage}-scope-open-owned"));
        click_footer(&terminal, "Cancel");
        terminal
            .expect_text("Confirm generation source scope", false, 5000)
            .unwrap();
        assert!(
            !state_path.exists(),
            "cancel persisted a generation request"
        );
        command(&terminal, "/generate scope");
        terminal
            .expect_text("Confirm generation source scope", true, 5000)
            .unwrap();
        click_footer(&terminal, "Iteration");
        terminal.expect_text("Sprint Scope", true, 5000).unwrap();
        terminal.click_text("Sprint Scope", 5000).unwrap();
        terminal
            .expect_text("Iteration scope: Sprint Scope", true, 5000)
            .unwrap();
        capture(&terminal, &format!("{stage}-scope-selected-iteration"));
        click_footer(&terminal, "Confirm");
        terminal
            .expect_text("Confirm generation source scope", false, 5000)
            .unwrap();
        let state_bytes = fs::read(&state_path).unwrap();
        let state: serde_json::Value = serde_json::from_slice(&state_bytes).unwrap();
        assert_eq!(state["stage"], "Gathering");
        assert_eq!(state["source_scope"]["iteration"]["named"], "Sprint Scope");
        assert_eq!(
            state["source_scope"]["store_id"],
            serde_json::to_value(index.store_id).unwrap()
        );
        save_evidence(&format!("{stage}-scope-confirmed-state.json"), &state_bytes);
        assert_eq!(fs::read(store.join("scope.md")).unwrap(), requirement);
        assert_eq!(fs::read(store.join("_teshi.json")).unwrap(), index_bytes);
        fixture.unchanged();
        capture(&terminal, &format!("{stage}-scope-confirmed"));
        close_app(&terminal);
    }
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn step_keyword_pointer_cancel_and_confirm_save_only_selected_step() {
    for (cols, rows, stage) in [(150, 30, "normal"), (55, 16, "narrow")] {
        let fixture = Fixture::new(cols, rows);
        let terminal = fixture.launch();
        click_footer(&terminal, "Edit");
        terminal.key("Ctrl+g").unwrap();
        terminal.key("Enter").unwrap();
        terminal.expect_text("Step keyword", true, 5000).unwrap();
        assert_background_owned(&terminal, "Step keyword", cols);
        terminal.click_text("When", 5000).unwrap();
        capture(
            &terminal,
            &format!("{stage}-keyword-selected-before-cancel"),
        );
        click_footer(&terminal, "Cancel");
        terminal.expect_text("Step keyword", false, 5000).unwrap();
        terminal.key("s").unwrap();
        fixture.unchanged();
        terminal.key("Enter").unwrap();
        terminal.expect_text("Step keyword", true, 5000).unwrap();
        terminal.click_text("When", 5000).unwrap();
        click_footer(&terminal, "Confirm");
        terminal.expect_text("Step keyword", false, 5000).unwrap();
        terminal.key("s").unwrap();
        let expected = String::from_utf8(fixture.original.clone())
            .unwrap()
            .replacen("Given SHARED", "When SHARED", 1);
        let saved = wait_for_saved_source(&terminal, &fixture.source, &expected);
        save_evidence(
            &format!("{stage}-keyword-confirmed.feature"),
            saved.as_bytes(),
        );
        capture(&terminal, &format!("{stage}-keyword-confirmed"));
        close_app(&terminal);
    }
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI"]
fn external_change_pointer_keep_then_reload_resolves_dirty_feature() {
    for (cols, rows, stage) in [(150, 30, "normal"), (55, 16, "narrow")] {
        let fixture = Fixture::new(cols, rows);
        let terminal = fixture.launch();
        click_footer(&terminal, "Edit");
        terminal.key("Enter").unwrap();
        terminal.write("LOCAL_ONLY_EDIT").unwrap();
        terminal.key("Enter").unwrap();
        // Gate on the commit landing before touching the disk: a commit Enter
        // still queued when the modal opens is consumed as the modal's
        // Reload [Enter] and silently discards the local edit.
        terminal.expect_text("Edit committed", true, 10000).unwrap();
        let external = String::from_utf8(fixture.original.clone())
            .unwrap()
            .replacen("ALPHA_END", "DISK_FIRST", 1);
        fs::write(&fixture.source, &external).unwrap();
        terminal
            .expect_text("Feature changed on disk", true, 10000)
            .unwrap();
        assert_background_owned(&terminal, "Feature changed on disk", cols);
        capture(&terminal, &format!("{stage}-external-change-owned"));
        click_footer(&terminal, "Keep");
        terminal
            .expect_text("Feature changed on disk", false, 5000)
            .unwrap();
        terminal.expect_text("LOCAL_ONLY_EDIT", true, 5000).unwrap();
        assert_eq!(fs::read_to_string(&fixture.source).unwrap(), external);
        capture(&terminal, &format!("{stage}-external-kept-local"));
        let next_external = external.replace("DISK_FIRST", "DISK_RELOAD_SENTINEL");
        fs::write(&fixture.source, &next_external).unwrap();
        terminal
            .expect_text("Feature changed on disk", true, 10000)
            .unwrap();
        click_footer(&terminal, "Reload");
        terminal
            .expect_text("Feature changed on disk", false, 5000)
            .unwrap();
        terminal
            .expect_text("DISK_RELOAD_SENTINEL", true, 5000)
            .unwrap();
        terminal
            .expect_text("LOCAL_ONLY_EDIT", false, 5000)
            .unwrap();
        terminal.key("s").unwrap();
        assert_eq!(fs::read_to_string(&fixture.source).unwrap(), next_external);
        save_evidence(
            &format!("{stage}-external-reloaded.feature"),
            next_external.as_bytes(),
        );
        capture(&terminal, &format!("{stage}-external-reloaded"));
        close_app(&terminal);
    }
}

struct ProposalServer(Child);

impl Drop for ProposalServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires TESHI_SELF_TEST_BIN pointing to a built CLI and Python"]
fn change_summary_pointer_select_jump_and_close_preserve_pending_source() {
    for (cols, rows, stage) in [(150, 30, "normal"), (55, 16, "narrow")] {
        let mut fixture = Fixture::new(cols, rows);
        let trace = fixture.root.path().join("proposal");
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/overlay_change_server.py");
        let _server = ProposalServer(
            Command::new(std::env::var("TESHI_TEST_PYTHON").unwrap_or_else(|_| "python".into()))
                .arg(script)
                .arg(&trace)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !trace.join("port").exists() {
            assert!(
                Instant::now() < deadline,
                "owned proposal server failed to start"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let port = fs::read_to_string(trace.join("port")).unwrap();
        for (key, value) in &mut fixture.config.env {
            match key.as_str() {
                "TESHI_LLM_API_KEY" => *value = "overlay-synthetic-key".into(),
                "TESHI_LLM_BASE_URL" => *value = format!("http://127.0.0.1:{port}/v1"),
                _ => {}
            }
        }
        fixture
            .config
            .env
            .push(("NO_PROXY".into(), "127.0.0.1,localhost,::1".into()));
        let terminal = fixture.launch();
        terminal.key("3").unwrap();
        terminal.expect_text("AI Chat", true, 5000).unwrap();
        terminal.key("Enter").unwrap();
        terminal
            .write("Propose a Beta step change for human review.")
            .unwrap();
        terminal.key("Enter").unwrap();
        terminal.expect_text("Change Summary", true, 15000).unwrap();
        assert_background_owned(&terminal, "Change Summary", cols);
        capture(&terminal, &format!("{stage}-summary-open-owned"));
        terminal
            .expect_text("2 change(s) pending", true, 5000)
            .unwrap();
        let screen = terminal.screen().unwrap();
        let (row, line) = screen
            .lines()
            .enumerate()
            .find(|(_, line)| line.contains("[~]") && line.contains("Beta"))
            .unwrap();
        let col = line[..line.find("Beta").unwrap()].chars().count() + 1;
        terminal
            .write(&format!(
                "\x1b[<0;{col};{}M\x1b[<0;{col};{}m",
                row + 1,
                row + 1
            ))
            .unwrap();
        capture(&terminal, &format!("{stage}-summary-selected-beta"));
        click_footer(&terminal, "Jump");
        terminal.expect_text("Change Summary", false, 5000).unwrap();
        terminal.expect_text("Scenario: Beta", true, 5000).unwrap();
        fixture.unchanged();
        capture(&terminal, &format!("{stage}-summary-jumped-beta"));
        terminal.key("s").unwrap();
        terminal.expect_text("Change Summary", true, 5000).unwrap();
        click_footer(&terminal, "Close");
        terminal.expect_text("Change Summary", false, 5000).unwrap();
        fixture.unchanged();
        terminal.key("n").unwrap();
        terminal.key("n").unwrap();
        capture(&terminal, &format!("{stage}-summary-closed-rejected"));
        fixture.unchanged();
        save_evidence(
            &format!("{stage}-summary-requests.jsonl"),
            &fs::read(trace.join("requests.jsonl")).unwrap(),
        );
        close_app(&terminal);
    }
}
