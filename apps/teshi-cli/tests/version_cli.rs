//! Integration coverage for the public CLI version output.

use std::process::Command;

#[test]
fn version_flag_prints_only_the_product_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_teshi"))
        .arg("-V")
        .output()
        .expect("run teshi -V");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version output is UTF-8"),
        format!("{}\n", teshi_core::version::version_display())
    );
    assert!(output.stderr.is_empty());
}
