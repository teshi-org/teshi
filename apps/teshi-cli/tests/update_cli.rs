//! Update CLI parsing and automatic-operation contracts, without network requests.

use std::process::Command;

#[test]
fn update_help_exposes_check_status_channel_and_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_teshi"))
        .args(["update", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for flag in ["--check", "--status", "--channel", "--yes", "--json"] {
        assert!(help.contains(flag), "{flag}");
    }
    assert!(help.contains("installation is automatic"));
}

#[test]
fn local_status_and_identity_do_not_require_a_release_or_install_manifest() {
    for args in [
        vec!["update", "--status", "--json"],
        vec!["--update-identity"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_teshi"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(serde_json::from_slice::<serde_json::Value>(&output.stdout).is_ok());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Checking GitHub"));
    }
}

#[test]
fn status_rejects_install_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_teshi"))
        .args(["update", "--status", "--yes"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}
