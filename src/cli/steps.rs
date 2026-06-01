use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use serde_json::json;
use teshi_runtime::{
    HighlightInfo, LocatorCandidate, PendingLocator, StepWaitUntil, confirm_pending_locator,
    list_step_bindings, propose_locator, read_active_step, reject_pending_locator,
    resolve_step_bindings, send_sidecar_command, step_binding_statuses, wait_for_step_status,
};

use super::{
    StepsCommand, StepsConfirmArgs, StepsListArgs, StepsProposeArgs, StepsResolveArgs,
    StepsWaitArgs, WaitUntilArg,
};

/// Handles `teshi steps ...` subcommands.
pub fn handle_steps_command(action: &StepsCommand) -> Result<()> {
    let project_root = std::env::current_dir().context("resolve current directory")?;
    match action {
        StepsCommand::Propose(args) => propose(&project_root, args),
        StepsCommand::Confirm(args) => confirm(&project_root, args),
        StepsCommand::Reject => reject(&project_root),
        StepsCommand::Wait(args) => wait(&project_root, args),
        StepsCommand::Resolve(args) => resolve(&project_root, args),
        StepsCommand::List(args) => list(&project_root, args),
    }
}

fn propose(project_root: &Path, args: &StepsProposeArgs) -> Result<()> {
    let started = Instant::now();
    let active = read_active_step(project_root).context("read .teshi/active-step.json")?;
    let value = proposal_value(args)?;
    debug_log(
        project_root,
        json!({
            "event": "steps_propose_start",
            "action": args.action,
            "value": value.clone(),
            "value_arg": args.value_arg.clone(),
            "step_line": active.step_line
        }),
    );
    let pending = PendingLocator {
        step_ref: active,
        candidates: vec![LocatorCandidate {
            rank: args.rank,
            strategy: args.strategy.clone(),
            value,
            action: args.action.clone(),
            value_arg: args.value_arg.clone(),
            confidence: args.confidence,
            rationale: args.rationale.clone(),
        }],
        highlight: Some(HighlightInfo {
            candidate_rank: args.rank,
            applied: args.highlight_applied,
        }),
        status: "pending".to_string(),
    };
    propose_locator(project_root, pending)?;
    debug_log(
        project_root,
        json!({
            "event": "steps_propose_end",
            "action": args.action.clone(),
            "elapsed_ms": started.elapsed().as_millis(),
            "ok": true
        }),
    );
    println!("{}", json!({ "ok": true, "status": "pending" }));
    Ok(())
}

fn confirm(project_root: &Path, args: &StepsConfirmArgs) -> Result<()> {
    let started = Instant::now();
    let pending = confirm_pending_locator(project_root, args.rank, args.value.clone())?;
    clear_highlight_best_effort(project_root);
    debug_log(
        project_root,
        json!({
            "event": "steps_confirm_end",
            "rank": args.rank,
            "elapsed_ms": started.elapsed().as_millis(),
            "status": pending.status
        }),
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": true,
            "status": pending.status,
            "feature": pending.step_ref.feature_relative_path,
            "step_line": pending.step_ref.step_line
        }))?
    );
    Ok(())
}

fn reject(project_root: &Path) -> Result<()> {
    let started = Instant::now();
    let pending = reject_pending_locator(project_root)?;
    clear_highlight_best_effort(project_root);
    debug_log(
        project_root,
        json!({
            "event": "steps_reject_end",
            "elapsed_ms": started.elapsed().as_millis(),
            "status": pending.status
        }),
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": false,
            "status": pending.status,
            "reason": "rejected"
        }))?
    );
    std::process::exit(2);
}

fn wait(project_root: &Path, args: &StepsWaitArgs) -> Result<()> {
    let started = Instant::now();
    let result = wait_for_step_status(
        project_root,
        match args.until {
            WaitUntilArg::Confirmed => StepWaitUntil::Confirmed,
            WaitUntilArg::Rejected => StepWaitUntil::Rejected,
            WaitUntilArg::Either => StepWaitUntil::Either,
        },
        Duration::from_secs(args.timeout),
    )?;
    debug_log(
        project_root,
        json!({
            "event": "steps_wait_end",
            "until": format!("{:?}", args.until),
            "elapsed_ms": started.elapsed().as_millis(),
            "status": result.status
        }),
    );
    println!("{}", serde_json::to_string_pretty(&result)?);
    if result.status == "rejected" {
        std::process::exit(2);
    }
    Ok(())
}

fn resolve(project_root: &Path, args: &StepsResolveArgs) -> Result<()> {
    let feature = resolve_feature_arg(project_root, args.feature.as_deref())?;
    let steps = resolve_step_bindings(project_root, &feature, args.until_line)?;
    println!("{}", serde_json::to_string_pretty(&steps)?);
    Ok(())
}

fn list(project_root: &Path, args: &StepsListArgs) -> Result<()> {
    let feature = resolve_feature_arg(project_root, args.feature.as_deref())?;
    let bindings = list_step_bindings(project_root, &feature)?;
    let statuses = step_binding_statuses(project_root, &feature)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "bindings": bindings,
            "statuses": statuses
        }))?
    );
    Ok(())
}

fn proposal_value(args: &StepsProposeArgs) -> Result<String> {
    if args.action == "navigate" {
        return args
            .value_arg
            .clone()
            .or_else(|| args.value.clone())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                anyhow!("navigate proposals require --value-arg <url> or --value <url>")
            });
    }
    args.value
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("{} proposals require --value <selector>", args.action))
}

fn resolve_feature_arg(project_root: &Path, feature: Option<&str>) -> Result<String> {
    if let Some(feature) = feature {
        return Ok(feature.replace('\\', "/"));
    }
    Ok(read_active_step(project_root)
        .context("read .teshi/active-step.json for default feature")?
        .feature_relative_path)
}

fn clear_highlight_best_effort(project_root: &Path) {
    let endpoint_path = project_root.join(".teshi").join("cdp-endpoint.json");
    let Ok(text) = fs::read_to_string(endpoint_path) else {
        return;
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let Some(ws_url) = payload.get("ws_url").and_then(|v| v.as_str()) else {
        return;
    };
    let _ = send_sidecar_command(
        ws_url,
        json!({ "cmd": "clear_highlight", "request_id": "steps-clear-highlight" }),
    );
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
