//! Regression test: verify daemon spawn does not block the parent process.
//!
//! This test validates that `teshi daemon start` exits promptly after spawning
//! the background daemon.  On Windows, certain `std::process::Command` flags
//! (DETACHED_PROCESS, CREATE_NEW_PROCESS_GROUP) can cause the parent to hang.
//!
//! ## How it works
//!
//! 1. Build a fake project root under a temp dir with a `.teshi/` directory.
//! 2. Spawn `teshi daemon start` as a child process with a generous timeout.
//! 3. Assert the parent process exits within the timeout.
//! 4. Assert `.teshi/daemon.json` was written and points to a live daemon.
//! 5. Clean up: POST /api/v1/daemon/shutdown, kill if necessary.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Max seconds we allow `teshi daemon start` to return.
const START_TIMEOUT_SECS: u64 = 5;
/// Max seconds to wait for the daemon to write its manifest.
const DAEMON_READY_TIMEOUT_SECS: u64 = 30;

/// Build the teshi binary path from CARGO_BIN_EXE_teshi or the target directory.
fn teshi_binary() -> String {
    std::env::var("CARGO_BIN_EXE_teshi")
        .or_else(|_| {
            let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
            let target = manifest_dir.join("target/debug/teshi.exe");
            if target.exists() {
                Ok(target.to_string_lossy().to_string())
            } else {
                Err(std::env::VarError::NotPresent)
            }
        })
        .expect("teshi binary not found; build with `cargo build` first")
}

/// A temporary directory that is cleaned up on drop.
struct TmpDir {
    path: std::path::PathBuf,
}

impl TmpDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("teshi-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TmpDir { path: dir }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

#[test]
fn daemon_start_exits_promptly() {
    let tmp = TmpDir::new();

    // Create .teshi/ directory (required for find_project_root)
    let teshi_dir = tmp.path().join(".teshi");
    std::fs::create_dir_all(&teshi_dir).unwrap();

    // Spawn `teshi daemon start` with CWD set to the temp project
    let mut child = Command::new(teshi_binary())
        .args(["daemon", "start"])
        .current_dir(tmp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn teshi daemon start");

    // Wait with timeout
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                assert!(
                    status.success(),
                    "daemon start exited with {status}"
                );
                break;
            }
            Ok(None) => {
                if start.elapsed() > Duration::from_secs(START_TIMEOUT_SECS) {
                    // Kill and fail
                    let _ = child.kill();
                    // Read any output for diagnostics
                    let mut stderr = String::new();
                    if let Some(mut pipe) = child.stderr.take() {
                        pipe.read_to_string(&mut stderr).ok();
                    }
                    panic!(
                        "daemon start did not exit within {START_TIMEOUT_SECS}s\n\
                         stderr: {stderr}"
                    );
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => panic!("failed to wait on daemon start: {e}"),
        }
    }

    // Verify daemon.json was written (give it time — PowerShell spawn adds latency)
    let manifest_path = teshi_dir.join("daemon.json");
    let start = std::time::Instant::now();
    loop {
        if manifest_path.exists() {
            break;
        }
        if start.elapsed() > Duration::from_secs(DAEMON_READY_TIMEOUT_SECS) {
            panic!(
                "daemon.json not found at {} after {:?}",
                manifest_path.display(),
                start.elapsed()
            );
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    // Parse manifest and verify daemon is alive (poll for TCP readiness)
    let data = std::fs::read_to_string(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&data).unwrap();
    let port = manifest["port"].as_u64().unwrap() as u16;

    // The daemon writes the manifest before the server is ready.
    // Poll for TCP connectivity (max 15s).
    let addr = format!("127.0.0.1:{port}");
    let start = std::time::Instant::now();
    loop {
        if std::net::TcpStream::connect_timeout(
            &addr.parse().unwrap(),
            Duration::from_millis(500),
        )
        .is_ok()
        {
            break;
        }
        if start.elapsed() > Duration::from_secs(15) {
            panic!("daemon not reachable on {addr} after {:?}", start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    // Shut down the daemon
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let _ = client
        .post(format!("http://{addr}/api/v1/daemon/shutdown"))
        .send();

    // Wait for daemon to exit
    for _ in 0..20 {
        if std::net::TcpStream::connect_timeout(
            &addr.parse().unwrap(),
            Duration::from_millis(100),
        )
        .is_err()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[test]
fn daemon_start_exits_fast_when_already_running() {
    let tmp = TmpDir::new();
    let teshi_dir = tmp.path().join(".teshi");
    std::fs::create_dir_all(&teshi_dir).unwrap();

    // First start
    let mut child1 = Command::new(teshi_binary())
        .args(["daemon", "start"])
        .current_dir(tmp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Wait for first start to complete
    let start = std::time::Instant::now();
    loop {
        if child1.try_wait().unwrap().is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(START_TIMEOUT_SECS) {
            child1.kill().ok();
            panic!("first daemon start hung");
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    // Read manifest to get port (poll with timeout)
    let manifest_path = teshi_dir.join("daemon.json");
    let start = std::time::Instant::now();
    loop {
        if manifest_path.exists() {
            break;
        }
        if start.elapsed() > Duration::from_secs(DAEMON_READY_TIMEOUT_SECS) {
            panic!("first daemon didn't write manifest after {:?}", start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let data = std::fs::read_to_string(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&data).unwrap();
    let port = manifest["port"].as_u64().unwrap() as u16;

    // Second start — should exit fast (daemon already running)
    let start2 = std::time::Instant::now();
    let output = Command::new(teshi_binary())
        .args(["daemon", "start"])
        .current_dir(tmp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "second daemon start failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        start2.elapsed() < Duration::from_secs(3),
        "second daemon start took too long: {:?}",
        start2.elapsed()
    );

    // Shut down daemon
    let addr = format!("127.0.0.1:{port}");
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let _ = client
        .post(format!("http://{addr}/api/v1/daemon/shutdown"))
        .send();
}
