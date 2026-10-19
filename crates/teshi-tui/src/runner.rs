use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use teshi_engine::DaemonManifestExt;

#[derive(Debug, Clone, Default, Deserialize)]
struct RunnerConfigFile {
    runner: Option<RunnerConfigPartial>,
}

#[derive(Debug)]
struct RunnerNotConfigured;

impl std::fmt::Display for RunnerNotConfigured {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("runner cmd missing (set in teshi.toml or TESHI_RUNNER_CMD)")
    }
}

impl std::error::Error for RunnerNotConfigured {}

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

#[derive(Debug, Clone, Serialize)]
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

#[allow(dead_code)]
pub fn load_runner_config(cli: Option<RunnerCliOverride>) -> Result<RunnerConfig> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    load_runner_config_for_project(&cwd, cli)
}

/// Load runner settings relative to an explicit project root.
///
/// TUI project switching must not depend on the process current directory:
/// each candidate project gets its own `teshi.toml`, and a relative runner
/// `cwd` is resolved below that project root.
pub fn load_runner_config_for_project(
    project_root: &Path,
    cli: Option<RunnerCliOverride>,
) -> Result<RunnerConfig> {
    let project_root = project_root.to_path_buf();
    let config_path = project_root.join("teshi.toml");
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

    let cmd = merged
        .cmd
        .filter(|s| !s.trim().is_empty())
        .ok_or(RunnerNotConfigured)?;

    let cwd = merged.cwd.map(PathBuf::from).map(|cwd| {
        if cwd.is_absolute() {
            cwd
        } else {
            project_root.join(cwd)
        }
    });

    Ok(RunnerConfig {
        cmd,
        args: merged.args.unwrap_or_default(),
        cwd,
        env: merged.env.unwrap_or_default(),
    })
}

/// Load an optional project runner configuration while preserving real
/// configuration errors. A project without a runner command is valid; a
/// malformed or unreadable `teshi.toml` must remain visible to callers that
/// are preparing a project switch.
pub fn load_optional_runner_config_for_project(
    project_root: &Path,
    cli: Option<RunnerCliOverride>,
) -> Result<Option<RunnerConfig>> {
    match load_runner_config_for_project(project_root, cli) {
        Ok(config) => Ok(Some(config)),
        Err(error) if error.downcast_ref::<RunnerNotConfigured>().is_some() => Ok(None),
        Err(error) => Err(error),
    }
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
    let (rx, _cancel) = spawn_runner_with_control(config, request)?;
    Ok(rx)
}

/// Spawn an external runner and return both its event stream and an owned
/// cancellation handle. Dropping the event receiver alone never stops the
/// child process.
pub fn spawn_runner_with_control(
    config: RunnerConfig,
    request: RunRequest,
) -> Result<(Receiver<RunEvent>, Sender<()>)> {
    let (tx, rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = mpsc::channel();
    thread::spawn(move || {
        if let Err(err) = run_child(config, request, tx.clone(), cancel_rx) {
            let _ = tx.send(RunEvent::RunnerError {
                message: err.to_string(),
            });
        }
    });
    Ok((rx, cancel_tx))
}

/// Returns whether every selected case has a terminal binding sidecar.
pub fn all_tui_cases(cases: &[RunCase]) -> bool {
    !cases.is_empty()
        && cases.iter().all(|case| {
            Path::new(&case.feature_path)
                .with_extension("tui.json")
                .is_file()
        })
}

/// Resolves the separately packaged Rust terminal runner without invoking Cargo.
pub fn tui_runner_config(command: Option<String>) -> RunnerConfig {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()
                .map(|dir| dir.join(format!("teshi-tui-runner{}", std::env::consts::EXE_SUFFIX)))
        })
        .filter(|path| path.is_file());
    RunnerConfig {
        cmd: command
            .or_else(|| std::env::var("TESHI_TUI_RUNNER").ok())
            .or_else(|| sibling.map(|path| path.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "teshi-tui-runner".into()),
        args: Vec::new(),
        cwd: None,
        env: HashMap::new(),
    }
}

/// Resolve a run's project root without walking from a relative feature path
/// into an unrelated ancestor project. Relative CLI paths are rooted at the
/// caller's working directory; absolute paths still honor an existing
/// `.teshi` marker in their own ancestor chain.
fn run_project_root(input_path: &Path, resolved_path: &Path) -> Result<PathBuf> {
    if !input_path.is_absolute() {
        return std::env::current_dir().context("resolve run working directory");
    }
    Ok(teshi_engine::find_project_root(Some(resolved_path))
        .or_else(|| resolved_path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from(".")))
}

/// Runs TUI scenarios with raw NDJSON output and a failing exit on case failure.
///
/// # Errors
/// Returns source validation, selection, spawn, protocol write or runner exit errors.
pub fn run_tui(path: &Path, scenario: Option<&str>, command: Option<String>) -> Result<()> {
    let input_path = path.to_path_buf();
    let path = std::fs::canonicalize(path)?;
    let root = run_project_root(&input_path, &path)?;
    let validation = teshi_engine::validate_feature_scope(&root, Some(&path))?;
    if validation.has_errors() {
        anyhow::bail!(
            "Feature validation failed: {}",
            serde_json::to_string(&validation)?
        );
    }
    let cases = build_cases_from_path(&path, scenario)?;
    if !all_tui_cases(&cases) {
        anyhow::bail!(
            "TUI execution requires selected scenarios and a .tui.json sidecar for each feature"
        );
    }
    let config = tui_runner_config(command);
    let request = RunRequest {
        command: "run".into(),
        cases,
        meta: HashMap::from([("project_root".into(), root.to_string_lossy().into_owned())]),
    };
    let mut child = Command::new(&config.cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| {
            format!(
                "failed to launch TUI runner {}; install it or set TESHI_TUI_RUNNER",
                config.cmd
            )
        })?;
    let write_result = (|| -> Result<()> {
        let mut stdin = child.stdin.take().context("TUI runner stdin unavailable")?;
        serde_json::to_writer(&mut stdin, &request)?;
        stdin.write_all(b"\n")?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("TUI runner failed with {status}");
    }
    Ok(())
}

/// Walk API/mixed scenarios in-process and emit the same NDJSON event types as an external runner.
pub fn spawn_teshi_dispatch(
    project_root: PathBuf,
    cases: Vec<RunCase>,
) -> Result<Receiver<RunEvent>> {
    let (rx, _cancel) = spawn_teshi_dispatch_with_control(project_root, cases)?;
    Ok(rx)
}

/// Spawn the in-process dispatch path with a best-effort cancellation handle.
/// The dispatch API is synchronous, so cancellation takes effect at the next
/// emitted event and prevents a successful terminal event from being reported.
pub fn spawn_teshi_dispatch_with_control(
    project_root: PathBuf,
    cases: Vec<RunCase>,
) -> Result<(Receiver<RunEvent>, Sender<()>)> {
    let (tx, rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut cancelled = false;
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
            // The compatibility wrapper intentionally drops its sender after
            // spawning.  A disconnected receiver therefore means "no
            // controller", while an explicit unit value is the cancellation
            // request for the controlled path.
            if matches!(cancel_rx.try_recv(), Ok(())) {
                cancelled = true;
                return;
            }
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
            success: !cancelled,
        });
    });
    Ok((rx, cancel_tx))
}

fn run_child(
    config: RunnerConfig,
    request: RunRequest,
    tx: Sender<RunEvent>,
    cancel_rx: Receiver<()>,
) -> Result<()> {
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

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A dedicated group lets cancellation stop only this runner and the
        // subprocesses that inherit its group, including pipe-owning workers.
        cmd.process_group(0);
    }

    let mut child = cmd.spawn().context("failed to spawn runner")?;

    if let Some(mut stdin) = child.stdin.take() {
        let payload = serde_json::to_string(&request)?;
        let _ = stdin.write_all(payload.as_bytes());
        let _ = stdin.write_all(b"\n");
    }

    let stderr_thread = if let Some(stderr) = child.stderr.take() {
        let tx_err = tx.clone();
        Some(thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx_err.send(RunEvent::Log {
                    case_id: None,
                    message: line,
                });
            }
        }))
    } else {
        None
    };

    let stdout_thread = if let Some(stdout) = child.stdout.take() {
        let tx_out = tx.clone();
        Some(thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(event) = parse_event_line(&line) {
                    let _ = tx_out.send(event);
                }
            }
        }))
    } else {
        None
    };

    let status = loop {
        match cancel_rx.try_recv() {
            Ok(()) => {
                match stop_runner_tree(&mut child) {
                    Ok(()) => break child.wait()?,
                    Err(error) => {
                        // Keep ownership and the receiver alive on failure.
                        // The switcher must time out in A, rather than treating
                        // a failed stop request as proof of quiescence.
                        let _ = tx.send(RunEvent::Log {
                            case_id: None,
                            message: format!("Could not stop runner process tree: {error}"),
                        });
                    }
                }
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => {}
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(25));
    };
    if let Some(handle) = stdout_thread {
        let _ = handle.join();
    }
    if let Some(handle) = stderr_thread {
        let _ = handle.join();
    }
    let code = status.code();
    let success = status.success();
    let _ = tx.send(RunEvent::RunnerExit { code, success });
    Ok(())
}

fn stop_runner_tree(child: &mut std::process::Child) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // This PID belongs to the Child we still own. Never select processes
        // by executable name: unrelated runners may use the same interpreter.
        let status = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("stop owned runner tree")?;
        anyhow::ensure!(status.success(), "taskkill exited with {status}");
    }
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("stop owned runner process group")?;
        anyhow::ensure!(status.success(), "process-group kill exited with {status}");
    }
    #[cfg(not(any(windows, unix)))]
    child.kill()?;
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
    pub report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
struct RunStepReport {
    step_id: String,
    text: Option<String>,
    status: String,
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RunCaseReport {
    case_id: String,
    feature_path: String,
    scenario: String,
    status: String,
    duration_ms: Option<u64>,
    error: Option<String>,
    steps: Vec<RunStepReport>,
    attachments: Vec<RunAttachment>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct RunSummary {
    total: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
    blocked: usize,
}

#[derive(Debug, Clone, Serialize)]
struct RunExitReport {
    code: Option<i32>,
    success: bool,
}

#[derive(Debug, Clone, Serialize)]
struct RunReport {
    schema_version: u32,
    feature: String,
    runner: String,
    status: String,
    started_at_unix_ms: u64,
    finished_at_unix_ms: u64,
    summary: RunSummary,
    cases: Vec<RunCaseReport>,
    runner_error: Option<String>,
    runner_exit: Option<RunExitReport>,
}

struct RunReportCollector {
    report: RunReport,
    case_indexes: HashMap<String, usize>,
}

impl RunReportCollector {
    fn new(feature: &Path, runner: &str, cases: &[RunCase]) -> Self {
        let mut case_indexes = HashMap::new();
        let mut reports = Vec::with_capacity(cases.len());
        for case in cases {
            case_indexes.insert(case.id.clone(), reports.len());
            reports.push(RunCaseReport {
                case_id: case.id.clone(),
                feature_path: case.feature_path.clone(),
                scenario: case.scenario.clone(),
                status: "not_started".to_string(),
                duration_ms: None,
                error: None,
                steps: Vec::new(),
                attachments: Vec::new(),
            });
        }
        let now = unix_timestamp_ms();
        Self {
            report: RunReport {
                schema_version: 1,
                feature: feature.to_string_lossy().into_owned(),
                runner: runner.to_string(),
                status: "running".to_string(),
                started_at_unix_ms: now,
                finished_at_unix_ms: now,
                summary: RunSummary {
                    total: reports.len(),
                    ..RunSummary::default()
                },
                cases: reports,
                runner_error: None,
                runner_exit: None,
            },
            case_indexes,
        }
    }

    fn case_mut(&mut self, case_id: &str) -> Option<&mut RunCaseReport> {
        self.case_indexes
            .get(case_id)
            .and_then(|index| self.report.cases.get_mut(*index))
    }

    fn observe(&mut self, event: &RunEvent) {
        match event {
            RunEvent::StartCase { case_id, .. } => {
                if let Some(case) = self.case_mut(case_id) {
                    case.status = "running".to_string();
                }
            }
            RunEvent::StartStep {
                case_id: Some(case_id),
                step_id,
                text,
                ..
            } => {
                if let Some(case) = self.case_mut(case_id) {
                    case.steps.push(RunStepReport {
                        step_id: step_id.clone(),
                        text: text.clone(),
                        status: "running".to_string(),
                        message: None,
                    });
                }
            }
            RunEvent::EndStep {
                case_id: Some(case_id),
                step_id,
                status,
                message,
            } => {
                if let Some(case) = self.case_mut(case_id) {
                    if let Some(step) = case
                        .steps
                        .iter_mut()
                        .rev()
                        .find(|step| step.step_id == *step_id)
                    {
                        step.status = status.clone().unwrap_or_else(|| "unknown".to_string());
                        step.message = message.clone();
                    } else {
                        case.steps.push(RunStepReport {
                            step_id: step_id.clone(),
                            text: None,
                            status: status.clone().unwrap_or_else(|| "unknown".to_string()),
                            message: message.clone(),
                        });
                    }
                }
            }
            RunEvent::CasePassed {
                case_id,
                duration_ms,
            } => {
                if let Some(case) = self.case_mut(case_id) {
                    case.status = "passed".to_string();
                    case.duration_ms = *duration_ms;
                }
            }
            RunEvent::CaseFailed {
                case_id,
                duration_ms,
                error,
            } => {
                if let Some(case) = self.case_mut(case_id) {
                    case.status = "failed".to_string();
                    case.duration_ms = *duration_ms;
                    case.error = Some(error.message.clone());
                    case.attachments.extend(error.attachments.clone());
                }
            }
            RunEvent::CaseSkipped { case_id, reason } => {
                if let Some(case) = self.case_mut(case_id) {
                    let blocked = reason
                        .as_deref()
                        .is_some_and(|value| value.starts_with("blocked:"));
                    case.status = if blocked { "blocked" } else { "skipped" }.to_string();
                    case.error = reason.clone();
                }
            }
            RunEvent::Artifact {
                case_id: Some(case_id),
                kind,
                path,
            } => {
                if let Some(case) = self.case_mut(case_id) {
                    case.attachments.push(RunAttachment {
                        kind: kind.clone(),
                        path: path.clone(),
                    });
                }
            }
            RunEvent::RunnerError { message } => {
                self.report.runner_error = Some(message.clone());
            }
            RunEvent::RunnerExit { code, success } => {
                self.report.runner_exit = Some(RunExitReport {
                    code: *code,
                    success: *success,
                });
            }
            _ => {}
        }
    }

    fn finish(&mut self, exited: bool) {
        let runner_failed = self
            .report
            .runner_exit
            .as_ref()
            .is_some_and(|exit| !exit.success)
            || self.report.runner_error.is_some()
            || !exited;
        for case in &mut self.report.cases {
            if matches!(case.status.as_str(), "not_started" | "running") {
                case.status = "failed".to_string();
                case.error = Some(if runner_failed {
                    "runner did not complete this scenario".to_string()
                } else {
                    "runner completed without reporting a scenario result".to_string()
                });
            }
        }
        self.report.summary = RunSummary {
            total: self.report.cases.len(),
            passed: self
                .report
                .cases
                .iter()
                .filter(|case| case.status == "passed")
                .count(),
            failed: self
                .report
                .cases
                .iter()
                .filter(|case| case.status == "failed")
                .count(),
            skipped: self
                .report
                .cases
                .iter()
                .filter(|case| case.status == "skipped")
                .count(),
            blocked: self
                .report
                .cases
                .iter()
                .filter(|case| case.status == "blocked")
                .count(),
        };
        self.report.status = if self.report.summary.failed > 0 {
            "failed"
        } else if self.report.summary.blocked > 0 {
            "blocked"
        } else {
            "passed"
        }
        .to_string();
        self.report.finished_at_unix_ms = unix_timestamp_ms();
    }

    fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create report directory {}", parent.display()))?;
        }
        let contents = serde_json::to_vec_pretty(&self.report).context("serialize run report")?;
        std::fs::write(path, contents)
            .with_context(|| format!("write run report {}", path.display()))?;
        Ok(())
    }
}

fn unix_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Runs BDD scenarios headlessly using the configured NDJSON runner.
pub fn run_with_options(opts: RunCliOptions) -> Result<()> {
    let uses_default_scope = opts.path.is_none();
    let input_path = opts
        .path
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let feature_path = std::fs::canonicalize(&input_path)
        .with_context(|| format!("resolve feature scope {}", input_path.display()))?;

    // Resolve project root once and reuse
    let project_root = if uses_default_scope {
        std::env::current_dir().context("resolve run working directory")?
    } else {
        run_project_root(&input_path, &feature_path)?
    };

    let validation = teshi_engine::validate_feature_scope(&project_root, Some(&feature_path))?;
    if !validation.diagnostics.is_empty() {
        eprintln!("{}", serde_json::to_string_pretty(&validation)?);
    }
    if validation.has_errors() {
        anyhow::bail!("Feature validation failed; runner was not started");
    }

    let runner_config = load_runner_config_for_project(
        &project_root,
        Some(RunnerCliOverride {
            cmd: opts.runner_cmd.clone(),
            args: opts.runner_args.clone(),
            cwd: opts.runner_cwd.clone(),
        }),
    );
    let runner_config = match runner_config {
        Err(error) if !error.is::<RunnerNotConfigured>() => return Err(error),
        result => result,
    };

    if runner_config
        .as_ref()
        .err()
        .is_some_and(|error| error.is::<RunnerNotConfigured>())
    {
        let selected = build_cases_from_path(&feature_path, opts.scenario.as_deref())?;
        if all_tui_cases(&selected) {
            return run_tui(&feature_path, opts.scenario.as_deref(), None);
        }
    }

    // An explicit NDJSON runner must win over a live daemon and Python engine
    // auto-detect (`import engine.cli` succeeds on many developer machines).
    if runner_config.is_err()
        && let Some(manifest) = teshi_engine::DaemonManifest::load_manifest(&project_root)
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

    if !had_mixed && runner_config.is_err() {
        match crate::engine::load_engine_config(&project_root, None, &[]) {
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

    let config = match runner_config {
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
        project_root.to_string_lossy().to_string(),
    );
    if let Some(report_path) = opts.report_path.as_ref() {
        let artifacts_dir = report_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .join("artifacts");
        meta.insert(
            "artifacts_dir".to_string(),
            artifacts_dir.to_string_lossy().into_owned(),
        );
    }
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
    let report_path = opts.report_path.clone();
    let mut report = report_path
        .as_ref()
        .map(|_| RunReportCollector::new(&feature_path, &config.cmd, &request.cases));
    let rx = spawn_runner(config, request)?;
    let mut failed = false;
    let mut exited = false;
    while let Ok(event) = rx.recv() {
        println!("{}", format_event(&event));
        if let Some(report) = &mut report {
            report.observe(&event);
        }
        failed |= matches!(
            event,
            RunEvent::CaseFailed { .. }
                | RunEvent::RunnerError { .. }
                | RunEvent::RunnerExit { success: false, .. }
        );
        if let RunEvent::EndRun { failed: count, .. } = &event {
            failed |= *count > 0;
        }
        if matches!(event, RunEvent::RunnerExit { .. }) {
            exited = true;
            break;
        }
    }
    if let Some(report) = &mut report {
        report.finish(exited);
        if let Some(path) = report_path {
            report.write(&path)?;
        }
    }
    if failed || !exited {
        anyhow::bail!("test runner failed or exited without a completion status");
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

    #[test]
    fn cli_runner_cmd_loads_without_relying_on_engine() {
        let cfg = load_runner_config(Some(RunnerCliOverride {
            cmd: Some("teshi-requirement-cli-runner".into()),
            args: vec!["--quiet".into()],
            cwd: Some(PathBuf::from(".")),
        }))
        .expect("cli runner override");
        assert_eq!(cfg.cmd, "teshi-requirement-cli-runner");
        assert_eq!(cfg.args, vec!["--quiet"]);
    }

    #[test]
    fn project_runner_config_is_loaded_from_the_explicit_root() {
        let project = tempfile::tempdir().expect("project tempdir");
        std::fs::write(
            project.path().join("teshi.toml"),
            "[runner]\ncmd = \"project-runner\"\ncwd = \".teshi\"\n",
        )
        .expect("write project config");
        let config =
            load_runner_config_for_project(project.path(), None).expect("project runner config");
        assert_eq!(config.cmd, "project-runner");
        assert_eq!(config.cwd, Some(project.path().join(".teshi")));
    }

    #[test]
    fn controlled_runner_can_be_cancelled_while_child_is_quiet() {
        let config = if cfg!(windows) {
            RunnerConfig {
                cmd: "cmd".into(),
                args: vec!["/C".into(), "ping -n 30 127.0.0.1 > NUL".into()],
                cwd: None,
                env: HashMap::new(),
            }
        } else {
            RunnerConfig {
                cmd: "sh".into(),
                args: vec!["-c".into(), "sleep 30".into()],
                cwd: None,
                env: HashMap::new(),
            }
        };
        let request = RunRequest {
            command: "run".into(),
            cases: Vec::new(),
            meta: HashMap::new(),
        };
        let (rx, cancel) = spawn_runner_with_control(config, request).expect("spawn runner");
        std::thread::sleep(Duration::from_millis(100));
        cancel.send(()).expect("send cancellation");
        let event = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("quiet child should acknowledge cancellation");
        assert!(matches!(event, RunEvent::RunnerExit { success: false, .. }));
    }

    #[test]
    fn report_collector_keeps_case_steps_attachments_and_blocked_status() {
        let cases = vec![
            RunCase {
                id: "f0:s0".into(),
                feature_path: "features/sample.feature".into(),
                scenario: "passes".into(),
                line_number: Some(3),
                until_line: Some(5),
            },
            RunCase {
                id: "f0:s1".into(),
                feature_path: "features/sample.feature".into(),
                scenario: "blocked".into(),
                line_number: Some(8),
                until_line: Some(10),
            },
        ];
        let mut collector =
            RunReportCollector::new(Path::new("features/sample.feature"), "runner", &cases);
        collector.observe(&RunEvent::StartCase {
            case_id: "f0:s0".into(),
            name: Some("passes".into()),
        });
        collector.observe(&RunEvent::StartStep {
            case_id: Some("f0:s0".into()),
            step_id: "4".into(),
            text: Some("the step".into()),
            is_api: None,
        });
        collector.observe(&RunEvent::EndStep {
            case_id: Some("f0:s0".into()),
            step_id: "4".into(),
            status: Some("passed".into()),
            message: None,
        });
        collector.observe(&RunEvent::Artifact {
            case_id: Some("f0:s0".into()),
            kind: "text".into(),
            path: "screen.txt".into(),
        });
        collector.observe(&RunEvent::CasePassed {
            case_id: "f0:s0".into(),
            duration_ms: Some(12),
        });
        collector.observe(&RunEvent::CaseSkipped {
            case_id: "f0:s1".into(),
            reason: Some("blocked: browser session unavailable".into()),
        });
        collector.finish(true);

        assert_eq!(collector.report.status, "blocked");
        assert_eq!(collector.report.summary.passed, 1);
        assert_eq!(collector.report.summary.blocked, 1);
        assert_eq!(collector.report.cases[0].steps[0].status, "passed");
        assert_eq!(collector.report.cases[0].attachments[0].path, "screen.txt");
        assert_eq!(collector.report.cases[1].status, "blocked");
    }
}
