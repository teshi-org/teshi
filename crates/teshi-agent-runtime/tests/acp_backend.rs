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

#[test]
fn acp_activity_retains_thoughts_and_merges_partial_tool_updates() {
    let mut backend = backend("activity");
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "hello")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentBackendEvent::ReasoningChunk { .. }))
    );
    let thought = backend
        .messages()
        .iter()
        .find(|m| m.source.as_deref() == Some("acp_reasoning"))
        .unwrap();
    assert_eq!(
        thought.reasoning_content.as_deref(),
        Some("Inspecting the project")
    );
    let tools: Vec<_> = backend
        .messages()
        .iter()
        .filter(|m| m.tool_call_id.as_deref() == Some("read-1"))
        .collect();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].content, "Read project");
    assert_eq!(tools[0].reasoning_content.as_deref(), Some("completed"));
}

#[test]
fn fresh_acp_process_receives_saved_context_only_on_first_prompt() {
    let mut backend = backend("restored");
    let messages = vec![
        ChatMessage::text("user", "previous question"),
        ChatMessage::text("assistant", "previous answer"),
        ChatMessage::text("user", "current question"),
    ];
    for _ in 0..2 {
        backend.start().unwrap();
        backend.submit_turn(None, messages.clone(), None).unwrap();
        let events = events_until_terminal(&mut backend);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentBackendEvent::Completed { .. })),
            "{events:?}"
        );
    }
}

/// Same as [`backend`] but wired with an interactive permission host, the
/// configuration the TUI uses for its Manual approval mode.
fn backend_with_permission_host(mode: &str) -> AgentBackendRuntime {
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
    config.permission_host = Some(teshi_agent_runtime::ChannelPermissionHost::new());
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

fn wait_controls(
    backend: &AgentBackendRuntime,
    predicate: impl Fn(&teshi_agent_runtime::AcpControls) -> bool,
) -> teshi_agent_runtime::AcpControls {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = backend.acp_controls().unwrap();
        if predicate(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "settings did not settle: {state:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn settings_discovery_mutation_and_prompt_share_one_session() {
    let mut backend = backend("settings");
    backend.connect_acp().unwrap();
    let initial = wait_controls(&backend, |s| s.ready);
    assert_eq!(backend.status(), AgentBackendStatus::Idle);
    backend
        .set_acp_option("model", "b".into(), initial.revision)
        .unwrap();
    let updated = wait_controls(&backend, |s| !s.pending);
    assert_eq!(updated.session_id, initial.session_id);
    assert_eq!(updated.config.options[1].current, "low");
    assert!(
        backend
            .set_acp_option("effort", "high".into(), updated.revision)
            .is_err()
    );
    assert!(
        backend
            .set_acp_option("model", "a".into(), initial.revision)
            .is_err()
    );
    backend
        .set_acp_option("fast", true.into(), updated.revision)
        .unwrap();
    let updated = wait_controls(&backend, |s| !s.pending);
    assert_eq!(updated.config.options[2].current, true);
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "hello")], None)
        .unwrap();
    events_until_terminal(&mut backend);
    assert_eq!(
        backend.acp_controls().unwrap().session_id,
        initial.session_id
    );
}

#[test]
fn settings_rejection_preserves_confirmed_values_and_recovers() {
    let mut backend = backend("settings");
    backend.connect_acp().unwrap();
    let initial = wait_controls(&backend, |s| s.ready);
    backend
        .set_acp_option("model", "reject".into(), initial.revision)
        .unwrap();
    let rejected = wait_controls(&backend, |s| !s.pending);
    assert_eq!(rejected.config, initial.config);
    assert!(rejected.error.unwrap().contains("setting rejected"));
    backend
        .set_acp_option("model", "b".into(), rejected.revision)
        .unwrap();
    assert!(wait_controls(&backend, |s| !s.pending).error.is_none());
}

#[test]
fn idle_updates_are_applied_without_a_prompt() {
    let mut backend = backend("settings_idle");
    backend.connect_acp().unwrap();
    let state = wait_controls(&backend, |s| {
        s.config.options.first().is_some_and(|o| o.current == "b")
    });
    assert!(state.ready);
    assert_eq!(backend.status(), AgentBackendStatus::Idle);
}

#[test]
fn timeout_requires_reconciliation() {
    let mut backend = backend("settings_timeout");
    backend.connect_acp().unwrap();
    let initial = wait_controls(&backend, |s| s.ready);
    backend
        .set_acp_option("model", "b".into(), initial.revision)
        .unwrap();
    let timed_out = wait_controls(&backend, |s| !s.pending);
    assert!(!timed_out.ready);
    assert!(timed_out.error.unwrap().contains("reconnect"));
    assert!(
        backend
            .set_acp_option("model", "a".into(), timed_out.revision)
            .is_err()
    );
}

#[test]
fn policy_changes_apply_after_connection_without_approving_pending_requests() {
    let mut backend = backend_with_permission_host("permission_then_exit");
    backend.connect_acp().unwrap();
    wait_controls(&backend, |s| s.ready);
    backend
        .set_acp_policy(teshi_acp::PermissionPolicy::Auto)
        .unwrap();
    wait_controls(&backend, |s| {
        s.policy == Some(teshi_acp::PermissionPolicy::Auto)
    });
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "hello")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentBackendEvent::PermissionRequest { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentBackendEvent::Completed { .. }))
    );
    backend
        .set_acp_policy(teshi_acp::PermissionPolicy::Manual)
        .unwrap();
    wait_controls(&backend, |s| {
        s.policy == Some(teshi_acp::PermissionPolicy::Manual)
    });
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "hello")], None)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let request_id = loop {
        if let Ok(Some(AgentBackendEvent::PermissionRequest { request_id, .. })) =
            backend.try_next_event()
        {
            break request_id;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    };
    backend
        .set_acp_policy(teshi_acp::PermissionPolicy::Auto)
        .unwrap();
    wait_controls(&backend, |s| {
        s.policy == Some(teshi_acp::PermissionPolicy::Auto)
    });
    assert!(
        backend.respond_permission(request_id, Some("allow_once".into())),
        "pending request must still require its explicit answer"
    );
    assert!(
        events_until_terminal(&mut backend)
            .iter()
            .any(|e| matches!(e, AgentBackendEvent::Completed { .. }))
    );
}

#[test]
fn separate_backends_keep_configuration_isolated() {
    let mut first = backend("settings");
    let mut second = backend("settings");
    first.connect_acp().unwrap();
    second.connect_acp().unwrap();
    let initial = wait_controls(&first, |s| s.ready);
    wait_controls(&second, |s| s.ready);
    first
        .set_acp_option("model", "b".into(), initial.revision)
        .unwrap();
    wait_controls(&first, |s| !s.pending);
    assert_eq!(
        second.acp_controls().unwrap().config.options[0].current,
        "a"
    );
    first.cancel();
    first.connect_acp().unwrap();
    let fresh = wait_controls(&first, |s| s.ready);
    assert_eq!(fresh.config.options[0].current, "a");
    assert!(
        first
            .set_acp_option("model", "b".into(), initial.revision)
            .is_err()
    );
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

#[test]
fn manual_permission_without_host_aborts_turn_when_agent_exits() {
    // Reproduces the "suddenly stops after a few words" scenario:
    // ACP agent streams some text, then asks for permission. Teshi's default
    // ACP backend uses PermissionPolicy::Manual with no PermissionHost, so it
    // rejects the request. The agent (like CodeBuddy) treats the rejection as
    // fatal and exits, cutting the response short.
    let mut backend = backend("permission_then_exit");
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "do task")], None)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        match backend.try_next_event() {
            Ok(Some(event)) => {
                eprintln!("ACP event: {event:?}");
                let terminal = matches!(
                    event,
                    AgentBackendEvent::Completed { .. } | AgentBackendEvent::Failed(_)
                );
                events.push(event);
                if terminal {
                    break;
                }
            }
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Empty) => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => panic!("event channel disconnected: {error}"),
        }
    }
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentBackendEvent::MessageChunk { content } if content == "hello"
        )),
        "expected at least one chunk before the permission failure; got {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentBackendEvent::Failed(error) if error.contains("ACP prompt failed")
        )),
        "expected prompt failure after agent exits; got {events:?}"
    );
    assert_eq!(backend.status(), AgentBackendStatus::Failed);
}

#[test]
fn manual_permission_with_host_continues_after_user_allows() {
    // Zed-style flow: the agent asks, the frontend renders the options, the
    // user allows, and the parked turn resumes to completion.
    let mut backend = backend_with_permission_host("permission_then_exit");
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "do task")], None)
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut events = Vec::new();
    let mut request_id = None;
    while Instant::now() < deadline {
        match backend.try_next_event() {
            Ok(Some(event)) => {
                if let AgentBackendEvent::PermissionRequest {
                    request_id: id,
                    options,
                } = &event
                {
                    assert_eq!(options.len(), 1, "expected one option; got {options:?}");
                    assert_eq!(options[0].display_label(), "Allow once");
                    request_id = Some(*id);
                    assert!(
                        backend.respond_permission(*id, Some("allow_once".into())),
                        "permission request must resolve while the turn is parked"
                    );
                    assert_eq!(backend.status(), AgentBackendStatus::Running);
                    continue;
                }
                let terminal = matches!(
                    event,
                    AgentBackendEvent::Completed { .. } | AgentBackendEvent::Failed(_)
                );
                events.push(event);
                if terminal {
                    break;
                }
            }
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Empty) => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => panic!("event channel disconnected: {error}"),
        }
    }
    assert!(request_id.is_some(), "agent never asked for permission");
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentBackendEvent::MessageChunk { content } if content == " world"
        )),
        "expected the post-permission chunk; got {events:?}"
    );
    assert!(events.iter().any(|event| matches!(
        event,
        AgentBackendEvent::Completed { finish_reason: Some(reason), .. } if reason == "end_turn"
    )));
    assert_eq!(backend.messages().last().unwrap().content, "hello world");
    assert_eq!(backend.status(), AgentBackendStatus::Completed);
}

#[test]
fn manual_permission_with_host_rejection_fails_turn() {
    // Rejecting must reach the agent as a cancellation so it can stop; the
    // turn then fails exactly like a host-less Manual policy.
    let mut backend = backend_with_permission_host("permission_then_exit");
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "do task")], None)
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut events = Vec::new();
    let mut answered = false;
    while Instant::now() < deadline {
        match backend.try_next_event() {
            Ok(Some(event)) => {
                if let AgentBackendEvent::PermissionRequest { request_id, .. } = &event {
                    assert!(
                        backend.respond_permission(*request_id, None),
                        "rejection must resolve while the turn is parked"
                    );
                    answered = true;
                    continue;
                }
                let terminal = matches!(
                    event,
                    AgentBackendEvent::Completed { .. } | AgentBackendEvent::Failed(_)
                );
                events.push(event);
                if terminal {
                    break;
                }
            }
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Empty) => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => panic!("event channel disconnected: {error}"),
        }
    }
    assert!(answered, "agent never asked for permission");
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentBackendEvent::Failed(error) if error.contains("ACP prompt failed")
        )),
        "expected prompt failure after the rejected agent exits; got {events:?}"
    );
    assert_eq!(backend.status(), AgentBackendStatus::Failed);
}
