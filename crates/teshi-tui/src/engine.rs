//! Engine adapter — spawns the Python teshi-engine as a subprocess and
//! communicates via NDJSON stdin/stdout for execution, recording, and code
//! generation.
//!
//! The engine processes inbound messages:
//!   RUN_SCENARIO, RECORD_START, RECORD_STOP, GENERATE_PAGE_OBJECT,
//!   GENERATE_STEP_DEFS, GENERATE_PROJECT
//!
//! And emits outbound messages:
//!   STEP_EXECUTED, SCENARIO_FINISHED, SCENARIO_TRACE, HEAL_DIFF,
//!   RECORD_STEP, RECORD_STOPPED, PAGE_OBJECT_GENERATED,
//!   STEP_DEFS_GENERATED, PROJECT_GENERATED, ERROR

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cli::GenerateCommand;
use crate::runner::RunCliOptions;

// ---------------------------------------------------------------------------
// Engine configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
struct EngineConfigFile {
    engine: Option<EngineConfigPartial>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct EngineConfigPartial {
    cmd: Option<String>,
    args: Option<Vec<String>>,
    cwd: Option<String>,
    env: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Engine event types (outbound from engine)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum EngineEvent {
    StepExecuted {
        step_id: String,
        status: String,
        duration_ms: f64,
        error: Option<String>,
        screenshot_path: Option<String>,
    },
    ScenarioFinished {
        scenario_id: String,
        overall_status: String,
        total_steps: usize,
        passed: usize,
        failed: usize,
        total_duration_ms: f64,
        self_healed: usize,
    },
    ScenarioTrace {
        scenario_id: String,
        steps_json: serde_json::Value,
    },
    HealDiff {
        step_id: String,
        scenario_id: String,
        original_locator: serde_json::Value,
        healed_locator: serde_json::Value,
        strategy: String,
    },
    RecordStep {
        session_id: String,
        action: serde_json::Value,
        screenshot: Option<String>,
    },
    RecordStopped {
        session_id: String,
        steps: Vec<serde_json::Value>,
        status: String,
    },
    PageObjectGenerated {
        name: String,
        method_count: usize,
        code_lines: usize,
    },
    StepDefsGenerated {
        code_lines: usize,
        method_count: usize,
    },
    ProjectGenerated {
        output_dir: String,
        files: Vec<String>,
        code_lines: usize,
    },
    EngineError {
        code: String,
        message: String,
    },
    EngineExit {
        code: Option<i32>,
        success: bool,
    },
    /// Raw code output captured from stderr (for generation results)
    CodeOutput {
        code: String,
    },
    /// Step-to-PageObject mapping notification from the engine
    MappingUpdated {
        step_id: String,
        page_object_name: String,
        locator_key: Option<String>,
        recording_step_id: Option<String>,
    },
    /// Engine heartbeat response
    Pong {
        ts: Option<f64>,
    },
}

// ---------------------------------------------------------------------------
// Engine step format (what we send to the engine)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EngineStepLocator {
    #[serde(rename = "type")]
    locator_type: String,
    value: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EngineStep {
    id: String,
    action: String,
    locator: EngineStepLocator,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

impl EngineStep {
    fn from_binding(
        step_line: usize,
        action: &str,
        strategy: &str,
        value: &str,
        value_arg: Option<&str>,
    ) -> Self {
        let (engine_action, url) = match action {
            "navigate" => ("goto", Some(value.to_string())),
            "goto" => ("goto", Some(value.to_string())),
            "open_project" => ("goto", None), // Not directly supported; skip or navigate
            _ => (action, None),
        };

        Self {
            id: format!("L{step_line}"),
            action: engine_action.to_string(),
            locator: EngineStepLocator {
                locator_type: strategy.to_string(),
                value: value.to_string(),
            },
            value: value_arg.map(|v| v.to_string()),
            url,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load engine configuration from teshi.toml or auto-detect from project root.
pub fn load_engine_config(
    project_root: &Path,
    cli_cmd: Option<&str>,
    cli_args: &[String],
) -> Result<EngineConfig> {
    // 1. Check env var
    if let Ok(cmd) = std::env::var("TESHI_ENGINE_CMD") {
        return Ok(EngineConfig {
            cmd,
            args: cli_args.to_vec(),
            cwd: Some(project_root.to_path_buf()),
            env: HashMap::new(),
        });
    }

    // 2. Check teshi.toml [engine] section
    let config_path = project_root.join("teshi.toml");
    if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let file: EngineConfigFile = toml::from_str(&raw)
            .with_context(|| format!("failed to parse {}", config_path.display()))?;
        if let Some(cfg) = file.engine
            && let Some(cmd) = cfg.cmd.filter(|s| !s.trim().is_empty())
        {
            return Ok(EngineConfig {
                cmd: cli_cmd.unwrap_or(&cmd).to_string(),
                args: cfg.args.unwrap_or_else(|| cli_args.to_vec()),
                cwd: cfg.cwd.map(PathBuf::from),
                env: cfg.env.unwrap_or_default(),
            });
        }
        // engine section exists but cmd is empty; fall through to auto-detect
    }

    // 3. Auto-detect: check common engine locations relative to project root
    //    Priority: engine/engine/__main__.py > engine/__main__.py > teshi-engine in PATH
    let candidates = [
        (
            "engine/engine/__main__.py",
            vec!["-m".to_string(), "engine".to_string()],
        ),
        (
            "engine/__main__.py",
            vec!["-m".to_string(), "engine".to_string()],
        ),
        (
            "engine/cli.py",
            vec![
                project_root
                    .join("engine")
                    .join("cli.py")
                    .to_string_lossy()
                    .to_string(),
            ],
        ),
    ];

    for (rel_path, args) in &candidates {
        let full_path = project_root.join(rel_path);
        if full_path.exists() {
            let final_args = if cli_args.is_empty() {
                args.clone()
            } else {
                cli_args.to_vec()
            };
            return Ok(EngineConfig {
                cmd: "python".to_string(),
                args: final_args,
                cwd: Some(project_root.to_path_buf()),
                env: HashMap::new(),
            });
        }
    }

    // 4. Try `python -m engine.cli` — check if engine is installed as a package
    let import_check = std::process::Command::new("python")
        .args(["-c", "import engine.cli; print('ok')"])
        .output();
    if let Ok(out) = import_check
        && out.status.success()
    {
        let mut args = cli_args.to_vec();
        if args.is_empty() {
            args = vec!["-m".to_string(), "engine.cli".to_string()];
        }
        return Ok(EngineConfig {
            cmd: "python".to_string(),
            args,
            cwd: Some(project_root.to_path_buf()),
            env: HashMap::new(),
        });
    }

    anyhow::bail!(
        "engine not found: set TESHI_ENGINE_CMD, add [engine] to teshi.toml, \
         or ensure engine/ is in the project root"
    )
}

/// Spawn the engine process and return a receiver for engine events.
pub fn spawn_engine(
    config: &EngineConfig,
) -> Result<(Sender<serde_json::Value>, Receiver<EngineEvent>)> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<serde_json::Value>();
    let (event_tx, event_rx) = mpsc::channel::<EngineEvent>();

    let cmd = config.cmd.clone();
    let args = config.args.clone();
    let cwd = config.cwd.clone();
    let env = config.env.clone();

    thread::spawn(move || {
        if let Err(err) =
            run_engine_child(&cmd, &args, cwd.as_deref(), &env, cmd_rx, event_tx.clone())
        {
            let _ = event_tx.send(EngineEvent::EngineError {
                code: "spawn_failed".to_string(),
                message: err.to_string(),
            });
        }
    });

    Ok((cmd_tx, event_rx))
}

/// Build engine step dicts from the Rust step bindings.
pub fn bindings_to_engine_steps(
    project_root: &Path,
    feature_relative_path: &str,
) -> Result<Vec<EngineStep>> {
    let bindings = teshi_engine::list_step_bindings(project_root, feature_relative_path)?;
    let steps: Vec<EngineStep> = bindings
        .steps
        .iter()
        .filter(|b| b.status == "confirmed")
        .map(|b| {
            EngineStep::from_binding(
                b.step_line,
                &b.primary.action,
                &b.primary.strategy,
                &b.primary.value,
                b.primary.value_arg.as_deref(),
            )
        })
        .collect();
    Ok(steps)
}

// ---------------------------------------------------------------------------
// High-level command handlers
// ---------------------------------------------------------------------------

/// Run a feature through the engine (used by `teshi run` when engine is detected).
pub fn run_feature_with_engine(
    config: &EngineConfig,
    feature_path: &Path,
    opts: &RunCliOptions,
) -> Result<()> {
    // Resolve project root
    let project_root = teshi_engine::find_project_root(Some(feature_path))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Build cases
    let cases = crate::runner::build_cases_from_path(feature_path, opts.scenario.as_deref())?;
    if cases.is_empty() {
        return Err(anyhow::anyhow!("no scenarios found to run"));
    }

    let (cmd_tx, event_rx) = spawn_engine(config)?;

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;

    for case in &cases {
        // Resolve step bindings for this case
        let steps = match bindings_to_engine_steps(&project_root, &case.feature_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "warning: failed to resolve bindings for {}: {e}",
                    case.feature_path
                );
                skipped += 1;
                continue;
            }
        };

        if steps.is_empty() {
            eprintln!("warning: no confirmed bindings for {}", case.feature_path);
            skipped += 1;
            continue;
        }

        let scenario_id = format!("{}:{}", case.id, case.scenario.replace(' ', "_"));
        let msg = serde_json::json!({
            "type": "RUN_SCENARIO",
            "scenario_id": scenario_id,
            "steps": steps,
        });

        if cmd_tx.send(msg).is_err() {
            eprintln!("engine process died");
            break;
        }

        let mut case_passed = true;
        loop {
            match event_rx.recv() {
                Ok(EngineEvent::StepExecuted {
                    step_id,
                    status,
                    duration_ms,
                    error,
                    ..
                }) => {
                    let icon = if status == "success" { "✓" } else { "✗" };
                    let err_info = error.map(|e| format!("  {e}")).unwrap_or_default();
                    println!("  {icon} {step_id} ({duration_ms:.0}ms){err_info}");
                    if status != "success" {
                        case_passed = false;
                    }
                }
                Ok(EngineEvent::ScenarioFinished {
                    scenario_id: sid,
                    overall_status,
                    passed: p,
                    failed: f,
                    total_duration_ms,
                    self_healed,
                    ..
                }) => {
                    let heal_info = if self_healed > 0 {
                        format!(" (self-healed: {self_healed})")
                    } else {
                        String::new()
                    };
                    println!(
                        "scenario {}: {} ({p} passed, {f} failed, {total_duration_ms:.0}ms){heal_info}",
                        sid, overall_status
                    );
                    if overall_status != "passed" {
                        case_passed = false;
                    }
                    break; // done with this scenario
                }
                Ok(EngineEvent::HealDiff {
                    step_id,
                    original_locator,
                    healed_locator,
                    strategy,
                    ..
                }) => {
                    eprintln!(
                        "  heal: {step_id} [{strategy}] {} → {}",
                        original_locator
                            .get("value")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?"),
                        healed_locator
                            .get("value")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?"),
                    );

                    // Persist the healed locator to the step bindings
                    let step_line: usize = step_id
                        .strip_prefix('L')
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    if step_line > 0 {
                        let healed_strategy = healed_locator
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("css");
                        let healed_value = healed_locator
                            .get("value")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !healed_value.is_empty() {
                            let _ = teshi_engine::update_binding_locator(
                                &project_root,
                                &case.feature_path,
                                step_line,
                                healed_strategy,
                                healed_value,
                            );
                        }
                    }
                }
                Ok(EngineEvent::ScenarioTrace { .. }) => {
                    // Trace is captured but not printed by default; could write to .teshi/logs/
                }
                Ok(EngineEvent::EngineError { code, message }) => {
                    eprintln!("engine error [{code}]: {message}");
                    case_passed = false;
                    break;
                }
                Ok(EngineEvent::EngineExit { code, success }) => {
                    if !success {
                        eprintln!("engine exited with code {:?}", code);
                    }
                    break;
                }
                Err(_) => {
                    eprintln!("engine communication lost");
                    return Ok(());
                }
                _ => {}
            }
        }

        if case_passed {
            passed += 1;
        } else {
            failed += 1;
        }
    }

    // Signal the engine to shut down by dropping the sender
    drop(cmd_tx);

    // Drain remaining events
    while let Ok(event) = event_rx.recv() {
        if matches!(event, EngineEvent::EngineExit { .. }) {
            break;
        }
    }

    println!("run complete: {passed} passed, {failed} failed, {skipped} skipped");
    if failed > 0 {
        anyhow::bail!("run completed with {failed} failure(s)");
    }
    Ok(())
}

/// Handle `teshi record` — start an engine recording session.
pub fn handle_record_command(url: &str, feature: Option<&str>, auto_propose: bool) -> Result<()> {
    let project_root = teshi_engine::find_project_root(None)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let config = load_engine_config(&project_root, None, &[])?;
    let (cmd_tx, event_rx) = spawn_engine(&config)?;

    let session_id = format!(
        "rec-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );

    let msg = serde_json::json!({
        "type": "RECORD_START",
        "session_id": session_id,
        "url": url,
    });

    if cmd_tx.send(msg).is_err() {
        return Err(anyhow::anyhow!(
            "engine process died before recording started"
        ));
    }

    eprintln!("[teshi] Recording started at {url}");
    eprintln!("[teshi] Interact with the browser, then close it to stop recording.");

    let mut recorded_steps: Vec<serde_json::Value> = Vec::new();

    loop {
        match event_rx.recv() {
            Ok(EngineEvent::RecordStep {
                session_id: _,
                action,
                ..
            }) => {
                let action_type = action.get("action").and_then(|v| v.as_str()).unwrap_or("?");
                let locator_value = action
                    .get("locator")
                    .and_then(|l| l.get("value"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                eprintln!("  ✓ {:<8}  {:60.60}", action_type, locator_value);
                recorded_steps.push(action.clone());
            }
            Ok(EngineEvent::RecordStopped { steps, status, .. }) => {
                eprintln!(
                    "[teshi] Recording {} — {} actions captured",
                    status,
                    steps.len()
                );
                recorded_steps = steps;
                break;
            }
            Ok(EngineEvent::EngineError { code, message }) => {
                eprintln!("[teshi] Engine error [{code}]: {message}");
                break;
            }
            Ok(EngineEvent::EngineExit { .. }) => break,
            Err(_) => break,
            _ => {}
        }
    }

    drop(cmd_tx);

    if auto_propose && !recorded_steps.is_empty() {
        // Write first recorded step as pending locator proposal
        if let Some(first_step) = recorded_steps.first() {
            let locator = first_step.get("locator").and_then(|l| l.as_object());
            let action = first_step
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("click");

            if let Some(loc) = locator {
                let strategy = loc.get("type").and_then(|v| v.as_str()).unwrap_or("css");
                let value = loc.get("value").and_then(|v| v.as_str()).unwrap_or("");

                let pending = teshi_engine::PendingLocator {
                    step_ref: teshi_engine::ActiveStep {
                        feature_relative_path: feature.unwrap_or("").to_string(),
                        scenario_line: 0,
                        scenario_name: String::new(),
                        step_line: 0,
                        step_keyword: String::new(),
                        step_text: String::new(),
                        updated_at: String::new(),
                    },
                    candidates: vec![teshi_engine::LocatorCandidate {
                        rank: 1,
                        strategy: strategy.to_string(),
                        value: value.to_string(),
                        action: action.to_string(),
                        value_arg: first_step
                            .get("value")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        confidence: 0.9,
                        rationale: format!("Recorded from browser interaction at {url}"),
                    }],
                    highlight: None,
                    status: "pending".to_string(),
                };

                teshi_engine::propose_locator(&project_root, pending)?;
                eprintln!("[teshi] Auto-proposed locator for active step");
            }
        }
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "session_id": session_id,
            "steps": recorded_steps.len(),
        }))?
    );

    Ok(())
}

/// Handle `teshi generate po|steps|project`
pub fn handle_generate_command(action: &GenerateCommand) -> Result<()> {
    let project_root = teshi_engine::find_project_root(None)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let config = load_engine_config(&project_root, None, &[])?;
    let (cmd_tx, event_rx) = spawn_engine(&config)?;

    match action {
        GenerateCommand::Po {
            scenario,
            feature,
            output,
        } => {
            let steps: Vec<serde_json::Value> = if let Some(path) = scenario {
                let content = std::fs::read_to_string(path)
                    .with_context(|| format!("failed to read {path}"))?;
                let data: serde_json::Value = serde_json::from_str(&content)?;
                data.get("steps")
                    .and_then(|s| s.as_array())
                    .map(|a| a.to_vec())
                    .unwrap_or_default()
            } else if let Some(feature_path) = feature {
                let engine_steps = bindings_to_engine_steps(&project_root, feature_path)?;
                engine_steps
                    .iter()
                    .map(|s| serde_json::to_value(s).unwrap_or_default())
                    .collect()
            } else {
                return Err(anyhow::anyhow!("Provide --scenario or --feature"));
            };

            let msg = serde_json::json!({
                "type": "GENERATE_PAGE_OBJECT",
                "steps": steps,
            });
            cmd_tx.send(msg)?;

            let code = collect_generated_code(&event_rx)?;
            if let Some(out) = output {
                std::fs::write(out, &code)?;
                println!("PageObject written to {out}");
            } else {
                println!("{code}");
            }
        }
        GenerateCommand::Steps {
            page_object,
            feature,
            output,
        } => {
            let po_code = if let Some(path) = page_object {
                std::fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?
            } else if let Some(feature_path) = feature {
                // Generate PO first, then steps from it
                let engine_steps = bindings_to_engine_steps(&project_root, feature_path)?;
                let steps_json: Vec<serde_json::Value> = engine_steps
                    .iter()
                    .map(|s| serde_json::to_value(s).unwrap_or_default())
                    .collect();

                let po_msg = serde_json::json!({
                    "type": "GENERATE_PAGE_OBJECT",
                    "steps": steps_json,
                });
                cmd_tx.send(po_msg)?;
                collect_generated_code(&event_rx)?
            } else {
                return Err(anyhow::anyhow!("Provide --page-object or --feature"));
            };

            let msg = serde_json::json!({
                "type": "GENERATE_STEP_DEFS",
                "page_object_code": po_code,
            });
            cmd_tx.send(msg)?;

            let code = collect_generated_code(&event_rx)?;
            if let Some(out) = output {
                std::fs::write(out, &code)?;
                println!("Step definitions written to {out}");
            } else {
                println!("{code}");
            }
        }
        GenerateCommand::Project { feature, output } => {
            let feature_path = std::path::Path::new(feature);
            let gherkin = if feature_path.exists() {
                std::fs::read_to_string(feature_path)
                    .with_context(|| format!("failed to read {feature}"))?
            } else {
                String::new()
            };

            let engine_steps = bindings_to_engine_steps(&project_root, feature)?;
            let steps_json: Vec<serde_json::Value> = engine_steps
                .iter()
                .map(|s| serde_json::to_value(s).unwrap_or_default())
                .collect();

            let msg = serde_json::json!({
                "type": "GENERATE_PROJECT",
                "output_dir": output,
                "steps": steps_json,
                "feature_gherkin": gherkin,
            });
            cmd_tx.send(msg)?;

            loop {
                match event_rx.recv() {
                    Ok(EngineEvent::ProjectGenerated {
                        output_dir,
                        files,
                        code_lines,
                    }) => {
                        println!(
                            "Project generated at {output_dir}: {} files, {code_lines} lines",
                            files.len()
                        );
                        for f in &files {
                            println!("  {f}");
                        }
                        break;
                    }
                    Ok(EngineEvent::EngineError { code, message }) => {
                        eprintln!("engine error [{code}]: {message}");
                        break;
                    }
                    Ok(EngineEvent::EngineExit { .. }) => break,
                    Err(_) => break,
                    _ => {}
                }
            }
        }
    }

    drop(cmd_tx);
    // Drain remaining events
    while let Ok(event) = event_rx.recv() {
        if matches!(event, EngineEvent::EngineExit { .. }) {
            break;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn run_engine_child(
    cmd: &str,
    args: &[String],
    cwd: Option<&Path>,
    env: &HashMap<String, String>,
    cmd_rx: Receiver<serde_json::Value>,
    event_tx: Sender<EngineEvent>,
) -> Result<()> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    for (k, v) in env {
        command.env(k, v);
    }

    let mut child = command.spawn().context("failed to spawn engine")?;

    // Writer thread: send commands to engine stdin
    let mut stdin = child.stdin.take().unwrap();
    thread::spawn(move || {
        for msg in &cmd_rx {
            let payload = serde_json::to_string(&msg).unwrap_or_default();
            if stdin.write_all(payload.as_bytes()).is_err() {
                break;
            }
            if stdin.write_all(b"\n").is_err() {
                break;
            }
            let _ = stdin.flush();
        }
        // stdin is dropped here, signalling EOF to the engine
        drop(stdin);
        // Just keep the thread alive until cmd_rx closes
    });

    // Stderr reader thread
    let stderr = child.stderr.take().unwrap();
    let stderr_tx = event_tx.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let mut collecting_code = false;
        let mut code_buffer = String::new();
        for line in reader.lines().map_while(Result::ok) {
            if line.starts_with("__TESHI_PO_CODE__") || line.starts_with("__TESHI_STEPS_CODE__") {
                collecting_code = true;
                code_buffer.clear();
                continue;
            }
            if collecting_code {
                if line == "__TESHI_PO_END__" || line == "__TESHI_STEPS_END__" {
                    collecting_code = false;
                    let _ = stderr_tx.send(EngineEvent::CodeOutput {
                        code: std::mem::take(&mut code_buffer),
                    });
                    continue;
                }
                if !code_buffer.is_empty() {
                    code_buffer.push('\n');
                }
                code_buffer.push_str(&line);
                continue;
            }
            // Normal stderr goes to parent stderr
            eprintln!("{line}");
        }
    });

    // Stdout reader thread (main event parser)
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
        if let Some(event) = parse_engine_event(&line) {
            let _ = event_tx.send(event);
        }
    }

    let status = child.wait()?;
    let _ = event_tx.send(EngineEvent::EngineExit {
        code: status.code(),
        success: status.success(),
    });

    Ok(())
}

fn parse_engine_event(line: &str) -> Option<EngineEvent> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let kind = value.get("type")?.as_str()?;

    match kind {
        "STEP_EXECUTED" => Some(EngineEvent::StepExecuted {
            step_id: value.get("step_id")?.as_str()?.to_string(),
            status: value.get("status")?.as_str()?.to_string(),
            duration_ms: value
                .get("duration_ms")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            error: value
                .get("error")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            screenshot_path: value
                .get("screenshot_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        }),
        "SCENARIO_FINISHED" => Some(EngineEvent::ScenarioFinished {
            scenario_id: value.get("scenario_id")?.as_str()?.to_string(),
            overall_status: value.get("overall_status")?.as_str()?.to_string(),
            total_steps: value
                .get("total_steps")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            passed: value.get("passed").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            failed: value.get("failed").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            total_duration_ms: value
                .get("total_duration_ms")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            self_healed: value
                .get("self_healed")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
        }),
        "SCENARIO_TRACE" => Some(EngineEvent::ScenarioTrace {
            scenario_id: value.get("scenario_id")?.as_str()?.to_string(),
            steps_json: value.get("steps").cloned().unwrap_or_default(),
        }),
        "HEAL_DIFF" => Some(EngineEvent::HealDiff {
            step_id: value.get("step_id")?.as_str()?.to_string(),
            scenario_id: value.get("scenario_id")?.as_str()?.to_string(),
            original_locator: value.get("original_locator").cloned().unwrap_or_default(),
            healed_locator: value.get("healed_locator").cloned().unwrap_or_default(),
            strategy: value
                .get("strategy")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string(),
        }),
        "RECORD_STEP" => Some(EngineEvent::RecordStep {
            session_id: value.get("session_id")?.as_str()?.to_string(),
            action: value.get("action").cloned().unwrap_or_default(),
            screenshot: value
                .get("screenshot")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        }),
        "RECORD_STOPPED" => Some(EngineEvent::RecordStopped {
            session_id: value.get("session_id")?.as_str()?.to_string(),
            steps: value
                .get("steps")
                .and_then(|v| v.as_array())
                .map(|a| a.to_vec())
                .unwrap_or_default(),
            status: value
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("completed")
                .to_string(),
        }),
        "PAGE_OBJECT_GENERATED" => Some(EngineEvent::PageObjectGenerated {
            name: value
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string(),
            method_count: value
                .get("method_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            code_lines: value
                .get("code_lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
        }),
        "STEP_DEFS_GENERATED" => Some(EngineEvent::StepDefsGenerated {
            code_lines: value
                .get("code_lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            method_count: value
                .get("method_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
        }),
        "PROJECT_GENERATED" => Some(EngineEvent::ProjectGenerated {
            output_dir: value
                .get("output_dir")
                .and_then(|v| v.as_str())
                .unwrap_or("generated")
                .to_string(),
            files: value
                .get("files")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            code_lines: value
                .get("code_lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
        }),
        "ERROR" => Some(EngineEvent::EngineError {
            code: value
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            message: value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
        }),
        "MAPPING_UPDATED" => Some(EngineEvent::MappingUpdated {
            step_id: value.get("step_id")?.as_str()?.to_string(),
            page_object_name: value
                .get("page_object_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string(),
            locator_key: value
                .get("locator_key")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            recording_step_id: value
                .get("recording_step_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        }),
        "PONG" => Some(EngineEvent::Pong {
            ts: value.get("ts").and_then(|v| v.as_f64()),
        }),
        _ => None,
    }
}

/// Wait for CodeOutput events and collect the generated code.
fn collect_generated_code(rx: &Receiver<EngineEvent>) -> Result<String> {
    loop {
        match rx.recv() {
            Ok(EngineEvent::CodeOutput { code }) => return Ok(code),
            Ok(EngineEvent::PageObjectGenerated { .. }) => {
                // Expected before code output; continue waiting
            }
            Ok(EngineEvent::StepDefsGenerated { .. }) => {
                // Expected before code output; continue waiting
            }
            Ok(EngineEvent::EngineError { code, message }) => {
                return Err(anyhow::anyhow!("engine error [{code}]: {message}"));
            }
            Ok(EngineEvent::EngineExit { .. }) => {
                return Err(anyhow::anyhow!("engine exited without generating code"));
            }
            Err(_) => {
                return Err(anyhow::anyhow!("engine communication lost"));
            }
            _ => {} // ignore other events
        }
    }
}
