use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::{Value, json};
use teshi_browser_broker::protocol::{BrokerError, BrokerErrorCode, OperationRequest};
use teshi_browser_broker::{BrokerEvent, BrokerRuntime, BrokerServerConfig, BrokerState};
use tokio::sync::{Semaphore, oneshot};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message};

const EXTENSION_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

type TestSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn test_config() -> BrokerServerConfig {
    let mut config = BrokerServerConfig::new(format!("chrome-extension://{EXTENSION_ID}"));
    config.discovery_addr = "127.0.0.1:0".parse().expect("loopback test address");
    config
}

fn client_ws_url(runtime: &BrokerRuntime) -> String {
    format!(
        "{}?token={}",
        runtime.endpoint_record().ws_url,
        runtime.credential()
    )
}

fn extension_ws_url(runtime: &BrokerRuntime, token: &str) -> String {
    format!(
        "{}?token={token}",
        runtime.endpoint_record().extension_frame_ws_url
    )
}

async fn read_json(socket: &mut TestSocket) -> Value {
    loop {
        let message = socket
            .next()
            .await
            .expect("broker closed before sending the response")
            .expect("broker WebSocket read failed");
        if let Message::Text(text) = message {
            return serde_json::from_str(text.as_str()).expect("broker response is JSON");
        }
    }
}

async fn run_operation(
    state: &mut BrokerState,
    runtime: &BrokerRuntime,
    request: OperationRequest,
) -> Result<Value, BrokerError> {
    let (reply, receiver) = oneshot::channel();
    let budget = Arc::new(Semaphore::new(1))
        .acquire_owned()
        .await
        .expect("operation budget");
    state
        .handle(
            BrokerEvent::Operation {
                request,
                reply,
                _budget: budget,
            },
            runtime,
        )
        .await;
    receiver.await.expect("operation reply")
}

#[tokio::test]
async fn hostile_origins_and_stale_tokens_cannot_cross_the_public_boundary() {
    let mut runtime = BrokerRuntime::start(test_config())
        .await
        .expect("start test broker");
    let endpoint = runtime.endpoint_record();
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .expect("build HTTP client");

    let hostile_discovery = client
        .get(&endpoint.discovery_url)
        .header("Origin", "https://attacker.example")
        .send()
        .await
        .expect("send hostile discovery request");
    assert_eq!(hostile_discovery.status(), StatusCode::FORBIDDEN);
    let hostile_body = hostile_discovery
        .text()
        .await
        .expect("read hostile response");
    assert!(!hostile_body.contains(runtime.credential()));

    let trusted_origin = format!("chrome-extension://{EXTENSION_ID}");
    let missing_origin = client
        .post(&endpoint.discovery_url)
        .json(&json!({}))
        .send()
        .await
        .expect("send missing-origin discovery request");
    assert_eq!(missing_origin.status(), StatusCode::FORBIDDEN);

    for (origin, token) in [
        ("https://attacker.example", runtime.credential().to_owned()),
        (
            trusted_origin.as_str(),
            "stale-token-that-must-never-authenticate".to_owned(),
        ),
    ] {
        let mut request = extension_ws_url(&runtime, &token)
            .into_client_request()
            .expect("build extension WebSocket request");
        request.headers_mut().insert(
            ORIGIN,
            HeaderValue::from_str(origin).expect("valid test Origin header"),
        );
        let error = connect_async(request)
            .await
            .expect_err("negative handshake accepted");
        let WebSocketError::Http(response) = error else {
            panic!("expected HTTP handshake rejection");
        };
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    assert!(
        tokio::time::timeout(Duration::from_millis(100), runtime.next_event())
            .await
            .is_err(),
        "rejected handshakes must not enqueue broker events"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn unknown_operations_and_malformed_targets_fail_before_dispatch() {
    let mut runtime = BrokerRuntime::start(test_config())
        .await
        .expect("start test broker");
    let (mut socket, _) = connect_async(client_ws_url(&runtime))
        .await
        .expect("connect authenticated client WebSocket");

    socket
        .send(Message::Text(
            json!({
                "schema_version": 1,
                "request_id": "unknown-operation",
                "cmd": "delete_all_files"
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send unknown operation");
    let unknown = read_json(&mut socket).await;
    assert_eq!(unknown["code"], "invalid_browser_operation");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), runtime.next_event())
            .await
            .is_err(),
        "unknown operations must not reach the state owner"
    );

    socket
        .send(Message::Text(
            json!({
                "schema_version": 1,
                "request_id": "malformed-target",
                "cmd": "get_page_snapshot",
                "target": {
                    "extension_instance_id": "profile-a",
                    "window_id": 2,
                    "tab_id": 3,
                    "unexpected": "must-be-rejected"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send malformed target");
    let malformed = read_json(&mut socket).await;
    assert_eq!(malformed["code"], "broker_protocol_error");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), runtime.next_event())
            .await
            .is_err(),
        "malformed targets must not reach the state owner"
    );

    socket.close(None).await.expect("close client WebSocket");
    runtime.shutdown().await;
}

#[tokio::test]
async fn configured_websocket_limit_closes_oversized_extension_messages() {
    let mut config = test_config();
    config.max_websocket_message_bytes = 1024;
    let mut runtime = BrokerRuntime::start(config)
        .await
        .expect("start test broker");
    let origin = format!("chrome-extension://{EXTENSION_ID}");
    let mut request = extension_ws_url(&runtime, runtime.credential())
        .into_client_request()
        .expect("build extension WebSocket request");
    request.headers_mut().insert(
        ORIGIN,
        HeaderValue::from_str(&origin).expect("valid test Origin header"),
    );
    let (mut socket, _) = connect_async(request)
        .await
        .expect("connect extension WebSocket");
    socket
        .send(Message::Text(
            json!({
                "type": "stream_hello",
                "protocol_version": 1,
                "extension_instance_id": "profile-a",
                "project_root": "C:/project-a",
                "extension_version": "test"
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send extension hello");
    let BrokerEvent::ExtensionConnected { reply, .. } =
        runtime.next_event().await.expect("extension hello event")
    else {
        panic!("expected extension connection event");
    };
    reply
        .send(json!({
            "type": "stream_hello_ack",
            "schema_version": 1,
            "protocol_version": 1,
            "ok": true
        }))
        .expect("send extension hello acknowledgement");
    assert_eq!(read_json(&mut socket).await["type"], "stream_hello_ack");

    socket
        .send(Message::Text("x".repeat(1025).into()))
        .await
        .expect("send oversized extension message");
    let closed = tokio::time::timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("oversized message was not terminated promptly");
    assert!(
        matches!(closed, None | Some(Err(_)) | Some(Ok(Message::Close(_)))),
        "oversized message must close the extension stream"
    );
    if let Ok(Some(BrokerEvent::Operation { .. })) =
        tokio::time::timeout(Duration::from_millis(100), runtime.next_event()).await
    {
        panic!("oversized messages must not enqueue a browser operation");
    }
    runtime.shutdown().await;
}

#[tokio::test]
async fn protocol_v0_cannot_attach_a_stream_or_bypass_current_authorization() {
    let mut config = test_config();
    config.broker_features = vec!["p0.control".into(), "p2.cookies".into()];
    let mut runtime = BrokerRuntime::start(config)
        .await
        .expect("start test broker");

    let mut stream_request = extension_ws_url(&runtime, runtime.credential())
        .into_client_request()
        .expect("build extension WebSocket request");
    stream_request.headers_mut().insert(
        ORIGIN,
        HeaderValue::from_str(&format!("chrome-extension://{EXTENSION_ID}"))
            .expect("valid test Origin header"),
    );
    let (mut stream, _) = connect_async(stream_request)
        .await
        .expect("connect extension WebSocket");
    stream
        .send(Message::Text(
            json!({
                "type": "stream_hello",
                "protocol_version": 0,
                "extension_instance_id": "legacy-single-session",
                "extension_version": "legacy"
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send protocol-v0 hello");
    let rejected = read_json(&mut stream).await;
    assert_eq!(rejected["code"], "incompatible_browser_session");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), runtime.next_event())
            .await
            .is_err(),
        "protocol-v0 stream hello must not reach the state owner"
    );

    let mut state = BrokerState::new();
    let legacy_id = state
        .sessions
        .register_heartbeat(
            serde_json::from_value(json!({
                "windows": [{
                    "id": 7,
                    "focused": true,
                    "tabs": [{
                        "id": 42,
                        "window_id": 7,
                        "url": "https://legacy.example.test/",
                        "active": true,
                        "debuggable": true
                    }]
                }],
                "active_window_id": 7,
                "active_tab_id": 42,
                "features": [
                    {"feature": "p0.control", "available": true},
                    {"feature": "p2.cookies", "available": true}
                ],
                "supported_operations": ["list_browser_cookies"],
                "optional_permissions": {"cookies": true}
            }))
            .expect("legacy heartbeat payload"),
            Instant::now(),
        )
        .expect("register protocol-v0 heartbeat");
    assert_eq!(legacy_id, "legacy-single-session");

    let target = json!({
        "extension_instance_id": "legacy-single-session",
        "window_id": 7,
        "tab_id": 42
    });
    let missing_lease: OperationRequest = serde_json::from_value(json!({
        "schema_version": 1,
        "request_id": "legacy-p0-without-lease",
        "caller_label": "legacy-caller",
        "project_root": "C:/legacy-project",
        "cmd": "get_page_snapshot",
        "target": target
    }))
    .expect("legacy P0 request");
    let error = run_operation(&mut state, &runtime, missing_lease)
        .await
        .expect_err("protocol-v0 P0 request bypassed the lease gate");
    assert_eq!(error.code, BrokerErrorCode::InvalidBrowserLease);

    let acquired: OperationRequest = serde_json::from_value(json!({
        "schema_version": 1,
        "request_id": "legacy-lease",
        "caller_label": "legacy-caller",
        "project_root": "C:/legacy-project",
        "cmd": "acquire_browser_lease",
        "extension_instance_id": "legacy-single-session",
        "owner_label": "legacy-caller"
    }))
    .expect("legacy lease request");
    let lease = run_operation(&mut state, &runtime, acquired)
        .await
        .expect("legacy lease acquisition")["lease_token"]
        .as_str()
        .expect("lease token")
        .to_owned();

    let missing_grant: OperationRequest = serde_json::from_value(json!({
        "schema_version": 1,
        "request_id": "legacy-p2-without-grant",
        "caller_label": "legacy-caller",
        "project_root": "C:/legacy-project",
        "cmd": "list_browser_cookies",
        "target": target,
        "lease_token": lease
    }))
    .expect("legacy P2 request");
    let error = run_operation(&mut state, &runtime, missing_grant)
        .await
        .expect_err("protocol-v0 P2 request bypassed the capability gate");
    assert_eq!(error.code, BrokerErrorCode::BrowserCapabilityDenied);

    runtime.shutdown().await;
}

#[tokio::test]
async fn state_owner_rechecks_operation_envelope_before_dispatch() {
    let runtime = BrokerRuntime::start(test_config())
        .await
        .expect("start test broker");
    let mut state = BrokerState::new();

    let unknown: OperationRequest = serde_json::from_value(json!({
        "schema_version": 1,
        "request_id": "state-unknown-operation",
        "cmd": "delete_all_files"
    }))
    .expect("unknown operation envelope");
    let error = run_operation(&mut state, &runtime, unknown)
        .await
        .expect_err("state owner accepted an unknown operation");
    assert_eq!(error.code, BrokerErrorCode::InvalidBrowserOperation);

    let incompatible: OperationRequest = serde_json::from_value(json!({
        "schema_version": 2,
        "request_id": "state-incompatible-schema",
        "cmd": "list_browser_sessions"
    }))
    .expect("incompatible operation envelope");
    let error = run_operation(&mut state, &runtime, incompatible)
        .await
        .expect_err("state owner accepted an incompatible schema");
    assert_eq!(error.code, BrokerErrorCode::IncompatibleBrowserSession);

    assert!(state.sessions.list_public(Instant::now()).is_empty());
    runtime.shutdown().await;
}
