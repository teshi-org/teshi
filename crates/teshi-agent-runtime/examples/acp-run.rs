//! Real end-to-end ACP run against a locally installed agent.
//!
//! Drives the same path the TUI uses: `AgentBackendRuntime` (ACP variant) →
//! `teshi-acp` client → external ACP agent over stdio. Mirrors the TUI's
//! per-send sequence (`start_agent_backend` → `start()` → `submit_turn`) so
//! multi-turn behaviour can be reproduced outside the TUI.
//!
//! Usage:
//!   cargo run -p teshi-agent-runtime --example acp-run -- \
//!       --program <exe> [--arg <a>]... [--cwd <dir>] [--prompt <text>] \
//!       [--policy auto|bypass|manual] [--turns N] [--clean-env] \
//!       [--deadline <secs>] [--prompt-timeout <secs>]

use std::{
    collections::HashMap,
    path::PathBuf,
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use teshi_acp::{AcpAgentCommand, PermissionPolicy, client::AcpClientConfig};
use teshi_agent::backend::{AgentBackendEvent, AgentBackendKind};
use teshi_agent_runtime::backend::{AcpBackendConfig, AgentBackendRuntime};
use teshi_core::llm::ChatMessage;

/// Host variables inherited from an embedding IDE/agent host that change or
/// break the child agent's own startup (e.g. its port and config directory).
const HOST_ENV_PREFIXES: [&str; 3] = ["CODEBUDDY_", "CLAUDE_CODE_", "WORKBUDDY_"];

struct Args {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    prompt: String,
    policy: PermissionPolicy,
    clean_env: bool,
    turns: u32,
    deadline: Duration,
    prompt_timeout: Duration,
    shutdown_timeout: Duration,
}

fn parse_args() -> Result<Args, String> {
    let mut program: Option<PathBuf> = None;
    let mut args = Vec::new();
    let mut cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut prompt = "Reply with exactly: ACP-OK".to_string();
    let mut policy = PermissionPolicy::Auto;
    let mut clean_env = false;
    let mut turns = 1u32;
    let mut deadline = Duration::from_secs(180);
    let mut prompt_timeout = Duration::from_secs(150);
    let mut shutdown_timeout = Duration::from_secs(10);

    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut next = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--program" => program = Some(PathBuf::from(next("--program")?)),
            "--arg" => args.push(next("--arg")?),
            "--cwd" => cwd = PathBuf::from(next("--cwd")?),
            "--prompt" => prompt = next("--prompt")?,
            "--turns" => {
                turns = next("--turns")?
                    .parse()
                    .map_err(|e| format!("bad --turns: {e}"))?
            }
            "--deadline" => {
                deadline = Duration::from_secs(
                    next("--deadline")?
                        .parse()
                        .map_err(|e| format!("bad --deadline: {e}"))?,
                )
            }
            "--prompt-timeout" => {
                prompt_timeout = Duration::from_secs(
                    next("--prompt-timeout")?
                        .parse()
                        .map_err(|e| format!("bad --prompt-timeout: {e}"))?,
                )
            }
            "--shutdown-timeout" => {
                shutdown_timeout = Duration::from_secs(
                    next("--shutdown-timeout")?
                        .parse()
                        .map_err(|e| format!("bad --shutdown-timeout: {e}"))?,
                )
            }
            "--policy" => {
                policy = match next("--policy")?.as_str() {
                    "auto" => PermissionPolicy::Auto,
                    "bypass" => PermissionPolicy::Bypass,
                    "manual" => PermissionPolicy::Manual,
                    other => return Err(format!("unknown policy `{other}`")),
                }
            }
            "--clean-env" => clean_env = true,
            other => return Err(format!("unknown flag `{other}`")),
        }
    }

    Ok(Args {
        program: program.ok_or("--program is required")?,
        args,
        cwd,
        prompt,
        policy,
        clean_env,
        turns,
        deadline,
        prompt_timeout,
        shutdown_timeout,
    })
}

/// Strip host-injected variables so the child agent starts with its own
/// defaults instead of inheriting an embedding session's ports/config.
fn strip_host_env() {
    let victims: Vec<String> = std::env::vars()
        .map(|(k, _)| k)
        .filter(|k| HOST_ENV_PREFIXES.iter().any(|p| k.starts_with(p)))
        .collect();
    for key in &victims {
        // SAFETY: single-threaded setup before any agent thread is spawned.
        unsafe { std::env::remove_var(key) };
    }
    println!(
        "[env] stripped {} host variable(s): {}{}",
        victims.len(),
        victims
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        if victims.len() > 6 { ", …" } else { "" }
    );
}

/// Pump events for one turn. Returns true when the turn reached a terminal event.
fn pump_turn(backend: &mut AgentBackendRuntime, started: Instant, deadline: Instant) -> bool {
    let mut terminal = false;
    while Instant::now() < deadline {
        match backend.try_next_event() {
            Ok(Some(event)) => {
                let ms = started.elapsed().as_millis();
                match event {
                    AgentBackendEvent::MessageChunk { content } => {
                        print!("{content}");
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                    }
                    AgentBackendEvent::ExternalToolActivity { title, status } => {
                        println!("  [{ms}ms] tool  : {title} ({status:?})");
                    }
                    AgentBackendEvent::PermissionRequest {
                        request_id,
                        options,
                    } => {
                        let labels: Vec<_> = options
                            .iter()
                            .map(|option| option.display_label().to_string())
                            .collect();
                        println!("  [{ms}ms] PERM  : {labels:?} (auto-allow first)");
                        backend.respond_permission(
                            request_id,
                            options.first().map(|option| option.id.clone()),
                        );
                    }
                    AgentBackendEvent::ToolActivity { tool_calls, .. } => {
                        let names: Vec<_> = tool_calls.iter().map(|c| c.name.clone()).collect();
                        println!("  [{ms}ms] tools : {names:?}");
                    }
                    AgentBackendEvent::Completed {
                        model,
                        finish_reason,
                        ..
                    } => {
                        println!();
                        println!("  [{ms}ms] DONE  : model={model} finish={finish_reason:?}");
                        terminal = true;
                        break;
                    }
                    AgentBackendEvent::Failed(message) => {
                        println!();
                        println!("  [{ms}ms] FAIL  : {message}");
                        terminal = true;
                        break;
                    }
                    AgentBackendEvent::Ignored => {}
                }
            }
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Empty) => {
                thread::sleep(Duration::from_millis(20))
            }
            Err(error) => {
                println!("  [fail] event channel disconnected: {error}");
                terminal = true;
                break;
            }
        }
    }
    terminal
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };

    if args.clean_env {
        strip_host_env();
    }

    println!("[cfg] program = {}", args.program.display());
    println!("[cfg] args    = {:?}", args.args);
    println!("[cfg] cwd     = {}", args.cwd.display());
    println!("[cfg] policy  = {:?}", args.policy);
    println!("[cfg] turns   = {}", args.turns);
    println!("[cfg] prompt  = {:?}", args.prompt);
    println!();

    let command = AcpAgentCommand {
        program: args.program.clone(),
        args: args.args.clone(),
        cwd: args.cwd.clone(),
        env: HashMap::new(),
    };
    let mut config = AcpBackendConfig::new(command);
    config.permission_policy = args.policy;
    config.client = AcpClientConfig {
        initialize_timeout: Duration::from_secs(60),
        authentication_timeout: Duration::from_secs(15),
        request_timeout: Duration::from_secs(60),
        prompt_timeout: Some(args.prompt_timeout),
        shutdown_timeout: args.shutdown_timeout,
        ..Default::default()
    };

    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    if let Err(error) = backend.configure_acp(config) {
        eprintln!("[fail] configure_acp: {error}");
        return ExitCode::FAILURE;
    }

    let started = Instant::now();
    let mut dead_turns = 0u32;

    for turn in 1..=args.turns {
        println!("=========== turn {turn}/{} ===========", args.turns);
        let t0 = Instant::now();

        // Mirror the TUI: start_agent_backend() calls backend.start() first.
        let start_begin = Instant::now();
        if let Err(error) = backend.start() {
            println!("  [start] ERR: {error}");
        }
        let start_ms = start_begin.elapsed().as_millis();
        println!("  [start] ok ({start_ms}ms) status={:?}", backend.status());

        let prompt = if args.turns > 1 {
            format!("{} (turn {turn})", args.prompt)
        } else {
            args.prompt.clone()
        };
        if let Err(error) = backend.submit_turn(None, vec![ChatMessage::text("user", prompt)], None)
        {
            println!("  [submit] ERR: {error}");
            dead_turns += 1;
            println!();
            continue;
        }

        let deadline = Instant::now() + args.deadline;
        let terminal = pump_turn(&mut backend, started, deadline);
        println!(
            "  [turn {turn}] terminal={terminal} status={:?} elapsed={}ms",
            backend.status(),
            t0.elapsed().as_millis()
        );
        if !terminal {
            println!("  [turn {turn}] >>> NO RESPONSE (deadline reached, backend silent)");
            dead_turns += 1;
        }
        println!();
    }

    println!(
        "[summary] turns={} dead={} status={:?} total={}ms",
        args.turns,
        dead_turns,
        backend.status(),
        started.elapsed().as_millis()
    );
    if dead_turns > 0 {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
