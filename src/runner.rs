use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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

fn parse_event_line(line: &str) -> Option<RunEvent> {
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
    pub feature: Option<PathBuf>,
    pub scenario: Option<String>,
    pub runner_cmd: Option<String>,
    pub runner_args: Vec<String>,
    pub runner_cwd: Option<PathBuf>,
}

pub fn run_cli(args: &[String]) -> Result<()> {
    let opts = parse_run_args(args);
    let feature_path = opts
        .feature
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--feature is required for run"))?;
    let config = load_runner_config(Some(RunnerCliOverride {
        cmd: opts.runner_cmd,
        args: opts.runner_args,
        cwd: opts.runner_cwd,
    }))?;
    let cases = build_cases_from_path(&feature_path, opts.scenario.as_deref())?;
    if cases.is_empty() {
        return Err(anyhow::anyhow!("no scenarios found to run"));
    }
    let request = RunRequest {
        command: "run".to_string(),
        cases,
        meta: HashMap::new(),
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

fn parse_run_args(args: &[String]) -> RunCliOptions {
    let mut opts = RunCliOptions {
        feature: None,
        scenario: None,
        runner_cmd: None,
        runner_args: Vec::new(),
        runner_cwd: None,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--feature" => {
                if let Some(v) = args.get(i + 1) {
                    opts.feature = Some(PathBuf::from(v));
                    i += 1;
                }
            }
            "--scenario" => {
                if let Some(v) = args.get(i + 1) {
                    opts.scenario = Some(v.to_string());
                    i += 1;
                }
            }
            "--runner-cmd" => {
                if let Some(v) = args.get(i + 1) {
                    opts.runner_cmd = Some(v.to_string());
                    i += 1;
                }
            }
            "--runner-arg" => {
                if let Some(v) = args.get(i + 1) {
                    opts.runner_args.push(v.to_string());
                    i += 1;
                }
            }
            "--runner-cwd" => {
                if let Some(v) = args.get(i + 1) {
                    opts.runner_cwd = Some(PathBuf::from(v));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    opts
}

fn build_cases_from_path(path: &Path, scenario_filter: Option<&str>) -> Result<Vec<RunCase>> {
    let mut cases = Vec::new();
    if path.is_dir() {
        let project = crate::gherkin::parse_project(path);
        for (fi, feature) in project.features.iter().enumerate() {
            collect_cases(&mut cases, fi, feature, scenario_filter);
        }
    } else {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let feature = crate::gherkin::parse_feature(&content, path.to_path_buf());
        collect_cases(&mut cases, 0, &feature, scenario_filter);
    }
    Ok(cases)
}

fn collect_cases(
    cases: &mut Vec<RunCase>,
    feature_idx: usize,
    feature: &crate::gherkin::BddFeature,
    scenario_filter: Option<&str>,
) {
    for (si, scenario) in feature.scenarios.iter().enumerate() {
        if let Some(name) = scenario_filter
            && scenario.name != name
        {
            continue;
        }
        cases.push(RunCase {
            id: format!("f{feature_idx}:s{si}"),
            feature_path: feature.file_path.to_string_lossy().to_string(),
            scenario: scenario.name.clone(),
            line_number: Some(scenario.line_number),
        });
    }
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
        RunEvent::RunnerExit { code, success } => {
            format!("runner_exit code={:?} success={success}", code)
        }
        RunEvent::RunnerError { message } => format!("runner_error {message}"),
    }
}
