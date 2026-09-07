//! NDJSON runner for requirement-library CLI Gherkin features.

mod steps;
mod world;

use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::steps::run_case;
use crate::world::locate_teshi_bin;

#[derive(Debug, Deserialize)]
struct RunRequest {
    command: String,
    cases: Vec<RunCase>,
}

#[derive(Debug, Deserialize)]
struct RunCase {
    id: String,
    feature_path: String,
    scenario: String,
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
struct RunErrorOut {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    stack: Option<String>,
    attachments: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct EndRun {
    passed: usize,
    failed: usize,
    skipped: usize,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            let _ = writeln!(io::stderr(), "runner error: {err:?}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let Some(Ok(line)) = lines.next() else {
        return Ok(ExitCode::SUCCESS);
    };
    let request: RunRequest = serde_json::from_str(&line)?;
    if request.command != "run" {
        return Ok(ExitCode::SUCCESS);
    }

    let teshi = locate_teshi_bin()?;
    let mut out = io::BufWriter::new(io::stdout());
    write_event(
        &mut out,
        Event {
            kind: "start_run",
            payload: StartRun {
                total: request.cases.len(),
            },
        },
    )?;

    let mut passed = 0usize;
    let mut failed = 0usize;
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
        let start = Instant::now();
        match run_case(&teshi, &case.feature_path, &case.scenario) {
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
                let message = err.to_string();
                let dbg = format!("{err:?}");
                write_event(
                    &mut out,
                    Event {
                        kind: "case_failed",
                        payload: CaseFailed {
                            case_id: &case.id,
                            duration_ms: start.elapsed().as_millis() as u64,
                            error: RunErrorOut {
                                message,
                                stack: (dbg != err.to_string()).then_some(dbg),
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
                skipped: 0,
            },
        },
    )?;
    out.flush()?;
    if failed == 0 {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn write_event<T: Serialize>(out: &mut impl Write, event: Event<'_, T>) -> io::Result<()> {
    serde_json::to_writer(&mut *out, &event)?;
    out.write_all(b"\n")?;
    out.flush()
}
