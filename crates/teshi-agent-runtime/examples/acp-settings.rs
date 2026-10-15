//! Live settings smoke test: pass an agent executable followed by its arguments.
//! Uses an isolated project and prints only advertised setting labels and outcomes.

use std::{
    collections::HashMap,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use teshi_agent::backend::{AgentBackendEvent, AgentBackendKind};
use teshi_agent_runtime::{
    AcpControls,
    backend::{AcpBackendConfig, AgentBackendRuntime},
};

fn wait(
    backend: &mut AgentBackendRuntime,
    predicate: impl Fn(&AcpControls) -> bool,
) -> anyhow::Result<AcpControls> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(Some(AgentBackendEvent::Failed(error))) = backend.try_next_event() {
            anyhow::bail!("{error}");
        }
        let state = backend
            .acp_controls()
            .ok_or_else(|| anyhow::anyhow!("missing ACP state"))?;
        if predicate(&state) {
            return Ok(state);
        }
        if Instant::now() >= deadline {
            anyhow::bail!("settings deadline exceeded");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let program = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("pass agent executable and arguments"))?,
    );
    let project = tempfile::tempdir()?;
    let verify_persistence = std::env::var_os("TESHI_ACP_VERIFY_PERSISTENCE").is_some();
    let mut config = AcpBackendConfig::new(teshi_acp::AcpAgentCommand {
        program,
        args: args.collect(),
        cwd: project.path().to_path_buf(),
        env: HashMap::new(),
    });
    config.permission_host = Some(teshi_agent_runtime::ChannelPermissionHost::new());
    config.client.prompt_timeout = Some(Duration::from_secs(90));
    if verify_persistence {
        config.model_preference_path = Some(project.path().join("model-preference.json"));
    }
    let reconnect_config = config.clone();
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    backend.configure_acp(config)?;
    backend.connect_acp()?;
    let initial = wait(&mut backend, |s| s.ready)?;
    for option in &initial.config.options {
        println!(
            "{}: {} ({} choices, {})",
            option.name,
            option.value_label(),
            option.choices.len(),
            option.kind
        );
    }
    for category in ["model", "thought_level", "mode"] {
        let state = backend
            .acp_controls()
            .ok_or_else(|| anyhow::anyhow!("missing state"))?;
        let Some(option) = state
            .config
            .options
            .iter()
            .find(|o| o.category.as_deref() == Some(category))
        else {
            continue;
        };
        let choice = if category == "mode" {
            option.choices.iter().find(|c| c.value == option.current)
        } else {
            option
                .choices
                .iter()
                .find(|c| c.value != option.current)
                .or_else(|| option.choices.first())
        };
        let Some(choice) = choice else {
            continue;
        };
        backend.set_acp_option(&option.id, choice.value.clone(), state.revision)?;
        let updated = wait(&mut backend, |s| !s.pending)?;
        if let Some(error) = updated.error {
            anyhow::bail!("{error}");
        }
        anyhow::ensure!(updated.session_id == initial.session_id, "session changed");
        println!(
            "Confirmed {} -> {} on same session",
            option.name, choice.name
        );
        if category == "model" && choice.value != option.current {
            if verify_persistence {
                let selected = choice.value.clone();
                let option_id = option.id.clone();
                let persisted: serde_json::Value = serde_json::from_slice(&std::fs::read(
                    project.path().join("model-preference.json"),
                )?)?;
                anyhow::ensure!(persisted == selected, "saved model differs from selection");
                // Drop the entire backend and external process so the next
                // connection cannot inherit the old process's in-memory state.
                drop(backend);
                backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 2);
                backend.configure_acp(reconnect_config.clone())?;
                backend.connect_acp()?;
                let fresh = wait(&mut backend, |s| s.ready)?;
                anyhow::ensure!(fresh.error.is_none(), "restore failed: {:?}", fresh.error);
                anyhow::ensure!(
                    fresh
                        .config
                        .options
                        .iter()
                        .any(|o| o.id == option_id && o.current == selected),
                    "new process did not restore model"
                );
                println!("ACP_MODEL_PERSISTENCE_OK: new process confirmed {selected}");
                break;
            }
            backend.set_acp_option(&option.id, option.current.clone(), updated.revision)?;
            let restored = wait(&mut backend, |s| !s.pending)?;
            anyhow::ensure!(restored.error.is_none(), "model restoration failed");
        }
    }
    backend.start()?;
    backend.submit_turn(
        None,
        vec![teshi_core::llm::ChatMessage::text(
            "user",
            "Do not use tools or modify files. Reply with exactly ACP_SETTINGS_OK.",
        )],
        None,
    )?;
    let deadline = Instant::now() + Duration::from_secs(100);
    loop {
        match backend.try_next_event() {
            Ok(Some(AgentBackendEvent::Completed { .. })) => break,
            Ok(Some(AgentBackendEvent::Failed(error))) => anyhow::bail!("{error}"),
            Ok(Some(AgentBackendEvent::PermissionRequest { request_id, .. })) => {
                backend.respond_permission(request_id, None);
            }
            _ => {}
        }
        anyhow::ensure!(Instant::now() < deadline, "prompt deadline exceeded");
        thread::sleep(Duration::from_millis(20));
    }
    anyhow::ensure!(
        backend
            .messages()
            .last()
            .is_some_and(|m| m.content.contains("ACP_SETTINGS_OK")),
        "unexpected response"
    );
    println!("ACP_SETTINGS_OK");
    Ok(())
}
