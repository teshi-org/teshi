use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::{Value, json};
use teshi_browser_broker::{BrokerEvent, BrokerRuntime, BrokerServerConfig};
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
