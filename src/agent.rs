//! Agent module: tool definitions and execution for LLM function calling.
//!
//! Tools defined here can be registered with the LLM so it can inspect project
//! state and (in future) modify editor content. Each tool is side-effect-free
//! and read-only unless otherwise noted.

use anyhow::{Context, Result};

use crate::llm::ToolDefinition;

/// Returns the full list of available tool definitions for the LLM.
pub fn get_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "get_project_info".into(),
            description: "Get basic information about the current project, including \
                          the project directory path, number of feature files, \
                          and counts of scenarios and steps."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "highlight_mindmap_nodes".into(),
            description: "Highlight MindMap tree nodes whose step text matches a \
                          condition. Use this to visually mark nodes for the user. \
                          Multiple calls stack; new rules replace previous ones."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "match_condition": {
                        "type": "object",
                        "description": "Condition for matching nodes",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["step_contains"],
                                "description": "Match type: 'step_contains' matches nodes whose label contains the given text (case-insensitive)"
                            },
                            "text": {
                                "type": "string",
                                "description": "Substring to match in node labels"
                            }
                        },
                        "required": ["type", "text"]
                    },
                    "color": {
                        "type": "string",
                        "enum": ["red", "green", "yellow", "blue", "magenta", "cyan", "white"],
                        "description": "Color to highlight matching nodes"
                    }
                },
                "required": ["match_condition", "color"]
            }),
        },
        ToolDefinition {
            name: "apply_mindmap_filter".into(),
            description: "Filter the MindMap tree to show only nodes whose label \
                          contains a substring (plus their ancestors to preserve \
                          tree structure). Use 'clear' as filter_type to remove \
                          the active filter."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "filter_type": {
                        "type": "string",
                        "enum": ["name_contains", "clear"],
                        "description": "Filter type: 'name_contains' for substring match, 'clear' to remove the filter"
                    },
                    "value": {
                        "type": "string",
                        "description": "Substring to match (ignored when filter_type is 'clear')"
                    }
                },
                "required": ["filter_type"]
            }),
        },
    ]
}

/// Execute a named tool with the given JSON arguments and return the result
/// as plain text for the LLM.
pub fn execute_tool(app: &mut crate::app::App, name: &str, args_json: &str) -> Result<String> {
    match name {
        "get_project_info" => execute_get_project_info(app),
        "highlight_mindmap_nodes" => execute_highlight_mindmap_nodes(app, args_json),
        "apply_mindmap_filter" => execute_apply_mindmap_filter(app, args_json),
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
