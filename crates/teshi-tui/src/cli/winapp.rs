use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::json;
use teshi_engine::{
    DaemonManifest, DaemonManifestExt, StepBinding, find_project_root, read_active_step,
    remove_daemon_manifest, send_sidecar_command_with_timeout, spawn_daemon_background,
};

use super::browser_endpoint::{read_cdp_endpoint, write_cdp_endpoint_from_rust};
use super::check::preflight_feature;
use super::replay_screenshots::{
    capture_and_save_screenshot, iso_now, load_or_create_index, save_index,
};
use super::{
    ShellCommand, TrayCommand, WinAppAttachArgs, WinAppCommand, WinAppExecuteArgs,
    WinAppLaunchArgs, WinAppReplayArgs, WinAppSelectorArgs, WinAppShellArgs, WinAppSnapshotArgs,
    WinAppTrayClickArgs, WinAppWaitWindowArgs,
};

/// Handles `teshi winapp ...` subcommands.
pub fn handle_winapp_command(action: &WinAppCommand) -> Result<()> {
    let result = handle_winapp_inner(action);
    if let Err(error) = &result
        && let Ok(response) = serde_json::from_str::<serde_json::Value>(&error.to_string())
    {
        print_json_response(response)?;
    }
    result
}

fn handle_winapp_inner(action: &WinAppCommand) -> Result<()> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let project_root = match action {
        WinAppCommand::Replay(args) => replay_project_root(&cwd, args.feature.as_deref()),
        _ => cwd.clone(),
    };
    std::fs::create_dir_all(project_root.join(".teshi")).ok();
    let visual = match action {
        WinAppCommand::Screenshot(args) => Some(json!({
            "cmd": "element_screenshot", "selector": args.selector, "out": args.out
        })),
        WinAppCommand::AssertScreenshot(args) => Some(json!({
            "cmd": "assert_screenshot", "selector": args.selector, "baseline": args.baseline,
            "diff_out": args.diff_out, "pixel_tolerance": args.pixel_tolerance
        })),
        _ => None,
    };
    if let Some(command) = visual {
        let response = ensure_winapp_sidecar(&project_root, false)
            .and_then(|()| {
                send_winapp_command(&project_root, command.clone(), Duration::from_secs(60))
            })
            .unwrap_or_else(|err| transport_failure(&err.to_string()));
        print_json_response(response.clone())?;
        return ensure_ok(&response);
    }
    let permission_error = match action {
        WinAppCommand::Execute(args) => {
            input_permission_error(&args.action, &args.mode, args.allow_interactive_input)
        }
        WinAppCommand::Shell(args) => {
            let ShellCommand::Tray(tray) = &args.command;
            match &tray.command {
                TrayCommand::Click(args) => input_permission_error(
                    "shell_tray_click",
                    &args.mode,
                    args.allow_interactive_input,
                ),
                TrayCommand::List => None,
            }
        }
        _ => None,
    };
    if let Some(error) = permission_error {
        print_json_response(error.clone())?;
        return ensure_ok(&error);
    }
    // Reject input permission failures before starting a sidecar or target.
    let elevated = matches!(&action, WinAppCommand::Launch(args) if args.elevated);
    if !matches!(action, WinAppCommand::Replay(_)) {
        ensure_winapp_sidecar(&project_root, elevated)?;
    }
    match action {
        WinAppCommand::VisualPlan {
            file,
            mode,
            allow_interactive_input,
        } => {
            let mut command: serde_json::Value = serde_json::from_slice(&std::fs::read(file)?)?;
            let object = command
                .as_object_mut()
                .context("plan must be a JSON object")?;
            object.insert("cmd".into(), json!("visual_plan"));
            object.insert("mode".into(), json!(mode));
            object.insert(
                "allow_interactive_input".into(),
                json!(allow_interactive_input),
            );
            object.insert("timeout_ms".into(), json!(60000));
            let response = send_winapp_command(&project_root, command, Duration::from_secs(60))?;
            print_json_response(response.clone())?;
            ensure_ok(&response)
        }
        WinAppCommand::Instance { file } | WinAppCommand::ExternalCheck { file } => {
            let mut command: serde_json::Value = serde_json::from_slice(&std::fs::read(file)?)?;
            command
                .as_object_mut()
                .context("instance request must be a JSON object")?
                .insert(
                    "cmd".into(),
                    json!(if matches!(action, WinAppCommand::Instance { .. }) {
                        "instance"
                    } else {
                        "external_check"
                    }),
                );
            command["timeout_ms"] = json!(45000);
            let response = send_winapp_command(&project_root, command, Duration::from_secs(45))?;
            print_json_response(response.clone())?;
            ensure_ok(&response)
        }
        WinAppCommand::Health => {
            let response = send_winapp_command(
                &project_root,
                json!({"cmd":"health"}),
                Duration::from_secs(2),
            )?;
            print_json_response(response.clone())?;
            ensure_ok(&response)
        }
        WinAppCommand::Cancel { request_id } => {
            let response = send_winapp_command(
                &project_root,
                json!({"cmd":"cancel","cancel_request_id":request_id}),
                Duration::from_secs(2),
            )?;
            print_json_response(response.clone())?;
            ensure_ok(&response)
        }
        WinAppCommand::ListWindows => list_windows(&project_root),
        WinAppCommand::Status => status(&project_root),
        WinAppCommand::Attach(args) => attach(&project_root, args),
        WinAppCommand::Launch(args) => launch(&project_root, args),
        WinAppCommand::Snapshot(args) => snapshot(&project_root, args),
        WinAppCommand::Highlight(args) => highlight(&project_root, args),
        WinAppCommand::ClearHighlight => clear_highlight(&project_root),
        WinAppCommand::Execute(args) => execute(&project_root, args),
        WinAppCommand::Replay(args) => replay(&project_root, args),
        WinAppCommand::Shell(args) => shell(&project_root, args),
        WinAppCommand::WaitWindow(args) => wait_window(&project_root, args),
        WinAppCommand::Screenshot(_) | WinAppCommand::AssertScreenshot(_) => unreachable!(),
    }
}

fn replay_project_root(cwd: &Path, feature: Option<&str>) -> std::path::PathBuf {
    let Some(feature) = feature else {
        return cwd.to_path_buf();
    };
    let path = Path::new(feature);
    if !path.is_absolute() {
        return cwd.to_path_buf();
    }
    find_project_root(Some(path))
        .or_else(|| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| cwd.to_path_buf())
}

fn list_windows(project_root: &Path) -> Result<()> {
    let response = send_winapp_command(
        project_root,
        json!({ "cmd": "list_windows", "request_id": "winapp-list-windows" }),
        Duration::from_secs(10),
    )?;
    print_json_response(response.clone())?;
    ensure_ok(&response)
}

fn status(project_root: &Path) -> Result<()> {
    let response = send_winapp_command(
        project_root,
        json!({ "cmd": "get_status", "request_id": "winapp-status" }),
        Duration::from_secs(10),
    )?;
    print_json_response(response.clone())?;
    ensure_ok(&response)
}

fn attach(project_root: &Path, args: &WinAppAttachArgs) -> Result<()> {
    let response = send_winapp_command(
        project_root,
        json!({
            "cmd": "attach_window",
            "request_id": "winapp-attach",
            "hwnd": args.hwnd,
            "title": args.title,
            "pid": args.pid,
            "process_name": args.process_name,
        }),
        Duration::from_secs(15),
    )?;
    print_json_response(response.clone())?;
    ensure_ok(&response)
}

fn launch(project_root: &Path, args: &WinAppLaunchArgs) -> Result<()> {
    let response = send_winapp_command(
        project_root,
        json!({
            "cmd": "launch_app",
            "request_id": "winapp-launch",
            "path": args.path,
            "args": args.args,
            "title": args.title,
            "timeout_ms": args.timeout_ms,
            "elevated": args.elevated,
        }),
        command_timeout_for_ms(args.timeout_ms),
    )?;
    print_json_response(response.clone())?;
    ensure_ok(&response)
}

fn snapshot(project_root: &Path, args: &WinAppSnapshotArgs) -> Result<()> {
    let response = send_winapp_command(
        project_root,
        json!({ "cmd": "get_ui_snapshot", "request_id": "winapp-snapshot" }),
        Duration::from_millis(args.timeout_ms),
    )?;
    print_json_response(response.clone())?;
    ensure_ok(&response)
}

fn highlight(project_root: &Path, args: &WinAppSelectorArgs) -> Result<()> {
    let response = send_winapp_command(
        project_root,
        json!({
            "cmd": "highlight_selector",
            "request_id": "winapp-highlight",
            "selector": args.selector
        }),
        Duration::from_secs(20),
    )?;
    print_json_response(response.clone())?;
    ensure_ok(&response)
}

fn clear_highlight(project_root: &Path) -> Result<()> {
    let response = send_winapp_command(
        project_root,
        json!({ "cmd": "clear_highlight", "request_id": "winapp-clear-highlight" }),
        Duration::from_secs(10),
    )?;
    print_json_response(response.clone())?;
    ensure_ok(&response)
}

fn execute(project_root: &Path, args: &WinAppExecuteArgs) -> Result<()> {
    let timeout = command_timeout_for_ms(args.timeout_ms);
    let response = execute_locator(
        project_root,
        &args.selector,
        &args.action,
        args.value_arg.as_deref(),
        args.timeout_ms,
        timeout,
        "winapp-execute",
        &args.mode,
        args.allow_interactive_input,
    )?;
    if args.action == "assert_screenshot" {
        print_json_response(response.clone())?;
        return ensure_ok(&response);
    }
    print_json_response(response.clone())?;
    ensure_ok(&response)
}

fn shell(project_root: &Path, args: &WinAppShellArgs) -> Result<()> {
    match &args.command {
        ShellCommand::Tray(tray) => match &tray.command {
            TrayCommand::List => {
                let response = send_winapp_command(
                    project_root,
                    json!({ "cmd": "shell_tray_list", "request_id": "winapp-shell-tray-list" }),
                    Duration::from_secs(15),
                )?;
                print_json_response(response)
            }
            TrayCommand::Click(click) => {
                let response = shell_tray_click(project_root, click)?;
                print_json_response(response.clone())?;
                ensure_ok(&response)
            }
        },
    }
}

fn shell_tray_click(project_root: &Path, args: &WinAppTrayClickArgs) -> Result<serde_json::Value> {
    if let Some(error) =
        input_permission_error("shell_tray_click", &args.mode, args.allow_interactive_input)
    {
        return Ok(error);
    }
    send_winapp_command(
        project_root,
        json!({
            "cmd": "shell_tray_click",
            "request_id": "winapp-shell-tray-click",
            "name": args.name,
            "pid": args.pid,
            "button": args.button,
            "count": args.count,
            "mode": args.mode,
            "allow_interactive_input": args.allow_interactive_input,
            "timeout_ms": args.timeout_ms,
        }),
        command_timeout_for_ms(args.timeout_ms),
    )
}

fn wait_window(project_root: &Path, args: &WinAppWaitWindowArgs) -> Result<()> {
    let state = if args.hidden { "hidden" } else { "visible" };
    let response = send_winapp_command(
        project_root,
        json!({
            "cmd": "wait_window",
            "request_id": "winapp-wait-window",
            "pid": args.pid,
            "state": state,
            "timeout_ms": args.timeout_ms,
        }),
        command_timeout_for_ms(args.timeout_ms),
    )?;
    print_json_response(response.clone())?;
    ensure_ok(&response)
}

fn replay(project_root: &Path, args: &WinAppReplayArgs) -> Result<()> {
    use super::replay_protocol::{PLATFORM_UNSUPPORTED, TARGET_UNAVAILABLE};
    let Some(case_id) = args.emit_runner_protocol.as_deref() else {
        return replay_steps(project_root, args, None);
    };
    let emitter = super::replay_protocol::CaseEmitter::new(case_id);
    // Target availability is settled before any step runs: a missing
    // platform, service or window blocks the scenario instead of failing it.
    if !cfg!(windows) {
        emitter.blocked(
            PLATFORM_UNSUPPORTED,
            "WinApp replay drives Windows UI Automation and is Windows-only",
        );
        return Ok(());
    }
    let feature = resolve_feature(project_root, args)?;
    let target = match resolve_winapp_target(project_root, &feature, args.launch.as_deref()) {
        Ok(target) => target,
        Err(error) => {
            emitter.target_blocked("winapp", error.code, &error.message);
            emitter.blocked(TARGET_UNAVAILABLE, &error.message);
            return Ok(());
        }
    };
    if !args.dry_run
        && let Err(error) = prepare_winapp_target(project_root, target.as_ref(), Some(&emitter))
    {
        emitter.target_blocked("winapp", error.code, &error.message);
        emitter.blocked(TARGET_UNAVAILABLE, &error.message);
        return Ok(());
    }
    let result = replay_steps_with_feature(project_root, args, Some(&emitter), &feature);
    emitter.finish(&result);
    Ok(())
}

fn replay_steps(
    project_root: &Path,
    args: &WinAppReplayArgs,
    emitter: Option<&super::replay_protocol::CaseEmitter>,
) -> Result<()> {
    let feature = resolve_feature(project_root, args)?;
    if !args.dry_run {
        let target = resolve_winapp_target(project_root, &feature, args.launch.as_deref())
            .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?;
        prepare_winapp_target(project_root, target.as_ref(), emitter)
            .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?;
    }
    replay_steps_with_feature(project_root, args, emitter, &feature)
}

fn resolve_feature(project_root: &Path, args: &WinAppReplayArgs) -> Result<String> {
    Ok(match args.feature.as_deref() {
        Some(feature) => feature.replace('\\', "/"),
        None => read_active_step(project_root)?.feature_relative_path,
    })
}

fn replay_steps_with_feature(
    project_root: &Path,
    args: &WinAppReplayArgs,
    emitter: Option<&super::replay_protocol::CaseEmitter>,
    feature: &str,
) -> Result<()> {
    let protocol = emitter.is_some();
    let print_json_response = |value: serde_json::Value| -> Result<()> {
        super::replay_protocol::progress(protocol, &serde_json::to_string_pretty(&value)?);
        Ok(())
    };
    preflight_feature(project_root, feature)?;
    let plan = super::replay_plan::build_selected(
        project_root,
        feature,
        args.until_line,
        args.scenario.as_deref(),
    )?;
    let scope = if plan.omitted > 0 { "partial" } else { "full" };
    let coverage = json!({"scope":scope,"until_line":args.until_line,
        "executable_steps":plan.steps.len(),"omitted_steps":plan.omitted,"gaps":plan.gaps});
    print_json_response(
        json!({"event":"replay_plan","feature":feature,"dry_run":args.dry_run,"coverage":coverage}),
    )?;
    if !plan.gaps.is_empty() {
        let status = if plan.gaps.iter().any(|gap| gap["status"] == "unbound") {
            "unbound"
        } else if plan.gaps.iter().any(|gap| gap["status"] == "blocked") {
            "blocked"
        } else {
            "manual"
        };
        print_json_response(
            json!({"event":"replay_rejected","status":status,"coverage":coverage}),
        )?;
        return Err(anyhow!(
            "replay {status}: selected Feature coverage is incomplete"
        ));
    }
    if args.dry_run {
        for step in &plan.steps {
            print_json_response(json!({"event":"planned_step","step_id":step.step_id,
                "scenario":step.scenario,"step_line":step.binding.step_line,"step":step.binding.step_text,
                "action":step.binding.primary.action,"target":step.binding.primary.value,
                "value_arg":step.binding.primary.value_arg}))?;
        }
        return Ok(());
    }
    let external_dir = args.evidence_dir.clone();
    let mut run = if external_dir.is_some() {
        None
    } else {
        Some(super::replay_screenshots::create_cli_replay(
            project_root,
            feature,
            args.scenario.as_deref(),
        )?)
    };
    let run_id = run
        .as_ref()
        .map(|run| run.id().to_string())
        .unwrap_or_else(|| {
            external_dir
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    macro_rules! record {
        ($value:expr) => {{
            let value = $value;
            if let Some(run) = run.as_mut() {
                run.record(value)?;
            }
        }};
    }
    if let Some(program) = args.launch.as_deref() {
        record!(teshi_engine::replay_evidence::target_executable_identity(
            Path::new(program),
        ));
    }
    record!(json!({"type":"coverage","coverage":coverage,"screenshot_policy":"required"}));
    print_json_response(json!({"event":"replay_started","run_id":run_id,"coverage":coverage}))?;
    for planned in &plan.steps {
        record!(
            json!({"type":"planned_step","step_id":planned.step_id,"case_id":planned.case_id,
            "feature":feature,"scenario":planned.scenario,"step_line":planned.binding.step_line,
            "screenshot_required":true,"source":"windows_window"})
        );
    }
    let directory = external_dir
        .clone()
        .or_else(|| run.as_ref().map(|run| run.directory().to_path_buf()))
        .unwrap_or_else(|| project_root.join(".teshi/logs/replay-screenshots"));
    let mut index = load_or_create_index(&directory, feature);
    save_index(&directory, &index)?;
    let mut failed_line = None;
    let mut capture_failures = 0usize;
    let result: Result<()> = (|| {
        for step in &plan.steps {
            if let Some(error) = input_permission_error(
                &step.binding.primary.action,
                &args.mode,
                args.allow_interactive_input,
            ) {
                record!(json!({"type":"preflight_error","response":error}));
                print_json_response(error.clone())?;
                ensure_ok(&error)?;
            }
        }
        ensure_winapp_sidecar(project_root, false)?;
        for planned in &plan.steps {
            let step = &planned.binding;
            if !args.non_interactive && !args.yes {
                prompt_continue(step)?;
            }
            record!(json!({"type":"start_step","step_id":planned.step_id,
                "case_id":planned.case_id,"scenario":planned.scenario,"step_line":step.step_line,"text":step.step_text}));
            if let Some(emitter) = emitter {
                emitter.start_step(
                    step.step_line,
                    &format!("{} {}", step.step_keyword, step.step_text),
                );
            }
            let capture_frame = || {
                read_cdp_endpoint(project_root).and_then(|endpoint| {
                    capture_and_save_screenshot(
                        &endpoint.ws_url,
                        project_root,
                        &teshi_engine::sanitize_feature_path(feature),
                        step.step_line,
                        &step.step_keyword,
                        &step.step_text,
                        &directory,
                    )
                })
            };
            let closing = step.primary.capture_before
                || teshi_engine::replay_evidence::capture_before_action(&step.primary.action);
            let before_capture = closing.then(capture_frame);
            // Cold-launched GPUI windows often need more than 5s before the first
            // UIA query completes; keep the transport budget above the deadline.
            let response = execute_locator(
                project_root,
                &step.primary.value,
                &step.primary.action,
                step.primary.value_arg.as_deref(),
                15_000,
                command_timeout_for_ms(15_000),
                &if step.primary.action == "assert_screenshot" {
                    format!(
                        "{}-L{}",
                        teshi_engine::sanitize_feature_path(feature),
                        step.step_line
                    )
                } else {
                    format!("{run_id}:{}", planned.step_id)
                },
                &args.mode,
                args.allow_interactive_input,
            )
            .unwrap_or_else(|error| transport_failure(&error.to_string()));
            let passed = ensure_ok(&response).is_ok();
            // The action result is durable before any potentially failing capture.
            record!(json!({"type":"step_result","step_id":planned.step_id,
                "case_id":planned.case_id,"scenario":planned.scenario,"step_line":step.step_line,
                "status":if passed {"passed"} else {"failed"},"response":response}));
            print_json_response(response.clone())?;
            let capture = before_capture.unwrap_or_else(capture_frame);
            // Protocol case ids belong to the TUI (`f0:s0`); plan ids
            // (`scenario:7:0`) are only for the local replay plan. Screenshots
            // must use the protocol id or the TUI demotes them to Logs and
            // never shows a Screenshots tab.
            let (case_id, step_id) = match emitter {
                Some(emitter) => (
                    emitter.case_id().to_string(),
                    emitter.step_id(step.step_line),
                ),
                None => (planned.case_id.clone(), planned.step_id.clone()),
            };
            let mut event = json!({"type":"step_screenshot","step_id":step_id,
                "case_id":case_id,"scenario":planned.scenario,"feature":feature,
                "step_line":step.step_line,"step_keyword":step.step_keyword,"step_text":step.step_text,
                "status":if passed {"passed"} else {"failed"}, "source":"windows_window",
                "capture_timing":if closing {"before_close"} else {"after_step"}});
            match capture {
                Ok(entry) => {
                    event["capture_status"] = json!("available");
                    event["screenshot_file"] = json!(entry.screenshot_file);
                    event["captured_at"] = json!(entry.captured_at);
                }
                Err(error) => {
                    capture_failures += 1;
                    event["capture_status"] = json!("failed");
                    event["capture_error"] = json!(error.to_string());
                    event["message"] = json!(error.to_string());
                }
            }
            if let Some(run) = run.as_mut() {
                run.record(event)?;
            } else if protocol {
                // The owning TUI process records this capture in its shared run.
                println!("{}", serde_json::to_string(&event)?);
            }
            if let Some(emitter) = emitter {
                let message = ensure_ok(&response).err().map(|error| error.to_string());
                emitter.end_step(step.step_line, passed, message.as_deref());
            }
            if !passed {
                failed_line = Some(step.step_line);
                return Err(anyhow!(
                    "replay failed at line {}: {}",
                    step.step_line,
                    response["error"]
                ));
            }
        }
        Ok(())
    })();
    let status = if result.is_err() {
        if failed_line.is_some() {
            "failed"
        } else {
            "blocked"
        }
    } else if plan.omitted > 0 {
        "partial"
    } else {
        "passed"
    };
    index = load_or_create_index(&directory, feature);
    index.status = status.into();
    index.completed_at = Some(iso_now());
    save_index(&directory, &index)?;
    record!(
        json!({"type":"end_run","status":status,"failed":usize::from(result.is_err()),
        "capture_failures":capture_failures,"error":result.as_ref().err().map(ToString::to_string)})
    );
    let (run_status, acceptance, evidence) = match run.as_ref() {
        Some(run) => (
            run.snapshot().status.clone(),
            run.snapshot().acceptance_status.clone(),
            json!(run.snapshot().evidence),
        ),
        None => (status.to_string(), "unverified".to_string(), json!(null)),
    };
    print_json_response(
        json!({"event":"replay_complete","run_id":run_id,"status":run_status,"test_status":status,"acceptance_status":acceptance,"evidence":evidence,
        "coverage":coverage,"screenshot_policy":"required","capture_failures":capture_failures,
        "screenshot_dir":directory,"screenshots":index.steps.len(),"failed_line":failed_line}),
    )?;
    result?;
    if let Some(run) = run.as_ref() {
        run.ensure_accepted()?;
    }
    Ok(())
}

/// Feature-level WinApp target resolved from the binding and its profile.
struct WinappTarget {
    profile: Option<String>,
    launch: Option<PathBuf>,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: std::collections::HashMap<String, String>,
    attach: Option<teshi_engine::StepBindingAttach>,
    title: Option<String>,
}

struct TargetPrepError {
    code: &'static str,
    message: String,
}

fn target_error(code: &'static str, message: impl Into<String>) -> TargetPrepError {
    TargetPrepError {
        code,
        message: message.into(),
    }
}

fn resolve_winapp_target(
    project_root: &Path,
    feature: &str,
    cli_launch: Option<&str>,
) -> Result<Option<WinappTarget>, TargetPrepError> {
    let binding = teshi_engine::step_binding_target(project_root, feature).map_err(|error| {
        target_error(
            "target_binding_invalid",
            format!("read step bindings: {error:#}"),
        )
    })?;
    let Some(binding) = binding else {
        return Ok(cli_launch.map(|path| WinappTarget {
            profile: None,
            launch: Some(PathBuf::from(path)),
            args: Vec::new(),
            cwd: None,
            env: std::collections::HashMap::new(),
            attach: None,
            title: None,
        }));
    };
    let kind = if binding.kind.trim().is_empty() {
        "winapp".to_string()
    } else {
        binding.kind.clone()
    };
    if !kind.eq_ignore_ascii_case("winapp") {
        return Err(target_error(
            "target_kind_mismatch",
            format!("feature target is `{kind}`, but this is a WinApp replay"),
        ));
    }
    let profile = match binding.profile.as_deref() {
        Some(name) => {
            let profile = teshi_engine::load_target_profile(project_root, name)
                .map_err(|error| {
                    target_error(
                        "target_profile_invalid",
                        format!("read target profile `{name}`: {error}"),
                    )
                })?
                .ok_or_else(|| {
                    target_error(
                        "target_profile_missing",
                        format!(
                            "target profile `{name}` is missing from teshi.toml; add [targets.{name}]"
                        ),
                    )
                })?;
            if !profile.kind.trim().is_empty() && !profile.is_winapp() {
                return Err(target_error(
                    "target_kind_mismatch",
                    format!(
                        "target profile `{name}` has kind `{}`, expected winapp",
                        profile.kind
                    ),
                ));
            }
            Some(profile)
        }
        None => None,
    };
    let launch = cli_launch
        .map(PathBuf::from)
        .or_else(|| profile.as_ref().and_then(|profile| profile.launch.clone()));
    let attach = binding
        .attach
        .clone()
        .or_else(|| profile.as_ref().and_then(|profile| profile.attach.clone()));
    let args = profile
        .as_ref()
        .map(|profile| profile.args.clone())
        .unwrap_or_default();
    let cwd = profile.as_ref().and_then(|profile| profile.cwd.clone());
    let env = profile
        .as_ref()
        .map(|profile| profile.env.clone())
        .unwrap_or_default();
    let title = attach.as_ref().and_then(|attach| attach.title.clone());
    Ok(Some(WinappTarget {
        profile: binding.profile,
        launch,
        args,
        cwd,
        env,
        attach,
        title,
    }))
}

fn prepare_winapp_target(
    project_root: &Path,
    target: Option<&WinappTarget>,
    emitter: Option<&super::replay_protocol::CaseEmitter>,
) -> Result<(), TargetPrepError> {
    let action = if target.and_then(|target| target.launch.as_ref()).is_some() {
        "launch"
    } else {
        "attach"
    };
    if let Some(emitter) = emitter {
        emitter.target_preparing(
            "winapp",
            target.and_then(|target| target.profile.as_deref()),
            action,
        );
    }
    ensure_winapp_sidecar(project_root, false)
        .map_err(|error| target_error("target_unavailable", format!("{error:#}")))?;
    // A stale saved HWND must not block profile launch/attach: treat identity
    // loss (or an unrelated attached window) as needing preparation.
    match winapp_target_identity(project_root) {
        Ok(Some(identity)) if identity_matches_target(&identity, target) => {
            if let Some(emitter) = emitter {
                emitter.target_ready(
                    "winapp",
                    target.and_then(|target| target.profile.as_deref()),
                    identity.title.as_deref(),
                    identity.process.as_deref(),
                );
            }
            return Ok(());
        }
        Ok(Some(_) | None) => {}
        Err(error) if is_recoverable_target_loss(&error) => {}
        Err(error) => {
            return Err(target_error("target_unavailable", format!("{error:#}")));
        }
    }
    let Some(target) = target else {
        return Err(target_error(
            "target_unavailable",
            "no target configured; add a feature target profile or pass --launch",
        ));
    };
    if let Some(launch) = &target.launch {
        let response = send_winapp_command(
            project_root,
            json!({
                "cmd": "launch_app",
                "request_id": "winapp-replay-launch",
                "path": launch.display().to_string(),
                "args": &target.args,
                "cwd": target.cwd.as_ref().map(|path| path.display().to_string()),
                "env": &target.env,
                "title": target.title.as_deref(),
                "timeout_ms": 15000,
                "elevated": false
            }),
            command_timeout_for_ms(15_000),
        )
        .map_err(|error| target_error("target_launch_failed", format!("{error:#}")))?;
        ensure_ok(&response)
            .map_err(|error| target_error("target_launch_failed", format!("{error:#}")))?;
        // Freshly launched windows need a beat before UIA trees are queryable.
        std::thread::sleep(Duration::from_secs(2));
    } else if let Some(attach) = &target.attach
        && !attach.is_empty()
    {
        let response = send_winapp_command(
            project_root,
            json!({
                "cmd": "attach_window",
                "request_id": "winapp-replay-attach",
                "hwnd": attach.hwnd,
                "title": attach.title,
                "pid": attach.pid,
                "process_name": attach.process_name
            }),
            Duration::from_secs(15),
        )
        .map_err(|error| target_error("target_attach_failed", format!("{error:#}")))?;
        ensure_ok(&response)
            .map_err(|error| target_error("target_attach_failed", format!("{error:#}")))?;
    } else {
        return Err(target_error(
            "target_unavailable",
            "no WinApp window attached; add an attach selector or launch command to the target profile",
        ));
    }
    let identity = winapp_target_identity(project_root)
        .map_err(|error| target_error("target_unavailable", format!("{error:#}")))?
        .ok_or_else(|| {
            target_error(
                "target_unavailable",
                "target preparation finished without an attached window",
            )
        })?;
    if let Some(emitter) = emitter {
        emitter.target_ready(
            "winapp",
            target.profile.as_deref(),
            identity.title.as_deref(),
            identity.process.as_deref(),
        );
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct AttachedTargetIdentity {
    hwnd: Option<u64>,
    pid: Option<u32>,
    title: Option<String>,
    process: Option<String>,
}

fn identity_matches_target(
    identity: &AttachedTargetIdentity,
    target: Option<&WinappTarget>,
) -> bool {
    let Some(attach) = target.and_then(|target| target.attach.as_ref()) else {
        return true;
    };
    if let Some(hwnd) = attach.hwnd
        && identity.hwnd != Some(hwnd)
    {
        return false;
    }
    if let Some(pid) = attach.pid
        && identity.pid != Some(pid)
    {
        return false;
    }
    if let Some(expected) = attach
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let expected_lower = expected.to_lowercase();
        if !identity
            .title
            .as_deref()
            .is_some_and(|title| title.to_lowercase().contains(&expected_lower))
        {
            return false;
        }
    }
    if let Some(expected) = attach
        .process_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && let Some(actual) = identity.process.as_deref()
    {
        let expected_lower = expected.to_lowercase();
        if !actual.to_lowercase().contains(&expected_lower) {
            return false;
        }
    }
    true
}

fn winapp_target_identity(project_root: &Path) -> Result<Option<AttachedTargetIdentity>> {
    let target = send_winapp_command(
        project_root,
        json!({ "cmd": "get_target", "request_id": "winapp-get-target" }),
        Duration::from_secs(10),
    )?;
    if target.get("ok") == Some(&serde_json::Value::Bool(false))
        && is_recoverable_target_loss_value(&target)
    {
        return Ok(None);
    }
    ensure_ok(&target)?;
    if !target
        .get("target")
        .and_then(|target| target.get("attached"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let hwnd = target["target"]["hwnd"].as_u64();
    let pid = target["target"]["pid"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok());
    let title = target["target"]["title"].as_str().map(str::to_string);
    let process = target["target"]["process_name"]
        .as_str()
        .or_else(|| target["target"]["process"].as_str())
        .map(str::to_string);
    Ok(Some(AttachedTargetIdentity {
        hwnd,
        pid,
        title,
        process,
    }))
}

fn is_recoverable_target_loss(error: &anyhow::Error) -> bool {
    let text = error.to_string();
    text.contains("Saved target exited or changed identity")
        || text.contains("Saved target identity changed")
        || text.contains("\"code\":\"target_invalid\"")
        || text.contains("No verified target")
}

fn is_recoverable_target_loss_value(value: &serde_json::Value) -> bool {
    value["code"].as_str() == Some("target_invalid")
        || value["error"].as_str().is_some_and(|message| {
            message.contains("Saved target exited or changed identity")
                || message.contains("Saved target identity changed")
        })
}

fn transport_failure(error: &str) -> serde_json::Value {
    if let Ok(response) = serde_json::from_str::<serde_json::Value>(error)
        && response["ok"] == false
    {
        return response;
    }
    let stage = if error.contains("stage=handshake") {
        "handshake"
    } else if error.contains("stage=connect") {
        "connect"
    } else if error.contains("stage=send") {
        "send"
    } else {
        "awaiting_response"
    };
    let code =
        if error.contains("timed out") || error.contains("timeout") || error.contains("10060") {
            "command_timeout"
        } else {
            "transport_failed"
        };
    json!({"ok":false,"code":code,"stage":stage,"error":error,"business_result":"unverified","retry_safe":false})
}

fn prompt_continue(step: &StepBinding) -> Result<()> {
    eprint!(
        "About to run L{}: {} {}. Press Enter to continue, or Ctrl+C to stop.",
        step.step_line, step.step_keyword, step.step_text
    );
    io::stderr().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(())
}

fn command_timeout_for_ms(timeout_ms: u64) -> Duration {
    let secs = timeout_ms.div_ceil(1000).saturating_add(5);
    Duration::from_secs(secs.max(15))
}

#[allow(clippy::too_many_arguments)]
fn execute_locator(
    project_root: &Path,
    selector: &str,
    action: &str,
    value: Option<&str>,
    timeout_ms: u64,
    sidecar_timeout: Duration,
    request_id: &str,
    mode: &str,
    allow_interactive_input: bool,
) -> Result<serde_json::Value> {
    if let Some(error) = input_permission_error(action, mode, allow_interactive_input) {
        return Ok(error);
    }
    let mut command = json!({
        "cmd": "execute_locator",
        "request_id": request_id,
        "selector": selector,
        "action": action,
        "value": value,
        "timeout_ms": timeout_ms,
        "mode": mode,
        "allow_interactive_input": allow_interactive_input,
    });
    if action == "assert_screenshot" {
        command["pixel_tolerance"] = json!(8);
        command["diff_out"] = json!(
            project_root
                .join(".teshi")
                .join("artifacts")
                .join("visual")
                .join(format!("{request_id}-diff.png"))
        );
    }
    send_winapp_command(project_root, command, sidecar_timeout)
}

fn send_winapp_command(
    project_root: &Path,
    command: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value> {
    let endpoint = read_cdp_endpoint(project_root)?;
    if endpoint.mode != "winapp" {
        ensure_winapp_sidecar(project_root, false)?;
    }
    let endpoint = read_cdp_endpoint(project_root)?;
    if endpoint.mode != "winapp" {
        return Err(anyhow!(
            "Teshi WinApp service is not active for this project (current mode: {:?})",
            endpoint.mode
        ));
    }
    let mut command = command;
    if command.get("timeout_ms").is_none() {
        command["timeout_ms"] = json!(timeout.as_millis().saturating_sub(1000).max(1));
    }
    // Loss of a reply does not imply loss of effects. Recover discovery only
    // before dispatch; never automatically replay an uncertain mutation.
    Ok(
        send_sidecar_command_with_timeout(&endpoint.ws_url, command, timeout).unwrap_or_else(
            |err| {
                let mut response = transport_failure(&err);
                response["timeout_ms"] = json!(timeout.as_millis());
                response
            },
        ),
    )
}

fn ensure_winapp_sidecar(project_root: &Path, elevated: bool) -> Result<()> {
    if !elevated && winapp_endpoint_is_healthy(project_root)? {
        return Ok(());
    }
    let port = ensure_daemon_for_project(project_root)?;
    let url = format!("http://127.0.0.1:{port}/api/v1/browser/start");
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(120))
        .build()
        .context("create loopback daemon client")?;
    let body = if elevated {
        json!({"mode": "winapp", "elevated": true})
    } else {
        json!({"mode": "winapp"})
    };
    let response = client
        .post(url)
        .json(&body)
        .send()
        .context("ask Teshi daemon to start WinApp")?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().unwrap_or_default();
        return Err(anyhow!(
            "Failed to start Teshi WinApp service through the Teshi daemon ({status}). {detail}"
        ));
    }
    let payload: serde_json::Value = response
        .json()
        .context("decode daemon WinApp start response")?;
    let ws_url = payload
        .get("ws_url")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("daemon did not return WinApp WebSocket URL"))?;
    write_cdp_endpoint_from_rust(project_root, ws_url, "winapp", "winapp://detached")?;
    if winapp_endpoint_is_healthy(project_root)? {
        Ok(())
    } else {
        Err(anyhow!(
            json!({"ok":false,"code":"stale_endpoint",
            "error":"Daemon returned an unreachable WinApp endpoint"})
            .to_string()
        ))
    }
}

fn winapp_endpoint_is_healthy(project_root: &Path) -> Result<bool> {
    let Ok(endpoint) = read_cdp_endpoint(project_root) else {
        return Ok(false);
    };
    if endpoint.mode != "winapp" {
        return Ok(false);
    }
    let response = match send_sidecar_command_with_timeout(
        &endpoint.ws_url,
        json!({"cmd":"health","request_id":"winapp-health"}),
        Duration::from_secs(2),
    ) {
        Ok(response) => response,
        Err(error) if error.contains("stage=connect") => return Ok(false),
        Err(error) => {
            let response = transport_failure(&error);
            return Err(anyhow!(serde_json::to_string(&response)?));
        }
    };
    if response["ok"] != true {
        return Err(anyhow!(serde_json::to_string(&response)?));
    }
    if response["protocol"] != "teshi-winapp" || response["version"] != 1 {
        let error = json!({"ok":false,"code":"protocol_mismatch","error":"Endpoint is not a compatible WinApp supervisor"});
        return Err(anyhow!(serde_json::to_string(&error)?));
    }
    if let Some(project) = response["project"].as_str()
        && Path::new(project).canonicalize().ok() != project_root.canonicalize().ok()
    {
        return Err(anyhow!(
            json!({"ok":false,"code":"project_identity_mismatch",
            "error":"Refusing another project's executor"})
            .to_string()
        ));
    }
    Ok(true)
}

fn ensure_daemon_for_project(project_root: &Path) -> Result<u16> {
    if let Some(manifest) = DaemonManifest::load_manifest(project_root) {
        if manifest.is_daemon_alive() {
            return Ok(manifest.port);
        }
        remove_daemon_manifest(project_root);
    }
    let port = 0;
    spawn_daemon_background(project_root, port, "127.0.0.1", None)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(anyhow!("Teshi daemon did not start within 15s"));
        }
        if let Some(manifest) = DaemonManifest::load_manifest(project_root)
            && manifest.is_daemon_alive()
        {
            return Ok(manifest.port);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn input_permission_error(action: &str, mode: &str, allowed: bool) -> Option<serde_json::Value> {
    let needs_permission = matches!(action, "pointer_click" | "shell_tray_click")
        || (mode == "foreground" && matches!(action, "click" | "fill" | "select" | "press_key"));
    (needs_permission && !(mode == "foreground" && allowed)).then(|| {
        json!({
            "ok": false,
            "code": "requires_interactive_input",
            "action": action,
            "required_mode": "foreground",
            "required_permission": "allow_interactive_input",
            "error": format!("{action} requires --mode foreground --allow-interactive-input"),
        })
    })
}

fn ensure_ok(response: &serde_json::Value) -> Result<()> {
    if response.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(());
    }
    let error = response
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("winapp command failed");
    Err(anyhow!("{error}"))
}

fn print_json_response(response: serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winapp_permission_matrix_never_promotes_auto_or_background() {
        for mode in ["auto", "background", "foreground"] {
            for allowed in [false, true] {
                for action in ["pointer_click", "shell_tray_click"] {
                    assert_eq!(
                        input_permission_error(action, mode, allowed).is_none(),
                        mode == "foreground" && allowed
                    );
                }
                for action in ["click", "fill", "select", "press_key"] {
                    assert_eq!(
                        input_permission_error(action, mode, allowed).is_none(),
                        mode != "foreground" || allowed
                    );
                }
                assert!(input_permission_error("assert_visible", mode, allowed).is_none());
            }
        }
    }

    fn target_with_attach(attach: Option<teshi_engine::StepBindingAttach>) -> WinappTarget {
        WinappTarget {
            profile: Some("app".into()),
            launch: None,
            args: Vec::new(),
            cwd: None,
            env: std::collections::HashMap::new(),
            attach,
            title: None,
        }
    }

    #[test]
    fn identity_matches_target_checks_hwnd_pid_and_title() {
        let identity = AttachedTargetIdentity {
            hwnd: Some(100),
            pid: Some(42),
            title: Some("Teshi Desktop - Main".into()),
            process: Some("teshi-desktop.exe".into()),
        };

        assert!(identity_matches_target(&identity, None));
        assert!(identity_matches_target(
            &identity,
            Some(&target_with_attach(None))
        ));

        let matching = target_with_attach(Some(teshi_engine::StepBindingAttach {
            hwnd: Some(100),
            pid: Some(42),
            title: Some("teshi desktop".into()),
            process_name: Some("TESHI-DESKTOP".into()),
        }));
        assert!(identity_matches_target(&identity, Some(&matching)));

        let wrong_title = target_with_attach(Some(teshi_engine::StepBindingAttach {
            hwnd: None,
            pid: None,
            title: Some("Calculator".into()),
            process_name: None,
        }));
        assert!(!identity_matches_target(&identity, Some(&wrong_title)));

        let wrong_pid = target_with_attach(Some(teshi_engine::StepBindingAttach {
            hwnd: None,
            pid: Some(99),
            title: None,
            process_name: None,
        }));
        assert!(!identity_matches_target(&identity, Some(&wrong_pid)));

        let wrong_hwnd = target_with_attach(Some(teshi_engine::StepBindingAttach {
            hwnd: Some(101),
            pid: None,
            title: None,
            process_name: None,
        }));
        assert!(!identity_matches_target(&identity, Some(&wrong_hwnd)));
    }
}
