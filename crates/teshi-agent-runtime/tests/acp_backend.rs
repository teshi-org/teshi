use std::{
    collections::HashMap,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use teshi_acp::AcpAgentCommand;
use teshi_agent::backend::{AgentBackendEvent, AgentBackendKind, AgentBackendStatus};
use teshi_agent_runtime::backend::{AcpBackendConfig, AgentBackendRuntime};
use teshi_core::llm::ChatMessage;

fn backend(mode: &str) -> AgentBackendRuntime {
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_acp_agent.py");
    let command = AcpAgentCommand {
        program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        args: vec![script.display().to_string(), mode.into()],
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        env: HashMap::new(),
    };
    let mut config = AcpBackendConfig::new(command);
    config.client.initialize_timeout = Duration::from_secs(2);
    config.client.request_timeout = Duration::from_secs(2);
    config.client.shutdown_timeout = Duration::from_secs(2);
    backend.configure_acp(config).unwrap();
    backend
}

fn events_until_terminal(backend: &mut AgentBackendRuntime) -> Vec<AgentBackendEvent> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut result = Vec::new();
    while Instant::now() < deadline {
        match backend.try_next_event() {
            Ok(Some(event)) => {
                let terminal = matches!(
                    event,
                    AgentBackendEvent::Completed { .. } | AgentBackendEvent::Failed(_)
                );
                result.push(event);
                if terminal {
                    return result;
                }
            }
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Empty) => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => panic!("event channel disconnected: {error}"),
        }
    }
    panic!("ACP backend did not reach a terminal result");
}

#[test]
fn successful_session_converts_stream_and_completion() {
    let mut backend = backend("success");
    backend.start().unwrap();
    backend
        .submit_turn(
            Some("task context".into()),
            vec![ChatMessage::text("user", "do task")],
            None,
        )
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(events.iter().any(
        |event| matches!(event, AgentBackendEvent::MessageChunk { content } if content == "hello")
    ));
    assert!(events.iter().any(|event| matches!(event, AgentBackendEvent::ExternalToolActivity { title, .. } if title == "reading project")));
    assert!(events.iter().any(|event| matches!(event, AgentBackendEvent::Completed { model, finish_reason: Some(reason), .. } if model == "fake-agent" && reason == "end_turn")));
    assert_eq!(backend.messages().last().unwrap().content, "hello");
    assert_eq!(backend.status(), AgentBackendStatus::Completed);
}

#[test]
fn process_launch_failure_is_reported() {
    let mut backend = backend("success");
    backend
        .configure_acp(AcpBackendConfig::new(AcpAgentCommand {
            program: PathBuf::from("definitely-missing-teshi-acp-agent"),
            args: vec![],
            cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            env: HashMap::new(),
        }))
        .unwrap();
    backend.start().unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(
        matches!(&events[0], AgentBackendEvent::Failed(error) if error.contains("launch failed"))
    );
    assert_eq!(backend.status(), AgentBackendStatus::Failed);
}

#[test]
fn initialize_session_prompt_and_crash_fail_distinctly() {
    for (mode, expected) in [
        ("initialize_failed", "initialize failed"),
        ("session_failed", "session creation failed"),
        ("prompt_failed", "prompt failed"),
        ("crash", "prompt failed"),
    ] {
        let mut backend = backend(mode);
        backend.start().unwrap();
        backend
            .submit_turn(None, vec![ChatMessage::text("user", "do task")], None)
            .unwrap();
        let events = events_until_terminal(&mut backend);
        assert!(events.iter().any(|event| matches!(event, AgentBackendEvent::Failed(error) if error.contains(expected))), "{mode}: {events:?}");
        assert_eq!(backend.status(), AgentBackendStatus::Failed);
    }
}

#[test]
fn dropping_backend_closes_worker_and_agent() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("exited.txt");
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_acp_agent.py");
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    let mut config = AcpBackendConfig::new(AcpAgentCommand {
        program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        args: vec![
            script.display().to_string(),
            "success".into(),
            marker.display().to_string(),
        ],
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        env: HashMap::new(),
    });
    config.client.shutdown_timeout = Duration::from_secs(2);
    backend.configure_acp(config).unwrap();
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "do task")], None)
        .unwrap();
    events_until_terminal(&mut backend);
    let pid = std::fs::read_to_string(&marker)
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let started = Instant::now();
    drop(backend);
    assert!(started.elapsed() < Duration::from_secs(4));
    let alive = if cfg!(windows) {
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Get-Process -Id {pid} -ErrorAction SilentlyContinue"),
            ])
            .output()
            .unwrap();
        !output.stdout.is_empty()
    } else {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    };
    assert!(!alive, "ACP agent process {pid} survived backend drop");
}
