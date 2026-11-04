//! ACP model preference acceptance with an owned real-terminal host.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use teshi_core::gherkin::parse_feature;
use teshi_tui_runner::{Launch, Terminal};

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
        let index = scenarios
            .iter()
            .position(|s| {
                s.name == case["scenario"].as_str().unwrap_or("")
                    && case["line_number"]
                        .as_u64()
                        .is_none_or(|l| l as usize == s.line_number)
            })
            .context("scenario not found")?;
        let english_name = match path.file_name().and_then(|s| s.to_str()).unwrap_or("") {
            "ACP模型选择持久化.feature" => "tui_acp_model_persistence.feature",
            name => name,
        };
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let english_path = repo.join("features/en-US").join(english_name);
        let english = parse_feature(&std::fs::read_to_string(&english_path)?, english_path);
        let canonical = english.scenario_at(index).context("missing English pair")?;
        let mut steps = Vec::new();
        let actual_steps = feature
            .background
            .iter()
            .flat_map(|b| &b.steps)
            .chain(&scenarios[index].steps);
        let canonical_steps: Vec<_> = english
            .background
            .iter()
            .flat_map(|b| &b.steps)
            .chain(&canonical.steps)
            .collect();
        for (i, step) in actual_steps.enumerate() {
            steps.push(json!({"line":step.line_number,"text":step.text,"canonical":canonical_steps.get(i).context("locale steps differ")?.text,"table":step.data_table}));
        }
        if steps.len() != canonical_steps.len() {
            bail!("locale step counts differ");
        }
        case["steps"] = json!(steps);
        case["examples"] = json!(
            canonical
                .examples
                .iter()
                .map(|table| json!({"headers":table.headers,"rows":table.rows}))
                .collect::<Vec<_>>()
        );
    }
    request["host"] = json!(std::env::current_exe()?);
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/model_feature_steps.py");
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
    let status = child.wait()?;
    std::process::exit(status.code().unwrap_or(1));
}
