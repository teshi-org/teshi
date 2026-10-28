//! Real pointer acceptance for Explore execution, deleted files, and quit ownership.
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
use teshi_tui_runner::{Launch, Terminal};

const SOURCE: &str = "Feature: Pointer runs\n  Scenario: Alpha\n    Given ALPHA_SENTINEL\n  Scenario: Beta\n    Given BETA_SENTINEL\n";
struct Fixture {
    root: tempfile::TempDir,
    project: PathBuf,
    source: PathBuf,
    trace: PathBuf,
    config: Launch,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        fs::create_dir(&project).unwrap();
        let source = project.join("pointer.feature");
        fs::write(&source, SOURCE).unwrap();
        let trace = root.path().join("request.json");
        let runner = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/explore_pointer_runner.py")
            .canonicalize()
            .unwrap();
        let python = std::env::var("TESHI_TEST_PYTHON")
            .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.into());
        fs::write(
            project.join("teshi.toml"),
            format!(
                "[runner]\ncmd = {}\nargs = {}\ncwd = \".\"\n",
                serde_json::to_string(&python).unwrap(),
                serde_json::to_string(&[runner, trace.clone()]).unwrap()
            ),
        )
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
        env.push(("TESHI_AGENT_BACKEND".into(), "native".into()));
        env.push(("TESHI_LLM_API_KEY".into(), String::new()));
        let config = Launch {
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
            cols: 140,
            rows: 30,
        };
        Self {
            root,
            project,
            source,
            trace,
            config,
        }
    }
    fn launch(&self) -> Terminal {
        let terminal = Terminal::launch(self.config.clone()).unwrap();
        terminal.expect_text("Alpha", true, 10000).unwrap();
        terminal
    }
    fn capture(&self, terminal: &Terminal, stage: &str) {
        let output = PathBuf::from(std::env::var_os("TESHI_EXPLORE_POINTER_ARTIFACTS").unwrap());
        fs::create_dir_all(&output).unwrap();
        fs::write(
            output.join(format!("{stage}.txt")),
            terminal.screen().unwrap(),
        )
        .unwrap();
        terminal
            .screenshot(&output.join(format!("{stage}.svg")))
            .unwrap();
        if self.trace.exists() {
            fs::copy(&self.trace, output.join(format!("{stage}-request.json"))).unwrap();
        }
    }
    fn git(&self, args: &[&str]) {
        let hooks = self.root.path().join("no-hooks");
        fs::create_dir_all(&hooks).unwrap();
        let result = Command::new("git")
            .arg("-c")
            .arg(format!("core.hooksPath={}", hooks.display()))
            .args([
                "-c",
                "user.name=Pointer Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&self.project)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
fn quit(terminal: Terminal) {
    terminal.key("q").unwrap();
    terminal.expect_exit(0, 10000).unwrap();
}

#[test]
#[ignore = "requires a built Teshi CLI and Python; set TESHI_SELF_TEST_BIN"]
fn explore_run_and_detail_pointer_use_the_selected_scenario_at_normal_and_narrow_sizes() {
    for narrow in [false, true] {
        let mut fixture = Fixture::new();
        if narrow {
            fixture.config.cols = 55;
            fixture.config.rows = 16;
        }
        let terminal = fixture.launch();
        if narrow {
            // doc-31/doc-36: below 70 columns only the focused pane is drawn,
            // so the scenario column has to be focused before it is clickable.
            terminal.key("Tab").unwrap();
            std::thread::sleep(Duration::from_millis(300));
        }
        terminal.click_text("Beta", 10000).unwrap();
        terminal
            .click_text(if narrow { " Run" } else { "Run [r]" }, 10000)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fixture.trace.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        fixture.capture(&terminal, if narrow { "run-narrow" } else { "run-normal" });
        let request: Value = serde_json::from_slice(&fs::read(&fixture.trace).unwrap()).unwrap();
        assert_eq!(request["request"]["cases"].as_array().unwrap().len(), 1);
        assert_eq!(request["request"]["cases"][0]["scenario"], "Beta");
        assert_eq!(
            fs::canonicalize(request["cwd"].as_str().unwrap()).unwrap(),
            fs::canonicalize(&fixture.project).unwrap()
        );
        terminal.expect_text("1 failed", true, 10000).unwrap();
        terminal
            .click_text(if narrow { " Detail" } else { "Detail [Enter]" }, 10000)
            .unwrap();
        terminal
            .expect_text("POINTER_FAILURE_EVIDENCE", true, 10000)
            .unwrap();
        fixture.capture(
            &terminal,
            if narrow {
                "detail-narrow"
            } else {
                "detail-normal"
            },
        );
        terminal.key("Escape").unwrap();
        terminal.expect_text("Scenarios", true, 10000).unwrap();
        assert_eq!(fs::read_to_string(&fixture.source).unwrap(), SOURCE);
        if narrow {
            terminal.key("Escape").unwrap();
        }
        quit(terminal);
    }
}

#[test]
#[ignore = "requires a built Teshi CLI, Python and Git"]
fn deleted_feature_pointer_shows_head_content_without_running_or_recreating_the_file() {
    let fixture = Fixture::new();
    let deleted = fixture.project.join("removed.feature");
    let original = "Feature: Removed\n  Scenario: DELETED_SCENARIO\n    Given DELETED_STEP\n";
    fs::write(&deleted, original).unwrap();
    fixture.git(&["init", "--quiet"]);
    fixture.git(&["add", "pointer.feature", "removed.feature"]);
    fixture.git(&["commit", "--quiet", "-m", "Seed fixture"]);
    fs::remove_file(&deleted).unwrap();
    let terminal = fixture.launch();
    terminal.click_text("removed", 10000).unwrap();
    terminal
        .expect_text("DELETED_SCENARIO", true, 10000)
        .unwrap();
    terminal.click_text("DELETED_SCENARIO", 10000).unwrap();
    terminal.expect_text("DELETED_STEP", true, 10000).unwrap();
    fixture.capture(&terminal, "deleted-head-preview");
    terminal.click_text("Run [r]", 10000).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert!(!fixture.trace.exists(), "deleted content reached runner");
    assert!(!deleted.exists(), "deleted source was recreated");
    fs::write(&deleted, original).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    terminal.key("1").unwrap();
    fixture.capture(&terminal, "externally-restored");
    assert_eq!(fs::read_to_string(&deleted).unwrap(), original);
    quit(terminal);
}

#[test]
#[ignore = "requires a built Teshi CLI"]
fn quit_modal_owns_pointer_and_no_keeps_unsaved_feature_content() {
    let fixture = Fixture::new();
    let terminal = fixture.launch();
    terminal.click_text("Edit [e]", 10000).unwrap();
    terminal.key("Enter").unwrap();
    terminal.write("POINTER_DIRTY").unwrap();
    terminal.key("Enter").unwrap();
    terminal.key("Escape").unwrap();
    terminal.key("q").unwrap();
    terminal.expect_text("Quit Teshi?", true, 10000).unwrap();
    terminal.click_text("AI [3]", 10000).unwrap();
    terminal.scroll_at(2, 5, false, false).unwrap();
    terminal.expect_text("Quit Teshi?", true, 10000).unwrap();
    fixture.capture(&terminal, "quit-owned");
    terminal.click_text("[No]", 10000).unwrap();
    terminal.expect_text("Quit Teshi?", false, 10000).unwrap();
    assert_eq!(fs::read_to_string(&fixture.source).unwrap(), SOURCE);
    fixture.capture(&terminal, "quit-no-keeps-edit");
    terminal.key("q").unwrap();
    terminal.expect_text("Quit Teshi?", true, 10000).unwrap();
    terminal.click_text("[Yes]", 10000).unwrap();
    terminal.expect_exit(0, 10000).unwrap();
    assert_eq!(fs::read_to_string(&fixture.source).unwrap(), SOURCE);
}
