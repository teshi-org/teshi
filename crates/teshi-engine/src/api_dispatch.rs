//! API BDD sidecar lifecycle and Teshi mixed-step dispatch.
//!
//! Interactive and mixed runs walk Gherkin steps: `[API]` is executed by
//! `api_service.py`; other steps use existing browser/WinApp locator bindings.

use std::collections::BTreeSet;
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
use crate::validation::validate_feature_scope;
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
    for case in cases {
        let report = validate_feature_scope(project_root, Some(&case.feature_path))?;
        anyhow::ensure!(
            !report.has_errors(),
            "Feature validation failed before dispatch: {}",
            serde_json::to_string(&report)?
        );
    }
    let run = crate::replay_evidence::ReplayEvidenceRun::create(project_root)?;
    dispatch_recorded(project_root, script, cases, run, &mut emit)
}

/// Start a durable run on a worker thread so GPUI can poll steps while it runs.
///
/// # Errors
/// Returns an error for unknown scenarios or unwritable evidence storage.
pub fn start_replay_run(project: &Path, ids: &[String]) -> Result<String> {
    let cases: Vec<DispatchCase> = list_runnable_scenarios(project)
        .into_iter()
        .filter(|item| ids.contains(&item.id))
        .map(|item| DispatchCase {
            id: item.id,
            feature_path: item.feature_path.into(),
            scenario: item.name,
        })
        .collect();
    anyhow::ensure!(
        !cases.is_empty() && cases.len() == ids.len(),
        "unknown or duplicate scenario selection"
    );
    let run = crate::replay_evidence::ReplayEvidenceRun::create(project)?;
    let id = run.id().to_string();
    let project = project.to_path_buf();
    std::thread::Builder::new()
        .name("replay-evidence".into())
        .spawn(move || {
            if let Err(error) = dispatch_recorded(
                &project,
                &crate::default_api_service_script(),
                &cases,
                run,
                &mut |_| {},
            ) {
                tracing::warn!(%error, "replay worker failed");
            }
        })?;
    Ok(id)
}

fn dispatch_recorded<F>(
    project_root: &Path,
    script: &Path,
    cases: &[DispatchCase],
    mut run: crate::replay_evidence::ReplayEvidenceRun,
    emit: &mut F,
) -> Result<()>
where
    F: FnMut(Value),
{
    let directory = run.directory().to_path_buf();
    let mut persistence_error = None;
    let result = dispatch_cases_inner(project_root, script, cases, &directory, &mut |value| {
        if let Err(error) = run.record(value.clone()) {
            persistence_error = Some(error);
        }
        let mut value = run.snapshot().events.last().cloned().unwrap_or(value);
        if value["type"] == "end_run" {
            value["test_status"] = json!(run.snapshot().test_status);
            value["status"] = json!(run.snapshot().status);
            value["evidence"] = json!(run.snapshot().evidence);
        }
        emit(value);
    });
    if let Err(error) = &result {
        run.record(json!({"type":"run_error", "message":error.to_string()}))?;
        run.record(json!({"type":"end_run", "failed":1}))?;
    }
    if let Some(error) = persistence_error {
        return Err(error);
    }
    result?;
    run.ensure_accepted()?;
    Ok(())
}

fn dispatch_cases_inner<F>(
    project_root: &Path,
    script: &Path,
    cases: &[DispatchCase],
    directory: &Path,
    emit: &mut F,
) -> Result<()>
where
    F: FnMut(Value),
{
    let feature_paths: BTreeSet<PathBuf> =
        cases.iter().map(|case| case.feature_path.clone()).collect();
    for feature_path in feature_paths {
        let report = validate_feature_scope(project_root, Some(&feature_path))?;
        if report.has_errors() {
            anyhow::bail!(
                "Feature validation failed before dispatch: {}",
                serde_json::to_string(&report)?
            );
        }
    }

    let cases = expand_dispatch_cases(cases)?;
    emit(json!({
        "type": "start_run",
        "total": cases.len(),
    }));
    for case in &cases {
        let feature = parse_feature(
            &fs::read_to_string(&case.feature_path)?,
            case.feature_path.clone(),
        );
        if let Some(scenario) = feature
            .all_scenarios()
            .into_iter()
            .find(|s| s.name == case.scenario)
        {
            if !case.feature_path.with_extension("tui.json").is_file() {
                let mode = validate_feature_scenario(&feature, scenario)
                    .map_err(|error| anyhow!(error.message))?;
                for (index, step) in scenario_steps(&feature, scenario).iter().enumerate() {
                    emit(
                        json!({"type":"planned_step","case_id":case.id,"step_id":format!("{}:step{index}",case.id),
                        "feature":feature_relative(project_root,&case.feature_path),"step_line":step.line_number,"text":step.text,
                        "screenshot_required":!matches!(mode,EngineMode::Api)}),
                    );
                }
            } else {
                let content = fs::read_to_string(&case.feature_path)?;
                let rows: Vec<Option<usize>> =
                    if scenario.kind == teshi_core::gherkin::ScenarioKind::ScenarioOutline {
                        scenario
                            .examples
                            .iter()
                            .flat_map(|table| {
                                let first = content
                                    .lines()
                                    .enumerate()
                                    .skip(table.line_number)
                                    .find(|(_, line)| line.trim_start().starts_with('|'))
                                    .map(|(index, _)| index + 2)
                                    .unwrap_or(table.line_number + 2);
                                (0..table.rows.len()).map(move |index| Some(first + index))
                            })
                            .collect()
                    } else {
                        vec![None]
                    };
                for row in rows {
                    let id = row.map_or_else(
                        || case.id.clone(),
                        |line| format!("{}:example-{line}", case.id),
                    );
                    for step in scenario_steps(&feature, scenario) {
                        emit(
                            json!({"type":"planned_step","case_id":id,"step_id":format!("{id}:L{}",step.line_number),
                            "feature":feature_relative(project_root,&case.feature_path),"step_line":step.line_number,"text":step.text,"screenshot_required":true,"source":"pty","example_line":row}),
                        );
                    }
                }
            }
        }
    }
    let mut passed = 0usize;
    let mut failed = 0usize;
    let skipped = 0usize;

    for case in &cases {
        emit(json!({
            "type": "start_case",
            "case_id": case.id,
            "name": case.scenario,
            "feature": feature_relative(project_root, &case.feature_path),
        }));
        match dispatch_one_case(project_root, script, case, directory, emit) {
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

fn expand_dispatch_cases(cases: &[DispatchCase]) -> Result<Vec<DispatchCase>> {
    let mut expanded = Vec::new();
    for case in cases {
        let feature = parse_feature(
            &fs::read_to_string(&case.feature_path)?,
            case.feature_path.clone(),
        );
        let scenario = feature
            .all_scenarios()
            .into_iter()
            .find(|s| s.name == case.scenario)
            .ok_or_else(|| anyhow!("scenario not found"))?;
        if scenario.kind == teshi_core::gherkin::ScenarioKind::ScenarioOutline
            && !case.feature_path.with_extension("tui.json").is_file()
        {
            let mut count = 0;
            for (table_index, table) in scenario.examples.iter().enumerate() {
                for (row_index, _) in table.rows.iter().enumerate() {
                    expanded.push(DispatchCase {
                        id: format!("{}:example-{table_index}-{row_index}", case.id),
                        feature_path: case.feature_path.clone(),
                        scenario: case.scenario.clone(),
                    });
                    count += 1;
                }
            }
            anyhow::ensure!(count > 0, "Scenario Outline has no Examples rows");
        } else {
            expanded.push(case.clone());
        }
    }
    Ok(expanded)
}

fn example_parameters(case: &DispatchCase, feature: &BddFeature) -> Vec<(String, String)> {
    let Some((_, suffix)) = case.id.rsplit_once(":example-") else {
        return Vec::new();
    };
    let Some((table, row)) = suffix
        .split_once('-')
        .and_then(|(t, r)| Some((t.parse::<usize>().ok()?, r.parse::<usize>().ok()?)))
    else {
        return Vec::new();
    };
    feature
        .all_scenarios()
        .into_iter()
        .find(|s| s.name == case.scenario)
        .and_then(|s| s.examples.get(table))
        .and_then(|t| {
            Some(
                t.headers
                    .iter()
                    .cloned()
                    .zip(t.rows.get(row)?.iter().cloned())
                    .collect(),
            )
        })
        .unwrap_or_default()
}

fn substitute_parameters(text: &str, parameters: &[(String, String)]) -> String {
    parameters
        .iter()
        .fold(text.to_string(), |text, (key, value)| {
            text.replace(&format!("<{key}>"), value)
        })
}

struct OwnedTuiReplayChild(std::process::Child);

impl Drop for OwnedTuiReplayChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn dispatch_tui_case<F: FnMut(Value)>(
    project: &Path,
    case: &DispatchCase,
    directory: &Path,
    emit: &mut F,
) -> Result<bool> {
    use std::io::{BufRead, Write};
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()
                .map(|p| p.join(format!("teshi-tui-runner{}", std::env::consts::EXE_SUFFIX)))
        })
        .filter(|p| p.is_file());
    let program = std::env::var_os("TESHI_TUI_RUNNER")
        .map(PathBuf::from)
        .or(sibling)
        .unwrap_or_else(|| "teshi-tui-runner".into());
    let mut child = OwnedTuiReplayChild(
        Command::new(program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("start TUI evidence runner")?,
    );
    let stderr = child.0.stderr.take().map(|stderr| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut bytes = Vec::new();
            let result = stderr.take(1024 * 1024).read_to_end(&mut bytes);
            (result, String::from_utf8_lossy(&bytes).into_owned())
        })
    });
    let request = json!({"command":"run","cases":[{"id":case.id,"feature_path":case.feature_path,"scenario":case.scenario}],
        "meta":{"project_root":project,"evidence_run_directory":directory}});
    if let Some(mut input) = child.0.stdin.take() {
        writeln!(input, "{request}")?;
    }
    let mut complete = false;
    let mut failed = false;
    if let Some(output) = child.0.stdout.take() {
        for line in std::io::BufReader::new(output).lines() {
            let event: Value = serde_json::from_str(&line?)?;
            match event["type"].as_str() {
                Some("start_run" | "start_case" | "case_passed") => {}
                Some("end_run") => {
                    complete = true;
                    failed = event["failed"].as_u64().unwrap_or(0) > 0;
                }
                _ => emit(event),
            }
        }
    }
    let status = child.0.wait()?;
    let diagnostic = if let Some(stderr) = stderr {
        let (result, text) = stderr
            .join()
            .map_err(|_| anyhow!("TUI stderr reader panicked"))?;
        result?;
        if !text.is_empty() {
            emit(json!({"type":"runner_stderr","case_id":case.id,"message":text}));
        }
        text
    } else {
        String::new()
    };
    anyhow::ensure!(
        complete,
        "TUI runner exited without completion: {status}: {diagnostic}"
    );
    Ok(!failed && status.success())
}

fn dispatch_one_case<F>(
    project_root: &Path,
    script: &Path,
    case: &DispatchCase,
    directory: &Path,
    emit: &mut F,
) -> Result<bool>
where
    F: FnMut(Value),
{
    if case.feature_path.with_extension("tui.json").is_file() {
        return dispatch_tui_case(project_root, case, directory, emit);
    }
    let content = fs::read_to_string(&case.feature_path)
        .with_context(|| format!("read {}", case.feature_path.display()))?;
    let original = parse_feature(&content, case.feature_path.clone());
    let parameters = example_parameters(case, &original);
    let feature = parse_feature(
        &substitute_parameters(&content, &parameters),
        case.feature_path.clone(),
    );
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
        if !dispatch_one_step(
            project_root,
            &feature,
            step,
            &parameters,
            mode,
            (&case.id, index),
            directory,
            emit,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn dispatch_one_step<F>(
    project_root: &Path,
    feature: &BddFeature,
    step: &BddStep,
    parameters: &[(String, String)],
    mode: EngineMode,
    identity: (&str, usize),
    directory: &Path,
    emit: &mut F,
) -> Result<bool>
where
    F: FnMut(Value),
{
    let (case_id, index) = identity;
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
        "screenshot_required": !matches!(mode, EngineMode::Api),
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
        )
        .unwrap_or_else(|error| json!({"ok":false,"error":error.to_string()}));
        if let Some(events) = response.get("events").and_then(Value::as_array) {
            for event in events {
                if event.get("type").and_then(Value::as_str) == Some("start_step") {
                    continue;
                }
                emit(event.clone());
            }
        }
        let ok = response.get("ok") == Some(&Value::Bool(true));
        if matches!(mode, EngineMode::Api) {
            emit(
                json!({"type":"step_screenshot","case_id":case_id,"step_id":step_id,
                "capture_status":"not_applicable","screenshot_required":false,"source":"api",
                "message":"API-only step; see recorded exchanges and assertions","response":response}),
            );
        } else {
            emit(capture_ui_evidence(
                project_root,
                feature,
                step,
                case_id,
                &step_id,
                directory,
                if ok { "passed" } else { "failed" },
                "after_step",
            ));
        }
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

    let closing = resolve_step_bindings(
        project_root,
        &feature_relative(project_root, &feature.file_path),
        Some(step.line_number),
    )?
    .iter()
    .any(|binding| {
        binding.primary.capture_before
            || crate::replay_evidence::capture_before_action(&binding.primary.action)
    });
    let before = closing.then(|| {
        capture_ui_evidence(
            project_root,
            feature,
            step,
            case_id,
            &step_id,
            directory,
            "pending",
            "before_close",
        )
    });
    let result = execute_ui_step(project_root, feature, step, parameters);
    let status = if result.is_ok() { "passed" } else { "failed" };
    let mut capture = before.unwrap_or_else(|| {
        capture_ui_evidence(
            project_root,
            feature,
            step,
            case_id,
            &step_id,
            directory,
            status,
            "after_step",
        )
    });
    capture["status"] = json!(status);
    emit(capture);
    emit(json!({
        "type": "end_step",
        "case_id": case_id,
        "step_id": step_id,
        "status": status,
    }));
    result?;
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn capture_ui_evidence(
    project_root: &Path,
    feature: &BddFeature,
    step: &BddStep,
    case_id: &str,
    step_id: &str,
    directory: &Path,
    status: &str,
    timing: &str,
) -> Value {
    let mut capture = json!({"type":"step_screenshot", "case_id":case_id, "step_id":step_id,
        "feature":feature_relative(project_root, &feature.file_path), "step_line":step.line_number,
        "step_keyword":step.keyword, "step_text":step.text, "status":status});
    let endpoint: Value = fs::read(project_root.join(".teshi/cdp-endpoint.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    if endpoint["mode"] == "chrome" {
        capture["capture_status"] = json!("unsupported");
        capture["message"] =
            json!("This backend requires an explicit browser target and screenshot capability.");
    } else {
        match crate::replay_evidence::capture_replay_viewport(project_root).and_then(|bytes| {
            anyhow::ensure!(
                !bytes.is_empty() && bytes.len() <= 16 * 1024 * 1024,
                "invalid screenshot size"
            );
            let file = format!("{}.jpg", uuid::Uuid::new_v4());
            fs::write(directory.join(&file), bytes)?;
            Ok(file)
        }) {
            Ok(file) => {
                capture["capture_status"] = json!("available");
                capture["screenshot_file"] = json!(file);
                capture["captured_at"] = json!(chrono::Utc::now().to_rfc3339());
            }
            Err(error) => {
                capture["capture_status"] = json!("failed");
                capture["message"] = json!(error.to_string());
            }
        }
    }
    capture["capture_timing"] = json!(timing);
    capture["source"] = json!(if endpoint["mode"] == "winapp" {
        "windows_window"
    } else {
        "web_page"
    });
    capture
}

fn execute_ui_step(
    project_root: &Path,
    feature: &BddFeature,
    step: &BddStep,
    parameters: &[(String, String)],
) -> Result<()> {
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
        "selector": substitute_parameters(&binding.primary.value, parameters),
        "action": binding.primary.action,
        "value": binding.primary.value_arg.as_deref().map(|v|substitute_parameters(v,parameters)),
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
    fn failed_ui_step_publishes_its_image_before_failed_run_completion() {
        use base64::Engine as _;
        use std::net::TcpListener;
        let project = tempfile::tempdir().unwrap();
        let feature_path = project.path().join("review.feature");
        fs::write(
            &feature_path,
            "Feature: Capture\n  Scenario: Failed assertion\n    Then marker is visible\n",
        )
        .unwrap();
        let feature = parse_feature(
            &fs::read_to_string(&feature_path).unwrap(),
            feature_path.clone(),
        );
        let step = &feature.scenarios[0].steps[0];
        fs::write(project.path().join("review.bindings.json"), json!({
            "format_version":2,"feature":"review.feature","steps":[{
                "step_line":step.line_number,"step_keyword":step.keyword,"step_text":step.text,
                "step_text_normalized":step.text,"source":"binding","status":"confirmed",
                "primary":{"strategy":"css","value":"button","action":"assert_visible"},"confirmed_at":"fixture"
            }]
        }).to_string()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        fs::create_dir_all(project.path().join(".teshi")).unwrap();
        fs::write(
            project.path().join(".teshi/cdp-endpoint.json"),
            json!({"mode":"embedded", "ws_url":format!("ws://{address}")}).to_string(),
        )
        .unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
        let worker = std::thread::spawn(move || {
            for _ in 0..2 {
                let (socket, _) = listener.accept().unwrap();
                let mut socket = tungstenite::accept(socket).unwrap();
                let request: Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                let mut response = if request["cmd"] == "screenshot" {
                    json!({"ok":true,"screenshot":encoded})
                } else {
                    json!({"ok":false,"error":"marker absent"})
                };
                response["type"] = json!("response");
                response["request_id"] = request["request_id"].clone();
                socket
                    .send(tungstenite::Message::Text(response.to_string()))
                    .unwrap();
            }
        });
        let mut events = Vec::new();
        dispatch_cases(
            project.path(),
            Path::new("unused-api-service"),
            &[DispatchCase {
                id: "case-one".into(),
                feature_path,
                scenario: "Failed assertion".into(),
            }],
            |event| {
                if event["type"] == "step_screenshot" {
                    let history = crate::replay_evidence::list_replay_runs(project.path()).unwrap();
                    assert_eq!(history[0].status, "running");
                    let stored =
                        crate::replay_evidence::load_replay_run(project.path(), &history[0].id)
                            .unwrap();
                    assert!(stored
                        .events
                        .iter()
                        .any(|item| item["type"] == "step_screenshot"));
                    assert!(!crate::replay_evidence::load_replay_image(
                        project.path(),
                        &history[0].id,
                        event["screenshot_file"].as_str().unwrap()
                    )
                    .unwrap()
                    .is_empty());
                }
                events.push(event);
            },
        )
        .unwrap();
        worker.join().unwrap();
        let capture = events
            .iter()
            .position(|event| event["type"] == "step_screenshot")
            .unwrap();
        let failed = events
            .iter()
            .position(|event| event["type"] == "case_failed")
            .unwrap();
        assert!(capture < failed);
        assert_eq!(events[capture]["status"], "failed");
        assert_eq!(events[capture]["capture_status"], "available");
        assert_eq!(
            crate::replay_evidence::list_replay_runs(project.path()).unwrap()[0].status,
            "failed"
        );
    }

    #[test]
    fn capture_busy_does_not_repeat_actions_or_skip_later_steps() {
        use base64::Engine as _;
        use std::net::TcpListener;
        let project = tempfile::tempdir().unwrap();
        let path = project.path().join("capture.feature");
        fs::write(&path,"Feature: Evidence\n  Scenario: Continue\n    When first action\n    Then second action\n").unwrap();
        let steps:Vec<Value>=[(3,"When","first action"),(4,"Then","second action")].into_iter().map(|(line,keyword,text)|json!({
            "step_line":line,"step_keyword":keyword,"step_text":text,"step_text_normalized":text,"source":"binding","status":"confirmed",
            "primary":{"strategy":"css","value":"button","action":"click"}})).collect();
        fs::write(
            project.path().join("capture.bindings.json"),
            json!({"format_version":2,"feature":"capture.feature","steps":steps}).to_string(),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        fs::create_dir_all(project.path().join(".teshi")).unwrap();
        fs::write(
            project.path().join(".teshi/cdp-endpoint.json"),
            json!({"mode":"embedded","ws_url":format!("ws://{}",listener.local_addr().unwrap())})
                .to_string(),
        )
        .unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
        let worker = std::thread::spawn(move || {
            let mut commands = Vec::new();
            for index in 0..4 {
                let (socket, _) = listener.accept().unwrap();
                let mut socket = tungstenite::accept(socket).unwrap();
                let request: Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                commands.push(request["cmd"].as_str().unwrap().to_owned());
                let mut response = if index == 1 {
                    json!({"ok":false,"code":"executor_busy","error":"executor_busy"})
                } else if request["cmd"] == "screenshot" {
                    json!({"ok":true,"screenshot":encoded})
                } else {
                    json!({"ok":true})
                };
                response["type"] = json!("response");
                response["request_id"] = request["request_id"].clone();
                socket
                    .send(tungstenite::Message::Text(response.to_string()))
                    .unwrap();
            }
            commands
        });
        let mut events = Vec::new();
        let result = dispatch_cases(
            project.path(),
            Path::new("unused"),
            &[DispatchCase {
                id: "case".into(),
                feature_path: path,
                scenario: "Continue".into(),
            }],
            |event| events.push(event),
        );
        assert!(result.is_err());
        assert_eq!(
            worker.join().unwrap(),
            [
                "execute_locator",
                "screenshot",
                "execute_locator",
                "screenshot"
            ]
        );
        let history = crate::replay_evidence::list_replay_runs(project.path()).unwrap();
        assert_eq!(history[0].test_status, "passed");
        assert_eq!(history[0].status, "incomplete");
        assert_eq!(
            (
                history[0].evidence.required,
                history[0].evidence.saved,
                history[0].evidence.failed
            ),
            (2, 1, 1)
        );
        let capture = events
            .iter()
            .find(|e| e["capture_status"] == "failed")
            .unwrap();
        assert!(capture["message"]
            .as_str()
            .unwrap()
            .contains("executor_busy"));
        assert_eq!(events.last().unwrap()["status"], "incomplete");
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

    #[test]
    fn dispatch_fails_before_emitting_run_events_for_invalid_source() {
        let dir = tempfile::TempDir::new().unwrap();
        let features = dir.path().join("features");
        fs::create_dir_all(&features).unwrap();
        fs::write(
            features.join("broken.feature"),
            "Feature: Broken\n  Scenario: Missing separator\n    Givenready\n",
        )
        .unwrap();

        let mut events = Vec::new();
        let result = dispatch_cases(
            dir.path(),
            Path::new("runner-that-must-not-start"),
            &[DispatchCase {
                id: "s0".into(),
                feature_path: PathBuf::from("features/broken.feature"),
                scenario: "Missing separator".into(),
            }],
            |event| events.push(event),
        );

        let error = result.expect_err("invalid Feature must stop dispatch");
        assert!(error.to_string().contains("missing_step_separator"));
        assert!(
            events.is_empty(),
            "dispatch emitted events before validation"
        );
    }
}
