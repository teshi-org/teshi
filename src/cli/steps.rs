use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use serde_json::json;
use teshi_runtime::{
    HighlightInfo, LocatorCandidate, PendingLocator, StepWaitUntil, confirm_pending_locator,
    first_unbound_feature_step, list_feature_step_refs, list_step_bindings, propose_locator,
    read_active_step, reject_pending_locator, resolve_step_bindings, send_sidecar_command,
    step_binding_statuses, unbind_step_binding, wait_for_step_status, write_active_step,
};

use super::locator_verify::locator_verify_satisfied;
use super::{
    StepsCommand, StepsConfirmArgs, StepsFeatureArgs, StepsListArgs, StepsProposeArgs,
    StepsResolveArgs, StepsSelectArgs, StepsUnbindArgs, StepsWaitArgs, WaitUntilArg,
};

/// Handles `teshi steps ...` subcommands.
pub fn handle_steps_command(action: &StepsCommand) -> Result<()> {
    let project_root = teshi_runtime::find_project_root(None)
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    // Try daemon for state-modifying operations
    if let Some(manifest) = teshi_runtime::DaemonManifest::load(&project_root) {
        if manifest.is_alive() {
            return handle_steps_via_daemon(&manifest, action);
        }
    }

    // Fallback: direct file I/O
    match action {
        StepsCommand::Select(args) => select(&project_root, args),
        StepsCommand::Unbound(args) => unbound(&project_root, args),
        StepsCommand::NextUnbound(args) => next_unbound(&project_root, args),
        StepsCommand::Propose(args) => propose(&project_root, args),
        StepsCommand::Confirm(args) => confirm(&project_root, args),
        StepsCommand::Reject => reject(&project_root),
        StepsCommand::Wait(args) => wait(&project_root, args),
        StepsCommand::Resolve(args) => resolve(&project_root, args),
        StepsCommand::List(args) => list(&project_root, args),
        StepsCommand::Unbind(args) => unbind(&project_root, args),
    }
}

/// Route steps commands through the daemon's REST API.
fn handle_steps_via_daemon(
    manifest: &teshi_runtime::DaemonManifest,
    action: &StepsCommand,
) -> Result<()> {
    let base = format!("http://127.0.0.1:{}", manifest.port);
    let client = reqwest::blocking::Client::new();

    match action {
        StepsCommand::Select(args) => {
            let resp = client
                .post(format!("{base}/api/v1/locator/sync-step"))
                .json(&json!({
                    "feature_path": args.feature,
                    "step_line": args.line,
                }))
                .send()
                .context("sync step via daemon")?;
            print_response(resp)?;
        }
        StepsCommand::Unbound(args) => {
            let feature =
                resolve_feature_arg_daemon(&client, &base, args.feature.as_deref())?;
            let resp = client
                .get(format!(
                    "{base}/api/v1/steps/statuses?feature_path={feature}"
                ))
                .send()
                .context("get step statuses via daemon")?;
            print_response(resp)?;
        }
        StepsCommand::Confirm(args) => {
            let resp = client
                .post(format!("{base}/api/v1/locator/confirm"))
                .json(&json!({
                    "candidate_rank": args.rank,
                    "edited_value": args.value,
                }))
                .send()
                .context("confirm locator via daemon")?;
            print_response(resp)?;
        }
        StepsCommand::Reject => {
            let resp = client
                .post(format!("{base}/api/v1/locator/reject"))
                .send()
                .context("reject locator via daemon")?;
            print_response(resp)?;
        }
        // For read-only ops that don't have direct daemon API equivalents,
        // fall back to direct file I/O:
        _ => {
            let project_root = teshi_runtime::find_project_root(None)
                .unwrap_or_else(|| std::env::current_dir().unwrap());
            return match action {
                StepsCommand::NextUnbound(args) => next_unbound(&project_root, args),
                StepsCommand::Propose(args) => propose(&project_root, args),
                StepsCommand::Wait(args) => wait(&project_root, args),
                StepsCommand::Resolve(args) => resolve(&project_root, args),
                StepsCommand::List(args) => list(&project_root, args),
                StepsCommand::Unbind(args) => unbind(&project_root, args),
                _ => unreachable!(),
            };
        }
    }
    Ok(())
}

fn print_response(resp: reqwest::blocking::Response) -> Result<()> {
    if !resp.status().is_success() {
        anyhow::bail!(
            "daemon returned {}: {}",
            resp.status(),
            resp.text().unwrap_or_default()
        );
    }
    let body = resp.text()?;
    if !body.trim().is_empty() {
        println!("{body}");
    }
    Ok(())
}

fn resolve_feature_arg_daemon(
    client: &reqwest::blocking::Client,
    base: &str,
    feature: Option<&str>,
) -> Result<String> {
    if let Some(f) = feature {
        return Ok(f.replace('\\', "/"));
    }
    // Get active step from daemon
    let resp = client
        .get(format!("{base}/api/v1/locator/active-step"))
        .send()
        .context("get active step via daemon")?;
    let body: serde_json::Value = resp.json()?;
    body.get("feature_relative_path")
        .and_then(|v| v.as_str())
        .map(String::from)
        .context("no active step; pass --feature explicitly")
}

fn select(project_root: &Path, args: &StepsSelectArgs) -> Result<()> {
    let active = write_active_step(project_root, &args.feature, args.line)?;
    println!("{}", serde_json::to_string_pretty(&active)?);
    Ok(())
}

fn unbound(project_root: &Path, args: &StepsFeatureArgs) -> Result<()> {
    let feature = resolve_feature_arg(project_root, args.feature.as_deref())?;
    let steps: Vec<_> = list_feature_step_refs(project_root, &feature)?
        .into_iter()
        .filter(|s| s.status == "unbound")
        .collect();
    println!("{}", serde_json::to_string_pretty(&steps)?);
    Ok(())
}

fn next_unbound(project_root: &Path, args: &StepsFeatureArgs) -> Result<()> {
    let feature = resolve_feature_arg(project_root, args.feature.as_deref())?;
    let Some(step) = first_unbound_feature_step(project_root, &feature)? else {
        println!(
            "{}",
            json!({ "ok": true, "message": "all steps are bound" })
        );
        return Ok(());
    };
    let active = write_active_step(project_root, &feature, step.step_line)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": true,
            "selected": step,
            "active_step": active
        }))?
    );
    Ok(())
}

fn propose(project_root: &Path, args: &StepsProposeArgs) -> Result<()> {
    let started = Instant::now();
    let active = read_active_step(project_root).context("read .teshi/active-step.json")?;
    if let Some(expected_line) = args.line {
        if active.step_line != expected_line {
            return Err(anyhow!(
                "active step line {} does not match --line {expected_line}; \
                 run `teshi steps select --feature '{}' --line {expected_line}`",
                active.step_line,
                active.feature_relative_path
            ));
        }
    } else {
        eprintln!(
            "steps propose: active step L{} — {}",
            active.step_line, active.step_text
        );
    }
    let value = proposal_value(args)?;
    if args.action == "exec"
        && args
            .value_arg
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(anyhow!("exec proposals require --value-arg <command>"));
    }
    if args.action != "open_project" {
        locator_verify_satisfied(project_root, active.step_line as u32, &value, &args.action)?;
    }
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
    let timeout_secs = effective_wait_timeout(args);
    let result = wait_for_step_status(
        project_root,
        match args.until {
            WaitUntilArg::Confirmed => StepWaitUntil::Confirmed,
            WaitUntilArg::Rejected => StepWaitUntil::Rejected,
            WaitUntilArg::Either => StepWaitUntil::Either,
        },
        Duration::from_secs(timeout_secs),
        args.auto_confirm,
    )?;
    debug_log(
        project_root,
        json!({
            "event": "steps_wait_end",
            "until": format!("{:?}", args.until),
            "auto_confirm": args.auto_confirm,
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

fn unbind(project_root: &Path, args: &StepsUnbindArgs) -> Result<()> {
    let removed = unbind_step_binding(project_root, &args.feature, args.line)?;
    match removed {
        Some(binding) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": true,
                    "removed": binding
                }))?
            );
        }
        None => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "ok": true,
                    "message": format!("no binding at line {}", args.line)
                }))?
            );
        }
    }
    Ok(())
}

fn effective_wait_timeout(args: &StepsWaitArgs) -> u64 {
    if args.auto_confirm && args.timeout == 120 {
        60
    } else {
        args.timeout
    }
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
    if args.action == "open_project" {
        return args
            .value_arg
            .clone()
            .or_else(|| args.value.clone())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                anyhow!("open_project proposals require --value-arg <absolute project path>")
            });
    }
    if args.action == "exec" {
        return Ok(args
            .value
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "exec".to_string()));
    }
    args.value
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            anyhow!(
                "{} proposals require --value <selector> (open_project uses --value-arg)",
                args.action
            )
        })
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
