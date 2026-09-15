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
        matches!(&seen[4], AcpEvent::Completed { stop_reason } if stop_reason.as_deref() == Some("end_turn"))
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
    });

    client.initialize().await.unwrap();
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
