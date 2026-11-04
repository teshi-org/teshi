//! Live ACP/MCP interoperability probe with an isolated, non-writing host.
//!
//! Usage: cargo run -p teshi-agent-runtime --example acp-authoring-probe --
//! <installed-agent-id> <teshi-executable>
//! The host records calls, not product acceptance of the full generation UI.

use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use teshi_agent::backend::{AgentBackendEvent, AgentBackendKind, AgentBackendStatus};
use teshi_agent_runtime::{
    backend::{AcpBackendConfig, AgentBackendRuntime},
    tool_bridge::ToolBridgeConfig,
};
use teshi_core::llm::ChatMessage;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let id = args.next().context("missing installed agent ID")?;
    let executable =
        PathBuf::from(args.next().context("missing Teshi proxy executable")?).canonicalize()?;
    let root = tempfile::tempdir()?;
    let profile = teshi_acp::profile::load(&teshi_acp::profile::store_dir()?)?
        .into_iter()
        .find(|profile| profile.id == id)
        .context("agent is not installed")?;
    let names = vec!["submit_requirements".into(), "propose_test_points".into()];
    let tools = teshi_agent::get_tools(Some(&names));
    let mut config = AcpBackendConfig::new(profile.command(root.path().to_path_buf())?);
    config.permission_policy = teshi_acp::PermissionPolicy::Auto;
    config.client.initialize_timeout = Duration::from_secs(60);
    config.client.request_timeout = Duration::from_secs(60);
    config.client.prompt_timeout = Some(Duration::from_secs(150));
    config.client.shutdown_timeout = Duration::from_secs(2);
    config.tool_bridge = Some(ToolBridgeConfig {
        executable,
        tools: tools.clone(),
    });
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    backend.configure_acp(config)?;
    backend.start()?;
    backend.submit_turn(None, vec![ChatMessage::text("user", "This is an isolated Teshi MCP interoperability test. Use the teshi-authoring MCP tools provided in this session. First call submit_requirements with feature_name 'Probe' and scenario_descriptions ['A user can sign in']. Then call propose_test_points with one point titled 'Valid sign in', objective 'The user signs in successfully', hierarchy_path ['Probe']. Do not use filesystem or terminal tools. Stop after proposing; human review happens later. Reply ACP-MCP-OK after both tool calls succeed.")], Some(tools))?;
    let started = Instant::now();
    let mut calls = Vec::new();
    loop {
        while let Some(request) = backend.host_tool_request() {
            let name = request.call.name.clone();
            let arguments: serde_json::Value = serde_json::from_str(&request.call.arguments)?;
            let result = match name.as_str() {
                "submit_requirements" if arguments["feature_name"].is_string() && arguments["scenario_descriptions"].is_array() => {
                    Ok("Requirements received by the isolated probe host. Next call propose_test_points.".into())
                }
                "propose_test_points" if calls.iter().any(|name| name == "submit_requirements") && arguments["test_points"].is_array() => {
                    Ok("Test points received by the isolated probe host. Stop now and await human review.".into())
                }
                _ => Err("Unexpected tool or arguments in interoperability probe".into()),
            };
            println!(
                "host_call={name} success={} elapsed_ms={}",
                result.is_ok(),
                started.elapsed().as_millis()
            );
            if result.is_ok() {
                calls.push(name);
            }
            request.respond(result);
        }
        match backend.try_next_event() {
            Ok(Some(AgentBackendEvent::Completed { finish_reason, .. })) => {
                println!("completed={finish_reason:?} host_calls={calls:?}");
                if backend.status() != AgentBackendStatus::Completed || calls != names {
                    bail!("agent did not complete both host tool calls");
                }
                break;
            }
            Ok(Some(AgentBackendEvent::Failed(error))) => bail!("ACP probe failed: {error}"),
            Ok(Some(AgentBackendEvent::ExternalToolActivity { status, .. })) => {
                println!("external_tool_status={status:?}")
            }
            _ => {}
        }
        if started.elapsed() > Duration::from_secs(180) {
            backend.cancel();
            bail!("ACP probe exceeded deadline");
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
