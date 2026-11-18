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

/// Directory under an installation root holding updater state.
const UPDATE_STATE: &str = ".teshi-update";
/// Exit-request marker read by long-running Teshi processes.
const EXIT_REQUEST: &str = "exit-request.json";
/// Requests older than this are ignored, even after a crashed helper.
pub const EXIT_REQUEST_TTL_SECS: i64 = 600;

/// Installation-scoped request for long-running Teshi processes to exit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitRequest {
    /// Protocol version; only `1` is accepted.
    pub schema: u32,
    /// Pending transaction that asked for the exit.
    pub transaction: String,
    /// Unix time of the request.
    pub requested_at: i64,
}

/// Removes the request when the close phase ends, on success or failure.
pub struct ExitRequestGuard {
    path: PathBuf,
}
impl Drop for ExitRequestGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Publishes an exit request owned by the pending transaction.
///
/// # Errors
/// Rejects linked paths and invalid transaction identifiers.
pub fn publish_exit_request(root: &Path, transaction: &str, now: i64) -> Result<ExitRequestGuard> {
    if !transaction.starts_with("transaction-") || transaction.contains(['/', '\\', ':']) {
        return Err(crate::invalid("Invalid update transaction id"));
    }
    let relative = Path::new(UPDATE_STATE);
    reject_links(root, relative)?;
    private_directory(&root.join(relative))?;
    let relative = relative.join(EXIT_REQUEST);
    reject_links(root, &relative)?;
    let path = root.join(&relative);
    write_json(
        &path,
        &ExitRequest {
            schema: 1,
            transaction: transaction.into(),
            requested_at: now,
        },
    )?;
    Ok(ExitRequestGuard { path })
}

/// Reads a fresh exit request that matches the pending transaction.
///
/// The pending tie keeps a marker left by an interrupted update from quitting a
/// later session; the TTL covers a helper that died before cleaning it up.
pub fn exit_requested(root: &Path, now: i64) -> Option<ExitRequest> {
    let relative = Path::new(UPDATE_STATE).join(EXIT_REQUEST);
    reject_links(root, &relative).ok()?;
    let data = fs::read(root.join(&relative)).ok()?;
    if data.len() > 4096 {
        return None;
    }
    let request: ExitRequest = serde_json::from_slice(&data).ok()?;
    if request.schema != 1
        || request.requested_at > now
        || now - request.requested_at > EXIT_REQUEST_TTL_SECS
    {
        return None;
    }
    let pending = root.join(UPDATE_STATE).join("pending.json");
    let pending: String = serde_json::from_slice(&fs::read(pending).ok()?).ok()?;
    (pending == request.transaction).then_some(request)
}

/// Polls the installation for one exit request, delivering each request once.
pub struct ExitRequestWatcher {
    root: PathBuf,
    next_poll: Instant,
    delivered: Option<(String, i64)>,
}

impl ExitRequestWatcher {
    /// Watches one installation root; source builds simply have no root.
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            next_poll: Instant::now(),
            delivered: None,
        }
    }

    /// Returns a request at most once per `(transaction, requested_at)` pair.
    ///
    /// The latch prevents a still-present marker from forcing a second quit
    /// past the unsaved-work confirmation.
    pub fn poll(&mut self) -> Option<ExitRequest> {
        let now = Instant::now();
        if now < self.next_poll {
            return None;
        }
        self.next_poll = now + Duration::from_millis(500);
        let request = exit_requested(&self.root, chrono::Utc::now().timestamp())?;
        let key = (request.transaction.clone(), request.requested_at);
        if self.delivered.as_ref() == Some(&key) {
            return None;
        }
        self.delivered = Some(key);
        Some(request)
    }
}

/// Running processes belonging to this executable's installation, if marked.
///
/// # Errors
/// Returns installation, process enumeration and filesystem errors.
pub fn installation_processes_for(
    executable: &Path,
    identity: &teshi_core::version::BuildIdentity,
) -> Result<Vec<ProcessIdentity>> {
    let installation = crate::install::Installation::detect(executable, identity)?;
    match (installation.root.as_deref(), installation.bundle.as_ref()) {
        (Some(root), Some(bundle)) => installation_processes(root, bundle),
        _ => processes_for_images(&[executable.to_path_buf()]),
    }
}

/// One `PID <pid> <path>` line per process, for prompts and errors.
pub fn describe_processes(processes: &[ProcessIdentity]) -> String {
    processes
        .iter()
        .map(|process| format!("PID {} {}", process.pid, process.executable.display()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Actionable description of the processes still holding this installation.
pub fn busy_message(processes: &[ProcessIdentity], force_close: bool) -> String {
    let mut message = String::from("Teshi processes from this installation are still running:");
    if !processes.is_empty() {
        message.push_str(&format!("\n{}", describe_processes(processes)));
    }
    if force_close {
        message.push_str(
            "\nForce close did not finish. Retry `teshi update --force-close`, or close the processes listed above. Unsaved work may be lost.",
        );
    } else {
        message.push_str(
            "\nClose them and retry, or allow a verified force close with `teshi update --force-close` (cached payload: `teshi update --retry-install --force-close`). Unsaved work may be lost.",
        );
    }
    message
}

/// Whether a preflight may continue with the running installation processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePlan {
    /// Nothing is running.
    Proceed,
    /// Graceful exit first, then a verified force close after the timeout.
    ForceClose,
    /// Refuse before downloading; the caller reports the process list.
    Stop,
}

/// Decides how to handle `occupying` running processes.
///
/// `consent` is the answer to an explicit prompt; `None` means non-interactive.
pub fn plan_close(occupying: usize, force_close: bool, consent: Option<bool>) -> ClosePlan {
    if occupying == 0 {
        return ClosePlan::Proceed;
    }
    if force_close || consent == Some(true) {
        ClosePlan::ForceClose
    } else {
        ClosePlan::Stop
    }
}

/// Attempts a verified daemon request, a normal desktop-window close, and the
/// installation-wide exit request that console apps observe.
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

    fn exit_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".teshi-update")).unwrap();
        write_json(
            &root.path().join(".teshi-update/pending.json"),
            &"transaction-test",
        )
        .unwrap();
        root
    }

    #[test]
    fn exit_request_needs_a_matching_pending_transaction_and_fresh_timestamp() {
        let root = exit_root();
        let now = chrono::Utc::now().timestamp();
        let guard = publish_exit_request(root.path(), "transaction-test", now).unwrap();
        assert_eq!(
            exit_requested(root.path(), now).unwrap().transaction,
            "transaction-test"
        );
        assert!(
            exit_requested(root.path(), now + EXIT_REQUEST_TTL_SECS + 1).is_none(),
            "stale requests never quit a later session"
        );
        assert!(
            exit_requested(root.path(), now - 1).is_none(),
            "future timestamps are rejected"
        );
        drop(guard);
        assert!(
            exit_requested(root.path(), now).is_none(),
            "the guard removes the marker"
        );

        let guard = publish_exit_request(root.path(), "transaction-other", now).unwrap();
        assert!(
            exit_requested(root.path(), now).is_none(),
            "only the pending transaction may quit participants"
        );
        drop(guard);
        assert!(publish_exit_request(root.path(), "../escape", now).is_err());
    }

    #[test]
    fn watcher_delivers_one_request_once_and_rearms_for_the_next() {
        let root = exit_root();
        let now = chrono::Utc::now().timestamp();
        let guard = publish_exit_request(root.path(), "transaction-test", now).unwrap();
        let mut watcher = ExitRequestWatcher::new(root.path());
        assert!(watcher.poll().is_some(), "the first poll delivers");
        std::thread::sleep(Duration::from_millis(600));
        assert!(
            watcher.poll().is_none(),
            "the same request is never delivered twice"
        );

        drop(guard);
        // A later transaction is a distinct request even within the same second.
        let guard = publish_exit_request(root.path(), "transaction-second", now).unwrap();
        write_json(
            &root.path().join(".teshi-update/pending.json"),
            &"transaction-second",
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(600));
        assert!(
            watcher.poll().is_some(),
            "a new request re-arms the watcher"
        );
        drop(guard);
    }

    #[test]
    fn close_plan_requires_consent_before_anything_runs() {
        assert_eq!(plan_close(0, false, None), ClosePlan::Proceed);
        assert_eq!(plan_close(0, true, None), ClosePlan::Proceed);
        assert_eq!(plan_close(2, true, None), ClosePlan::ForceClose);
        assert_eq!(plan_close(2, false, Some(true)), ClosePlan::ForceClose);
        assert_eq!(plan_close(2, false, Some(false)), ClosePlan::Stop);
        assert_eq!(plan_close(2, false, None), ClosePlan::Stop);
    }

    #[test]
    fn busy_message_names_each_process_and_both_retry_paths() {
        let processes = vec![
            ProcessIdentity {
                pid: 11,
                executable: PathBuf::from("/opt/teshi/teshi"),
                started: "start-1".into(),
            },
            ProcessIdentity {
                pid: 12,
                executable: PathBuf::from("/opt/teshi/teshi-update-helper"),
                started: "start-2".into(),
            },
        ];
        let message = busy_message(&processes, false);
        assert!(message.contains("PID 11 /opt/teshi/teshi"), "{message}");
        assert!(message.contains("PID 12 /opt/teshi/teshi-update-helper"));
        assert!(message.contains("teshi update --force-close"));
        assert!(message.contains("teshi update --retry-install --force-close"));
        assert!(message.contains("Unsaved work"));
        assert_eq!(
            describe_processes(&processes),
            "PID 11 /opt/teshi/teshi\nPID 12 /opt/teshi/teshi-update-helper"
        );
    }
}
#[cfg(not(windows))]
#[path = "processes_unix.rs"]
mod platform;
