use std::{
    collections::HashMap,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use teshi_acp::AcpAgentCommand;
use teshi_agent::backend::{AgentBackendEvent, AgentBackendKind, AgentBackendStatus};
use teshi_agent_runtime::backend::{AcpBackendConfig, AgentBackendRuntime, TurnRetryPolicy};
use teshi_core::llm::ChatMessage;

fn backend(mode: &str) -> AgentBackendRuntime {
    backend_with_preference(mode, None)
}

fn backend_with_preference(mode: &str, path: Option<PathBuf>) -> AgentBackendRuntime {
    backend_with(mode, Vec::new(), HashMap::new(), path)
}

/// Builds a backend for one fixture mode with extra argv, environment, and an
/// optional saved-model path.
fn backend_with(
    mode: &str,
    extra_args: Vec<String>,
    env: HashMap<String, String>,
    path: Option<PathBuf>,
) -> AgentBackendRuntime {
    let mut config = fixture_config(mode, extra_args, env);
    config.model_preference_path = path;
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    backend.configure_acp(config).unwrap();
    backend
}

/// Builds a backend with both per-agent preference files.
fn backend_with_option_preferences(
    mode: &str,
    model: Option<PathBuf>,
    options: PathBuf,
) -> AgentBackendRuntime {
    let mut config = fixture_config(mode, Vec::new(), HashMap::new());
    config.model_preference_path = model;
    config.option_preferences_path = Some(options);
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    backend.configure_acp(config).unwrap();
    backend
}

fn fixture_config(
    mode: &str,
    extra_args: Vec<String>,
    env: HashMap<String, String>,
) -> AcpBackendConfig {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_acp_agent.py");
    let mut args = vec![script.display().to_string(), mode.into()];
    args.extend(extra_args);
    let command = AcpAgentCommand {
        program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        args,
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        env,
    };
    let mut config = AcpBackendConfig::new(command);
    config.client.initialize_timeout = Duration::from_secs(2);
    config.client.request_timeout = Duration::from_secs(2);
    config.client.shutdown_timeout = Duration::from_secs(2);
    config
}

fn option_value<'a>(
    state: &'a teshi_agent_runtime::AcpControls,
    id: &str,
) -> &'a serde_json::Value {
    &state
        .config
        .options
        .iter()
        .find(|o| o.id == id)
        .unwrap_or_else(|| panic!("option {id} is advertised: {state:?}"))
        .current
}

#[test]
fn confirmed_session_options_survive_backend_recreation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("options.json");
    {
        let mut first = backend_with_option_preferences("settings_approval", None, path.clone());
        first.connect_acp().unwrap();
        let initial = wait_controls(&first, |s| s.ready);
        first
            .set_acp_option("fast", true.into(), initial.revision)
            .unwrap();
        let fast = wait_controls(&first, |s| !s.pending);
        assert!(fast.error.is_none(), "{:?}", fast.error);
        first
            .set_acp_option("approval_policy", "auto".into(), fast.revision)
            .unwrap();
        let approval = wait_controls(&first, |s| !s.pending);
        assert!(approval.error.is_none(), "{:?}", approval.error);
        first
            .set_acp_option("approval_policy", "reject".into(), approval.revision)
            .unwrap();
        let rejected = wait_controls(&first, |s| !s.pending);
        assert!(rejected.error.is_some(), "rejection is visible");
    }
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        saved,
        serde_json::json!({"fast": true, "approval_policy": "auto"}),
        "only confirmed non-model values are saved"
    );
    let mut second = backend_with_option_preferences("settings_approval", None, path.clone());
    second.connect_acp().unwrap();
    let restored = wait_controls(&second, |s| s.ready);
    assert!(restored.error.is_none(), "{:?}", restored.error);
    assert_eq!(option_value(&restored, "fast"), &serde_json::json!(true));
    assert_eq!(option_value(&restored, "approval_policy"), "auto");
    let mut other =
        backend_with_option_preferences("settings_approval", None, root.path().join("other.json"));
    other.connect_acp().unwrap();
    let isolated = wait_controls(&other, |s| s.ready);
    assert_eq!(option_value(&isolated, "fast"), &serde_json::json!(false));
    assert_eq!(option_value(&isolated, "approval_policy"), "ask");
}

#[test]
fn saved_options_restore_after_model_and_fall_back_when_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let model = root.path().join("model.json");
    let options = root.path().join("options.json");
    std::fs::write(&model, "\"b\"").unwrap();
    // Model "b" only offers the "low" thinking level, so "high" must fall back.
    std::fs::write(&options, r#"{"effort":"high","fast":true,"removed":"x"}"#).unwrap();
    let mut backend = backend_with_option_preferences("settings", Some(model), options);
    backend.connect_acp().unwrap();
    let state = wait_controls(&backend, |s| s.ready);
    assert_eq!(option_value(&state, "model"), "b");
    assert_eq!(option_value(&state, "effort"), "low");
    assert_eq!(option_value(&state, "fast"), &serde_json::json!(true));
    let error = state.error.expect("fallbacks are reported");
    assert!(
        error.contains("Saved Thinking is no longer available"),
        "{error}"
    );
    assert!(
        error.contains("Saved setting removed is no longer offered"),
        "{error}"
    );
}

#[test]
fn option_restore_timeout_blocks_the_uncertain_session() {
    let root = tempfile::tempdir().unwrap();
    let options = root.path().join("options.json");
    std::fs::write(&options, r#"{"fast":true}"#).unwrap();
    let mut config = fixture_config("settings_timeout", Vec::new(), HashMap::new());
    config.option_preferences_path = Some(options);
    config.client.request_timeout = Duration::from_millis(200);
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    backend.configure_acp(config).unwrap();
    backend.connect_acp().unwrap();
    let state = wait_controls(&backend, |s| s.error.is_some());
    assert!(!state.ready, "an uncertain session is never marked ready");
    assert!(
        state
            .error
            .as_deref()
            .unwrap()
            .contains("session options could not be restored"),
        "{:?}",
        state.error
    );
}

#[test]
fn corrupt_option_preferences_are_reported_and_not_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let options = root.path().join("options.json");
    std::fs::write(&options, "not json").unwrap();
    let mut backend = backend_with_option_preferences("settings", None, options.clone());
    backend.connect_acp().unwrap();
    let state = wait_controls(&backend, |s| s.ready);
    assert!(state.error.is_some(), "corrupt preferences are visible");
    backend
        .set_acp_option("fast", true.into(), state.revision)
        .unwrap();
    let changed = wait_controls(&backend, |s| !s.pending);
    assert_eq!(option_value(&changed, "fast"), &serde_json::json!(true));
    assert!(
        changed
            .error
            .as_deref()
            .is_some_and(|e| e.contains("saving its preference failed")),
        "{:?}",
        changed.error
    );
    assert_eq!(std::fs::read_to_string(&options).unwrap(), "not json");
}

#[test]
fn confirmed_model_survives_backend_recreation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("model.json");
    {
        let mut first = backend_with_preference("settings", Some(path.clone()));
        first.connect_acp().unwrap();
        let initial = wait_controls(&first, |s| s.ready);
        first
            .set_acp_option("model", "b".into(), initial.revision)
            .unwrap();
        let changed = wait_controls(&first, |s| !s.pending);
        assert!(changed.error.is_none(), "{:?}", changed.error);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "\"b\"");
        first
            .set_acp_option("model", "reject".into(), changed.revision)
            .unwrap();
        let rejected = wait_controls(&first, |s| !s.pending);
        assert!(rejected.error.is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "\"b\"");
    }
    let mut second = backend_with_preference("settings", Some(path.clone()));
    second.connect_acp().unwrap();
    let restored = wait_controls(&second, |s| s.ready);
    assert_eq!(restored.config.options[0].current, "b");
    assert!(restored.error.is_none(), "{:?}", restored.error);
    let mut other = backend_with_preference("settings", Some(root.path().join("other.json")));
    other.connect_acp().unwrap();
    assert_eq!(
        wait_controls(&other, |s| s.ready).config.options[0].current,
        "a"
    );
}

#[test]
fn unavailable_saved_model_keeps_agent_default() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("model.json");
    std::fs::write(&path, "\"removed\"").unwrap();
    let mut backend = backend_with_preference("settings", Some(path));
    backend.connect_acp().unwrap();
    let state = wait_controls(&backend, |s| s.ready);
    assert_eq!(state.config.options[0].current, "a");
    assert!(state.error.unwrap().contains("no longer available"));
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
    let messages = backend.messages();
    assert_eq!(messages[0].content, "I will inspect the project");
    assert_eq!(messages[0].source.as_deref(), Some("agent_progress"));
    assert_eq!(messages[1].source.as_deref(), Some("acp_reasoning"));
    assert_eq!(messages[2].source.as_deref(), Some("acp_activity"));
    let details = messages[2]
        .activity
        .as_ref()
        .expect("initial tool metadata survives a status-only update");
    assert_eq!(details.kind.as_deref(), Some("read"));
    assert_eq!(details.input.as_ref().unwrap()["path"], "src/sample.rs");
    assert_eq!(details.locations, ["src/sample.rs"]);
    let saved = serde_json::to_string(&messages[2]).unwrap();
    let restored: teshi_agent_runtime::AiChatMessage = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored.activity, messages[2].activity);
    assert_eq!(messages.last().unwrap().content, "Inspection finished");
    assert!(messages.last().unwrap().source.is_none());
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
    // Text preceding a tool event is progress, even if no final text follows.
    assert_eq!(backend.messages()[0].content, "hello");
    assert_eq!(
        backend.messages()[0].source.as_deref(),
        Some("agent_progress")
    );
    assert_eq!(
        backend.messages().last().unwrap().source.as_deref(),
        Some("acp_activity")
    );
    assert_eq!(backend.status(), AgentBackendStatus::Completed);
}

fn image_message(text: &str) -> ChatMessage {
    use teshi_core::llm::{ContentBlock, ImageSource, MessageContent};
    ChatMessage::text(
        "user",
        MessageContent::Blocks(vec![
            ContentBlock::Text { text: text.into() },
            ContentBlock::Image {
                source: ImageSource::Data {
                    media_type: "image/png".into(),
                    data: std::sync::Arc::from([1_u8, 2, 3]),
                },
            },
        ]),
    )
}

#[test]
fn test_image_blocks_reach_an_agent_that_accepts_images() {
    let mut backend = backend("images");
    backend.connect_acp().unwrap();
    assert!(wait_controls(&backend, |s| s.ready).image_prompts);
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![image_message("look")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    // Three bytes are four base64 characters.
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentBackendEvent::MessageChunk { content } if content == r#"["text", "image/png:4"]"#
        )),
        "{events:?}"
    );
}

#[test]
fn test_text_only_agent_rejects_images_before_sending() {
    let mut backend = backend("success");
    backend.connect_acp().unwrap();
    assert!(!wait_controls(&backend, |s| s.ready).image_prompts);
    backend.start().unwrap();
    let error = backend
        .submit_turn(None, vec![image_message("look")], None)
        .unwrap_err();
    assert!(
        error.to_string().contains("does not accept images"),
        "{error}"
    );
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
    // Cancelling ends the turn, not the connection: the worker and its
    // confirmed settings survive and reconnecting is a no-op.
    first.cancel();
    assert_eq!(first.status(), AgentBackendStatus::Cancelled);
    assert!(first.is_connected(), "cancel must not tear down the worker");
    first.connect_acp().unwrap();
    assert!(!first.shutdown_pending());
    assert_eq!(first.acp_controls().unwrap().config.options[0].current, "b");
    // Only an explicit teardown replaces the worker and rediscovers settings.
    first.request_shutdown();
    let deadline = Instant::now() + Duration::from_secs(3);
    while first.shutdown_pending() {
        first.poll_shutdown();
        assert!(
            Instant::now() < deadline,
            "shutdown did not complete after an explicit teardown request"
        );
        thread::sleep(Duration::from_millis(5));
    }
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
fn test_shutdown_of_a_wedged_agent_with_a_blocked_prompt_write_completes() {
    let mut backend = backend("stalled_stdin");
    backend.connect_acp().unwrap();
    wait_controls(&backend, |s| s.ready);
    backend.start().unwrap();
    // Larger than any pipe buffer, so the write parks until the agent reads.
    let restored_history = "x".repeat(8 * 1024 * 1024);
    backend
        .submit_turn(
            None,
            vec![ChatMessage::text("user", restored_history.as_str())],
            None,
        )
        .unwrap();
    thread::sleep(Duration::from_millis(500));
    backend.request_shutdown();
    let deadline = Instant::now() + Duration::from_secs(20);
    while backend.shutdown_pending() {
        backend.poll_shutdown();
        assert!(
            Instant::now() < deadline,
            "a wedged agent kept the backend in the stopping state"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(backend.start().is_ok(), "the conversation must reconnect");
}

/// Waits for the fixture's first streamed chunk so the turn is in flight.
fn wait_for_first_chunk(backend: &mut AgentBackendRuntime) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match backend.try_next_event() {
            Ok(Some(AgentBackendEvent::MessageChunk { .. })) => return,
            Ok(Some(_)) => {}
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "turn never started streaming");
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("event channel disconnected: {error}"),
        }
    }
}

#[test]
fn cancel_ends_the_turn_without_replacing_the_acp_session() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("agent.pid");
    let mut backend = backend_with(
        "cancel_keeps_session",
        vec![marker.display().to_string()],
        HashMap::new(),
        None,
    );
    backend.start().unwrap();
    backend
        .submit_turn(
            None,
            vec![ChatMessage::text("user", "first question")],
            None,
        )
        .unwrap();
    wait_for_first_chunk(&mut backend);
    let session = backend.acp_controls().unwrap().session_id.unwrap();
    let pid = std::fs::read_to_string(&marker).unwrap();

    backend.cancel();
    assert_eq!(backend.status(), AgentBackendStatus::Cancelled);
    assert!(backend.is_connected(), "cancel must keep the ACP process");

    // The follow-up arrives while the cancelled turn is still draining; the
    // worker must defer it into the same session instead of failing.
    backend.start().unwrap();
    assert!(backend.is_connected());
    backend
        .submit_turn(
            None,
            vec![
                ChatMessage::text("user", "first question"),
                ChatMessage::text("assistant", "working"),
                ChatMessage::text("user", "second question"),
            ],
            None,
        )
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentBackendEvent::MessageChunk { content } if content == "second-turn-ok"
        )),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentBackendEvent::Failed(_))),
        "{events:?}"
    );
    assert_eq!(
        backend.acp_controls().unwrap().session_id.unwrap(),
        session,
        "the follow-up turn must reuse the existing ACP session"
    );
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        pid,
        "the ACP process was replaced by a cancel"
    );
}

#[test]
fn cancel_answers_a_pending_permission_and_keeps_the_session() {
    let mut backend = backend_with_permission_host("cancel_then_permission");
    backend.start().unwrap();
    backend
        .submit_turn(
            None,
            vec![ChatMessage::text("user", "first question")],
            None,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match backend.try_next_event() {
            Ok(Some(AgentBackendEvent::PermissionRequest { .. })) => break,
            Ok(Some(_)) => {}
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "no permission request");
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("event channel disconnected: {error}"),
        }
    }
    backend.cancel();
    backend.start().unwrap();
    backend
        .submit_turn(
            None,
            vec![
                ChatMessage::text("user", "first question"),
                ChatMessage::text("user", "second question"),
            ],
            None,
        )
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentBackendEvent::MessageChunk { content } if content == "second-turn-ok"
        )),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentBackendEvent::Failed(_))),
        "{events:?}"
    );
}

#[test]
fn reconnecting_after_a_transport_failure_restores_the_transcript() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("crashed");
    let mut env = HashMap::new();
    env.insert(
        "TESHI_CRASH_STATE".to_string(),
        marker.display().to_string(),
    );
    let mut backend = backend_with("crash_once", Vec::new(), env, None);
    let messages = vec![
        ChatMessage::text("user", "previous question"),
        ChatMessage::text("assistant", "previous answer"),
        ChatMessage::text("user", "current question"),
    ];
    backend.start().unwrap();
    backend.submit_turn(None, messages.clone(), None).unwrap();
    let crashed = events_until_terminal(&mut backend);
    assert!(
        matches!(crashed.last(), Some(AgentBackendEvent::Failed(_))),
        "{crashed:?}"
    );

    // Reconnects after a failed transport, waiting for the old worker to exit.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        backend.poll_shutdown();
        if backend.start().is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the failed ACP worker never released the backend"
        );
        thread::sleep(Duration::from_millis(5));
    }
    backend.submit_turn(None, messages, None).unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentBackendEvent::MessageChunk { content } if content == "restored-ok"
        )),
        "{events:?}"
    );
}

/// Builds a backend whose automatic turn retries wait `backoff` each time.
fn backend_with_retry(mode: &str, backoff: Vec<Duration>) -> AgentBackendRuntime {
    let mut config = fixture_config(mode, Vec::new(), HashMap::new());
    config.turn_retry = TurnRetryPolicy { backoff };
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    backend.configure_acp(config).unwrap();
    backend
}

fn retry_attempts(events: &[AgentBackendEvent]) -> Vec<(u32, u32)> {
    events
        .iter()
        .filter_map(|event| match event {
            AgentBackendEvent::Retrying {
                attempt,
                max_attempts,
                ..
            } => Some((*attempt, *max_attempts)),
            _ => None,
        })
        .collect()
}

fn shows_retriable_notice(backend: &AgentBackendRuntime, events: &[AgentBackendEvent]) -> bool {
    events.iter().any(|event| {
        matches!(event, AgentBackendEvent::MessageChunk { content } if content.contains("RetriableError"))
    }) || backend
        .messages()
        .iter()
        .any(|message| message.content.contains("RetriableError"))
}

#[test]
fn test_transient_notice_without_progress_resends_the_original_prompt() {
    let mut backend = backend_with_retry("transient_once", vec![Duration::from_millis(10); 3]);
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "do task")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert_eq!(retry_attempts(&events), [(1, 3)], "{events:?}");
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentBackendEvent::MessageChunk { content }
                if content.starts_with("recovered:") && content.contains("do task")
        )),
        "the retry must resend the user's request: {events:?}"
    );
    assert!(
        matches!(events.last(), Some(AgentBackendEvent::Completed { .. })),
        "{events:?}"
    );
    assert!(
        !shows_retriable_notice(&backend, &events),
        "a retried notice must not reach the transcript"
    );
    assert_eq!(backend.status(), AgentBackendStatus::Completed);
}

#[test]
fn test_transient_notice_after_progress_asks_the_agent_to_continue() {
    let mut backend =
        backend_with_retry("transient_after_progress", vec![Duration::from_millis(10)]);
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "do task")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert_eq!(retry_attempts(&events), [(1, 1)], "{events:?}");
    assert!(
        events.iter().any(
            |event| matches!(event, AgentBackendEvent::MessageChunk { content } if content == "continued")
        ),
        "a retry after tool work must not resend the request: {events:?}"
    );
    assert!(!shows_retriable_notice(&backend, &events));
    let messages = backend.messages();
    assert_eq!(messages[0].content, "working");
    assert_eq!(messages[0].source.as_deref(), Some("agent_progress"));
    assert_eq!(messages.last().unwrap().content, "continued");
}

#[test]
fn test_transient_prompt_error_is_retried() {
    let mut backend = backend_with_retry(
        "transient_prompt_error_once",
        vec![Duration::from_millis(10)],
    );
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "do task")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert_eq!(retry_attempts(&events), [(1, 1)], "{events:?}");
    assert!(
        matches!(events.last(), Some(AgentBackendEvent::Completed { .. })),
        "{events:?}"
    );
}

#[test]
fn test_transient_failures_fail_the_turn_once_the_budget_is_spent() {
    for (backoff, retries, expected) in [
        (
            vec![Duration::from_millis(10); 2],
            vec![(1, 2), (2, 2)],
            "after 2 automatic retries",
        ),
        (
            Vec::new(),
            Vec::new(),
            "ACP prompt failed: Error: RetriableError",
        ),
    ] {
        let mut backend = backend_with_retry("transient_always", backoff);
        backend.start().unwrap();
        backend
            .submit_turn(None, vec![ChatMessage::text("user", "do task")], None)
            .unwrap();
        let events = events_until_terminal(&mut backend);
        assert_eq!(retry_attempts(&events), retries, "{events:?}");
        assert!(
            matches!(
                events.last(),
                Some(AgentBackendEvent::Failed(error))
                    if error.contains(expected) && error.contains("http/2 stream closed")
            ),
            "{events:?}"
        );
        assert_eq!(backend.status(), AgentBackendStatus::Failed);
    }
}

#[test]
fn test_cancel_during_retry_backoff_keeps_the_session() {
    let mut backend = backend_with_retry("transient_once", vec![Duration::from_secs(30)]);
    backend.start().unwrap();
    backend
        .submit_turn(
            None,
            vec![ChatMessage::text("user", "first question")],
            None,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match backend.try_next_event() {
            Ok(Some(AgentBackendEvent::Retrying { .. })) => break,
            Ok(Some(event)) => assert!(
                !matches!(
                    event,
                    AgentBackendEvent::Completed { .. } | AgentBackendEvent::Failed(_)
                ),
                "turn ended before retrying: {event:?}"
            ),
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "no retry was scheduled");
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("event channel disconnected: {error}"),
        }
    }
    let session = backend.acp_controls().unwrap().session_id;
    backend.cancel();
    assert_eq!(backend.status(), AgentBackendStatus::Cancelled);

    // The cancel must end the 30-second backoff, not wait it out.
    backend.start().unwrap();
    backend
        .submit_turn(
            None,
            vec![
                ChatMessage::text("user", "first question"),
                ChatMessage::text("user", "second question"),
            ],
            None,
        )
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentBackendEvent::MessageChunk { content }
                if content.starts_with("recovered:") && content.contains("second question")
        )),
        "{events:?}"
    );
    assert!(retry_attempts(&events).is_empty(), "{events:?}");
    assert_eq!(backend.acp_controls().unwrap().session_id, session);
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
