//! Execution and binding commands fail closed before touching persisted state.

use std::process::Command;

fn run_steps(project: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_teshi"))
        .args(["steps"].into_iter().chain(args.iter().copied()))
        .current_dir(project)
        .output()
        .expect("run teshi steps")
}

fn invalid_project() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("temp project");
    std::fs::create_dir_all(project.path().join(".teshi")).expect("create teshi directory");
    let feature = project.path().join("features").join("broken.feature");
    std::fs::create_dir_all(feature.parent().expect("feature parent"))
        .expect("create feature directory");
    std::fs::write(
        feature,
        "Feature: Broken\n  Scenario: Missing separator\n    Givenready\n",
    )
    .expect("write invalid feature");
    project
}

#[test]
fn unbound_fails_with_diagnostics_instead_of_returning_empty_success() {
    let project = invalid_project();
    let output = run_steps(
        project.path(),
        &["unbound", "--feature", "features/broken.feature"],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("missing_step_separator"), "{stderr}");
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());
}

#[test]
fn next_unbound_rejects_before_changing_active_step_state() {
    let project = invalid_project();
    let active_step = project.path().join(".teshi").join("active-step.json");
    let before = br#"{"sentinel":"keep"}
"#;
    std::fs::write(&active_step, before).expect("write sentinel active step");

    let output = run_steps(
        project.path(),
        &["next-unbound", "--feature", "features/broken.feature"],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing_step_separator"));
    assert_eq!(
        std::fs::read(&active_step).expect("read active step"),
        before
    );
}

#[test]
fn unbind_rejects_before_changing_binding_state() {
    let project = invalid_project();
    let bindings = project.path().join("features").join("broken.bindings.json");
    let before = br#"{"format_version":2,"feature":"features/broken.feature","steps":[]}"#;
    std::fs::write(&bindings, before).expect("write sentinel bindings");

    let output = run_steps(
        project.path(),
        &[
            "unbind",
            "--feature",
            "features/broken.feature",
            "--line",
            "3",
        ],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing_step_separator"));
    assert_eq!(std::fs::read(&bindings).expect("read bindings"), before);
}

#[test]
fn run_rejects_before_loading_or_starting_the_runner() {
    let project = invalid_project();
    let output = Command::new(env!("CARGO_BIN_EXE_teshi"))
        .args([
            "run",
            "features/broken.feature",
            "--runner-cmd",
            "teshi-command-that-must-not-start",
        ])
        .current_dir(project.path())
        .output()
        .expect("run teshi run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("missing_step_separator"), "{stderr}");
    assert!(
        !stderr.contains("spawn runner"),
        "runner was reached: {stderr}"
    );
}

#[test]
fn directory_run_validates_only_the_selected_directory() {
    let project = tempfile::tempdir().expect("temp project");
    std::fs::create_dir_all(project.path().join(".teshi")).expect("create teshi directory");
    std::fs::create_dir_all(project.path().join("selected")).expect("create selected directory");
    std::fs::create_dir_all(project.path().join("sibling")).expect("create sibling directory");
    std::fs::write(
        project.path().join("selected").join("ok.feature"),
        "Feature: Selected\n  Scenario: Works\n    Given the page is open\n    When I act\n    Then it succeeds\n",
    )
    .expect("write selected Feature");
    std::fs::write(
        project.path().join("sibling").join("broken.feature"),
        "Feature: Sibling\n  Scenario: Broken\n    Givenready\n",
    )
    .expect("write sibling Feature");

    let (runner_cmd, runner_args): (&str, &[&str]) = if cfg!(windows) {
        ("cmd.exe", &["/C", "exit", "0"])
    } else {
        ("sh", &["-c", "exit 0"])
    };
    let mut args = vec![
        "run".to_string(),
        "selected".to_string(),
        "--runner-cmd".to_string(),
        runner_cmd.to_string(),
    ];
    for runner_arg in runner_args {
        args.push("--runner-arg".to_string());
        args.push((*runner_arg).to_string());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_teshi"))
        .args(&args)
        .current_dir(project.path())
        .output()
        .expect("run selected directory");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("missing_step_separator"),
        "sibling diagnostic leaked into selected run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn browser_and_winapp_replay_reject_before_target_setup() {
    let project = invalid_project();
    for command in ["browser", "winapp"] {
        let output = Command::new(env!("CARGO_BIN_EXE_teshi"))
            .args([
                command,
                "replay",
                "--feature",
                "features/broken.feature",
                "--dry-run",
            ])
            .current_dir(project.path())
            .output()
            .expect("run replay command");
        assert!(
            !output.status.success(),
            "{command} replay unexpectedly passed"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("missing_step_separator"),
            "{command}: {stderr}"
        );
        assert!(
            !stderr.contains("sidecar"),
            "target setup was reached: {command}: {stderr}"
        );
    }
}
