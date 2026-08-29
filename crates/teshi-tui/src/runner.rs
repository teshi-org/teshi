use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use teshi_engine::DaemonManifestExt;

#[derive(Debug, Clone, Default, Deserialize)]
struct RunnerConfigFile {
    runner: Option<RunnerConfigPartial>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RunnerConfigPartial {
    cmd: Option<String>,
    args: Option<Vec<String>>,
    cwd: Option<String>,
    env: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct RunnerCliOverride {
    pub cmd: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunCase {
    pub id: String,
    pub feature_path: String,
    pub scenario: String,
    pub line_number: Option<usize>,
    /// Last Gherkin step line (inclusive) for WinApp replay filtering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until_line: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunRequest {
    pub command: String,
    pub cases: Vec<RunCase>,
    pub meta: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct RunAttachment {
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct RunError {
    pub message: String,
    pub stack: Option<String>,
    pub attachments: Vec<RunAttachment>,
}

/// Events from the external runner process (NDJSON). Some fields are reserved for the protocol
/// and are not yet surfaced in the UI.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum RunEvent {
    StartRun {
        run_id: Option<String>,
        total: Option<usize>,
    },
    StartCase {
        case_id: String,
        name: Option<String>,
    },
    CasePassed {
        case_id: String,
        duration_ms: Option<u64>,
    },
    CaseFailed {
        case_id: String,
        duration_ms: Option<u64>,
        error: RunError,
    },
    CaseSkipped {
        case_id: String,
        reason: Option<String>,
    },
    Log {
        case_id: Option<String>,
        message: String,
    },
    Artifact {
        case_id: Option<String>,
        kind: String,
        path: String,
    },
    EndRun {
        passed: usize,
        failed: usize,
        skipped: usize,
    },
    StartStep {
        case_id: Option<String>,
        step_id: String,
        text: Option<String>,
        is_api: Option<bool>,
    },
    EndStep {
        case_id: Option<String>,
        step_id: String,
        status: Option<String>,
        message: Option<String>,
    },
    HttpExchange(Box<teshi_core::HttpExchange>),
    RunnerExit {
        code: Option<i32>,
        success: bool,
    },
    RunnerError {
        message: String,
    },
}

pub fn load_runner_config(cli: Option<RunnerCliOverride>) -> Result<RunnerConfig> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config_path = cwd.join("teshi.toml");
    let mut base = RunnerConfigPartial::default();
    if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let file: RunnerConfigFile = toml::from_str(&raw)
            .with_context(|| format!("failed to parse {}", config_path.display()))?;
        if let Some(cfg) = file.runner {
            base = cfg;
        }
    }

    let mut merged = base;

    let env_override = RunnerConfigPartial {
        cmd: std::env::var("TESHI_RUNNER_CMD").ok(),
        args: std::env::var("TESHI_RUNNER_ARGS")
            .ok()
            .map(|s| s.split_whitespace().map(|v| v.to_string()).collect()),
        cwd: std::env::var("TESHI_RUNNER_CWD").ok(),
        env: None,
    };
    merged = merge_config(merged, env_override);

    if let Some(cli) = cli {
        let cli_partial = RunnerConfigPartial {
            cmd: cli.cmd,
            args: if cli.args.is_empty() {
                None
            } else {
                Some(cli.args)
            },
            cwd: cli.cwd.map(|p| p.to_string_lossy().to_string()),
            env: None,
        };
        merged = merge_config(merged, cli_partial);
    }

    let cmd = merged.cmd.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
        anyhow::anyhow!("runner cmd missing (set in teshi.toml or TESHI_RUNNER_CMD)")
    })?;

    Ok(RunnerConfig {
        cmd,
        args: merged.args.unwrap_or_default(),
        cwd: merged.cwd.map(PathBuf::from),
        env: merged.env.unwrap_or_default(),
    })
}

fn merge_config(
    base: RunnerConfigPartial,
    override_cfg: RunnerConfigPartial,
) -> RunnerConfigPartial {
    RunnerConfigPartial {
        cmd: override_cfg.cmd.or(base.cmd),
        args: override_cfg.args.or(base.args),
        cwd: override_cfg.cwd.or(base.cwd),
        env: match (base.env, override_cfg.env) {
            (None, None) => None,
            (Some(m), None) => Some(m),
            (None, Some(m)) => Some(m),
            (Some(mut m), Some(o)) => {
                m.extend(o);
                Some(m)
            }
        },
    }
}

pub fn spawn_runner(config: RunnerConfig, request: RunRequest) -> Result<Receiver<RunEvent>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Err(err) = run_child(config, request, tx.clone()) {
            let _ = tx.send(RunEvent::RunnerError {
                message: err.to_string(),
            });
        }
    });
    Ok(rx)
}

/// Walk API/mixed scenarios in-process and emit the same NDJSON event types as an external runner.
pub fn spawn_teshi_dispatch(
    project_root: PathBuf,
    cases: Vec<RunCase>,
) -> Result<Receiver<RunEvent>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let dispatch: Vec<teshi_engine::DispatchCase> = cases
            .iter()
            .map(|case| teshi_engine::DispatchCase {
                id: case.id.clone(),
                feature_path: PathBuf::from(&case.feature_path),
                scenario: case.scenario.clone(),
            })
            .collect();
        let script = teshi_engine::default_api_service_script();
        let emit = |value: serde_json::Value| {
            if let Some(event) = parse_event_line(&value.to_string()) {
                let _ = tx.send(event);
            }
        };
        if let Err(err) = teshi_engine::dispatch_cases(&project_root, &script, &dispatch, emit) {
            let _ = tx.send(RunEvent::RunnerError {
                message: err.to_string(),
            });
        }
        let _ = tx.send(RunEvent::RunnerExit {
            code: Some(0),
            success: true,
        });
    });
    Ok(rx)
}

fn run_child(config: RunnerConfig, request: RunRequest, tx: Sender<RunEvent>) -> Result<()> {
    let mut cmd = Command::new(&config.cmd);
    cmd.args(&config.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &config.cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in &config.env {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().context("failed to spawn runner")?;

    if let Some(mut stdin) = child.stdin.take() {
        let payload = serde_json::to_string(&request)?;
        let _ = stdin.write_all(payload.as_bytes());
        let _ = stdin.write_all(b"\n");
    }

    if let Some(stderr) = child.stderr.take() {
        let tx_err = tx.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx_err.send(RunEvent::Log {
                    case_id: None,
                    message: line,
                });
            }
        });
    }

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(event) = parse_event_line(&line) {
                let _ = tx.send(event);
            }
        }
    }

    let status = child.wait()?;
    let code = status.code();
    let success = status.success();
    let _ = tx.send(RunEvent::RunnerExit { code, success });
    Ok(())
}

pub(crate) fn parse_event_line(line: &str) -> Option<RunEvent> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let kind = value.get("type")?.as_str()?;
    match kind {
        "start_run" => Some(RunEvent::StartRun {
            run_id: value
                .get("run_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            total: value
                .get("total")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize),
        }),
        "start_case" => Some(RunEvent::StartCase {
            case_id: value.get("case_id")?.as_str()?.to_string(),
            name: value
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        }),
        "case_passed" => Some(RunEvent::CasePassed {
            case_id: value.get("case_id")?.as_str()?.to_string(),
            duration_ms: value.get("duration_ms").and_then(|v| v.as_u64()),
        }),
        "case_failed" => Some(RunEvent::CaseFailed {
            case_id: value.get("case_id")?.as_str()?.to_string(),
            duration_ms: value.get("duration_ms").and_then(|v| v.as_u64()),
            error: parse_error(value.get("error")),
        }),
        "case_skipped" => Some(RunEvent::CaseSkipped {
            case_id: value.get("case_id")?.as_str()?.to_string(),
            reason: value
                .get("reason")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        }),
        "log" => Some(RunEvent::Log {
            case_id: value
                .get("case_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            message: value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        }),
        "artifact" => Some(RunEvent::Artifact {
            case_id: value
                .get("case_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            kind: value
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("artifact")
                .to_string(),
            path: value
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        }),
        "end_run" => Some(RunEvent::EndRun {
            passed: value.get("passed").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            failed: value.get("failed").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            skipped: value.get("skipped").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        }),
        "start_step" => Some(RunEvent::StartStep {
            case_id: value
                .get("case_id")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            step_id: value.get("step_id")?.as_str()?.to_string(),
            text: value
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            is_api: value.get("is_api").and_then(|v| v.as_bool()),
        }),
        "end_step" => Some(RunEvent::EndStep {
            case_id: value
                .get("case_id")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            step_id: value.get("step_id")?.as_str()?.to_string(),
            status: value
                .get("status")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            message: value
                .get("message")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }),
        "http_exchange" => teshi_core::HttpExchange::from_value(&value)
            .ok()
            .map(|exchange| RunEvent::HttpExchange(Box::new(exchange))),
        _ => None,
    }
}

fn parse_error(value: Option<&serde_json::Value>) -> RunError {
    let message = value
        .and_then(|v| v.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown error")
        .to_string();
    let stack = value
        .and_then(|v| v.get("stack"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let mut attachments = Vec::new();
    if let Some(items) = value
        .and_then(|v| v.get("attachments"))
        .and_then(|v| v.as_array())
    {
        for item in items {
            let kind = item
                .get("type")
                .or_else(|| item.get("kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("artifact")
                .to_string();
            let path = item
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            attachments.push(RunAttachment { kind, path });
        }
    }
    RunError {
        message,
        stack,
        attachments,
    }
}

pub struct RunCliOptions {
    pub path: Option<PathBuf>,
    pub scenario: Option<String>,
    pub runner_cmd: Option<String>,
    pub runner_args: Vec<String>,
    pub runner_cwd: Option<PathBuf>,
}

/// Runs BDD scenarios headlessly using the configured NDJSON runner.
pub fn run_with_options(opts: RunCliOptions) -> Result<()> {
    let feature_path = opts
        .path
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Resolve project root once and reuse
    let project_root = teshi_engine::find_project_root(Some(&feature_path))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Try daemon first
    if let Some(manifest) = teshi_engine::DaemonManifest::load_manifest(&project_root)
        && manifest.is_daemon_alive()
    {
        return run_via_daemon(&manifest, &feature_path, opts);
    }

    // Fallback: Teshi mixed dispatch, then engine / generic runner
    let cases = build_cases_from_path(&feature_path, opts.scenario.as_deref())?;
    if cases.is_empty() {
        return Err(anyhow::anyhow!("no scenarios found to run"));
    }
    let (mixed, rest) = classify_cases(&cases)?;
    let had_mixed = !mixed.is_empty();
    if !mixed.is_empty() {
        drain_dispatch_events(&project_root, mixed)?;
    }
    if rest.is_empty() {
        return Ok(());
    }

    if !had_mixed {
        match crate::engine::load_engine_config(
            &project_root,
            opts.runner_cmd.as_deref(),
            &opts.runner_args,
        ) {
            Ok(engine_config) => {
                return crate::engine::run_feature_with_engine(
                    &engine_config,
                    &feature_path,
                    &opts,
                );
            }
            Err(_engine_err) => {}
        }
    }

    let config = match load_runner_config(Some(RunnerCliOverride {
        cmd: opts.runner_cmd,
        args: opts.runner_args,
        cwd: opts.runner_cwd,
    })) {
        Ok(config) => config,
        Err(err) => {
            let all_api = rest
                .iter()
                .all(|case| case_engine_mode(case) == Some(teshi_core::EngineMode::Api));
            if all_api {
                return drain_dispatch_events(&project_root, rest);
            }
            return Err(err);
        }
    };
    let mut meta = HashMap::new();
    meta.insert(
        "project_root".to_string(),
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .to_string_lossy()
            .to_string(),
    );
    if let Some(mode) = read_sidecar_mode(&feature_path) {
        match mode.as_str() {
            "winapp" | "embedded" | "chrome" => {
                meta.insert("runner_mode".to_string(), mode);
            }
            _ => {}
        }
    }
    let request = RunRequest {
        command: "run".to_string(),
        cases: rest,
        meta,
    };
    let rx = spawn_runner(config, request)?;
    while let Ok(event) = rx.recv() {
        println!("{}", format_event(&event));
        if matches!(event, RunEvent::RunnerExit { .. }) {
            break;
        }
    }
    Ok(())
}

/// POST run request to daemon and stream NDJSON events to stdout.
fn run_via_daemon(
    manifest: &teshi_engine::DaemonManifest,
    feature_path: &Path,
    opts: RunCliOptions,
) -> Result<()> {
    let url = format!("http://127.0.0.1:{}/api/v1/daemon/run", manifest.port);
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "feature_path": feature_path.to_string_lossy(),
            "scenario": opts.scenario,
        }))
        .send()
        .context("send run request to daemon")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        anyhow::bail!("daemon returned {status}: {body}");
    }

    // Stream NDJSON lines to stdout
    use std::io::{BufRead, BufReader, Write};
    let reader = BufReader::new(resp);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    for line in reader.lines() {
        let line = line.context("read daemon response")?;
        writeln!(handle, "{line}")?;
    }
    Ok(())
}

pub fn build_cases_from_path(path: &Path, scenario_filter: Option<&str>) -> Result<Vec<RunCase>> {
    let mut cases = Vec::new();
    if path.is_dir() {
        let project = teshi_core::gherkin::parse_project(path);
        for (fi, feature) in project.features.iter().enumerate() {
            collect_cases(&mut cases, fi, feature, scenario_filter);
        }
    } else {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let feature = teshi_core::gherkin::parse_feature(&content, path.to_path_buf());
        collect_cases(&mut cases, 0, &feature, scenario_filter);
    }
    Ok(cases)
}

fn collect_cases(
    cases: &mut Vec<RunCase>,
    feature_idx: usize,
    feature: &teshi_core::gherkin::BddFeature,
    scenario_filter: Option<&str>,
) {
    for (si, scenario) in feature.all_scenarios().into_iter().enumerate() {
        if let Some(name) = scenario_filter
            && scenario.name != name
        {
            continue;
        }
        let until_line = scenario.steps.last().map(|s| s.line_number);
        cases.push(RunCase {
            id: format!("f{feature_idx}:s{si}"),
            feature_path: feature.file_path.to_string_lossy().to_string(),
            scenario: scenario.name.clone(),
            line_number: Some(scenario.line_number),
            until_line,
        });
    }
}

fn read_sidecar_mode(path: &Path) -> Option<String> {
    let project_root = resolve_run_project_root(path)?;
    let endpoint = project_root.join(".teshi").join("cdp-endpoint.json");
    let text = std::fs::read_to_string(endpoint).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("mode")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Locates the BDD project root for headless runs (cwd or ancestors of the feature path).
fn resolve_run_project_root(path: &Path) -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir()
        && cwd.join(".teshi").join("cdp-endpoint.json").is_file()
    {
        return Some(cwd);
    }
    let mut dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    loop {
        if dir.join(".teshi").join("cdp-endpoint.json").is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn format_event(event: &RunEvent) -> String {
    match event {
        RunEvent::StartRun { total, .. } => {
            format!("start_run total={}", total.unwrap_or(0))
        }
        RunEvent::StartCase { case_id, name } => {
            format!("start_case {case_id} {}", name.clone().unwrap_or_default())
        }
        RunEvent::CasePassed { case_id, .. } => format!("case_passed {case_id}"),
        RunEvent::CaseFailed { case_id, error, .. } => {
            format!("case_failed {case_id} {}", error.message)
        }
        RunEvent::CaseSkipped { case_id, .. } => format!("case_skipped {case_id}"),
        RunEvent::Log { message, .. } => format!("log {message}"),
        RunEvent::Artifact { kind, path, .. } => format!("artifact {kind} {path}"),
        RunEvent::EndRun {
            passed,
            failed,
            skipped,
        } => {
            format!("end_run passed={passed} failed={failed} skipped={skipped}")
        }
        RunEvent::StartStep { step_id, text, .. } => {
            format!("start_step {step_id} {}", text.clone().unwrap_or_default())
        }
        RunEvent::EndStep {
            step_id, status, ..
        } => format!("end_step {step_id} {}", status.clone().unwrap_or_default()),
        RunEvent::HttpExchange(exchange) => format!(
            "http_exchange {} {} {}",
            exchange.method, exchange.url, exchange.exchange_id
        ),
        RunEvent::RunnerExit { code, success } => {
            format!("runner_exit code={:?} success={success}", code)
        }
        RunEvent::RunnerError { message } => format!("runner_error {message}"),
    }
}

fn drain_dispatch_events(project_root: &Path, cases: Vec<RunCase>) -> Result<()> {
    let rx = spawn_teshi_dispatch(project_root.to_path_buf(), cases)?;
    while let Ok(event) = rx.recv() {
        println!("{}", format_event(&event));
        if matches!(event, RunEvent::RunnerExit { .. }) {
            break;
        }
    }
    Ok(())
}

fn case_engine_mode(case: &RunCase) -> Option<teshi_core::EngineMode> {
    let content = std::fs::read_to_string(&case.feature_path).ok()?;
    let feature = teshi_core::gherkin::parse_feature(&content, PathBuf::from(&case.feature_path));
    let scenario = feature
        .all_scenarios()
        .into_iter()
        .find(|item| item.name == case.scenario)?;
    Some(teshi_core::scenario_engine_mode(&feature, scenario))
}

fn classify_cases(cases: &[RunCase]) -> Result<(Vec<RunCase>, Vec<RunCase>)> {
    let mut mixed = Vec::new();
    let mut rest = Vec::new();
    for case in cases {
        match case_engine_mode(case) {
            Some(teshi_core::EngineMode::Mixed) => mixed.push(case.clone()),
            _ => rest.push(case.clone()),
        }
    }
    Ok((mixed, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_event_line_keeps_legacy_case_events() {
        let event =
            parse_event_line(r#"{"type":"case_passed","case_id":"f0:s0","duration_ms":12}"#)
                .expect("parse");
        assert!(matches!(event, RunEvent::CasePassed { case_id, .. } if case_id == "f0:s0"));
    }

    #[test]
    fn parse_event_line_reads_http_exchange() {
        let line = r#"{"type":"http_exchange","exchange_id":"e1","case_id":"c1","step_id":"s1","template":"create_user.json.j2","method":"POST","url":"https://example.test/users","request_headers":{"Authorization":"***"},"request_body":{"name":"Ada"},"status":201,"response_headers":{},"response_body":{"id":"42"},"duration_ms":9,"extract":{"user_id":"42"},"asserts":[{"name":"status_ok","passed":true}],"redacted":true}"#;
        let event = parse_event_line(line).expect("parse exchange");
        let RunEvent::HttpExchange(exchange) = event else {
            panic!("expected http_exchange");
        };
        assert_eq!(exchange.exchange_id, "e1");
        assert_eq!(exchange.method, "POST");
        assert!(exchange.redacted);
        assert_eq!(
            exchange
                .request_headers
                .get("Authorization")
                .and_then(|v| v.as_str()),
            Some("***")
        );
    }

    #[test]
    fn parse_event_line_ignores_unknown_types() {
        assert!(parse_event_line(r#"{"type":"not_a_real_event"}"#).is_none());
    }
}
