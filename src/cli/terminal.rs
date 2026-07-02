use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use teshi_runtime::send_sidecar_command_with_timeout;

use super::browser_endpoint::read_cdp_endpoint;
use super::{TerminalCommand, TerminalExecArgs, TerminalResizeArgs, TerminalSendArgs};

/// Handles `teshi terminal ...` subcommands.
pub fn handle_terminal_command(action: &TerminalCommand) -> Result<()> {
    let project_root = std::env::current_dir().context("resolve current directory")?;
    match action {
        TerminalCommand::ServeEmbedded => serve_embedded(&project_root),
        TerminalCommand::Snapshot => snapshot(&project_root),
        TerminalCommand::Status => status(&project_root),
        TerminalCommand::Exec(args) => exec(&project_root, args),
        TerminalCommand::Send(args) => send(&project_root, args),
        TerminalCommand::Resize(args) => resize(&project_root, args),
        TerminalCommand::Kill => kill(&project_root),
    }
}

/// Starts the terminal sidecar as a child process and waits for it to exit.
/// Ctrl+C is automatically propagated from the parent to the child process group.
fn serve_embedded(_project_root: &Path) -> Result<()> {
    // Find the sidecar binary relative to the teshi executable
    let mut sidecar_path = std::env::current_exe()
        .context("resolve current executable path")?
        .parent()
        .context("executable path has no parent")?
        .join("teshi-terminal-sidecar");
    if cfg!(windows) {
        sidecar_path.set_extension("exe");
    }

    if !sidecar_path.exists() {
        anyhow::bail!(
            "terminal sidecar not found at {}; build with `cargo build -p teshi-terminal-sidecar`",
            sidecar_path.display()
        );
    }

    eprintln!("starting terminal sidecar...");
    eprintln!("  binary: {}", sidecar_path.display());
    eprintln!("  cwd:    {}", _project_root.display());

    let mut child = std::process::Command::new(&sidecar_path)
        .current_dir(_project_root)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawn {}", sidecar_path.display()))?;

    eprintln!(
        "terminal sidecar started (pid {}); press Ctrl+C to stop",
        child.id()
    );

    // Block until the child exits (Ctrl+C propagates automatically)
    let status = child.wait().context("wait for sidecar")?;
    if let Some(code) = status.code() {
        eprintln!("terminal sidecar exited with code {code}");
    } else {
        eprintln!("terminal sidecar terminated by signal");
    }
    Ok(())
}

fn snapshot(project_root: &Path) -> Result<()> {
    let response = send_terminal_command(
        project_root,
        json!({ "cmd": "snapshot", "request_id": "terminal-snapshot" }),
        Duration::from_secs(10),
    )?;
    print_json_response(response)
}

fn status(project_root: &Path) -> Result<()> {
    let response = send_terminal_command(
        project_root,
        json!({ "cmd": "status", "request_id": "terminal-status" }),
        Duration::from_secs(10),
    )?;
    print_json_response(response)
}

fn exec(project_root: &Path, args: &TerminalExecArgs) -> Result<()> {
    let timeout = Duration::from_millis(args.timeout_ms + 5_000).max(Duration::from_secs(15));
    let response = send_terminal_command(
        project_root,
        json!({
            "cmd": "exec",
            "request_id": "terminal-exec",
            "command": args.command,
            "timeout_ms": args.timeout_ms
        }),
        timeout,
    )?;
    ensure_ok(&response)?;
    print_json_response(response)
}

fn send(project_root: &Path, args: &TerminalSendArgs) -> Result<()> {
    let response = send_terminal_command(
        project_root,
        json!({
            "cmd": "send",
            "request_id": "terminal-send",
            "data": args.text,
            "newline": args.newline
        }),
        Duration::from_secs(10),
    )?;
    print_json_response(response)
}

fn resize(project_root: &Path, args: &TerminalResizeArgs) -> Result<()> {
    let response = send_terminal_command(
        project_root,
        json!({
            "cmd": "resize",
            "request_id": "terminal-resize",
            "cols": args.cols,
            "rows": args.rows
        }),
        Duration::from_secs(10),
    )?;
    print_json_response(response)
}

fn kill(project_root: &Path) -> Result<()> {
    let response = send_terminal_command(
        project_root,
        json!({ "cmd": "kill", "request_id": "terminal-kill" }),
        Duration::from_secs(10),
    )?;
    print_json_response(response)
}

/// Sends a JSON command to the terminal sidecar via WebSocket.
fn send_terminal_command(project_root: &Path, command: Value, timeout: Duration) -> Result<Value> {
    let endpoint = read_cdp_endpoint(project_root)?;
    let mode = endpoint.mode;
    if !mode.is_empty() && mode != "terminal" {
        anyhow::bail!("expected mode 'terminal' but found '{mode}'; start the terminal sidecar");
    }
    let ws_url = endpoint.ws_url;
    send_sidecar_command_with_timeout(&ws_url, command, timeout).map_err(anyhow::Error::msg)
}

fn ensure_ok(response: &Value) -> Result<()> {
    if response.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(());
    }
    let error = response
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("terminal command failed");
    Err(anyhow!("{error}"))
}

fn print_json_response(response: Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
