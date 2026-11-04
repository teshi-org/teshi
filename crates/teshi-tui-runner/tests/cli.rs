//! Public CLI selection, precedence and failure propagation with real PTYs.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

#[test]
fn child_fixture() {
    if std::env::var_os("TESHI_TUI_CLI_FIXTURE").is_none() {
        return;
    }
    let mut stdout = std::io::stdout();
    write!(stdout, "\x1b[2J\x1b[Hcli-fixture-ready\r\n").unwrap();
    stdout.flush().unwrap();
    for line in std::io::stdin().lock().lines() {
        if line.unwrap().trim() == "quit" {
            return;
        }
    }
}

fn setup() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let feature = root.path().join("sample.feature");
    std::fs::write(&feature,"Feature: Terminal CLI\n  Background:\n    Given the application is ready\n  Scenario: Pass\n    Then ready is visible\n  Scenario: Fail\n    Then missing text is visible\n").unwrap();
    let assertion = |text, timeout| json!({"action":"expect_text","text":text,"visible":true,"timeout_ms":timeout});
    let bindings = json!({"version":1,"mode":"tui","steps":[
        {"line":3,"text":"the application is ready","confirmed":true,"actions":[{"action":"launch","config":{
            "program":std::env::current_exe().unwrap(),"args":["--exact","child_fixture","--nocapture"],"env":[["TESHI_TUI_CLI_FIXTURE","1"]],"cols":80,"rows":24
        }},assertion("cli-fixture-ready",5000)]},
        {"line":5,"text":"ready is visible","confirmed":true,"actions":[assertion("cli-fixture-ready",5000)]},
        {"line":7,"text":"missing text is visible","confirmed":true,"actions":[assertion("never-visible",50)]}
    ]});
    std::fs::write(
        feature.with_extension("tui.json"),
        serde_json::to_vec_pretty(&bindings).unwrap(),
    )
    .unwrap();
    (root, feature)
}

fn cli(root: &Path) -> Command {
    let binary = std::fs::canonicalize(
        std::env::var("TESHI_SELF_TEST_BIN").expect("set TESHI_SELF_TEST_BIN"),
    )
    .unwrap();
    let mut command = Command::new(binary);
    command
        .current_dir(root)
        .env("TESHI_TUI_RUNNER", env!("CARGO_BIN_EXE_teshi-tui-runner"))
        .env_remove("TESHI_RUNNER_CMD")
        .env_remove("TESHI_RUNNER_ARGS")
        .env_remove("TESHI_RUNNER_CWD");
    command
}

fn events(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn outline_rows_reach_the_public_cli_with_parent_failure_and_row_totals() {
    let (root, feature) = setup();
    std::fs::write(&feature,"Feature: Terminal CLI\n  Background:\n    Given the application is ready\n  Scenario Outline: Rows\n    Then see <expected>\n    Examples:\n      | expected |\n      | cli-fixture-ready |\n      | missing-answer |\n    Examples:\n      | expected |\n      | cli-fixture-ready |\n").unwrap();
    let sidecar = feature.with_extension("tui.json");
    let mut bindings: Value =
        serde_json::from_str(&std::fs::read_to_string(&sidecar).unwrap()).unwrap();
    bindings["steps"].as_array_mut().unwrap().truncate(2);
    bindings["steps"][1]["text"] = json!("see <expected>");
    bindings["steps"][1]["actions"][0]["text"] = json!("<expected>");
    bindings["steps"][1]["actions"][0]["timeout_ms"] = json!(100);
    std::fs::write(sidecar, serde_json::to_vec(&bindings).unwrap()).unwrap();
    for command in ["tui", "run"] {
        let output = cli(root.path())
            .args([command, "sample.feature", "--scenario", "Rows"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let events = events(&output);
        let rows: Vec<_> = events
            .iter()
            .filter(|e| e["type"] == "end_example")
            .collect();
        assert_eq!(rows.len(), 3, "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(rows[2]["status"], "passed");
        let summary = events.iter().find(|e| e["type"] == "end_run").unwrap();
        assert_eq!(summary["examples_passed"], 2);
        assert_eq!(summary["examples_failed"], 1);
        assert_eq!(summary["failed"], 1);
    }
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn selected_scenario_uses_relative_feature_path_and_excludes_failure() {
    let (root, _) = setup();
    for command in ["tui", "run"] {
        let output = cli(root.path())
            .args([command, "sample.feature", "--scenario", "Pass"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events = events(&output);
        assert_eq!(events.first().unwrap()["total"], 1);
        assert_eq!(events.last().unwrap()["passed"], 1);
        assert!(!events.iter().any(|event| event["name"] == "Fail"));
    }
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn failed_case_produces_nonzero_cli_exit_and_readable_screen_attachment() {
    let (root, _) = setup();
    for command in ["tui", "run"] {
        let output = cli(root.path())
            .args([command, "sample.feature", "--scenario", "Fail"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let events = events(&output);
        let failure = events
            .iter()
            .find(|event| event["type"] == "case_failed")
            .unwrap();
        let path = failure["error"]["attachments"][0]["path"].as_str().unwrap();
        let screen = std::fs::read_to_string(path).unwrap();
        assert!(
            screen.contains("cli-fixture-ready"),
            "{command}: failure={failure}, screen={screen:?}"
        );
        assert_eq!(events.last().unwrap()["failed"], 1);
    }
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn explicit_cli_and_environment_runners_take_precedence_and_propagate_failure() {
    let (root, _) = setup();
    let runner = env!("CARGO_BIN_EXE_teshi-tui-runner");
    for scenario in ["Pass", "Fail"] {
        for environment in [false, true] {
            let mut command = cli(root.path());
            command
                .args(["run", "sample.feature", "--scenario", scenario])
                .env("TESHI_TUI_RUNNER", "runner-that-must-not-start");
            if environment {
                command.env("TESHI_RUNNER_CMD", runner);
            } else {
                command.args(["--runner-cmd", runner]);
            }
            let output = command.output().unwrap();
            assert_eq!(
                output.status.success(),
                scenario == "Pass",
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("runner_exit"));
        }
    }
    std::fs::write(
        root.path().join("teshi.toml"),
        format!(
            "[runner]\ncmd = {}\n",
            serde_json::to_string(runner).unwrap()
        ),
    )
    .unwrap();
    for scenario in ["Pass", "Fail"] {
        let output = cli(root.path())
            .args(["run", "sample.feature", "--scenario", scenario])
            .env("TESHI_TUI_RUNNER", "runner-that-must-not-start")
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            scenario == "Pass",
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("runner_exit"));
    }
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn unknown_scenario_and_malformed_runner_configuration_fail_before_launch() {
    let (root, _) = setup();
    let output = cli(root.path())
        .args(["tui", "sample.feature", "--scenario", "Unknown"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    std::fs::write(root.path().join("teshi.toml"), "[runner\n").unwrap();
    let output = cli(root.path())
        .args(["run", "sample.feature"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
