//! Thin terminal adapter for the native update core.

use std::sync::atomic::AtomicBool;
use teshi_core::version::{ReleaseChannel, build_identity};
use teshi_update::manager;

/// Runs the update CLI, reserving exit 3 for an accepted helper handoff.
pub fn handle_update(
    status_only: bool,
    check_only: bool,
    retry_install: bool,
    force_close: bool,
    channel: Option<&str>,
    _yes: bool,
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
            let executable = std::env::current_exe()?;
            let key = installation.root.as_deref().unwrap_or(&executable);
            let failure = teshi_update::diagnostics::last_failure(key);
            let running = if let (Some(root), Some(bundle)) =
                (installation.root.as_deref(), installation.bundle.as_ref())
            {
                teshi_update::processes::installation_processes(root, bundle)?
            } else {
                teshi_update::processes::processes_for_images(std::slice::from_ref(&executable))?
            };
            return Ok((
                serde_json::json!({"status":"local_status", "installation":installation, "previous_transaction":previous, "last_failure":failure, "running_processes":running}),
                0,
            ));
        }
        let channel = channel.map(|value| {
            if value == "nightly" {
                ReleaseChannel::Nightly
            } else {
                ReleaseChannel::Stable
            }
        });
        let mut previous_percent = None;
        let mut emit = |event: teshi_update::UpdateEvent| {
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
        };
        let (report, transaction) = if retry_install {
            eprintln!("Revalidating cached update (offline)...");
            manager::retry_install_with_close(
                &std::env::current_exe()?,
                build_identity(),
                &AtomicBool::new(false),
                &mut emit,
                force_close,
            )?
        } else {
            eprintln!("Checking GitHub for updates...");
            let report = manager::check(&std::env::current_exe()?, build_identity(), channel)?;
            if check_only || report.candidate.is_none() {
                return Ok((serde_json::to_value(report)?, 0));
            }
            let transaction = manager::install_with_options(
                &report,
                &AtomicBool::new(false),
                &mut emit,
                manager::InstallOptions {
                    force_close,
                    ..Default::default()
                },
            )?;
            (report, transaction)
        };
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
                if let Some(failure) = value.get("last_failure").filter(|v| !v.is_null()) {
                    println!(
                        "Last failure [{}/{}]: {}",
                        failure["stage"].as_str().unwrap_or_default(),
                        failure["error"]["code"].as_str().unwrap_or_default(),
                        failure["error"]["message"].as_str().unwrap_or_default()
                    );
                    if let Some(payload) = failure.get("payload").filter(|v| !v.is_null()) {
                        println!(
                            "Payload: {} ({})",
                            payload["asset"]["name"].as_str().unwrap_or_default(),
                            payload["asset"]["sha256"].as_str().unwrap_or_default()
                        );
                    }
                    println!("{}", failure["next_attempt"].as_str().unwrap_or_default());
                }
                if let Some(processes) = value
                    .get("running_processes")
                    .and_then(|value| value.as_array())
                {
                    for process in processes {
                        println!(
                            "Running: PID {} {}",
                            process["pid"],
                            process["executable"].as_str().unwrap_or_default()
                        );
                    }
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
