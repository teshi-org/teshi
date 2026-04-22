use std::io::{self, BufRead, Write};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use teshi_tui_steps as tui_steps;

#[derive(Debug, Deserialize)]
struct RunRequest {
    command: String,
    cases: Vec<RunCase>,
    #[allow(dead_code)]
    meta: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct RunCase {
    id: String,
    #[allow(dead_code)]
    feature_path: String,
    scenario: String,
    #[allow(dead_code)]
    line_number: Option<usize>,
}

#[derive(Debug, Serialize)]
struct Event<'a, T> {
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(flatten)]
    payload: T,
}

#[derive(Debug, Serialize)]
struct StartRun {
    total: usize,
}

#[derive(Debug, Serialize)]
struct StartCase<'a> {
    case_id: &'a str,
    name: &'a str,
}

#[derive(Debug, Serialize)]
struct CasePassed<'a> {
    case_id: &'a str,
    duration_ms: u64,
}

#[derive(Debug, Serialize)]
struct CaseFailed<'a> {
    case_id: &'a str,
    duration_ms: u64,
    error: RunErrorOut,
}

#[derive(Debug, Serialize)]
struct CaseSkipped<'a> {
    case_id: &'a str,
    reason: &'a str,
}

#[derive(Debug, Serialize)]
struct RunErrorOut {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    stack: Option<String>,
    attachments: Vec<AttachmentOut>,
}

#[derive(Debug, Serialize)]
struct AttachmentOut {
    #[serde(rename = "type")]
    kind: String,
    path: String,
}

#[derive(Debug, Serialize)]
struct EndRun {
    passed: usize,
    failed: usize,
    skipped: usize,
}

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let Some(Ok(line)) = lines.next() else {
        return Ok(());
    };

    let request: RunRequest = match serde_json::from_str(&line) {
        Ok(req) => req,
        Err(_) => return Ok(()),
    };
    if request.command != "run" {
        return Ok(());
    }

    if let Err(err) = run_request(request) {
        let _ = writeln!(io::stderr(), "runner error: {err:?}");
    }
    Ok(())
}

fn write_event<T: Serialize>(out: &mut impl Write, event: Event<'_, T>) -> io::Result<()> {
    serde_json::to_writer(&mut *out, &event)?;
    out.write_all(b"\n")?;
    out.flush()
}

fn run_request(request: RunRequest) -> Result<()> {
    let mut out = io::BufWriter::new(io::stdout());
    let total = request.cases.len();
    let mut teshi_bin: Option<std::path::PathBuf> = None;

    write_event(
        &mut out,
        Event {
            kind: "start_run",
            payload: StartRun { total },
        },
    )?;

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;

    for case in &request.cases {
        write_event(
            &mut out,
            Event {
                kind: "start_case",
                payload: StartCase {
                    case_id: &case.id,
                    name: &case.scenario,
                },
            },
        )?;

        if tui_steps::is_tui_scenario(&case.scenario) && !tui_steps::tui_e2e_host_supported() {
            skipped = skipped.saturating_add(1);
            write_event(
                &mut out,
                Event {
                    kind: "case_skipped",
                    payload: CaseSkipped {
                        case_id: &case.id,
                        reason: "TUI E2E tests run on Linux only",
                    },
                },
            )?;
            continue;
        }

        if !tui_steps::supports_scenario(&case.scenario) {
            skipped = skipped.saturating_add(1);
            write_event(
                &mut out,
                Event {
                    kind: "case_skipped",
                    payload: CaseSkipped {
                        case_id: &case.id,
                        reason: "unimplemented scenario",
                    },
                },
            )?;
            continue;
        }

        if teshi_bin.is_none() {
            teshi_bin = Some(locate_teshi_bin().context("locate teshi binary")?);
        }
        let bin = teshi_bin.as_ref().context("teshi binary path")?;

        let start = Instant::now();
        match tui_steps::run_scenario(&case.scenario, bin) {
            Ok(()) => {
                passed = passed.saturating_add(1);
                write_event(
                    &mut out,
                    Event {
                        kind: "case_passed",
                        payload: CasePassed {
                            case_id: &case.id,
                            duration_ms: start.elapsed().as_millis() as u64,
                        },
                    },
                )?;
            }
            Err(err) => {
                failed = failed.saturating_add(1);
                let msg = err.to_string();
                let dbg = format!("{err:?}");
                let stack = if dbg != msg { Some(dbg) } else { None };
                write_event(
                    &mut out,
                    Event {
                        kind: "case_failed",
                        payload: CaseFailed {
                            case_id: &case.id,
                            duration_ms: start.elapsed().as_millis() as u64,
                            error: RunErrorOut {
                                message: msg,
                                stack,
                                attachments: Vec::new(),
                            },
                        },
                    },
                )?;
            }
        }
    }

    write_event(
        &mut out,
        Event {
            kind: "end_run",
            payload: EndRun {
                passed,
                failed,
                skipped,
            },
        },
    )?;

    Ok(())
}

fn locate_teshi_bin() -> Result<std::path::PathBuf> {
    if let Ok(bin) = std::env::var("TESHI_BIN") {
        let path = std::path::PathBuf::from(bin);
        if path.exists() {
            return Ok(path);
        }
    }
    let exe = std::env::current_exe().context("current_exe")?;
    let dir = exe.parent().context("runner exe has no parent directory")?;
    let candidate = dir.join("teshi.exe");
    if candidate.exists() {
        return Ok(candidate);
    }
    anyhow::bail!("teshi binary not found; set TESHI_BIN");
}
