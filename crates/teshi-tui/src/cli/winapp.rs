use std::io::{self, Write};
use std::path::Path;
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
    let feature = match args.feature.as_deref() {
        Some(feature) => feature.replace('\\', "/"),
        None => read_active_step(project_root)?.feature_relative_path,
    };
    preflight_feature(project_root, &feature)?;
    let plan = super::winapp_plan::build(project_root, &feature, args.until_line)?;
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
    let mut run = super::replay_screenshots::create_cli_replay(project_root, &feature, None)?;
    run.record(json!({"type":"coverage","coverage":coverage,"screenshot_policy":"optional"}))?;
    print_json_response(json!({"event":"replay_started","run_id":run.id(),"coverage":coverage}))?;
    let directory = run.directory().to_path_buf();
    let mut index = load_or_create_index(&directory, &feature);
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
                run.record(json!({"type":"preflight_error","response":error}))?;
                print_json_response(error.clone())?;
                ensure_ok(&error)?;
            }
        }
        ensure_winapp_sidecar(project_root, false)?;
        ensure_winapp_attached(project_root, args.launch.as_deref())?;
        for planned in &plan.steps {
            let step = &planned.binding;
            if !args.non_interactive && !args.yes {
                prompt_continue(step)?;
            }
            run.record(json!({"type":"start_step","step_id":planned.step_id,
                "case_id":planned.case_id,"scenario":planned.scenario,"step_line":step.step_line,"text":step.step_text}))?;
            let response = execute_locator(
                project_root,
                &step.primary.value,
                &step.primary.action,
                step.primary.value_arg.as_deref(),
                5000,
                command_timeout_for_ms(5000),
                &if step.primary.action == "assert_screenshot" {
                    format!(
                        "{}-L{}",
                        teshi_engine::sanitize_feature_path(&feature),
                        step.step_line
                    )
                } else {
                    format!("{}:{}", run.id(), planned.step_id)
                },
                &args.mode,
                args.allow_interactive_input,
            )
            .unwrap_or_else(|error| transport_failure(&error.to_string()));
            let passed = ensure_ok(&response).is_ok();
            // The action result is durable before any potentially failing capture.
            run.record(json!({"type":"step_result","step_id":planned.step_id,
                "case_id":planned.case_id,"scenario":planned.scenario,"step_line":step.step_line,
                "status":if passed {"passed"} else {"failed"},"response":response}))?;
            print_json_response(response.clone())?;
            let capture = read_cdp_endpoint(project_root).and_then(|endpoint| {
                capture_and_save_screenshot(
                    &endpoint.ws_url,
                    project_root,
                    &teshi_engine::sanitize_feature_path(&feature),
                    step.step_line,
                    &step.step_keyword,
                    &step.step_text,
                    &directory,
                )
            });
            let mut event = json!({"type":"step_screenshot","step_id":planned.step_id,
                "case_id":planned.case_id,"scenario":planned.scenario,"feature":feature,
                "step_line":step.step_line,"step_keyword":step.step_keyword,"step_text":step.step_text,
                "status":if passed {"passed"} else {"failed"}});
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
            run.record(event)?;
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
    index = load_or_create_index(&directory, &feature);
    index.status = status.into();
    index.completed_at = Some(iso_now());
    save_index(&directory, &index)?;
    run.record(json!({"type":"end_run","status":status,"failed":usize::from(result.is_err()),
        "capture_failures":capture_failures,"error":result.as_ref().err().map(ToString::to_string)}))?;
    print_json_response(
        json!({"event":"replay_complete","run_id":run.id(),"status":status,
        "coverage":coverage,"screenshot_policy":"optional","capture_failures":capture_failures,
        "screenshot_dir":directory,"screenshots":index.steps.len(),"failed_line":failed_line}),
    )?;
    result
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

fn ensure_winapp_attached(project_root: &Path, launch: Option<&str>) -> Result<()> {
    let target = send_winapp_command(
        project_root,
        json!({ "cmd": "get_target", "request_id": "winapp-get-target" }),
        Duration::from_secs(10),
    )?;
    ensure_ok(&target)?;
    let attached = target
        .get("target")
        .and_then(|t| t.get("attached"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if attached {
        return Ok(());
    }
    if let Some(path) = launch {
        let response = send_winapp_command(
            project_root,
            json!({
                "cmd": "launch_app",
                "request_id": "winapp-replay-launch",
                "path": path,
                "timeout_ms": 15000
            }),
            command_timeout_for_ms(15_000),
        )?;
        ensure_ok(&response)?;
        return Ok(());
    }
    Err(anyhow!(
        "no WinUI3 window attached; run `teshi winapp attach ...` or `teshi winapp replay --launch <exe>`"
    ))
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
}
