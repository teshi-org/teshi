//! Source-bound project discovery acceptance using an owned real PTY.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use teshi_core::gherkin::parse_feature;
use teshi_tui_runner::{Launch, Terminal};

fn capture(session: &Terminal, request: &Value) -> Result<Value> {
    let directory =
        std::path::Path::new(request["directory"].as_str().context("missing directory")?);
    let stem = request["stem"].as_str().context("missing capture stem")?;
    let frame = session.capture_frame(directory, stem)?;
    Ok(json!({"screenshot_file":frame.png,"svg_file":frame.svg,"text_file":frame.text}))
}

fn terminal_host() -> Result<()> {
    let mut terminal: Option<Terminal> = None;
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let result = (|| -> Result<Value> {
            if request["op"] == "launch" {
                drop(terminal.take());
                terminal = Some(Terminal::launch(serde_json::from_value::<Launch>(
                    request["config"].clone(),
                )?)?);
                return Ok(Value::Null);
            }
            if request["op"] == "close" {
                drop(terminal.take());
                return Ok(Value::Null);
            }
            let session = terminal.as_ref().context("no owned terminal")?;
            let text = request["text"].as_str().unwrap_or("");
            match request["op"].as_str().unwrap_or("") {
                "write" => session.write(text)?,
                "key" => session.key(text)?,
                "click" => session.click_text(text, 8000)?,
                "screen" => return Ok(json!(session.screen()?)),
                "expect" => {
                    session.expect_text(text, request["visible"].as_bool().unwrap_or(true), 8000)?
                }
                "screenshot" => session.screenshot(std::path::Path::new(text))?,
                "capture" => return capture(session, &request),
                "exit" => session.expect_exit(0, 10000)?,
                _ => bail!("unknown terminal operation"),
            }
            Ok(Value::Null)
        })();
        let reply = match result {
            Ok(value) => json!({"ok":true,"value":value}),
            Err(error) => json!({"ok":false,"error":format!("{error:#}")}),
        };
        println!("{reply}");
        std::io::stdout().flush()?;
    }
    Ok(())
}

fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--terminal-host") {
        return terminal_host();
    }
    let line = std::io::stdin()
        .lock()
        .lines()
        .next()
        .context("missing run request")??;
    let mut request: Value = serde_json::from_str(&line)?;
    let root = request["meta"]["project_root"]
        .as_str()
        .context("missing project root")?
        .to_owned();
    for case in request["cases"].as_array_mut().context("missing cases")? {
        let path = std::path::Path::new(&root)
            .join(case["feature_path"].as_str().context("missing feature")?);
        let feature = parse_feature(&std::fs::read_to_string(&path)?, path.clone());
        let scenarios = feature.all_scenarios();
        let scenario = scenarios
            .iter()
            .find(|s| {
                s.name == case["scenario"].as_str().unwrap_or("")
                    && case["line_number"]
                        .as_u64()
                        .is_none_or(|n| n as usize == s.line_number)
            })
            .context("scenario not found")?;
        case["steps"] = json!(
            feature
                .background
                .iter()
                .flat_map(|b| &b.steps)
                .chain(&scenario.steps)
                .map(|s| json!({"line":s.line_number,"text":s.text}))
                .collect::<Vec<_>>()
        );
        case["examples"] = json!(
            scenario
                .examples
                .iter()
                .map(|e| json!({"headers":e.headers,"rows":e.rows}))
                .collect::<Vec<_>>()
        );
    }
    request["host"] = json!(std::env::current_exe()?);
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/directory_feature_steps.py");
    let mut child = Command::new(if cfg!(windows) { "python" } else { "python3" })
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    serde_json::to_writer(
        child.stdin.as_mut().context("missing child stdin")?,
        &request,
    )?;
    child.stdin.take();
    std::process::exit(child.wait()?.code().unwrap_or(1));
}
