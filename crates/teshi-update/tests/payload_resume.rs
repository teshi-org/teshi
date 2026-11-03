//! Socket-level transfer faults and persistent cache acceptance.

use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Mutex, atomic::AtomicBool},
    thread,
};
use teshi_core::version::{BuildIdentity, ReleaseChannel};
use teshi_update::{
    ErrorCode, Result, download,
    github::{Candidate, Http, HttpResponse},
    manifest::{InstallKind, ReleaseAsset, ReleaseManifest},
    payload::Payload,
};

const BYTES: &[u8] = b"a complete payload with resumable contents";

fn candidate(url: String) -> Candidate {
    let tag = "v0.7.10-nightly.20260908.bbbbbbb".to_string();
    let asset = ReleaseAsset {
        name: format!("teshi-{tag}-x64-setup.exe"),
        target: "x86_64-pc-windows-msvc".into(),
        kind: InstallKind::Exe,
        size: BYTES.len() as u64,
        sha256: format!("{:x}", Sha256::digest(BYTES)),
    };
    Candidate {
        release_id: 1,
        asset_id: 2,
        download_url: url,
        release_url: "https://github.com/release".into(),
        manifest: ReleaseManifest {
            schema: 1,
            minimum_updater: 1,
            tag,
            identity: BuildIdentity {
                semver: "0.7.10".into(),
                channel: ReleaseChannel::Nightly,
                git_sha: "b".repeat(40),
                build_timestamp: "2026-09-08T12:00:00Z".into(),
                build_sequence: 12,
            },
            assets: vec![asset.clone()],
        },
        asset,
    }
}

/// Real loopback HTTP, injected only in tests; production still restricts HTTPS origins.
struct SocketHttp {
    client: reqwest::blocking::Client,
    requests: Mutex<Vec<Option<u64>>>,
}
impl SocketHttp {
    fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap(),
            requests: Mutex::default(),
        }
    }
    fn request(&self, url: &str, offset: Option<u64>) -> Result<HttpResponse> {
        self.requests.lock().unwrap().push(offset);
        let mut request = self.client.get(url);
        if let Some(offset) = offset {
            request = request.header("Range", format!("bytes={offset}-"));
        }
        let response = request.send().unwrap();
        Ok(HttpResponse {
            status: response.status().as_u16(),
            etag: None,
            retry_after: None,
            content_range: response
                .headers()
                .get("content-range")
                .map(|v| v.to_str().unwrap().to_string()),
            body: Box::new(response),
        })
    }
}
impl Http for SocketHttp {
    fn get(&self, url: &str, _: Option<&str>) -> Result<HttpResponse> {
        self.request(url, None)
    }
    fn get_range(&self, url: &str, start: u64) -> Result<HttpResponse> {
        self.request(url, Some(start))
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut data = Vec::new();
    let mut byte = [0];
    while !data.ends_with(b"\r\n\r\n") {
        assert_eq!(stream.read(&mut byte).unwrap(), 1);
        data.push(byte[0]);
        assert!(data.len() < 8192);
    }
    String::from_utf8(data).unwrap()
}

fn server(responses: Vec<(String, Vec<u8>)>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/payload", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        responses
            .into_iter()
            .map(|(headers, bytes)| {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                stream.write_all(headers.as_bytes()).unwrap();
                stream.write_all(&bytes).unwrap();
                request
            })
            .collect()
    });
    (url, handle)
}
fn full() -> (String, Vec<u8>) {
    (
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            BYTES.len()
        ),
        BYTES.to_vec(),
    )
}

#[test]
fn disconnected_transfer_resumes_only_the_missing_interval() {
    let partial = 7;
    let (headers, _) = full();
    let (url, handle) = server(vec![
        (headers, BYTES[..partial].to_vec()),
        (
            format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {partial}-{}/{}\r\nConnection: close\r\n\r\n",
                BYTES.len() - partial,
                BYTES.len() - 1,
                BYTES.len()
            ),
            BYTES[partial..].to_vec(),
        ),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("payload.part");
    let candidate = candidate(url);
    let http = SocketHttp::new();
    let cancel = AtomicBool::new(false);
    assert_eq!(
        download::download(&http, &candidate, &path, &cancel, &mut |_| {})
            .unwrap_err()
            .code,
        ErrorCode::Network
    );
    assert_eq!(std::fs::read(&path).unwrap(), &BYTES[..partial]);
    download::download(&http, &candidate, &path, &cancel, &mut |_| {}).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), BYTES);
    let requests = handle.join().unwrap();
    assert!(!requests[0].to_ascii_lowercase().contains("range:"));
    assert!(requests[1].to_ascii_lowercase().contains("range: bytes=7-"));
}

#[test]
fn unsupported_range_restarts_and_invalid_interval_is_rejected() {
    for response in [
        full(),
        (
            format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\n\r\n",
                BYTES.len(),
                BYTES.len() - 1,
                BYTES.len()
            ),
            BYTES.to_vec(),
        ),
    ] {
        let bad_range = response.0.contains("206");
        let (url, handle) = server(vec![response]);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("payload.part");
        std::fs::write(&path, &BYTES[..7]).unwrap();
        let result = download::download(
            &SocketHttp::new(),
            &candidate(url),
            &path,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        if bad_range {
            assert_eq!(result.unwrap_err().code, ErrorCode::Verification);
            assert!(!path.exists());
        } else {
            result.unwrap();
            assert_eq!(std::fs::read(path).unwrap(), BYTES);
        }
        assert!(
            handle.join().unwrap()[0]
                .to_ascii_lowercase()
                .contains("range: bytes=7-")
        );
    }
}

#[test]
fn cache_survives_failure_rechecks_offline_and_redownloads_tampering() {
    let (url, handle) = server(vec![full(), full()]);
    let candidate = candidate(url);
    let temp = tempfile::tempdir().unwrap();
    let http = SocketHttp::new();
    let cancel = AtomicBool::new(false);
    let payload = Payload::reserve(temp.path(), &candidate).unwrap();
    let path = payload
        .obtain(&http, &candidate, false, &cancel, &mut |_| {})
        .unwrap();
    drop(payload); // Simulates disposal of a failed installation transaction.
    let payload = Payload::reserve(temp.path(), &candidate).unwrap();
    assert_eq!(
        payload
            .obtain(&http, &candidate, true, &cancel, &mut |_| {})
            .unwrap(),
        path
    );
    assert_eq!(http.requests.lock().unwrap().len(), 1);
    std::fs::write(&path, vec![b'x'; BYTES.len()]).unwrap();
    assert_eq!(
        payload
            .obtain(&http, &candidate, true, &cancel, &mut |_| {})
            .unwrap_err()
            .code,
        ErrorCode::Verification
    );
    assert_eq!(
        http.requests.lock().unwrap().len(),
        1,
        "offline retry must not download damaged cache"
    );
    payload
        .obtain(&http, &candidate, false, &cancel, &mut |_| {})
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), BYTES);
    assert_eq!(handle.join().unwrap().len(), 2);
}

#[test]
fn cache_reservation_excludes_a_second_updater_and_metadata_must_match() {
    let temp = tempfile::tempdir().unwrap();
    let mut candidate = candidate("https://github.com/payload".into());
    let first = Payload::reserve(temp.path(), &candidate).unwrap();
    assert!(
        matches!(Payload::reserve(temp.path(), &candidate), Err(error) if error.code == ErrorCode::Busy)
    );
    drop(first);
    Payload::reserve(temp.path(), &candidate).unwrap();
    candidate.asset.sha256 = "c".repeat(64);
    assert!(
        matches!(Payload::reserve(temp.path(), &candidate), Err(error) if error.code == ErrorCode::InvalidManifest)
    );
}

#[test]
fn exhausted_retries_keep_partial_bytes_for_the_next_invocation() {
    let partial = 7;
    let (headers, _) = full();
    let unavailable = || {
        (
            "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            vec![],
        )
    };
    let (url, handle) = server(vec![
        (headers, BYTES[..partial].to_vec()),
        unavailable(),
        unavailable(),
        (
            format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {partial}-{}/{}\r\nConnection: close\r\n\r\n",
                BYTES.len() - partial,
                BYTES.len() - 1,
                BYTES.len()
            ),
            BYTES[partial..].to_vec(),
        ),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let http = SocketHttp::new();
    let candidate = candidate(url);
    let cancel = AtomicBool::new(false);
    let payload = Payload::reserve(temp.path(), &candidate).unwrap();
    assert_eq!(
        payload
            .obtain(&http, &candidate, false, &cancel, &mut |_| {})
            .unwrap_err()
            .code,
        ErrorCode::Network
    );
    assert_eq!(
        http.requests.lock().unwrap().len(),
        3,
        "each invocation must stop after three transfer attempts"
    );
    drop(payload);
    let payload = Payload::reserve(temp.path(), &candidate).unwrap();
    let path = payload
        .obtain(&http, &candidate, false, &cancel, &mut |_| {})
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), BYTES);
    let requests = handle.join().unwrap();
    assert!(
        requests[1..]
            .iter()
            .all(|request| request.to_ascii_lowercase().contains("range: bytes=7-"))
    );
}
