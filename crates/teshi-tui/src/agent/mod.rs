//! Agent module: tool definitions and execution for LLM function calling.
//!
//! Tools defined here can be registered with the LLM so it can inspect project
//! state and modify editor content. Read-only tools return results immediately;
//! file-modifying tools (e.g. `insert_scenario`) queue changes for user confirmation.

use anyhow::{Context, Result};
use std::collections::HashSet;

use crate::app::{AgentMutation, AgentPendingChange};
use teshi_core::gherkin_lang::StructuralType;

// Browser tool helpers
fn resolve_sidecar_ws_url() -> Option<String> {
    let project_root = std::env::current_dir().ok()?;
    let endpoint_path = project_root.join(".teshi").join("cdp-endpoint.json");
    let text = std::fs::read_to_string(endpoint_path).ok()?;
    let payload: serde_json::Value = serde_json::from_str(&text).ok()?;
    payload.get("ws_url")?.as_str().map(|s| s.to_string())
}

fn send_browser_command(cmd: &str, args: serde_json::Value) -> Result<String> {
    use std::time::Duration;
    let ws_url = resolve_sidecar_ws_url().ok_or_else(|| {
        anyhow::anyhow!("no browser sidecar connected; start Embedded or connect Chrome first")
    })?;

    let mut command = args.clone();
    let request_id = format!(
        "agent-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    command["cmd"] = serde_json::json!(cmd);
    command["request_id"] = serde_json::json!(request_id);

    let response =
        teshi_engine::send_sidecar_command_with_timeout(&ws_url, command, Duration::from_secs(15))
            .map_err(|e| anyhow::anyhow!("browser command failed: {e}"))?;

    Ok(serde_json::to_string_pretty(&response)?)
}

/// Execute a named tool with the given JSON arguments and return the result
/// as plain text for the LLM.
///
/// `tool_call_id` is the unique identifier from the LLM tool-call request —
/// needed so high-risk tools like `insert_scenario` can associate a pending
/// change with the correct tool result.
fn execute_tool_impl(
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
        "submit_requirements" => execute_submit_requirements(app, args_json),
        "propose_test_points" => execute_propose_test_points(app, args_json),
        "generate_plan" => execute_generate_plan(app, args_json),
        "validate_feature" => execute_validate_feature(app, args_json),
        // Browser agent exploration tools
        "browser_snapshot" => execute_browser_snapshot(app),
        "browser_click" => execute_browser_click(app, args_json),
        "browser_type" => execute_browser_type(app, args_json),
        "browser_assert" => execute_browser_assert(app, args_json),
        "browser_go_back" => execute_browser_go_back(app),
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

impl teshi_agent::AgentHost for crate::app::App {
    fn execute_tool(
        &mut self,
        name: &str,
        args_json: &str,
        tool_call_id: &str,
        agent_idx: usize,
    ) -> Result<String> {
        execute_tool_impl(self, name, args_json, tool_call_id, agent_idx)
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
            teshi_core::gherkin::ScenarioKind::Scenario => "Scenario",
            teshi_core::gherkin::ScenarioKind::ScenarioOutline => "Scenario Outline",
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
                    teshi_core::gherkin::ScenarioKind::Scenario => "Scenario",
                    teshi_core::gherkin::ScenarioKind::ScenarioOutline => "Scenario Outline",
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

fn ensure_feature_writing_allowed(app: &crate::app::App, tool_name: &str) -> Result<()> {
    use teshi_agent::pipeline::GenerationStage;

    if matches!(
        app.generation_stage,
        GenerationStage::Gathering
            | GenerationStage::GeneratingTestPoints
            | GenerationStage::ReviewingTestPoints
            | GenerationStage::Planning
    ) {
        anyhow::bail!(
            "{tool_name} is blocked until generate_plan advances the pipeline to Feature Writing (current stage: {})",
            app.generation_stage.label()
        );
    }
    Ok(())
}

fn execute_insert_scenario(
    app: &mut crate::app::App,
    args_json: &str,
    tool_call_id: &str,
    agent_idx: usize,
) -> Result<String> {
    ensure_feature_writing_allowed(app, "insert_scenario")?;

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
    let test_point_ids: Vec<String> = args
        .get("test_point_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    validate_insert_scenario_traceability(app, file_path, scenario_name, &test_point_ids)?;
    let tags = teshi_core::authoring::merge_scenario_tags(&tags, &test_point_ids);

    // Build the Gherkin text block
    let mut text_block = String::new();
    // Leading blank line for separation
    text_block.push('\n');
    // Tags (including @teshi-tp:<id> for traceability)
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

fn validate_insert_scenario_traceability(
    app: &crate::app::App,
    file_path: &str,
    scenario_name: &str,
    test_point_ids: &[String],
) -> Result<()> {
    if test_point_ids.is_empty() {
        anyhow::bail!("insert_scenario requires at least one approved test_point_id");
    }
    let unique_ids: HashSet<&str> = test_point_ids.iter().map(String::as_str).collect();
    if unique_ids.len() != test_point_ids.len() {
        anyhow::bail!("insert_scenario test_point_ids must not contain duplicates");
    }

    let plan = app
        .pipeline_plan
        .as_ref()
        .context("insert_scenario requires an accepted generation plan")?;
    let feature = plan
        .features
        .iter()
        .find(|feature| {
            feature.file_name == file_path
                || std::path::Path::new(file_path)
                    .file_name()
                    .is_some_and(|name| name == std::ffi::OsStr::new(&feature.file_name))
        })
        .with_context(|| {
            format!("insert_scenario file '{file_path}' is not in the accepted plan")
        })?;
    let scenario = feature
        .scenarios
        .iter()
        .find(|scenario| scenario.name == scenario_name)
        .with_context(|| {
            format!(
                "insert_scenario scenario '{scenario_name}' is not planned for '{}'",
                feature.file_name
            )
        })?;
    let planned_ids: HashSet<&str> = scenario.test_point_ids.iter().map(String::as_str).collect();
    if unique_ids != planned_ids {
        anyhow::bail!(
            "insert_scenario test_point_ids for '{}' must exactly match the accepted plan",
            scenario_name
        );
    }

    let test_points = app
        .authoring_ui
        .artifacts
        .as_ref()
        .map(|artifacts| artifacts.test_points.test_points.as_slice())
        .unwrap_or(&[]);
    for id in test_point_ids {
        let test_point = test_points
            .iter()
            .find(|test_point| test_point.id == *id)
            .with_context(|| format!("insert_scenario references unknown test point '{id}'"))?;
        if test_point.review_state != teshi_core::authoring::ReviewState::Approved {
            anyhow::bail!(
                "insert_scenario test point '{id}' is {:?} (must be Approved)",
                test_point.review_state
            );
        }
        if test_point
            .requirement_links
            .iter()
            .any(|link| link.resolution != teshi_core::authoring::ResolutionState::Resolved)
        {
            anyhow::bail!("insert_scenario test point '{id}' has stale requirement links");
        }
    }
    Ok(())
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
    ensure_feature_writing_allowed(app, "create_feature_file")?;

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
        teshi_core::gherkin::ScenarioKind::Scenario => "Scenario:",
        teshi_core::gherkin::ScenarioKind::ScenarioOutline => "Scenario Outline:",
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
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .filter(|s| !s.trim().is_empty())
                .collect()
        })
        .unwrap_or_default();
    let source_refs = parse_source_refs(args.get("source_refs"))?;
    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let requirement = teshi_agent::pipeline::Requirement {
        feature_name,
        description,
        scenario_descriptions,
        source_refs,
        tags,
    };
    if !requirement.has_usable_sources() {
        anyhow::bail!(
            "submit_requirements requires source_refs and/or scenario_descriptions (pasted text)"
        );
    }

    app.pipeline_requirement = Some(requirement);
    app.generation_stage = teshi_agent::pipeline::GenerationStage::GeneratingTestPoints;
    app.persist_generation_state()?;

    Ok(
        "Requirements collected. Now call `propose_test_points` with non-Gherkin verification intents."
            .into(),
    )
}

fn parse_source_refs(
    value: Option<&serde_json::Value>,
) -> Result<Vec<teshi_agent::pipeline::RequirementSourceRef>> {
    let Some(arr) = value.and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    let mut refs = Vec::new();
    for item in arr {
        let document_id = item
            .get("document_id")
            .and_then(|v| v.as_str())
            .context("source_refs[].document_id is required")?
            .to_string();
        let range = if let Some(range_val) = item.get("range") {
            let start = range_val
                .get("start")
                .and_then(|v| v.as_u64())
                .context("source_refs[].range.start is required")? as u32;
            let end = range_val
                .get("end")
                .and_then(|v| v.as_u64())
                .context("source_refs[].range.end is required")? as u32;
            if start >= end {
                anyhow::bail!("source_refs[].range must be non-empty (start < end)");
            }
            Some(teshi_core::authoring::TextRange::new(start, end))
        } else {
            None
        };
        refs.push(teshi_agent::pipeline::RequirementSourceRef { document_id, range });
    }
    Ok(refs)
}

// ── propose_test_points ───────────────────────────────────────────────────────

fn execute_propose_test_points(app: &mut crate::app::App, args_json: &str) -> Result<String> {
    if app.pipeline_requirement.is_none()
        && !matches!(
            app.generation_stage,
            teshi_agent::pipeline::GenerationStage::GeneratingTestPoints
                | teshi_agent::pipeline::GenerationStage::ReviewingTestPoints
        )
    {
        anyhow::bail!(
            "propose_test_points requires submitted requirements (call submit_requirements first)"
        );
    }

    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;
    let items = args
        .get("test_points")
        .and_then(|v| v.as_array())
        .context("missing 'test_points' array")?;
    if items.is_empty() {
        anyhow::bail!("propose_test_points requires at least one test point");
    }

    ensure_authoring_artifacts(app);
    let artifacts = app
        .authoring_ui
        .artifacts
        .as_mut()
        .context("authoring artifacts unavailable")?;

    let mut id_allocation_pool = artifacts.test_points.test_points.clone();
    let mut proposed = Vec::with_capacity(items.len());
    let mut proposed_ids = HashSet::new();
    for item in items {
        let test_point =
            parse_proposed_test_point(item, &id_allocation_pool, &artifacts.documents)?;
        if !proposed_ids.insert(test_point.id.clone()) {
            anyhow::bail!(
                "propose_test_points contains duplicate id '{}'",
                test_point.id
            );
        }
        id_allocation_pool.push(test_point.clone());
        proposed.push(test_point);
    }
    for tp in &proposed {
        if let Some(existing) = artifacts
            .test_points
            .test_points
            .iter()
            .find(|existing| existing.id == tp.id)
            && existing.review_state != teshi_core::authoring::ReviewState::Proposed
        {
            anyhow::bail!(
                "cannot re-propose test point '{}' after human review ({:?})",
                tp.id,
                existing.review_state
            );
        }
    }

    let mut created_ids = Vec::new();
    for tp in proposed {
        created_ids.push(tp.id.clone());
        // Replace existing id if re-proposing; otherwise append.
        if let Some(idx) = artifacts
            .test_points
            .test_points
            .iter()
            .position(|existing| existing.id == tp.id)
        {
            let mut replacement = tp;
            replacement.scenario_refs =
                std::mem::take(&mut artifacts.test_points.test_points[idx].scenario_refs);
            artifacts.test_points.test_points[idx] = replacement;
        } else {
            artifacts.test_points.test_points.push(tp);
        }
    }

    teshi_engine::save_test_points(&app.project.root_dir, &artifacts.test_points)
        .context("persist proposed test points")?;
    app.test_points_ui.rebuild_tree(Some(artifacts));

    app.generation_stage = teshi_agent::pipeline::GenerationStage::ReviewingTestPoints;
    app.persist_generation_state()?;

    Ok(format!(
        "Persisted {} Proposed test point(s): {}. Pipeline paused for human review in the Test Points tab (key 5). Do NOT call generate_plan until the user continues generation.",
        created_ids.len(),
        created_ids.join(", ")
    ))
}

fn ensure_authoring_artifacts(app: &mut crate::app::App) {
    if app.authoring_ui.artifacts.is_none() {
        app.authoring_ui.artifacts = Some(teshi_core::authoring::AuthoringArtifacts {
            index: Default::default(),
            documents: Vec::new(),
            test_points: Default::default(),
            diagnostics: Vec::new(),
        });
        app.authoring_ui.discovered = true;
    }
}

fn parse_proposed_test_point(
    item: &serde_json::Value,
    existing: &[teshi_core::authoring::TestPoint],
    documents: &[teshi_core::authoring::RequirementDocumentContent],
) -> Result<teshi_core::authoring::TestPoint> {
    let title = item
        .get("title")
        .and_then(|v| v.as_str())
        .context("test_points[].title is required")?
        .trim()
        .to_string();
    if title.is_empty() {
        anyhow::bail!("test_points[].title must be non-empty");
    }
    let objective = item
        .get("objective")
        .and_then(|v| v.as_str())
        .context("test_points[].objective is required")?
        .trim()
        .to_string();
    if objective.is_empty() {
        anyhow::bail!("test_points[].objective must be non-empty");
    }
    let hierarchy_segments: Vec<String> = item
        .get("hierarchy_path")
        .and_then(|v| v.as_array())
        .context("test_points[].hierarchy_path is required")?
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect();
    if hierarchy_segments.is_empty() {
        anyhow::bail!("test_points[].hierarchy_path must contain at least one non-empty segment");
    }

    let id = item
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| next_agent_test_point_id(existing));

    let preconditions = item
        .get("preconditions")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let expected_outcomes = item
        .get("expected_outcomes")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let requirement_links = parse_requirement_links(item.get("requirement_links"), documents)?;

    Ok(teshi_core::authoring::TestPoint {
        id,
        title,
        objective,
        preconditions,
        expected_outcomes,
        hierarchy_path: teshi_core::authoring::HierarchyPath::new(hierarchy_segments),
        // Hard gate: agent proposals are always Proposed; never Approved.
        review_state: teshi_core::authoring::ReviewState::Proposed,
        requirement_links,
        scenario_refs: Vec::new(),
    })
}

fn next_agent_test_point_id(existing: &[teshi_core::authoring::TestPoint]) -> String {
    let mut max_n = 0u64;
    for tp in existing {
        if let Some(n) = tp.id.strip_prefix("tp-").and_then(|s| s.parse().ok()) {
            max_n = max_n.max(n);
        }
    }
    format!("tp-{}", max_n + 1)
}

fn parse_requirement_links(
    value: Option<&serde_json::Value>,
    documents: &[teshi_core::authoring::RequirementDocumentContent],
) -> Result<Vec<teshi_core::authoring::RequirementLink>> {
    let Some(arr) = value.and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    let mut links = Vec::new();
    for item in arr {
        let document_id = item
            .get("document_id")
            .and_then(|v| v.as_str())
            .context("requirement_links[].document_id is required")?
            .trim()
            .to_string();
        if document_id.is_empty() {
            anyhow::bail!("requirement_links[].document_id must be non-empty");
        }
        let document = documents
            .iter()
            .find(|document| document.meta.id == document_id)
            .with_context(|| {
                format!("requirement link references unknown document '{document_id}'")
            })?;
        let document_revision = item
            .get("document_revision")
            .and_then(|v| v.as_str())
            .context("requirement_links[].document_revision is required")?
            .trim()
            .to_string();
        if document_revision != document.meta.revision.as_str() {
            anyhow::bail!(
                "requirement link revision '{}' does not match current revision '{}' for document '{}'",
                document_revision,
                document.meta.revision.as_str(),
                document_id
            );
        }
        let position = item
            .get("position")
            .context("requirement_links[].position is required")?;
        let start = u32::try_from(
            position
                .get("start")
                .and_then(|v| v.as_u64())
                .context("requirement_links[].position.start is required")?,
        )
        .context("requirement_links[].position.start exceeds u32")?;
        let end = u32::try_from(
            position
                .get("end")
                .and_then(|v| v.as_u64())
                .context("requirement_links[].position.end is required")?,
        )
        .context("requirement_links[].position.end exceeds u32")?;
        if start >= end {
            anyhow::bail!("requirement_links[].position must be non-empty");
        }
        let quote_obj = item
            .get("quote")
            .context("requirement_links[].quote is required")?;
        let quote = quote_obj
            .get("quote")
            .and_then(|v| v.as_str())
            .context("requirement_links[].quote.quote is required")?
            .to_string();
        if quote.is_empty() {
            anyhow::bail!("requirement_links[].quote.quote must be non-empty");
        }
        let range = teshi_core::authoring::TextRange::new(start, end);
        let selected_quote = teshi_core::authoring::slice_by_char_range(&document.body, range)
            .with_context(|| {
                format!(
                    "requirement link range [{start}, {end}) is outside document '{}'",
                    document_id
                )
            })?;
        if selected_quote != quote {
            anyhow::bail!(
                "requirement link quote does not match range [{start}, {end}) in document '{}'",
                document_id
            );
        }
        let prefix = quote_obj
            .get("prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let suffix = quote_obj
            .get("suffix")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        links.push(teshi_core::authoring::RequirementLink {
            document_id,
            document_revision,
            position: range,
            quote: teshi_core::authoring::QuoteSelector {
                quote,
                prefix,
                suffix,
            },
            resolution: teshi_core::authoring::ResolutionState::Resolved,
        });
    }
    Ok(links)
}

// ── generate_plan ───────────────────────────────────────────────────────────

fn execute_generate_plan(app: &mut crate::app::App, args_json: &str) -> Result<String> {
    if matches!(
        app.generation_stage,
        teshi_agent::pipeline::GenerationStage::ReviewingTestPoints
            | teshi_agent::pipeline::GenerationStage::GeneratingTestPoints
            | teshi_agent::pipeline::GenerationStage::Gathering
            | teshi_agent::pipeline::GenerationStage::Idle
    ) {
        anyhow::bail!(
            "generate_plan is blocked until human approval advances the pipeline to Planning (current stage: {})",
            app.generation_stage.label()
        );
    }

    let plan: teshi_agent::pipeline::GenerationPlan = serde_json::from_str(args_json)
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

    let test_points = app
        .authoring_ui
        .artifacts
        .as_ref()
        .map(|a| a.test_points.test_points.as_slice())
        .unwrap_or(&[]);
    if let Err(errors) = teshi_agent::pipeline::validate_plan_test_point_ids(&plan, test_points) {
        anyhow::bail!("generate_plan rejected: {}", errors.join("; "));
    }

    app.pipeline_plan = Some(plan);
    app.generation_stage = teshi_agent::pipeline::GenerationStage::Writing;
    app.persist_generation_state()?;

    Ok("Plan accepted. Now use `create_feature_file` and `insert_scenario` to generate the feature files.".into())
}

// ── validate_feature ─────────────────────────────────────────────────────────

fn execute_validate_feature(app: &mut crate::app::App, args_json: &str) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;

    let file_path_opt = args.get("file_path").and_then(|v| v.as_str());

    // Build a temporary project filtered by file if needed
    let project = &app.project;

    let issues = if let Some(fp) = file_path_opt {
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

        let temp_project = teshi_core::gherkin::BddProject {
            root_dir: project.root_dir.clone(),
            features: filtered,
        };
        teshi_agent::validator::validate_project(&temp_project)
    } else {
        teshi_agent::validator::validate_project(project)
    };

    if issues.is_empty() {
        app.generation_stage = teshi_agent::pipeline::GenerationStage::Complete;
        app.persist_generation_state()?;
    }

    Ok(teshi_agent::validator::format_validation_result(&issues))
}

// ── Browser agent exploration tool handlers ──

fn execute_browser_snapshot(_app: &mut crate::app::App) -> Result<String> {
    send_browser_command("get_structured_snapshot", serde_json::json!({}))
}

fn execute_browser_click(_app: &mut crate::app::App, args_json: &str) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;
    let ref_id = args
        .get("ref")
        .and_then(|v| v.as_str())
        .context("missing 'ref'")?;
    send_browser_command("click_ref", serde_json::json!({"ref": ref_id}))
}

fn execute_browser_type(_app: &mut crate::app::App, args_json: &str) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;
    let ref_id = args
        .get("ref")
        .and_then(|v| v.as_str())
        .context("missing 'ref'")?;
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .context("missing 'text'")?;
    send_browser_command("type_ref", serde_json::json!({"ref": ref_id, "text": text}))
}

fn execute_browser_assert(_app: &mut crate::app::App, args_json: &str) -> Result<String> {
    let args: serde_json::Value =
        serde_json::from_str(args_json).context("invalid JSON arguments")?;
    let condition_type = args
        .get("condition_type")
        .and_then(|v| v.as_str())
        .context("missing 'condition_type'")?;
    let value = args
        .get("value")
        .and_then(|v| v.as_str())
        .context("missing 'value'")?;

    match condition_type {
        "text_visible" => {
            // Get snapshot and check for text in the page
            let snap = send_browser_command("get_structured_snapshot", serde_json::json!({}))?;
            let snap_val: serde_json::Value = serde_json::from_str(&snap)?;
            if let Some(snapshot) = snap_val.get("snapshot")
                && let Some(elements) = snapshot.get("elements").and_then(|v| v.as_array())
            {
                for el in elements {
                    if let Some(text) = el.get("text").and_then(|v| v.as_str())
                        && text.contains(value)
                    {
                        return Ok(format!("assertion passed: text '{}' found on page", value));
                    }
                    if let Some(name) = el.get("name").and_then(|v| v.as_str())
                        && name.contains(value)
                    {
                        return Ok(format!(
                            "assertion passed: accessible name '{}' found on page",
                            value
                        ));
                    }
                }
            }
            Ok(format!(
                "assertion failed: text '{}' not found on page",
                value
            ))
        }
        "url_match" => {
            let snap = send_browser_command("get_structured_snapshot", serde_json::json!({}))?;
            let snap_val: serde_json::Value = serde_json::from_str(&snap)?;
            if let Some(snapshot) = snap_val.get("snapshot")
                && let Some(url) = snapshot.get("url").and_then(|v| v.as_str())
            {
                let matches = url.contains(value) || url.starts_with(value);
                if matches {
                    return Ok(format!(
                        "assertion passed: URL '{}' contains '{}'",
                        url, value
                    ));
                }
                return Ok(format!(
                    "assertion failed: URL '{}' does not contain '{}'",
                    url, value
                ));
            }
            Ok("assertion failed: could not get current URL".to_string())
        }
        other => anyhow::bail!("unknown condition_type: {other}"),
    }
}

fn execute_browser_go_back(_app: &mut crate::app::App) -> Result<String> {
    send_browser_command("go_back", serde_json::json!({}))
}

#[cfg(test)]
mod pipeline_gate_tests {
    use std::fs;
    use tempfile::tempdir;
    use teshi_agent::AgentHost;
    use teshi_agent::approval::ApprovalMode;
    use teshi_agent::pipeline::GenerationStage;
    use teshi_core::authoring::{
        DocumentRevision, QuoteSelector, RequirementDocumentContent, RequirementDocumentMeta,
        RequirementLink, ResolutionState, ReviewState, ScenarioRef, TestPointsFile, TextRange,
    };

    fn app_in_temp_project() -> (crate::app::App, tempfile::TempDir) {
        let dir = tempdir().expect("tempdir");
        let feature = dir.path().join("sample.feature");
        fs::write(
            &feature,
            "Feature: Sample\n  Scenario: placeholder\n    Given noop\n",
        )
        .expect("write feature");
        let app = crate::app::App::from_file(&feature, crate::config::load_config().unwrap())
            .expect("open app");
        (app, dir)
    }

    fn persisted_test_points(dir: &tempfile::TempDir) -> TestPointsFile {
        let json =
            fs::read_to_string(dir.path().join("testpoints/testpoints.json")).expect("test points");
        serde_json::from_str(&json).expect("valid test points")
    }

    fn set_writing_plan(app: &mut crate::app::App, file_path: &str, scenarios: &[(&str, &[&str])]) {
        let scenarios: Vec<_> = scenarios
            .iter()
            .map(|(name, ids)| {
                serde_json::json!({
                    "name": name,
                    "steps": ["Given x"],
                    "test_point_ids": ids,
                })
            })
            .collect();
        app.pipeline_plan = Some(
            serde_json::from_value(serde_json::json!({
                "features": [{
                    "file_name": file_path,
                    "feature_name": "Sample",
                    "scenarios": scenarios,
                }]
            }))
            .expect("generation plan"),
        );
        app.generation_stage = GenerationStage::Writing;
    }

    #[test]
    fn submit_requirements_advances_to_generating_test_points() {
        let (mut app, _dir) = app_in_temp_project();
        let result = app
            .execute_tool(
                "submit_requirements",
                r#"{
                    "feature_name": "Auth",
                    "scenario_descriptions": ["user can log in"],
                    "source_refs": [{"document_id": "doc-1"}]
                }"#,
                "tc-1",
                0,
            )
            .expect("submit");
        assert!(result.contains("propose_test_points"));
        assert_eq!(app.generation_stage, GenerationStage::GeneratingTestPoints);
        assert!(
            app.pipeline_requirement
                .as_ref()
                .unwrap()
                .has_usable_sources()
        );
    }

    #[test]
    fn propose_pauses_in_review_as_proposed_only() {
        let (mut app, dir) = app_in_temp_project();
        app.execute_tool(
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "tc-1",
            0,
        )
        .unwrap();
        let result = app
            .execute_tool(
                "propose_test_points",
                r#"{
                    "test_points": [{
                        "title": "Valid login",
                        "objective": "User authenticates successfully",
                        "hierarchy_path": ["Auth", "Login"]
                    }]
                }"#,
                "tc-2",
                0,
            )
            .expect("propose");
        assert!(result.contains("Proposed"));
        assert_eq!(app.generation_stage, GenerationStage::ReviewingTestPoints);
        let tp = &app
            .authoring_ui
            .artifacts
            .as_ref()
            .unwrap()
            .test_points
            .test_points[0];
        assert_eq!(tp.review_state, ReviewState::Proposed);
        assert!(dir.path().join("testpoints/testpoints.json").is_file());
    }

    #[test]
    fn proposal_allocates_unique_ids_for_every_omitted_id() {
        let (mut app, dir) = app_in_temp_project();
        app.execute_tool(
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "tc-1",
            0,
        )
        .unwrap();

        app.execute_tool(
            "propose_test_points",
            r#"{"test_points":[
                {"title":"First","objective":"o1","hierarchy_path":["A"]},
                {"title":"Second","objective":"o2","hierarchy_path":["A"]},
                {"title":"Third","objective":"o3","hierarchy_path":["A"]}
            ]}"#,
            "tc-2",
            0,
        )
        .unwrap();

        let ids: Vec<_> = persisted_test_points(&dir)
            .test_points
            .into_iter()
            .map(|test_point| test_point.id)
            .collect();
        assert_eq!(ids, vec!["tp-1", "tp-2", "tp-3"]);
    }

    #[test]
    fn proposal_rejects_invalid_requirement_links() {
        let document = RequirementDocumentContent {
            meta: RequirementDocumentMeta {
                id: "doc-1".into(),
                path: "auth.md".into(),
                title: "Auth".into(),
                revision: DocumentRevision::new("rev-1"),
            },
            body: "Login required".into(),
        };
        let documents = vec![document];
        let cases = [
            (
                serde_json::json!([{
                    "document_id": "missing",
                    "document_revision": "rev-1",
                    "position": {"start": 0, "end": 5},
                    "quote": {"quote": "Login"}
                }]),
                "unknown document",
            ),
            (
                serde_json::json!([{
                    "document_id": "doc-1",
                    "document_revision": "old",
                    "position": {"start": 0, "end": 5},
                    "quote": {"quote": "Login"}
                }]),
                "does not match current revision",
            ),
            (
                serde_json::json!([{
                    "document_id": "doc-1",
                    "document_revision": "rev-1",
                    "position": {"start": 0, "end": 50},
                    "quote": {"quote": "Login"}
                }]),
                "outside document",
            ),
            (
                serde_json::json!([{
                    "document_id": "doc-1",
                    "document_revision": "rev-1",
                    "position": {"start": 0, "end": 5},
                    "quote": {"quote": "Logout"}
                }]),
                "does not match range",
            ),
        ];

        for (value, expected) in cases {
            let error = super::parse_requirement_links(Some(&value), &documents).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "expected '{expected}' in '{error}'"
            );
        }

        let valid = serde_json::json!([{
            "document_id": "doc-1",
            "document_revision": "rev-1",
            "position": {"start": 0, "end": 5},
            "quote": {"quote": "Login"}
        }]);
        let links = super::parse_requirement_links(Some(&valid), &documents).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].resolution, ResolutionState::Resolved);
    }

    #[test]
    fn generate_plan_blocked_during_review_even_with_bypass() {
        let (mut app, _dir) = app_in_temp_project();
        app.approval_mode = ApprovalMode::Bypass;
        app.execute_tool(
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "tc-1",
            0,
        )
        .unwrap();
        app.execute_tool(
            "propose_test_points",
            r#"{"test_points":[{"title":"t","objective":"o","hierarchy_path":["A"]}]}"#,
            "tc-2",
            0,
        )
        .unwrap();
        let err = app
            .execute_tool(
                "generate_plan",
                r#"{
                    "features": [{
                        "file_name": "auth.feature",
                        "feature_name": "Auth",
                        "scenarios": [{
                            "name": "Login",
                            "steps": ["Given x"],
                            "test_point_ids": ["tp-1"]
                        }]
                    }]
                }"#,
                "tc-3",
                0,
            )
            .unwrap_err();
        assert!(err.to_string().contains("blocked"));
        assert_eq!(app.generation_stage, GenerationStage::ReviewingTestPoints);
        assert_eq!(app.approval_mode, ApprovalMode::Bypass);
    }

    #[test]
    fn writing_tools_are_blocked_before_plan_even_with_bypass() {
        let (mut app, dir) = app_in_temp_project();
        app.approval_mode = ApprovalMode::Bypass;
        let file_path = app.project.features[0]
            .file_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let insert_args =
            format!(r#"{{"file_path":"{file_path}","scenario_name":"Login","steps":["Given x"]}}"#);

        for stage in [
            GenerationStage::Gathering,
            GenerationStage::GeneratingTestPoints,
            GenerationStage::ReviewingTestPoints,
            GenerationStage::Planning,
        ] {
            app.generation_stage = stage;
            let insert_error = app
                .execute_tool("insert_scenario", &insert_args, "tc-insert", 0)
                .unwrap_err();
            assert!(
                insert_error.to_string().contains("blocked"),
                "insert_scenario should be blocked during {stage:?}"
            );

            let create_error = app
                .execute_tool(
                    "create_feature_file",
                    r#"{"file_name":"blocked.feature","feature_name":"Blocked"}"#,
                    "tc-create",
                    0,
                )
                .unwrap_err();
            assert!(
                create_error.to_string().contains("blocked"),
                "create_feature_file should be blocked during {stage:?}"
            );
        }

        assert!(app.pending_agent_changes.is_empty());
        assert!(!dir.path().join("blocked.feature").exists());
    }

    #[test]
    fn re_proposing_an_approved_test_point_is_rejected_without_mutation() {
        let (mut app, _dir) = app_in_temp_project();
        app.execute_tool(
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "tc-1",
            0,
        )
        .unwrap();
        app.execute_tool(
            "propose_test_points",
            r#"{"test_points":[{"id":"tp-login","title":"Original","objective":"o","hierarchy_path":["A"]}]}"#,
            "tc-2",
            0,
        )
        .unwrap();
        let tp = &mut app
            .authoring_ui
            .artifacts
            .as_mut()
            .unwrap()
            .test_points
            .test_points[0];
        assert!(crate::test_points_tab::TestPointsUiState::approve(tp));

        let error = app
            .execute_tool(
                "propose_test_points",
                r#"{"test_points":[{"id":"tp-login","title":"Changed","objective":"new","hierarchy_path":["B"]}]}"#,
                "tc-3",
                0,
            )
            .unwrap_err();
        assert!(error.to_string().contains("cannot re-propose"));

        let tp = &app
            .authoring_ui
            .artifacts
            .as_ref()
            .unwrap()
            .test_points
            .test_points[0];
        assert_eq!(tp.review_state, ReviewState::Approved);
        assert_eq!(tp.title, "Original");
    }

    #[test]
    fn re_proposing_preserves_existing_scenario_references() {
        let (mut app, _dir) = app_in_temp_project();
        app.execute_tool(
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "tc-1",
            0,
        )
        .unwrap();
        app.execute_tool(
            "propose_test_points",
            r#"{"test_points":[{"id":"tp-login","title":"Original","objective":"o","hierarchy_path":["A"]}]}"#,
            "tc-2",
            0,
        )
        .unwrap();
        app.authoring_ui
            .artifacts
            .as_mut()
            .unwrap()
            .test_points
            .test_points[0]
            .scenario_refs
            .push(ScenarioRef {
                feature_path: "sample.feature".into(),
                scenario_name: Some("Login".into()),
                scenario_line: Some(2),
            });

        app.execute_tool(
            "propose_test_points",
            r#"{"test_points":[{"id":"tp-login","title":"Updated","objective":"new","hierarchy_path":["B"]}]}"#,
            "tc-3",
            0,
        )
        .unwrap();

        let test_point = &app
            .authoring_ui
            .artifacts
            .as_ref()
            .unwrap()
            .test_points
            .test_points[0];
        assert_eq!(test_point.title, "Updated");
        assert_eq!(test_point.scenario_refs.len(), 1);
        assert_eq!(
            test_point.scenario_refs[0].scenario_name.as_deref(),
            Some("Login")
        );
    }

    #[test]
    fn approve_continue_then_generate_plan_accepts_approved_ids() {
        let (mut app, _dir) = app_in_temp_project();
        app.approval_mode = ApprovalMode::Auto;
        app.execute_tool(
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "tc-1",
            0,
        )
        .unwrap();
        app.execute_tool(
            "propose_test_points",
            r#"{"test_points":[{"id":"tp-login","title":"t","objective":"o","hierarchy_path":["A"]}]}"#,
            "tc-2",
            0,
        )
        .unwrap();

        // Reject generate_plan while still Proposed
        let err = app
            .execute_tool(
                "generate_plan",
                r#"{"features":[{"file_name":"a.feature","feature_name":"A","scenarios":[{"name":"S","steps":["Given x"],"test_point_ids":["tp-login"]}]}]}"#,
                "tc-x",
                0,
            )
            .unwrap_err();
        assert!(err.to_string().contains("blocked"));

        // Human approve via UI helper
        let artifacts = app.authoring_ui.artifacts.as_mut().unwrap();
        let tp = artifacts
            .test_points
            .test_points
            .iter_mut()
            .find(|tp| tp.id == "tp-login")
            .unwrap();
        assert!(crate::test_points_tab::TestPointsUiState::approve(tp));

        // Human continue (Auto mode must not matter)
        app.active_tab = crate::app::MainTab::TestPoints;
        app.handle_action(crate::keymap::Action::TpContinueGeneration)
            .unwrap();
        assert_eq!(app.generation_stage, GenerationStage::Planning);

        let ok = app
            .execute_tool(
                "generate_plan",
                r#"{"features":[{"file_name":"a.feature","feature_name":"A","scenarios":[{"name":"S","steps":["Given x"],"test_point_ids":["tp-login"]}]}]}"#,
                "tc-3",
                0,
            )
            .expect("plan after approval");
        assert!(ok.contains("Plan accepted"));
        assert_eq!(app.generation_stage, GenerationStage::Writing);
    }

    #[test]
    fn generate_plan_rejects_unknown_and_rejected_ids() {
        let (mut app, _dir) = app_in_temp_project();
        app.execute_tool(
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "tc-1",
            0,
        )
        .unwrap();
        app.execute_tool(
            "propose_test_points",
            r#"{"test_points":[{"id":"tp-1","title":"t","objective":"o","hierarchy_path":["A"]}]}"#,
            "tc-2",
            0,
        )
        .unwrap();
        let artifacts = app.authoring_ui.artifacts.as_mut().unwrap();
        let tp = &mut artifacts.test_points.test_points[0];
        assert!(crate::test_points_tab::TestPointsUiState::approve(tp));
        app.generation_stage = GenerationStage::Planning;

        let err = app
            .execute_tool(
                "generate_plan",
                r#"{"features":[{"file_name":"a.feature","feature_name":"A","scenarios":[{"name":"S","steps":["Given x"],"test_point_ids":["missing"]}]}]}"#,
                "tc-3",
                0,
            )
            .unwrap_err();
        assert!(err.to_string().contains("unknown"));

        // Reject then try plan
        let tp = &mut app
            .authoring_ui
            .artifacts
            .as_mut()
            .unwrap()
            .test_points
            .test_points[0];
        tp.review_state = ReviewState::Rejected;
        let err = app
            .execute_tool(
                "generate_plan",
                r#"{"features":[{"file_name":"a.feature","feature_name":"A","scenarios":[{"name":"S","steps":["Given x"],"test_point_ids":["tp-1"]}]}]}"#,
                "tc-4",
                0,
            )
            .unwrap_err();
        assert!(err.to_string().contains("Rejected") || err.to_string().contains("rejected"));
    }

    #[test]
    fn restart_restores_review_without_approving() {
        let (mut app, dir) = app_in_temp_project();
        app.execute_tool(
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "tc-1",
            0,
        )
        .unwrap();
        app.execute_tool(
            "propose_test_points",
            r#"{"test_points":[{"id":"tp-1","title":"t","objective":"o","hierarchy_path":["A"]}]}"#,
            "tc-2",
            0,
        )
        .unwrap();
        assert_eq!(app.generation_stage, GenerationStage::ReviewingTestPoints);

        // Simulate restart by reopening the same project root
        let feature = dir.path().join("sample.feature");
        let restored =
            crate::app::App::from_file(&feature, crate::config::load_config().unwrap()).unwrap();
        assert_eq!(
            restored.generation_stage,
            GenerationStage::ReviewingTestPoints
        );
        let tp = &restored
            .authoring_ui
            .artifacts
            .as_ref()
            .unwrap()
            .test_points
            .test_points
            .iter()
            .find(|tp| tp.id == "tp-1")
            .unwrap();
        assert_eq!(tp.review_state, ReviewState::Proposed);
    }

    #[test]
    fn continue_key_bound_on_test_points_tab() {
        use crate::app::{MainTab, MindMapFocus, ViewStage};
        use crate::authoring_tab::RequirementsFocus;
        use crate::keymap::{Action, KeyContext};
        use crate::test_points_tab::TestPointsFocus;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let context = KeyContext {
            step_keyword_picker_active: false,
            step_input_active: false,
            external_change_prompt_active: false,
            agent_change_prompt_active: false,
            active_tab: MainTab::TestPoints,
            view_stage: ViewStage::TreeOnly,
            explore_edit_mode: false,
            pending_char: None,
            mindmap_focus: MindMapFocus::Main,
            mindmap_ai_panel_visible: false,
            ai_input_focused: false,
            slash_suggestion_active: false,
            auth_panel_active: false,
            model_panel_active: false,
            model_panel_adding: false,
            session_panel_active: false,
            change_summary_visible: false,
            ai_status_waiting: false,
            scenario_dropdown_open: false,
            approval_panel_active: false,
            agent_profile_panel_active: false,
            requirements_focus: RequirementsFocus::Tree,
            test_points_focus: TestPointsFocus::Tree,
            quit_pending_confirm: false,
        };
        assert_eq!(
            Action::from_key_event(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
                context
            ),
            Some(Action::TpContinueGeneration)
        );
    }

    #[test]
    fn insert_scenario_encodes_teshi_tp_tags() {
        let (mut app, _dir) = app_in_temp_project();
        app.execute_tool(
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "tc-1",
            0,
        )
        .unwrap();
        app.execute_tool(
            "propose_test_points",
            r#"{"test_points":[
                {"id":"tp-1","title":"First","objective":"o","hierarchy_path":["A"]},
                {"id":"tp-2","title":"Second","objective":"o","hierarchy_path":["A"]}
            ]}"#,
            "tc-2",
            0,
        )
        .unwrap();
        for test_point in &mut app
            .authoring_ui
            .artifacts
            .as_mut()
            .unwrap()
            .test_points
            .test_points
        {
            test_point.review_state = ReviewState::Approved;
        }
        let file_path = app.project.features[0]
            .file_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        set_writing_plan(&mut app, &file_path, &[("Login", &["tp-1", "tp-2"])]);
        let args = format!(
            r#"{{"file_path":"{file_path}","scenario_name":"Login","steps":["Given x"],"tags":["@smoke"],"test_point_ids":["tp-1","tp-2"]}}"#
        );
        app.execute_tool("insert_scenario", &args, "tc-3", 0)
            .expect("insert");
        let pending = &app.pending_agent_changes[0];
        match &pending.mutation {
            crate::app::AgentMutation::InsertAfterLine { text, .. } => {
                assert!(text.contains("@teshi-tp:tp-1"));
                assert!(text.contains("@teshi-tp:tp-2"));
                assert!(text.contains("@smoke"));
                assert!(text.contains("Scenario: Login"));
            }
            _ => panic!("expected InsertAfterLine"),
        }
    }

    #[test]
    fn insert_scenario_rechecks_plan_and_current_test_point_state() {
        let (mut app, _dir) = app_in_temp_project();
        app.execute_tool(
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "tc-1",
            0,
        )
        .unwrap();
        app.execute_tool(
            "propose_test_points",
            r#"{"test_points":[{"id":"tp-login","title":"Login","objective":"o","hierarchy_path":["A"]}]}"#,
            "tc-2",
            0,
        )
        .unwrap();
        let file_path = app.project.features[0]
            .file_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        set_writing_plan(&mut app, &file_path, &[("Login", &["tp-login"])]);

        let unplanned = format!(
            r#"{{"file_path":"{file_path}","scenario_name":"Login","steps":["Given x"],"test_point_ids":["arbitrary"]}}"#
        );
        let error = app
            .execute_tool("insert_scenario", &unplanned, "tc-unplanned", 0)
            .unwrap_err();
        assert!(error.to_string().contains("exactly match"));

        let planned = format!(
            r#"{{"file_path":"{file_path}","scenario_name":"Login","steps":["Given x"],"test_point_ids":["tp-login"]}}"#
        );
        let error = app
            .execute_tool("insert_scenario", &planned, "tc-proposed", 0)
            .unwrap_err();
        assert!(error.to_string().contains("must be Approved"));

        let test_point = &mut app
            .authoring_ui
            .artifacts
            .as_mut()
            .unwrap()
            .test_points
            .test_points[0];
        test_point.review_state = ReviewState::Approved;
        test_point.requirement_links.push(RequirementLink {
            document_id: "doc-1".into(),
            document_revision: "rev-1".into(),
            position: TextRange::new(0, 5),
            quote: QuoteSelector {
                quote: "Login".into(),
                prefix: String::new(),
                suffix: String::new(),
            },
            resolution: ResolutionState::Stale,
        });

        let error = app
            .execute_tool("insert_scenario", &planned, "tc-stale", 0)
            .unwrap_err();
        assert!(error.to_string().contains("stale requirement links"));
        assert!(app.pending_agent_changes.is_empty());
    }

    #[test]
    fn scenario_refs_are_persisted_only_after_acceptance() {
        let (mut app, dir) = app_in_temp_project();
        app.execute_tool(
            "submit_requirements",
            r#"{"feature_name":"Auth","scenario_descriptions":["login"]}"#,
            "tc-1",
            0,
        )
        .unwrap();
        app.execute_tool(
            "propose_test_points",
            r#"{"test_points":[{"id":"tp-login","title":"Login","objective":"o","hierarchy_path":["A"]}]}"#,
            "tc-2",
            0,
        )
        .unwrap();
        let tp = &mut app
            .authoring_ui
            .artifacts
            .as_mut()
            .unwrap()
            .test_points
            .test_points[0];
        assert!(crate::test_points_tab::TestPointsUiState::approve(tp));

        let file_path = app.project.features[0]
            .file_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        set_writing_plan(
            &mut app,
            &file_path,
            &[
                ("Rejected Login", &["tp-login"]),
                ("Accepted Login", &["tp-login"]),
            ],
        );
        let rejected_args = format!(
            r#"{{"file_path":"{file_path}","scenario_name":"Rejected Login","steps":["Given x"],"test_point_ids":["tp-login"]}}"#
        );
        app.execute_tool("insert_scenario", &rejected_args, "tc-reject", 0)
            .unwrap();
        assert!(
            persisted_test_points(&dir).test_points[0]
                .scenario_refs
                .is_empty()
        );
        app.reject_agent_change();
        assert!(
            persisted_test_points(&dir).test_points[0]
                .scenario_refs
                .is_empty()
        );

        let accepted_args = format!(
            r#"{{"file_path":"{file_path}","scenario_name":"Accepted Login","steps":["Given x"],"test_point_ids":["tp-login"]}}"#
        );
        app.execute_tool("insert_scenario", &accepted_args, "tc-accept", 0)
            .unwrap();
        app.accept_agent_change().expect("accept scenario");

        let in_memory = &app
            .authoring_ui
            .artifacts
            .as_ref()
            .unwrap()
            .test_points
            .test_points[0]
            .scenario_refs;
        assert_eq!(in_memory.len(), 1);
        assert_eq!(
            in_memory[0].scenario_name.as_deref(),
            Some("Accepted Login")
        );
        let persisted = persisted_test_points(&dir);
        assert_eq!(persisted.test_points[0].scenario_refs, *in_memory);
    }
}
