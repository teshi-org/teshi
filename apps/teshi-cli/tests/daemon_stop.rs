//! Process-level daemon stop contracts using only executor-owned MCP fixtures.

#![cfg(any(windows, target_os = "linux"))]

use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};
use teshi_update::processes::{self, DaemonEndpoint, ProcessIdentity};

struct OwnedProcess(Child);
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_fixture(project: &Path) -> OwnedProcess {
    std::fs::create_dir_all(project.join(".teshi")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_teshi"));
    command
        .args(["mcp", "serve", "--stdio", "--project"])
        .arg(project)
        .current_dir(project)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = OwnedProcess(command.spawn().unwrap());
    thread::sleep(Duration::from_millis(150));
    assert!(child.0.try_wait().unwrap().is_none());
    child
}

fn manifest(project: &Path, process: &ProcessIdentity, port: u16, start: &str) {
    std::fs::write(project.join(".teshi/daemon.json"), serde_json::to_vec(&serde_json::json!({
        "pid": process.pid, "port": port, "started": "2026-11-03T00:00:00Z", "process_start": start,
    })).unwrap()).unwrap();
}

fn stop(project: &Path, force: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_teshi"));
    command
        .args(["daemon", "stop"])
        .current_dir(project)
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1");
    if force {
        command.arg("--force");
    }
    command.output().unwrap()
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn rejected_shutdown_retains_manifest_and_explicit_force_confirms_exit() {
    let root = tempfile::tempdir().unwrap();
    let mut child = spawn_fixture(root.path());
    let process = processes::inspect(child.0.id()).unwrap().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    manifest(root.path(), &process, port, &process.started);
    let endpoint = DaemonEndpoint {
        process: process.clone(),
        port,
        project_root: Some(root.path().canonicalize().unwrap()),
    };
    let body = serde_json::to_string(&endpoint).unwrap();
    let server = thread::spawn(move || {
        for first in [true, false] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                assert_eq!(stream.read(&mut byte).unwrap(), 1);
                request.push(byte[0]);
            }
            if first {
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            } else {
                stream
                    .write_all(
                        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            }
        }
    });
    let output = stop(root.path(), false);
    server.join().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("403"));
    assert!(root.path().join(".teshi/daemon.json").exists());
    assert!(
        child.0.try_wait().unwrap().is_none(),
        "graceful failure must not kill the process"
    );
    let output = stop(root.path(), true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    child.0.wait().unwrap();
    assert!(!processes::is_running(&process).unwrap());
    assert!(!root.path().join(".teshi/daemon.json").exists());
}

#[test]
fn reused_pid_is_refused_even_with_force_and_manifest_is_preserved() {
    let root = tempfile::tempdir().unwrap();
    let mut child = spawn_fixture(root.path());
    let process = processes::inspect(child.0.id()).unwrap().unwrap();
    manifest(root.path(), &process, 0, "a-different-process-generation");
    let output = stop(root.path(), true);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("PID was reused"));
    assert!(child.0.try_wait().unwrap().is_none());
    assert!(root.path().join(".teshi/daemon.json").exists());
}
