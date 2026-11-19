//! Probe Cursor (or any ACP agent) for real session restore support.
//!
//! Flow:
//! 1. initialize → print loadSession / resume capabilities
//! 2. session/new → capture sessionId
//! 3. optional short prompt so the agent has something to resume
//! 4. shutdown
//! 5. spawn again → try session/resume, then session/load, with that id
//!
//! Usage:
//!   cargo run -p teshi-acp --example restore-probe -- \
//!     --program <exe> [--arg <a>]... [--cwd <dir>] [--timeout <secs>]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};

use teshi_acp::{AcpAgentCommand, AcpClient, AcpEvent, PermissionPolicy, client::AcpClientConfig};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> ExitCode {
    let mut program: Option<PathBuf> = None;
    let mut args = Vec::new();
    let mut cwd = std::env::current_dir().unwrap_or_default();
    let mut timeout = Duration::from_secs(60);

    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--program" => program = it.next().map(PathBuf::from),
            "--arg" => args.push(it.next().unwrap_or_default()),
            "--cwd" => cwd = it.next().map(PathBuf::from).unwrap_or(cwd),
            "--timeout" => {
                timeout = Duration::from_secs(it.next().and_then(|v| v.parse().ok()).unwrap_or(60))
            }
            other => {
                eprintln!("unknown flag {other}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(program) = program else {
        eprintln!("--program required");
        return ExitCode::from(2);
    };

    println!(
        "[probe] program={} args={args:?} cwd={}",
        program.display(),
        cwd.display()
    );

    let (session_id, supports_resume, supports_load) =
        match open_and_create_session(&program, &args, &cwd, timeout).await {
            Ok(value) => value,
            Err(error) => {
                eprintln!("[fail] first process: {error}");
                return ExitCode::FAILURE;
            }
        };

    println!("[probe] saved sessionId={session_id}");
    println!("[probe] supports_resume={supports_resume} supports_load={supports_load}");
    if !supports_resume && !supports_load {
        eprintln!(
            "[result] Cursor ACP advertises neither resume nor load; Teshi will keep text reseeding for this agent"
        );
        return ExitCode::from(3);
    }

    match reopen_and_restore(
        &program,
        &args,
        &cwd,
        timeout,
        &session_id,
        supports_resume,
        supports_load,
    )
    .await
    {
        Ok(path) => {
            println!("[result] OK restored via {path}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("[result] FAIL restore: {error}");
            ExitCode::FAILURE
        }
    }
}

fn client_config(timeout: Duration) -> AcpClientConfig {
    AcpClientConfig {
        initialize_timeout: Duration::from_secs(45),
        authentication_timeout: Duration::from_secs(15),
        request_timeout: timeout,
        prompt_timeout: Some(timeout),
        shutdown_timeout: Duration::from_secs(15),
        ..Default::default()
    }
}

fn command(program: &Path, args: &[String], cwd: &Path) -> AcpAgentCommand {
    AcpAgentCommand {
        program: program.to_path_buf(),
        args: args.to_vec(),
        cwd: cwd.to_path_buf(),
        env: HashMap::new(),
    }
}

fn spawn_event_drain() -> mpsc::Sender<AcpEvent> {
    let (tx, mut rx) = mpsc::channel(128);
    tokio::spawn(async move {
        let mut n = 0usize;
        while let Some(event) = rx.recv().await {
            n += 1;
            let label = match &event {
                AcpEvent::AgentMessageChunk(text) => {
                    format!("chunk {:?}", text.chars().take(80).collect::<String>())
                }
                AcpEvent::ToolCall(_) => "tool_call".into(),
                AcpEvent::ToolCallUpdate(_) => "tool_call_update".into(),
                AcpEvent::PermissionRequest(_) => "permission_request".into(),
                AcpEvent::SessionUpdate(_) => "session_update".into(),
                AcpEvent::ExtensionNotification { method, .. } => format!("ext:{method}"),
                AcpEvent::Completed { stop_reason, .. } => format!("completed {stop_reason:?}"),
                AcpEvent::Error(error) => format!("error {error}"),
                AcpEvent::Cancelled => "cancelled".into(),
            };
            println!("  [events#{n}] {label}");
        }
    });
    tx
}

async fn open_and_create_session(
    program: &Path,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
) -> anyhow::Result<(String, bool, bool)> {
    let start = Instant::now();
    let mut client = AcpClient::spawn(
        command(program, args, cwd),
        client_config(timeout),
        spawn_event_drain(),
        PermissionPolicy::Auto,
        None,
    )
    .await?;
    let init = client.initialize().await?;
    let supports_resume = init.supports_resume_session();
    let supports_load = init.supports_load_session();
    println!(
        "[{:6.2}s] initialize ok capabilities={}",
        start.elapsed().as_secs_f64(),
        init.capabilities
    );

    let session = client.new_session(cwd.to_path_buf()).await?;
    println!(
        "[{:6.2}s] session/new ok id={}",
        start.elapsed().as_secs_f64(),
        session.id
    );

    // Give the agent a tiny turn so restore is more than an empty shell.
    match client
        .prompt(&session, "Reply with exactly: restore-probe-ok")
        .await
    {
        Ok(stop_reason) => println!(
            "[{:6.2}s] prompt ok stop={stop_reason:?}",
            start.elapsed().as_secs_f64()
        ),
        Err(error) => println!(
            "[{:6.2}s] prompt skipped/failed: {error}",
            start.elapsed().as_secs_f64()
        ),
    }

    let _ = client.shutdown().await;
    println!(
        "[{:6.2}s] first process shutdown",
        start.elapsed().as_secs_f64()
    );
    Ok((session.id, supports_resume, supports_load))
}

async fn reopen_and_restore(
    program: &Path,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
    session_id: &str,
    supports_resume: bool,
    supports_load: bool,
) -> anyhow::Result<&'static str> {
    let start = Instant::now();
    let mut client = AcpClient::spawn(
        command(program, args, cwd),
        client_config(timeout),
        spawn_event_drain(),
        PermissionPolicy::Auto,
        None,
    )
    .await?;
    let init = client.initialize().await?;
    println!(
        "[{:6.2}s] second initialize ok resume={} load={}",
        start.elapsed().as_secs_f64(),
        init.supports_resume_session(),
        init.supports_load_session()
    );

    if supports_resume {
        match client.resume_session(session_id, cwd.to_path_buf()).await {
            Ok(session) => {
                println!(
                    "[{:6.2}s] session/resume OK id={}",
                    start.elapsed().as_secs_f64(),
                    session.id
                );
                let _ = client.shutdown().await;
                return Ok("session/resume");
            }
            Err(error) => println!(
                "[{:6.2}s] session/resume ERR: {error}",
                start.elapsed().as_secs_f64()
            ),
        }
    }

    if supports_load {
        match client.load_session(session_id, cwd.to_path_buf()).await {
            Ok(session) => {
                println!(
                    "[{:6.2}s] session/load OK id={}",
                    start.elapsed().as_secs_f64(),
                    session.id
                );
                let _ = client.shutdown().await;
                return Ok("session/load");
            }
            Err(error) => {
                let _ = client.shutdown().await;
                anyhow::bail!("session/load failed: {error}");
            }
        }
    }

    let _ = client.shutdown().await;
    anyhow::bail!("no restore path succeeded")
}
