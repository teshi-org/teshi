use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use serde_json::json;
use teshi_runtime::{
    StepBinding, read_active_step, resolve_step_bindings, send_sidecar_command_with_timeout,
};

use super::{
    BrowserCommand, BrowserExecuteArgs, BrowserNavigateArgs, BrowserReplayArgs,
    BrowserSelectorArgs, BrowserSnapshotArgs,
};

/// Handles `teshi browser ...` subcommands.
pub fn handle_browser_command(action: &BrowserCommand) -> Result<()> {
    let project_root = std::env::current_dir().context("resolve current directory")?;
    match action {
        BrowserCommand::Snapshot(args) => snapshot(&project_root, args),
        BrowserCommand::Navigate(args) => navigate(&project_root, args),
        BrowserCommand::Highlight(args) => highlight(&project_root, args),
        BrowserCommand::ClearHighlight => clear_highlight(&project_root),
        BrowserCommand::Execute(args) => execute(&project_root, args),
        BrowserCommand::Replay(args) => replay(&project_root, args),
    }
}

fn navigate(project_root: &Path, args: &BrowserNavigateArgs) -> Result<()> {
    let timeout = command_timeout_for_ms(args.timeout_ms);
    let response = navigate_to_url(
        project_root,
        &args.url,
        args.timeout_ms,
        timeout,
        "browser-navigate",
    )?;
    ensure_ok(&response)?;
    print_json_response(response)
}

fn snapshot(project_root: &Path, args: &BrowserSnapshotArgs) -> Result<()> {
    let timeout = Duration::from_millis(args.timeout_ms);
    let response = send_browser_command(
        project_root,
        json!({ "cmd": "get_page_snapshot", "request_id": "browser-snapshot" }),
        timeout,
    )?;
    print_json_response(response)
}

fn highlight(project_root: &Path, args: &BrowserSelectorArgs) -> Result<()> {
    let response = send_browser_command(
        project_root,
        json!({
            "cmd": "highlight_selector",
            "request_id": "browser-highlight",
            "selector": args.selector
        }),
        Duration::from_secs(20),
    )?;
    print_json_response(response)
}

fn clear_highlight(project_root: &Path) -> Result<()> {
    let response = send_browser_command(
        project_root,
        json!({ "cmd": "clear_highlight", "request_id": "browser-clear-highlight" }),
        Duration::from_secs(10),
    )?;
    print_json_response(response)
}

fn execute(project_root: &Path, args: &BrowserExecuteArgs) -> Result<()> {
    let timeout = command_timeout_for_ms(args.timeout_ms);
    let response = execute_locator(
        project_root,
        &args.selector,
        &args.action,
        args.value_arg.as_deref(),
        args.timeout_ms,
        timeout,
        "browser-execute",
    )?;
    ensure_ok(&response)?;
    print_json_response(response)
}

fn replay(project_root: &Path, args: &BrowserReplayArgs) -> Result<()> {
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
        let response = if step.primary.action == "navigate" {
            let url = step
                .primary
                .value_arg
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(&step.primary.value);
            let timeout_ms = 15_000;
            navigate_to_url(
                project_root,
                url,
                timeout_ms,
                command_timeout_for_ms(timeout_ms),
                &format!("browser-replay-{}", idx + 1),
            )?
        } else {
            let timeout_ms = 5_000;
            execute_locator(
                project_root,
                &step.primary.value,
                &step.primary.action,
                step.primary.value_arg.as_deref(),
                timeout_ms,
                command_timeout_for_ms(timeout_ms),
                &format!("browser-replay-{}", idx + 1),
            )?
        };
        ensure_ok(&response).with_context(|| {
            format!(
                "replay failed at line {}: {} {}",
                step.step_line, step.step_keyword, step.step_text
            )
        })?;
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

/// Sidecar wait budget: locator timeout plus slack for Chrome heartbeat and CDP work.
fn command_timeout_for_ms(timeout_ms: u64) -> Duration {
    let secs = timeout_ms.div_ceil(1000).saturating_add(5);
    Duration::from_secs(secs.max(15))
}

fn navigate_to_url(
    project_root: &Path,
    url: &str,
    timeout_ms: u64,
    sidecar_timeout: Duration,
    request_id: &str,
) -> Result<serde_json::Value> {
    send_browser_command(
        project_root,
        json!({
            "cmd": "navigate",
            "request_id": request_id,
            "url": url,
            "timeout_ms": timeout_ms
        }),
        sidecar_timeout,
    )
}

fn execute_locator(
    project_root: &Path,
    selector: &str,
    action: &str,
    value: Option<&str>,
    timeout_ms: u64,
    sidecar_timeout: Duration,
    request_id: &str,
) -> Result<serde_json::Value> {
    send_browser_command(
        project_root,
        json!({
            "cmd": "execute_locator",
            "request_id": request_id,
            "selector": selector,
            "action": action,
            "value": value,
            "timeout_ms": timeout_ms
        }),
        sidecar_timeout,
    )
}

fn send_browser_command(
    project_root: &Path,
    command: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value> {
    let started = Instant::now();
    let cmd = command
        .get("cmd")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let request_id = command
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    debug_log(
        project_root,
        json!({
            "event": "browser_command_start",
            "cmd": cmd.clone(),
            "request_id": request_id.clone(),
            "command": command.clone(),
            "timeout_ms": timeout.as_millis()
        }),
    );
    let endpoint_path = project_root.join(".teshi").join("cdp-endpoint.json");
    let text = fs::read_to_string(&endpoint_path)
        .with_context(|| format!("read {}", endpoint_path.display()))?;
    let endpoint: serde_json::Value = serde_json::from_str(&text).context("parse cdp endpoint")?;
    let ws_url = endpoint
        .get("ws_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("cdp endpoint missing ws_url"))?;
    match send_sidecar_command_with_timeout(ws_url, command, timeout).map_err(anyhow::Error::msg) {
        Ok(response) => {
            debug_log(
                project_root,
                json!({
                    "event": "browser_command_end",
                    "cmd": cmd.clone(),
                    "request_id": request_id.clone(),
                    "elapsed_ms": started.elapsed().as_millis(),
                    "ok": response.get("ok").and_then(|v| v.as_bool()),
                    "error": response.get("error")
                }),
            );
            Ok(response)
        }
        Err(err) => {
            debug_log(
                project_root,
                json!({
                    "event": "browser_command_error",
                    "cmd": cmd.clone(),
                    "request_id": request_id.clone(),
                    "elapsed_ms": started.elapsed().as_millis(),
                    "error": err.to_string()
                }),
            );
            Err(err)
        }
    }
}

fn ensure_ok(response: &serde_json::Value) -> Result<()> {
    if response.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(());
    }
    let error = response
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("browser command failed");
    Err(anyhow!("{error}"))
}

fn print_json_response(response: serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn debug_log(project_root: &Path, mut payload: serde_json::Value) {
    if std::env::var_os("TESHI_BROWSER_DEBUG").is_none() {
        return;
    }
    if let serde_json::Value::Object(ref mut object) = payload {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        object.insert("ts_ms".to_string(), json!(ts_ms));
    }
    let log_dir = project_root.join(".teshi").join("logs");
    if fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let path = log_dir.join("cli-browser.log");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{}", payload);
    }
}
