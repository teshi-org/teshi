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

fn backend_with_restore(mode: &str, restore_session_id: &str) -> AgentBackendRuntime {
    let mut config = fixture_config(mode, Vec::new(), HashMap::new());
    config.restore_session_id = Some(restore_session_id.into());
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    backend.configure_acp(config).unwrap();
    backend
}

fn history_messages() -> Vec<ChatMessage> {
    vec![
        ChatMessage::text("user", "previous question"),
        ChatMessage::text("assistant", "previous answer"),
        ChatMessage::text("user", "current question"),
    ]
}

#[test]
fn resume_restores_remote_context_without_text_reseeding() {
    let mut backend = backend_with_restore("resume_then_prompt", "saved-session");
    backend.connect_acp().unwrap();
    let controls = wait_controls(&backend, |s| s.ready);
    assert!(controls.remote_context_restored);
    assert_eq!(controls.session_id.as_deref(), Some("saved-session"));
    backend.start().unwrap();
    backend.submit_turn(None, history_messages(), None).unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentBackendEvent::MessageChunk { content } if content == "restored-ok"
        )),
        "{events:?}"
    );
}

#[test]
fn load_restores_remote_context_and_mutes_replay_from_ui_events() {
    let mut backend = backend_with_restore("load_then_prompt", "saved-session");
    backend.connect_acp().unwrap();
    let controls = wait_controls(&backend, |s| s.ready);
    assert!(controls.remote_context_restored);
    assert_eq!(controls.session_id.as_deref(), Some("saved-session"));
    // Replay frames must not surface as live message chunks before the turn.
    let mut leaked = Vec::new();
    while let Ok(Some(event)) = backend.try_next_event() {
        leaked.push(event);
    }
    assert!(
        !leaked
            .iter()
            .any(|e| matches!(e, AgentBackendEvent::MessageChunk { .. })),
        "session/load replay leaked into UI events: {leaked:?}"
    );
    backend.start().unwrap();
    backend.submit_turn(None, history_messages(), None).unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentBackendEvent::MessageChunk { content } if content == "restored-ok"
        )),
        "{events:?}"
    );
}

#[test]
fn failed_session_load_falls_back_to_new_and_text_seed() {
    let mut backend = backend_with_restore("load_missing", "saved-session");
    backend.connect_acp().unwrap();
    let controls = wait_controls(&backend, |s| s.ready);
    assert!(!controls.remote_context_restored);
    assert_eq!(controls.session_id.as_deref(), Some("fake-session"));
    // A failed restore falls back to a fresh session that is still usable, so
    // it must not put the connected chip into an error state.
    assert_eq!(controls.error, None);
    backend.start().unwrap();
    backend.submit_turn(None, history_messages(), None).unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentBackendEvent::MessageChunk { content } if content == "seeded-ok"
        )),
        "{events:?}"
    );
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
    let deadline = Instant::now() + Duration::from_secs(30);
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
    let deadline = Instant::now() + Duration::from_secs(30);
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
    let deadline = Instant::now() + Duration::from_secs(30);
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
    let deadline = Instant::now() + Duration::from_secs(15);
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
    let deadline = Instant::now() + Duration::from_secs(30);
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
    let deadline = Instant::now() + Duration::from_secs(30);
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
    let deadline = Instant::now() + Duration::from_secs(30);
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
    let deadline = Instant::now() + Duration::from_secs(30);
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
    let deadline = Instant::now() + Duration::from_secs(30);
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

    let deadline = Instant::now() + Duration::from_secs(30);
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

    let deadline = Instant::now() + Duration::from_secs(30);
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

#[test]
fn connection_pool_reuses_process_closes_sessions_and_evicts_crashed_process() {
    let temp = tempfile::tempdir().unwrap();
    let log_path = temp.path().join("pool.jsonl");
    let mut env = HashMap::new();
    env.insert("TESHI_POOL_LOG".into(), log_path.display().to_string());
    let pool = std::sync::Arc::new(teshi_acp::AcpConnectionPool::new());

    let make_backend = |pool: &std::sync::Arc<teshi_acp::AcpConnectionPool>,
                        env: &HashMap<String, String>| {
        let mut config = fixture_config("pooled_reuse", Vec::new(), env.clone());
        config.provider_id = Some("fake-pooled".into());
        config.connection_pool = Some(std::sync::Arc::clone(pool));
        let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
        backend.configure_acp(config).unwrap();
        backend
    };

    let mut first = make_backend(&pool, &env);
    let mut second = make_backend(&pool, &env);
    first.connect_acp().unwrap();
    let c1 = wait_controls(&first, |s| s.ready);
    assert!(c1.supports_list_sessions);
    assert!(c1.supports_delete_session);
    let sid1 = c1.session_id.expect("first session id");

    second.connect_acp().unwrap();
    let c2 = wait_controls(&second, |s| s.ready);
    let sid2 = c2.session_id.expect("second session id");
    assert_ne!(sid1, sid2, "pooled sessions must have distinct session ids");

    first.start().unwrap();
    first
        .submit_turn(None, vec![ChatMessage::text("user", "one")], None)
        .unwrap();
    let e1 = events_until_terminal(&mut first);
    let reply1 = e1
        .iter()
        .find_map(|e| match e {
            AgentBackendEvent::MessageChunk { content } => Some(content.clone()),
            _ => None,
        })
        .expect("first reply");

    second.start().unwrap();
    second
        .submit_turn(None, vec![ChatMessage::text("user", "two")], None)
        .unwrap();
    let e2 = events_until_terminal(&mut second);
    let reply2 = e2
        .iter()
        .find_map(|e| match e {
            AgentBackendEvent::MessageChunk { content } => Some(content.clone()),
            _ => None,
        })
        .expect("second reply");

    let pid1 = reply1
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("pid=")
        .unwrap()
        .to_string();
    let pid2 = reply2
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("pid=")
        .unwrap()
        .to_string();
    assert_eq!(pid1, pid2, "second session must reuse the first process");
    assert!(
        reply1.contains(&format!("sid={sid1};prompt=")) && reply1.contains("one"),
        "{reply1}"
    );
    assert!(
        reply2.contains(&format!("sid={sid2};prompt=")) && reply2.contains("two"),
        "{reply2}"
    );

    // Closing session 1 must send session/close without killing session 2.
    drop(first);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let log_now = std::fs::read_to_string(&log_path).unwrap_or_default();
        if log_now.contains("\"session/close\"") && log_now.contains(&sid1) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let log_text = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        log_text.contains("\"session/close\"") && log_text.contains(&sid1),
        "{log_text}"
    );

    second.start().unwrap();
    second
        .submit_turn(None, vec![ChatMessage::text("user", "still-alive")], None)
        .unwrap();
    let e2_after = events_until_terminal(&mut second);
    assert!(e2_after.iter().any(|e| matches!(
        e,
        AgentBackendEvent::MessageChunk { content }
            if content.contains(&format!("pid={pid1};sid={sid2};prompt="))
                && content.contains("still-alive")
    )));

    // Crashing the pooled process must evict it so a third session spawns a fresh PID.
    second.start().unwrap();
    second
        .submit_turn(None, vec![ChatMessage::text("user", "crash-now")], None)
        .unwrap();
    let crashed = events_until_terminal(&mut second);
    assert!(
        matches!(crashed.last(), Some(AgentBackendEvent::Failed(_))),
        "{crashed:?}"
    );
    drop(second);

    let mut third = make_backend(&pool, &env);
    third.connect_acp().unwrap();
    let _c3 = wait_controls(&third, |s| s.ready);
    third.start().unwrap();
    third
        .submit_turn(None, vec![ChatMessage::text("user", "after-crash")], None)
        .unwrap();
    let e3 = events_until_terminal(&mut third);
    let reply3 = e3
        .iter()
        .find_map(|e| match e {
            AgentBackendEvent::MessageChunk { content } => Some(content.clone()),
            _ => None,
        })
        .expect("third reply");
    let pid3 = reply3
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("pid=")
        .unwrap()
        .to_string();
    assert_ne!(pid3, pid1, "crashed pooled process must be evicted");

    drop(third);
    pool.block_on(pool.shutdown_all());
    assert!(!pool.shutdown_pending());
}

#[test]
fn connection_pool_exclusive_mode_isolates_backends_and_reuses_idle() {
    let pool = std::sync::Arc::new(teshi_acp::AcpConnectionPool::new());
    let make_backend = |pool: &std::sync::Arc<teshi_acp::AcpConnectionPool>| {
        let mut config = fixture_config("pooled_reuse", Vec::new(), HashMap::new());
        config.provider_id = Some("exclusive-agent".into());
        config.connection_pool = Some(std::sync::Arc::clone(pool));
        config.pool_mode = teshi_acp::AcpPoolMode::Exclusive {
            prewarm_standby: 0,
            max_idle: 2,
        };
        let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
        backend.configure_acp(config).unwrap();
        backend
    };

    let mut first = make_backend(&pool);
    let mut second = make_backend(&pool);
    first.connect_acp().unwrap();
    second.connect_acp().unwrap();
    let _c1 = wait_controls(&first, |s| s.ready);
    let _c2 = wait_controls(&second, |s| s.ready);

    // Both backends run concurrently on separate OS processes.
    first.start().unwrap();
    second.start().unwrap();
    first
        .submit_turn(None, vec![ChatMessage::text("user", "alpha")], None)
        .unwrap();
    second
        .submit_turn(None, vec![ChatMessage::text("user", "beta")], None)
        .unwrap();

    let e1 = events_until_terminal(&mut first);
    let e2 = events_until_terminal(&mut second);
    let pid_of = |events: &[AgentBackendEvent]| {
        events
            .iter()
            .find_map(|e| match e {
                AgentBackendEvent::MessageChunk { content } => content
                    .split(';')
                    .next()
                    .and_then(|s| s.strip_prefix("pid="))
                    .map(str::to_string),
                _ => None,
            })
            .expect("pid in reply")
    };
    let pid1 = pid_of(&e1);
    let pid2 = pid_of(&e2);
    assert_ne!(
        pid1, pid2,
        "concurrent exclusive backends must run on distinct processes"
    );

    // Dropping the first backend returns its live process to the idle cache.
    drop(first);
    let deadline = Instant::now() + Duration::from_secs(5);
    while pool.idle_connection_count() < 1 {
        assert!(
            Instant::now() < deadline,
            "dropped lease should become idle"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let mut third = make_backend(&pool);
    third.connect_acp().unwrap();
    let _c3 = wait_controls(&third, |s| s.ready);
    third.start().unwrap();
    third
        .submit_turn(None, vec![ChatMessage::text("user", "gamma")], None)
        .unwrap();
    let e3 = events_until_terminal(&mut third);
    let pid3 = pid_of(&e3);
    assert_eq!(
        pid3, pid1,
        "new exclusive backend should reuse the returned idle process"
    );

    drop(second);
    drop(third);
    pool.block_on(pool.shutdown_all());
    assert_eq!(pool.active_connection_count(), 0);
    assert!(!pool.shutdown_pending());
}

#[test]
fn available_commands_and_plan_updates_are_consumed_and_cleared() {
    let mut backend = backend("commands_and_plan");
    backend.connect_acp().unwrap();
    wait_controls(&backend, |s| s.ready);

    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "plan please")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    let cmds = events
        .iter()
        .find_map(|e| match e {
            AgentBackendEvent::AvailableCommands { commands } => Some(commands.clone()),
            _ => None,
        })
        .expect("AvailableCommands event");
    assert_eq!(cmds.len(), 2, "{cmds:?}");
    assert_eq!(cmds[0].name, "review");
    assert_eq!(cmds[0].input_hint.as_deref(), Some("path"));
    assert_eq!(cmds[1].name, "compact");

    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentBackendEvent::PlanUpdate { entries } if entries.len() == 2
        )),
        "{events:?}"
    );
    let plan_msg = backend
        .messages()
        .iter()
        .find(|m| m.source.as_deref() == Some("acp_plan"))
        .expect("acp_plan message recorded");
    assert!(
        plan_msg.content.contains("Read feature files"),
        "{}",
        plan_msg.content
    );
    assert!(
        plan_msg.content.contains("Verify scenario steps"),
        "{}",
        plan_msg.content
    );

    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "done")], None)
        .unwrap();
    let events2 = events_until_terminal(&mut backend);
    assert!(
        events2.iter().any(|e| matches!(
            e,
            AgentBackendEvent::PlanUpdate { entries } if entries.is_empty()
        )),
        "{events2:?}"
    );
    assert!(
        backend
            .messages()
            .iter()
            .all(|m| m.source.as_deref() != Some("acp_plan")),
        "plan_removed must clear acp_plan message"
    );
}

#[test]
fn interactive_authentication_required_recovers_after_authenticate_acp() {
    let mut backend = backend("auth_required");
    backend.connect_acp().unwrap();
    let auth_state = wait_controls(&backend, |s| s.auth_required);
    assert!(!auth_state.ready);
    assert_eq!(auth_state.auth_methods.len(), 2);
    assert_eq!(auth_state.auth_methods[0].id, "oauth");
    let failed_events = events_until_terminal(&mut backend);
    assert!(
        matches!(failed_events.last(), Some(AgentBackendEvent::Failed(_))),
        "{failed_events:?}"
    );

    backend.authenticate_acp("oauth".into()).unwrap();
    let ready_state = wait_controls(&backend, |s| s.ready && !s.auth_required);
    assert!(ready_state.error.is_none(), "{:?}", ready_state.error);

    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "hi")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentBackendEvent::MessageChunk { content } if content == "authed-ok"
        )),
        "{events:?}"
    );
    assert!(matches!(
        events.last(),
        Some(AgentBackendEvent::Completed { .. })
    ));
}

#[test]
fn failed_turn_on_ready_session_resets_worker_on_next_connect() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("pid.txt");
    let mut backend = backend_with(
        "prompt_failed",
        vec![marker.display().to_string()],
        HashMap::new(),
        None,
    );
    backend.connect_acp().unwrap();
    let initial = wait_controls(&backend, |s| s.ready);
    assert!(initial.ready);
    let first_pid = std::fs::read_to_string(&marker).unwrap();

    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "boom")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(matches!(events.last(), Some(AgentBackendEvent::Failed(_))));
    assert_eq!(backend.status(), AgentBackendStatus::Failed);
    // Worker must spontaneously clear ready and exit after turn failure.
    let stopped = wait_controls(&backend, |s| !s.ready);
    assert!(!stopped.ready);

    // Give the detached worker thread a brief moment to finish process teardown, then
    // verify the first reconnect attempt succeeds without failing with "still stopping".
    thread::sleep(Duration::from_millis(150));
    backend
        .connect_acp()
        .expect("first connect_acp after failed turn should succeed cleanly");
    wait_controls(&backend, |s| s.ready);
    let second_pid = std::fs::read_to_string(&marker).unwrap();
    assert_ne!(
        first_pid, second_pid,
        "connect_acp after a Failed turn must spawn a fresh worker process"
    );
}

#[test]
fn process_exit_during_auth_required_clears_auth_required_flag() {
    let mut backend = backend("auth_required_exit");
    backend.connect_acp().unwrap();
    let _ = wait_controls(&backend, |s| s.auth_required);
    let cleared = wait_controls(&backend, |s| !s.auth_required && s.error.is_some());
    assert!(!cleared.auth_required);
    assert!(!cleared.pending);
    assert!(
        cleared
            .error
            .as_deref()
            .is_some_and(|e| e.contains("exited before authentication")),
        "{:?}",
        cleared.error
    );
}

#[test]
fn manual_permission_host_is_restored_across_reconnect_after_failure() {
    let dir = tempfile::tempdir().unwrap();
    let crash_state = dir.path().join("crashed.flag");
    let mut env = HashMap::new();
    env.insert(
        "TESHI_CRASH_STATE".into(),
        crash_state.display().to_string(),
    );
    let mut config = fixture_config("crash_once_then_permission", Vec::new(), env);
    config.permission_policy = teshi_acp::PermissionPolicy::Manual;
    config.permission_host = Some(teshi_agent_runtime::ChannelPermissionHost::new());
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    backend.configure_acp(config).unwrap();

    // First turn crashes the agent process.
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "first")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(matches!(events.last(), Some(AgentBackendEvent::Failed(_))));
    assert_eq!(backend.status(), AgentBackendStatus::Failed);

    // Reconnect and submit a second turn that requires Manual permission approval.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        backend.poll_shutdown();
        match backend.connect_acp() {
            Ok(()) => break,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Err(err) => panic!("reconnect failed: {err}"),
        }
    }
    wait_controls(&backend, |s| s.ready);
    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "second")], None)
        .unwrap();

    let wait_perm = Instant::now() + Duration::from_secs(5);
    let mut request_id = None;
    let mut saw_completed = false;
    while Instant::now() < wait_perm {
        match backend.try_next_event() {
            Ok(Some(AgentBackendEvent::PermissionRequest {
                request_id: id,
                options,
            })) => {
                assert!(!options.is_empty());
                request_id = Some(id);
                assert!(
                    backend.respond_permission(id, Some("allow_once".into())),
                    "permission_host must be restored after reconnect"
                );
            }
            Ok(Some(AgentBackendEvent::Completed { .. })) => {
                saw_completed = true;
                break;
            }
            Ok(Some(AgentBackendEvent::Failed(err))) => panic!("unexpected failure: {err}"),
            Ok(_) => {}
            Err(std::sync::mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(10)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        }
    }
    assert!(
        request_id.is_some(),
        "reconnected backend must surface PermissionRequest"
    );
    assert!(
        saw_completed,
        "reconnected backend must complete after permission response"
    );
}

#[test]
fn authenticate_without_draining_failed_event_does_not_reset_authenticated_worker() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("pid.txt");
    let mut backend = backend_with(
        "auth_required",
        vec![marker.display().to_string()],
        HashMap::new(),
        None,
    );
    backend.connect_acp().unwrap();
    let auth_state = wait_controls(&backend, |s| s.auth_required);
    assert!(auth_state.auth_required);

    // Intentionally do NOT drain the AuthenticationRequired `Failed` event before authenticating.
    backend.authenticate_acp("oauth".into()).unwrap();
    let ready_state = wait_controls(&backend, |s| s.ready && !s.auth_required);
    assert_eq!(ready_state.confirmed_auth_method.as_deref(), Some("oauth"));
    let pid_before = std::fs::read_to_string(&marker).unwrap();

    // Polling events after authenticating must ignore the stale pre-auth Failed event.
    while let Ok(Some(ev)) = backend.try_next_event() {
        assert!(
            !matches!(ev, AgentBackendEvent::Failed(_)),
            "stale pre-auth Failed event leaked after authenticate_acp: {ev:?}"
        );
    }
    assert_ne!(backend.status(), AgentBackendStatus::Failed);

    backend.start().unwrap();
    let pid_after_start = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(
        pid_before, pid_after_start,
        "start() must keep the freshly authenticated worker instead of resetting it"
    );
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "hi")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    assert!(matches!(
        events.last(),
        Some(AgentBackendEvent::Completed { .. })
    ));
}

#[test]
fn failed_authenticate_does_not_persist_rejected_auth_method_across_reconnect() {
    let mut backend = backend("auth_required");
    backend.connect_acp().unwrap();
    let _ = wait_controls(&backend, |s| s.auth_required);
    let _ = events_until_terminal(&mut backend);

    // Try a rejected auth method first ("token" fails in fake_acp_agent.py).
    backend.authenticate_acp("token".into()).unwrap();
    let rejected = events_until_terminal(&mut backend);
    assert!(matches!(
        rejected.last(),
        Some(AgentBackendEvent::Failed(_))
    ));
    assert!(
        backend
            .acp_controls()
            .unwrap()
            .confirmed_auth_method
            .is_none(),
        "rejected auth method must not be marked confirmed"
    );

    // Force a fresh worker reconnect after failed auth; the rejected method must NOT be replayed automatically.
    backend.request_shutdown();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        backend.poll_shutdown();
        match backend.connect_acp() {
            Ok(()) => break,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Err(err) => panic!("reconnect failed: {err}"),
        }
    }
    let re_auth = wait_controls(&backend, |s| s.auth_required);
    assert!(
        re_auth.auth_required,
        "fresh worker must prompt for interactive auth instead of failing with rejected method: {re_auth:?}"
    );
    let _ = events_until_terminal(&mut backend);

    // Now succeed with the valid method ("oauth") and verify reconnect carries the confirmed method cleanly.
    backend.authenticate_acp("oauth".into()).unwrap();
    let ready = wait_controls(&backend, |s| s.ready && !s.auth_required);
    assert_eq!(ready.confirmed_auth_method.as_deref(), Some("oauth"));

    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "fail-prompt")], None)
        .unwrap();
    let prompt_fail = events_until_terminal(&mut backend);
    assert!(matches!(
        prompt_fail.last(),
        Some(AgentBackendEvent::Failed(_))
    ));
    let _ = wait_controls(&backend, |s| !s.ready);
    thread::sleep(Duration::from_millis(150));

    backend.connect_acp().unwrap();
    let recovered = wait_controls(&backend, |s| s.ready);
    assert!(recovered.ready);
    assert!(!recovered.auth_required);
    assert!(recovered.error.is_none(), "{:?}", recovered.error);
}

#[test]
fn auth_required_controls_are_visible_before_failed_event_is_polled() {
    let mut backend = backend("auth_required");
    backend.connect_acp().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match backend.try_next_event() {
            Ok(Some(AgentBackendEvent::Failed(_))) => {
                let controls = backend.acp_controls().unwrap();
                assert!(
                    controls.auth_required,
                    "auth_required must already be true when Failed event is observed: {controls:?}"
                );
                assert!(!controls.pending);
                assert!(!controls.auth_methods.is_empty());
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Empty) => {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for auth_required Failed event"
                );
                thread::sleep(Duration::from_millis(5));
            }
            Err(err) => panic!("event channel disconnected: {err}"),
        }
    }
}

#[test]
fn pooled_load_session_drains_large_history_replay_without_leaking_into_live_turn() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("large_replay_acp.py");
    std::fs::write(
        &script,
        r#"import json, sys

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method = msg.get("method")
    req_id = msg.get("id")
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": 1,
                "agentInfo": {"name": "replay-agent", "version": "1.0"},
                "agentCapabilities": {"loadSession": True},
            },
        })
    elif method == "session/load":
        sid = msg["params"]["sessionId"]
        for i in range(600):
            send({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": sid,
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": f"replay-{i}"},
                    },
                },
            })
        send({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": sid,
                "update": {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": [
                        {"name": "summary", "description": "Summarize history"}
                    ],
                },
            },
        })
        send({"jsonrpc": "2.0", "id": req_id, "result": {"sessionId": sid}})
    elif method == "session/prompt":
        sid = msg["params"]["sessionId"]
        send({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": sid,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "live-answer"},
                },
            },
        })
        send({"jsonrpc": "2.0", "id": req_id, "result": {"stopReason": "end_turn"}})
"#,
    )
    .unwrap();

    let pool = std::sync::Arc::new(teshi_acp::AcpConnectionPool::new());
    let mut config = AcpBackendConfig::new(AcpAgentCommand {
        program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        args: vec![script.display().to_string()],
        cwd: dir.path().to_path_buf(),
        env: HashMap::new(),
    });
    config.connection_pool = Some(std::sync::Arc::clone(&pool));
    config.pool_mode = teshi_acp::AcpPoolMode::Shared;
    config.provider_id = Some("large-replay-test".into());
    config.restore_session_id = Some("saved-large-session".into());
    config.client.initialize_timeout = Duration::from_secs(5);
    config.client.request_timeout = Duration::from_secs(5);
    config.client.shutdown_timeout = Duration::from_secs(2);

    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    backend.configure_acp(config).unwrap();
    backend.connect_acp().unwrap();
    let controls = wait_controls(&backend, |s| s.ready);
    assert!(controls.remote_context_restored);
    assert_eq!(controls.session_id.as_deref(), Some("saved-large-session"));

    let mut pre_turn_events = Vec::new();
    while let Ok(Some(ev)) = backend.try_next_event() {
        pre_turn_events.push(ev);
    }
    assert!(
        !pre_turn_events
            .iter()
            .any(|e| matches!(e, AgentBackendEvent::MessageChunk { .. })),
        "none of the 600 replay chunks may leak into live events: {pre_turn_events:?}"
    );
    assert!(
        pre_turn_events.iter().any(|e| matches!(
            e,
            AgentBackendEvent::AvailableCommands { commands }
                if commands.iter().any(|c| c.name == "summary")
        )),
        "trailing available_commands_update from replay must be preserved: {pre_turn_events:?}"
    );

    backend.start().unwrap();
    backend
        .submit_turn(None, vec![ChatMessage::text("user", "next question")], None)
        .unwrap();
    let events = events_until_terminal(&mut backend);
    let chunks: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            AgentBackendEvent::MessageChunk { content } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        chunks,
        vec!["live-answer"],
        "live turn must contain only live-answer without replay leftovers"
    );

    backend.request_shutdown();
    let deadline = Instant::now() + Duration::from_secs(5);
    while backend.shutdown_pending() {
        backend.poll_shutdown();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    pool.runtime_handle().unwrap().block_on(pool.shutdown_all());
}
