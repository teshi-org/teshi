//! Real helper acceptance against disposable bundles; never touches installed Teshi.

use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
use teshi_core::version::{BUILD_TARGET, BuildIdentity, ReleaseChannel};
use teshi_update::{
    UpdateStatus,
    download::{file_hash, verify_bundle},
    manifest::{BUNDLE_MANIFEST, BundleManifest, InstallKind, ManagedFile},
    storage::write_json,
    transaction::{Preparation, last_result, participate},
};

fn fixture(root: &Path, sequence: u64, bad_identity: bool) -> BundleManifest {
    fs::create_dir_all(root).unwrap();
    let identity = BuildIdentity {
        semver: "0.7.10".into(),
        channel: ReleaseChannel::Nightly,
        git_sha: format!("{sequence:040x}"),
        build_timestamp: "2026-09-08T12:00:00Z".into(),
        build_sequence: sequence,
    };
    let cli = if cfg!(windows) { "teshi.exe" } else { "teshi" };
    let helper = if cfg!(windows) {
        "teshi-update-helper.exe"
    } else {
        "teshi-update-helper"
    };
    let source = root.join("fixture.rs");
    let response = if bad_identity {
        "{}".into()
    } else {
        serde_json::to_string(&identity).unwrap()
    };
    fs::write(
        &source,
        format!("fn main() {{ println!(\"{{}}\", r#\"{response}\"#); }}"),
    )
    .unwrap();
    assert!(
        Command::new("rustc")
            .arg(&source)
            .arg("--crate-name")
            .arg("update_fixture")
            .arg("-o")
            .arg(root.join(cli))
            .status()
            .unwrap()
            .success()
    );
    fs::remove_file(source).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_teshi-update-helper"), root.join(helper)).unwrap();
    let desktop = if cfg!(windows) {
        "teshi-desktop.exe"
    } else {
        "teshi-desktop"
    };
    fs::copy(root.join(cli), root.join(desktop)).unwrap();
    fs::create_dir(root.join("share")).unwrap();
    fs::write(root.join("share/version.txt"), sequence.to_string()).unwrap();
    let files = [cli, helper, desktop, "share/version.txt"]
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
    let bundle = BundleManifest {
        schema: 1,
        identity,
        target: BUILD_TARGET.into(),
        kind: InstallKind::Portable,
        layout: 1,
        update_explanation: None,
        files,
    };
    write_json(&root.join(BUNDLE_MANIFEST), &bundle).unwrap();
    bundle
}

fn run_case(bad_identity: bool) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("teshi-old-version-directory");
    let old = fixture(&root, 1, false);
    let cli = root.join(if cfg!(windows) { "teshi.exe" } else { "teshi" });
    let participant = participate(&cli).unwrap();
    let preparation = Preparation::new(&root).unwrap();
    let new = fixture(&preparation.directory.path().join("stage"), 2, bad_identity);
    fs::write(root.join("user.txt"), "preserved user file").unwrap();
    let result = preparation.handoff(&root, &old, &new, None).unwrap();
    assert_eq!(result.status, UpdateStatus::WaitingForExit);
    assert_eq!(
        fs::read_to_string(root.join("share/version.txt")).unwrap(),
        "1"
    );
    assert!(
        participate(&cli).is_err(),
        "helper must hold the gate before acknowledgement"
    );
    drop(participant);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = last_result(&root)
            && matches!(
                result.status,
                UpdateStatus::Completed | UpdateStatus::Errored
            )
        {
            assert_eq!(
                result.status,
                if bad_identity {
                    UpdateStatus::Errored
                } else {
                    UpdateStatus::Completed
                },
                "{}",
                result.detail
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "helper did not finish: {:?}",
            last_result(&root)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    verify_bundle(&root, if bad_identity { &old } else { &new }).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("user.txt")).unwrap(),
        "preserved user file"
    );
    // Wait for the helper to release its own executable before TempDir cleanup.
    let deadline = Instant::now() + Duration::from_secs(5);
    while root.join(".teshi-update/pending.json").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn real_helper_waits_for_exit_and_updates_complete_bundle() {
    run_case(false);
}

#[test]
fn real_helper_rolls_back_failed_new_binary_smoke_check() {
    run_case(true);
}

#[test]
fn real_helper_rolls_back_when_a_managed_file_is_locked() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("teshi-old-version-directory");
    let old = fixture(&root, 1, false);
    let cli = root.join(if cfg!(windows) { "teshi.exe" } else { "teshi" });
    let participant = participate(&cli).unwrap();
    let preparation = Preparation::new(&root).unwrap();
    let new = fixture(&preparation.directory.path().join("stage"), 2, false);
    #[cfg(windows)]
    let mut hold = Some({
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(root.join("share/version.txt"))
            .unwrap()
    });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Deny opening the file. chmod on the parent directory also blocks rollback
        // temp files, so the helper would die while still Installing.
        fs::set_permissions(
            root.join("share/version.txt"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
    }
    let result = preparation.handoff(&root, &old, &new, None).unwrap();
    assert_eq!(result.status, UpdateStatus::WaitingForExit);
    drop(participant);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(result) = last_result(&root)
            && result.status == UpdateStatus::Errored
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "helper did not roll back a locked file: {:?}",
            last_result(&root)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    #[cfg(windows)]
    drop(hold.take());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            root.join("share/version.txt"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
    }
    verify_bundle(&root, &old).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("share/version.txt")).unwrap(),
        "1"
    );
}

#[test]
fn competing_cli_cannot_stage_while_desktop_holds_preparation() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("teshi-old-version-directory");
    let _old = fixture(&root, 1, false);
    let cli = root.join(if cfg!(windows) { "teshi.exe" } else { "teshi" });
    let _desktop = participate(&cli).unwrap();
    let _preparation = Preparation::new(&root).unwrap();
    assert!(
        Preparation::new(&root).is_err(),
        "a second CLI/desktop updater must not share the install reservation"
    );
}

#[test]
fn helper_handoff_is_pending_not_installed_until_processes_exit() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("teshi-old-version-directory");
    let old = fixture(&root, 1, false);
    let cli = root.join(if cfg!(windows) { "teshi.exe" } else { "teshi" });
    let desktop = participate(&cli).unwrap();
    let preparation = Preparation::new(&root).unwrap();
    let new = fixture(&preparation.directory.path().join("stage"), 2, false);
    let result = preparation.handoff(&root, &old, &new, None).unwrap();
    assert_eq!(result.status, UpdateStatus::WaitingForExit);
    assert!(!teshi_update::lifecycle::replacement_committed(
        result.status
    ));
    assert_eq!(
        last_result(&root).unwrap().status,
        UpdateStatus::WaitingForExit
    );
    assert_eq!(
        teshi_update::lifecycle::desktop_handoff(&result, false),
        teshi_update::lifecycle::DesktopHandoff::KeepOpen
    );
    drop(desktop);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = last_result(&root)
            && result.status == UpdateStatus::Completed
        {
            assert!(teshi_update::lifecycle::replacement_committed(
                result.status
            ));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "helper did not finish: {:?}",
            last_result(&root)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    verify_bundle(&root, &new).unwrap();
}

#[test]
fn active_daemon_blocks_replacement_without_false_installed_status() {
    // Isolated helper test process; no concurrent env writers in this file besides this test.
    unsafe {
        std::env::set_var("TESHI_UPDATE_QUIESCE_SECS", "1");
    }
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("teshi-old-version-directory");
    let old = fixture(&root, 1, false);
    let cli = root.join(if cfg!(windows) { "teshi.exe" } else { "teshi" });
    let desktop = participate(&cli).unwrap();
    let daemon = participate(&cli).unwrap();
    let preparation = Preparation::new(&root).unwrap();
    let new = fixture(&preparation.directory.path().join("stage"), 2, false);
    let pending = preparation.handoff(&root, &old, &new, None).unwrap();
    assert_eq!(pending.status, UpdateStatus::WaitingForExit);
    drop(desktop);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(result) = last_result(&root)
            && result.status == UpdateStatus::Errored
        {
            assert!(!teshi_update::lifecycle::replacement_committed(
                result.status
            ));
            assert_eq!(
                teshi_update::lifecycle::desktop_handoff(&result, true),
                teshi_update::lifecycle::DesktopHandoff::Report
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "helper did not fail while a daemon participant was held: {:?}",
            last_result(&root)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    verify_bundle(&root, &old).unwrap();
    drop(daemon);
}
