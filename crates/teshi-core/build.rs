//! Inject build identity into the compiled binary.
//!
//! CI injects a published identity through the `TESHI_BUILD_*` environment
//! variables. When none of them are set the build is an unidentified local
//! source build, so derive a complete `local` identity from the checkout and
//! the clock. `teshi -V` then reports a local build instead of an anonymous
//! development build, and the updater treats it as a stream that is
//! deliberately not published.
//!
//! The fallback is all-or-nothing on purpose: a partially configured CI
//! environment keeps failing closed to `dev` rather than silently becoming a
//! credible-looking `local` identity.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Environment variables CI injects to identify a published build.
const IDENTITY_KEYS: [&str; 6] = [
    "TESHI_BUILD_CHANNEL",
    "TESHI_BUILD_DATE",
    "TESHI_GIT_SHA",
    "TESHI_BUILD_TIMESTAMP",
    "TESHI_BUILD_LOCAL_TIMESTAMP",
    "TESHI_BUILD_SEQUENCE",
];

fn main() {
    println!("cargo:rerun-if-env-changed=TESHI_NIGHTLY_NUMBER");
    for key in IDENTITY_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
    }
    if let Ok(target) = std::env::var("TARGET") {
        println!("cargo:rustc-env=TESHI_TARGET={target}");
    }
    // Any injected variable means a caller is asserting identity; leave it alone.
    if IDENTITY_KEYS
        .iter()
        .any(|key| std::env::var_os(key).is_some())
    {
        return;
    }
    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return;
    };
    let seconds = now.as_secs();
    let sha = head_sha();
    // Without a source commit there is nothing to identify; stay anonymous.
    if sha.is_empty() {
        return;
    }
    let identity = [
        ("TESHI_BUILD_CHANNEL", "local".to_string()),
        ("TESHI_GIT_SHA", sha),
        (
            "TESHI_BUILD_TIMESTAMP",
            stamp(seconds, "%Y-%m-%dT%H:%M:%SZ", false),
        ),
        ("TESHI_BUILD_DATE", stamp(seconds, "%Y%m%d", false)),
        ("TESHI_BUILD_SEQUENCE", seconds.to_string()),
        (
            "TESHI_BUILD_LOCAL_TIMESTAMP",
            stamp(seconds, "%Y-%m-%dT%H:%M:%S%:z", true),
        ),
    ];
    for (key, value) in identity {
        if !value.is_empty() {
            println!("cargo:rustc-env={key}={value}");
        }
    }
}

/// Full source commit, or empty when the checkout is unavailable.
fn head_sha() -> String {
    let Ok(output) = Command::new("git").args(["rev-parse", "HEAD"]).output() else {
        return String::new();
    };
    if !output.status.success() {
        return String::new();
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        value
    } else {
        String::new()
    }
}

/// Formats the build instant in UTC, or in the build machine's zone with offset.
fn stamp(seconds: u64, format: &str, local: bool) -> String {
    let Some(instant) = chrono::DateTime::from_timestamp(seconds as i64, 0) else {
        return String::new();
    };
    if local {
        instant
            .with_timezone(&chrono::Local)
            .format(format)
            .to_string()
    } else {
        instant.format(format).to_string()
    }
}
