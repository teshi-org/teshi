use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use serde_json::json;
use teshi_engine::{
    BrowserMode, RuntimeConfig, StepBinding, TeshiEngine, default_browser_service_script,
    default_winapp_service_script, open_project, read_active_step, resolve_step_bindings,
    send_sidecar_command_with_timeout, start_browser_sidecar, stop_browser_sidecar,
};

use super::browser_endpoint::{
    auto_reconnect_enabled, doctor_endpoint, ensure_sidecar_healthy, read_cdp_endpoint,
    reconnect_embedded, resolve_browser_project_root, write_cdp_endpoint_from_rust,
};
use super::locator_verify::{LocatorVerifyRecord, append_locator_verify, verify_record_json};
use super::replay_screenshots::{
    ReplayScreenshotEntry, capture_and_save_screenshot, iso_now, load_or_create_index, save_index,
};
use super::{
    BrowserCommand, BrowserExecuteArgs, BrowserNavigateArgs, BrowserReconnectArgs,
    BrowserReplayArgs, BrowserSelectorArgs, BrowserServeEmbeddedArgs, BrowserSnapshotArgs,
    BrowserVerifyArgs,
};

/// Handles `teshi browser ...` subcommands.
pub fn handle_browser_command(action: &BrowserCommand) -> Result<()> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let project_root = resolve_browser_project_root(&cwd).unwrap_or(cwd);
    match action {
        BrowserCommand::Snapshot(args) => snapshot(&project_root, args),
        BrowserCommand::Navigate(args) => navigate(&project_root, args),
        BrowserCommand::Highlight(args) => highlight(&project_root, args),
        BrowserCommand::ClearHighlight => clear_highlight(&project_root),
        BrowserCommand::Execute(args) => execute(&project_root, args),
        BrowserCommand::Replay(args) => replay(&project_root, args),
        BrowserCommand::ServeEmbedded(args) => serve_embedded(args),
        BrowserCommand::Doctor => doctor(&project_root),
        BrowserCommand::Reconnect(args) => reconnect(&project_root, args),
        BrowserCommand::Verify(args) => verify(&project_root, args),
        BrowserCommand::Enhance(args) => enhance(&project_root, args),
        BrowserCommand::HealExecute(args) => heal_execute(&project_root, args),
    }
}

fn doctor(project_root: &Path) -> Result<()> {
    let report = doctor_endpoint(project_root)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn reconnect(project_root: &Path, args: &BrowserReconnectArgs) -> Result<()> {
    let endpoint = reconnect_embedded(project_root, args.navigate.as_deref(), args.wait_secs)?;
    let report = doctor_endpoint(project_root)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": report.ok,
            "ws_url": endpoint.ws_url,
            "mode": endpoint.mode,
            "page_url": endpoint.page_url,
            "doctor": report
        }))?
    );
    if !report.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn verify(project_root: &Path, args: &BrowserVerifyArgs) -> Result<()> {
    let timeout = command_timeout_for_ms(args.timeout_ms);
    let highlight = send_browser_command(
        project_root,
        json!({
            "cmd": "highlight_selector",
            "request_id": "browser-verify-highlight",
            "selector": args.selector
        }),
        Duration::from_secs(20),
        false,
    )?;
    let highlight_ok = highlight.get("ok").and_then(|v| v.as_bool()) == Some(true);
    let response = if args.action == "open_project" {
        open_project_via_sidecar(
            project_root,
            args.value_arg.as_deref().unwrap_or(&args.selector),
            timeout,
            "browser-verify-open-project",
        )?
    } else if args.action == "navigate" {
        let url = args
            .value_arg
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or(&args.selector);
        navigate_to_url(
            project_root,
            url,
            args.timeout_ms,
            timeout,
            "browser-verify-navigate",
            false,
        )?
    } else {
        execute_locator(
            project_root,
            ExecuteLocatorParams {
                selector: &args.selector,
                action: &args.action,
                value: args.value_arg.as_deref(),
                timeout_ms: args.timeout_ms,
                request_id: "browser-verify-execute",
                health_check: false,
            },
            timeout,
        )?
    };
    let execute_ok = response.get("ok").and_then(|v| v.as_bool()) == Some(true);
    let ok = highlight_ok && execute_ok;
    let record = LocatorVerifyRecord {
        ts_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default(),
        step_line: args.step_line,
        selector: args.selector.clone(),
        action: args.action.clone(),
        value_arg: args.value_arg.clone(),
        ok,
    };
    if ok {
        append_locator_verify(project_root, &record)?;
    }
    let mut output = verify_record_json(project_root, &record);
    if let Some(obj) = output.as_object_mut() {
        obj.insert("highlight_ok".into(), json!(highlight_ok));
        obj.insert("execute_ok".into(), json!(execute_ok));
        obj.insert("response".into(), response);
    }
    println!("{}", serde_json::to_string_pretty(&output)?);
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

fn navigate(project_root: &Path, args: &BrowserNavigateArgs) -> Result<()> {
    let timeout = command_timeout_for_ms(args.timeout_ms);
    let response = navigate_to_url(
        project_root,
        &args.url,
        args.timeout_ms,
        timeout,
        "browser-navigate",
        true,
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
        true,
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
        false,
    )?;
    print_json_response(response)
}

fn clear_highlight(project_root: &Path) -> Result<()> {
    let response = send_browser_command(
        project_root,
        json!({ "cmd": "clear_highlight", "request_id": "browser-clear-highlight" }),
        Duration::from_secs(10),
        false,
    )?;
    print_json_response(response)
}

fn execute(project_root: &Path, args: &BrowserExecuteArgs) -> Result<()> {
    let timeout = command_timeout_for_ms(args.timeout_ms);
    let response = execute_locator(
        project_root,
        ExecuteLocatorParams {
            selector: &args.selector,
            action: &args.action,
            value: args.value_arg.as_deref(),
            timeout_ms: args.timeout_ms,
            request_id: "browser-execute",
            health_check: true,
        },
        timeout,
    )?;
    ensure_ok(&response)?;
    print_json_response(response)
}

fn enhance(project_root: &Path, args: &BrowserSelectorArgs) -> Result<()> {
    let timeout = command_timeout_for_ms(10_000);
    let response = send_browser_command(
        project_root,
        json!({
            "cmd": "enhance_locator",
            "request_id": "browser-enhance",
            "selector": args.selector,
        }),
        timeout,
        true,
    )?;
    let ok = response
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !ok {
        let error = response
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        anyhow::bail!("enhance failed: {error}");
    }
    print_json_response(response)
}

fn heal_execute(project_root: &Path, args: &BrowserExecuteArgs) -> Result<()> {
    let timeout = command_timeout_for_ms(args.timeout_ms + 15_000);
    let response = send_browser_command(
        project_root,
        json!({
            "cmd": "heal_execute_locator",
            "request_id": "browser-heal-execute",
            "selector": args.selector,
            "action": args.action,
            "value": args.value_arg,
            "timeout_ms": args.timeout_ms,
        }),
        timeout,
        true,
    )?;
    let ok = response
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !ok {
        let error = response
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        anyhow::bail!("heal_execute failed: {error}");
    }
    if response
        .get("healed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        eprintln!(
            "  healed: original={} → {}",
            response
                .get("original_selector")
                .and_then(|v| v.as_str())
                .unwrap_or("?"),
            response
                .get("selector")
                .and_then(|v| v.as_str())
                .unwrap_or("?"),
        );
    }
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
                true,
            )?
        } else if step.primary.action == "open_project" {
            let path = step
                .primary
                .value_arg
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(&step.primary.value);
            let timeout_ms = 15_000;
            open_project_via_sidecar(
                project_root,
                path,
                command_timeout_for_ms(timeout_ms),
                &format!("browser-replay-{}", idx + 1),
            )?
        } else {
            let timeout_ms = 5_000;
            execute_locator(
                project_root,
                ExecuteLocatorParams {
                    selector: &step.primary.value,
                    action: &step.primary.action,
                    value: step.primary.value_arg.as_deref(),
                    timeout_ms,
                    request_id: &format!("browser-replay-{}", idx + 1),
                    health_check: true,
                },
                command_timeout_for_ms(timeout_ms),
            )?
        };
        // Capture screenshot after each step (before ensure_ok so we capture even on failure)
        if !args.dry_run {
            match read_cdp_endpoint(project_root) {
                Ok(endpoint) => {
                    match capture_and_save_screenshot(
                        &endpoint.ws_url,
                        project_root,
                        &teshi_engine::sanitize_feature_path(&feature),
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

/// Starts the embedded Playwright sidecar and blocks until interrupted (for CI/scripts).
fn serve_embedded(args: &BrowserServeEmbeddedArgs) -> Result<()> {
    let rt = tokio::runtime::Runtime::new().context("create tokio runtime")?;
    rt.block_on(serve_embedded_async(args))
}

async fn serve_embedded_async(args: &BrowserServeEmbeddedArgs) -> Result<()> {
    let project_root = match &args.project {
        Some(path) => path.clone(),
        None => std::env::current_dir().context("resolve current directory")?,
    };
    let project_root = project_root
        .canonicalize()
        .with_context(|| format!("canonicalize project {}", project_root.display()))?;

    let dev_browser_script = resolve_embedded_browser_service_script(&project_root);
    // Headless CI: use repo browser_service.py and disable JPEG preview.
    // SAFETY: set before constructing runtime / spawning the Python sidecar child.
    unsafe {
        std::env::set_var("TESHI_BROWSER_SERVICE", &dev_browser_script);
    }

    let runtime = TeshiEngine::new(
        RuntimeConfig {
            browser_service_script: dev_browser_script,
            winapp_service_script: default_winapp_service_script(),
            embedded_no_preview_stream: false,
        },
        None,
    );

    open_project(runtime.clone(), project_root.to_string_lossy().to_string())
        .await
        .map_err(|e| anyhow!("open project: {e}"))?;

    let start = start_browser_sidecar(runtime.clone(), BrowserMode::Embedded)
        .await
        .map_err(browser_error)?;

    eprintln!("embedded sidecar ws_url={}", start.ws_url);
    eprintln!("cdp endpoint={}", start.cdp_endpoint_path);

    // Ensure cdp-endpoint.json is written from the Rust side with the actual ws_url,
    // so subsequent commands (e.g. navigate) don't race with the Python sidecar's write.
    if let Err(e) =
        write_cdp_endpoint_from_rust(&project_root, &start.ws_url, &start.mode, "about:blank")
    {
        eprintln!("warning: failed to write cdp-endpoint.json: {e}");
    }

    if let Some(url) = args.navigate.as_deref() {
        let timeout_ms = 15_000;
        let response = navigate_to_url(
            &project_root,
            url,
            timeout_ms,
            command_timeout_for_ms(timeout_ms),
            "serve-embedded-navigate",
            false,
        )?;
        ensure_ok(&response).with_context(|| format!("navigate to {url}"))?;
        eprintln!("navigated to {url}");
    }

    eprintln!("embedded sidecar running; press Ctrl+C to stop");
    tokio::signal::ctrl_c().await.context("wait for Ctrl+C")?;
    stop_browser_sidecar(&runtime)
        .await
        .map_err(|e| anyhow!("stop sidecar: {e}"))?;
    Ok(())
}

fn browser_error(err: teshi_engine::BrowserError) -> anyhow::Error {
    if let Some(hint) = err.hint {
        anyhow!("{} — {hint}", err.message)
    } else {
        anyhow!(err.message)
    }
}

/// Prefers the repo `browser_service.py` so CI picks up the latest embedded flags.
fn resolve_embedded_browser_service_script(project_root: &Path) -> PathBuf {
    let repo_script = project_root.join("resources/browser_service.py");
    if repo_script.is_file() {
        return repo_script;
    }
    default_browser_service_script()
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
    health_check: bool,
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
        health_check,
    )
}

fn open_project_via_sidecar(
    project_root: &Path,
    path: &str,
    sidecar_timeout: Duration,
    request_id: &str,
) -> Result<serde_json::Value> {
    send_browser_command(
        project_root,
        json!({
            "cmd": "open_project",
            "request_id": request_id,
            "path": path
        }),
        sidecar_timeout,
        true,
    )
}

/// Bundles parameters for a single `execute_locator` sidecar command.
struct ExecuteLocatorParams<'a> {
    selector: &'a str,
    action: &'a str,
    value: Option<&'a str>,
    timeout_ms: u64,
    request_id: &'a str,
    health_check: bool,
}

fn execute_locator(
    project_root: &Path,
    params: ExecuteLocatorParams<'_>,
    sidecar_timeout: Duration,
) -> Result<serde_json::Value> {
    send_browser_command(
        project_root,
        json!({
            "cmd": "execute_locator",
            "request_id": params.request_id,
            "selector": params.selector,
            "action": params.action,
            "value": params.value,
            "timeout_ms": params.timeout_ms
        }),
        sidecar_timeout,
        params.health_check,
    )
}

fn send_browser_command(
    project_root: &Path,
    command: serde_json::Value,
    timeout: Duration,
    health_check: bool,
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
    if health_check && auto_reconnect_enabled() {
        let _ = ensure_sidecar_healthy(project_root);
    }
    let endpoint = read_cdp_endpoint(project_root)?;
    debug_log(
        project_root,
        json!({
            "event": "browser_command_start",
            "cmd": cmd.clone(),
            "request_id": request_id.clone(),
            "command": command.clone(),
            "timeout_ms": timeout.as_millis(),
            "endpoint_path": endpoint.endpoint_path.display().to_string(),
            "project_root": endpoint.project_root.display().to_string(),
        }),
    );
    let ws_url = endpoint.ws_url;
    match send_sidecar_command_with_timeout(&ws_url, command, timeout).map_err(anyhow::Error::msg) {
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
