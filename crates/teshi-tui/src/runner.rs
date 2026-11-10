use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use teshi_engine::run_routing::{
    BuiltinExecutor, RouteDecision, RouteRequest, RunPlan, RunRoutes, RunnerKind,
};
use teshi_engine::runner_protocol::{RUNNER_PROTOCOL_VERSION, RunnerHandshake};

/// An external process that speaks runner protocol v1.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Executable to spawn.
    pub cmd: String,
    /// Arguments passed to the executable.
    pub args: Vec<String>,
    /// Working directory; inherits Teshi's when `None`.
    pub cwd: Option<PathBuf>,
    /// Extra environment variables.
    pub env: HashMap<String, String>,
}

/// How one batch of scenarios is executed.
#[derive(Debug, Clone)]
pub enum Executor {
    /// In-process step walker for API and mixed scenarios.
    Dispatch,
    /// An external protocol process, including the built-in TUI runner.
    Process(RunnerConfig),
}

/// One executor and the cases it runs, in request order.
#[derive(Debug, Clone)]
pub struct ExecBatch {
    /// Rule name shown in logs and errors.
    pub name: String,
    /// How the cases are executed.
    pub executor: Executor,
    /// Cases for this executor.
    pub cases: Vec<RunCase>,
}

/// A run that failed only because external targets were unavailable.
///
/// `teshi run` maps this to its own exit code so CI can tell "go connect the
/// browser" apart from assertion failures.
#[derive(Debug, thiserror::Error)]
#[error("{blocked} scenario(s) blocked by unavailable targets; none failed")]
pub struct RunBlocked {
    /// Number of blocked scenarios.
    pub blocked: usize,
}

/// Exit code of `teshi run` when scenarios were blocked but none failed.
pub const RUN_BLOCKED_EXIT_CODE: i32 = 3;

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
    /// Runner protocol version this request speaks.
    pub protocol_version: u64,
    pub cases: Vec<RunCase>,
    pub meta: HashMap<String, String>,
}

impl RunRequest {
    /// A v1 `run` request.
    #[must_use]
    pub fn run(cases: Vec<RunCase>, meta: HashMap<String, String>) -> Self {
        Self {
            command: "run".into(),
            protocol_version: RUNNER_PROTOCOL_VERSION,
            cases,
            meta,
        }
    }
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
    /// The scenario could not start because its external target is unavailable.
    CaseBlocked {
        case_id: String,
        reason: Option<String>,
        category: Option<String>,
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
        blocked: usize,
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

/// Resolves a routing rule's executor.
#[must_use]
pub fn executor_for(kind: &RunnerKind) -> Executor {
    match kind {
        RunnerKind::Builtin(BuiltinExecutor::Dispatch) => Executor::Dispatch,
        RunnerKind::Builtin(BuiltinExecutor::Tui) => Executor::Process(tui_runner_config(None)),
        RunnerKind::External {
            cmd,
            args,
            cwd,
            env,
        } => Executor::Process(RunnerConfig {
            cmd: cmd.clone(),
            args: args.clone(),
            cwd: cwd.clone(),
            env: env.clone(),
        }),
    }
}

fn route_requests(cases: &[RunCase]) -> Vec<RouteRequest> {
    cases
        .iter()
        .map(|case| RouteRequest {
            id: case.id.clone(),
            feature_path: PathBuf::from(&case.feature_path),
            scenario: case.scenario.clone(),
        })
        .collect()
}

fn batches_from_plan(plan: RunPlan, cases: &[RunCase]) -> Vec<ExecBatch> {
    plan.batches
        .into_iter()
        .map(|batch| ExecBatch {
            executor: executor_for(&batch.rule.kind),
            name: batch.rule.name,
            cases: batch
                .case_ids
                .iter()
                .filter_map(|id| cases.iter().find(|case| &case.id == id).cloned())
                .collect(),
        })
        .collect()
}

/// Loads a project's `[[runners]]`, keeping a configuration error as text so
/// the project still opens and the error surfaces when a run is requested.
pub fn load_project_routes(project_root: &Path) -> std::result::Result<RunRoutes, String> {
    teshi_engine::run_routing::load_run_routes(project_root).map_err(|error| error.to_string())
}

/// Loads a project's `[[runners]]` for project preparation.
///
/// # Errors
/// An unreadable or syntactically broken `teshi.toml` fails preparation, as it
/// would for any other project file. Migration and rule errors (the removed
/// `[runner]` section, `TESHI_RUNNER_*` variables, invalid rules) are kept in
/// the returned value instead, so they cannot make every project unopenable.
pub fn prepare_project_routes(
    project_root: &Path,
) -> Result<std::result::Result<RunRoutes, String>> {
    use teshi_engine::run_routing::RouteConfigError;
    match teshi_engine::run_routing::load_run_routes(project_root) {
        Ok(routes) => Ok(Ok(routes)),
        Err(error @ (RouteConfigError::Read { .. } | RouteConfigError::Parse { .. })) => {
            Err(error.into())
        }
        Err(error) => Ok(Err(error.to_string())),
    }
}

/// Routes and starts an interactive run rooted at `project_root`.
///
/// # Errors
/// Returns the stored configuration error or the routing error; nothing is
/// started unless every case routes to exactly one runner.
pub fn start_routed_run(
    project_root: &Path,
    routes: &std::result::Result<RunRoutes, String>,
    cases: Vec<RunCase>,
) -> Result<(Receiver<RunEvent>, Sender<()>)> {
    let routes = routes
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let batches = plan_batches(project_root, routes, &cases)?;
    let meta = HashMap::from([(
        "project_root".to_string(),
        project_root.to_string_lossy().into_owned(),
    )]);
    Ok(spawn_routed_run(project_root.to_path_buf(), batches, meta))
}

/// Routes cases through the project's `[[runners]]` into executor batches.
///
/// # Errors
/// Returns the routing error when no runner is configured or any case does not
/// match exactly one rule.
pub fn plan_batches(
    project_root: &Path,
    routes: &RunRoutes,
    cases: &[RunCase],
) -> Result<Vec<ExecBatch>> {
    let plan = routes.plan(project_root, &route_requests(cases))?;
    Ok(batches_from_plan(plan, cases))
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
fn all_tui_cases(cases: &[RunCase]) -> bool {
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
    let request = RunRequest::run(
        cases,
        HashMap::from([("project_root".into(), root.to_string_lossy().into_owned())]),
    );
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
        let mut gate = ProtocolGate::default();
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
            // A disconnected receiver means "no controller"; an explicit unit
            // value is the cancellation request. Cancellation and protocol
            // violations are sticky: the synchronous walker cannot be stopped,
            // so its remaining events are suppressed instead.
            if cancelled || matches!(cancel_rx.try_recv(), Ok(())) {
                cancelled = true;
                return;
            }
            if gate.admit(&value).is_err() {
                return;
            }
            if let Some(event) = parse_event_line(&value.to_string()) {
                let _ = tx.send(event);
            }
        };
        let result = teshi_engine::dispatch_cases(&project_root, &script, &dispatch, emit);
        let error = gate
            .violation
            .clone()
            .or_else(|| result.err().map(|error| error.to_string()));
        if let Some(message) = error {
            let _ = tx.send(RunEvent::RunnerError { message });
        }
        let _ = tx.send(RunEvent::RunnerExit {
            code: Some(0),
            success: !cancelled && gate.violation.is_none(),
        });
    });
    Ok((rx, cancel_tx))
}

/// Enforces runner protocol v1 on one executor's event stream.
///
/// Only JSON objects with a string `type` are protocol events; other output
/// is diagnostic text and never counts toward or against the handshake.
#[derive(Debug, Default)]
struct ProtocolGate {
    handshake: Option<RunnerHandshake>,
    violation: Option<String>,
}

impl ProtocolGate {
    /// Admits one protocol event. The first violation is sticky and every
    /// later event is rejected with the same message.
    fn admit(&mut self, event: &serde_json::Value) -> Result<(), String> {
        if let Some(violation) = &self.violation {
            return Err(violation.clone());
        }
        let result = match &self.handshake {
            None => RunnerHandshake::from_first_event(event).map(|handshake| {
                self.handshake = Some(handshake);
            }),
            Some(handshake) => handshake.check_event(event),
        };
        result.map_err(|error| {
            let message = error.to_string();
            self.violation = Some(message.clone());
            message
        })
    }
}

fn is_protocol_event(value: &serde_json::Value) -> bool {
    value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some()
}

fn run_child(
    config: RunnerConfig,
    mut request: RunRequest,
    tx: Sender<RunEvent>,
    cancel_rx: Receiver<()>,
) -> Result<()> {
    let root = request
        .meta
        .get("project_root")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let mut evidence = teshi_engine::replay_evidence::ReplayEvidenceRun::create(&root)?;
    let directory = evidence.directory().to_path_buf();
    request.meta.insert(
        "evidence_run_directory".into(),
        directory.to_string_lossy().into_owned(),
    );
    evidence.record(serde_json::json!({"type":"runner_command","command":config.cmd,"args":config.args,"cwd":config.cwd}))?;
    let mut nonvisual = std::collections::HashSet::new();
    // The published plan owns step identity for adapters that report a source
    // line instead, so one execution is never counted as a planned gap twice.
    let mut planned: HashMap<String, std::collections::BTreeSet<u64>> = HashMap::new();
    for case in &request.cases {
        let path = if Path::new(&case.feature_path).is_absolute() {
            PathBuf::from(&case.feature_path)
        } else {
            root.join(&case.feature_path)
        };
        if let Ok(source) = std::fs::read_to_string(&path) {
            let feature = teshi_core::parse_feature(&source, path);
            if let Some(scenario) = feature
                .all_scenarios()
                .into_iter()
                .find(|s| s.name == case.scenario)
            {
                let nonvisual_case =
                    request
                        .meta
                        .get("runner_mode")
                        .is_some_and(|mode| mode == "cli")
                        || feature.tags.iter().chain(&scenario.tags).any(|tag| {
                            matches!(tag.trim_start_matches('@'), "cli" | "logic" | "unit")
                        })
                        || matches!(
                            teshi_core::scenario_engine_mode(&feature, scenario),
                            teshi_core::EngineMode::Api
                        );
                if nonvisual_case {
                    nonvisual.insert(case.id.clone());
                }
                // Specialized TUI runners expand Examples and publish their own plan.
                if !Path::new(&case.feature_path)
                    .with_extension("tui.json")
                    .is_file()
                {
                    for step in teshi_core::scenario_steps(&feature, scenario) {
                        planned
                            .entry(case.id.clone())
                            .or_default()
                            .insert(step.line_number as u64);
                        evidence.record(serde_json::json!({"type":"planned_step","case_id":case.id,"step_id":format!("{}:L{}",case.id,step.line_number),
                            "step_line":step.line_number,"feature":case.feature_path,"screenshot_required":!nonvisual_case}))?;
                    }
                }
            }
        }
    }
    let stdout_file = std::fs::File::create(directory.join("stdout.ndjson"))?;
    let stderr_file = std::fs::File::create(directory.join("stderr.txt"))?;
    let evidence = std::sync::Arc::new(std::sync::Mutex::new(evidence));
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
        let evidence = evidence.clone();
        Some(thread::spawn(move || {
            let mut saved = stderr_file;
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if let Err(error) = writeln!(saved, "{line}") {
                    let _ = tx_err.send(RunEvent::RunnerError {
                        message: error.to_string(),
                    });
                }
                let recorded = evidence
                    .lock()
                    .map_err(|_| anyhow::anyhow!("evidence writer poisoned"))
                    .and_then(|mut run| {
                        run.record(serde_json::json!({"type":"stderr","message":line}))
                    });
                if let Err(error) = recorded {
                    let _ = tx_err.send(RunEvent::RunnerError {
                        message: error.to_string(),
                    });
                }
                let _ = tx_err.send(RunEvent::Log {
                    case_id: None,
                    message: line,
                });
            }
        }))
    } else {
        None
    };

    let protocol_violated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stdout_thread = if let Some(stdout) = child.stdout.take() {
        let tx_out = tx.clone();
        let evidence = evidence.clone();
        let protocol_violated = protocol_violated.clone();
        Some(thread::spawn(move || {
            let mut gate = ProtocolGate::default();
            let mut saved = stdout_file;
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if let Err(error) = writeln!(saved, "{line}") {
                    let _ = tx_out.send(RunEvent::RunnerError {
                        message: error.to_string(),
                    });
                }
                if gate.violation.is_some() {
                    // Output after a violation is preserved in stdout.ndjson
                    // but never interpreted as results.
                    continue;
                }
                let parsed = serde_json::from_str::<serde_json::Value>(&line)
                    .ok()
                    .filter(is_protocol_event);
                if let Some(event) = &parsed {
                    let was_handshaken = gate.handshake.is_some();
                    match gate.admit(event) {
                        Ok(()) if !was_handshaken => {
                            if let (Some(handshake), Ok(mut run)) =
                                (&gate.handshake, evidence.lock())
                            {
                                let _ = run.record(serde_json::json!({
                                    "type": "runner_handshake",
                                    "protocol_version": RUNNER_PROTOCOL_VERSION,
                                    "capabilities": handshake.capability_names(),
                                }));
                            }
                        }
                        Ok(()) => {}
                        Err(message) => {
                            if let Ok(mut run) = evidence.lock() {
                                let _ = run.record(serde_json::json!({
                                    "type": "protocol_error",
                                    "message": message,
                                }));
                            }
                            let _ = tx_out.send(RunEvent::RunnerError {
                                message: format!("runner protocol violation: {message}"),
                            });
                            protocol_violated.store(true, std::sync::atomic::Ordering::SeqCst);
                            continue;
                        }
                    }
                }
                let value =
                    parsed.unwrap_or_else(|| serde_json::json!({"type":"stdout","message":line}));
                if let Ok(mut run) = evidence.lock() {
                    let mut value = value;
                    if let Some(step_id) = planned_step_id(&value, &planned) {
                        value["step_id"] = serde_json::json!(step_id);
                    }
                    if value["case_id"]
                        .as_str()
                        .is_some_and(|id| nonvisual.contains(id))
                    {
                        value["screenshot_required"] = serde_json::json!(false);
                        if value["type"] == "end_step" {
                            let mut capture = value.clone();
                            capture["type"] = serde_json::json!("step_screenshot");
                            capture["capture_status"] = serde_json::json!("not_applicable");
                            capture["source"] = serde_json::json!("cli");
                            capture["message"] = serde_json::json!(
                                "nonvisual_step; see stdout.ndjson, stderr.txt and assertion events"
                            );
                            if let Err(error) = run.record(capture) {
                                let _ = tx_out.send(RunEvent::RunnerError {
                                    message: error.to_string(),
                                });
                            }
                        }
                    }
                    if let Err(error) = run.record(value) {
                        let _ = tx_out.send(RunEvent::RunnerError {
                            message: error.to_string(),
                        });
                    }
                }
                if let Some(event) = parse_event_line(&line) {
                    let _ = tx_out.send(event);
                }
            }
            gate
        }))
    } else {
        None
    };

    let mut stopped_for_violation = false;
    let mut cancelled = false;
    let status = loop {
        // A violating runner is stopped once; its exit is then awaited below.
        if !stopped_for_violation && protocol_violated.load(std::sync::atomic::Ordering::SeqCst) {
            stopped_for_violation = true;
            if let Err(error) = stop_runner_tree(&mut child) {
                let _ = tx.send(RunEvent::Log {
                    case_id: None,
                    message: format!("Could not stop runner process tree: {error}"),
                });
            }
        }
        match cancel_rx.try_recv() {
            Ok(()) => {
                cancelled = true;
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
    let gate = stdout_thread
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    if let Some(handle) = stderr_thread {
        let _ = handle.join();
    }
    let mut run = evidence
        .lock()
        .map_err(|_| anyhow::anyhow!("evidence writer poisoned"))?;
    run.record(serde_json::json!({"type":"runner_exit","exit_code":status.code(),"stdout_file":"stdout.ndjson","stderr_file":"stderr.txt"}))?;
    let handshake_missing = gate.handshake.is_none() && gate.violation.is_none() && !cancelled;
    if handshake_missing {
        let message = format!(
            "runner exited before the protocol v{RUNNER_PROTOCOL_VERSION} handshake; \
             its first event must be start_run with protocol_version and capabilities"
        );
        run.record(serde_json::json!({"type":"protocol_error","message":message}))?;
        let _ = tx.send(RunEvent::RunnerError { message });
    }
    if run.snapshot().test_status == "running" {
        run.record(serde_json::json!({"type":"end_run","status":"interrupted","reason":"runner_exited_without_completion"}))?;
    }
    // Missing evidence only taints a passing run. A run that failed, was
    // blocked, or never completed is unsuccessful on its own status.
    let accepted = run.snapshot().acceptance_status == "passed"
        && gate.violation.is_none()
        && !handshake_missing;
    if status.success() && run.snapshot().rejected() {
        let _ = tx.send(RunEvent::RunnerError {
            message: format!(
                "Replay acceptance {} (tests: {}, evidence: {:?}); run {}",
                run.snapshot().acceptance_status,
                run.snapshot().test_status,
                run.snapshot().evidence,
                run.id()
            ),
        });
    }
    let code = status.code();
    let success = status.success() && accepted;
    let _ = tx.send(RunEvent::RunnerExit { code, success });
    Ok(())
}

/// Map an adapter's step identity onto the plan published for its case.
///
/// Adapters may report the source step line as `step_line` or reuse it as
/// `step_id`. The plan is authoritative, so a planned step and its execution
/// share one identity instead of counting as a planned gap plus a missing
/// capture. Canonical identities and cases without a plan are left untouched.
fn planned_step_id(
    value: &serde_json::Value,
    planned: &HashMap<String, std::collections::BTreeSet<u64>>,
) -> Option<String> {
    let case = value["case_id"].as_str()?;
    let lines = planned.get(case)?;
    let line = value["step_line"]
        .as_u64()
        .or_else(|| value["step_id"].as_str().and_then(|id| id.parse().ok()))?;
    lines.contains(&line).then(|| format!("{case}:L{line}"))
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
        "case_blocked" => Some(RunEvent::CaseBlocked {
            case_id: value.get("case_id")?.as_str()?.to_string(),
            reason: value
                .get("reason")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            category: value
                .get("category")
                .and_then(|v| v.as_str())
                .map(str::to_string),
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
            blocked: value.get("blocked").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
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

/// Options for headless `teshi run`.
#[derive(Debug, Default)]
pub struct RunCliOptions {
    /// Feature file or directory; defaults to the working directory.
    pub path: Option<PathBuf>,
    /// Only run the scenario with this exact name.
    pub scenario: Option<String>,
    /// Ad-hoc external runner that replaces `[[runners]]` routing for this run.
    pub runner_cmd: Option<String>,
    /// Arguments for `runner_cmd`.
    pub runner_args: Vec<String>,
    /// Working directory for `runner_cmd`, relative to the project root.
    pub runner_cwd: Option<PathBuf>,
    /// Optional JSON report destination.
    pub report_path: Option<PathBuf>,
    /// Print the routing decision for every scenario without executing.
    pub explain: bool,
    /// Emit `explain` output as JSON.
    pub json: bool,
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
                    case.status = "skipped".to_string();
                    case.error = reason.clone();
                }
            }
            RunEvent::CaseBlocked {
                case_id,
                reason,
                category,
            } => {
                if let Some(case) = self.case_mut(case_id) {
                    case.status = "blocked".to_string();
                    case.error = Some(blocked_message(reason.as_deref(), category.as_deref()));
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

    let cases = build_cases_from_path(&feature_path, opts.scenario.as_deref())?;
    if cases.is_empty() {
        anyhow::bail!("no scenarios found to run");
    }

    let batches = if let Some(cmd) = opts.runner_cmd.clone() {
        anyhow::ensure!(
            !opts.explain,
            "--explain inspects [[runners]] routing and cannot be combined with --runner-cmd"
        );
        let cwd = opts.runner_cwd.as_ref().map(|cwd| project_root.join(cwd));
        vec![ExecBatch {
            name: "--runner-cmd".into(),
            executor: Executor::Process(RunnerConfig {
                cmd,
                args: opts.runner_args.clone(),
                cwd,
                env: HashMap::new(),
            }),
            cases,
        }]
    } else {
        let routes = teshi_engine::run_routing::load_run_routes(&project_root)?;
        if opts.explain {
            return explain_routes(&project_root, &routes, &cases, opts.json);
        }
        plan_batches(&project_root, &routes, &cases)?
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
    let runner_names = batches
        .iter()
        .map(|batch| batch.name.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let all_cases: Vec<RunCase> = batches
        .iter()
        .flat_map(|batch| batch.cases.iter().cloned())
        .collect();
    let report_path = opts.report_path.clone();
    let mut report = report_path
        .as_ref()
        .map(|_| RunReportCollector::new(&feature_path, &runner_names, &all_cases));
    let (rx, _cancel) = spawn_routed_run(project_root, batches, meta);
    let mut failed = false;
    let mut blocked = 0;
    let mut exited = false;
    while let Ok(event) = rx.recv() {
        println!("{}", format_event(&event));
        if let Some(report) = &mut report {
            report.observe(&event);
        }
        failed |= matches!(
            event,
            RunEvent::CaseFailed { .. } | RunEvent::RunnerError { .. }
        );
        if let RunEvent::EndRun {
            failed: count,
            blocked: blocked_count,
            ..
        } = &event
        {
            failed |= *count > 0;
            blocked = *blocked_count;
        }
        if matches!(
            event,
            RunEvent::RunnerExit { .. } | RunEvent::RunnerError { .. }
        ) {
            exited = matches!(event, RunEvent::RunnerExit { .. });
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
    if blocked > 0 {
        return Err(RunBlocked { blocked }.into());
    }
    Ok(())
}

/// Prints how every selected scenario routes, without executing anything.
fn explain_routes(
    project_root: &Path,
    routes: &RunRoutes,
    cases: &[RunCase],
    json: bool,
) -> Result<()> {
    let decisions = routes.resolve(project_root, &route_requests(cases));
    if json {
        println!("{}", serde_json::to_string_pretty(&decisions)?);
    } else {
        for route in &decisions {
            let decision = match &route.decision {
                RouteDecision::Routed { runner } => format!("-> {runner}"),
                RouteDecision::Unmatched => "UNMATCHED".to_string(),
                RouteDecision::Ambiguous { runners } => {
                    format!("AMBIGUOUS ({})", runners.join(", "))
                }
                RouteDecision::Unreadable { reason } => format!("UNREADABLE ({reason})"),
            };
            println!(
                "{} :: {} [{}] {decision}",
                route.feature,
                route.scenario,
                route.tags.join(" ")
            );
        }
    }
    let unrouted = decisions
        .iter()
        .filter(|route| !matches!(route.decision, RouteDecision::Routed { .. }))
        .count();
    anyhow::ensure!(
        unrouted == 0,
        "{unrouted} scenario(s) do not route to exactly one runner"
    );
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
        RunEvent::CaseBlocked {
            case_id,
            reason,
            category,
        } => format!(
            "case_blocked {case_id} {}",
            blocked_message(reason.as_deref(), category.as_deref())
        ),
        RunEvent::Log { message, .. } => format!("log {message}"),
        RunEvent::Artifact { kind, path, .. } => format!("artifact {kind} {path}"),
        RunEvent::EndRun {
            passed,
            failed,
            skipped,
            blocked,
        } => {
            format!("end_run passed={passed} failed={failed} skipped={skipped} blocked={blocked}")
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

/// Human-readable blocked reason, prefixed with its category when known.
pub fn blocked_message(reason: Option<&str>, category: Option<&str>) -> String {
    let reason = reason.unwrap_or("target unavailable");
    match category {
        Some(category) => format!("[{category}] {reason}"),
        None => reason.to_string(),
    }
}

/// Final classification of one case inside a routed run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaseOutcome {
    Passed,
    Failed,
    Skipped,
    Blocked,
}

fn case_outcome(event: &RunEvent) -> Option<(&str, CaseOutcome)> {
    match event {
        RunEvent::CasePassed { case_id, .. } => Some((case_id, CaseOutcome::Passed)),
        RunEvent::CaseFailed { case_id, .. } => Some((case_id, CaseOutcome::Failed)),
        RunEvent::CaseSkipped { case_id, .. } => Some((case_id, CaseOutcome::Skipped)),
        RunEvent::CaseBlocked { case_id, .. } => Some((case_id, CaseOutcome::Blocked)),
        _ => None,
    }
}

fn spawn_batch(
    batch: &ExecBatch,
    project_root: &Path,
    meta: &HashMap<String, String>,
) -> Result<(Receiver<RunEvent>, Sender<()>)> {
    match &batch.executor {
        Executor::Dispatch => {
            spawn_teshi_dispatch_with_control(project_root.to_path_buf(), batch.cases.clone())
        }
        Executor::Process(config) => {
            let mut meta = meta.clone();
            meta.insert("runner_name".into(), batch.name.clone());
            spawn_runner_with_control(config.clone(), RunRequest::run(batch.cases.clone(), meta))
        }
    }
}

/// Runs routed batches sequentially as one logical run.
///
/// The returned stream has exactly one `StartRun`, one aggregated `EndRun`
/// (including `blocked`), and one terminal event: `RunnerError` when any
/// executor failed to run or violated the protocol (consumers stop reading at
/// that point), otherwise `RunnerExit`. Every case receives a terminal case
/// event: cases an executor never reported fail, and cases left behind by a
/// cancellation are skipped. Sending on the returned handle cancels the active
/// batch and skips the remaining ones.
pub fn spawn_routed_run(
    project_root: PathBuf,
    batches: Vec<ExecBatch>,
    meta: HashMap<String, String>,
) -> (Receiver<RunEvent>, Sender<()>) {
    let (tx, rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        let total = batches.iter().map(|batch| batch.cases.len()).sum();
        let _ = tx.send(RunEvent::StartRun {
            run_id: None,
            total: Some(total),
        });
        let mut outcomes: HashMap<String, CaseOutcome> = HashMap::new();
        let mut errors = Vec::new();
        let mut all_exits_ok = true;
        let mut cancelled = false;
        for batch in &batches {
            if !cancelled {
                cancelled = matches!(cancel_rx.try_recv(), Ok(()));
            }
            if !cancelled {
                let _ = tx.send(RunEvent::Log {
                    case_id: None,
                    message: format!("runner {}: {} scenario(s)", batch.name, batch.cases.len()),
                });
                match spawn_batch(batch, &project_root, &meta) {
                    Ok((batch_rx, batch_cancel)) => {
                        let exit_ok = forward_batch(
                            batch,
                            &batch_rx,
                            &batch_cancel,
                            &cancel_rx,
                            &tx,
                            &mut outcomes,
                            &mut errors,
                            &mut cancelled,
                        );
                        all_exits_ok &= exit_ok;
                    }
                    Err(error) => {
                        all_exits_ok = false;
                        errors.push(format!("{}: {error:#}", batch.name));
                    }
                }
            }
            // Every case must end with a result so summaries stay complete.
            for case in &batch.cases {
                if outcomes.contains_key(&case.id) {
                    continue;
                }
                let event = if cancelled {
                    outcomes.insert(case.id.clone(), CaseOutcome::Skipped);
                    RunEvent::CaseSkipped {
                        case_id: case.id.clone(),
                        reason: Some("run cancelled".into()),
                    }
                } else {
                    outcomes.insert(case.id.clone(), CaseOutcome::Failed);
                    RunEvent::CaseFailed {
                        case_id: case.id.clone(),
                        duration_ms: None,
                        error: RunError {
                            message: format!("runner {} did not report a result", batch.name),
                            stack: None,
                            attachments: Vec::new(),
                        },
                    }
                };
                let _ = tx.send(event);
            }
        }
        let count = |wanted: CaseOutcome| outcomes.values().filter(|o| **o == wanted).count();
        let failed = count(CaseOutcome::Failed);
        let blocked = count(CaseOutcome::Blocked);
        let _ = tx.send(RunEvent::EndRun {
            passed: count(CaseOutcome::Passed),
            failed,
            skipped: count(CaseOutcome::Skipped),
            blocked,
        });
        if errors.is_empty() {
            let _ = tx.send(RunEvent::RunnerExit {
                code: None,
                success: all_exits_ok && !cancelled && failed == 0 && blocked == 0,
            });
        } else {
            let _ = tx.send(RunEvent::RunnerError {
                message: errors.join("; "),
            });
        }
    });
    (rx, cancel_tx)
}

/// Forwards one batch's events into the routed stream and returns whether the
/// executor exited successfully.
#[allow(clippy::too_many_arguments)] // Shared orchestration state threaded through one loop.
fn forward_batch(
    batch: &ExecBatch,
    batch_rx: &Receiver<RunEvent>,
    batch_cancel: &Sender<()>,
    cancel_rx: &Receiver<()>,
    tx: &Sender<RunEvent>,
    outcomes: &mut HashMap<String, CaseOutcome>,
    errors: &mut Vec<String>,
    cancelled: &mut bool,
) -> bool {
    let mut exit_ok = false;
    loop {
        if !*cancelled && matches!(cancel_rx.try_recv(), Ok(())) {
            *cancelled = true;
            let _ = batch_cancel.send(());
        }
        let event = match batch_rx.recv_timeout(Duration::from_millis(25)) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match event {
            // The orchestrator owns run framing for the merged stream.
            RunEvent::StartRun { .. } | RunEvent::EndRun { .. } => {}
            RunEvent::RunnerExit { success, .. } => {
                exit_ok = success;
                break;
            }
            RunEvent::RunnerError { message } => {
                let _ = tx.send(RunEvent::Log {
                    case_id: None,
                    message: format!("runner {} error: {message}", batch.name),
                });
                errors.push(format!("{}: {message}", batch.name));
            }
            event => {
                if let Some((case_id, outcome)) = case_outcome(&event)
                    && batch.cases.iter().any(|case| case.id == case_id)
                {
                    outcomes.insert(case_id.to_string(), outcome);
                }
                let _ = tx.send(event);
            }
        }
    }
    exit_ok
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
    fn test_parse_event_line_reads_case_blocked_and_blocked_count() {
        let event = parse_event_line(
            r#"{"type":"case_blocked","case_id":"c1","reason":"no browser","category":"target_unavailable"}"#,
        )
        .expect("parse case_blocked");
        assert!(
            matches!(&event, RunEvent::CaseBlocked { case_id, reason: Some(reason), category: Some(category) }
                if case_id == "c1" && reason == "no browser" && category == "target_unavailable"),
            "unexpected event: {event:?}"
        );
        let end =
            parse_event_line(r#"{"type":"end_run","passed":1,"failed":0,"skipped":0,"blocked":2}"#)
                .expect("parse end_run");
        assert!(
            matches!(end, RunEvent::EndRun { blocked: 2, .. }),
            "unexpected event: {end:?}"
        );
    }

    #[test]
    fn test_project_routes_keep_legacy_config_error_for_run_time() {
        let project = tempfile::tempdir().expect("project tempdir");
        std::fs::write(
            project.path().join("teshi.toml"),
            "[runner]\ncmd = \"project-runner\"\n",
        )
        .expect("write project config");
        let routes = load_project_routes(project.path());
        let error = routes
            .as_ref()
            .expect_err("legacy [runner] must be rejected");
        assert!(
            error.contains("[[runners]]"),
            "missing migration hint: {error}"
        );
        let started = start_routed_run(project.path(), &routes, Vec::new());
        assert!(started.is_err(), "a legacy config must not start a run");
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
        let request = RunRequest::run(Vec::new(), HashMap::new());
        let (rx, cancel) = spawn_runner_with_control(config, request).expect("spawn runner");
        std::thread::sleep(Duration::from_millis(100));
        cancel.send(()).expect("send cancellation");
        let event = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("quiet child should acknowledge cancellation");
        assert!(
            matches!(event, RunEvent::RunnerExit { success: false, .. }),
            "unexpected cancellation event: {event:?}"
        );
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
        collector.observe(&RunEvent::CaseBlocked {
            case_id: "f0:s1".into(),
            reason: Some("browser session unavailable".into()),
            category: Some("target_unavailable".into()),
        });
        collector.finish(true);

        assert_eq!(collector.report.status, "blocked");
        assert_eq!(collector.report.summary.passed, 1);
        assert_eq!(collector.report.summary.blocked, 1);
        assert_eq!(collector.report.cases[0].steps[0].status, "passed");
        assert_eq!(collector.report.cases[0].attachments[0].path, "screen.txt");
        assert_eq!(collector.report.cases[1].status, "blocked");
        assert_eq!(
            collector.report.cases[1].error.as_deref(),
            Some("[target_unavailable] browser session unavailable")
        );
    }
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

    #[test]
    fn evidence_runner_child() {
        if std::env::var_os("TESHI_EVIDENCE_RUNNER_CHILD").is_none() {
            return;
        }
        println!(
            "{}",
            serde_json::json!({"type":"start_run","total":1,"protocol_version":1,"capabilities":["steps"]})
        );
        println!(
            "{}",
            serde_json::json!({"type":"end_step","case_id":"case","step_id":"case:L3","status":"passed"})
        );
        println!(
            "{}",
            serde_json::json!({"type":"end_run","passed":1,"failed":0,"skipped":0})
        );
    }

    #[test]
    fn external_runner_preserves_cli_output_and_blocks_visual_missing_evidence() {
        for nonvisual in [true, false] {
            let project = tempfile::tempdir().unwrap();
            std::fs::write(
                project.path().join("case.feature"),
                "Feature: Evidence\n  Scenario: Output\n    Then output exists\n",
            )
            .unwrap();
            let config = RunnerConfig {
                cmd: std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                args: vec![
                    "--exact".into(),
                    "runner::evidence_tests::evidence_runner_child".into(),
                    "--nocapture".into(),
                ],
                cwd: None,
                env: HashMap::from([("TESHI_EVIDENCE_RUNNER_CHILD".into(), "1".into())]),
            };
            let mut meta = HashMap::from([(
                "project_root".into(),
                project.path().to_string_lossy().into_owned(),
            )]);
            if nonvisual {
                meta.insert("runner_mode".into(), "cli".into());
            }
            let request = RunRequest::run(
                vec![RunCase {
                    id: "case".into(),
                    feature_path: "case.feature".into(),
                    scenario: "Output".into(),
                    line_number: None,
                    until_line: None,
                }],
                meta,
            );
            let (tx, rx) = mpsc::channel();
            let (_cancel, cancel_rx) = mpsc::channel();
            run_child(config, request, tx, cancel_rx).unwrap();
            let events: Vec<_> = rx.try_iter().collect();
            assert!(events.iter().any(
                |event| matches!(event,RunEvent::RunnerExit{success,..} if *success==nonvisual)
            ));
            let runs = teshi_engine::replay_evidence::list_replay_runs(project.path()).unwrap();
            assert_eq!(runs[0].test_status, "passed");
            assert_eq!(
                runs[0].status,
                if nonvisual { "passed" } else { "incomplete" }
            );
            let directory = project
                .path()
                .join(".teshi/logs/replay-screenshots")
                .join(&runs[0].id);
            assert!(
                std::fs::read_to_string(directory.join("stdout.ndjson"))
                    .unwrap()
                    .contains("end_step")
            );
            assert!(directory.join("stderr.txt").is_file());
        }
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;

    const CHILD_MODE: &str = "TESHI_PROTOCOL_CHILD_MODE";
    const CHILD_CASE: &str = "TESHI_PROTOCOL_CHILD_CASE";

    /// Re-entered as a runner process; the mode selects its protocol behavior.
    #[test]
    fn protocol_runner_child() {
        let (Ok(mode), Ok(case)) = (std::env::var(CHILD_MODE), std::env::var(CHILD_CASE)) else {
            return;
        };
        let handshake = |capabilities: &[&str]| {
            println!(
                "{}",
                serde_json::json!({"type":"start_run","total":1,"protocol_version":1,"capabilities":capabilities})
            );
        };
        match mode.as_str() {
            "pass" => {
                handshake(&[]);
                println!(
                    "{}",
                    serde_json::json!({"type":"case_passed","case_id":case})
                );
            }
            "blocked" => {
                handshake(&["blocked"]);
                println!(
                    "{}",
                    serde_json::json!({"type":"case_blocked","case_id":case,"reason":"no session","category":"target_unavailable"})
                );
            }
            "undeclared" => {
                handshake(&[]);
                println!(
                    "{}",
                    serde_json::json!({"type":"case_blocked","case_id":case,"reason":"no session"})
                );
            }
            "silent" => {
                handshake(&[]);
            }
            _ => {
                println!(
                    "{}",
                    serde_json::json!({"type":"case_passed","case_id":case})
                );
            }
        }
    }

    fn child_batch(name: &str, mode: &str, case_id: &str) -> ExecBatch {
        ExecBatch {
            name: name.into(),
            executor: Executor::Process(RunnerConfig {
                cmd: std::env::current_exe()
                    .expect("test executable")
                    .to_string_lossy()
                    .into_owned(),
                args: vec![
                    "--exact".into(),
                    "runner::protocol_tests::protocol_runner_child".into(),
                    "--nocapture".into(),
                ],
                cwd: None,
                env: HashMap::from([
                    (CHILD_MODE.into(), mode.into()),
                    (CHILD_CASE.into(), case_id.into()),
                ]),
            }),
            cases: vec![RunCase {
                id: case_id.into(),
                feature_path: "case.feature".into(),
                scenario: case_id.into(),
                line_number: None,
                until_line: None,
            }],
        }
    }

    fn run_batches(batches: Vec<ExecBatch>) -> Vec<RunEvent> {
        let project = tempfile::tempdir().expect("project tempdir");
        let meta = HashMap::from([(
            "project_root".into(),
            project.path().to_string_lossy().into_owned(),
        )]);
        let (rx, _cancel) = spawn_routed_run(project.path().to_path_buf(), batches, meta);
        rx.iter().collect()
    }

    fn end_run(events: &[RunEvent]) -> (usize, usize, usize, usize) {
        events
            .iter()
            .find_map(|event| match event {
                RunEvent::EndRun {
                    passed,
                    failed,
                    skipped,
                    blocked,
                } => Some((*passed, *failed, *skipped, *blocked)),
                _ => None,
            })
            .expect("routed run must end with end_run")
    }

    #[test]
    fn test_routed_run_merges_batches_into_one_framed_run() {
        let events = run_batches(vec![
            child_batch("api", "pass", "a"),
            child_batch("web", "blocked", "b"),
            child_batch("cli", "silent", "c"),
        ]);
        let starts = events
            .iter()
            .filter(|event| matches!(event, RunEvent::StartRun { .. }))
            .count();
        assert_eq!(starts, 1, "events: {events:?}");
        assert!(
            matches!(
                events.first(),
                Some(RunEvent::StartRun { total: Some(3), .. })
            ),
            "events: {events:?}"
        );
        assert_eq!(end_run(&events), (1, 1, 0, 1), "events: {events:?}");
        assert!(
            events.iter().any(
                |event| matches!(event, RunEvent::CaseFailed { case_id, error, .. }
                if case_id == "c" && error.message.contains("did not report"))
            ),
            "an unreported case must fail: {events:?}"
        );
        assert!(
            matches!(
                events.last(),
                Some(RunEvent::RunnerExit { success: false, .. })
            ),
            "events: {events:?}"
        );
    }

    #[test]
    fn test_runner_without_handshake_is_rejected() {
        let events = run_batches(vec![child_batch("legacy", "no_handshake", "a")]);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RunEvent::CasePassed { .. })),
            "pre-handshake results must not be accepted: {events:?}"
        );
        assert!(
            matches!(events.last(), Some(RunEvent::RunnerError { message }) if message.contains("handshake")),
            "events: {events:?}"
        );
    }

    #[test]
    fn test_undeclared_capability_is_a_protocol_violation() {
        let events = run_batches(vec![child_batch("web", "undeclared", "a")]);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RunEvent::CaseBlocked { .. })),
            "undeclared events must not be forwarded: {events:?}"
        );
        assert!(
            matches!(events.last(), Some(RunEvent::RunnerError { message }) if message.contains("protocol violation")),
            "events: {events:?}"
        );
    }
}
