//! Integration coverage for the canonical source validation command.

use std::process::Command;

use serde_json::Value;

fn run_check(project: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_teshi"))
        .args(["check"].into_iter().chain(args.iter().copied()))
        .current_dir(project)
        .output()
        .expect("run teshi check")
}

fn write_feature(project: &std::path::Path, relative: &str, content: &str) {
    let path = project.join(relative);
    std::fs::create_dir_all(path.parent().expect("feature parent")).expect("create feature dir");
    std::fs::write(path, content).expect("write feature");
}

fn valid_feature() -> &'static str {
    "Feature: Login\n  Scenario: Success\n    Given the login page is open\n    When the user logs in\n    Then the dashboard is visible\n"
}

#[test]
fn default_scope_checks_all_project_features() {
    let project = tempfile::tempdir().expect("temp project");
    write_feature(
        project.path(),
        "features/en-US/login.feature",
        valid_feature(),
    );
    write_feature(
        project.path(),
        "features/zh-CN/登录.feature",
        "# language: zh-CN\n功能: 登录\n  场景: 成功\n    当用户登录\n",
    );

    let output = run_check(project.path(), &["--json"]);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["scope"].as_array().expect("scope").len(), 2);
    assert!(report["summary"]["errors"].as_u64().unwrap_or_default() >= 2);
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| { diagnostic["code"] == "missing_step_separator" })
    );
}

#[test]
fn all_scope_is_explicit_and_multilingual_json_is_stable() {
    let project = tempfile::tempdir().expect("temp project");
    write_feature(project.path(), "a.feature", valid_feature());
    write_feature(
        project.path(),
        "b.feature",
        "# language: zh-CN\n功能: 登录\n  场景: 成功\n    当用户登录\n",
    );

    let output = run_check(project.path(), &["--all", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(
        report["scope"],
        serde_json::json!(["a.feature", "b.feature"])
    );
    let separator = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "missing_step_separator")
        .expect("missing separator diagnostic");
    assert_eq!(separator["path"], "b.feature");
    assert_eq!(separator["line"], 4);
}

#[test]
fn explicit_feature_scope_does_not_include_siblings() {
    let project = tempfile::tempdir().expect("temp project");
    write_feature(project.path(), "features/good.feature", valid_feature());
    write_feature(
        project.path(),
        "features/bad.feature",
        "Feature: Bad\n  Scenario: Broken\n    Givenready\n",
    );

    let output = run_check(
        project.path(),
        &["--feature", "features/bad.feature", "--json"],
    );
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["scope"], serde_json::json!(["features/bad.feature"]));
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| { diagnostic["code"] == "missing_step_separator" })
    );
}

#[test]
fn warnings_only_report_succeeds() {
    let project = tempfile::tempdir().expect("temp project");
    write_feature(
        project.path(),
        "warning.feature",
        "Feature: Warning\n  Scenario: Starts with When\n    When the operation runs\n",
    );

    let output = run_check(project.path(), &["--feature", "warning.feature"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("warning"));
    assert!(stdout.contains("scenario_starts_without_given"));
    assert!(stdout.contains("Summary: 0 error(s), 1 warning(s)"));
}

#[test]
fn legal_repeated_given_steps_remain_non_blocking() {
    let project = tempfile::tempdir().expect("temp project");
    write_feature(
        project.path(),
        "repeated-given.feature",
        "Feature: Setup\n  Scenario: Preconditions only\n    Given the account exists\n    And the account is active\n",
    );

    let output = run_check(project.path(), &["--feature", "repeated-given.feature"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("missing_when"), "{stdout}");
    assert!(stdout.contains("missing_then"), "{stdout}");
    assert!(
        stdout.contains("Summary: 0 error(s), 2 warning(s)"),
        "{stdout}"
    );
}

#[test]
fn missing_feature_is_an_invocation_error() {
    let project = tempfile::tempdir().expect("temp project");
    let output = run_check(project.path(), &["--feature", "missing.feature", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&output.stdout).expect("JSON error envelope");
    assert_eq!(error["error"]["code"], "check_io_error");
}

#[test]
fn conflicting_scope_flags_are_rejected_by_cli() {
    let project = tempfile::tempdir().expect("temp project");
    let output = run_check(project.path(), &["--feature", "one.feature", "--all"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}
