//! Real helper acceptance for the installation-scoped process-close policy.
//!
//! These cases spawn disposable bundles and real helper processes; they never
//! touch an installed Teshi.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
use teshi_core::version::{BUILD_TARGET, BuildIdentity, ReleaseChannel};
use teshi_update::{
    UpdateStatus,
    download::file_hash,
    manifest::{BUNDLE_MANIFEST, BundleManifest, InstallKind, ManagedFile},
    storage::write_json,
    transaction::{Preparation, last_result},
};

// Serialize these cases: they spawn helpers, mutate process-wide env and
// enumerate OS processes.
static HELPER_TEST_LOCK: Mutex<()> = Mutex::new(());

fn serialize_helper_tests() -> std::sync::MutexGuard<'static, ()> {
    HELPER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn cli_name() -> &'static str {
    if cfg!(windows) { "teshi.exe" } else { "teshi" }
}

fn helper_name() -> &'static str {
    if cfg!(windows) {
        "teshi-update-helper.exe"
    } else {
        "teshi-update-helper"
    }
}

fn desktop_name() -> &'static str {
    if cfg!(windows) {
        "teshi-desktop.exe"
    } else {
        "teshi-desktop"
    }
}

/// The app under test: a console process with no window and no IPC endpoint.
const POLL_EXIT_BODY: &str = r#"
    let root = std::env::current_exe().unwrap();
    let root = root.parent().unwrap();
    let request = root.join(".teshi-update").join("exit-request.json");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if request.exists() {
            std::process::exit(0);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    std::process::exit(2);
"#;

/// A window-less process that ignores every graceful channel.
const BLOCKER_BODY: &str = r#"
    std::thread::sleep(std::time::Duration::from_secs(300));
"#;

fn identity(version: u64) -> BuildIdentity {
    BuildIdentity {
        semver: "0.7.10".into(),
        channel: ReleaseChannel::Nightly,
        git_sha: format!("{version:040x}"),
        build_timestamp: "2026-11-18T00:00:00Z".into(),
        build_sequence: version,
    }
}

/// Fixture source that also answers the post-install identity smoke check.
fn fixture_source(behavior: &str, identity: &BuildIdentity) -> String {
    let identity = serde_json::to_string(identity).unwrap();
    format!(
        "fn main() {{\n    if std::env::args().any(|arg| arg == \"--update-identity\") {{\n        println!(\"{{}}\", r#\"{identity}\"#);\n        return;\n    }}\n{behavior}\n}}\n"
    )
}

fn compile(root: &Path, source_text: &str) -> PathBuf {
    fs::create_dir_all(root).unwrap();
    let source = root.join("fixture.rs");
    fs::write(&source, source_text).unwrap();
    let output = root.join(cli_name());
    assert!(
        Command::new("rustc")
            .arg(&source)
            .arg("--crate-name")
            .arg("process_close_fixture")
            .arg("-o")
            .arg(&output)
            .status()
            .unwrap()
            .success(),
        "fixture did not compile"
    );
    fs::remove_file(source).unwrap();
    output
}

fn bundle(root: &Path, behavior: &str, version: u64) -> BundleManifest {
    fs::create_dir_all(root.join("share")).unwrap();
    let identity = identity(version);
    compile(root, &fixture_source(behavior, &identity));
    let cli_path = root.join(cli_name());
    fs::copy(
        env!("CARGO_BIN_EXE_teshi-update-helper"),
        root.join(helper_name()),
    )
    .unwrap();
    fs::copy(&cli_path, root.join(desktop_name())).unwrap();
    fs::write(root.join("share/version.txt"), version.to_string()).unwrap();
    let files = [
        cli_name(),
        helper_name(),
        desktop_name(),
        "share/version.txt",
    ]
    .into_iter()
    .map(|name| {
        let path = root.join(name);
        ManagedFile {
            path: name.into(),
            size: path.metadata().unwrap().len(),
            sha256: file_hash(&path).unwrap(),
            executable: name != "share/version.txt",
        }
    })
    .collect();
    let manifest = BundleManifest {
        schema: 1,
        identity,
        target: BUILD_TARGET.into(),
        kind: InstallKind::Portable,
        layout: 1,
        update_explanation: None,
        files,
    };
    write_json(&root.join(BUNDLE_MANIFEST), &manifest).unwrap();
    manifest
}

fn spawn(executable: &Path, cwd: &Path) -> Child {
    let mut command = Command::new(executable);
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn().unwrap()
}

struct OwnedChild(Child);
impl OwnedChild {
    fn running(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for_result(root: &Path) -> teshi_update::transaction::TransactionResult {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = last_result(root)
            && matches!(
                result.status,
                UpdateStatus::Completed | UpdateStatus::Errored
            )
        {
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "helper never published a terminal result"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn prepared(root: &Path, behavior: &str, version: u64) -> (Preparation, BundleManifest) {
    let preparation = Preparation::new(root).unwrap();
    let new = bundle(
        &preparation.directory.path().join("stage"),
        behavior,
        version,
    );
    (preparation, new)
}

fn setup(
    behavior: &str,
) -> (
    tempfile::TempDir,
    PathBuf,
    BundleManifest,
    OwnedChild,
    PathBuf,
) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("teshi-install");
    let old = bundle(&root, behavior, 1);
    // The helper canonicalizes its transaction root, so use the same form for
    // per-installation diagnostics lookups.
    let root = fs::canonicalize(&root).unwrap();
    let executable = root.join(cli_name());
    let child = OwnedChild(spawn(&executable, &root));
    (temporary, root, old, child, executable)
}

#[test]
fn console_process_observes_the_exit_request_and_lets_the_helper_commit() {
    let _guard = serialize_helper_tests();
    unsafe { std::env::set_var("TESHI_UPDATE_QUIESCE_SECS", "5") };
    let (_temporary, root, old, mut child, _executable) = setup(POLL_EXIT_BODY);
    let (preparation, new) = prepared(&root, POLL_EXIT_BODY, 2);
    let result = preparation
        .handoff_with_close(&root, &old, &new, None, false)
        .unwrap();
    assert_eq!(result.status, UpdateStatus::WaitingForExit);
    // The helper publishes exit-request.json; the fixture exits by itself.
    let result = wait_for_result(&root);
    assert_eq!(result.status, UpdateStatus::Completed, "{}", result.detail);
    assert_eq!(
        fs::read_to_string(root.join("share/version.txt")).unwrap(),
        "2"
    );
    assert!(!child.running(), "the requested process exited");
    assert!(
        !root
            .join(".teshi-update")
            .join("exit-request.json")
            .exists(),
        "the guard removed the request"
    );
}

#[test]
fn window_less_blocker_fails_with_pid_path_and_force_help() {
    let _guard = serialize_helper_tests();
    unsafe { std::env::set_var("TESHI_UPDATE_QUIESCE_SECS", "1") };
    let (_temporary, root, old, mut child, _executable) = setup(BLOCKER_BODY);
    let (preparation, new) = prepared(&root, BLOCKER_BODY, 2);
    let result = preparation
        .handoff_with_close(&root, &old, &new, None, false)
        .unwrap();
    assert_eq!(result.status, UpdateStatus::WaitingForExit);
    let result = wait_for_result(&root);
    assert_eq!(result.status, UpdateStatus::Errored, "{}", result.detail);
    let failure = teshi_update::diagnostics::last_failure(&root).unwrap();
    assert_eq!(failure.stage, "process_exit");
    let pid = child.0.id().to_string();
    assert!(
        failure.error.message.contains(&pid),
        "missing PID {pid}: {}",
        failure.error.message
    );
    assert!(
        failure.error.message.contains("teshi update --force-close"),
        "missing the force-close next step: {}",
        failure.error.message
    );
    assert!(
        failure.next_attempt.contains("--force-close"),
        "{}",
        failure.next_attempt
    );
    assert!(child.running(), "without force the blocker survives");
    assert_eq!(
        fs::read_to_string(root.join("share/version.txt")).unwrap(),
        "1"
    );
}

#[test]
#[cfg(any(windows, target_os = "linux"))]
fn force_close_only_targets_the_verified_installation() {
    let _guard = serialize_helper_tests();
    unsafe { std::env::set_var("TESHI_UPDATE_QUIESCE_SECS", "1") };
    let (temporary, root, old, mut child, executable) = setup(BLOCKER_BODY);
    // Same file name, different installation: force close must not touch it.
    let unrelated = temporary.path().join("other-install");
    fs::create_dir_all(&unrelated).unwrap();
    let other = unrelated.join(cli_name());
    fs::copy(&executable, &other).unwrap();
    let mut other_child = OwnedChild(spawn(&other, &unrelated));
    let (preparation, new) = prepared(&root, BLOCKER_BODY, 2);
    let result = preparation
        .handoff_with_close(&root, &old, &new, None, true)
        .unwrap();
    assert_eq!(result.status, UpdateStatus::WaitingForExit);
    let result = wait_for_result(&root);
    assert_eq!(result.status, UpdateStatus::Completed, "{}", result.detail);
    assert!(!child.running(), "the verified generation was force-closed");
    assert!(other_child.running(), "another installation kept running");
    assert_eq!(
        fs::read_to_string(root.join("share/version.txt")).unwrap(),
        "2"
    );
    let _ = other_child.0.kill();
}

#[test]
#[cfg(not(any(windows, target_os = "linux")))]
fn force_close_reports_unsupported_on_non_pidfd_platforms() {
    let _guard = serialize_helper_tests();
    unsafe { std::env::set_var("TESHI_UPDATE_QUIESCE_SECS", "1") };
    let (_temporary, root, old, _child, _executable) = setup(BLOCKER_BODY);
    let (preparation, new) = prepared(&root, BLOCKER_BODY, 2);
    let result = preparation
        .handoff_with_close(&root, &old, &new, None, true)
        .unwrap();
    assert_eq!(result.status, UpdateStatus::WaitingForExit);
    let result = wait_for_result(&root);
    assert_eq!(result.status, UpdateStatus::Errored, "{}", result.detail);
    assert!(
        result
            .detail
            .contains("Safe force close is supported on Windows and Linux"),
        "unexpected failure detail: {}",
        result.detail
    );
}
