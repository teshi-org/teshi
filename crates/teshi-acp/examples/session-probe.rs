//! Probe: drive an ACP agent directly through `AcpClient` with a concurrent
//! event drain, then attempt `session/new` twice.
//!
//! The second attempt distinguishes "the response was never matched" from
//! "the connection/reader died after the first failure".
//!
//! Usage:
//!   cargo run -p teshi-acp --example session-probe -- \
//!       --program <exe> [--arg <a>]... [--cwd <dir>] [--clean-env] [--timeout <secs>]

use std::{
    collections::HashMap,
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant},
};

use teshi_acp::{AcpAgentCommand, AcpClient, AcpEvent, PermissionPolicy, client::AcpClientConfig};
use tokio::sync::mpsc;

const HOST_ENV_PREFIXES: [&str; 3] = ["CODEBUDDY_", "CLAUDE_CODE_", "WORKBUDDY_"];

#[tokio::main]
async fn main() -> ExitCode {
    let mut program: Option<PathBuf> = None;
    let mut args = Vec::new();
    let mut cwd = std::env::current_dir().unwrap_or_default();
    let mut clean_env = false;
    let mut minimal_env = false;
    let mut timeout = Duration::from_secs(30);

    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--program" => program = it.next().map(PathBuf::from),
            "--arg" => args.push(it.next().unwrap_or_default()),
            "--cwd" => cwd = it.next().map(PathBuf::from).unwrap_or(cwd),
            "--clean-env" => clean_env = true,
            "--minimal-env" => minimal_env = true,
            "--timeout" => {
                timeout = Duration::from_secs(it.next().and_then(|v| v.parse().ok()).unwrap_or(30))
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

    // Show exactly what teshi's own `session/new` params serialize to.
    {
        let params = agent_client_protocol::schema::v1::NewSessionRequest::new(cwd.clone());
        println!(
            "[params] session/new = {}",
            serde_json::to_string(&params).unwrap_or_else(|e| format!("<err {e}>"))
        );
    }

    if minimal_env {
        const KEEP: [&str; 11] = [
            "SystemRoot",
            "windir",
            "PATH",
            "USERPROFILE",
            "HOME",
            "TEMP",
            "TMP",
            "APPDATA",
            "LOCALAPPDATA",
            "ComSpec",
            "PATHEXT",
        ];
        let victims: Vec<String> = std::env::vars()
            .map(|(k, _)| k)
            .filter(|k| !KEEP.iter().any(|keep| keep.eq_ignore_ascii_case(k)))
            .collect();
        for key in &victims {
            // SAFETY: called before any agent thread is spawned.
            unsafe { std::env::remove_var(key) };
        }
        println!("[env] minimal env: removed {} variable(s)", victims.len());
    }

    if clean_env {
        let victims: Vec<String> = std::env::vars()
            .map(|(k, _)| k)
            .filter(|k| HOST_ENV_PREFIXES.iter().any(|p| k.starts_with(p)))
            .collect();
        for key in &victims {
            // SAFETY: called before any agent thread is spawned.
            unsafe { std::env::remove_var(key) };
        }
        println!("[env] stripped {} host variable(s)", victims.len());
    }

    let start = Instant::now();
    let (events_tx, mut events_rx) = mpsc::channel(128);
    tokio::spawn(async move {
        let mut n = 0usize;
        while let Some(event) = events_rx.recv().await {
            n += 1;
            let label = match &event {
                AcpEvent::AgentMessageChunk(t) => {
                    format!("chunk {:?}", t.chars().take(60).collect::<String>())
                }
                AcpEvent::ToolCall(_) => "tool_call".into(),
                AcpEvent::ToolCallUpdate(_) => "tool_call_update".into(),
                AcpEvent::PermissionRequest(_) => "permission_request".into(),
                AcpEvent::SessionUpdate(_) => "session_update".into(),
                AcpEvent::ExtensionNotification { method, .. } => format!("ext:{method}"),
                AcpEvent::Completed { stop_reason } => format!("completed {stop_reason:?}"),
                AcpEvent::Error(e) => format!("error {e}"),
                AcpEvent::Cancelled => "cancelled".into(),
            };
            println!(
                "  [{:6.2}s events#{n}] {label}",
                start.elapsed().as_secs_f64()
            );
        }
    });

    let command = AcpAgentCommand {
        program,
        args,
        cwd: cwd.clone(),
        env: HashMap::new(),
    };
    let config = AcpClientConfig {
        initialize_timeout: Duration::from_secs(45),
        authentication_timeout: Duration::from_secs(10),
        request_timeout: timeout,
        prompt_timeout: Some(timeout),
        shutdown_timeout: Duration::from_secs(10),
        ..Default::default()
    };

    let mut client =
        match AcpClient::spawn(command, config, events_tx, PermissionPolicy::Auto, None).await {
            Ok(client) => client,
            Err(error) => {
                eprintln!("[fail] spawn: {error}");
                return ExitCode::FAILURE;
            }
        };

    match client.initialize().await {
        Ok(init) => {
            println!(
                "[{:6.2}s] initialize ok: protocol={:?} authMethods={:?}",
                start.elapsed().as_secs_f64(),
                init.protocol_version,
                init.auth_methods
            );
        }
        Err(error) => {
            eprintln!("[fail] initialize: {error}");
            return ExitCode::FAILURE;
        }
    }

    for attempt in 1..=2 {
        let began = Instant::now();
        let result = client.new_session(cwd.clone()).await;
        match result {
            Ok(session) => println!(
                "[{:6.2}s] session/new #{attempt} OK: id={} ({:.2}s)",
                start.elapsed().as_secs_f64(),
                session.id,
                began.elapsed().as_secs_f64()
            ),
            Err(error) => println!(
                "[{:6.2}s] session/new #{attempt} ERR: {error} ({:.2}s)",
                start.elapsed().as_secs_f64(),
                began.elapsed().as_secs_f64()
            ),
        }
    }

    let _ = client.shutdown().await;
    println!("[{:6.2}s] shutdown done", start.elapsed().as_secs_f64());
    ExitCode::SUCCESS
}
