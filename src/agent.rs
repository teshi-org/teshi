//! Agent module: tool definitions and execution for LLM function calling.
//!
//! Tools defined here can be registered with the LLM so it can inspect project
//! state and (in future) modify editor content. Each tool is side-effect-free
//! and read-only unless otherwise noted.

use anyhow::Result;

use crate::llm::ToolDefinition;

/// Returns the full list of available tool definitions for the LLM.
pub fn get_tools() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
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
    }]
}

/// Execute a named tool with the given JSON arguments and return the result
/// as plain text for the LLM.
pub fn execute_tool(app: &crate::app::App, name: &str, _args_json: &str) -> Result<String> {
    match name {
        "get_project_info" => execute_get_project_info(app),
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

fn execute_get_project_info(app: &crate::app::App) -> Result<String> {
    let project = &app.project;

    let total_scenarios: usize = project
        .features
        .iter()
        .map(|f| f.scenarios.len())
        .sum();
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
        .map(|f| {
            f.file_path
                .to_string_lossy()
                .to_string()
        })
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
