//! Validated Gherkin bindings and the NDJSON runner protocol.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use teshi_core::gherkin::{ScenarioKind, parse_feature};

use crate::{Launch, Terminal};

/// One explicitly recorded terminal operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// Launch the scenario's application.
    Launch { config: Launch },
    /// Send literal text.
    Write { text: String },
    /// Press a named key.
    Key { key: String },
    /// Resize the viewport.
    Resize { cols: u16, rows: u16 },
    /// Click the leftmost viewport cell currently rendering this text.
    ClickText { text: String, timeout_ms: u64 },
    /// Assert current viewport text with a finite deadline.
    ExpectText {
        text: String,
        visible: bool,
        timeout_ms: u64,
    },
    /// Require a viewport condition to remain true across successive frames.
    ExpectStableText {
        text: String,
        visible: bool,
        timeout_ms: u64,
        stable_ms: u64,
    },
    /// Assert the actual process exit code.
    ExpectExit { code: i32, timeout_ms: u64 },
}

/// A binding tied to the exact source step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// One-based Gherkin source line.
    pub line: usize,
    /// Exact parsed step body, used to reject stale recordings.
    pub text: String,
    /// Whether the recording is ready to execute.
    pub confirmed: bool,
    /// Operations in execution order.
    pub actions: Vec<Action>,
    /// Optional recorded doc string; emitted with substituted values during replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_string: Option<String>,
    /// Optional recorded step table, checked against the source when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_table: Option<Vec<Vec<String>>>,
}

/// Versioned sidecar stored next to `name.feature` as `name.tui.json`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bindings {
    /// Schema version, currently one.
    pub version: u32,
    /// Must be `tui`.
    pub mode: String,
    /// Source-bound step recordings.
    pub steps: Vec<Binding>,
}

/// Scenario identity received from Teshi's existing runner protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Case {
    /// Result correlation identifier.
    pub id: String,
    /// Feature file path, relative to project_root or absolute.
    pub feature_path: String,
    /// Scenario name.
    pub scenario: String,
    /// Optional scenario header or Examples data-row line for precise selection.
    pub line_number: Option<usize>,
    /// Optional last step source line, inclusive.
    pub until_line: Option<usize>,
}

/// Standard run request read from one line on stdin.
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    /// Must be `run`.
    pub command: String,
    /// Ordered scenarios to execute.
    pub cases: Vec<Case>,
    /// Runner metadata, including project_root and optional artifacts_dir.
    #[serde(default)]
    pub meta: HashMap<String, String>,
}

fn plan(case: &Case, root: &Path) -> Result<Vec<Binding>> {
    let path = root.join(&case.feature_path);
    let source = std::fs::read_to_string(&path)?;
    let feature = parse_feature(&source, path.clone());
    let language = teshi_core::gherkin_lang::GherkinLanguages::global().get(&feature.language);
    let mut backgrounds = 0;
    let mut seen_rule = false;
    let mut doc_string_delimiter = None;
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(delimiter) = doc_string_delimiter {
            if trimmed.starts_with(delimiter) {
                doc_string_delimiter = None;
            }
            continue;
        }
        if trimmed.starts_with("\"\"\"") || trimmed.starts_with("```") {
            doc_string_delimiter = Some(if trimmed.starts_with("\"\"\"") {
                "\"\"\""
            } else {
                "```"
            });
            continue;
        }
        if let Some((_, kind)) = language.match_structural_prefix(trimmed) {
            match kind {
                teshi_core::gherkin_lang::StructuralType::Rule => seen_rule = true,
                teshi_core::gherkin_lang::StructuralType::Background => {
                    backgrounds += 1;
                    if seen_rule || backgrounds > 1 {
                        bail!(
                            "Rule Background or multiple Background blocks are not supported by the current Gherkin AST"
                        );
                    }
                }
                _ => {}
            }
        }
    }
    let candidates: Vec<_> = feature
        .all_scenarios()
        .into_iter()
        .filter(|s| {
            s.name == case.scenario && case.line_number.is_none_or(|line| s.line_number == line)
        })
        .collect();
    if candidates.len() != 1 {
        bail!(
            "scenario identity must resolve exactly once: {:?} at {:?}",
            case.scenario,
            case.line_number
        );
    }
    let scenario = candidates[0];
    if scenario.kind == ScenarioKind::ScenarioOutline && scenario.examples.is_empty() {
        bail!("Scenario Outline requires Examples with at least one data row");
    }
    let bindings: Bindings =
        serde_json::from_str(&std::fs::read_to_string(path.with_extension("tui.json"))?)?;
    if bindings.version != 1 || bindings.mode != "tui" {
        bail!("expected TUI binding schema version 1 and mode tui");
    }
    let mut by_line = HashMap::new();
    for binding in bindings.steps {
        if by_line.insert(binding.line, binding).is_some() {
            bail!("duplicate TUI binding source line");
        }
    }
    let mut steps = Vec::new();
    if let Some(background) = &feature.background {
        steps.extend(background.steps.iter());
    }
    steps.extend(
        scenario
            .steps
            .iter()
            .filter(|step| case.until_line.is_none_or(|last| step.line_number <= last)),
    );
    if let Some(last) = case.until_line
        && !scenario.steps.iter().any(|s| s.line_number == last)
    {
        bail!("until_line must identify a step in the selected scenario");
    }
    if scenario.steps.is_empty() || steps.is_empty() {
        bail!("scenario has no executable steps");
    }
    let mut plan = Vec::new();
    let mut launched = false;
    let mut exit_asserted = false;
    for step in steps {
        let mut binding = by_line
            .remove(&step.line_number)
            .with_context(|| format!("unbound TUI step at line {}", step.line_number))?;
        if !binding.confirmed || binding.text != step.text || binding.actions.is_empty() {
            bail!(
                "unconfirmed, stale or empty TUI binding at line {}",
                step.line_number
            );
        }
        if binding
            .doc_string
            .as_ref()
            .is_some_and(|doc| Some(doc) != step.doc_string.as_ref())
            || binding
                .data_table
                .as_ref()
                .is_some_and(|table| Some(table) != step.data_table.as_ref())
        {
            bail!("stale TUI step arguments at line {}", step.line_number);
        }
        binding.doc_string = step.doc_string.clone();
        binding.data_table = step.data_table.clone();
        for action in &binding.actions {
            match action {
                Action::Launch { config } => {
                    if (launched && !exit_asserted)
                        || config.program.trim().is_empty()
                        || config.cols == 0
                        || config.rows == 0
                    {
                        bail!(
                            "invalid or repeated TUI launch at line {}",
                            step.line_number
                        );
                    }
                    launched = true;
                    exit_asserted = false;
                }
                _ if !launched => bail!("TUI action before launch at line {}", step.line_number),
                Action::Resize { cols, rows } if *cols == 0 || *rows == 0 => {
                    bail!("invalid TUI dimensions")
                }
                Action::Key { key } => {
                    if scenario.kind == ScenarioKind::ScenarioOutline && key.contains('<') {
                        continue;
                    }
                    crate::validate_key(key)
                        .with_context(|| format!("invalid TUI key at line {}", step.line_number))?;
                }
                Action::ExpectText {
                    text, timeout_ms, ..
                } if text.is_empty() || *timeout_ms == 0 || *timeout_ms > 120_000 => {
                    bail!("invalid TUI text assertion or timeout (1..120000ms)")
                }
                Action::ExpectExit { timeout_ms, .. }
                    if *timeout_ms == 0 || *timeout_ms > 120_000 =>
                {
                    bail!("invalid TUI exit timeout (1..120000ms)")
                }
                // execute() must observe this exit before reaching a later
                // launch; a failed exit assertion aborts the entire scenario.
                Action::ExpectExit { .. } => exit_asserted = true,
                Action::ExpectStableText {
                    text,
                    timeout_ms,
                    stable_ms,
                    ..
                } if text.is_empty()
                    || *stable_ms == 0
                    || *stable_ms > *timeout_ms
                    || *timeout_ms > 120_000 =>
                {
                    bail!("invalid stable TUI text assertion")
                }
                _ => {}
            }
        }
        plan.push(binding);
    }
    Ok(plan)
}

/// Substitute a template once, so example values cannot introduce new substitutions.
fn substitute(text: &str, values: &HashMap<String, String>) -> Result<String> {
    let mut result = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start + 1..].find('>') else {
            result.push_str(&rest[start..]);
            return Ok(result);
        };
        let end = start + 1 + end;
        let key = &rest[start + 1..end];
        result.push_str(
            values
                .get(key)
                .with_context(|| format!("unresolved Examples placeholder <{key}>"))?,
        );
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

fn substitute_json(value: &mut serde_json::Value, values: &HashMap<String, String>) -> Result<()> {
    match value {
        serde_json::Value::String(text) => *text = substitute(text, values)?,
        serde_json::Value::Array(items) => {
            for item in items {
                substitute_json(item, values)?;
            }
        }
        serde_json::Value::Object(items) => {
            for item in items.values_mut() {
                substitute_json(item, values)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug)]
struct ExamplePlan {
    line: Option<usize>,
    steps: Vec<Binding>,
}

fn example_plans(case: &Case, root: &Path) -> Result<Vec<ExamplePlan>> {
    let path = root.join(&case.feature_path);
    let source = std::fs::read_to_string(&path)?;
    let feature = parse_feature(&source, path);
    let candidates: Vec<_> = feature
        .all_scenarios()
        .into_iter()
        .filter(|s| s.name == case.scenario)
        .collect();
    let candidates: Vec<_> = candidates
        .into_iter()
        .filter(|s| {
            case.line_number.is_none_or(|line| {
                s.line_number == line
                    || s.examples.iter().any(|table| {
                        let first = source
                            .lines()
                            .enumerate()
                            .skip(table.line_number)
                            .find(|(_, text)| text.trim().starts_with('|'))
                            .map(|(i, _)| i + 2);
                        first.is_some_and(|first| (first..first + table.rows.len()).contains(&line))
                    })
            })
        })
        .collect();
    if candidates.len() != 1 {
        bail!("scenario identity must resolve exactly once");
    }
    let scenario = candidates[0];
    let mut template_case = case.clone();
    template_case.line_number = Some(scenario.line_number);
    // Always check the recorded template before expanding any data row.
    let template = plan(&template_case, root)?;
    if scenario.kind == ScenarioKind::Scenario {
        return Ok(vec![ExamplePlan {
            line: None,
            steps: template,
        }]);
    }
    let mut plans = Vec::new();
    for table in &scenario.examples {
        let unique: std::collections::HashSet<_> = table.headers.iter().collect();
        if table.headers.is_empty()
            || table.headers.iter().any(|h| h.is_empty())
            || unique.len() != table.headers.len()
            || table.rows.is_empty()
        {
            bail!(
                "invalid Examples headers or missing data rows at line {}",
                table.line_number
            );
        }
        let first = source
            .lines()
            .enumerate()
            .skip(table.line_number)
            .find(|(_, text)| text.trim().starts_with('|'))
            .context("missing Examples header")?
            .0
            + 2;
        for (index, row) in table.rows.iter().enumerate() {
            let line = first + index;
            if row.len() != table.headers.len() {
                bail!("Examples column count mismatch at line {line}");
            }
            let values: HashMap<_, _> = table
                .headers
                .iter()
                .cloned()
                .zip(row.iter().cloned())
                .collect();
            // Validate attached arguments as well as the visible step and action strings.
            for step in &scenario.steps {
                if let Some(doc) = &step.doc_string {
                    substitute(doc, &values)?;
                }
                if let Some(rows) = &step.data_table {
                    for cells in rows {
                        for cell in cells {
                            substitute(cell, &values)?;
                        }
                    }
                }
            }
            let mut steps = Vec::new();
            for binding in &template {
                let mut value = serde_json::to_value(binding)?;
                substitute_json(&mut value, &values)?;
                let binding: Binding = serde_json::from_value(value)?;
                for action in &binding.actions {
                    if let Action::Key { key } = action {
                        crate::validate_key(key)?;
                    }
                    if let Action::Launch { config } = action
                        && config.program.trim().is_empty()
                    {
                        bail!("empty expanded launch program at Examples line {line}");
                    }
                    if let Action::ExpectText { text, .. } = action
                        && text.is_empty()
                    {
                        bail!("empty expanded assertion at Examples line {line}");
                    }
                }
                steps.push(binding);
            }
            if case
                .line_number
                .is_none_or(|selected| selected == scenario.line_number || selected == line)
            {
                plans.push(ExamplePlan {
                    line: Some(line),
                    steps,
                });
            }
        }
    }
    if plans.is_empty() {
        bail!("Scenario Outline requires Examples with at least one data row");
    }
    Ok(plans)
}

fn emit(out: &mut impl Write, value: serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *out, &value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

fn execute(
    case: &Case,
    root: &Path,
    steps: Vec<Binding>,
    terminal: &mut Option<Terminal>,
    out: &mut impl Write,
    artifacts: &Path,
) -> Result<()> {
    // Validate all selected bindings before launching any application.
    for step in steps {
        let capture_viewport = step.actions.iter().any(|action| {
            matches!(
                action,
                Action::ExpectText { .. } | Action::ExpectStableText { .. }
            )
        });
        emit(
            out,
            json!({"type":"start_step","case_id":case.id,"step_id":step.line.to_string(),"text":step.text,"doc_string":step.doc_string,"data_table":step.data_table}),
        )?;
        let result: Result<()> = (|| {
            for action in step.actions {
                if let Action::Launch { mut config } = action {
                    let cwd = config
                        .cwd
                        .as_ref()
                        .map_or_else(|| root.to_path_buf(), |cwd| root.join(cwd));
                    config.cwd = Some(cwd.to_string_lossy().into_owned());
                    *terminal = Some(Terminal::launch(config)?);
                    continue;
                }
                let session = terminal.as_ref().context("TUI session missing")?;
                match action {
                    Action::Write { text } => session.write(&text)?,
                    Action::Key { key } => session.key(&key)?,
                    Action::Resize { cols, rows } => session.resize(cols, rows)?,
                    Action::ClickText { text, timeout_ms } => {
                        session.click_text(&text, timeout_ms)?
                    }
                    Action::ExpectText {
                        text,
                        visible,
                        timeout_ms,
                    } => session.expect_text(&text, visible, timeout_ms)?,
                    Action::ExpectStableText {
                        text,
                        visible,
                        timeout_ms,
                        stable_ms,
                    } => session.expect_stable_text(&text, visible, timeout_ms, stable_ms)?,
                    Action::ExpectExit { code, timeout_ms } => {
                        session.expect_exit(code, timeout_ms)?
                    }
                    Action::Launch { .. } => unreachable!(),
                }
            }
            Ok(())
        })();
        emit(
            out,
            json!({"type":"end_step","case_id":case.id,"step_id":step.line.to_string(),"status":if result.is_ok(){"passed"}else{"failed"},"message":result.as_ref().err().map(|e|format!("{e:#}"))}),
        )?;
        result.with_context(|| format!("TUI step at line {}", step.line))?;
        if capture_viewport && let Some(session) = terminal.as_ref() {
            std::fs::create_dir_all(artifacts)?;
            std::fs::write(
                artifacts.join(format!("step-{}.screen.txt", step.line)),
                session.screen()?,
            )?;
        }
    }
    if let Some(session) = terminal.take() {
        session.close()?;
    }
    Ok(())
}

/// Executes scenarios and writes standard NDJSON case/step events.
///
/// Returns the number of failed cases. Each case owns its PTY and failure artifacts.
/// # Errors
/// Returns errors for invalid requests or failed event/artifact I/O.
pub fn run(request: Request, out: &mut impl Write) -> Result<usize> {
    if request.command != "run" {
        bail!("expected command run");
    }
    let root = request
        .meta
        .get("project_root")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let artifacts = request
        .meta
        .get("artifacts_dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".teshi/tui-results"));
    emit(out, json!({"type":"start_run","total":request.cases.len()}))?;
    let mut failed = 0;
    let mut examples_passed = 0;
    let mut examples_failed = 0;
    for case in &request.cases {
        emit(
            out,
            json!({"type":"start_case","case_id":case.id,"name":case.scenario}),
        )?;
        let start = Instant::now();
        let mut terminal = None;
        let mut case_attachments = Vec::new();
        let result = (|| -> Result<()> {
            let plans = example_plans(case, &root)?;
            let mut errors = Vec::new();
            for row in plans {
                if let Some(line) = row.line {
                    emit(
                        out,
                        json!({"type":"start_example","case_id":case.id,"example_line":line}),
                    )?;
                }
                let case_id: String = case
                    .id
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                let row_artifacts = artifacts.join(case_id).join(
                    row.line
                        .map_or_else(|| "case".into(), |line| format!("example-{line}")),
                );
                let result = execute(case, &root, row.steps, &mut terminal, out, &row_artifacts);
                let mut attachments = Vec::new();
                if let Err(error) = &result {
                    if let Some(session) = &terminal {
                        std::fs::create_dir_all(&artifacts)?;
                        let directory = tempfile::Builder::new()
                            .prefix("example-")
                            .tempdir_in(&artifacts)?
                            .keep();
                        let path = directory.join("screen.txt");
                        std::fs::write(
                            &path,
                            session
                                .screen()
                                .unwrap_or_else(|e| format!("screen unavailable: {e:#}")),
                        )?;
                        attachments
                            .push(json!({"type":"text","path":path,"example_line":row.line}));
                    }
                    errors.push(match row.line {
                        Some(line) => format!("Examples row {line}: {error:#}"),
                        None => format!("{error:#}"),
                    });
                }
                case_attachments.extend(attachments.iter().cloned());
                if let Some(line) = row.line {
                    if result.is_ok() {
                        examples_passed += 1;
                    } else {
                        examples_failed += 1;
                    }
                    emit(
                        out,
                        json!({"type":"end_example","case_id":case.id,"example_line":line,"status":if result.is_ok(){"passed"}else{"failed"},"error":result.err().map(|e|format!("{e:#}")),"attachments":attachments}),
                    )?;
                }
                // Close failed sessions before advancing, even when later rows pass.
                drop(terminal.take());
            }
            if !errors.is_empty() {
                bail!("{}", errors.join("; "));
            }
            Ok(())
        })();
        match result {
            Ok(()) => emit(
                out,
                json!({"type":"case_passed","case_id":case.id,"duration_ms":start.elapsed().as_millis() as u64}),
            )?,
            Err(error) => {
                failed += 1;
                drop(terminal.take());
                emit(
                    out,
                    json!({"type":"case_failed","case_id":case.id,"duration_ms":start.elapsed().as_millis() as u64,"error":{"message":format!("{error:#}"),"attachments":case_attachments}}),
                )?;
            }
        }
    }
    emit(
        out,
        json!({"type":"end_run","passed":request.cases.len()-failed,"failed":failed,"skipped":0,"examples_passed":examples_passed,"examples_failed":examples_failed}),
    )?;
    Ok(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(source: &str) -> (tempfile::TempDir, Case) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("case.feature"), source).unwrap();
        let case = Case {
            id: "case".into(),
            feature_path: "case.feature".into(),
            scenario: "Sample".into(),
            line_number: None,
            until_line: None,
        };
        (directory, case)
    }

    fn record(directory: &Path, bindings: serde_json::Value) {
        std::fs::write(
            directory.join("case.tui.json"),
            serde_json::to_vec(&bindings).unwrap(),
        )
        .unwrap();
    }

    fn launch() -> serde_json::Value {
        json!({"action":"launch","config":{"program":"not-spawned-by-planning","args":[],"cols":80,"rows":24}})
    }

    #[test]
    fn rejects_missing_stale_unconfirmed_duplicate_and_unknown_bindings() {
        let (directory, case) = fixture("Feature: Test\n  Scenario: Sample\n    Given ready\n");
        for (steps, expected) in [
            (json!([]), "unbound"),
            (
                json!([{"line":3,"text":"old","confirmed":true,"actions":[launch()]}]),
                "stale",
            ),
            (
                json!([{"line":3,"text":"ready","confirmed":false,"actions":[launch()]}]),
                "unconfirmed",
            ),
            (
                json!([{"line":3,"text":"ready","confirmed":true,"actions":[launch()]},{"line":3,"text":"ready","confirmed":true,"actions":[launch()]}]),
                "duplicate",
            ),
            (
                json!([{"line":3,"text":"ready","confirmed":true,"actions":[{"action":"guess"}]}]),
                "unknown variant",
            ),
        ] {
            record(
                directory.path(),
                json!({"version":1,"mode":"tui","steps":steps}),
            );
            assert!(
                plan(&case, directory.path())
                    .unwrap_err()
                    .to_string()
                    .contains(expected),
                "expected {expected}"
            );
        }
    }

    #[test]
    fn resolves_rule_scenario_and_partial_step_boundary() {
        let (directory, mut case) = fixture(
            "Feature: Test\n  Rule: Group\n    Scenario: Sample\n      Given ready\n      Then complete\n",
        );
        record(
            directory.path(),
            json!({"version":1,"mode":"tui","steps":[{"line":4,"text":"ready","confirmed":true,"actions":[launch()]},{"line":5,"text":"complete","confirmed":true,"actions":[{"action":"expect_text","text":"done","visible":true,"timeout_ms":100}]}]}),
        );
        assert_eq!(plan(&case, directory.path()).unwrap().len(), 2);
        case.until_line = Some(4);
        assert_eq!(plan(&case, directory.path()).unwrap().len(), 1);
        case.until_line = Some(3);
        assert!(
            plan(&case, directory.path())
                .unwrap_err()
                .to_string()
                .contains("until_line")
        );
    }

    #[test]
    fn relaunch_requires_an_explicit_exit_assertion() {
        let (directory, case) =
            fixture("Feature: Test\n  Scenario: Sample\n    Given ready\n    When restart\n");
        for with_exit in [false, true] {
            let mut actions = Vec::new();
            if with_exit {
                actions.push(json!({"action":"expect_exit","code":0,"timeout_ms":1000}));
            }
            actions.push(launch());
            record(
                directory.path(),
                json!({"version":1,"mode":"tui","steps":[
                    {"line":3,"text":"ready","confirmed":true,"actions":[launch()]},
                    {"line":4,"text":"restart","confirmed":true,"actions":actions}
                ]}),
            );
            assert_eq!(plan(&case, directory.path()).is_ok(), with_exit);
        }
    }

    #[test]
    fn rejects_ambiguous_identity_and_outline_without_examples() {
        let (directory, mut case) = fixture(
            "Feature: Test\n  Scenario: Sample\n    Given ready\n  Scenario: Sample\n    Given ready\n",
        );
        assert!(
            plan(&case, directory.path())
                .unwrap_err()
                .to_string()
                .contains("exactly once")
        );
        case.line_number = Some(999);
        assert!(
            plan(&case, directory.path())
                .unwrap_err()
                .to_string()
                .contains("exactly once")
        );
        let (directory, case) =
            fixture("Feature: Test\n  Scenario Outline: Sample\n    Given <value>\n");
        assert!(
            plan(&case, directory.path())
                .unwrap_err()
                .to_string()
                .contains("Outline")
        );
    }

    #[test]
    fn expands_multiple_tables_and_selects_a_source_row_without_changing_template_checks() {
        let (directory, mut case) = fixture(
            "Feature: Test\n  Scenario Outline: Sample\n    Given ready <value>\n      \"\"\"\n      doc <value>\n      \"\"\"\n    Then table\n      | <value> |\n    Examples:\n      | value |\n      | first |\n      | second |\n    Examples:\n      | value |\n      | third |\n",
        );
        record(
            directory.path(),
            json!({"version":1,"mode":"tui","steps":[
                {"line":3,"text":"ready <value>","confirmed":true,"actions":[{"action":"launch","config":{"program":"<value>","args":["<value>"],"cols":80,"rows":24}}]},
                {"line":7,"text":"table","confirmed":true,"actions":[{"action":"write","text":"<value>"}]}
            ]}),
        );
        let plans = example_plans(&case, directory.path()).unwrap();
        assert_eq!(
            plans.iter().map(|p| p.line).collect::<Vec<_>>(),
            vec![Some(11), Some(12), Some(15)]
        );
        assert_eq!(plans[1].steps[0].text, "ready second");
        assert_eq!(plans[1].steps[0].doc_string.as_deref(), Some("doc second"));
        assert_eq!(
            plans[2].steps[1].data_table.as_ref().unwrap()[0][0],
            "third"
        );
        case.line_number = Some(12);
        assert_eq!(example_plans(&case, directory.path()).unwrap().len(), 1);
        case.until_line = Some(3);
        assert_eq!(
            example_plans(&case, directory.path()).unwrap()[0]
                .steps
                .len(),
            1
        );
        let path = directory.path().join("case.tui.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        value["steps"][0]["text"] = json!("ready second");
        record(directory.path(), value);
        assert!(
            example_plans(&case, directory.path())
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
    }

    #[test]
    fn rejects_invalid_example_shapes_and_missing_parameters() {
        for (header, rows, expected) in [
            ("value", "", "missing data"),
            ("value | value", "      | a | b |\n", "headers"),
            ("value", "      | a | b |\n", "column count"),
            ("other", "      | a |\n", "unresolved"),
        ] {
            let source = format!(
                "Feature: Test\n  Scenario Outline: Sample\n    Given ready <value>\n    Examples:\n      | {header} |\n{rows}"
            );
            let (directory, case) = fixture(&source);
            record(
                directory.path(),
                json!({"version":1,"mode":"tui","steps":[{"line":3,"text":"ready <value>","confirmed":true,"actions":[launch()]}]}),
            );
            assert!(
                example_plans(&case, directory.path())
                    .unwrap_err()
                    .to_string()
                    .contains(expected),
                "{expected}"
            );
        }
        assert_eq!(
            substitute(
                "<a> <b>",
                &HashMap::from([("a".into(), "<b>".into()), ("b".into(), "done".into())])
            )
            .unwrap(),
            "<b> done"
        );
    }

    #[test]
    fn rejects_invalid_actions_before_application_launch() {
        let (directory, case) = fixture("Feature: Test\n  Scenario: Sample\n    Given ready\n");
        for actions in [
            json!([{"action":"key","key":"Enter"}]),
            json!([launch(),{"action":"key","key":"typo-key-name"}]),
            json!([launch(),{"action":"resize","cols":0,"rows":24}]),
            json!([launch(),{"action":"expect_text","text":"ready","visible":true,"timeout_ms":0}]),
            json!([launch(),{"action":"expect_exit","code":0,"timeout_ms":120001}]),
            json!([launch(), launch()]),
        ] {
            record(
                directory.path(),
                json!({"version":1,"mode":"tui","steps":[{"line":3,"text":"ready","confirmed":true,"actions":actions}]}),
            );
            assert!(plan(&case, directory.path()).is_err());
        }
    }

    #[test]
    fn rule_background_fails_explicitly_but_headers_in_doc_strings_are_data() {
        let (directory, case) = fixture(
            "Feature: Test\n  Rule: Group\n    Background:\n      Given ready\n    Scenario: Sample\n      Then done\n",
        );
        assert!(
            plan(&case, directory.path())
                .unwrap_err()
                .to_string()
                .contains("Rule Background")
        );
        let (directory, case) = fixture(
            "Feature: Test\n  Scenario: Sample\n    Given ready\n      \"\"\"\n      Rule: quoted\n      Background: quoted\n      Background: also quoted\n      \"\"\"\n",
        );
        record(
            directory.path(),
            json!({"version":1,"mode":"tui","steps":[{"line":3,"text":"ready","confirmed":true,"actions":[launch()]}]}),
        );
        assert_eq!(plan(&case, directory.path()).unwrap().len(), 1);
    }
}
