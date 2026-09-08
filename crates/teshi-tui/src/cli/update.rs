//! Thin terminal adapter for the native update core.

use std::{
    io::{self, IsTerminal, Write},
    sync::atomic::AtomicBool,
};
use teshi_core::version::{ReleaseChannel, build_identity};
use teshi_update::{ErrorCode, UpdateError, manager};

/// Runs the update CLI, reserving exit 3 for an accepted helper handoff.
pub fn handle_update(
    status_only: bool,
    check_only: bool,
    channel: Option<&str>,
    yes: bool,
    json: bool,
) -> anyhow::Result<()> {
    let result = (|| -> teshi_update::Result<(serde_json::Value, i32)> {
        if status_only {
            let installation = teshi_update::install::Installation::detect(
                &std::env::current_exe()?,
                &build_identity(),
            )?;
            let previous = installation
                .root
                .as_deref()
                .and_then(teshi_update::transaction::last_result);
            return Ok((
                serde_json::json!({"status":"local_status", "installation":installation, "previous_transaction":previous}),
                0,
            ));
        }
        if !check_only && !yes && (json || !io::stdin().is_terminal()) {
            return Err(UpdateError::new(
                ErrorCode::Cancelled,
                "Installation requires --yes when input is non-interactive or --json is set",
            ));
        }
        let channel = channel.map(|value| {
            if value == "nightly" {
                ReleaseChannel::Nightly
            } else {
                ReleaseChannel::Stable
            }
        });
        let report = manager::check(&std::env::current_exe()?, build_identity(), channel)?;
        if check_only || report.candidate.is_none() {
            return Ok((serde_json::to_value(report)?, 0));
        }
        if !report.installation.can_install() {
            return Err(UpdateError::new(
                ErrorCode::Unsupported,
                report.installation.explanation.clone().unwrap_or_default(),
            ));
        }
        if !yes {
            let tag = report
                .candidate
                .as_ref()
                .map(|c| c.manifest.tag.as_str())
                .unwrap_or_default();
            eprint!("Install {tag}? Close other Teshi processes before continuing. [y/N] ");
            io::stderr().flush()?;
            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                return Err(UpdateError::new(ErrorCode::Cancelled, "Update cancelled"));
            }
        }
        let mut previous_percent = None;
        let transaction = manager::install(&report, &AtomicBool::new(false), &mut |event| {
            let percent = event.progress.map(|p| (p * 100.0) as u32);
            if percent != previous_percent || percent.is_none() {
                eprintln!(
                    "{:?} {}{}",
                    event.status,
                    event.detail,
                    percent.map(|p| format!(" {p}%")).unwrap_or_default()
                );
                previous_percent = percent;
            }
        })?;
        Ok((
            serde_json::json!({
                "status":"pending",
                "installed": false,
                "current":report.current,
                "candidate":report.candidate,
                "installation":report.installation,
                "transaction":transaction
            }),
            3,
        ))
    })();
    match result {
        Ok((value, code)) => {
            if json {
                println!("{}", serde_json::to_string(&value)?);
            } else {
                println!(
                    "{}",
                    value
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("completed")
                );
                if let Some(candidate) = value.get("candidate").filter(|c| !c.is_null()) {
                    println!(
                        "Release: {}",
                        candidate["manifest"]["tag"].as_str().unwrap_or_default()
                    );
                    println!("{}", candidate["release_url"].as_str().unwrap_or_default());
                }
                if let Some(reason) = value
                    .pointer("/installation/explanation")
                    .and_then(|v| v.as_str())
                {
                    println!("{reason}");
                }
                if let Some(previous) = value.get("previous_transaction").filter(|v| !v.is_null()) {
                    println!(
                        "Previous update: {}",
                        previous["detail"].as_str().unwrap_or_default()
                    );
                }
                if code == 3 {
                    println!(
                        "Update is pending. The helper will install after all Teshi processes exit; check the result on the next invocation."
                    );
                }
            }
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Err(error) => {
            if json {
                println!("{}", serde_json::json!({"status":"errored", "error":error}));
            } else {
                eprintln!("Update failed: {error}");
            }
            std::process::exit(1);
        }
    }
}
