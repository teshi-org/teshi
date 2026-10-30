//! Replay coverage is derived from source, never from the binding subset.

use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::HashMap, fs, path::Path};
use teshi_core::{gherkin::ScenarioKind, locator::StepBinding};

pub(crate) struct PlannedStep {
    pub binding: StepBinding,
    pub case_id: String,
    pub scenario: String,
    pub step_id: String,
}

pub(crate) struct ReplayPlan {
    pub steps: Vec<PlannedStep>,
    pub gaps: Vec<Value>,
    pub omitted: usize,
}

pub(crate) fn build(project: &Path, feature: &str, until: Option<usize>) -> Result<ReplayPlan> {
    build_selected(project, feature, until, None)
}

pub(crate) fn build_selected(
    project: &Path,
    feature: &str,
    until: Option<usize>,
    selected_scenario: Option<&str>,
) -> Result<ReplayPlan> {
    let path = project.join(feature);
    let source = fs::read_to_string(&path)?;
    let parsed = teshi_core::parse_feature(&source, path);
    let bindings = teshi_engine::list_step_bindings(project, feature)?;
    let by_line: HashMap<_, _> = bindings.steps.iter().map(|b| (b.step_line, b)).collect();
    let mut plan = ReplayPlan {
        steps: Vec::new(),
        gaps: Vec::new(),
        omitted: 0,
    };
    let scenarios = parsed.all_scenarios();
    if let Some(name) = selected_scenario {
        anyhow::ensure!(
            scenarios.iter().filter(|s| s.name == name).count() == 1,
            "scenario name must match exactly one scenario: {name}"
        );
    }
    for scenario in scenarios
        .into_iter()
        .filter(|s| selected_scenario.is_none_or(|name| s.name == name))
    {
        let rule = parsed.rules.iter().find(|rule| {
            rule.scenarios
                .iter()
                .any(|s| s.line_number == scenario.line_number)
        });
        let manual = parsed
            .tags
            .iter()
            .chain(&scenario.tags)
            .chain(rule.into_iter().flat_map(|rule| &rule.tags))
            .any(|tag| tag.trim_start_matches('@').eq_ignore_ascii_case("manual"));
        let rows: Vec<_> = if scenario.kind == ScenarioKind::ScenarioOutline {
            scenario
                .examples
                .iter()
                .flat_map(|examples| {
                    examples.rows.iter().enumerate().map(move |(index, row)| {
                        (
                            format!("{}:{}", examples.line_number, index),
                            examples.headers.iter().zip(row).collect::<Vec<_>>(),
                            examples.tags.iter().any(|tag| {
                                tag.trim_start_matches('@').eq_ignore_ascii_case("manual")
                            }),
                        )
                    })
                })
                .collect()
        } else {
            vec![("0".into(), Vec::new(), false)]
        };
        if rows.is_empty() && until.is_none_or(|line| scenario.line_number <= line) {
            plan.gaps.push(
                json!({"status":"blocked", "scenario":scenario.name, "reason":"no_example_rows"}),
            );
        }
        for (row, values, row_manual) in rows {
            let case_id = format!("scenario:{}:{row}", scenario.line_number);
            let selected = until.is_none_or(|line| scenario.line_number <= line);
            for step in parsed
                .background
                .iter()
                .flat_map(|b| &b.steps)
                .chain(&scenario.steps)
            {
                if !selected || until.is_some_and(|line| step.line_number > line) {
                    plan.omitted += 1;
                    continue;
                }
                let gap = if manual || row_manual {
                    Some(("manual", "manual_case"))
                } else if let Some(binding) = by_line.get(&step.line_number) {
                    if binding.status != "confirmed" {
                        Some(("unbound", "binding_not_confirmed"))
                    } else if teshi_engine::normalize_step_text(&binding.step_text)
                        != teshi_engine::normalize_step_text(&step.text)
                    {
                        Some(("unbound", "stale_binding"))
                    } else {
                        None
                    }
                } else {
                    Some(("unbound", "missing_binding"))
                };
                if let Some((status, reason)) = gap {
                    plan.gaps
                        .push(json!({"status":status,"reason":reason,"case_id":case_id,
                                         "scenario":scenario.name,"step_line":step.line_number}));
                    continue;
                }
                let mut binding = (*by_line[&step.line_number]).clone();
                let expand = |text: &str| {
                    values.iter().fold(text.to_owned(), |text, (key, value)| {
                        text.replace(&format!("<{key}>"), value)
                    })
                };
                binding.step_text = expand(&step.text);
                binding.primary.value = expand(&binding.primary.value);
                binding.primary.value_arg = binding.primary.value_arg.as_deref().map(expand);
                plan.steps.push(PlannedStep {
                    binding,
                    scenario: scenario.name.clone(),
                    step_id: format!("{case_id}:L{}", step.line_number),
                    case_id: case_id.clone(),
                });
            }
        }
    }
    if plan.steps.is_empty() && plan.gaps.is_empty() {
        plan.gaps
            .push(json!({"status":"blocked","reason":"empty_selection"}));
    }
    Ok(plan)
}
