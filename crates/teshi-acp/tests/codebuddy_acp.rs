//! Local integration tests against the real CodeBuddy Code ACP agent.
//!
//! These tests require a local `npx` installation and the
//! `@tencent-ai/codebuddy-code` package. They do not call remote LLMs or
//! cloud services; they only verify the stdio ACP handshake.

use std::{collections::HashMap, path::PathBuf, time::Duration};

use teshi_acp::{
    AcpClient, AcpResult, PermissionPolicy, client::AcpClientConfig, command::AcpAgentCommand,
};
use tokio::sync::mpsc;

fn codebuddy_command() -> AcpAgentCommand {
    AcpAgentCommand {
        program: PathBuf::from(if cfg!(windows) { "npx.cmd" } else { "npx" }),
        args: vec![
            "@tencent-ai/codebuddy-code".into(),
            "--acp".into(),
            "--no-session-persistence".into(),
        ],
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        env: HashMap::new(),
    }
}

fn client_config() -> AcpClientConfig {
    AcpClientConfig {
        initialize_timeout: Duration::from_secs(30),
        authentication_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        prompt_timeout: Some(Duration::from_secs(10)),
        shutdown_timeout: Duration::from_secs(5),
        ..Default::default()
    }
}

/// Verifies that the official CodeBuddy Code agent can be spawned and returns
/// a valid ACP initialize response advertising supported auth methods.
#[tokio::test]
#[ignore = "requires local npx and @tencent-ai/codebuddy-code installation"]
async fn codebuddy_acp_initialize_advertises_auth_methods() -> AcpResult<()> {
    let (events_tx, mut events_rx) = mpsc::channel(16);
    let mut client = AcpClient::spawn(
        codebuddy_command(),
        client_config(),
        events_tx,
        PermissionPolicy::Manual,
        None,
    )
    .await?;

    let initialized = client.initialize().await?;
    assert_eq!(initialized.protocol_version, Some(1));
    assert!(
        !initialized.auth_methods.is_empty(),
        "codebuddy-code should advertise at least one auth method"
    );
    assert!(
        initialized.auth_methods.iter().any(|m| m == "external"),
        "expected external auth method; got {:?}",
        initialized.auth_methods
    );

    client.shutdown().await?;
    // Drain any pending events so the channel does not deadlock the test runner.
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), events_rx.recv()).await
    {
    }
    Ok(())
}

/// Verifies that codebuddy-code rejects session creation until authenticated.
/// This reproduces the auth gate users hit before they can send prompts.
#[tokio::test]
#[ignore = "requires local npx and @tencent-ai/codebuddy-code installation"]
async fn codebuddy_acp_requires_auth_before_session() -> AcpResult<()> {
    let (events_tx, mut events_rx) = mpsc::channel(16);
    let mut client = AcpClient::spawn(
        codebuddy_command(),
        client_config(),
        events_tx,
        PermissionPolicy::Manual,
        None,
    )
    .await?;

    client.initialize().await?;
    let err = client
        .new_session(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .await
        .expect_err("session/new should fail without authentication");
    assert!(
        err.to_string().contains("AuthenticationRequired")
            || err.to_string().contains("authentication required"),
        "unexpected error: {err}"
    );

    client.shutdown().await?;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), events_rx.recv()).await
    {
    }
    Ok(())
}

/// Verifies that four concurrent CodeBuddy Code sessions run cleanly without
/// deadlocking when acquired through `AcpConnectionPool` in exclusive warm mode,
/// and that subsequent session acquisition reuses an idle/warm process.
#[test]
#[ignore = "requires locally installed and authenticated codebuddy-code"]
fn codebuddy_acp_exclusive_pool_runs_four_concurrent_sessions() {
    use std::collections::HashSet;
    use std::time::Instant;
    use teshi_acp::{AcpConnectionKey, AcpConnectionPool, AcpEvent, AcpPoolMode, profile};

    let root = profile::store_dir().expect("ACP store dir");
    let installed = profile::load(&root).expect("load installed ACP agents");
    let codebuddy = installed
        .into_iter()
        .find(|p| p.id == "codebuddy-code")
        .expect("codebuddy-code must be installed in Teshi ACP store");
    let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();
    let command = codebuddy.command(cwd.clone()).expect("build command");
    let key = AcpConnectionKey::from_command("codebuddy-code", &command, None);
    let cfg = AcpClientConfig {
        initialize_timeout: Duration::from_secs(30),
        request_timeout: Duration::from_secs(30),
        prompt_timeout: Some(Duration::from_secs(60)),
        shutdown_timeout: Duration::from_secs(5),
        ..Default::default()
    };

    let pool = AcpConnectionPool::new();
    let mode = AcpPoolMode::exclusive_warm();

    pool.block_on(async {
        let t_acquire = Instant::now();
        let (c1, c2, c3, c4) = tokio::join!(
            pool.acquire_with_mode(
                key.clone(),
                command.clone(),
                cfg.clone(),
                PermissionPolicy::Auto,
                None,
                mode,
            ),
            pool.acquire_with_mode(
                key.clone(),
                command.clone(),
                cfg.clone(),
                PermissionPolicy::Auto,
                None,
                mode,
            ),
            pool.acquire_with_mode(
                key.clone(),
                command.clone(),
                cfg.clone(),
                PermissionPolicy::Auto,
                None,
                mode,
            ),
            pool.acquire_with_mode(
                key.clone(),
                command.clone(),
                cfg.clone(),
                PermissionPolicy::Auto,
                None,
                mode,
            ),
        );
        let c1 = c1.expect("acquire 1");
        let c2 = c2.expect("acquire 2");
        let c3 = c3.expect("acquire 3");
        let c4 = c4.expect("acquire 4");
        let acquire_ms = t_acquire.elapsed().as_millis();
        let pids: Vec<u32> = [
            c1.process_id().expect("pid 1"),
            c2.process_id().expect("pid 2"),
            c3.process_id().expect("pid 3"),
            c4.process_id().expect("pid 4"),
        ]
        .to_vec();
        eprintln!("[live-4x] 4 concurrent acquires completed in {acquire_ms}ms, pids={pids:?}");
        assert_eq!(
            pids.iter().copied().collect::<HashSet<_>>().len(),
            4,
            "all 4 concurrent codebuddy sessions must have distinct PIDs"
        );

        let run_one = |label: &'static str,
                       client: std::sync::Arc<AcpClient<tokio::process::ChildStdin>>,
                       cwd: PathBuf| async move {
            let t0 = Instant::now();
            let (tx, mut rx) = mpsc::channel::<AcpEvent>(128);
            let session = client
                .new_session_with_mcp_routed(cwd, Vec::new(), tx, PermissionPolicy::Auto, None)
                .await
                .expect("session/new");
            let new_ms = t0.elapsed().as_millis();
            let prompt_text = format!("Reply with ONLY the single word: {label}");
            let t_prompt = Instant::now();
            let collector = tokio::spawn(async move {
                let mut text = String::new();
                while let Some(ev) = rx.recv().await {
                    if let AcpEvent::AgentMessageChunk(chunk) = ev {
                        text.push_str(&chunk);
                    }
                }
                text
            });
            let completion = client
                .prompt(&session, &prompt_text)
                .await
                .expect("session/prompt");
            let prompt_ms = t_prompt.elapsed().as_millis();
            client.unregister_session(&session.id).await;
            let text = collector.await.unwrap_or_default();
            eprintln!(
                "[live-4x] {label}: session/new={new_ms}ms prompt={prompt_ms}ms stop={completion:?} text={:?}",
                text.trim()
            );
            assert_eq!(completion.as_deref(), Some("end_turn"));
            assert!(
                !text.trim().is_empty(),
                "session {label} must return non-empty assistant text"
            );
        };

        let t_prompts = Instant::now();
        tokio::join!(
            run_one("ONE", std::sync::Arc::clone(c1.client()), cwd.clone()),
            run_one("TWO", std::sync::Arc::clone(c2.client()), cwd.clone()),
            run_one("THREE", std::sync::Arc::clone(c3.client()), cwd.clone()),
            run_one("FOUR", std::sync::Arc::clone(c4.client()), cwd.clone()),
        );
        eprintln!(
            "[live-4x] all 4 concurrent prompts finished in {}ms",
            t_prompts.elapsed().as_millis()
        );

        drop(c1);
        drop(c2);
        drop(c3);
        drop(c4);

        // Opening a 5th session should check out an already-initialized idle/standby process instantly.
        let t_reuse = Instant::now();
        let c5 = pool
            .acquire_with_mode(
                key.clone(),
                command.clone(),
                cfg.clone(),
                PermissionPolicy::Auto,
                None,
                mode,
            )
            .await
            .expect("acquire 5 (warm/idle reuse)");
        let reuse_ms = t_reuse.elapsed().as_millis();
        eprintln!(
            "[live-4x] 5th acquire (warm/idle checkout) took {reuse_ms}ms, pid={:?}",
            c5.process_id()
        );
        assert!(
            reuse_ms < 500,
            "warm/idle checkout should be near-instant, took {reuse_ms}ms"
        );
        run_one("FIVE", std::sync::Arc::clone(c5.client()), cwd.clone()).await;
        drop(c5);

        pool.shutdown_all().await;
        assert_eq!(pool.active_connection_count(), 0);
        assert!(!pool.shutdown_pending());
    });
}
