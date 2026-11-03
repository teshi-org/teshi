//! Installation-scoped process discovery and verified shutdown coordination.

use crate::{
    ErrorCode, Result, UpdateError,
    install::{Installation, reject_links},
    manifest::BundleManifest,
    storage::{private_directory, write_json},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// An OS process generation, never just a reusable PID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    /// OS process identifier.
    pub pid: u32,
    /// Canonical executable path.
    pub executable: PathBuf,
    /// OS creation identity, compared again on the handle used for termination.
    pub started: String,
}

/// Daemon identity advertised on a verified loopback endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonEndpoint {
    /// Exact process generation owning the endpoint.
    pub process: ProcessIdentity,
    /// Bound loopback port.
    pub port: u16,
    /// Original project root, used to reject stale project manifests.
    pub project_root: Option<PathBuf>,
}

/// Reads the current process generation, or None after it exits.
///
/// # Errors
/// Returns access, process inspection or unsupported-platform errors.
pub fn inspect(pid: u32) -> Result<Option<ProcessIdentity>> {
    platform::inspect(pid)
}

/// Whether the captured process generation is still running.
///
/// # Errors
/// Propagates OS inspection errors instead of claiming that exit succeeded.
pub fn is_running(process: &ProcessIdentity) -> Result<bool> {
    Ok(inspect(process.pid)?.as_ref() == Some(process))
}

/// Waits for one captured process to exit without affecting a reused PID.
///
/// # Errors
/// Returns OS inspection failures.
pub fn wait_for_exit(process: &ProcessIdentity, timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    while is_running(process)? {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(true)
}

/// Force-terminates the verified process generation and confirms actual exit.
///
/// # Errors
/// Refuses PID reuse, self-termination, access failures or unavailable safe APIs.
pub fn terminate(process: &ProcessIdentity) -> Result<()> {
    if process.pid == std::process::id() {
        return Err(crate::invalid("Refusing to terminate the calling process"));
    }
    platform::terminate(process)
}

/// Finds running executables owned by this exact installation inventory.
///
/// # Errors
/// Returns manifest, process enumeration and filesystem errors.
pub fn installation_processes(
    root: &Path,
    manifest: &BundleManifest,
) -> Result<Vec<ProcessIdentity>> {
    let images: Vec<_> = manifest
        .files
        .iter()
        .filter(|file| file.executable)
        .map(|file| root.join(&file.path))
        .collect();
    processes_for_images(&images)
}

/// Finds processes for exact executable paths, including unmarked Cargo builds.
///
/// # Errors
/// Returns OS enumeration errors. Uninspectable unrelated processes are skipped.
pub fn processes_for_images(images: &[PathBuf]) -> Result<Vec<ProcessIdentity>> {
    let canonical: Vec<_> = images
        .iter()
        .filter_map(|image| fs::canonicalize(image).ok())
        .collect();
    let mut processes = Vec::new();
    for pid in platform::pids()? {
        if pid == std::process::id() {
            continue;
        }
        if let Ok(Some(process)) = inspect(pid)
            && canonical
                .iter()
                .any(|image| same_image(image, &process.executable))
        {
            processes.push(process);
        }
    }
    processes.sort_by_key(|process| process.pid);
    Ok(processes)
}

/// Compares canonical executable paths using the host's path semantics.
pub fn same_image(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

/// Keeps a daemon endpoint registration alive until server shutdown.
pub struct EndpointRegistration {
    path: PathBuf,
}
impl Drop for EndpointRegistration {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn endpoint_name(process: &ProcessIdentity) -> String {
    // Hash the opaque creation token so it never becomes a path component.
    use sha2::{Digest, Sha256};
    format!(
        "{}-{:x}.json",
        process.pid,
        Sha256::digest(process.started.as_bytes())
    )
}

/// Registers a daemon for update-time shutdown only when self-update is supported.
/// Check-only and unmarked installations do not create updater installation state.
///
/// # Errors
/// Returns process, manifest or filesystem errors.
pub fn register_daemon(
    port: u16,
    project_root: Option<PathBuf>,
) -> Result<Option<EndpointRegistration>> {
    let installation = Installation::detect(
        &std::env::current_exe()?,
        &teshi_core::version::build_identity(),
    )?;
    if !installation.can_install() {
        return Ok(None);
    }
    let root = installation
        .root
        .as_deref()
        .ok_or_else(|| crate::invalid("Missing installation root"))?;
    let process = inspect(std::process::id())?
        .ok_or_else(|| crate::invalid("Daemon process exited during registration"))?;
    let relative = PathBuf::from(".teshi-update/daemons").join(endpoint_name(&process));
    reject_links(root, &relative)?;
    let path = root.join(relative);
    private_directory(
        path.parent()
            .ok_or_else(|| crate::invalid("Missing endpoint directory"))?,
    )?;
    write_json(
        &path,
        &DaemonEndpoint {
            process,
            port,
            project_root,
        },
    )?;
    Ok(Some(EndpointRegistration { path }))
}

/// Requests shutdown only after verifying the endpoint's process and project.
/// The server rechecks the process generation on the shutdown request itself.
///
/// # Errors
/// Returns identity mismatch, HTTP failure or a non-success shutdown response.
pub fn request_daemon_shutdown(endpoint: &DaemonEndpoint) -> Result<()> {
    if !is_running(&endpoint.process)? {
        return Ok(());
    }
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| UpdateError::new(ErrorCode::Network, e.to_string()))?;
    let base = format!("http://127.0.0.1:{}", endpoint.port);
    let response = client
        .get(format!("{base}/api/v1/daemon/process"))
        .send()
        .map_err(|e| UpdateError::new(ErrorCode::Network, e.to_string()))?;
    if !response.status().is_success() {
        return Err(UpdateError::new(
            ErrorCode::Installation,
            format!("Daemon identity returned HTTP {}", response.status()),
        ));
    }
    let actual: DaemonEndpoint = serde_json::from_reader(response.take(64 * 1024))?;
    if actual.process != endpoint.process
        || actual.project_root != endpoint.project_root
        || actual.port != endpoint.port
    {
        return Err(crate::invalid(
            "Daemon endpoint identity/project does not match the captured process",
        ));
    }
    let response = client
        .post(format!("{base}/api/v1/daemon/shutdown"))
        .header("X-Teshi-Process-Pid", endpoint.process.pid)
        .header("X-Teshi-Process-Start", &endpoint.process.started)
        .send()
        .map_err(|e| UpdateError::new(ErrorCode::Network, e.to_string()))?;
    if !response.status().is_success() {
        return Err(UpdateError::new(
            ErrorCode::Installation,
            format!("Daemon shutdown returned HTTP {}", response.status()),
        ));
    }
    Ok(())
}

/// Attempts a verified daemon request and a normal desktop-window close.
/// Returns diagnostic text; an unsuccessful request never counts as process exit.
pub(crate) fn request_exit(root: &Path, process: &ProcessIdentity) -> String {
    use std::io::Read;
    let relative = PathBuf::from(".teshi-update/daemons").join(endpoint_name(process));
    let endpoint = (|| -> Result<DaemonEndpoint> {
        reject_links(root, &relative)?;
        let file = fs::File::open(root.join(&relative))?;
        Ok(serde_json::from_reader(file.take(64 * 1024))?)
    })();
    if let Ok(endpoint) = endpoint
        && endpoint.process == *process
    {
        return match request_daemon_shutdown(&endpoint) {
            Ok(()) => "Daemon accepted graceful shutdown".into(),
            Err(error) => error.to_string(),
        };
    }
    platform::request_window_close(process);
    "Waiting for the application to exit".into()
}

use std::io::Read;
#[cfg(windows)]
#[path = "processes_windows.rs"]
mod platform;

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use super::*;
    use std::{
        io::Write,
        net::TcpListener,
        process::{Child, Command, Stdio},
        thread,
    };

    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    #[ignore = "spawned only by the process identity test"]
    fn owned_child_fixture() {
        if std::env::var_os("TESHI_PROCESS_TEST_CHILD").is_some() {
            thread::sleep(Duration::from_secs(300));
        }
    }

    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn force_close_checks_generation_and_confirms_owned_child_exit() {
        let exe = std::env::current_exe().unwrap();
        let mut command = Command::new(&exe);
        command
            .args([
                "--exact",
                "processes::tests::owned_child_fixture",
                "--ignored",
            ])
            .env("TESHI_PROCESS_TEST_CHILD", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = OwnedChild(command.spawn().unwrap());
        let identity = inspect(child.0.id()).unwrap().unwrap();
        let mut wrong = identity.clone();
        wrong.started.push('0');
        assert!(
            terminate(&wrong).is_err(),
            "PID alone must not authorize termination"
        );
        assert!(is_running(&identity).unwrap());
        let listed = processes_for_images(std::slice::from_ref(&exe)).unwrap();
        assert!(listed.contains(&identity));
        terminate(&identity).unwrap();
        assert!(!is_running(&identity).unwrap());
        child.0.wait().unwrap();
    }

    #[test]
    fn daemon_request_rejects_http_failure_and_changed_endpoint_identity() {
        let process = inspect(std::process::id()).unwrap().unwrap();
        for changed in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = DaemonEndpoint {
                process: process.clone(),
                port: listener.local_addr().unwrap().port(),
                project_root: None,
            };
            let mut actual = endpoint.clone();
            if changed {
                actual.process.started.push('0');
            }
            let body = serde_json::to_string(&actual).unwrap();
            let server = thread::spawn(move || {
                for index in 0..if changed { 1 } else { 2 } {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        assert_eq!(socket.read(&mut byte).unwrap(), 1);
                        request.push(byte[0]);
                    }
                    if index == 0 {
                        write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    } else {
                        assert!(
                            String::from_utf8_lossy(&request)
                                .to_ascii_lowercase()
                                .contains("x-teshi-process-start:")
                        );
                        socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    }
                }
            });
            let error = request_daemon_shutdown(&endpoint).unwrap_err();
            if changed {
                assert_eq!(error.code, ErrorCode::InvalidManifest);
            } else {
                assert!(error.message.contains("403"));
            }
            server.join().unwrap();
            assert!(is_running(&process).unwrap());
        }
    }
}
#[cfg(not(windows))]
#[path = "processes_unix.rs"]
mod platform;
