//! Thin terminal adapter for the native update core.

use std::{
    io::{self, BufRead, IsTerminal, Write},
    path::Path,
    sync::atomic::AtomicBool,
};
use teshi_core::version::{ReleaseChannel, build_identity};
use teshi_update::manager;

/// Only an explicit `y`/`yes` consents to closing running processes.
fn parse_close_answer(line: &str) -> bool {
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Refuses before downloading when this installation still has running
/// processes unless the caller already allowed a force close or consents here.
///
/// Returns whether a verified force close is permitted for this attempt.
fn preflight_running_processes(
    executable: &Path,
    force_close: bool,
    json: bool,
) -> teshi_update::Result<bool> {
    let identity = build_identity();
    let installation = teshi_update::install::Installation::detect(executable, &identity)?;
    // Unmarked, portable and MSI installs fail for a different reason; they must
    // not be held up by a process prompt that cannot lead to an install.
    if !installation.can_install() {
        return Ok(force_close);
    }
    let processes = teshi_update::processes::installation_processes_for(executable, &identity)?;
    if processes.is_empty() {
        return Ok(force_close);
    }
    let interactive = !json && io::stdin().is_terminal() && io::stderr().is_terminal();
    let consent = if interactive && !force_close {
        eprintln!(
            "Teshi processes from this installation are still running:\n{}",
            teshi_update::processes::describe_processes(&processes)
        );
        eprint!("Close them and continue? Unsaved work may be lost. [y/N] ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        Some(parse_close_answer(&line))
    } else {
        None
    };
    match teshi_update::processes::plan_close(processes.len(), force_close, consent) {
        teshi_update::processes::ClosePlan::Proceed => Ok(force_close),
        teshi_update::processes::ClosePlan::ForceClose => {
            if !force_close {
                eprintln!(
                    "Closing running Teshi processes; unsaved work may be lost if they do not exit gracefully."
                );
            }
            Ok(true)
        }
        teshi_update::processes::ClosePlan::Stop => Err(teshi_update::UpdateError::new(
            teshi_update::ErrorCode::Busy,
            teshi_update::processes::busy_message(&processes, force_close),
        )),
    }
}

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
            let running = teshi_update::processes::installation_processes_for(
                &executable,
                &build_identity(),
            )?;
            let next_step = (!running.is_empty()).then_some(
                "Close the listed processes and retry, or allow a verified force close with `teshi update --force-close` (cached payload: `teshi update --retry-install --force-close`).",
            );
            return Ok((
                serde_json::json!({"status":"local_status", "installation":installation, "previous_transaction":previous, "last_failure":failure, "running_processes":running, "next_step":next_step}),
                0,
            ));
        }
        // Refuse before checking GitHub or downloading when this installation is
        // still in use; the prompt (TTY only) may grant a verified force close.
        let force_close = if check_only {
            force_close
        } else {
            preflight_running_processes(&std::env::current_exe()?, force_close, json)?
        };
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
                    if let Some(cached) = failure.get("cached_payload").and_then(|v| v.as_bool()) {
                        println!("Cached payload: {cached}");
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
                if let Some(next) = value.get("next_step").and_then(|value| value.as_str()) {
                    println!("Next: {next}");
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

#[cfg(test)]
mod tests {
    use super::parse_close_answer;

    #[test]
    fn only_an_explicit_yes_consents_to_closing_processes() {
        for answer in ["y", "Y", "yes", " YES \n"] {
            assert!(parse_close_answer(answer), "{answer:?}");
        }
        for answer in ["", "\n", "n", "no", "maybe"] {
            assert!(!parse_close_answer(answer), "{answer:?}");
        }
    }
}
