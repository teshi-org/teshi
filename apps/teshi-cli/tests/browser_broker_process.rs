use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use teshi_browser_broker::protocol::ExtensionStreamMessage;
use teshi_browser_broker::{
    BROWSER_BROKER_PROTOCOL_VERSION, DiscoveryResponse, EndpointRecord, PrivateCredentialStore,
};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::http::header::ORIGIN;
use tungstenite::{Message, connect};

const TRUSTED_ORIGIN: &str = "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SECOND_TRUSTED_ORIGIN: &str = "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const UNTRUSTED_ORIGIN: &str = "chrome-extension://bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbc";

struct BrokerChild(Child);

impl Drop for BrokerChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn unused_loopback_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

fn spawn_internal_broker(state_dir: &Path, port: u16) -> (BrokerChild, EndpointRecord) {
    let mut child = BrokerChild(
        Command::new(env!("CARGO_BIN_EXE_teshi"))
            .args([
                "--browser-broker-internal",
                "--state-dir",
                state_dir.to_str().unwrap(),
                "--trusted-extension-origin",
                TRUSTED_ORIGIN,
                "--discovery-port",
                &port.to_string(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the Teshi Rust broker process"),
    );
    let mut ready_line = String::new();
    BufReader::new(child.0.stdout.take().unwrap())
        .read_line(&mut ready_line)
        .expect("read Rust broker readiness line");
    assert!(ready_line.starts_with("BROWSER_BROKER_READY "));
    let endpoint = serde_json::from_str(
        ready_line
            .trim_start_matches("BROWSER_BROKER_READY ")
            .trim(),
    )
    .unwrap();
    (child, endpoint)
}

fn get_discovery(port: u16, origin: Option<&str>) -> (u16, Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "GET /v1/bridge HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n"
    )
    .unwrap();
    if let Some(origin) = origin {
        write!(stream, "Origin: {origin}\r\n").unwrap();
    }
    write!(stream, "\r\n").unwrap();

    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response must contain a header terminator");
    let headers = std::str::from_utf8(&response[..header_end]).unwrap();
    let status = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let body = &response[header_end + 4..];
    let payload = serde_json::from_slice(body).unwrap_or(Value::Null);
    (status, payload)
}

fn post_json(port: u16, path: &str, token: &str, origin: &str, payload: &Value) -> (u16, Value) {
    let response = reqwest::blocking::Client::new()
        .post(format!("http://127.0.0.1:{port}{path}?token={token}"))
        .header("Content-Type", "application/json")
        .header("Origin", origin)
        .header("X-Teshi-Broker-Token", token)
        .json(payload)
        .send()
        .unwrap();
    let status = response.status().as_u16();
    let body = response.json().unwrap_or(Value::Null);
    (status, body)
}

#[test]
fn internal_cli_process_serves_authenticated_rust_transport_and_state_without_python() {
    let temp = tempfile::tempdir().unwrap();
    let state_dir = temp.path().join("user-state");
    let port = unused_loopback_port();
    let mut child = BrokerChild(
        Command::new(env!("CARGO_BIN_EXE_teshi"))
            .args([
                "--browser-broker-internal",
                "--state-dir",
                state_dir.to_str().unwrap(),
                "--trusted-extension-origin",
                TRUSTED_ORIGIN,
                "--trusted-extension-origin",
                SECOND_TRUSTED_ORIGIN,
                "--discovery-port",
                &port.to_string(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the Teshi Rust broker process"),
    );

    let stdout = child.0.stdout.take().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let stdout_reader = thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line);
        let _ = ready_tx.send(result.map(|_| line));
    });
    let ready_line = ready_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Rust broker must announce readiness")
        .expect("read Rust broker readiness line");
    assert!(ready_line.starts_with("BROWSER_BROKER_READY "));
    let public: EndpointRecord = serde_json::from_str(
        ready_line
            .trim_start_matches("BROWSER_BROKER_READY ")
            .trim(),
    )
    .unwrap();
    assert_eq!(public.broker_pid, child.0.id());
    assert_eq!(public.broker_features, vec!["transport.v1"]);
    assert!(!public.ws_url.contains("token="));
    assert!(!public.extension_frame_ws_url.contains("token="));

    let credentials = PrivateCredentialStore::new(&state_dir);
    let credential = credentials.read_for_endpoint(&public).unwrap();
    assert_eq!(
        credential.trusted_extension_origins(),
        &[TRUSTED_ORIGIN.to_owned(), SECOND_TRUSTED_ORIGIN.to_owned()]
    );
    assert!(!format!("{credential:?}").contains(credential.token()));
    assert!(!ready_line.contains(credential.token()));

    let (status, local) = get_discovery(port, None);
    assert_eq!(status, 200);
    let local: DiscoveryResponse = serde_json::from_value(local).unwrap();
    assert!(!local.ws_url.contains("token="));
    assert!(!local.extension_frame_ws_url.contains("token="));

    let (status, extension) = get_discovery(port, Some(TRUSTED_ORIGIN));
    assert_eq!(status, 200);
    let extension: DiscoveryResponse = serde_json::from_value(extension).unwrap();
    assert!(
        extension
            .extension_frame_ws_url
            .contains(&format!("token={}", credential.token()))
    );

    let (status, second_extension) = get_discovery(port, Some(SECOND_TRUSTED_ORIGIN));
    assert_eq!(status, 200);
    let second_extension: DiscoveryResponse = serde_json::from_value(second_extension).unwrap();
    assert!(
        second_extension
            .extension_frame_ws_url
            .contains(&format!("token={}", credential.token()))
    );

    let (status, _) = get_discovery(port, Some(UNTRUSTED_ORIGIN));
    assert_eq!(
        status, 403,
        "an untrusted extension origin must fail closed"
    );

    let heartbeat = serde_json::json!({
        "schema_version": 1,
        "protocol_version": 1,
        "extension_instance_id": "integration-test-profile",
        "extension_version": "integration-test",
        "features": [{"feature": "p0.control", "available": true}],
        "supported_actions": ["click"],
        "supported_operations": [],
        "optional_permissions": {},
        "browser": {"name": "Chromium", "version": "test"},
        "url": "https://integration.test",
        "title": "Integration",
        "active_window_id": 7,
        "active_tab_id": 42,
        "windows": [{
            "id": 7,
            "focused": true,
            "tabs": [{
                "id": 42,
                "window_id": 7,
                "title": "Integration",
                "url": "https://integration.test",
                "active": true,
                "favIconUrl": "",
                "debuggable": true
            }]
        }]
    });
    let (status, heartbeat_response) = post_json(
        port,
        "/v1/bridge/heartbeat",
        credential.token(),
        TRUSTED_ORIGIN,
        &heartbeat,
    );
    assert_eq!(status, 200);
    assert_eq!(heartbeat_response["ok"], true);
    assert_eq!(
        heartbeat_response["extension_instance_id"],
        "integration-test-profile"
    );

    let mut denied = extension
        .extension_frame_ws_url
        .as_str()
        .into_client_request()
        .unwrap();
    denied
        .headers_mut()
        .insert(ORIGIN, HeaderValue::from_static(UNTRUSTED_ORIGIN));
    assert!(
        connect(denied).is_err(),
        "WebSocket Origin must be checked too"
    );

    let mut request = extension
        .extension_frame_ws_url
        .as_str()
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(ORIGIN, HeaderValue::from_static(TRUSTED_ORIGIN));
    let (mut socket, response) = connect(request).expect("trusted extension WS handshake");
    assert_eq!(response.status(), 101);
    let hello = ExtensionStreamMessage::StreamHello {
        project_root: None,
        extension_instance_id: "integration-test-profile".into(),
        protocol_version: BROWSER_BROKER_PROTOCOL_VERSION,
        extension_version: "integration-test".into(),
    };
    socket
        .send(Message::Text(serde_json::to_string(&hello).unwrap()))
        .unwrap();
    let stream_ack = socket.read().unwrap();
    let Message::Text(stream_ack) = stream_ack else {
        panic!("Rust broker must acknowledge the extension stream");
    };
    let stream_ack: Value = serde_json::from_str(&stream_ack).unwrap();
    assert_eq!(stream_ack["ok"], true);

    let client_request = format!("{}?token={}", public.ws_url, credential.token())
        .into_client_request()
        .unwrap();
    let (mut client_socket, _) = connect(client_request).expect("control WebSocket");
    client_socket
        .send(Message::Text(
            serde_json::json!({
                "schema_version": 1,
                "request_id": "list-sessions-1",
                "caller_label": "integration-test",
                "project_root": "C:/integration-project",
                "cmd": "list_browser_sessions"
            })
            .to_string(),
        ))
        .unwrap();
    let Message::Text(response) = client_socket.read().unwrap() else {
        panic!("Rust broker must return the session list response");
    };
    let response: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["operation"], "list_browser_sessions");
    assert_eq!(
        response["sessions"][0]["identity"]["extension_instance_id"],
        "integration-test-profile"
    );
    assert!(
        response
            .to_string()
            .find("C:/integration-project")
            .is_none()
    );
    client_socket.close(None).unwrap();
    socket.close(None).unwrap();

    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    assert!(
        !status.success(),
        "test explicitly terminates only its child"
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && TcpStream::connect(("127.0.0.1", port)).is_ok() {
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "the exact child listener must close after child termination"
    );
    stdout_reader.join().unwrap();
}

#[test]
fn internal_cli_broker_recovers_after_child_crash_without_touching_unrelated_listener() {
    let temp = tempfile::tempdir().unwrap();
    let state_dir = temp.path().join("user-state");
    let port = unused_loopback_port();
    let (mut first, first_endpoint) = spawn_internal_broker(&state_dir, port);
    let first_pid = first.0.id();

    first.0.kill().unwrap();
    let first_status = first.0.wait().unwrap();
    assert!(!first_status.success());
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && TcpStream::connect(("127.0.0.1", port)).is_ok() {
        thread::sleep(Duration::from_millis(25));
    }
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());

    let (second, second_endpoint) = spawn_internal_broker(&state_dir, port);
    assert_ne!(second.0.id(), first_pid);
    assert_ne!(
        second_endpoint.broker_start_id,
        first_endpoint.broker_start_id
    );
    let credential = PrivateCredentialStore::new(&state_dir)
        .read_for_endpoint(&second_endpoint)
        .unwrap();
    assert_eq!(
        &credential.broker_start_id,
        &second_endpoint.broker_start_id
    );

    let unrelated_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let unrelated_port = unrelated_listener.local_addr().unwrap().port();
    let conflict_state = temp.path().join("conflict-state");
    let mut conflicting_child = Command::new(env!("CARGO_BIN_EXE_teshi"))
        .args([
            "--browser-broker-internal",
            "--state-dir",
            conflict_state.to_str().unwrap(),
            "--trusted-extension-origin",
            TRUSTED_ORIGIN,
            "--discovery-port",
            &unrelated_port.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let conflict_status = conflicting_child.wait().unwrap();
    assert!(!conflict_status.success());
    assert!(TcpStream::connect(("127.0.0.1", unrelated_port)).is_ok());
}
