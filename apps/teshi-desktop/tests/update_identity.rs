//! Desktop identity probe used by the updater; must not open a GPUI window.

use std::process::Command;

#[test]
fn update_identity_prints_json_without_starting_the_shell() {
    let output = Command::new(env!("CARGO_BIN_EXE_teshi-desktop"))
        .arg("--update-identity")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value.get("semver").and_then(|v| v.as_str()).is_some());
    assert!(value.get("channel").and_then(|v| v.as_str()).is_some());
}
