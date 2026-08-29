//! API BDD sidecar lifecycle and Teshi mixed-step dispatch.
//!
//! Interactive and mixed runs walk Gherkin steps: `[API]` is executed by
//! `api_service.py`; other steps use existing browser/WinApp locator bindings.

use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use teshi_core::gherkin::{parse_feature, BddFeature, BddStep};
use teshi_core::{
    scenario_engine_mode, scenario_steps, strip_api_marker, validate_feature_scenario, EngineMode,
};

use crate::locator::resolve_step_bindings;
use crate::sidecar::send_sidecar_command_with_timeout;
use crate::venv::{build_import_check_command, resolve_project_venv};

/// Loopback discovery record written by `api_service.py`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEndpoint {
    /// WebSocket URL (`ws://127.0.0.1:port`).
    pub ws_url: String,
    /// Sidecar process id when known.
    #[serde(default)]
    pub pid: Option<u32>,
    /// Project root the sidecar was started with.
    #[serde(default)]
    pub project_root: Option<String>,
}

/// One runnable scenario for the GPUI Run surface and dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnableScenario {
    /// Stable id (`feature_index:scenario_index`).
    pub id: String,
    /// Absolute or project-relative feature path.
    pub feature_path: String,
    /// Scenario title.
    pub name: String,
    /// Feature + scenario tags (scenario last).
    pub tags: Vec<String>,
    /// Resolved engine mode (`api`, `ui`, `mixed`).
    pub engine_mode: String,
}

/// Case identity matching the NDJSON runner protocol.
#[derive(Debug, Clone)]
pub struct DispatchCase {
    /// Runner case id.
    pub id: String,
    /// Feature file path.
    pub feature_path: PathBuf,
    /// Scenario title.
    pub scenario: String,
}

fn engine_mode_str(mode: EngineMode) -> &'static str {
    match mode {
        EngineMode::Api => "api",
        EngineMode::Ui => "ui",
        EngineMode::Mixed => "mixed",
    }
}

/// True when Explore / mixed `teshi run` should walk steps instead of spawning behave.
#[must_use]
pub fn mode_uses_teshi_dispatch(mode: EngineMode) -> bool {
    matches!(mode, EngineMode::Api | EngineMode::Mixed)
}

/// Path to `.teshi/api-endpoint.json` under `project_root`.
#[must_use]
pub fn api_endpoint_path(project_root: &Path) -> PathBuf {
    project_root.join(".teshi").join("api-endpoint.json")
}

/// Read a previously written API sidecar endpoint.
///
/// # Errors
///
/// Returns an error when the file is missing or not valid JSON.
pub fn read_api_endpoint(project_root: &Path) -> Result<ApiEndpoint> {
    let path = api_endpoint_path(project_root);
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

fn ws_port(ws_url: &str) -> Option<u16> {
    let url = ws_url.trim();
    let after = url.rsplit_once(':')?.1;
    let port: u16 = after
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    Some(port)
}

fn api_sidecar_reachable(endpoint: &ApiEndpoint) -> bool {
    let Some(port) = ws_port(&endpoint.ws_url) else {
        return false;
    };
    TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(250)).is_ok()
}

/// Send one JSON command to the API sidecar and return the typed response object.
///
/// # Errors
///
/// Returns an error when the WebSocket handshake, timeout, or payload fails.
pub fn send_api_command(project_root: &Path, command: Value, timeout: Duration) -> Result<Value> {
    let endpoint = read_api_endpoint(project_root)?;
    send_sidecar_command_with_timeout(&endpoint.ws_url, command, timeout)
        .map_err(|err| anyhow!(err))
}

/// Start `api_service.py` if no healthy endpoint exists and return the endpoint.
///
/// # Errors
///
/// Returns an error when Python, the script, or the sidecar handshake fails.
pub fn ensure_api_sidecar(project_root: &Path, script: &Path) -> Result<ApiEndpoint> {
    if let Ok(existing) = read_api_endpoint(project_root) {
        if api_sidecar_reachable(&existing)
            && send_api_command(
                project_root,
                json!({"cmd": "ping", "request_id": "api-ensure"}),
                Duration::from_secs(2),
            )
            .is_ok()
        {
            return Ok(existing);
        }
    }

    if !script.is_file() {
        anyhow::bail!("api_service.py not found at {}", script.display());
    }

    let mut cmd = if let Some(venv) = resolve_project_venv(project_root) {
        build_import_check_command(&venv)
    } else {
        Command::new("python")
    };
    cmd.arg(script).args([
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        "--project",
        &project_root.to_string_lossy(),
    ]);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }
    let mut child = cmd.spawn().context("spawn api_service.py")?;

    let deadline = Instant::now() + Duration::from_secs(15);
    let path = api_endpoint_path(project_root);
    while Instant::now() < deadline {
        if path.is_file() {
            if let Ok(endpoint) = read_api_endpoint(project_root) {
                if api_sidecar_reachable(&endpoint) {
                    let _ = child.stdout.take();
                    std::mem::forget(child);
                    return Ok(endpoint);
                }
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut err = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    use std::io::Read;
                    let _ = stderr.read_to_string(&mut err);
                }
                anyhow::bail!("api sidecar exited ({status}): {}", err.trim());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(err) => anyhow::bail!("wait for api sidecar: {err}"),
        }
    }
    let _ = child.kill();
    anyhow::bail!("timed out waiting for .teshi/api-endpoint.json")
}

/// Stop a sidecar recorded in `api-endpoint.json` (best-effort).
///
/// # Errors
///
/// Returns an error when the endpoint file cannot be read. Kill failures are ignored.
pub fn stop_api_sidecar(project_root: &Path) -> Result<()> {
    let endpoint = read_api_endpoint(project_root)?;
    if let Some(pid) = endpoint.pid {
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(not(windows))]
        {
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
        }
    }
    let _ = fs::remove_file(api_endpoint_path(project_root));
    Ok(())
}

/// List scenarios in a project for the GPUI Run surface.
#[must_use]
pub fn list_runnable_scenarios(project_root: &Path) -> Vec<RunnableScenario> {
    let project = teshi_core::gherkin::parse_project(project_root);
    let mut out = Vec::new();
    for (fi, feature) in project.features.iter().enumerate() {
        for (si, scenario) in feature.all_scenarios().into_iter().enumerate() {
            let mode = scenario_engine_mode(feature, scenario);
            let mut tags = feature.tags.clone();
            tags.extend(scenario.tags.iter().cloned());
            out.push(RunnableScenario {
                id: format!("f{fi}:s{si}"),
                feature_path: feature.file_path.to_string_lossy().into_owned(),
                name: scenario.name.clone(),
                tags,
                engine_mode: engine_mode_str(mode).to_string(),
            });
        }
    }
    out
}

/// Dispatch every case through the Teshi step walker, emitting NDJSON-shaped events.
///
/// # Errors
///
/// Returns an error when a feature file cannot be read. Per-case failures are
/// emitted as `case_failed` events rather than aborting the run.
pub fn dispatch_cases<F>(
    project_root: &Path,
    script: &Path,
    cases: &[DispatchCase],
    mut emit: F,
) -> Result<()>
where
    F: FnMut(Value),
{
    emit(json!({
        "type": "start_run",
        "total": cases.len(),
    }));
    let mut passed = 0usize;
    let mut failed = 0usize;
    let skipped = 0usize;

    for case in cases {
        emit(json!({
            "type": "start_case",
            "case_id": case.id,
            "name": case.scenario,
        }));
        match dispatch_one_case(project_root, script, case, &mut emit) {
            Ok(true) => {
                passed += 1;
                emit(json!({
                    "type": "case_passed",
                    "case_id": case.id,
                }));
            }
            Ok(false) => {
                failed += 1;
            }
            Err(err) => {
                failed += 1;
                emit(json!({
                    "type": "case_failed",
                    "case_id": case.id,
                    "error": {"message": err.to_string(), "attachments": []}
                }));
            }
        }
    }

    emit(json!({
        "type": "end_run",
        "passed": passed,
        "failed": failed,
        "skipped": skipped,
    }));
    Ok(())
}

fn dispatch_one_case<F>(
    project_root: &Path,
    script: &Path,
    case: &DispatchCase,
    emit: &mut F,
) -> Result<bool>
where
    F: FnMut(Value),
{
    let content = fs::read_to_string(&case.feature_path)
        .with_context(|| format!("read {}", case.feature_path.display()))?;
    let feature = parse_feature(&content, case.feature_path.clone());
    let scenario = feature
        .all_scenarios()
        .into_iter()
        .find(|item| item.name == case.scenario)
        .ok_or_else(|| anyhow!("scenario not found: {}", case.scenario))?;

    let mode = match validate_feature_scenario(&feature, scenario) {
        Ok(mode) => mode,
        Err(mismatch) => {
            emit(json!({
                "type": "case_failed",
                "case_id": case.id,
                "error": {"message": mismatch.message, "attachments": []}
            }));
            return Ok(false);
        }
    };

    if mode_uses_teshi_dispatch(mode) {
        ensure_api_sidecar(project_root, script)?;
        let begin = send_api_command(
            project_root,
            json!({
                "cmd": "begin_scenario",
                "request_id": format!("begin-{}", case.id),
                "case_id": case.id,
            }),
            Duration::from_secs(10),
        )?;
        if begin.get("ok") != Some(&Value::Bool(true)) {
            let message = begin
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("begin_scenario failed");
            emit(json!({
                "type": "case_failed",
                "case_id": case.id,
                "error": {"message": message, "attachments": []}
            }));
            return Ok(false);
        }
    }

    for (index, step) in scenario_steps(&feature, scenario).into_iter().enumerate() {
        if !dispatch_one_step(project_root, &feature, step, mode, &case.id, index, emit)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn dispatch_one_step<F>(
    project_root: &Path,
    feature: &BddFeature,
    step: &BddStep,
    mode: EngineMode,
    case_id: &str,
    index: usize,
    emit: &mut F,
) -> Result<bool>
where
    F: FnMut(Value),
{
    let (is_api, stripped) = strip_api_marker(&step.text);
    let step_id = format!("{case_id}:step{index}");
    emit(json!({
        "type": "start_step",
        "case_id": case_id,
        "step_id": step_id,
        "keyword": step.keyword,
        "text": stripped,
        "is_api": is_api,
        "line_number": step.line_number,
    }));

    if is_api {
        let response = send_api_command(
            project_root,
            json!({
                "cmd": "execute_step",
                "request_id": step_id,
                "case_id": case_id,
                "step_id": step_id,
                "text": step.text,
            }),
            Duration::from_secs(120),
        )?;
        if let Some(events) = response.get("events").and_then(Value::as_array) {
            for event in events {
                if event.get("type").and_then(Value::as_str) == Some("start_step") {
                    continue;
                }
                emit(event.clone());
            }
        }
        let ok = response.get("ok") == Some(&Value::Bool(true));
        if !ok {
            let message = response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("API step failed")
                .to_string();
            emit(json!({
                "type": "case_failed",
                "case_id": case_id,
                "error": {"message": message, "attachments": []}
            }));
            return Ok(false);
        }
        return Ok(true);
    }

    if matches!(mode, EngineMode::Api) {
        // validate_feature_scenario already forbids this; keep a hard stop.
        anyhow::bail!("UI step in @api-only scenario");
    }

    execute_ui_step(project_root, feature, step)?;
    emit(json!({
        "type": "end_step",
        "case_id": case_id,
        "step_id": step_id,
        "status": "passed",
    }));
    Ok(true)
}

fn execute_ui_step(project_root: &Path, feature: &BddFeature, step: &BddStep) -> Result<()> {
    let rel = feature_relative(project_root, &feature.file_path);
    let bindings = resolve_step_bindings(project_root, &rel, Some(step.line_number))?;
    let binding = bindings
        .into_iter()
        .find(|item| item.step_line == step.line_number)
        .ok_or_else(|| {
            anyhow!(
                "no confirmed UI step-binding for line {} ({}); API steps do not use step-bindings",
                step.line_number,
                step.text
            )
        })?;

    let endpoint_path = project_root.join(".teshi").join("cdp-endpoint.json");
    let raw = fs::read_to_string(&endpoint_path).with_context(|| {
        format!(
            "read {} (start teshi browser serve-embedded or Connect Chrome/WinApp)",
            endpoint_path.display()
        )
    })?;
    let cdp: Value = serde_json::from_str(&raw)?;
    let ws_url = cdp
        .get("ws_url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("cdp-endpoint.json missing ws_url"))?;
    let command = json!({
        "cmd": "execute_locator",
        "request_id": format!("ui-{}", step.line_number),
        "selector": binding.primary.value,
        "action": binding.primary.action,
        "value": binding.primary.value_arg,
        "timeout_ms": 5000,
    });
    let response = send_sidecar_command_with_timeout(ws_url, command, Duration::from_secs(30))
        .map_err(|err| anyhow!(err))?;
    if response.get("ok") == Some(&Value::Bool(false)) {
        let message = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("UI locator execute failed");
        anyhow::bail!("{message}");
    }
    Ok(())
}

fn feature_relative(project_root: &Path, feature_path: &Path) -> String {
    feature_path
        .strip_prefix(project_root)
        .unwrap_or(feature_path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use teshi_core::gherkin::parse_feature;

    #[test]
    fn list_runnable_scenarios_reports_engine_mode() {
        let dir = tempfile::TempDir::new().unwrap();
        let features = dir.path().join("features");
        fs::create_dir_all(&features).unwrap();
        fs::write(
            features.join("api.feature"),
            "@api\nFeature: API\n  Scenario: Create\n    When [API] I create a user named \"Ada\"\n",
        )
        .unwrap();
        let listed = list_runnable_scenarios(dir.path());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].engine_mode, "api");
        assert_eq!(listed[0].name, "Create");
    }

    #[test]
    fn mismatch_is_detected_before_http() {
        let feature = parse_feature(
            "@api\nFeature: X\n  Scenario: Click\n    When I click save\n",
            PathBuf::from("x.feature"),
        );
        let scenario = &feature.scenarios[0];
        assert!(validate_feature_scenario(&feature, scenario).is_err());
    }
}
