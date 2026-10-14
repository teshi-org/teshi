//! Actual Teshi interface regressions through the public CLI and Rust runner.

use serde_json::json;
use std::io::{BufRead, Write};
use std::process::Command;

#[test]
fn runner_child_fixture() {
    if std::env::var_os("TESHI_SELF_RUNNER_FIXTURE").is_none() {
        return;
    }
    let mut stdout = std::io::stdout();
    write!(stdout, "\x1b[2J\x1b[Hself-runner-ready\r\n").unwrap();
    stdout.flush().unwrap();
    for line in std::io::stdin().lock().lines() {
        if line.unwrap().trim() == "quit" {
            return;
        }
    }
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn english_tui_command_edits_and_saves() {
    verify(
        include_str!("../../../features/en-US/tui_editor.feature"),
        "tui",
    );
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn chinese_run_command_routes_to_tui_and_saves() {
    verify(
        include_str!("../../../features/zh-CN/终端功能编辑.feature"),
        "run",
    );
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn wrapped_acp_replies_remain_visible_after_six_turns() {
    verify_wrapped_chat(
        include_str!("../../../features/en-US/tui_acp_wrapped_chat.feature"),
        &[(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6)],
    );
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn thirty_turn_wrapped_acp_conversation_keeps_each_reply_visible() {
    verify_wrapped_chat(
        include_str!("../../../features/en-US/tui_acp_long_conversation.feature"),
        &[(1, 10), (11, 20), (21, 30)],
    );
}

#[test]
#[ignore = "requires authenticated CodeBuddy and TESHI_CODEBUDDY_LIVE=1"]
fn live_codebuddy_completes_ten_visible_turns() {
    if std::env::var("TESHI_CODEBUDDY_LIVE").as_deref() != Ok("1") {
        eprintln!("skipped: set TESHI_CODEBUDDY_LIVE=1 to run the live provider test");
        return;
    }
    verify_live_codebuddy(include_str!(
        "../../../features/en-US/tui_codebuddy_live_conversation.feature"
    ));
}

fn verify_live_codebuddy(source: &str) {
    let binary = std::fs::canonicalize(
        std::env::var("TESHI_SELF_TEST_BIN").expect("set TESHI_SELF_TEST_BIN"),
    )
    .unwrap();
    let local_app_data = std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA is required");
    let app_data = std::env::var("APPDATA").expect("APPDATA is required");
    let manifest = std::path::Path::new(&local_app_data).join("teshi/acp/installed.json");
    let installed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).expect("read installed ACP agents"))
            .expect("parse installed ACP agents");
    let codebuddy = installed
        .as_array()
        .and_then(|agents| agents.iter().find(|agent| agent["id"] == "codebuddy-code"))
        .expect("CodeBuddy ACP is not installed");
    let program = std::path::PathBuf::from(codebuddy["program"].as_str().unwrap());
    let package_root = program.parent().unwrap().parent().unwrap();
    let entry = package_root.join("@tencent-ai/codebuddy-code/bin/codebuddy");
    assert!(
        entry.is_file(),
        "CodeBuddy entry is missing: {}",
        entry.display()
    );
    let agent_args = codebuddy["args"].as_array().cloned().unwrap_or_default();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("sample.feature"),
        "Feature: Live CodeBuddy fixture\n  Scenario: Ready\n    Given a sample step\n",
    )
    .unwrap();
    let feature = root.path().join("codebuddy-live.feature");
    std::fs::write(&feature, source).unwrap();
    let proxy = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/codebuddy_acp_proxy.mjs")
        .canonicalize()
        .unwrap();
    let protocol_log = root.path().join("codebuddy-protocol.ndjson");
    let text = |value: &str| json!({"action":"expect_text","text":value,"visible":true,"timeout_ms":120000});
    let key = |value: &str| json!({"action":"key","key":value});
    let parsed = teshi_core::gherkin::parse_feature(source, feature.clone());
    let background = parsed.background.as_ref().unwrap();
    let scenario = parsed.all_scenarios()[0];
    let launch = json!({"action":"launch","config":{
        "program":binary,
        "args":[project,"--requirements-root",root.path().join("requirements")],
        "cwd":root.path(),
        "env":[
            ["APPDATA",root.path().join("appdata")],
            ["LOCALAPPDATA",root.path().join("localappdata")],
            ["XDG_CONFIG_HOME",root.path().join("config")],
            ["XDG_DATA_HOME",root.path().join("data")],
            ["TESHI_AGENT_BACKEND","acp"],
            ["TESHI_ACP_PROGRAM","node"],
            ["TESHI_ACP_ARGS_JSON",serde_json::to_string(&vec![proxy]).unwrap()],
            ["TESHI_CODEBUDDY_ENTRY",entry],
            ["TESHI_CODEBUDDY_ARGS_JSON",serde_json::to_string(&agent_args).unwrap()],
            ["TESHI_CODEBUDDY_PROTOCOL_LOG",protocol_log],
            ["TESHI_CODEBUDDY_APPDATA",app_data],
            ["TESHI_CODEBUDDY_LOCALAPPDATA",local_app_data]
        ],
        "cols":100,
        "rows":30
    }});
    let mut actions = vec![
        vec![launch, text("Ready")],
        vec![key("3"), text("Welcome to AI Chat!"), key("Enter")],
    ];
    for turn in 1..=10 {
        actions.push(vec![
            json!({
                "action":"write",
                "text":format!(
                    "This is live conversation turn {turn}. Reply only with the concatenation of LIVE_REPLY_ and {turn:02}. Do not use tools or add explanation."
                )
            }),
            key("Enter"),
        ]);
        actions.push(vec![text(&format!("LIVE_REPLY_{turn:02}")), text("▸ ○")]);
    }
    actions.extend([
        vec![key("Escape")],
        vec![key("q")],
        vec![json!({"action":"expect_exit","code":0,"timeout_ms":10000})],
    ]);
    let steps = background
        .steps
        .iter()
        .chain(scenario.steps.iter())
        .collect::<Vec<_>>();
    assert_eq!(steps.len(), actions.len());
    let bindings = steps
        .into_iter()
        .zip(actions)
        .map(|(step, actions)| {
            json!({"line":step.line_number,"text":step.text,"confirmed":true,"actions":actions})
        })
        .collect::<Vec<_>>();
    std::fs::write(
        feature.with_extension("tui.json"),
        serde_json::to_vec_pretty(&json!({"version":1,"mode":"tui","steps":bindings})).unwrap(),
    )
    .unwrap();
    let output = Command::new(&binary)
        .args(["tui", &feature.to_string_lossy()])
        .env("TESHI_TUI_RUNNER", env!("CARGO_BIN_EXE_teshi-tui-runner"))
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}\nprotocol log: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        protocol_log.display()
    );
}

fn verify_wrapped_chat(source: &str, batches: &[(usize, usize)]) {
    let binary = std::fs::canonicalize(
        std::env::var("TESHI_SELF_TEST_BIN").expect("set TESHI_SELF_TEST_BIN"),
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("sample.feature"),
        "Feature: Wrapped chat fixture\n  Scenario: Ready\n    Given a sample step\n",
    )
    .unwrap();
    let feature = root.path().join("wrapped.feature");
    std::fs::write(&feature, source).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wrapped_chat_acp.py")
        .canonicalize()
        .unwrap();
    let python = std::env::var("TESHI_TEST_PYTHON").unwrap_or_else(|_| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    });
    let text = |value: &str| json!({"action":"expect_text","text":value,"visible":true,"timeout_ms":10000});
    let key = |value: &str| json!({"action":"key","key":value});
    let parsed = teshi_core::gherkin::parse_feature(source, feature.clone());
    let background = parsed.background.as_ref().unwrap();
    let scenario = parsed.all_scenarios()[0];
    let launch = json!({"action":"launch","config":{
        "program":binary,
        "args":[project,"--requirements-root",root.path().join("requirements")],
        "cwd":root.path(),
        "env":[
            ["APPDATA",root.path().join("appdata")],
            ["LOCALAPPDATA",root.path().join("localappdata")],
            ["XDG_CONFIG_HOME",root.path().join("config")],
            ["XDG_DATA_HOME",root.path().join("data")],
            ["TESHI_AGENT_BACKEND","acp"],
            ["TESHI_ACP_PROGRAM",python],
            ["TESHI_ACP_ARGS_JSON",serde_json::to_string(&vec![fixture]).unwrap()]
        ],
        "cols":100,
        "rows":30
    }});
    let long_prompt = |turn: usize| {
        json!({
            "action":"write",
            "text":format!(
                "Conversation turn {turn}: inspect the project context carefully and return the deterministic response marker after considering this deliberately long wrapped request."
            )
        })
    };
    let mut actions = vec![
        vec![launch, text("Ready")],
        vec![key("3"), text("Welcome to AI Chat!"), key("Enter")],
    ];
    for &(first, last) in batches {
        let mut batch = Vec::new();
        for turn in first..=last {
            batch.extend([
                long_prompt(turn),
                key("Enter"),
                text(&format!("WRAPPED_REPLY_{turn:02}")),
                text("│ ○ Conversation"),
            ]);
        }
        actions.push(batch);
        actions.push(vec![text(&format!("WRAPPED_REPLY_{last:02}"))]);
    }
    actions.extend([
        vec![key("Escape")],
        vec![key("q")],
        vec![json!({"action":"expect_exit","code":0,"timeout_ms":10000})],
    ]);
    let steps = background
        .steps
        .iter()
        .chain(scenario.steps.iter())
        .collect::<Vec<_>>();
    assert_eq!(steps.len(), actions.len());
    let bindings = steps
        .into_iter()
        .zip(actions)
        .map(|(step, actions)| {
            json!({"line":step.line_number,"text":step.text,"confirmed":true,"actions":actions})
        })
        .collect::<Vec<_>>();
    std::fs::write(
        feature.with_extension("tui.json"),
        serde_json::to_vec_pretty(&json!({"version":1,"mode":"tui","steps":bindings})).unwrap(),
    )
    .unwrap();
    let output = Command::new(&binary)
        .args(["tui", &feature.to_string_lossy()])
        .env("TESHI_TUI_RUNNER", env!("CARGO_BIN_EXE_teshi-tui-runner"))
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn acp_permission_allow_continues_the_turn() {
    verify_permission_prompt(
        include_str!("../../../features/en-US/tui_acp_permission_allow.feature"),
        true,
    );
}

#[test]
#[ignore = "requires a built Teshi CLI; set TESHI_SELF_TEST_BIN"]
fn acp_permission_reject_stops_the_turn() {
    verify_permission_prompt(
        include_str!("../../../features/en-US/tui_acp_permission_reject.feature"),
        false,
    );
}

/// Drives the TUI against an ACP fixture that requests permission mid-turn.
/// `allow` answers the prompt with the second option; otherwise it rejects.
/// The follow-up reply only streams when the decision reaches the agent, so
/// the allow path is the regression for "conversation dies after the first
/// message" (Manual policy without a PermissionHost).
fn verify_permission_prompt(source: &str, allow: bool) {
    let binary = std::fs::canonicalize(
        std::env::var("TESHI_SELF_TEST_BIN").expect("set TESHI_SELF_TEST_BIN"),
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("sample.feature"),
        "Feature: Permission chat fixture\n  Scenario: Ready\n    Given a sample step\n",
    )
    .unwrap();
    let feature = root.path().join("permission.feature");
    std::fs::write(&feature, source).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/permission_prompt_acp.py")
        .canonicalize()
        .unwrap();
    let python = std::env::var("TESHI_TEST_PYTHON").unwrap_or_else(|_| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    });
    let text = |value: &str| json!({"action":"expect_text","text":value,"visible":true,"timeout_ms":15000});
    let absent = |value: &str| json!({"action":"expect_text","text":value,"visible":false,"timeout_ms":5000});
    let key = |value: &str| json!({"action":"key","key":value});
    let parsed = teshi_core::gherkin::parse_feature(source, feature.clone());
    let background = parsed.background.as_ref().unwrap();
    let scenario = parsed.all_scenarios()[0];
    let launch = json!({"action":"launch","config":{
        "program":binary,
        "args":[project,"--requirements-root",root.path().join("requirements")],
        "cwd":root.path(),
        "env":[
            ["APPDATA",root.path().join("appdata")],
            ["LOCALAPPDATA",root.path().join("localappdata")],
            ["XDG_CONFIG_HOME",root.path().join("config")],
            ["XDG_DATA_HOME",root.path().join("data")],
            ["TESHI_AGENT_BACKEND","acp"],
            ["TESHI_ACP_PROGRAM",python],
            ["TESHI_ACP_ARGS_JSON",serde_json::to_string(&vec![fixture]).unwrap()]
        ],
        "cols":100,
        "rows":30
    }});
    let send_turn = || {
        vec![
            json!({"action":"write","text":"Please inspect the project and report back."}),
            key("Enter"),
        ]
    };
    let mut actions = vec![
        vec![launch, text("Ready")],
        vec![key("3"), text("Welcome to AI Chat!"), key("Enter")],
    ];
    if allow {
        actions.extend([
            send_turn(),
            vec![text("PERMISSION_BEFORE")],
            vec![text("ACP agent requests permission"), text("Allow once")],
            vec![key("Down")],
            vec![key("Enter")],
            vec![text("PERMISSION_AFTER_allow_always")],
            vec![key("Escape")],
            vec![key("q")],
            vec![json!({"action":"expect_exit","code":0,"timeout_ms":10000})],
        ]);
    } else {
        actions.extend([
            send_turn(),
            vec![
                text("PERMISSION_BEFORE"),
                text("ACP agent requests permission"),
            ],
            vec![key("Escape")],
            vec![absent("PERMISSION_AFTER")],
            vec![key("Escape")],
            vec![key("q")],
            vec![json!({"action":"expect_exit","code":0,"timeout_ms":10000})],
        ]);
    }
    let steps = background
        .steps
        .iter()
        .chain(scenario.steps.iter())
        .collect::<Vec<_>>();
    assert_eq!(steps.len(), actions.len());
    let bindings = steps
        .into_iter()
        .zip(actions)
        .map(|(step, actions)| {
            json!({"line":step.line_number,"text":step.text,"confirmed":true,"actions":actions})
        })
        .collect::<Vec<_>>();
    std::fs::write(
        feature.with_extension("tui.json"),
        serde_json::to_vec_pretty(&json!({"version":1,"mode":"tui","steps":bindings})).unwrap(),
    )
    .unwrap();
    let output = Command::new(&binary)
        .args(["tui", &feature.to_string_lossy()])
        .env("TESHI_TUI_RUNNER", env!("CARGO_BIN_EXE_teshi-tui-runner"))
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn verify(source: &str, command: &str) {
    let binary = std::fs::canonicalize(
        std::env::var("TESHI_SELF_TEST_BIN").expect("set TESHI_SELF_TEST_BIN"),
    )
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("sample.feature"),
        "Feature: Terminal fixture\n  Scenario: Sample scenario\n    Given a sample step\n",
    )
    .unwrap();
    let feature = root.path().join("self.feature");
    std::fs::write(&feature, source).unwrap();
    let text = |value: &str| json!({"action":"expect_text","text":value,"visible":true,"timeout_ms":10000});
    let key = |value: &str| json!({"action":"key","key":value});
    // r must select the native runner and consume real step and case events.
    std::fs::write(project.join("sample.tui.json"),serde_json::to_vec(&json!({"version":1,"mode":"tui","steps":[{
        "line":3,"text":"a sample step","confirmed":true,"actions":[{"action":"launch","config":{
            "program":std::env::current_exe().unwrap(),"args":["--exact","runner_child_fixture","--nocapture"],
            "env":[["TESHI_SELF_RUNNER_FIXTURE","1"]],"cols":80,"rows":24
        }},text("self-runner-ready")]
    }]})).unwrap()).unwrap();
    let launch_actions = vec![
        json!({"action":"launch","config":{
            "program":binary,"args":[project,"--requirements-root",root.path().join("requirements")],"cwd":root.path(),
            "env":[["APPDATA",root.path().join("appdata")],["LOCALAPPDATA",root.path().join("localappdata")],["XDG_CONFIG_HOME",root.path().join("config")],["XDG_DATA_HOME",root.path().join("data")]],
            "cols":140,"rows":40
        }}),
        text("Sample scenario"),
    ];
    let exit = || json!({"action":"expect_exit","code":0,"timeout_ms":10000});
    let scenario_actions = [
        vec![
            vec![text("Sample scenario")],
            vec![key("3")],
            vec![text("Welcome to AI Chat!")],
            vec![key("1")],
            vec![text("Sample scenario")],
            vec![key("q")],
            vec![exit()],
        ],
        vec![
            vec![text("Sample scenario")],
            vec![key("r")],
            vec![key("e"), text("Edit [Enter]")],
            vec![text("Status: Passed"), text("[passed] a sample step")],
            vec![
                key("Enter"),
                key("End"),
                json!({"action":"write","text":" updated"}),
                key("Enter"),
            ],
            vec![text("a sample step updated")],
            vec![key("s")],
            vec![text("a sample step updated")],
            vec![key("q")],
            vec![exit()],
        ],
    ];
    let parsed = teshi_core::gherkin::parse_feature(source, feature.clone());
    assert_eq!(parsed.scenario_count(), 2);
    let background = parsed.background.as_ref().unwrap();
    assert_eq!(background.steps.len(), 1);
    let binding = |step: &teshi_core::gherkin::BddStep, actions| json!({"line":step.line_number,"text":step.text,"confirmed":true,"actions":actions});
    let mut bindings = vec![binding(&background.steps[0], launch_actions)];
    for (scenario, actions) in parsed.all_scenarios().into_iter().zip(scenario_actions) {
        assert_eq!(scenario.steps.len(), actions.len());
        bindings.extend(
            scenario
                .steps
                .iter()
                .zip(actions)
                .map(|(step, actions)| binding(step, actions)),
        );
    }
    std::fs::write(
        feature.with_extension("tui.json"),
        serde_json::to_vec_pretty(&json!({"version":1,"mode":"tui","steps":bindings})).unwrap(),
    )
    .unwrap();
    let output = Command::new(&binary)
        .arg(command)
        .arg(&feature)
        .env("TESHI_TUI_RUNNER", env!("CARGO_BIN_EXE_teshi-tui-runner"))
        .env_remove("TESHI_RUNNER_CMD")
        .env_remove("TESHI_RUNNER_ARGS")
        .env_remove("TESHI_RUNNER_CWD")
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.last().unwrap()["passed"], 2);
    assert_eq!(
        std::fs::read_to_string(project.join("sample.feature")).unwrap(),
        "Feature: Terminal fixture\n  Scenario: Sample scenario\n    Given a sample step updated\n"
    );
}
