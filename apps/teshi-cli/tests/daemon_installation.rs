//! Daemon runtime and updater registration contracts for isolated installations.

#![cfg(any(windows, target_os = "linux"))]

use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use teshi_core::version::{BUILD_TARGET, BuildIdentity};
use teshi_engine::{DaemonManifest, DaemonManifestExt};
use teshi_update::{
    download::file_hash,
    install::Installation,
    manifest::{BUNDLE_MANIFEST, BundleManifest, InstallKind, ManagedFile, shipped_executable},
    processes::{self, DaemonEndpoint},
};

struct OwnedProcess(Child);
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Fixture {
    temporary: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
    cli: PathBuf,
}

impl Fixture {
    fn new(kind: InstallKind) -> Self {
        let temporary = tempfile::Builder::new()
            .prefix("teshi-daemon-installation-")
            .tempdir()
            .unwrap();
        let root = temporary.path().join("install");
        let project = temporary.path().join("project");
        fs::create_dir_all(project.join(".teshi")).unwrap();
        let cli = shipped_executable(
            &root,
            kind,
            &format!("teshi{}", std::env::consts::EXE_SUFFIX),
        );
        fs::create_dir_all(cli.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_teshi"), &cli).unwrap();
        if kind != InstallKind::Unknown {
            let output = Command::new(&cli)
                .arg("--update-identity")
                .output()
                .unwrap();
            assert!(output.status.success());
            let identity: BuildIdentity = serde_json::from_slice(&output.stdout).unwrap();
            let helper = shipped_executable(
                &root,
                kind,
                &format!("teshi-update-helper{}", std::env::consts::EXE_SUFFIX),
            );
            fs::write(&helper, b"unused helper fixture").unwrap();
            let bundle = BundleManifest {
                schema: 1,
                layout: 1,
                target: BUILD_TARGET.into(),
                identity: identity.clone(),
                kind,
                update_explanation: None,
                files: [&cli, &helper]
                    .into_iter()
                    .map(|path| ManagedFile {
                        path: path
                            .strip_prefix(&root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                        size: path.metadata().unwrap().len(),
                        sha256: file_hash(path).unwrap(),
                        executable: true,
                    })
                    .collect(),
            };
            bundle.validate().unwrap();
            fs::write(
                root.join(BUNDLE_MANIFEST),
                serde_json::to_vec(&bundle).unwrap(),
            )
            .unwrap();
            let installation = Installation::detect(&cli, &identity).unwrap();
            assert_eq!(installation.kind, kind);
            assert_eq!(
                installation.can_install(),
                kind == InstallKind::Exe && cfg!(windows)
            );
        }
        fs::create_dir_all(temporary.path().join("app-data")).unwrap();
        Self {
            temporary,
            root,
            project,
            cli,
        }
    }

    fn start(&self) -> OwnedProcess {
        let mut command = self.command();
        command
            .args(["--daemon-internal", "--project-root"])
            .arg(&self.project)
            .args(["--port", "0", "--host", "127.0.0.1"])
            .current_dir(&self.project)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(self.project.join(".teshi/launch.stderr")).unwrap());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        OwnedProcess(command.spawn().unwrap())
    }

    fn log(&self) -> String {
        format!(
            "{}\n{}",
            fs::read_to_string(self.project.join(".teshi/logs/daemon.log")).unwrap_or_default(),
            fs::read_to_string(self.project.join(".teshi/launch.stderr")).unwrap_or_default(),
        )
    }

    fn ready(&self, daemon: &mut OwnedProcess) {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            assert!(
                daemon.0.try_wait().unwrap().is_none(),
                "daemon exited: {}",
                self.log()
            );
            if let Some(manifest) = DaemonManifest::load_manifest(&self.project)
                && let Ok(response) = client
                    .get(format!(
                        "http://127.0.0.1:{}/api/v1/daemon/process",
                        manifest.port
                    ))
                    .send()
                && response.status().is_success()
            {
                let endpoint: DaemonEndpoint = response.json().unwrap();
                assert_eq!(
                    endpoint.process,
                    processes::inspect(daemon.0.id()).unwrap().unwrap()
                );
                assert_eq!(
                    endpoint.project_root,
                    Some(self.project.canonicalize().unwrap())
                );
                assert_eq!(endpoint.port, manifest.port);
                return;
            }
            assert!(
                Instant::now() < deadline,
                "daemon readiness timed out: {}",
                self.log()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_for_exit(&self, daemon: &mut OwnedProcess) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = daemon.0.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "daemon did not exit: {}",
                self.log()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn stop(&self, daemon: &mut OwnedProcess) {
        let output = self
            .command()
            .args(["daemon", "stop"])
            .current_dir(&self.project)
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(self.wait_for_exit(daemon).success(), "{}", self.log());
        assert!(!self.project.join(".teshi/daemon.json").exists());
    }

    fn open_web_without_browser(&self) {
        let output = self
            .command()
            .args(["web", "--no-open", "--project"])
            .arg(&self.project)
            .current_dir(&self.project)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.cli);
        command.env("TESHI_APP_DATA_DIR", self.temporary.path().join("app-data"));
        command
    }
}

#[test]
fn check_only_installations_run_without_updater_registration() {
    for kind in [
        InstallKind::Msi,
        InstallKind::Portable,
        InstallKind::External,
        InstallKind::Unknown,
    ] {
        let fixture = Fixture::new(kind);
        // Block endpoint registration without preventing the legacy participant gate.
        fs::create_dir(fixture.root.join(".teshi-update")).unwrap();
        let blocker = fixture.root.join(".teshi-update/daemons");
        fs::write(&blocker, b"preserve me").unwrap();
        let mut daemon = fixture.start();
        fixture.ready(&mut daemon);
        fixture.open_web_without_browser();
        fixture.stop(&mut daemon);
        assert_eq!(fs::read(&blocker).unwrap(), b"preserve me");
    }
}

#[test]
#[cfg(windows)]
fn setup_installation_registers_and_removes_its_endpoint() {
    let fixture = Fixture::new(InstallKind::Exe);
    let mut daemon = fixture.start();
    fixture.ready(&mut daemon);
    let directory = fixture.root.join(".teshi-update/daemons");
    let paths: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(paths.len(), 1);
    let endpoint: DaemonEndpoint = serde_json::from_slice(&fs::read(&paths[0]).unwrap()).unwrap();
    assert_eq!(
        endpoint.process,
        processes::inspect(daemon.0.id()).unwrap().unwrap()
    );
    fixture.stop(&mut daemon);
    assert_eq!(fs::read_dir(directory).unwrap().count(), 0);
}

#[test]
#[cfg(windows)]
fn failed_setup_registration_cleans_project_manifest_and_reports_failure() {
    let fixture = Fixture::new(InstallKind::Exe);
    fs::create_dir(fixture.root.join(".teshi-update")).unwrap();
    fs::write(fixture.root.join(".teshi-update/daemons"), b"preserve me").unwrap();
    let mut daemon = fixture.start();
    assert!(!fixture.wait_for_exit(&mut daemon).success());
    assert!(!fixture.project.join(".teshi/daemon.json").exists());
    assert_eq!(
        fs::read(fixture.root.join(".teshi-update/daemons")).unwrap(),
        b"preserve me"
    );
    assert!(
        fixture.log().contains("daemon listening"),
        "manifest must have been published"
    );
    assert!(fixture.log().contains("daemon server"), "{}", fixture.log());
}

#[cfg(windows)]
struct ReadOnlyInstallation(PathBuf);

#[cfg(windows)]
impl ReadOnlyInstallation {
    fn new(root: &std::path::Path) -> Self {
        let guard = Self(root.canonicalize().unwrap());
        let output = Command::new("icacls")
            .arg(&guard.0)
            .args(["/deny", "*S-1-1-0:(OI)(CI)(WD,AD,WEA,WA)", "/Q"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::create_dir(guard.0.join("write-probe"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        guard
    }
}

#[cfg(windows)]
impl Drop for ReadOnlyInstallation {
    fn drop(&mut self) {
        let output = Command::new("icacls")
            .arg(&self.0)
            .args(["/remove:d", "*S-1-1-0", "/Q"])
            .output()
            .expect("restore permissions on the disposable installation");
        assert!(
            output.status.success(),
            "failed to restore disposable installation ACL"
        );
    }
}

#[test]
#[cfg(windows)]
fn msi_daemon_runs_from_a_read_only_installation() {
    let fixture = Fixture::new(InstallKind::Msi);
    let _permissions = ReadOnlyInstallation::new(&fixture.root);
    let mut daemon = fixture.start();
    fixture.ready(&mut daemon);
    fixture.open_web_without_browser();
    fixture.stop(&mut daemon);
    assert!(!fixture.root.join(".teshi-update").exists());
}
