use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::json;
use teshi_runtime::{
    StepBinding, read_active_step, resolve_step_bindings, send_sidecar_command_with_timeout,
};

use super::browser_endpoint::read_cdp_endpoint;
use super::replay_screenshots::{
    ReplayScreenshotEntry, capture_and_save_screenshot, iso_now, load_or_create_index, save_index,
};
use super::{
    WinAppAttachArgs, WinAppCommand, WinAppExecuteArgs, WinAppLaunchArgs, WinAppReplayArgs,
    WinAppSelectorArgs, WinAppSnapshotArgs,
};

/// Handles `teshi winapp ...` subcommands.
pub fn handle_winapp_command(action: &WinAppCommand) -> Result<()> {
    let project_root = std::env::current_dir().context("resolve current directory")?;
    match action {
        WinAppCommand::ListWindows => list_windows(&project_root),
        WinAppCommand::Attach(args) => attach(&project_root, args),
        WinAppCommand::Launch(args) => launch(&project_root, args),
        WinAppCommand::Snapshot(args) => snapshot(&project_root, args),
        WinAppCommand::Highlight(args) => highlight(&project_root, args),
        WinAppCommand::ClearHighlight => clear_highlight(&project_root),
        WinAppCommand::Execute(args) => execute(&project_root, args),
        WinAppCommand::Replay(args) => replay(&project_root, args),
    }
}

fn list_windows(project_root: &Path) -> Result<()> {
    let response = send_winapp_command(
        project_root,
        json!({ "cmd": "list_windows", "request_id": "winapp-list-windows" }),
        Duration::from_secs(10),
    )?;
    print_json_response(response)
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
    ensure_ok(&response)?;
    print_json_response(response)
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
        }),
        command_timeout_for_ms(args.timeout_ms),
    )?;
    ensure_ok(&response)?;
    print_json_response(response)
}

fn snapshot(project_root: &Path, args: &WinAppSnapshotArgs) -> Result<()> {
    let response = send_winapp_command(
        project_root,
        json!({ "cmd": "get_ui_snapshot", "request_id": "winapp-snapshot" }),
        Duration::from_millis(args.timeout_ms),
    )?;
    print_json_response(response)
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
    print_json_response(response)
}

fn clear_highlight(project_root: &Path) -> Result<()> {
    let response = send_winapp_command(
        project_root,
        json!({ "cmd": "clear_highlight", "request_id": "winapp-clear-highlight" }),
        Duration::from_secs(10),
    )?;
    print_json_response(response)
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
    )?;
    ensure_ok(&response)?;
    print_json_response(response)
}

fn replay(project_root: &Path, args: &WinAppReplayArgs) -> Result<()> {
    if let Some(path) = args.launch.as_deref() {
        ensure_winapp_attached(project_root, Some(path))?;
    } else {
        ensure_winapp_attached(project_root, None)?;
    }

    let feature = match args.feature.as_deref() {
        Some(feature) => feature.replace('\\', "/"),
        None => {
            read_active_step(project_root)
                .context("read .teshi/active-step.json for default feature")?
                .feature_relative_path
        }
    };
    let steps = resolve_step_bindings(project_root, &feature, args.until_line)?;
    if steps.is_empty() {
        return Err(anyhow!("no confirmed bindings found for {feature}"));
    }

    let mut screenshot_entries: Vec<ReplayScreenshotEntry> = Vec::new();
    let screenshot_dir = project_root
        .join(".teshi")
        .join("logs")
        .join("replay-screenshots");
    let _ = fs::create_dir_all(&screenshot_dir);

    let non_interactive = args.non_interactive || args.yes;
    for (idx, step) in steps.iter().enumerate() {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "index": idx + 1,
                "total": steps.len(),
                "step_line": step.step_line,
                "step": format!("{} {}", step.step_keyword, step.step_text),
                "action": step.primary.action,
                "target": step.primary.value,
                "value_arg": step.primary.value_arg
            }))?
        );
        if args.dry_run {
            continue;
        }
        if !non_interactive {
            prompt_continue(step)?;
        }
        let timeout_ms = 5_000;
        let response = execute_locator(
            project_root,
            &step.primary.value,
            &step.primary.action,
            step.primary.value_arg.as_deref(),
            timeout_ms,
            command_timeout_for_ms(timeout_ms),
            &format!("winapp-replay-{}", idx + 1),
            &args.mode,
        )?;
        // Capture screenshot after each step (before ensure_ok so we capture even on failure)
        if !args.dry_run {
            match read_cdp_endpoint(project_root) {
                Ok(endpoint) => {
                    match capture_and_save_screenshot(
                        &endpoint.ws_url,
                        project_root,
                        &teshi_runtime::sanitize_feature_path(&feature),
                        step.step_line,
                        &step.step_keyword,
                        &step.step_text,
                        &screenshot_dir,
                    ) {
                        Ok(entry) => screenshot_entries.push(entry),
                        Err(e) => eprintln!(
                            "warning: screenshot capture failed at L{}: {e}",
                            step.step_line
                        ),
                    }
                }
                Err(e) => eprintln!(
                    "warning: cannot read cdp-endpoint for screenshot at L{}: {e}",
                    step.step_line
                ),
            }
        }
        ensure_ok(&response).with_context(|| {
            format!(
                "replay failed at line {}: {} {}",
                step.step_line, step.step_keyword, step.step_text
            )
        })?;
    }

    if !args.dry_run && !screenshot_entries.is_empty() {
        let mut index = load_or_create_index(&screenshot_dir, &feature);
        index.steps = screenshot_entries;
        index.completed_at = Some(iso_now());
        index.status = "completed".to_string();
        if let Err(e) = save_index(&screenshot_dir, &index) {
            eprintln!("warning: failed to write screenshot index: {e}");
        }
        println!(
            "{}",
            serde_json::to_string(&json!({
                "event": "replay_complete",
                "screenshots_saved": index.steps.len(),
                "screenshots_dir": screenshot_dir.to_string_lossy(),
            }))?
        );
    }

    Ok(())
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

fn execute_locator(
    project_root: &Path,
    selector: &str,
    action: &str,
    value: Option<&str>,
    timeout_ms: u64,
    sidecar_timeout: Duration,
    request_id: &str,
    mode: &str,
) -> Result<serde_json::Value> {
    send_winapp_command(
        project_root,
        json!({
            "cmd": "execute_locator",
            "request_id": request_id,
            "selector": selector,
            "action": action,
            "value": value,
            "timeout_ms": timeout_ms,
            "mode": mode,
        }),
        sidecar_timeout,
    )
}

fn ensure_winapp_attached(project_root: &Path, launch: Option<&str>) -> Result<()> {
    let target = send_winapp_command(
        project_root,
        json!({ "cmd": "get_target", "request_id": "winapp-get-target" }),
        Duration::from_secs(10),
    )?;
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
    let endpoint_path = project_root.join(".teshi").join("cdp-endpoint.json");
    let text = fs::read_to_string(&endpoint_path)
        .with_context(|| format!("read {}", endpoint_path.display()))?;
    let endpoint: serde_json::Value = serde_json::from_str(&text).context("parse cdp endpoint")?;
    let mode = endpoint.get("mode").and_then(|v| v.as_str()).unwrap_or("");
    if mode != "winapp" {
        return Err(anyhow!(
            "current sidecar mode is {mode:?}, expected \"winapp\"; start Connect WinUI3 App first"
        ));
    }
    let ws_url = endpoint
        .get("ws_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("cdp endpoint missing ws_url"))?;
    send_sidecar_command_with_timeout(ws_url, command, timeout).map_err(anyhow::Error::msg)
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
