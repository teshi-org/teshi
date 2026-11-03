//! Opt-in Windows acceptance: real sockets, Inno Setup, CLI and helper processes.

use super::*;
use crate::{
    download::{file_hash, verify_bundle},
    manifest::{BundleManifest, ManagedFile, ReleaseAsset, ReleaseManifest},
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

struct LoopbackHttp(reqwest::blocking::Client);
impl Http for LoopbackHttp {
    fn get(&self, url: &str, _: Option<&str>) -> Result<HttpResponse> {
        self.request(url, None)
    }
    fn get_range(&self, url: &str, start: u64) -> Result<HttpResponse> {
        self.request(url, Some(start))
    }
}
impl LoopbackHttp {
    fn request(&self, url: &str, start: Option<u64>) -> Result<HttpResponse> {
        let mut request = self.0.get(url);
        if let Some(start) = start {
            request = request.header("Range", format!("bytes={start}-"));
        }
        let response = request.send().unwrap();
        Ok(HttpResponse {
            status: response.status().as_u16(),
            etag: None,
            retry_after: None,
            content_range: response
                .headers()
                .get("content-range")
                .map(|v| v.to_str().unwrap().into()),
            body: Box::new(response),
        })
    }
}

fn inventory(root: &Path, identity: BuildIdentity) -> BundleManifest {
    let files = [
        "bin/teshi.exe",
        "bin/teshi-update-helper.exe",
        "share/version.txt",
    ]
    .into_iter()
    .map(|name| {
        let path = root.join(name);
        ManagedFile {
            path: name.into(),
            size: path.metadata().unwrap().len(),
            sha256: file_hash(&path).unwrap(),
            executable: name.ends_with(".exe"),
        }
    })
    .collect();
    let manifest = BundleManifest {
        schema: 1,
        layout: 1,
        identity,
        target: BUILD_TARGET.into(),
        kind: InstallKind::Exe,
        update_explanation: None,
        files,
    };
    manifest.validate().unwrap();
    write_json(&root.join(BUNDLE_MANIFEST), &manifest).unwrap();
    manifest
}

fn run_update(cli: &Path, args: &[&str]) -> std::process::Output {
    run_update_with_wait(cli, args, "30")
}

fn run_update_with_wait(cli: &Path, args: &[&str], seconds: &str) -> std::process::Output {
    // A broken proxy ensures an accidental network call cannot succeed offline.
    Command::new(cli)
        .arg("update")
        .args(args)
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("TESHI_UPDATE_QUIESCE_SECS", seconds)
        .output()
        .unwrap()
}

struct OwnedProcess(std::process::Child);
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_owned(cli: &Path, args: &[&str], project: &Path) -> OwnedProcess {
    use std::{os::windows::process::CommandExt, process::Stdio};
    let mut command = Command::new(cli);
    command
        .args(args)
        .current_dir(project)
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = OwnedProcess(command.spawn().unwrap());
    thread::sleep(Duration::from_millis(150));
    assert!(
        child.0.try_wait().unwrap().is_none(),
        "owned process exited before acceptance"
    );
    child
}

fn spawn_daemon(cli: &Path, project: &Path) -> OwnedProcess {
    fs::create_dir_all(project.join(".teshi")).unwrap();
    let mut daemon = spawn_owned(
        cli,
        &[
            "--daemon-internal",
            "--project-root",
            project.to_str().unwrap(),
            "--port",
            "0",
            "--host",
            "127.0.0.1",
        ],
        project,
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !project.join(".teshi/daemon.json").is_file() {
        assert!(
            daemon.0.try_wait().unwrap().is_none(),
            "daemon exited before readiness"
        );
        assert!(Instant::now() < deadline, "daemon readiness timed out");
        thread::sleep(Duration::from_millis(50));
    }
    // Endpoint registration occurs just after publishing the project manifest.
    thread::sleep(Duration::from_millis(100));
    daemon
}

fn await_helper(root: &Path, expected: UpdateStatus) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if !root.join(".teshi-update/pending.json").exists()
            && let Some(result) = last_result(root)
            && matches!(
                result.status,
                UpdateStatus::Completed | UpdateStatus::Errored
            )
        {
            assert_eq!(result.status, expected, "{result:?}");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "helper timed out: {:?}",
            last_result(root)
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires built CLI/helper and Inno Setup; see doc/development.md"]
fn interrupted_download_setup_failure_offline_retry_and_helper_rollback() {
    let name =
        "manager::acceptance::interrupted_download_setup_failure_offline_retry_and_helper_rollback";
    if std::env::var_os("TESHI_UPDATE_ACCEPTANCE_CHILD").is_none() {
        let temporary = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--ignored", "--nocapture"])
            .env("TESHI_UPDATE_ACCEPTANCE_CHILD", "1")
            .env("LOCALAPPDATA", temporary.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{}", String::from_utf8_lossy(&output.stdout));
        return;
    }
    let cli_source = PathBuf::from(
        std::env::var_os("TESHI_UPDATE_CLI_TEST_BIN")
            .expect("set TESHI_UPDATE_CLI_TEST_BIN to built teshi.exe"),
    );
    let helper_source = PathBuf::from(
        std::env::var_os("TESHI_UPDATE_HELPER_TEST_BIN")
            .expect("set TESHI_UPDATE_HELPER_TEST_BIN to built teshi-update-helper.exe"),
    );
    let iscc = PathBuf::from(r"C:\Program Files (x86)\Inno Setup 6\ISCC.exe");
    assert!(iscc.is_file(), "Inno Setup 6 is required");
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("disposable-install");
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(root.join("share")).unwrap();
    fs::copy(&cli_source, root.join("bin/teshi.exe")).unwrap();
    fs::copy(&helper_source, root.join("bin/teshi-update-helper.exe")).unwrap();
    fs::write(root.join("share/version.txt"), "old").unwrap();
    let cli = root.join("bin/teshi.exe");
    let current: BuildIdentity = serde_json::from_slice(
        &Command::new(&cli)
            .arg("--update-identity")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let old = inventory(&root, current.clone());
    let root = fs::canonicalize(root).unwrap();
    let cli = root.join("bin/teshi.exe");
    fs::write(root.join("user.txt"), "preserve me").unwrap();
    let new_root = temporary.path().join("new-bundle");
    fs::create_dir_all(new_root.join("bin")).unwrap();
    fs::create_dir_all(new_root.join("share")).unwrap();
    let identity = BuildIdentity {
        semver: current.semver.clone(),
        channel: ReleaseChannel::Nightly,
        git_sha: "b".repeat(40),
        build_timestamp: "2026-11-03T00:00:00Z".into(),
        build_sequence: current.build_sequence + 1,
    };
    let source = temporary.path().join("fixture.rs");
    let raw = serde_json::to_string(&identity).unwrap();
    fs::write(&source, format!(r##"fn main() {{
        let exe = std::env::current_exe().unwrap();
        if exe.parent().unwrap().parent().unwrap().join("fail-smoke").exists() {{ println!("{{}}", "{{}}"); }}
        else {{ println!("{{}}", r#"{raw}"#); }}
    }}"##)).unwrap();
    assert!(
        Command::new("rustc")
            .arg(&source)
            .args(["--crate-name", "update_acceptance", "-o"])
            .arg(new_root.join("bin/teshi.exe"))
            .status()
            .unwrap()
            .success()
    );
    fs::copy(&helper_source, new_root.join("bin/teshi-update-helper.exe")).unwrap();
    fs::write(new_root.join("share/version.txt"), "new").unwrap();
    let new = inventory(&new_root, identity.clone());
    let tag = format!("v{}-nightly.20261103.bbbbbbb", current.semver);
    let asset_name = format!("teshi-{tag}-x64-setup");
    // Exercise actual Inno execution while excluding registry, shortcuts and
    // uninstall registration. Only the disposable /DIR install tree is touched.
    let script = temporary.path().join("fixture.iss");
    fs::write(&script, format!(r#"[Setup]
AppName=Teshi update acceptance
AppVersion=1.0.0
DefaultDirName={{localappdata}}\unused-acceptance-default
PrivilegesRequired=lowest
Uninstallable=no
CreateUninstallRegKey=no
CloseApplications=no
OutputDir={}
OutputBaseFilename={asset_name}
Compression=none
DisableDirPage=yes
[Files]
Source: "{}\*"; DestDir: "{{app}}\install"; Flags: ignoreversion recursesubdirs createallsubdirs
[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  if FileExists(ExpandConstant('{{app}}\fail-setup')) then Result := 'Injected setup failure' else Result := '';
end;
"#, temporary.path().display(), new_root.display())).unwrap();
    let compiled = Command::new(iscc).arg("/Q").arg(&script).output().unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stdout)
    );
    let package = temporary.path().join(format!("{asset_name}.exe"));
    let bytes = fs::read(&package).unwrap();
    let size = bytes.len();
    let half = size / 2;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/payload", listener.local_addr().unwrap());
    let transfer = thread::spawn(move || {
        let mut requests = Vec::new();
        for offset in [0, half] {
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
            requests.push(String::from_utf8(request).unwrap());
            let header = if offset == 0 {
                format!("HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n")
            } else {
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {half}-{}/{size}\r\nConnection: close\r\n\r\n",
                    size - half,
                    size - 1
                )
            };
            stream.write_all(header.as_bytes()).unwrap();
            stream
                .write_all(if offset == 0 {
                    &bytes[..half]
                } else {
                    &bytes[half..]
                })
                .unwrap();
        }
        requests
    });
    let asset = ReleaseAsset {
        name: format!("{asset_name}.exe"),
        target: BUILD_TARGET.into(),
        kind: InstallKind::Exe,
        size: size as u64,
        sha256: file_hash(&package).unwrap(),
    };
    let candidate = Candidate {
        release_id: 991,
        asset_id: 992,
        download_url: url,
        release_url: "https://github.com/teshi-org/teshi/releases/test".into(),
        manifest: ReleaseManifest {
            schema: 1,
            minimum_updater: 1,
            identity,
            tag,
            assets: vec![asset.clone()],
        },
        asset,
    };
    let installation = Installation::detect(&cli, &current).unwrap();
    let report = CheckResult {
        status: UpdateStatus::UpdateAvailable,
        current,
        candidate: Some(candidate),
        installation,
        previous_transaction: None,
    };
    fs::write(root.join("fail-setup"), "").unwrap();
    let http = LoopbackHttp(
        reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap(),
    );
    let mut checked_reservation = false;
    let error = install_impl(
        &report,
        &AtomicBool::new(false),
        &mut |_| {
            if !checked_reservation {
                assert!(
                    matches!(Preparation::new(&root), Err(error) if error.code == ErrorCode::Busy)
                );
                checked_reservation = true;
            }
        },
        InstallOptions::default(),
        false,
        &http,
        &state_directory().unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Installation);
    let requests = transfer.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1]
            .to_ascii_lowercase()
            .contains(&format!("range: bytes={half}-"))
    );
    let failure = crate::diagnostics::last_failure(&root).unwrap();
    assert_eq!(failure.stage, "setup");
    assert!(failure.cached_payload);
    assert!(!root.join(".teshi-update/pending.json").exists());
    assert!(
        !root.join("install").exists(),
        "failed setup staging must be removed"
    );
    verify_bundle(&root, &old).unwrap();
    let status = run_update(&cli, &["--status", "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["last_failure"]["stage"], "setup");
    fs::remove_file(root.join("fail-setup")).unwrap();
    fs::write(root.join("fail-smoke"), "").unwrap();
    let project = temporary.path().join("daemon-project");
    let mut daemon = spawn_daemon(&cli, &project);
    {
        let _gate =
            crate::storage::StateLock::acquire(&root.join(".teshi-update/gate.lock")).unwrap();
        assert!(
            !run_update(&cli, &["--retry-install", "--json"])
                .status
                .success(),
            "a competing update must be refused while the gate is held"
        );
        let stopped = Command::new(&cli)
            .args(["daemon", "stop"])
            .current_dir(&project)
            .output()
            .unwrap();
        assert!(
            stopped.status.success(),
            "daemon stop must work under the helper gate: {}",
            String::from_utf8_lossy(&stopped.stderr)
        );
        assert!(daemon.0.wait().unwrap().success());
        assert!(!project.join(".teshi/daemon.json").exists());
    }
    let mut daemon = spawn_daemon(&cli, &project);
    let retry = run_update(&cli, &["--retry-install", "--json"]);
    assert_eq!(
        retry.status.code(),
        Some(3),
        "{}\n{}",
        String::from_utf8_lossy(&retry.stdout),
        String::from_utf8_lossy(&retry.stderr)
    );
    await_helper(&root, UpdateStatus::Errored);
    assert!(
        daemon.0.wait().unwrap().success(),
        "update must stop its daemon gracefully"
    );
    assert!(!project.join(".teshi/daemon.json").exists());
    verify_bundle(&root, &old).unwrap();
    let failure = crate::diagnostics::last_failure(&root).unwrap();
    assert_eq!(failure.stage, "commit", "{failure:?}");
    fs::remove_file(root.join("fail-smoke")).unwrap();
    let mut blocker = spawn_owned(
        &cli,
        &[
            "mcp",
            "serve",
            "--stdio",
            "--project",
            project.to_str().unwrap(),
        ],
        &project,
    );
    let blocker_id = crate::processes::inspect(blocker.0.id()).unwrap().unwrap();
    let status = run_update(&cli, &["--status", "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(
        value["running_processes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|process| process["pid"].as_u64() == Some(blocker_id.pid as u64))
    );
    let unrelated_root = temporary.path().join("unrelated-install");
    fs::create_dir(&unrelated_root).unwrap();
    let unrelated_cli = unrelated_root.join("teshi.exe");
    fs::copy(&cli_source, &unrelated_cli).unwrap();
    let mut unrelated = spawn_owned(
        &unrelated_cli,
        &[
            "mcp",
            "serve",
            "--stdio",
            "--project",
            project.to_str().unwrap(),
        ],
        &project,
    );
    let retry = run_update_with_wait(&cli, &["--retry-install", "--json"], "1");
    assert_eq!(
        retry.status.code(),
        Some(3),
        "{}\n{}",
        String::from_utf8_lossy(&retry.stdout),
        String::from_utf8_lossy(&retry.stderr)
    );
    await_helper(&root, UpdateStatus::Errored);
    assert!(
        blocker.0.try_wait().unwrap().is_none(),
        "default update must not force-close MCP"
    );
    let failure = crate::diagnostics::last_failure(&root).unwrap();
    assert_eq!(failure.stage, "process_exit");
    assert!(failure.error.message.contains(&blocker_id.pid.to_string()));
    assert!(failure.error.message.contains("--force-close"));
    verify_bundle(&root, &old).unwrap();
    let retry = run_update_with_wait(&cli, &["--retry-install", "--force-close", "--json"], "1");
    assert_eq!(
        retry.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    await_helper(&root, UpdateStatus::Completed);
    assert!(
        blocker.0.try_wait().unwrap().is_some(),
        "explicit force close must confirm MCP exit"
    );
    assert!(
        unrelated.0.try_wait().unwrap().is_none(),
        "same-named process from another directory must survive"
    );
    verify_bundle(&root, &new).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("user.txt")).unwrap(),
        "preserve me"
    );
    assert!(crate::diagnostics::last_failure(&root).is_none());
    println!(
        "PASS: interrupted TCP transfer resumed at {half}/{size}; setup failure retained cache; daemon stop worked under helper gate; update gracefully stopped its daemon; MCP blocker was reported without force; --force-close ended only the verified installation MCP and preserved a same-named unrelated process; offline retries required no server; smoke failure rolled back; final commit preserved user files"
    );
}
