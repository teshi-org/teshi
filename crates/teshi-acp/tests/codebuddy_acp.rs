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
