//! `teshi api` — start, inspect, and doctor the HTTP API BDD sidecar.

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use teshi_engine::{
    default_api_service_script, ensure_api_sidecar, read_api_endpoint, send_api_command,
    stop_api_sidecar,
};

use super::{ApiCommand, ApiExchangeArgs, ApiServeArgs};

/// Handles `teshi api ...` subcommands.
pub fn handle_api_command(action: &ApiCommand) -> Result<()> {
    let project_root = std::env::current_dir().context("resolve current directory")?;
    match action {
        ApiCommand::Serve(args) => serve(&project_root, args),
        ApiCommand::Doctor => doctor(&project_root),
        ApiCommand::Stop => stop(&project_root),
        ApiCommand::Exchange(args) => exchange(&project_root, args),
    }
}

fn serve(project_root: &Path, args: &ApiServeArgs) -> Result<()> {
    let root = args
        .project
        .clone()
        .unwrap_or_else(|| project_root.to_path_buf());
    let endpoint = ensure_api_sidecar(&root, &default_api_service_script())?;
    println!("{}", serde_json::to_string_pretty(&endpoint)?);
    Ok(())
}

fn doctor(project_root: &Path) -> Result<()> {
    let endpoint = match read_api_endpoint(project_root) {
        Ok(endpoint) => endpoint,
        Err(err) => {
            let report = json!({
                "ok": false,
                "error": err.to_string(),
            });
            println!("{}", serde_json::to_string_pretty(&report)?);
            anyhow::bail!("API sidecar endpoint not found");
        }
    };
    let response = send_api_command(
        project_root,
        json!({"cmd": "doctor", "request_id": "api-doctor"}),
        Duration::from_secs(5),
    );
    match response {
        Ok(payload) => {
            let mut report = payload;
            if let Value::Object(map) = &mut report {
                map.insert("ws_url".into(), json!(endpoint.ws_url));
                map.insert("pid".into(), json!(endpoint.pid));
            }
            println!("{}", serde_json::to_string_pretty(&report)?);
            if report.get("ok") != Some(&Value::Bool(true)) {
                anyhow::bail!("API sidecar doctor failed");
            }
            Ok(())
        }
        Err(err) => {
            let report = json!({
                "ok": false,
                "ws_url": endpoint.ws_url,
                "error": err.to_string(),
            });
            println!("{}", serde_json::to_string_pretty(&report)?);
            Err(anyhow!("API sidecar is not reachable"))
        }
    }
}

fn stop(project_root: &Path) -> Result<()> {
    stop_api_sidecar(project_root)?;
    println!("{{\"ok\":true}}");
    Ok(())
}

fn exchange(project_root: &Path, args: &ApiExchangeArgs) -> Result<()> {
    let response = send_api_command(
        project_root,
        json!({
            "cmd": "get_exchange",
            "request_id": "api-exchange",
            "exchange_id": args.id,
            "redact": !args.plaintext,
        }),
        Duration::from_secs(5),
    )?;
    let mut stdout = io::stdout();
    writeln!(stdout, "{}", serde_json::to_string_pretty(&response)?)?;
    if response.get("ok") != Some(&Value::Bool(true)) {
        anyhow::bail!("get_exchange failed");
    }
    Ok(())
}
