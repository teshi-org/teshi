//! Backend teardown must retain ownership until local transports have exited.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use teshi_acp::AcpAgentCommand;
use teshi_agent::backend::{AgentBackendEvent, AgentBackendKind, AgentBackendStatus};
use teshi_agent_runtime::backend::{AcpBackendConfig, AgentBackendRuntime};
use teshi_core::llm::ChatMessage;
use teshi_engine::llm::LlmConfig;
use teshi_engine::model_profile::{ApiStyle, DeepSeekThinking};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project_switch_agent.py")
}

fn python() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

struct Provider(Child);
impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_reply(backend: &mut AgentBackendRuntime) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if matches!(backend.try_next_event(), Ok(Some(AgentBackendEvent::MessageChunk { content })) if content.contains("A_AGENT_ACTIVE"))
        {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("fixture never entered its active turn");
}

fn stop_and_join(backend: &mut AgentBackendRuntime) {
    let requested = Instant::now();
    backend.request_shutdown();
    assert!(
        requested.elapsed() < Duration::from_millis(100),
        "shutdown blocked the frontend"
    );
    assert!(backend.shutdown_pending());
    assert_eq!(backend.status(), AgentBackendStatus::Cancelled);
    if backend.kind() == AgentBackendKind::Acp {
        let attempted = Instant::now();
        assert!(
            backend.start().is_err(),
            "a still-stopping ACP session was restarted"
        );
        assert!(attempted.elapsed() < Duration::from_millis(100));
    }
    assert!(
        matches!(backend.try_next_event(), Ok(None)),
        "old events survived shutdown"
    );
    let deadline = Instant::now() + Duration::from_secs(4);
    while backend.shutdown_pending() && Instant::now() < deadline {
        backend.poll_shutdown();
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !backend.shutdown_pending(),
        "backend did not acknowledge exit"
    );
}

#[test]
fn acp_teardown_is_nonblocking_and_joins_delayed_process_exit() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project-a");
    std::fs::create_dir(&project).unwrap();
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
    let mut config = AcpBackendConfig::new(AcpAgentCommand {
        program: python().into(),
        args: vec![
            fixture().display().to_string(),
            "acp".into(),
            root.path().display().to_string(),
        ],
        cwd: project.clone(),
        env: HashMap::new(),
    });
    config.client.initialize_timeout = Duration::from_secs(2);
    config.client.shutdown_timeout = Duration::from_secs(3);
    backend.configure_acp(config).unwrap();
    backend.start().unwrap();
    backend
        .submit_turn(
            None,
            vec![ChatMessage::text("user", "A_PROJECT_ONLY_REQUEST")],
            None,
        )
        .unwrap();
    wait_reply(&mut backend);
    stop_and_join(&mut backend);
    let trace = std::fs::read_to_string(root.path().join("agent-evidence/trace.jsonl")).unwrap();
    assert!(trace.contains("acp_late_attempt"));
    assert!(trace.contains("acp_exit"));
    let path = project.join("agent-writes.txt");
    let before = std::fs::read(&path).unwrap();
    thread::sleep(Duration::from_millis(75));
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert!(matches!(backend.try_next_event(), Ok(None)));
}

#[test]
fn native_teardown_closes_a_stalled_stream_and_joins_worker() {
    // Keep environment changes in a child test process: local HTTP fixtures
    // must not traverse a user's proxy or race other tests' configuration.
    if std::env::var_os("TESHI_LOCAL_SHUTDOWN_TEST_CHILD").is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "native_teardown_closes_a_stalled_stream_and_joins_worker",
                "--nocapture",
            ])
            .env("TESHI_LOCAL_SHUTDOWN_TEST_CHILD", "1")
            .env("NO_PROXY", "127.0.0.1,localhost,::1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let _provider = Provider(
        Command::new(python())
            .args([
                fixture().as_os_str(),
                Path::new("native").as_os_str(),
                root.path().as_os_str(),
            ])
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(root.path().join("provider-stderr.log")).unwrap())
            .spawn()
            .unwrap(),
    );
    let port = root.path().join("agent-evidence/native-port");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !port.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let config = LlmConfig {
        api_key: "isolated-fixture-key".into(),
        base_url: format!(
            "http://127.0.0.1:{}/v1",
            std::fs::read_to_string(port).unwrap()
        ),
        model: "project-fixture".into(),
        max_tokens: 100,
        temperature: 0.0,
        context_window: None,
        provider: "openai".into(),
        api_style: ApiStyle::ChatCompletions,
        thinking: DeepSeekThinking::High,
        stream: true,
        http_headers: HashMap::new(),
        chat_options: HashMap::new(),
    };
    let mut backend = AgentBackendRuntime::new(AgentBackendKind::Native, 1);
    backend.attach_native_model(config).unwrap();
    backend.start().unwrap();
    backend
        .submit_turn(
            None,
            vec![ChatMessage::text("user", "A_PROJECT_ONLY_REQUEST")],
            None,
        )
        .unwrap();
    wait_reply(&mut backend);
    stop_and_join(&mut backend);
    let trace_path = root.path().join("agent-evidence/trace.jsonl");
    let deadline = Instant::now() + Duration::from_secs(1);
    while !std::fs::read_to_string(&trace_path)
        .unwrap()
        .contains("native_a_closed")
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
    }
    let trace = std::fs::read_to_string(trace_path).unwrap();
    assert!(
        trace.contains("native_a_closed"),
        "{trace}\n{}",
        std::fs::read_to_string(root.path().join("provider-stderr.log")).unwrap()
    );
}
