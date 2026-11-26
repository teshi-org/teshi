use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use serde_json::json;
use teshi_acp::{
    AcpClient, AcpEvent, PermissionDecision, PermissionHost, PermissionOption, PermissionPolicy,
    client::AcpClientConfig,
    jsonrpc::{JsonRpcMessage, error_response, read_next, request, response, write_message},
};
use tokio::{io::BufReader, sync::mpsc, time::Duration};

fn test_config() -> AcpClientConfig {
    AcpClientConfig {
        initialize_timeout: Duration::from_millis(200),
        authentication_timeout: Duration::from_millis(200),
        request_timeout: Duration::from_millis(200),
        shutdown_timeout: Duration::from_millis(100),
        ..Default::default()
    }
}

#[test]
fn prompts_have_no_default_turn_timeout() {
    assert_eq!(AcpClientConfig::default().prompt_timeout, None);
}

#[tokio::test]
async fn settings_use_advertised_routes_and_typed_values() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (tx, _rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(cr, cw, test_config(), tx, PermissionPolicy::Manual, None);
    let server = tokio::spawn(async move {
        let mut reader = BufReader::new(sr);
        for (method_expected, result) in [
            ("initialize", json!({"protocolVersion":1})),
            (
                "session/new",
                json!({"sessionId":"s","configOptions":[{"id":"fast","type":"boolean","currentValue":false}],"modes":{"currentModeId":"ask","availableModes":[{"id":"ask","name":"Ask"}]},"models":{"currentModelId":"a","availableModels":[{"modelId":"a","name":"A"}]}}),
            ),
            ("session/set_config_option", json!({})),
            ("session/set_mode", json!({})),
            ("session/set_model", json!({})),
        ] {
            let JsonRpcMessage::Request { id, method, params } =
                read_next(&mut reader).await.unwrap().unwrap()
            else {
                panic!("request expected");
            };
            assert_eq!(method, method_expected);
            match method.as_str() {
                "session/set_config_option" => assert_eq!(
                    params,
                    json!({"sessionId":"s","configId":"fast","type":"boolean","value":true})
                ),
                "session/set_mode" => assert_eq!(params, json!({"sessionId":"s","modeId":"ask"})),
                "session/set_model" => assert_eq!(params, json!({"sessionId":"s","modelId":"a"})),
                _ => {}
            }
            write_message(&mut sw, &response(id, result)).await.unwrap();
        }
    });
    client.initialize().await.unwrap();
    let session = client.new_session(PathBuf::from("/project")).await.unwrap();
    for (option, value) in
        session
            .config
            .options
            .iter()
            .zip([json!(true), json!("ask"), json!("a")])
    {
        client
            .set_session_option(&session, option, value)
            .await
            .unwrap();
    }
    server.await.unwrap();
}

#[tokio::test]
async fn initialize_sends_required_fields_and_session_uses_cwd() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (events_tx, _events_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        let init = read_next(&mut r).await.unwrap().unwrap();
        match init {
            JsonRpcMessage::Request { id, method, params } => {
                assert_eq!(method, "initialize");
                assert_eq!(params["protocolVersion"], 1);
                assert_eq!(params["clientInfo"]["name"], "teshi");
                assert!(!params["clientInfo"]["version"].as_str().unwrap().is_empty());
                assert_eq!(params["clientCapabilities"]["filesystem"], false);
                write_message(
                    &mut sw,
                    &response(id, json!({"protocolVersion":1,"capabilities":{}})),
                )
                .await
                .unwrap();
            }
            _ => panic!("expected init request"),
        }
        let sess = read_next(&mut r).await.unwrap().unwrap();
        match sess {
            JsonRpcMessage::Request { id, method, params } => {
                assert_eq!(method, "session/new");
                assert!(params["cwd"].as_str().unwrap().contains("C:"));
                assert_eq!(params["mcpServers"], json!([]));
                assert!(params.get("projectPath").is_none());
                write_message(&mut sw, &response(id, json!({"sessionId":"s1"})))
                    .await
                    .unwrap();
            }
            _ => panic!("expected session request"),
        }
    });

    client.initialize().await.unwrap();
    let session = client
        .new_session(PathBuf::from(r"C:\project\app"))
        .await
        .unwrap();
    assert_eq!(session.id, "s1");
    server.await.unwrap();
}

#[tokio::test]
async fn streaming_updates_are_preserved_and_prompt_concurrency_rejected() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (events_tx, mut events_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(
                &mut sw,
                &response(id, json!({"protocolVersion":1,"capabilities":{}})),
            )
            .await
            .unwrap();
        }
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(&mut sw, &response(id, json!({"sessionId":"s1"})))
                .await
                .unwrap();
        }
        if let JsonRpcMessage::Request { id, method, params } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/prompt");
            assert_eq!(params["prompt"][0]["type"], "text");
            assert_eq!(params["prompt"][0]["text"], "hello");
            write_message(
                &mut sw,
                &json!({"jsonrpc":"2.0","method":"session/update","params":{"text":"A"}}),
            )
            .await
            .unwrap();
            write_message(
                &mut sw,
                &json!({"jsonrpc":"2.0","method":"session/update","params":{"text":"B"}}),
            )
            .await
            .unwrap();
            write_message(&mut sw, &json!({"jsonrpc":"2.0","method":"session/update","params":{"toolCall":{"id":"t"}}})).await.unwrap();
            write_message(
                &mut sw,
                &json!({"jsonrpc":"2.0","method":"session/update","params":{"text":"C"}}),
            )
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            write_message(&mut sw, &response(id, json!({"stopReason":"end_turn"})))
                .await
                .unwrap();
        }
    });

    client.initialize().await.unwrap();
    let session = client.new_session(PathBuf::from("/project")).await.unwrap();
    let prompt_future = client.prompt(&session, "hello");
    tokio::pin!(prompt_future);
    assert!(
        tokio::time::timeout(Duration::from_millis(5), prompt_future.as_mut())
            .await
            .is_err()
    );
    assert!(matches!(
        client.prompt(&session, "second").await,
        Err(teshi_acp::AcpError::PromptAlreadyRunning)
    ));
    assert_eq!(prompt_future.await.unwrap(), Some("end_turn".into()));
    let mut seen = vec![];
    for _ in 0..5 {
        seen.push(events_rx.recv().await.unwrap());
    }
    assert!(matches!(&seen[0], AcpEvent::AgentMessageChunk(s) if s == "A"));
    assert!(matches!(&seen[1], AcpEvent::AgentMessageChunk(s) if s == "B"));
    assert!(matches!(&seen[2], AcpEvent::ToolCall(_)));
    assert!(matches!(&seen[3], AcpEvent::AgentMessageChunk(s) if s == "C"));
    assert!(
        matches!(&seen[4], AcpEvent::Completed { stop_reason, detail: None } if stop_reason.as_deref() == Some("end_turn"))
    );
    server.await.unwrap();
}

#[tokio::test]
async fn dropped_prompt_future_cancels_and_keeps_slot_until_agent_finishes() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (events_tx, _events_rx) = mpsc::channel(16);
    let (cancel_seen_tx, cancel_seen_rx) = tokio::sync::oneshot::channel();
    let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(
                &mut sw,
                &response(id, json!({"protocolVersion":1,"capabilities":{}})),
            )
            .await
            .unwrap();
        }
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(&mut sw, &response(id, json!({"sessionId":"s1"})))
                .await
                .unwrap();
        }
        let prompt_id = if let JsonRpcMessage::Request { id, method, params } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/prompt");
            assert_eq!(params["prompt"][0]["text"], "abandoned");
            id
        } else {
            panic!("expected first prompt request")
        };
        if let JsonRpcMessage::Notification { method, params } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/cancel");
            assert_eq!(params["sessionId"], "s1");
            let _ = cancel_seen_tx.send(());
            let _ = finish_rx.await;
            write_message(
                &mut sw,
                &response(prompt_id, json!({"stopReason":"cancelled"})),
            )
            .await
            .unwrap();
        } else {
            panic!("expected cancellation notification")
        }
    });

    client.initialize().await.unwrap();
    let session = client.new_session(PathBuf::from("/project")).await.unwrap();
    let mut prompt_future = Box::pin(client.prompt(&session, "abandoned"));
    assert!(
        tokio::time::timeout(Duration::from_millis(5), prompt_future.as_mut())
            .await
            .is_err()
    );
    drop(prompt_future);
    cancel_seen_rx.await.unwrap();
    assert!(matches!(
        client.prompt(&session, "second").await,
        Err(teshi_acp::AcpError::PromptAlreadyRunning)
    ));
    finish_tx.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn list_and_load_use_v1_capabilities_and_wire_schema() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (events_tx, _events_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(
                &mut sw,
                &response(
                    id,
                    json!({
                        "protocolVersion": 1,
                        "agentCapabilities": {
                            "loadSession": true,
                            "sessionCapabilities": {"list": {}}
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        }
        if let JsonRpcMessage::Request { id, method, params } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/list");
            assert_eq!(params, json!({}));
            write_message(&mut sw, &response(id, json!({"sessions": []})))
                .await
                .unwrap();
        }
        if let JsonRpcMessage::Request { id, method, params } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/load");
            assert_eq!(params["sessionId"], "saved");
            assert_eq!(params["cwd"], "/project");
            assert_eq!(params["mcpServers"], json!([]));
            write_message(&mut sw, &response(id, json!({})))
                .await
                .unwrap();
        }
        if let JsonRpcMessage::Request { id, method, params } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/load");
            assert_eq!(params["sessionId"], "saved");
            assert_eq!(params["cwd"], "/project");
            assert_eq!(params["mcpServers"][0]["name"], "teshi-tools");
            assert_eq!(params["mcpServers"][0]["command"], "teshi");
            write_message(&mut sw, &response(id, json!({"sessionId":"saved"})))
                .await
                .unwrap();
        }
    });

    let init = client.initialize().await.unwrap();
    assert!(init.supports_load_session());
    assert!(!init.supports_resume_session());
    assert_eq!(
        client.list_sessions().await.unwrap(),
        json!({"sessions": []})
    );
    let loaded = client
        .load_session("saved", PathBuf::from("/project"))
        .await
        .unwrap();
    assert_eq!(loaded.id, "saved");
    assert_eq!(loaded.cwd, PathBuf::from("/project"));
    let with_mcp = client
        .load_session_with_mcp(
            "saved",
            PathBuf::from("/project"),
            vec![teshi_acp::McpServer::Stdio(teshi_acp::McpServerStdio::new(
                "teshi-tools",
                "teshi",
            ))],
        )
        .await
        .unwrap();
    assert_eq!(with_mcp.id, "saved");
    server.await.unwrap();
}

#[tokio::test]
async fn resume_session_requires_capability_and_forwards_mcp_servers() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (events_tx, _events_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(
                &mut sw,
                &response(
                    id,
                    json!({
                        "protocolVersion": 1,
                        "agentCapabilities": {
                            "sessionCapabilities": {"resume": {}}
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        }
        if let JsonRpcMessage::Request { id, method, params } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/resume");
            assert_eq!(params["sessionId"], "live");
            assert_eq!(params["cwd"], "/project");
            assert_eq!(params["mcpServers"][0]["name"], "teshi-tools");
            write_message(&mut sw, &response(id, json!({"sessionId":"live"})))
                .await
                .unwrap();
        }
    });

    let init = client.initialize().await.unwrap();
    assert!(init.supports_resume_session());
    assert!(!init.supports_load_session());
    assert!(matches!(
        client.load_session("live", PathBuf::from("/project")).await,
        Err(teshi_acp::AcpError::CapabilityUnsupported("session/load"))
    ));
    let resumed = client
        .resume_session_with_mcp(
            "live",
            PathBuf::from("/project"),
            vec![teshi_acp::McpServer::Stdio(teshi_acp::McpServerStdio::new(
                "teshi-tools",
                "teshi",
            ))],
        )
        .await
        .unwrap();
    assert_eq!(resumed.id, "live");
    assert_eq!(resumed.cwd, PathBuf::from("/project"));
    server.await.unwrap();
}

#[tokio::test]
async fn cancel_keeps_prompt_slot_reserved_until_prompt_finishes() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (events_tx, _events_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(
                &mut sw,
                &response(id, json!({"protocolVersion":1,"capabilities":{}})),
            )
            .await
            .unwrap();
        }
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(&mut sw, &response(id, json!({"sessionId":"s1"})))
                .await
                .unwrap();
        }
        let prompt_id = if let JsonRpcMessage::Request { id, method, .. } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/prompt");
            id
        } else {
            panic!("expected prompt request")
        };
        if let JsonRpcMessage::Notification { method, .. } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/cancel");
        } else {
            panic!("expected cancel notification")
        }
        write_message(
            &mut sw,
            &response(prompt_id, json!({"stopReason":"cancelled"})),
        )
        .await
        .unwrap();
    });

    client.initialize().await.unwrap();
    let session = client.new_session(PathBuf::from("/project")).await.unwrap();
    let prompt_future = client.prompt(&session, "hello");
    tokio::pin!(prompt_future);
    assert!(
        tokio::time::timeout(Duration::from_millis(5), prompt_future.as_mut())
            .await
            .is_err()
    );
    client.cancel(&session).await.unwrap();
    assert!(matches!(
        client.prompt(&session, "second").await,
        Err(teshi_acp::AcpError::PromptAlreadyRunning)
    ));
    assert_eq!(prompt_future.await.unwrap(), Some("cancelled".into()));
    server.await.unwrap();
}

struct RecordingHost(Arc<Mutex<bool>>);
impl PermissionHost for RecordingHost {
    fn request_permission<'a>(
        &'a self,
        options: &'a [PermissionOption],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = teshi_acp::AcpResult<PermissionDecision>> + Send + 'a>,
    > {
        Box::pin(async move {
            *self.0.lock().unwrap() = true;
            Ok(PermissionDecision::Selected {
                option_id: options[0].id.clone(),
            })
        })
    }
}

struct PendingHost;

impl PermissionHost for PendingHost {
    fn request_permission<'a>(
        &'a self,
        _options: &'a [PermissionOption],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = teshi_acp::AcpResult<PermissionDecision>> + Send + 'a>,
    > {
        Box::pin(std::future::pending())
    }
}

#[tokio::test]
async fn cancelling_prompt_cancels_pending_permission_request() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (events_tx, mut events_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Manual,
        Some(Arc::new(PendingHost)),
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(
                &mut sw,
                &response(id, json!({"protocolVersion":1,"agentCapabilities":{}})),
            )
            .await
            .unwrap();
        }
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(&mut sw, &response(id, json!({"sessionId":"s1"})))
                .await
                .unwrap();
        }
        let prompt_id = if let JsonRpcMessage::Request { id, method, .. } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/prompt");
            id
        } else {
            panic!("expected prompt")
        };
        write_message(
            &mut sw,
            &request(
                teshi_acp::jsonrpc::JsonRpcId::Number(91),
                "session/request_permission",
                json!({
                    "sessionId": "s1",
                    "options": [{"optionId":"allow_once","name":"Allow once","kind":"allow_once"}]
                }),
            ),
        )
        .await
        .unwrap();
        if let JsonRpcMessage::Notification { method, .. } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/cancel");
        } else {
            panic!("expected session/cancel")
        }
        if let JsonRpcMessage::Response { id, result } = read_next(&mut r).await.unwrap().unwrap() {
            assert_eq!(id, teshi_acp::jsonrpc::JsonRpcId::Number(91));
            assert_eq!(result, json!({"outcome":{"outcome":"cancelled"}}));
        } else {
            panic!("expected cancelled permission response")
        }
        write_message(
            &mut sw,
            &response(prompt_id, json!({"stopReason":"cancelled"})),
        )
        .await
        .unwrap();
    });

    client.initialize().await.unwrap();
    let session = client.new_session(PathBuf::from("/project")).await.unwrap();
    let prompt = client.prompt(&session, "needs permission");
    tokio::pin!(prompt);
    assert!(
        tokio::time::timeout(Duration::from_millis(5), prompt.as_mut())
            .await
            .is_err()
    );
    assert!(matches!(
        events_rx.recv().await.unwrap(),
        AcpEvent::PermissionRequest(_)
    ));
    client.cancel(&session).await.unwrap();
    assert_eq!(prompt.await.unwrap(), Some("cancelled".into()));
    server.await.unwrap();
}

#[tokio::test]
async fn manual_permission_waits_for_host_and_cursor_extensions_do_not_deadlock() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (events_tx, mut events_rx) = mpsc::channel(16);
    let called = Arc::new(Mutex::new(false));
    let host = Arc::new(RecordingHost(called.clone()));
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Manual,
        Some(host),
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(
                &mut sw,
                &response(id, json!({"protocolVersion":1,"capabilities":{}})),
            )
            .await
            .unwrap();
        }
        write_message(
            &mut sw,
            &request(
                teshi_acp::jsonrpc::JsonRpcId::Number(77),
                "session/request_permission",
                json!({"options":[
                    {"optionId":"allow_once","name":"Allow once","kind":"allow_once"},
                    {"optionId":"reject","name":"Reject","kind":"reject_once"}
                ]}),
            ),
        )
        .await
        .unwrap();
        if let JsonRpcMessage::Response { id, result } = read_next(&mut r).await.unwrap().unwrap() {
            assert_eq!(id, teshi_acp::jsonrpc::JsonRpcId::Number(77));
            assert_eq!(result["outcome"]["outcome"], "selected");
            assert_eq!(result["outcome"]["optionId"], "allow_once");
        } else {
            panic!("expected permission response")
        }
        write_message(
            &mut sw,
            &request(
                teshi_acp::jsonrpc::JsonRpcId::Number(78),
                "cursor/ask_question",
                json!({"q":"x"}),
            ),
        )
        .await
        .unwrap();
        if let JsonRpcMessage::Error { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            assert_eq!(id, Some(teshi_acp::jsonrpc::JsonRpcId::Number(78)));
        } else {
            panic!("expected extension error")
        }
    });

    client.initialize().await.unwrap();
    assert!(matches!(
        events_rx.recv().await.unwrap(),
        AcpEvent::PermissionRequest(_)
    ));
    server.await.unwrap();
    assert!(*called.lock().unwrap());
}

#[tokio::test]
async fn new_session_retries_after_agent_returns_auth_required() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (events_tx, _events_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(
                &mut sw,
                &response(
                    id,
                    json!({"protocolVersion":1,"authMethods":[{"id":"test-login"}]}),
                ),
            )
            .await
            .unwrap();
        }
        if let JsonRpcMessage::Request { id, method, .. } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/new");
            write_message(
                &mut sw,
                &error_response(Some(id), -32000, "authentication required"),
            )
            .await
            .unwrap();
        }
        if let JsonRpcMessage::Request { id, method, .. } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "authenticate");
            write_message(&mut sw, &response(id, json!({})))
                .await
                .unwrap();
        }
        if let JsonRpcMessage::Request { id, method, .. } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "session/new");
            write_message(&mut sw, &response(id, json!({"sessionId":"s1"})))
                .await
                .unwrap();
        }
    });

    client.initialize().await.unwrap();
    assert!(matches!(
        client.new_session(PathBuf::from("/project")).await,
        Err(teshi_acp::AcpError::AuthenticationRequired)
    ));
    client.authenticate("test-login").await.unwrap();
    assert_eq!(
        client
            .new_session(PathBuf::from("/project"))
            .await
            .unwrap()
            .id,
        "s1"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn schema_shaped_auth_and_updates_are_handled() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (events_tx, mut events_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        if let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() {
            write_message(
                &mut sw,
                &response(
                    id,
                    json!({
                        "protocolVersion":1,
                        "agentCapabilities":{"loadSession":true},
                        "authMethods":[{"id":"test-login","name":"Test Login"}]
                    }),
                ),
            )
            .await
            .unwrap();
        }
        if let JsonRpcMessage::Request { id, method, params } =
            read_next(&mut r).await.unwrap().unwrap()
        {
            assert_eq!(method, "authenticate");
            assert_eq!(params["methodId"], "test-login");
            write_message(&mut sw, &response(id, json!({})))
                .await
                .unwrap();
        }
        write_message(
            &mut sw,
            &json!({
                "jsonrpc":"2.0",
                "method":"session/update",
                "params":{
                    "sessionId":"s1",
                    "update":{
                        "sessionUpdate":"agent_message_chunk",
                        "content":{"content":{"type":"text","text":"schema text"}}
                    }
                }
            }),
        )
        .await
        .unwrap();
    });

    let init = client.initialize().await.unwrap();
    assert_eq!(init.auth_methods, vec!["test-login"]);
    assert_eq!(init.capabilities["loadSession"], true);
    client.authenticate("test-login").await.unwrap();
    assert!(matches!(
        events_rx.recv().await.unwrap(),
        AcpEvent::AgentMessageChunk(s) if s == "schema text"
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn process_death_unblocks_pending_initialize() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    drop(server_io);
    let (cr, cw) = tokio::io::split(client_io);
    let (events_tx, _events_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Auto,
        None,
    );
    assert!(client.initialize().await.is_err());
}

#[tokio::test]
async fn multiplexed_sessions_route_updates_permissions_and_turns_independently() {
    let (client_io, server_io) = tokio::io::duplex(16384);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (default_tx, mut default_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        default_tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        let JsonRpcMessage::Request { id, method, .. } = read_next(&mut r).await.unwrap().unwrap()
        else {
            panic!("init expected");
        };
        assert_eq!(method, "initialize");
        write_message(
            &mut sw,
            &response(id, json!({"protocolVersion":1,"agentCapabilities":{}})),
        )
        .await
        .unwrap();

        let JsonRpcMessage::Request { id, method, .. } = read_next(&mut r).await.unwrap().unwrap()
        else {
            panic!("session 1 expected");
        };
        assert_eq!(method, "session/new");
        write_message(&mut sw, &response(id, json!({"sessionId":"s1"})))
            .await
            .unwrap();

        let JsonRpcMessage::Request { id, method, .. } = read_next(&mut r).await.unwrap().unwrap()
        else {
            panic!("session 2 expected");
        };
        assert_eq!(method, "session/new");
        write_message(&mut sw, &response(id, json!({"sessionId":"s2"})))
            .await
            .unwrap();

        let mut prompt_ids = std::collections::HashMap::new();
        for _ in 0..2 {
            let JsonRpcMessage::Request { id, method, params } =
                read_next(&mut r).await.unwrap().unwrap()
            else {
                panic!("prompt expected");
            };
            assert_eq!(method, "session/prompt");
            let sid = params["sessionId"].as_str().unwrap().to_string();
            prompt_ids.insert(sid, id);
        }

        // Send interleaved updates for both sessions and a permission request for s2.
        write_message(
            &mut sw,
            &json!({
                "jsonrpc":"2.0",
                "method":"session/update",
                "params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"from-s1"}}}
            }),
        )
        .await
        .unwrap();
        write_message(
            &mut sw,
            &json!({
                "jsonrpc":"2.0",
                "method":"session/update",
                "params":{"sessionId":"s2","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"from-s2"}}}
            }),
        )
        .await
        .unwrap();
        write_message(
            &mut sw,
            &request(
                teshi_acp::jsonrpc::JsonRpcId::Number(99),
                "session/request_permission",
                json!({
                    "sessionId": "s2",
                    "options": [{"optionId": "allow_once", "kind": "allow_once"}]
                }),
            ),
        )
        .await
        .unwrap();
        let perm_reply = read_next(&mut r).await.unwrap().unwrap();
        match perm_reply {
            JsonRpcMessage::Response { id, result } => {
                assert_eq!(id, teshi_acp::jsonrpc::JsonRpcId::Number(99));
                assert_eq!(result["outcome"]["optionId"], "allow_once");
            }
            other => panic!("unexpected perm reply: {other:?}"),
        }

        write_message(
            &mut sw,
            &response(
                prompt_ids.remove("s1").unwrap(),
                json!({"stopReason":"end_turn"}),
            ),
        )
        .await
        .unwrap();
        write_message(
            &mut sw,
            &response(
                prompt_ids.remove("s2").unwrap(),
                json!({"stopReason":"end_turn"}),
            ),
        )
        .await
        .unwrap();
    });

    client.initialize().await.unwrap();
    let (s1_tx, mut s1_rx) = mpsc::channel(16);
    let (s2_tx, mut s2_rx) = mpsc::channel(16);
    let s1 = client
        .new_session_with_mcp_routed(
            PathBuf::from("/project"),
            Vec::new(),
            s1_tx,
            PermissionPolicy::Auto,
            None,
        )
        .await
        .unwrap();
    let s2 = client
        .new_session_with_mcp_routed(
            PathBuf::from("/project"),
            Vec::new(),
            s2_tx,
            PermissionPolicy::Auto,
            None,
        )
        .await
        .unwrap();

    // Run both prompts concurrently on the single AcpClient.
    let (r1, r2) = tokio::join!(client.prompt(&s1, "one"), client.prompt(&s2, "two"));
    assert_eq!(r1.unwrap(), Some("end_turn".into()));
    assert_eq!(r2.unwrap(), Some("end_turn".into()));

    let mut s1_events = Vec::new();
    while let Ok(ev) = s1_rx.try_recv() {
        s1_events.push(ev);
    }
    let mut s2_events = Vec::new();
    while let Ok(ev) = s2_rx.try_recv() {
        s2_events.push(ev);
    }
    assert!(
        s1_events
            .iter()
            .any(|e| matches!(e, AcpEvent::AgentMessageChunk(t) if t == "from-s1"))
    );
    assert!(
        !s1_events
            .iter()
            .any(|e| matches!(e, AcpEvent::AgentMessageChunk(t) if t == "from-s2"))
    );
    assert!(
        !s1_events
            .iter()
            .any(|e| matches!(e, AcpEvent::PermissionRequest(_)))
    );
    assert!(
        s2_events
            .iter()
            .any(|e| matches!(e, AcpEvent::AgentMessageChunk(t) if t == "from-s2"))
    );
    assert!(
        s2_events
            .iter()
            .any(|e| matches!(e, AcpEvent::PermissionRequest(_)))
    );
    assert!(default_rx.try_recv().is_err());

    server.await.unwrap();
}

#[tokio::test]
async fn close_delete_and_list_sessions_check_capabilities_and_schemas() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (events_tx, _events_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        events_tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() else {
            panic!("init expected");
        };
        write_message(
            &mut sw,
            &response(
                id,
                json!({
                    "protocolVersion": 1,
                    "agentCapabilities": {
                        "sessionCapabilities": {
                            "list": {},
                            "close": {},
                            "delete": {}
                        }
                    }
                }),
            ),
        )
        .await
        .unwrap();

        let JsonRpcMessage::Request { id, method, params } =
            read_next(&mut r).await.unwrap().unwrap()
        else {
            panic!("list expected");
        };
        assert_eq!(method, "session/list");
        assert!(params["cwd"].as_str().unwrap().contains("project"));
        write_message(
            &mut sw,
            &response(
                id,
                json!({
                    "sessions": [
                        {"sessionId": "s-old", "cwd": "/project", "title": "Old Chat", "updatedAt": "2026-11-26T00:00:00Z"}
                    ]
                }),
            ),
        )
        .await
        .unwrap();

        let JsonRpcMessage::Request { id, method, params } =
            read_next(&mut r).await.unwrap().unwrap()
        else {
            panic!("close expected");
        };
        assert_eq!(method, "session/close");
        assert_eq!(params, json!({"sessionId": "s-old"}));
        write_message(&mut sw, &response(id, json!({})))
            .await
            .unwrap();

        let JsonRpcMessage::Request { id, method, params } =
            read_next(&mut r).await.unwrap().unwrap()
        else {
            panic!("delete expected");
        };
        assert_eq!(method, "session/delete");
        assert_eq!(params, json!({"sessionId": "s-old"}));
        write_message(&mut sw, &response(id, json!({})))
            .await
            .unwrap();
    });

    let init = client.initialize().await.unwrap();
    assert!(init.supports_list_sessions());
    assert!(init.supports_close_session());
    assert!(init.supports_delete_session());

    let listed = client
        .list_sessions_for_cwd(Some(PathBuf::from("/project")))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].session_id, "s-old");
    assert_eq!(listed[0].title.as_deref(), Some("Old Chat"));

    client.close_session("s-old").await.unwrap();
    client.delete_session("s-old").await.unwrap();
    server.await.unwrap();
}

#[test]
fn connection_pool_reuses_live_process_and_evicts_crashed_process() {
    let script = r#"
import json, os, sys
counter = 0
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":1,"agentCapabilities":{"sessionCapabilities":{"close":{}}}}}), flush=True)
    elif method == "session/new":
        counter += 1
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"sessionId":f"s-{os.getpid()}-{counter}"}}), flush=True)
    elif method == "session/close":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{}}), flush=True)
    elif method == "session/prompt":
        prompt_text = msg["params"]["prompt"][0]["text"]
        if prompt_text == "crash":
            sys.exit(1)
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"stopReason":"end_turn"}}), flush=True)
"#;
    let command = teshi_acp::AcpAgentCommand {
        program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        args: vec!["-c".into(), script.into()],
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        env: Default::default(),
    };
    let key = teshi_acp::AcpConnectionKey::from_command("fake", &command, None);
    let pool = teshi_acp::AcpConnectionPool::new();

    pool.block_on(async {
        let conn1 = pool
            .acquire(
                key.clone(),
                command.clone(),
                test_config(),
                PermissionPolicy::Auto,
                None,
            )
            .await
            .unwrap();
        let conn2 = pool
            .acquire(
                key.clone(),
                command.clone(),
                test_config(),
                PermissionPolicy::Auto,
                None,
            )
            .await
            .unwrap();
        let pid1 = conn1.process_id().expect("pid");
        assert_eq!(conn2.process_id(), Some(pid1));
        assert_eq!(pool.active_connection_count(), 1);
        assert_eq!(pool.total_active_sessions(), 2);

        let (tx1, _rx1) = mpsc::channel(8);
        let (tx2, _rx2) = mpsc::channel(8);
        let s1 = conn1
            .client()
            .new_session_with_mcp_routed(
                command.cwd.clone(),
                Vec::new(),
                tx1,
                PermissionPolicy::Auto,
                None,
            )
            .await
            .unwrap();
        let s2 = conn2
            .client()
            .new_session_with_mcp_routed(
                command.cwd.clone(),
                Vec::new(),
                tx2,
                PermissionPolicy::Auto,
                None,
            )
            .await
            .unwrap();
        assert_ne!(s1.id, s2.id);

        conn1.client().close_session(&s1.id).await.unwrap();
        drop(conn1);
        assert_eq!(pool.total_active_sessions(), 1);

        // Crash the underlying process via s2 and verify pool evicts the dead entry.
        let err = conn2.client().prompt(&s2, "crash").await.unwrap_err();
        assert!(matches!(err, teshi_acp::AcpError::ProcessExited));
        drop(conn2);
        let evict_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while pool.active_connection_count() > 0 {
            assert!(
                tokio::time::Instant::now() < evict_deadline,
                "crashed pooled process should be evicted"
            );
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        assert_eq!(pool.active_connection_count(), 0);

        let conn3 = pool
            .acquire(
                key.clone(),
                command.clone(),
                test_config(),
                PermissionPolicy::Auto,
                None,
            )
            .await
            .unwrap();
        assert_ne!(conn3.process_id(), Some(pid1));
        drop(conn3);
        pool.shutdown_all().await;
        assert_eq!(pool.active_connection_count(), 0);
    });
}

#[test]
fn connection_pool_exclusive_mode_isolates_concurrent_leases_reuses_idle_and_prewarms() {
    let script = r#"
import json, os, sys
counter = 0
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":1,"agentCapabilities":{"sessionCapabilities":{"close":{}}}}}), flush=True)
    elif method == "session/new":
        counter += 1
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"sessionId":f"s-{os.getpid()}-{counter}"}}), flush=True)
    elif method == "session/close":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{}}), flush=True)
    elif method == "session/prompt":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"stopReason":"end_turn"}}), flush=True)
"#;
    let command = teshi_acp::AcpAgentCommand {
        program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        args: vec!["-c".into(), script.into()],
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        env: Default::default(),
    };
    let key = teshi_acp::AcpConnectionKey::from_command("exclusive-fake", &command, None);
    let mode = teshi_acp::AcpPoolMode::Exclusive {
        prewarm_standby: 1,
        max_idle: 2,
    };
    let pool = teshi_acp::AcpConnectionPool::new();

    pool.block_on(async {
        let conn1 = pool
            .acquire_with_mode(
                key.clone(),
                command.clone(),
                test_config(),
                PermissionPolicy::Auto,
                None,
                mode,
            )
            .await
            .unwrap();
        let pid1 = conn1.process_id().expect("pid1");

        // Wait for the background standby process to finish initializing.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while pool.idle_connection_count() < 1 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "standby prewarm should initialize an idle process"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(pool.active_connection_count(), 2);
        assert_eq!(pool.total_active_sessions(), 1);

        // Acquire three more concurrent sessions — all four must get distinct PIDs.
        let (c2, c3, c4) = tokio::join!(
            pool.acquire_with_mode(
                key.clone(),
                command.clone(),
                test_config(),
                PermissionPolicy::Auto,
                None,
                mode,
            ),
            pool.acquire_with_mode(
                key.clone(),
                command.clone(),
                test_config(),
                PermissionPolicy::Auto,
                None,
                mode,
            ),
            pool.acquire_with_mode(
                key.clone(),
                command.clone(),
                test_config(),
                PermissionPolicy::Auto,
                None,
                mode,
            ),
        );
        let conn2 = c2.unwrap();
        let conn3 = c3.unwrap();
        let conn4 = c4.unwrap();
        let pids: std::collections::HashSet<u32> = [
            pid1,
            conn2.process_id().expect("pid2"),
            conn3.process_id().expect("pid3"),
            conn4.process_id().expect("pid4"),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            pids.len(),
            4,
            "four concurrent exclusive leases must own four distinct processes"
        );
        assert_eq!(pool.total_active_sessions(), 4);

        // Run concurrent prompts across all four isolated connections.
        let run_turn = |client: std::sync::Arc<AcpClient<tokio::process::ChildStdin>>,
                        cwd: PathBuf| async move {
            let (tx, _rx) = mpsc::channel(8);
            let session = client
                .new_session_with_mcp_routed(cwd, Vec::new(), tx, PermissionPolicy::Auto, None)
                .await
                .unwrap();
            let completion = client.prompt(&session, "hello").await.unwrap();
            assert_eq!(completion.as_deref(), Some("end_turn"));
            client.close_session(&session.id).await.unwrap();
            client.unregister_session(&session.id).await;
        };
        tokio::join!(
            run_turn(std::sync::Arc::clone(conn1.client()), command.cwd.clone()),
            run_turn(std::sync::Arc::clone(conn2.client()), command.cwd.clone()),
            run_turn(std::sync::Arc::clone(conn3.client()), command.cwd.clone()),
            run_turn(std::sync::Arc::clone(conn4.client()), command.cwd.clone()),
        );

        // Drop all four leases; excess idle connections must be trimmed down to max_idle (2).
        drop(conn1);
        drop(conn2);
        drop(conn3);
        drop(conn4);
        let trim_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while pool.idle_connection_count() > 2 || pool.shutdown_pending() {
            assert!(
                tokio::time::Instant::now() < trim_deadline,
                "excess idle exclusive connections should be trimmed to max_idle"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let idle_before = pool.idle_connection_count();
        assert!((1..=2).contains(&idle_before));
        assert_eq!(pool.total_active_sessions(), 0);

        // Re-acquiring must reuse an existing idle connection rather than spawning a fresh PID.
        let conn5 = pool
            .acquire_with_mode(
                key.clone(),
                command.clone(),
                test_config(),
                PermissionPolicy::Auto,
                None,
                teshi_acp::AcpPoolMode::Exclusive {
                    prewarm_standby: 0,
                    max_idle: 2,
                },
            )
            .await
            .unwrap();
        assert!(conn5.is_alive());
        assert_eq!(pool.idle_connection_count(), idle_before - 1);
        assert!(
            pids.contains(&conn5.process_id().expect("pid5"))
                || pool.active_connection_count() <= 2,
            "re-acquire must check out an existing idle standby/cached process"
        );
        // Invalidating a lease removes it from the pool and shuts it down on drop.
        conn5.invalidate();
        drop(conn5);
        let retire_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while pool.shutdown_pending() {
            assert!(
                tokio::time::Instant::now() < retire_deadline,
                "invalidated lease shutdown should finish promptly"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(pool.idle_connection_count(), idle_before - 1);

        pool.shutdown_all().await;
        assert_eq!(pool.active_connection_count(), 0);
        assert!(!pool.shutdown_pending());
    });
}

#[tokio::test]
async fn slow_session_consumer_does_not_block_sibling_session_on_shared_client() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (server_read, mut server_write) = tokio::io::split(server_io);
    let (default_tx, _default_rx) = mpsc::channel(4);
    let client = Arc::new(AcpClient::from_io(
        client_read,
        client_write,
        test_config(),
        default_tx,
        PermissionPolicy::Auto,
        None,
    ));

    let server = tokio::spawn(async move {
        let mut reader = BufReader::new(server_read);
        let JsonRpcMessage::Request { id, .. } = read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected initialize request");
        };
        write_message(
            &mut server_write,
            &response(id, json!({ "protocolVersion": 1, "agentCapabilities": {} })),
        )
        .await
        .unwrap();
        for sid in ["s-slow", "s-fast"] {
            let JsonRpcMessage::Request { id, .. } = read_next(&mut reader).await.unwrap().unwrap()
            else {
                panic!("expected session/new request");
            };
            write_message(
                &mut server_write,
                &response(id, json!({ "sessionId": sid })),
            )
            .await
            .unwrap();
        }
        // Flood s-slow with more updates than its tiny bounded receiver (capacity 1)
        // AND more than inbound channel capacity (128), while s-slow never polls.
        let JsonRpcMessage::Request { id: fast_id, .. } =
            read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected session/prompt request");
        };
        for idx in 0..200 {
            write_message(
                &mut server_write,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": "s-slow",
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": format!("slow-{idx}") }
                        }
                    }
                }),
            )
            .await
            .unwrap();
        }
        write_message(
            &mut server_write,
            &json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": "s-fast",
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": "fast-ok" }
                    }
                }
            }),
        )
        .await
        .unwrap();
        write_message(
            &mut server_write,
            &response(fast_id, json!({ "stopReason": "end_turn" })),
        )
        .await
        .unwrap();
    });

    client.initialize_shared().await.unwrap();
    let (slow_tx, _slow_rx) = mpsc::channel(1);
    let _s_slow = client
        .new_session_with_mcp_routed(
            PathBuf::from("/w"),
            Vec::new(),
            slow_tx,
            PermissionPolicy::Auto,
            None,
        )
        .await
        .unwrap();
    let (fast_tx, mut fast_rx) = mpsc::channel(16);
    let s_fast = client
        .new_session_with_mcp_routed(
            PathBuf::from("/w"),
            Vec::new(),
            fast_tx,
            PermissionPolicy::Auto,
            None,
        )
        .await
        .unwrap();

    let stop = tokio::time::timeout(Duration::from_secs(2), client.prompt(&s_fast, "hi"))
        .await
        .expect("sibling session must not deadlock when another route stops draining")
        .unwrap();
    assert_eq!(stop.as_deref(), Some("end_turn"));

    let mut chunks = Vec::new();
    while let Ok(ev) = fast_rx.try_recv() {
        if let AcpEvent::AgentMessageChunk(t) = ev {
            chunks.push(t);
        }
    }
    assert_eq!(chunks, vec!["fast-ok"]);
    server.await.unwrap();
}

#[tokio::test]
async fn list_sessions_for_cwd_follows_next_cursor_pagination() {
    let (client_io, server_io) = tokio::io::duplex(16 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (server_read, mut server_write) = tokio::io::split(server_io);
    let (tx, _rx) = mpsc::channel(8);
    let mut client = AcpClient::from_io(
        client_read,
        client_write,
        test_config(),
        tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut reader = BufReader::new(server_read);
        let JsonRpcMessage::Request { id, .. } = read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected initialize request");
        };
        write_message(
            &mut server_write,
            &response(
                id,
                json!({
                    "protocolVersion": 1,
                    "agentCapabilities": { "sessionCapabilities": { "list": {} } }
                }),
            ),
        )
        .await
        .unwrap();
        let JsonRpcMessage::Request { id, params, .. } =
            read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected first session/list request");
        };
        assert!(params.get("cursor").is_none());
        write_message(
            &mut server_write,
            &response(
                id,
                json!({
                    "sessions": [{ "sessionId": "s-1", "cwd": "/w" }],
                    "nextCursor": "c2"
                }),
            ),
        )
        .await
        .unwrap();
        let JsonRpcMessage::Request { id, params, .. } =
            read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected second session/list request");
        };
        assert_eq!(params["cursor"], json!("c2"));
        write_message(
            &mut server_write,
            &response(
                id,
                json!({
                    "sessions": [{ "sessionId": "s-2", "cwd": "/w" }]
                }),
            ),
        )
        .await
        .unwrap();
    });

    client.initialize().await.unwrap();
    let sessions = client
        .list_sessions_for_cwd(Some(PathBuf::from("/w")))
        .await
        .unwrap();
    let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
    assert_eq!(ids, vec!["s-1", "s-2"]);
    server.await.unwrap();
}

#[test]
fn connection_key_debug_redacts_secret_env_values() {
    let mut env = std::collections::HashMap::new();
    env.insert("CURSOR_API_KEY".into(), "sk-super-secret-value-123".into());
    let cmd = teshi_acp::AcpAgentCommand {
        program: PathBuf::from("agent"),
        args: vec!["acp".into()],
        cwd: PathBuf::from("/w"),
        env,
    };
    let key = teshi_acp::AcpConnectionKey::from_command("cursor", &cmd, None);
    let formatted = format!("{key:?}");
    assert!(
        !formatted.contains("sk-super-secret-value-123"),
        "Debug output must redact secret env values: {formatted}"
    );
}

#[tokio::test]
async fn prompt_blocks_with_tiny_bounded_channel_does_not_deadlock() {
    let (client_io, server_io) = tokio::io::duplex(32 * 1024);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (default_tx, _default_rx) = mpsc::channel(4);
    let client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        default_tx,
        PermissionPolicy::Auto,
        None,
    );

    let server = tokio::spawn(async move {
        let mut r = BufReader::new(sr);
        let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() else {
            panic!("expected initialize");
        };
        write_message(
            &mut sw,
            &response(id, json!({ "protocolVersion": 1, "agentCapabilities": {} })),
        )
        .await
        .unwrap();

        let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() else {
            panic!("expected session/new");
        };
        write_message(&mut sw, &response(id, json!({ "sessionId": "s-bounded" })))
            .await
            .unwrap();

        let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap() else {
            panic!("expected session/prompt");
        };
        for idx in 0..10 {
            write_message(
                &mut sw,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": "s-bounded",
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": format!("chunk-{idx}") }
                        }
                    }
                }),
            )
            .await
            .unwrap();
        }
        write_message(&mut sw, &response(id, json!({ "stopReason": "end_turn" })))
            .await
            .unwrap();
    });

    client.initialize_shared().await.unwrap();
    // Capacity 1 route receiver that is intentionally NOT polled during prompt_blocks.
    let (route_tx, mut route_rx) = mpsc::channel(1);
    let session = client
        .new_session_with_mcp_routed(
            PathBuf::from("/w"),
            Vec::new(),
            route_tx,
            PermissionPolicy::Auto,
            None,
        )
        .await
        .unwrap();

    let completion = tokio::time::timeout(
        Duration::from_secs(2),
        client.prompt_blocks(&session, &[teshi_acp::PromptBlock::Text("hi".into())]),
    )
    .await
    .expect("prompt_blocks must not deadlock when bounded route channel is full")
    .unwrap();
    assert_eq!(completion.stop_reason.as_deref(), Some("end_turn"));

    assert!(route_rx.try_recv().is_ok());
    server.await.unwrap();
}

#[tokio::test]
async fn trailing_session_update_arrives_before_completed_event() {
    use tokio::io::AsyncWriteExt;

    for iter in 0..15 {
        let (client_io, server_io) = tokio::io::duplex(32 * 1024);
        let (cr, cw) = tokio::io::split(client_io);
        let (sr, mut sw) = tokio::io::split(server_io);
        let (default_tx, _default_rx) = mpsc::channel(8);
        let client = AcpClient::from_io(
            cr,
            cw,
            test_config(),
            default_tx,
            PermissionPolicy::Auto,
            None,
        );

        let server = tokio::spawn(async move {
            let mut r = BufReader::new(sr);
            let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap()
            else {
                panic!("expected initialize");
            };
            write_message(
                &mut sw,
                &response(id, json!({ "protocolVersion": 1, "agentCapabilities": {} })),
            )
            .await
            .unwrap();

            let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap()
            else {
                panic!("expected session/new");
            };
            write_message(&mut sw, &response(id, json!({ "sessionId": "s-order" })))
                .await
                .unwrap();

            let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap()
            else {
                panic!("expected session/prompt");
            };
            // Write trailing chunks and the prompt completion response in one contiguous write.
            let mut batch = String::new();
            for idx in 0..5 {
                let note = json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": "s-order",
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": format!("c{idx}") }
                        }
                    }
                });
                batch.push_str(&serde_json::to_string(&note).unwrap());
                batch.push('\n');
            }
            let resp = response(id, json!({ "stopReason": "end_turn" }));
            batch.push_str(&serde_json::to_string(&resp).unwrap());
            batch.push('\n');
            sw.write_all(batch.as_bytes()).await.unwrap();
            sw.flush().await.unwrap();
        });

        client.initialize_shared().await.unwrap();
        let (route_tx, mut route_rx) = mpsc::channel(32);
        let session = client
            .new_session_with_mcp_routed(
                PathBuf::from("/w"),
                Vec::new(),
                route_tx,
                PermissionPolicy::Auto,
                None,
            )
            .await
            .unwrap();

        let completion = client
            .prompt_blocks(&session, &[teshi_acp::PromptBlock::Text("go".into())])
            .await
            .unwrap();
        assert_eq!(completion.stop_reason.as_deref(), Some("end_turn"));

        let mut drained = Vec::new();
        while let Ok(ev) = route_rx.try_recv() {
            drained.push(ev);
        }
        assert!(
            matches!(drained.last(), Some(AcpEvent::Completed { .. })),
            "iter {iter}: Completed must be last event after trailing updates: {drained:?}"
        );
        let chunks: Vec<String> = drained
            .into_iter()
            .filter_map(|e| match e {
                AcpEvent::AgentMessageChunk(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(
            chunks,
            vec!["c0", "c1", "c2", "c3", "c4"],
            "iter {iter}: all trailing chunks must precede Completed"
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn authenticate_preserves_timeout_and_process_exit_errors() {
    // Timeout preserves AcpError::Timeout
    {
        let (client_io, server_io) = tokio::io::duplex(8192);
        let (cr, cw) = tokio::io::split(client_io);
        let (sr, mut sw) = tokio::io::split(server_io);
        let (tx, _rx) = mpsc::channel(8);
        let mut cfg = test_config();
        cfg.authentication_timeout = Duration::from_millis(40);
        let client = AcpClient::from_io(cr, cw, cfg, tx, PermissionPolicy::Auto, None);

        let server = tokio::spawn(async move {
            let mut r = BufReader::new(sr);
            let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap()
            else {
                panic!("expected initialize");
            };
            write_message(
                &mut sw,
                &response(
                    id,
                    json!({ "protocolVersion": 1, "authMethods": [{ "id": "oauth", "name": "OAuth" }] }),
                ),
            )
            .await
            .unwrap();
            let _auth_req = read_next(&mut r).await.unwrap().unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        client.initialize_shared().await.unwrap();
        let err = client.authenticate("oauth").await.unwrap_err();
        assert!(
            matches!(err, teshi_acp::AcpError::Timeout(_)),
            "expected Timeout, got {err:?}"
        );
        server.await.unwrap();
    }

    // Process exit preserves AcpError::ProcessExited
    {
        let (client_io, server_io) = tokio::io::duplex(8192);
        let (cr, cw) = tokio::io::split(client_io);
        let (sr, mut sw) = tokio::io::split(server_io);
        let (tx, _rx) = mpsc::channel(8);
        let client = AcpClient::from_io(cr, cw, test_config(), tx, PermissionPolicy::Auto, None);

        let server = tokio::spawn(async move {
            let mut r = BufReader::new(sr);
            let JsonRpcMessage::Request { id, .. } = read_next(&mut r).await.unwrap().unwrap()
            else {
                panic!("expected initialize");
            };
            write_message(
                &mut sw,
                &response(
                    id,
                    json!({ "protocolVersion": 1, "authMethods": [{ "id": "oauth", "name": "OAuth" }] }),
                ),
            )
            .await
            .unwrap();
            let _auth_req = read_next(&mut r).await.unwrap().unwrap();
            drop(sw);
            drop(r);
        });

        client.initialize_shared().await.unwrap();
        let err = client.authenticate("oauth").await.unwrap_err();
        assert!(
            matches!(err, teshi_acp::AcpError::ProcessExited),
            "expected ProcessExited, got {err:?}"
        );
        server.await.unwrap();
    }
}

#[test]
fn get_live_and_find_live_skip_invalidated_dirty_and_active_exclusive() {
    let script = r#"
import json, os, sys
counter = 0
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":1,"agentCapabilities":{"sessionCapabilities":{"close":{},"list":{}}}}}), flush=True)
    elif method == "session/new":
        counter += 1
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"sessionId":f"s-{os.getpid()}-{counter}"}}), flush=True)
    elif method == "session/close":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{}}), flush=True)
    elif method == "session/list":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"sessions":[]}}), flush=True)
"#;
    let command = teshi_acp::AcpAgentCommand {
        program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        args: vec!["-c".into(), script.into()],
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        env: Default::default(),
    };
    let excl_key = teshi_acp::AcpConnectionKey::from_command("excl-lookup", &command, None);
    let shared_key = teshi_acp::AcpConnectionKey::from_command("shared-lookup", &command, None);
    let pool = teshi_acp::AcpConnectionPool::new();

    pool.block_on(async {
        let excl_mode = teshi_acp::AcpPoolMode::Exclusive {
            prewarm_standby: 0,
            max_idle: 2,
        };
        let conn1 = pool
            .acquire_with_mode(
                excl_key.clone(),
                command.clone(),
                test_config(),
                PermissionPolicy::Auto,
                None,
                excl_mode,
            )
            .await
            .unwrap();
        let pid1 = conn1.process_id().unwrap();
        // While conn1 is checked out and active, neither lookup may borrow it.
        assert!(pool.get_live(&excl_key).is_none());
        assert!(
            pool.find_live_by_cwd_and_provider(&command.cwd, "excl-lookup")
                .is_none()
        );

        // Open and keep a session, then drop without closing -> unclean exclusive must be retired.
        let (tx, _rx) = mpsc::channel(4);
        let _s1 = conn1
            .client()
            .new_session_with_mcp_routed(
                command.cwd.clone(),
                Vec::new(),
                tx,
                PermissionPolicy::Auto,
                None,
            )
            .await
            .unwrap();
        drop(conn1);
        let retire_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while pool.active_connection_count() > 0 || pool.shutdown_pending() {
            assert!(tokio::time::Instant::now() < retire_deadline);
            tokio::time::sleep(Duration::from_millis(15)).await;
        }

        // Acquire clean connection, open/close/unregister session, then drop so it stays idle clean.
        let conn2 = pool
            .acquire_with_mode(
                excl_key.clone(),
                command.clone(),
                test_config(),
                PermissionPolicy::Auto,
                None,
                excl_mode,
            )
            .await
            .unwrap();
        assert_ne!(conn2.process_id().unwrap(), pid1);
        let (tx2, _rx2) = mpsc::channel(4);
        let s2 = conn2
            .client()
            .new_session_with_mcp_routed(
                command.cwd.clone(),
                Vec::new(),
                tx2,
                PermissionPolicy::Auto,
                None,
            )
            .await
            .unwrap();
        conn2.client().close_session(&s2.id).await.unwrap();
        conn2.client().unregister_session(&s2.id).await;
        drop(conn2);
        assert_eq!(pool.idle_connection_count(), 1);

        // Now find_live_by_cwd_and_provider checks out the idle clean exclusive connection.
        let live = pool
            .find_live_by_cwd_and_provider(&command.cwd, "excl-lookup")
            .expect("idle clean exclusive connection should be discoverable");
        assert!(pool.get_live(&excl_key).is_none());
        drop(live);

        // Shared connection: invalidate() immediately hides it from get_live & find_live_by_cwd_and_provider.
        let shared = pool
            .acquire_with_mode(
                shared_key.clone(),
                command.clone(),
                test_config(),
                PermissionPolicy::Auto,
                None,
                teshi_acp::AcpPoolMode::Shared,
            )
            .await
            .unwrap();
        assert!(pool.get_live(&shared_key).is_some());
        assert!(
            pool.find_live_by_cwd_and_provider(&command.cwd, "shared-lookup")
                .is_some()
        );
        shared.invalidate();
        assert!(pool.get_live(&shared_key).is_none());
        assert!(
            pool.find_live_by_cwd_and_provider(&command.cwd, "shared-lookup")
                .is_none()
        );
        drop(shared);

        pool.shutdown_all().await;
        assert_eq!(pool.active_connection_count(), 0);
    });
}

#[test]
fn exclusive_drop_under_reacquire_contention_never_retires_active_checkout() {
    let script = r#"
import json, os, sys
counter = 0
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":1,"agentCapabilities":{"sessionCapabilities":{"close":{}}}}}), flush=True)
    elif method == "session/new":
        counter += 1
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"sessionId":f"s-{os.getpid()}-{counter}"}}), flush=True)
    elif method == "session/close":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{}}), flush=True)
"#;
    let command = teshi_acp::AcpAgentCommand {
        program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        args: vec!["-c".into(), script.into()],
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        env: Default::default(),
    };
    let key = teshi_acp::AcpConnectionKey::from_command("drop-race", &command, None);
    let mode = teshi_acp::AcpPoolMode::Exclusive {
        prewarm_standby: 0,
        max_idle: 1,
    };
    let pool = teshi_acp::AcpConnectionPool::new();

    pool.block_on(async {
        for _ in 0..8 {
            let conn = pool
                .acquire_with_mode(
                    key.clone(),
                    command.clone(),
                    test_config(),
                    PermissionPolicy::Auto,
                    None,
                    mode,
                )
                .await
                .unwrap();
            let (tx, _rx) = mpsc::channel(4);
            let sess = conn
                .client()
                .new_session_with_mcp_routed(
                    command.cwd.clone(),
                    Vec::new(),
                    tx,
                    PermissionPolicy::Auto,
                    None,
                )
                .await
                .unwrap();
            conn.client().close_session(&sess.id).await.unwrap();
            conn.client().unregister_session(&sess.id).await;

            drop(conn);
            let next = pool
                .acquire_with_mode(
                    key.clone(),
                    command.clone(),
                    test_config(),
                    PermissionPolicy::Auto,
                    None,
                    mode,
                )
                .await
                .unwrap();
            let (tx2, _rx2) = mpsc::channel(4);
            let s2 = next
                .client()
                .new_session_with_mcp_routed(
                    command.cwd.clone(),
                    Vec::new(),
                    tx2,
                    PermissionPolicy::Auto,
                    None,
                )
                .await
                .unwrap();
            assert!(next.is_alive());
            next.client().close_session(&s2.id).await.unwrap();
            next.client().unregister_session(&s2.id).await;
            drop(next);
        }
        pool.shutdown_all().await;
    });
}

#[test]
fn shared_acquire_racing_with_shutdown_all_does_not_leak_processes() {
    let script = r#"
import json, sys, time
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        time.sleep(0.08)
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":1,"agentCapabilities":{}}}), flush=True)
"#;
    let command = teshi_acp::AcpAgentCommand {
        program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        args: vec!["-c".into(), script.into()],
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        env: Default::default(),
    };
    let key = teshi_acp::AcpConnectionKey::from_command("shutdown-race", &command, None);
    let pool = teshi_acp::AcpConnectionPool::new();

    pool.block_on(async {
        let mut tasks = Vec::new();
        for _ in 0..3 {
            let p = pool.clone();
            let k = key.clone();
            let c = command.clone();
            tasks.push(tokio::spawn(async move {
                p.acquire_with_mode(
                    k,
                    c,
                    test_config(),
                    PermissionPolicy::Auto,
                    None,
                    teshi_acp::AcpPoolMode::Shared,
                )
                .await
            }));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        pool.shutdown_all().await;
        for t in tasks {
            let res = t.await.unwrap();
            assert!(res.is_err(), "queued acquire must abort on shutdown_all");
        }
        assert_eq!(pool.active_connection_count(), 0);
        assert!(!pool.shutdown_pending());
    });
}

#[tokio::test]
async fn transport_crash_cancels_pending_manual_permission_and_exits_dispatcher() {
    struct ParkingPermissionHost {
        entered_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        cancelled_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    }

    struct CancelDropGuard(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for CancelDropGuard {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

    impl PermissionHost for ParkingPermissionHost {
        fn request_permission<'a>(
            &'a self,
            _options: &'a [PermissionOption],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = teshi_acp::AcpResult<PermissionDecision>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                let cancel_tx = self.cancelled_tx.lock().unwrap().take();
                let _guard = CancelDropGuard(cancel_tx);
                if let Some(tx) = self.entered_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
                std::future::pending::<teshi_acp::AcpResult<PermissionDecision>>().await
            })
        }
    }

    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (cancelled_tx, cancelled_rx) = tokio::sync::oneshot::channel();
    let host = Arc::new(ParkingPermissionHost {
        entered_tx: Mutex::new(Some(entered_tx)),
        cancelled_tx: Mutex::new(Some(cancelled_tx)),
    });

    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (tx, _rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        tx,
        PermissionPolicy::Manual,
        Some(host),
    );

    let (drop_server_tx, drop_server_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut reader = BufReader::new(sr);
        let JsonRpcMessage::Request { id, method, .. } =
            read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected initialize");
        };
        assert_eq!(method, "initialize");
        write_message(&mut sw, &response(id, json!({"protocolVersion": 1})))
            .await
            .unwrap();
        let JsonRpcMessage::Request { id, method, .. } =
            read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected session/new");
        };
        assert_eq!(method, "session/new");
        write_message(&mut sw, &response(id, json!({"sessionId": "s1"})))
            .await
            .unwrap();
        // Send a manual permission request, wait for host entry, then crash (drop server write half).
        write_message(
            &mut sw,
            &request(
                teshi_acp::jsonrpc::JsonRpcId::Number(99),
                "session/request_permission",
                json!({
                    "sessionId": "s1",
                    "options": [{ "optionId": "allow", "name": "Allow", "kind": "allow_once" }]
                }),
            ),
        )
        .await
        .unwrap();
        let _ = drop_server_rx.await;
        drop(sw);
        drop(reader);
    });

    client.initialize().await.unwrap();
    let _session = client.new_session(PathBuf::from(".")).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), entered_rx)
        .await
        .expect("permission host must receive manual request")
        .unwrap();

    // Simulate child crash / stdout EOF.
    let _ = drop_server_tx.send(());
    server.await.unwrap();

    tokio::time::timeout(Duration::from_secs(1), cancelled_rx)
        .await
        .expect("pending manual permission must be cancelled when transport crashes")
        .unwrap();

    // Subsequent flush must return promptly instead of waiting for the 2s fence timeout.
    let started = std::time::Instant::now();
    client.flush_session_events(None).await;
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "flush_session_events stalled after transport crash: {:?}",
        started.elapsed()
    );
}

#[test]
fn dead_shared_connection_triggers_async_shutdown_on_final_lease_drop_and_idle_exit() {
    let script = r#"
import json, sys
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":1,"agentCapabilities":{}}}), flush=True)
    elif method == "session/new":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"sessionId":"s1"}}), flush=True)
    elif method == "session/prompt":
        sys.exit(0)
"#;
    let command = teshi_acp::AcpAgentCommand {
        program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        args: vec!["-c".into(), script.into()],
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        env: Default::default(),
    };
    let config = AcpClientConfig {
        initialize_timeout: Duration::from_secs(5),
        authentication_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(5),
        shutdown_timeout: Duration::from_secs(2),
        ..Default::default()
    };
    let key = teshi_acp::AcpConnectionKey::from_command("shared-crash", &command, None);
    let pool = teshi_acp::AcpConnectionPool::new();

    pool.block_on(async {
        // Case A: Shared connection dies while a lease is held (without explicit invalidate()).
        let lease = pool
            .acquire_with_mode(
                key.clone(),
                command.clone(),
                config.clone(),
                PermissionPolicy::Auto,
                None,
                teshi_acp::AcpPoolMode::Shared,
            )
            .await
            .unwrap();
        let client = Arc::clone(lease.client());
        let (tx, _rx) = mpsc::channel(8);
        let session = client
            .new_session_with_mcp_routed(
                command.cwd.clone(),
                Vec::new(),
                tx,
                PermissionPolicy::Auto,
                None,
            )
            .await
            .unwrap();
        let _ = client.prompt(&session, "crash").await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while client.is_alive() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!client.is_alive());
        drop(lease);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while client.lifecycle().await != teshi_acp::client::AcpLifecycle::Exited
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            client.lifecycle().await,
            teshi_acp::client::AcpLifecycle::Exited,
            "dropping final lease of a dead Shared connection must run shutdown_shared()"
        );

        // Case B: Idle pooled connection exits and exit watcher schedules shutdown_shared().
        let idle_script = r#"
import json, sys, time
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":1,"agentCapabilities":{}}}), flush=True)
        time.sleep(0.15)
        sys.exit(0)
"#;
        let idle_cmd = teshi_acp::AcpAgentCommand {
            program: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
            args: vec!["-c".into(), idle_script.into()],
            cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            env: Default::default(),
        };
        let idle_key =
            teshi_acp::AcpConnectionKey::from_command("idle-exit-watcher", &idle_cmd, None);
        let idle_lease = pool
            .acquire_with_mode(
                idle_key,
                idle_cmd,
                config,
                PermissionPolicy::Auto,
                None,
                teshi_acp::AcpPoolMode::Shared,
            )
            .await
            .unwrap();
        let idle_client = Arc::clone(idle_lease.client());
        // Drop while still alive so entry stays idle in the pool until child exits.
        drop(idle_lease);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while idle_client.lifecycle().await != teshi_acp::client::AcpLifecycle::Exited
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            idle_client.lifecycle().await,
            teshi_acp::client::AcpLifecycle::Exited,
            "exit watcher must schedule shutdown_shared() for idle crashed entry"
        );
        pool.shutdown_all().await;
    });
}

#[tokio::test]
async fn routed_client_cancels_late_permission_after_last_session_unregisters() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (default_tx, _default_rx) = mpsc::channel(16);
    // Default route is Auto: if late permission falls back to default_route, it would auto-approve!
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        default_tx,
        PermissionPolicy::Auto,
        None,
    );

    let (send_late_tx, send_late_rx) = tokio::sync::oneshot::channel::<()>();
    let (perm_outcome_tx, perm_outcome_rx) = tokio::sync::oneshot::channel::<serde_json::Value>();
    let server = tokio::spawn(async move {
        let mut reader = BufReader::new(sr);
        let JsonRpcMessage::Request { id, .. } = read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected initialize");
        };
        write_message(&mut sw, &response(id, json!({"protocolVersion": 1})))
            .await
            .unwrap();
        let JsonRpcMessage::Request { id, .. } = read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected session/new");
        };
        write_message(&mut sw, &response(id, json!({"sessionId": "s1"})))
            .await
            .unwrap();
        send_late_rx.await.unwrap();
        write_message(
            &mut sw,
            &request(
                teshi_acp::jsonrpc::JsonRpcId::Number(77),
                "session/request_permission",
                json!({
                    "sessionId": "s1",
                    "options": [{ "optionId": "allow", "name": "Allow", "kind": "allow_once" }]
                }),
            ),
        )
        .await
        .unwrap();
        let JsonRpcMessage::Response { result, .. } =
            read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected permission response");
        };
        let _ = perm_outcome_tx.send(result);
    });

    client.initialize().await.unwrap();
    let (s1_tx, _s1_rx) = mpsc::channel(16);
    let s1 = client
        .new_session_with_mcp_routed(
            PathBuf::from("."),
            Vec::new(),
            s1_tx,
            PermissionPolicy::Auto,
            None,
        )
        .await
        .unwrap();
    client.unregister_session(&s1.id).await;

    let _ = send_late_tx.send(());
    let outcome = tokio::time::timeout(Duration::from_secs(1), perm_outcome_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        outcome.pointer("/outcome/outcome").and_then(|v| v.as_str()),
        Some("cancelled"),
        "late permission for unregistered session must be cancelled, not auto-approved via default_route: {outcome}"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn routed_client_does_not_misroute_unregistered_session_to_pending_new_session() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (cr, cw) = tokio::io::split(client_io);
    let (sr, mut sw) = tokio::io::split(server_io);
    let (default_tx, _default_rx) = mpsc::channel(16);
    let mut client = AcpClient::from_io(
        cr,
        cw,
        test_config(),
        default_tx,
        PermissionPolicy::Auto,
        None,
    );

    let (perm_outcome_tx, perm_outcome_rx) = tokio::sync::oneshot::channel::<serde_json::Value>();
    let server = tokio::spawn(async move {
        let mut reader = BufReader::new(sr);
        let JsonRpcMessage::Request { id, .. } = read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected initialize");
        };
        write_message(&mut sw, &response(id, json!({"protocolVersion": 1})))
            .await
            .unwrap();
        // First session: s1.
        let JsonRpcMessage::Request { id, .. } = read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected session/new 1");
        };
        write_message(&mut sw, &response(id, json!({"sessionId": "s1"})))
            .await
            .unwrap();
        // Second session: s2 (hold response while sending late s1 traffic).
        let JsonRpcMessage::Request { id: s2_req_id, .. } =
            read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected session/new 2");
        };
        // Late notification + late permission request for already-unregistered s1 while s2 is pending.
        write_message(
            &mut sw,
            &json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": "s1",
                    "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "leaked-from-s1" } }
                }
            }),
        )
        .await
        .unwrap();
        write_message(
            &mut sw,
            &request(
                teshi_acp::jsonrpc::JsonRpcId::Number(88),
                "session/request_permission",
                json!({
                    "sessionId": "s1",
                    "options": [{ "optionId": "allow", "name": "Allow", "kind": "allow_once" }]
                }),
            ),
        )
        .await
        .unwrap();
        let JsonRpcMessage::Response { result, .. } =
            read_next(&mut reader).await.unwrap().unwrap()
        else {
            panic!("expected cancelled permission response for s1");
        };
        let _ = perm_outcome_tx.send(result);
        write_message(&mut sw, &response(s2_req_id, json!({"sessionId": "s2"})))
            .await
            .unwrap();
    });

    client.initialize().await.unwrap();
    let (s1_tx, _s1_rx) = mpsc::channel(16);
    let s1 = client
        .new_session_with_mcp_routed(
            PathBuf::from("."),
            Vec::new(),
            s1_tx,
            PermissionPolicy::Auto,
            None,
        )
        .await
        .unwrap();
    client.unregister_session(&s1.id).await;

    let (s2_tx, mut s2_rx) = mpsc::channel(16);
    let s2 = client
        .new_session_with_mcp_routed(
            PathBuf::from("."),
            Vec::new(),
            s2_tx,
            PermissionPolicy::Auto,
            None,
        )
        .await
        .unwrap();
    assert_eq!(s2.id, "s2");
    client.flush_session_events(Some(&s2.id)).await;

    let outcome = perm_outcome_rx.await.unwrap();
    assert_eq!(
        outcome.pointer("/outcome/outcome").and_then(|v| v.as_str()),
        Some("cancelled"),
        "late permission for s1 must not route to pending s2: {outcome}"
    );
    if let Ok(ev) = s2_rx.try_recv() {
        panic!("s2 must not receive late s1 event: {ev:?}");
    }
    server.await.unwrap();
}
