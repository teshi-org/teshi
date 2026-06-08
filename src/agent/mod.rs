//! Agent module: tool definitions and execution for LLM function calling.
//!
//! Tools defined here can be registered with the LLM so it can inspect project
//! state and modify editor content. Read-only tools return results immediately;
//! file-modifying tools (e.g. `insert_scenario`) queue changes for user confirmation.

pub mod approval;
pub mod definition;
pub mod loader;
pub mod pipeline;
pub mod registry;
pub mod skills;
mod tools;
pub mod validator;

pub use tools::get_tools;

use anyhow::{Context, Result};

use crate::app::{AgentMutation, AgentPendingChange};
use crate::gherkin_lang::StructuralType;

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
    agent_idx: usize,
) -> Result<String> {
    match name {
        "get_project_info" => execute_get_project_info(app),
        "highlight_mindmap_nodes" => execute_highlight_mindmap_nodes(app, args_json),
        "apply_mindmap_filter" => execute_apply_mindmap_filter(app, args_json),
        "get_feature_content" => execute_get_feature_content(app, args_json),
        "insert_scenario" => execute_insert_scenario(app, args_json, tool_call_id, agent_idx),
        "update_step" => execute_update_step(app, args_json, tool_call_id, agent_idx),
        "create_feature_file" => {
            execute_create_feature_file(app, args_json, tool_call_id, agent_idx)
        }
        "delete_scenario" => execute_delete_scenario(app, args_json, tool_call_id, agent_idx),
        "rename_scenario" => execute_rename_scenario(app, args_json, tool_call_id, agent_idx),
        "reorder_steps" => execute_reorder_steps(app, args_json, tool_call_id, agent_idx),
        "search_features" => execute_search_features(app, args_json),
        "run_tests" => execute_run_tests(app, args_json),
        "load_skill" => execute_load_skill(app, args_json),
        "submit_requirements" => execute_submit_requirements(app, args_json),
        "generate_plan" => execute_generate_plan(app, args_json),
        "validate_feature" => execute_validate_feature(app, args_json),
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

    if project.features.is_empty() {
        let current_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "(unknown)".into());
        return Ok(format!(
            "Project directory: {current_dir}\n             Feature files: 0\n             \n             The project is empty — there are no .feature files to inspect or edit.\n             Do NOT call get_feature_content, insert_scenario, or update_step;\n             there are no files to operate on. Inform the user that the project\n             directory is empty and suggest adding a .feature file first.\n             Stop making tool calls."
        ));
    }

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
    out.push_str(&format!(
        "Feature: {} ({})\n",
        feature.name, feature.language
    ));
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
                "  [{:?}] {} {} (line {})\n",
                step.keyword_type, step.keyword, step.text, step.line_number
            ));
            if let Some(ref ds) = step.doc_string {
                out.push_str("    \"\"\"\n");
                for l in ds.lines() {
                    out.push_str(&format!("    {l}\n"));
                }
                out.push_str("    \"\"\"\n");
            }
            if let Some(ref dt) = step.data_table {
                for row in dt {
                    out.push_str(&format!("      | {} |\n", row.join(" | ")));
                }
            }
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
                "      [{:?}] {} {} (line {})\n",
                step.keyword_type, step.keyword, step.text, step.line_number
            ));
            if let Some(ref ds) = step.doc_string {
                out.push_str("        \"\"\"\n");
                for l in ds.lines() {
                    out.push_str(&format!("        {l}\n"));
                }
                out.push_str("        \"\"\"\n");
            }
            if let Some(ref dt) = step.data_table {
                for row in dt {
                    out.push_str(&format!("          | {} |\n", row.join(" | ")));
                }
            }
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

    if !feature.rules.is_empty() {
        out.push_str(&format!("\nRules: {}\n", feature.rules.len()));
        for rule in &feature.rules {
            out.push_str(&format!(
                "\n  Rule: {} (line {})\n",
                rule.name, rule.line_number
            ));
            if !rule.tags.is_empty() {
                out.push_str(&format!("    Tags: {}\n", rule.tags.join(" ")));
            }
            for sc in &rule.scenarios {
                let kind = match sc.kind {
                    crate::gherkin::ScenarioKind::Scenario => "Scenario",
                    crate::gherkin::ScenarioKind::ScenarioOutline => "Scenario Outline",
                };
                out.push_str(&format!(
                    "\n    {}: {} (line {})\n",
                    kind, sc.name, sc.line_number
                ));
                if !sc.tags.is_empty() {
                    out.push_str(&format!("      Tags: {}\n", sc.tags.join(" ")));
                }
                for step in &sc.steps {
                    out.push_str(&format!(
                        "      [{:?}] {} {} (line {})\n",
                        step.keyword_type, step.keyword, step.text, step.line_number
                    ));
                    if let Some(ref ds) = step.doc_string {
                        out.push_str("        \"\"\"\n");
                        for l in ds.lines() {
                            out.push_str(&format!("        {l}\n"));
                        }
                        out.push_str("        \"\"\"\n");
                    }
                    if let Some(ref dt) = step.data_table {
                        for row in dt {
                            out.push_str(&format!("          | {} |\n", row.join(" | ")));
                        }
                    }
                }
            }
        }
    }

    Ok(out)
}

fn execute_insert_scenario(
    app: &mut crate::app::App,
    args_json: &str,
    tool_call_id: &str,
    agent_idx: usize,
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

    // Verify the file exists in the project and get index
    let feature_idx = app.find_feature_idx_for_file(file_path).with_context(|| {
        let available: Vec<String> = app
            .project
            .features
            .iter()
            .map(|f| f.file_path.to_string_lossy().to_string())
            .collect();
        format!(
            "Feature file '{}' not found in project. Available files: {}",
            file_path,
            if available.is_empty() {
                "(none)".into()
            } else {
                available.join(", ")
            }
        )
    })?;

    let change = AgentPendingChange {
        description: format!("insert scenario \"{scenario_name}\" in {file_path}"),
        file_path: file_path.to_string(),
        mutation: AgentMutation::InsertAfterLine {
            after_line_1based: line,
            text: text_block.clone(),
        },
        scenario_name: scenario_name.to_string(),
        tool_call_id: tool_call_id.to_string(),
        old_buffer_snapshot: app
            .buffers
            .get(feature_idx)
            .map_or(String::new(), |b| b.as_string()),
        agent_idx,
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
    agent_idx: usize,
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
        old_buffer_snapshot: app
            .buffers
            .get(feature_idx)
            .map_or(String::new(), |b| b.as_string()),
        agent_idx,
    };

    app.queue_agent_change(change);

    Ok(format!(
        "Step {step_index} in scenario \"{scenario_name}\" queued for update. New text will be: \"{}\"",
        new_text
    ))
}

// ── create_feature_file ──────────────────────────────────────────────────────

fn execute_create_feature_file(
    app: &mut crate::app::App,
    args_json: &str,
    tool_call_id: &str,
    agent_idx: usize,
) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;

    let file_name = args
        .get("file_name")
        .and_then(|v| v.as_str())
        .context("missing 'file_name'")?;
    let feature_name = args
        .get("feature_name")
        .and_then(|v| v.as_str())
        .context("missing 'feature_name'")?;
    let description: Vec<String> = args
        .get("description")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let background_steps: Vec<String> = args
        .get("background_steps")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Validate file name ends with .feature
    if !file_name.ends_with(".feature") {
        anyhow::bail!("file_name must end with '.feature', got: {file_name}");
    }

    // Check for collision
    let full_path = app.project.root_dir.join(file_name);
    if full_path.exists() {
        anyhow::bail!("file already exists: {}", full_path.display());
    }

    // Build Gherkin content
    let mut content = String::new();
    if !tags.is_empty() {
        content.push_str(&tags.join(" "));
        content.push('\n');
    }
    content.push_str(&format!("Feature: {feature_name}\n"));
    if !description.is_empty() {
        for line in &description {
            content.push_str(&format!("  {line}\n"));
        }
    }
    if !background_steps.is_empty() {
        content.push_str("\n  Background:\n");
        for step in &background_steps {
            content.push_str(&format!("    {step}\n"));
        }
    }

    let change = AgentPendingChange {
        description: format!("create feature file \"{file_name}\""),
        file_path: file_name.to_string(),
        mutation: AgentMutation::CreateFile {
            file_name: file_name.to_string(),
            text: content.clone(),
        },
        scenario_name: feature_name.to_string(),
        tool_call_id: tool_call_id.to_string(),
        old_buffer_snapshot: String::new(),
        agent_idx,
    };

    app.queue_agent_change(change);

    Ok(format!(
        "Feature file \"{file_name}\" queued for creation. Awaiting user confirmation."
    ))
}

// ── delete_scenario ──────────────────────────────────────────────────────────

fn execute_delete_scenario(
    app: &mut crate::app::App,
    args_json: &str,
    tool_call_id: &str,
    agent_idx: usize,
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

    let feature_idx = app
        .find_feature_idx_for_file(file_path)
        .with_context(|| format!("feature file not found: {file_path}"))?;

    let feature = &app.project.features[feature_idx];
    let scenario = feature
        .scenarios
        .iter()
        .find(|s| s.name == scenario_name)
        .with_context(|| format!("scenario \"{scenario_name}\" not found in {file_path}"))?;

    let start_row = scenario.line_number.saturating_sub(1);

    // Determine end row: scan buffer forward from start_row until we hit
    // the next scenario header or EOF
    let buffer_content = app.buffers[feature_idx].as_string();
    let lines: Vec<&str> = buffer_content.lines().collect();
    let mut end_row = lines.len();
    for (i, line) in lines.iter().enumerate().skip(start_row + 1) {
        let trimmed = line.trim_start();
        let lang = app.buffers[feature_idx].language();
        if lang
            .match_structural_prefix(trimmed)
            .is_some_and(|(_, st)| {
                matches!(
                    st,
                    StructuralType::Scenario | StructuralType::ScenarioOutline
                )
            })
        {
            end_row = i;
            break;
        }
        if lang
            .match_structural_prefix(trimmed)
            .is_some_and(|(_, st)| st == StructuralType::Examples)
        {
            // Let it pass — still part of the current scenario outline
            continue;
        }
        if trimmed.starts_with('|') {
            // Table row — part of Examples
            continue;
        }
    }

    let short_desc = if scenario_name.len() > 30 {
        format!("{}...", &scenario_name[..27])
    } else {
        scenario_name.to_string()
    };

    let change = AgentPendingChange {
        description: format!("delete scenario \"{short_desc}\" from {file_path}"),
        file_path: file_path.to_string(),
        mutation: AgentMutation::DeleteRange {
            start_row_0based: start_row,
            end_row_0based: end_row,
        },
        scenario_name: scenario_name.to_string(),
        tool_call_id: tool_call_id.to_string(),
        old_buffer_snapshot: app.buffers[feature_idx].as_string(),
        agent_idx,
    };

    app.queue_agent_change(change);

    Ok(format!(
        "Scenario \"{scenario_name}\" queued for deletion from {file_path}. Awaiting user confirmation."
    ))
}

// ── rename_scenario ──────────────────────────────────────────────────────────

fn execute_rename_scenario(
    app: &mut crate::app::App,
    args_json: &str,
    tool_call_id: &str,
    agent_idx: usize,
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
    let new_name = args
        .get("new_name")
        .and_then(|v| v.as_str())
        .context("missing 'new_name'")?;

    let feature_idx = app
        .find_feature_idx_for_file(file_path)
        .with_context(|| format!("feature file not found: {file_path}"))?;

    let feature = &app.project.features[feature_idx];
    let scenario = feature
        .scenarios
        .iter()
        .find(|s| s.name == scenario_name)
        .with_context(|| format!("scenario \"{scenario_name}\" not found in {file_path}"))?;

    let row_0based = scenario.line_number.saturating_sub(1);
    let old_line = app.buffers[feature_idx].line(row_0based);
    let trimmed = old_line.trim_start();
    let leading_len = old_line.len().saturating_sub(trimmed.len());
    let leading_ws = &old_line[..leading_len];
    let keyword = match scenario.kind {
        crate::gherkin::ScenarioKind::Scenario => "Scenario:",
        crate::gherkin::ScenarioKind::ScenarioOutline => "Scenario Outline:",
    };
    let new_line = format!("{leading_ws}{keyword} {new_name}");

    let change = AgentPendingChange {
        description: format!(
            "rename scenario \"{scenario_name}\" to \"{new_name}\" in {file_path}"
        ),
        file_path: file_path.to_string(),
        mutation: AgentMutation::ReplaceLine {
            row_0based,
            new_text: new_line,
        },
        scenario_name: new_name.to_string(),
        tool_call_id: tool_call_id.to_string(),
        old_buffer_snapshot: app.buffers[feature_idx].as_string(),
        agent_idx,
    };

    app.queue_agent_change(change);

    Ok(format!(
        "Scenario \"{scenario_name}\" queued for rename to \"{new_name}\" in {file_path}. Awaiting user confirmation."
    ))
}

// ── reorder_steps ────────────────────────────────────────────────────────────

fn execute_reorder_steps(
    app: &mut crate::app::App,
    args_json: &str,
    tool_call_id: &str,
    agent_idx: usize,
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
    let step_order: Vec<usize> = args
        .get("step_order")
        .and_then(|v| v.as_array())
        .context("missing 'step_order'")?
        .iter()
        .map(|v| {
            v.as_u64()
                .map(|n| n as usize)
                .context("step_order must be integers")
        })
        .collect::<Result<Vec<_>>>()?;

    let feature_idx = app
        .find_feature_idx_for_file(file_path)
        .with_context(|| format!("feature file not found: {file_path}"))?;

    let feature = &app.project.features[feature_idx];
    let scenario = feature
        .scenarios
        .iter()
        .find(|s| s.name == scenario_name)
        .with_context(|| format!("scenario \"{scenario_name}\" not found in {file_path}"))?;

    let n = scenario.steps.len();
    if step_order.len() != n {
        anyhow::bail!(
            "step_order must be a permutation of 0..{n} (got {} values)",
            step_order.len()
        );
    }
    let mut seen = vec![false; n];
    for &idx in &step_order {
        if idx >= n {
            anyhow::bail!("step index {idx} out of bounds (scenario has {n} steps)");
        }
        seen[idx] = true;
    }
    for (i, &was_seen) in seen.iter().enumerate() {
        if !was_seen {
            anyhow::bail!("step_order is missing index {i} (must be a permutation)");
        }
    }

    // Build new step lines in the new order
    let mut new_step_lines: Vec<String> = Vec::new();
    for &idx in &step_order {
        let step = &scenario.steps[idx];
        let row_0based = step.line_number.saturating_sub(1);
        let old_line = app.buffers[feature_idx].line(row_0based);
        new_step_lines.push(old_line.to_string());
    }

    let start_row = scenario.steps[0].line_number.saturating_sub(1);
    let last_step = &scenario.steps[n - 1];
    let end_row = last_step.line_number; // exclusive (1-based → 0-based + 1)

    let new_text = new_step_lines.join("\n");

    let short_desc = if scenario_name.len() > 30 {
        format!("{}...", &scenario_name[..27])
    } else {
        scenario_name.to_string()
    };

    let change = AgentPendingChange {
        description: format!("reorder steps in scenario \"{short_desc}\" in {file_path}"),
        file_path: file_path.to_string(),
        mutation: AgentMutation::ReplaceRange {
            start_row_0based: start_row,
            end_row_0based: end_row,
            new_text,
        },
        scenario_name: scenario_name.to_string(),
        tool_call_id: tool_call_id.to_string(),
        old_buffer_snapshot: app.buffers[feature_idx].as_string(),
        agent_idx,
    };

    app.queue_agent_change(change);

    Ok(format!(
        "Steps in scenario \"{scenario_name}\" queued for reordering. Awaiting user confirmation."
    ))
}

// ── search_features ──────────────────────────────────────────────────────────

fn execute_search_features(app: &crate::app::App, args_json: &str) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;

    let tag_filter = args.get("tag").and_then(|v| v.as_str());
    let step_contains = args.get("step_contains").and_then(|v| v.as_str());
    let name_contains = args.get("scenario_name_contains").and_then(|v| v.as_str());

    if tag_filter.is_none() && step_contains.is_none() && name_contains.is_none() {
        anyhow::bail!(
            "at least one filter (tag, step_contains, or scenario_name_contains) must be provided"
        );
    }

    let mut results: Vec<String> = Vec::new();

    for feature in &app.project.features {
        let file_path = feature.file_path.to_string_lossy();
        for scenario in &feature.scenarios {
            // Check tag filter
            if let Some(tag) = tag_filter
                && !scenario.tags.iter().any(|t| t == tag)
            {
                continue;
            }
            // Check name contains filter
            if let Some(nc) = name_contains
                && !scenario.name.to_lowercase().contains(&nc.to_lowercase())
            {
                continue;
            }
            // Check step contains filter
            if let Some(sc) = step_contains {
                let step_matches = scenario
                    .steps
                    .iter()
                    .any(|s| s.text.to_lowercase().contains(&sc.to_lowercase()));
                if !step_matches {
                    continue;
                }
            }

            results.push(format!(
                "{}:{} | Scenario: {} | {} step(s)",
                file_path,
                scenario.line_number,
                scenario.name,
                scenario.steps.len()
            ));
        }
    }

    if results.is_empty() {
        Ok("No scenarios matched the given filters.".to_string())
    } else {
        Ok(format!(
            "Found {} matching scenario(s):\n{}",
            results.len(),
            results.join("\n")
        ))
    }
}

// ── run_tests ────────────────────────────────────────────────────────────────

fn execute_run_tests(app: &mut crate::app::App, args_json: &str) -> Result<String> {
    use std::time::Duration;

    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;

    let feature_path_filter = args.get("feature_path").and_then(|v| v.as_str());
    let scenario_name_filter = args.get("scenario_name").and_then(|v| v.as_str());

    let runner_config = app
        .runner_config
        .clone()
        .context("no test runner configured. Set up a runner in .teshi/config.toml")?;

    // Build run cases from project features
    let mut cases: Vec<crate::runner::RunCase> = Vec::new();
    for feature in &app.project.features {
        let fp = feature.file_path.to_string_lossy().to_string();
        if let Some(filter_path) = feature_path_filter
            && fp != filter_path
            && !fp.ends_with(filter_path)
        {
            continue;
        }
        for scenario in &feature.scenarios {
            if let Some(filter_name) = scenario_name_filter
                && scenario.name != filter_name
            {
                continue;
            }
            cases.push(crate::runner::RunCase {
                id: format!("{}:{}", feature.file_path.to_string_lossy(), scenario.name),
                feature_path: fp.clone(),
                scenario: scenario.name.clone(),
                line_number: Some(scenario.line_number),
                until_line: scenario.steps.last().map(|s| s.line_number),
            });
        }
    }

    if cases.is_empty() {
        anyhow::bail!("no matching scenarios found to run");
    }

    let total = cases.len();
    let request = crate::runner::RunRequest {
        command: "run".to_string(),
        cases,
        meta: Default::default(),
    };

    let rx = crate::runner::spawn_runner(runner_config, request)
        .context("failed to spawn test runner")?;

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut details: Vec<String> = Vec::new();

    loop {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(event) => match event {
                crate::runner::RunEvent::CasePassed {
                    case_id,
                    duration_ms,
                } => {
                    passed += 1;
                    details.push(format!(
                        "  PASS  {} ({}ms)",
                        case_id,
                        duration_ms.unwrap_or(0)
                    ));
                }
                crate::runner::RunEvent::CaseFailed {
                    case_id,
                    duration_ms,
                    error,
                } => {
                    failed += 1;
                    details.push(format!(
                        "  FAIL  {} ({}ms): {}",
                        case_id,
                        duration_ms.unwrap_or(0),
                        error.message
                    ));
                }
                crate::runner::RunEvent::CaseSkipped { case_id, reason } => {
                    skipped += 1;
                    details.push(format!(
                        "  SKIP  {}: {}",
                        case_id,
                        reason.unwrap_or_else(|| "unknown".to_string())
                    ));
                }
                _ => {}
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }
    }

    Ok(format!(
        "Test run complete: {passed} passed, {failed} failed, {skipped} skipped out of {total} total.\n\nDetails:\n{}",
        details.join("\n")
    ))
}

// ── load_skill ────────────────────────────────────────────────────────────

fn execute_load_skill(app: &mut crate::app::App, args_json: &str) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;
    let skill_name = args
        .get("skill_name")
        .and_then(|v| v.as_str())
        .context("missing 'skill_name'")?;

    if let Some(skill) = app.skill_registry.get(skill_name) {
        Ok(format!("## Skill: {}\n\n{}", skill.name, skill.content))
    } else {
        let catalog_text = app.skill_registry.catalog();
        let available: Vec<_> = catalog_text
            .lines()
            .filter(|l| l.starts_with("  - "))
            .collect();
        Ok(format!(
            "Skill '{}' not found. Available templates:\n{}",
            skill_name,
            if available.is_empty() {
                "  (no templates loaded)".into()
            } else {
                available.join("\n")
            }
        ))
    }
}

// ── submit_requirements ───────────────────────────────────────────────────────

fn execute_submit_requirements(app: &mut crate::app::App, args_json: &str) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;

    let feature_name = args
        .get("feature_name")
        .and_then(|v| v.as_str())
        .context("missing 'feature_name'")?
        .to_string();
    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from);
    let scenario_descriptions: Vec<String> = args
        .get("scenario_descriptions")
        .and_then(|v| v.as_array())
        .context("missing 'scenario_descriptions'")?
        .iter()
        .map(|v| v.as_str().unwrap_or("").to_string())
        .collect();
    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let requirement = crate::agent::pipeline::Requirement {
        feature_name,
        description,
        scenario_descriptions,
        tags,
    };

    app.pipeline_requirement = Some(requirement);
    app.generation_stage = crate::agent::pipeline::GenerationStage::Planning;

    Ok("Requirements collected. Now call `generate_plan` to design the scenario structure.".into())
}

// ── generate_plan ───────────────────────────────────────────────────────────

fn execute_generate_plan(app: &mut crate::app::App, args_json: &str) -> Result<String> {
    let plan: crate::agent::pipeline::GenerationPlan = serde_json::from_str(args_json)
        .context("invalid plan JSON — expected a GenerationPlan object with a 'features' array")?;

    if plan.features.is_empty() {
        anyhow::bail!("plan must include at least one feature");
    }

    for feature in &plan.features {
        if !feature.file_name.ends_with(".feature") {
            anyhow::bail!(
                "file_name must end with '.feature', got: '{}'",
                feature.file_name
            );
        }
        if feature.scenarios.is_empty() {
            anyhow::bail!(
                "feature '{}' has no scenarios (each feature needs at least one)",
                feature.feature_name
            );
        }
    }

    app.pipeline_plan = Some(plan);
    app.generation_stage = crate::agent::pipeline::GenerationStage::Writing;

    Ok("Plan accepted. Now use `create_feature_file` and `insert_scenario` to generate the feature files.".into())
}

// ── validate_feature ─────────────────────────────────────────────────────────

fn execute_validate_feature(app: &mut crate::app::App, args_json: &str) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;

    let file_path_opt = args.get("file_path").and_then(|v| v.as_str());

    // Build a temporary project filtered by file if needed
    let project = &app.project;

    let mut issues = if let Some(fp) = file_path_opt {
        // Filter to just the requested file
        let filtered: Vec<_> = project
            .features
            .iter()
            .filter(|f| {
                let path_str = f.file_path.to_string_lossy();
                path_str == fp || path_str.ends_with(fp)
            })
            .cloned()
            .collect();

        if filtered.is_empty() {
            let available: Vec<_> = project
                .features
                .iter()
                .map(|f| f.file_path.to_string_lossy().to_string())
                .collect();
            anyhow::bail!(
                "Feature file '{}' not found. Available files: {}",
                fp,
                if available.is_empty() {
                    "(none)".into()
                } else {
                    available.join(", ")
                }
            );
        }

        let temp_project = crate::gherkin::BddProject {
            root_dir: project.root_dir.clone(),
            features: filtered,
        };
        crate::agent::validator::validate_project(&temp_project)
    } else {
        crate::agent::validator::validate_project(project)
    };

    // Coverage check: compare generated scenarios against skill templates
    if let Some(fp) = file_path_opt
        && let Some(feature) = project.features.iter().find(|f| {
            let path_str = f.file_path.to_string_lossy();
            path_str == fp || path_str.ends_with(fp)
        })
    {
        // Collect scenarios from both scenarios and rules for coverage check
        let all_scenarios: Vec<crate::gherkin::BddScenario> = feature
            .scenarios
            .iter()
            .chain(feature.rules.iter().flat_map(|r| r.scenarios.iter()))
            .cloned()
            .collect();

        if !all_scenarios.is_empty() {
            let coverage_issues = crate::agent::validator::check_coverage(
                &feature.name,
                &all_scenarios,
                &app.skill_registry,
            );
            for mut ci in coverage_issues {
                ci.file = fp.to_string();
                issues.push(ci);
            }
        }
    }

    if issues.is_empty() {
        app.generation_stage = crate::agent::pipeline::GenerationStage::Complete;
    }

    Ok(crate::agent::validator::format_validation_result(&issues))
}
