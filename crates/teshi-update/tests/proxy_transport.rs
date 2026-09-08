//! Isolated-process tests for update proxy discovery and intercepting TLS.
//!
//! Child processes inherit only the proxy environment for the test under way so
//! parallel cargo tests cannot leak `HTTPS_PROXY` into unrelated cases.

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::mpsc,
    thread,
    time::Duration,
};
use teshi_update::{
    ErrorCode,
    github::{GithubHttp, Http},
};

const GITHUB_RELEASES: &str = "https://api.github.com/repos/teshi-org/teshi/releases?per_page=1";
const API_CERT: &[u8] = include_bytes!("fixtures/api_github_com.crt.der");
const API_KEY: &[u8] = include_bytes!("fixtures/api_github_com.key.der");
const WRONG_CERT: &[u8] = include_bytes!("fixtures/not-github_example.crt.der");
const WRONG_KEY: &[u8] = include_bytes!("fixtures/not-github_example.key.der");

#[derive(Clone, Copy)]
enum ProxyMode {
    HangUp,
    Reject,
    TlsGithub,
    TlsWrongHost,
}

fn in_child(test: &str) -> bool {
    std::env::var("TESHI_UPDATE_PROXY_CHILD").ok().as_deref() == Some(test)
}

fn spawn_child(test: &str, envs: &[(&str, String)]) -> std::process::Output {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", test, "--nocapture"]);
    command.env("TESHI_UPDATE_PROXY_CHILD", test);
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        // Windows process inheritance can keep parent proxy vars after env_remove.
        command.env(key, "");
    }
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("re-exec proxy transport test")
}

fn assert_child_ok(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}stderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn start_proxy(mode: ProxyMode) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind proxy");
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            handle_proxy_client(stream, mode, tx);
        }
    });
    (url, rx)
}

fn handle_proxy_client(mut stream: std::net::TcpStream, mode: ProxyMode, tx: mpsc::Sender<String>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while buf.len() < 8192 {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                buf.push(byte[0]);
                if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let request = String::from_utf8_lossy(&buf);
    let target = request
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("CONNECT "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("invalid")
        .to_string();
    let _ = tx.send(target);
    match mode {
        ProxyMode::HangUp => {}
        ProxyMode::Reject => {
            let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
        }
        ProxyMode::TlsGithub => {
            let _ = stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
            serve_self_signed_tls(stream, API_CERT, API_KEY);
        }
        ProxyMode::TlsWrongHost => {
            let _ = stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
            serve_self_signed_tls(stream, WRONG_CERT, WRONG_KEY);
        }
    }
}

fn serve_self_signed_tls(mut stream: std::net::TcpStream, cert: &[u8], key: &[u8]) {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cert = CertificateDer::from(cert.to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec()));
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .expect("tls server config");
    let mut server = rustls::ServerConnection::new(std::sync::Arc::new(config)).expect("tls conn");
    let mut tls = rustls::Stream::new(&mut server, &mut stream);
    let mut buf = [0u8; 32];
    let _ = tls.read(&mut buf);
    let _ = tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n[]");
}

fn github_get_error() -> teshi_update::UpdateError {
    let Err(error) = GithubHttp::new()
        .expect("github client")
        .get(GITHUB_RELEASES, None)
    else {
        panic!("proxied request should fail in this fixture");
    };
    error
}

#[test]
fn https_proxy_tunnels_github() {
    if in_child("https_proxy_tunnels_github") {
        let error = github_get_error();
        assert_eq!(error.code, ErrorCode::Network);
        assert!(!error.message.contains("secret"));
        return;
    }
    let (url, rx) = start_proxy(ProxyMode::HangUp);
    let output = spawn_child("https_proxy_tunnels_github", &[("HTTPS_PROXY", url)]);
    assert_child_ok(&output);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)).expect("CONNECT"),
        "api.github.com:443"
    );
}

#[test]
fn all_proxy_tunnels_github_when_https_proxy_is_unset() {
    if in_child("all_proxy_tunnels_github_when_https_proxy_is_unset") {
        let error = github_get_error();
        assert_eq!(error.code, ErrorCode::Network);
        return;
    }
    let (url, rx) = start_proxy(ProxyMode::HangUp);
    let output = spawn_child(
        "all_proxy_tunnels_github_when_https_proxy_is_unset",
        &[("ALL_PROXY", url)],
    );
    assert_child_ok(&output);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)).expect("CONNECT"),
        "api.github.com:443"
    );
}

#[test]
fn https_proxy_takes_precedence_over_all_proxy() {
    if in_child("https_proxy_takes_precedence_over_all_proxy") {
        let error = github_get_error();
        assert_eq!(error.code, ErrorCode::Network);
        return;
    }
    let (https_url, https_rx) = start_proxy(ProxyMode::HangUp);
    let (all_url, all_rx) = start_proxy(ProxyMode::HangUp);
    let output = spawn_child(
        "https_proxy_takes_precedence_over_all_proxy",
        &[("HTTPS_PROXY", https_url), ("ALL_PROXY", all_url)],
    );
    assert_child_ok(&output);
    assert_eq!(
        https_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("HTTPS_PROXY CONNECT"),
        "api.github.com:443"
    );
    assert!(
        all_rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "ALL_PROXY must not intercept HTTPS when HTTPS_PROXY is set"
    );
}

#[test]
fn no_proxy_bypasses_https_proxy_for_github() {
    if in_child("no_proxy_bypasses_https_proxy_for_github") {
        let _ = GithubHttp::new()
            .expect("github client")
            .get(GITHUB_RELEASES, None);
        return;
    }
    let (url, rx) = start_proxy(ProxyMode::HangUp);
    let output = spawn_child(
        "no_proxy_bypasses_https_proxy_for_github",
        &[("HTTPS_PROXY", url), ("NO_PROXY", "api.github.com".into())],
    );
    assert_child_ok(&output);
    assert!(
        rx.recv_timeout(Duration::from_millis(400)).is_err(),
        "NO_PROXY should skip the proxy"
    );
}

#[test]
fn proxy_connect_failure_is_a_network_error() {
    if in_child("proxy_connect_failure_is_a_network_error") {
        let error = github_get_error();
        assert_eq!(error.code, ErrorCode::Network);
        return;
    }
    let (url, rx) = start_proxy(ProxyMode::Reject);
    let output = spawn_child(
        "proxy_connect_failure_is_a_network_error",
        &[("HTTPS_PROXY", url)],
    );
    assert_child_ok(&output);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)).expect("CONNECT"),
        "api.github.com:443"
    );
}

#[test]
fn intercepting_proxy_with_untrusted_certificate_is_rejected() {
    if in_child("intercepting_proxy_with_untrusted_certificate_is_rejected") {
        let error = github_get_error();
        assert_eq!(error.code, ErrorCode::Network);
        return;
    }
    let (url, rx) = start_proxy(ProxyMode::TlsGithub);
    let output = spawn_child(
        "intercepting_proxy_with_untrusted_certificate_is_rejected",
        &[("HTTPS_PROXY", url)],
    );
    assert_child_ok(&output);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)).expect("CONNECT"),
        "api.github.com:443"
    );
}

#[test]
fn intercepting_proxy_with_hostname_mismatch_is_rejected() {
    if in_child("intercepting_proxy_with_hostname_mismatch_is_rejected") {
        let error = github_get_error();
        assert_eq!(error.code, ErrorCode::Network);
        return;
    }
    let (url, rx) = start_proxy(ProxyMode::TlsWrongHost);
    let output = spawn_child(
        "intercepting_proxy_with_hostname_mismatch_is_rejected",
        &[("HTTPS_PROXY", url)],
    );
    assert_child_ok(&output);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)).expect("CONNECT"),
        "api.github.com:443"
    );
}

#[test]
fn trusted_intercept_root_is_accepted_by_tls_certs_only() {
    if in_child("trusted_intercept_root_is_accepted_by_tls_certs_only") {
        let cert = reqwest::Certificate::from_der(API_CERT).expect("fixture cert");
        let client = reqwest::blocking::Client::builder()
            .tls_certs_only([cert])
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(8))
            .build()
            .expect("strict intercept client");
        let response = client
            .get(GITHUB_RELEASES)
            .send()
            .expect("trusted intercept handshake");
        assert_eq!(response.status().as_u16(), 200);
        return;
    }
    let (url, rx) = start_proxy(ProxyMode::TlsGithub);
    let output = spawn_child(
        "trusted_intercept_root_is_accepted_by_tls_certs_only",
        &[("HTTPS_PROXY", url)],
    );
    assert_child_ok(&output);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)).expect("CONNECT"),
        "api.github.com:443"
    );
}

#[test]
fn github_http_does_not_disable_certificate_verification() {
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/github.rs"));
    assert!(
        !source.contains("danger_accept_invalid_certs"),
        "update TLS must not disable certificate verification"
    );
}
