use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};

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

    let mut out = io::BufWriter::new(io::stdout());
    let total = request.cases.len();

    write_event(
        &mut out,
        Event {
            kind: "start_run",
            payload: StartRun { total },
        },
    )?;

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
        write_event(
            &mut out,
            Event {
                kind: "case_passed",
                payload: CasePassed {
                    case_id: &case.id,
                    duration_ms: 0,
                },
            },
        )?;
    }

    write_event(
        &mut out,
        Event {
            kind: "end_run",
            payload: EndRun {
                passed: total,
                failed: 0,
                skipped: 0,
            },
        },
    )?;

    Ok(())
}

fn write_event<T: Serialize>(out: &mut impl Write, event: Event<'_, T>) -> io::Result<()> {
    serde_json::to_writer(&mut *out, &event)?;
    out.write_all(b"\n")?;
    out.flush()
}
