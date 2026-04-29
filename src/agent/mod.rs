//! Agent module: tool definitions and execution for LLM function calling.
//!
//! Tools defined here can be registered with the LLM so it can inspect project
//! state and modify editor content. Read-only tools return results immediately;
//! file-modifying tools (e.g. `insert_scenario`) queue changes for user confirmation.

mod tools;

pub use tools::get_tools;

use anyhow::{Context, Result};

use crate::app::{AgentMutation, AgentPendingChange};

/// Execute a named tool with the given JSON arguments and return the result
/// as plain text for the LLM.
///
/// `tool_call_id` is the unique identifier from the LLM tool-call request —
/// needed so high-risk tools like `insert_scenario` can associate a pending
/// change with the correct tool result.
pub fn execute_tool(
    app: &mut crate::app::App,
    name: &str,
    args_json: &str,
    tool_call_id: &str,
) -> Result<String> {
    match name {
        "get_project_info" => execute_get_project_info(app),
        "highlight_mindmap_nodes" => execute_highlight_mindmap_nodes(app, args_json),
        "apply_mindmap_filter" => execute_apply_mindmap_filter(app, args_json),
        "get_feature_content" => execute_get_feature_content(app, args_json),
        "insert_scenario" => execute_insert_scenario(app, args_json, tool_call_id),
        "update_step" => execute_update_step(app, args_json, tool_call_id),
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

fn execute_highlight_mindmap_nodes(app: &mut crate::app::App, args_json: &str) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;

    let cond = args
        .get("match_condition")
        .context("missing 'match_condition'")?
        .as_object()
        .context("'match_condition' must be an object")?;
    let cond_type = cond
        .get("type")
        .and_then(|v| v.as_str())
        .context("missing 'match_condition.type'")?;
    let cond_text = cond
        .get("text")
        .and_then(|v| v.as_str())
        .context("missing 'match_condition.text'")?;
    let color_str = args
        .get("color")
        .and_then(|v| v.as_str())
        .context("missing 'color'")?;

    let condition = match cond_type {
        "step_contains" => crate::mindmap::MatchCondition::StepContains(cond_text.into()),
        other => anyhow::bail!("unknown match condition type: {other}"),
    };

    let color = crate::mindmap::parse_color(color_str)
        .ok_or_else(|| anyhow::anyhow!("unknown color: {color_str}"))?;

    let rule = crate::mindmap::HighlightRule { condition, color };
    app.apply_mindmap_highlights(vec![rule]);

    Ok(format!(
        "Highlighted MindMap nodes matching 'step_contains={}' in {}",
        cond_text, color_str
    ))
}

fn execute_apply_mindmap_filter(app: &mut crate::app::App, args_json: &str) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;

    let filter_type = args
        .get("filter_type")
        .and_then(|v| v.as_str())
        .context("missing 'filter_type'")?;

    match filter_type {
        "clear" => {
            app.clear_mindmap_filter();
            app.clear_mindmap_highlights();
            Ok("Cleared MindMap filter and highlights".into())
        }
        "name_contains" => {
            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .context("missing 'value' for 'name_contains' filter")?;
            let filter = crate::mindmap::MindMapFilter::NameContains(value.into());
            app.apply_mindmap_filter(filter);
            Ok(format!("Applied MindMap filter: name_contains='{}'", value))
        }
        other => anyhow::bail!("unknown filter_type: {other}"),
    }
}

fn execute_get_project_info(app: &crate::app::App) -> Result<String> {
    let project = &app.project;

    let total_scenarios: usize = project.features.iter().map(|f| f.scenarios.len()).sum();
    let total_steps: usize = project
        .features
        .iter()
        .map(|f| {
            f.background.as_ref().map(|bg| bg.steps.len()).unwrap_or(0)
                + f.scenarios.iter().map(|s| s.steps.len()).sum::<usize>()
        })
        .sum();
    let total_backgrounds: usize = project
        .features
        .iter()
        .filter(|f| f.background.is_some())
        .count();

    let file_list: Vec<String> = project
        .features
        .iter()
        .map(|f| f.file_path.to_string_lossy().to_string())
        .collect();

    let active_file = app
        .file_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "(none)".into());

    let current_dir = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "(unknown)".into());

    Ok(format!(
        "Project directory: {current_dir}\n\
         Feature files: {}\n\
         Total scenarios: {total_scenarios}\n\
         Total steps: {total_steps}\n\
         Features with backgrounds: {total_backgrounds}\n\
         Active file: {active_file}\n\
         Files:\n{}",
        project.features.len(),
        file_list
            .iter()
            .map(|f| format!("  - {f}"))
            .collect::<Vec<_>>()
            .join("\n"),
    ))
}

fn execute_get_feature_content(app: &mut crate::app::App, args_json: &str) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;
    let file_path = args
        .get("file_path")
        .and_then(|v| v.as_str())
        .context("missing 'file_path'")?;

    let feature_idx = app
        .find_feature_idx_for_file(file_path)
        .with_context(|| format!("feature file not found: {file_path}"))?;

    let feature = &app.project.features[feature_idx];
    let path = feature.file_path.to_string_lossy();
    let mut out = String::new();

    out.push_str(&format!("File: {path} ({} lines)\n", feature.line_count));
    out.push_str(&format!("Feature: {}\n", feature.name));
    if !feature.tags.is_empty() {
        out.push_str(&format!("Tags: {}\n", feature.tags.join(" ")));
    }
    if !feature.description.is_empty() {
        out.push_str("Description:\n");
        for line in &feature.description {
            out.push_str(&format!("  {line}\n"));
        }
    }
    if let Some(bg) = &feature.background {
        out.push_str(&format!("\nBackground (line {}):\n", bg.line_number));
        for step in &bg.steps {
            out.push_str(&format!(
                "  {} {} (line {})\n",
                step.keyword, step.text, step.line_number
            ));
        }
    }
    out.push_str(&format!("\nScenarios: {}\n", feature.scenarios.len()));
    for (idx, sc) in feature.scenarios.iter().enumerate() {
        let kind = match sc.kind {
            crate::gherkin::ScenarioKind::Scenario => "Scenario",
            crate::gherkin::ScenarioKind::ScenarioOutline => "Scenario Outline",
        };
        out.push_str(&format!(
            "\n  [{idx}] {kind}: {} (line {})\n",
            sc.name, sc.line_number
        ));
        if !sc.tags.is_empty() {
            out.push_str(&format!("      Tags: {}\n", sc.tags.join(" ")));
        }
        for step in &sc.steps {
            out.push_str(&format!(
                "      {} {} (line {})\n",
                step.keyword, step.text, step.line_number
            ));
        }
        for (ei, ex) in sc.examples.iter().enumerate() {
            out.push_str(&format!(
                "      Examples [{ei}] (line {}):\n",
                ex.line_number
            ));
            if !ex.headers.is_empty() {
                out.push_str(&format!("        | {} |\n", ex.headers.join(" | ")));
            }
            for row in &ex.rows {
                out.push_str(&format!("        | {} |\n", row.join(" | ")));
            }
        }
    }

    Ok(out)
}

fn execute_insert_scenario(
    app: &mut crate::app::App,
    args_json: &str,
    tool_call_id: &str,
) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;

    let file_path = args
        .get("file_path")
        .and_then(|v| v.as_str())
        .context("missing 'file_path'")?;
    let scenario_name = args
        .get("scenario_name")
        .and_then(|v| v.as_str())
        .context("missing 'scenario_name'")?;
    let steps: Vec<String> = args
        .get("steps")
        .and_then(|v| v.as_array())
        .context("missing 'steps'")?
        .iter()
        .map(|v| v.as_str().unwrap_or("").to_string())
        .collect();
    let insert_after_line: Option<usize> = args
        .get("insert_after_line")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Build the Gherkin text block
    let mut text_block = String::new();
    // Leading blank line for separation
    text_block.push('\n');
    // Tags
    if !tags.is_empty() {
        text_block.push_str("  ");
        text_block.push_str(&tags.join(" "));
        text_block.push('\n');
    }
    // Scenario header
    text_block.push_str(&format!("  Scenario: {scenario_name}\n"));
    // Steps
    if steps.is_empty() {
        anyhow::bail!("at least one step is required");
    }
    for step in &steps {
        text_block.push_str(&format!("    {step}\n"));
    }

    // Determine insertion line: use provided value, or default to end of file
    let line = insert_after_line.unwrap_or_else(|| app.line_count_for_file(file_path).unwrap_or(0));

    // Verify the file exists in the project
    if app.find_feature_idx_for_file(file_path).is_none() {
        let available: Vec<String> = app
            .project
            .features
            .iter()
            .map(|f| f.file_path.to_string_lossy().to_string())
            .collect();
        anyhow::bail!(
            "Feature file '{}' not found in project. Available files: {}",
            file_path,
            if available.is_empty() {
                "(none)".into()
            } else {
                available.join(", ")
            }
        );
    }

    let change = AgentPendingChange {
        description: format!("insert scenario \"{scenario_name}\" in {file_path}"),
        file_path: file_path.to_string(),
        mutation: AgentMutation::InsertAfterLine {
            after_line_1based: line,
            text: text_block.clone(),
        },
        scenario_name: scenario_name.to_string(),
        tool_call_id: tool_call_id.to_string(),
    };

    app.queue_agent_change(change);

    Ok(format!(
        "Scenario \"{scenario_name}\" queued for insertion in {file_path} at line {line}. Awaiting user confirmation."
    ))
}

fn execute_update_step(
    app: &mut crate::app::App,
    args_json: &str,
    tool_call_id: &str,
) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;

    let file_path = args
        .get("file_path")
        .and_then(|v| v.as_str())
        .context("missing 'file_path'")?;
    let scenario_name = args
        .get("scenario_name")
        .and_then(|v| v.as_str())
        .context("missing 'scenario_name'")?;
    let step_index: usize = args
        .get("step_index")
        .and_then(|v| v.as_u64())
        .context("missing 'step_index'")? as usize;
    let new_text = args
        .get("new_text")
        .and_then(|v| v.as_str())
        .context("missing 'new_text'")?;

    // Find the feature file
    let feature_idx = app
        .find_feature_idx_for_file(file_path)
        .with_context(|| format!("feature file not found: {file_path}"))?;

    // Find the scenario by name in the parsed AST
    let scenario = app.project.features[feature_idx]
        .scenarios
        .iter()
        .find(|s| s.name == scenario_name)
        .with_context(|| format!("scenario \"{scenario_name}\" not found in {file_path}"))?;

    // Verify step index is in bounds
    if step_index >= scenario.steps.len() {
        let count = scenario.steps.len();
        anyhow::bail!(
            "step_index {step_index} is out of bounds. Scenario \"{scenario_name}\" has {count} step(s) (valid indices: 0..{})",
            if count == 0 { 0 } else { count - 1 }
        );
    }

    let step = &scenario.steps[step_index];
    let row_0based = step.line_number.saturating_sub(1);

    // Read the current line from the buffer
    let old_line = app.buffers[feature_idx].line(row_0based);

    // Reconstruct the line: preserve indentation and keyword, replace the body text
    let trimmed = old_line.trim_start();
    let leading_len = old_line.len().saturating_sub(trimmed.len());
    let leading_ws = &old_line[..leading_len];
    let new_line = format!("{leading_ws}{} {}", step.keyword, new_text);

    let short_desc = if scenario_name.len() > 30 {
        format!("{}...", &scenario_name[..27])
    } else {
        scenario_name.to_string()
    };

    let change = AgentPendingChange {
        description: format!(
            "update step {} in scenario \"{short_desc}\" in {file_path}",
            step_index
        ),
        file_path: file_path.to_string(),
        mutation: AgentMutation::ReplaceLine {
            row_0based,
            new_text: new_line,
        },
        scenario_name: scenario_name.to_string(),
        tool_call_id: tool_call_id.to_string(),
    };

    app.queue_agent_change(change);

    Ok(format!(
        "Step {step_index} in scenario \"{scenario_name}\" queued for update. New text will be: \"{}\"",
        new_text
    ))
}
