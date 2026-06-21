//! Tool definitions registered with the LLM for function calling.
//!
//! Each tool is described by its name, natural-language description, and JSON Schema
//! parameters. The full list is returned by [`get_tools`] and passed to the LLM at
//! chat-request time.

use crate::llm::ToolDefinition;

/// Returns all tool definitions (unfiltered).
fn get_all_tools() -> Vec<ToolDefinition> {
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
        ToolDefinition {
            name: "get_feature_content".into(),
            description: "Return the full parsed content of a specific .feature file: \
                          feature name, description, background steps, all scenarios \
                          with their steps and line numbers. Use this before inserting \
                          or editing scenarios to understand the current file structure."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the target .feature file (e.g. 'features/login.feature')"
                    }
                },
                "required": ["file_path"]
            }),
        },
        ToolDefinition {
            name: "insert_scenario".into(),
            description: "Insert a new Scenario (or Scenario Outline) into a \
                          specified feature file. The change is staged in the \
                          editor buffer and requires user confirmation before \
                          being applied."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the target .feature file (e.g. 'features/login.feature')"
                    },
                    "scenario_name": {
                        "type": "string",
                        "description": "The name/title of the Scenario (e.g. 'Account locked after 3 failed attempts')"
                    },
                    "steps": {
                        "type": "array",
                        "description": "Ordered step lines (e.g. ['Given a registered user', 'When I enter an incorrect password 3 times', 'Then my account should be temporarily locked'])",
                        "items": {
                            "type": "string"
                        }
                    },
                    "insert_after_line": {
                        "type": "integer",
                        "description": "1-based line number after which to insert the scenario (omit to append at end of file)"
                    },
                    "tags": {
                        "type": "array",
                        "description": "Optional tags for the scenario (e.g. ['@smoke', '@security'])",
                        "items": {
                            "type": "string"
                        }
                    }
                },
                "required": ["file_path", "scenario_name", "steps"]
            }),
        },
        ToolDefinition {
            name: "update_step".into(),
            description: "Update the text body of a specific step within a named \
                          scenario. Finds the scenario by name, locates the step \
                          by 0-based index, and replaces its body text while \
                          preserving the keyword and indentation. The change is \
                          staged and requires user confirmation before being applied."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the target .feature file (e.g. 'features/login.feature')"
                    },
                    "scenario_name": {
                        "type": "string",
                        "description": "Exact name of the Scenario containing the step to update"
                    },
                    "step_index": {
                        "type": "integer",
                        "description": "0-based index of the step within the scenario (0 = first step)"
                    },
                    "new_text": {
                        "type": "string",
                        "description": "New body text for the step (without the keyword, e.g. 'I am on the home page')"
                    }
                },
                "required": ["file_path", "scenario_name", "step_index", "new_text"]
            }),
        },
        ToolDefinition {
            name: "create_feature_file".into(),
            description: "Create a new .feature file with the given name, feature name, \
                          optional description, tags, and background steps. The file is \
                          created in the project root directory. The change requires user \
                          confirmation before being applied."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_name": {
                        "type": "string",
                        "description": "Name of the new feature file (must end with .feature, e.g. 'authentication.feature')"
                    },
                    "feature_name": {
                        "type": "string",
                        "description": "The name/title of the Feature (e.g. 'User Authentication')"
                    },
                    "description": {
                        "type": "array",
                        "description": "Optional description lines for the feature",
                        "items": { "type": "string" }
                    },
                    "tags": {
                        "type": "array",
                        "description": "Optional tags for the feature (e.g. ['@smoke', '@auth'])",
                        "items": { "type": "string" }
                    },
                    "background_steps": {
                        "type": "array",
                        "description": "Optional background steps (e.g. ['Given a registered user'])",
                        "items": { "type": "string" }
                    }
                },
                "required": ["file_name", "feature_name"]
            }),
        },
        ToolDefinition {
            name: "delete_scenario".into(),
            description: "Delete a scenario (or scenario outline) from a feature file by name. \
                          The change is staged and requires user confirmation before being applied."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the target .feature file (e.g. 'features/login.feature')"
                    },
                    "scenario_name": {
                        "type": "string",
                        "description": "Exact name of the Scenario to delete"
                    }
                },
                "required": ["file_path", "scenario_name"]
            }),
        },
        ToolDefinition {
            name: "rename_scenario".into(),
            description: "Rename a scenario in a feature file. The change is staged and \
                          requires user confirmation before being applied."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the target .feature file (e.g. 'features/login.feature')"
                    },
                    "scenario_name": {
                        "type": "string",
                        "description": "Current exact name of the Scenario to rename"
                    },
                    "new_name": {
                        "type": "string",
                        "description": "New name for the scenario"
                    }
                },
                "required": ["file_path", "scenario_name", "new_name"]
            }),
        },
        ToolDefinition {
            name: "reorder_steps".into(),
            description: "Reorder the steps within a scenario. Provide the new order as \
                          a list of 0-based step indices. The change is staged and requires \
                          user confirmation before being applied."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path to the target .feature file (e.g. 'features/login.feature')"
                    },
                    "scenario_name": {
                        "type": "string",
                        "description": "Exact name of the Scenario whose steps to reorder"
                    },
                    "step_order": {
                        "type": "array",
                        "description": "New order of steps as 0-based indices (must be a permutation of 0..N-1)",
                        "items": { "type": "integer" }
                    }
                },
                "required": ["file_path", "scenario_name", "step_order"]
            }),
        },
        ToolDefinition {
            name: "search_features".into(),
            description: "Search across all feature files for scenarios matching optional \
                          filters: tag, step content, or scenario name. At least one filter \
                          must be provided. Returns matching scenarios with file path, line \
                          number, name, and step count."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "tag": {
                        "type": "string",
                        "description": "Filter: match scenarios that have this tag (e.g. '@smoke')"
                    },
                    "step_contains": {
                        "type": "string",
                        "description": "Filter: match scenarios that contain a step whose text includes this substring"
                    },
                    "scenario_name_contains": {
                        "type": "string",
                        "description": "Filter: match scenarios whose name includes this substring (case-insensitive)"
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "run_tests".into(),
            description: "Run the external test runner for scenarios in the project. \
                          Optionally filter by feature file path or scenario name. \
                          Returns a summary with passed/failed/skipped counts and details."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "feature_path": {
                        "type": "string",
                        "description": "Optional: only run scenarios from this .feature file"
                    },
                    "scenario_name": {
                        "type": "string",
                        "description": "Optional: only run scenarios with this exact name"
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "submit_requirements".into(),
            description: "Submit gathered requirements for a new feature and advance to the \
                          planning phase. Call this after asking the user enough questions \
                          to understand what they want to build."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "feature_name": {
                        "type": "string",
                        "description": "Name of the feature (e.g. 'User Authentication')"
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional description/user story for the feature"
                    },
                    "scenario_descriptions": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Short descriptions of each scenario to include"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional tags for the feature (e.g. ['@auth', '@smoke'])"
                    }
                },
                "required": ["feature_name", "scenario_descriptions"]
            }),
        },
        ToolDefinition {
            name: "generate_plan".into(),
            description: "Submit a complete scenario plan based on gathered requirements. \
                          Call this AFTER submit_requirements to propose the full structure \
                          including file names, scenarios, steps, and Examples tables."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "features": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "file_name": { "type": "string", "description": "Feature file name (must end with .feature)" },
                                "feature_name": { "type": "string", "description": "The feature title" },
                                "tags": { "type": "array", "items": { "type": "string" }, "description": "Feature-level tags" },
                                "background_steps": { "type": "array", "items": { "type": "string" }, "description": "Optional background steps" },
                                "scenarios": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "is_outline": { "type": "boolean", "description": "True for Scenario Outline" },
                                            "name": { "type": "string" },
                                            "tags": { "type": "array", "items": { "type": "string" } },
                                            "steps": { "type": "array", "items": { "type": "string" } },
                                            "examples_headers": { "type": "array", "items": { "type": "string" } },
                                            "examples_rows": {
                                                "type": "array",
                                                "items": { "type": "array", "items": { "type": "string" } }
                                            }
                                        },
                                        "required": ["name", "steps"]
                                    }
                                }
                            },
                            "required": ["file_name", "feature_name", "scenarios"]
                        }
                    }
                },
                "required": ["features"]
            }),
        },
        ToolDefinition {
            name: "validate_feature".into(),
            description: "Validate one or all feature files for Gherkin best practices. \
                          Checks: Given/When/Then ordering, missing Examples tables, \
                          duplicate scenario names, step count warnings."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Optional: validate only this file (e.g. 'login.feature'). If omitted, validates all files."
                    }
                },
                "required": []
            }),
        },
        // ── Browser agent exploration tools ──
        ToolDefinition {
            name: "browser_snapshot".into(),
            description: "Get a structured snapshot of the current browser page, \
                          listing all interactive elements with their teshi-id ref, \
                          role, accessible name, and element type. Use this before \
                          taking any action to understand the current page state."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "browser_click".into(),
            description: "Click an interactive element identified by its teshi-id ref. \
                          Use browser_snapshot first to discover available refs. \
                          Returns success or an error if the ref is not found."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "The teshi-id ref of the element to click (e.g. 'e15')"
                    }
                },
                "required": ["ref"]
            }),
        },
        ToolDefinition {
            name: "browser_type".into(),
            description: "Type text into an input element identified by its teshi-id ref. \
                          The element must be an input, textarea, or contenteditable element. \
                          Use browser_snapshot first to discover the correct ref."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "The teshi-id ref of the input element (e.g. 'e22')"
                    },
                    "text": {
                        "type": "string",
                        "description": "The text to type into the element"
                    }
                },
                "required": ["ref", "text"]
            }),
        },
        ToolDefinition {
            name: "browser_assert".into(),
            description: "Assert a condition on the current browser page. \
                          Supported types: 'text_visible' checks if a text substring \
                          is visible on the page; 'url_match' checks if the current URL \
                          matches a regex pattern. Returns success or failure with details."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "condition_type": {
                        "type": "string",
                        "enum": ["text_visible", "url_match"],
                        "description": "Type of assertion: 'text_visible' checks page text, 'url_match' checks URL regex"
                    },
                    "value": {
                        "type": "string",
                        "description": "The text substring to find (for text_visible) or regex pattern (for url_match)"
                    }
                },
                "required": ["condition_type", "value"]
            }),
        },
        ToolDefinition {
            name: "browser_go_back".into(),
            description: "Navigate the browser back one page in history. \
                          Use this if the agent navigated to an unexpected page \
                          or needs to return to a previous state."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    ]
}

/// Returns tool definitions for the LLM, optionally filtered by `allowed` list.
///
/// When `allowed` is `None` or empty, all tools are returned.
pub fn get_tools(allowed: Option<&[String]>) -> Vec<ToolDefinition> {
    let all = get_all_tools();
    match allowed {
        Some(list) if !list.is_empty() => {
            all.into_iter().filter(|t| list.contains(&t.name)).collect()
        }
        _ => all,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_all_tools_contains_browser_tools() {
        let tools = get_all_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"browser_snapshot"));
        assert!(names.contains(&"browser_click"));
        assert!(names.contains(&"browser_type"));
        assert!(names.contains(&"browser_assert"));
        assert!(names.contains(&"browser_go_back"));
    }

    #[test]
    fn test_browser_click_schema_has_ref_required() {
        let tools = get_all_tools();
        let click = tools.iter().find(|t| t.name == "browser_click").unwrap();
        let params = click.parameters.as_object().unwrap();
        let required = params["required"].as_array().unwrap();
        assert!(
            required.iter().any(|v| v == "ref"),
            "browser_click should require 'ref'"
        );
        assert_eq!(params["type"], "object");
    }

    #[test]
    fn test_browser_type_schema_has_ref_and_text_required() {
        let tools = get_all_tools();
        let type_tool = tools.iter().find(|t| t.name == "browser_type").unwrap();
        let params = type_tool.parameters.as_object().unwrap();
        let required: Vec<&str> = params["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"ref"));
        assert!(required.contains(&"text"));
    }

    #[test]
    fn test_browser_assert_schema_has_enum() {
        let tools = get_all_tools();
        let assert_tool = tools.iter().find(|t| t.name == "browser_assert").unwrap();
        let params = assert_tool.parameters.as_object().unwrap();
        let condition_type = &params["properties"]["condition_type"];
        let enum_values: Vec<&str> = condition_type["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(enum_values.contains(&"text_visible"));
        assert!(enum_values.contains(&"url_match"));
    }

    #[test]
    fn test_get_tools_filters_by_name() {
        let all = get_all_tools();
        let allowed = vec!["browser_snapshot".to_string(), "browser_click".to_string()];
        let filtered = get_tools(Some(&allowed));
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].name, "browser_snapshot");
        assert_eq!(filtered[1].name, "browser_click");
    }

    #[test]
    fn test_get_tools_empty_allowed_returns_all() {
        let all = get_all_tools();
        let empty: Vec<String> = vec![];
        let result = get_tools(Some(&empty));
        assert_eq!(result.len(), all.len());
    }

    #[test]
    fn test_get_tools_none_returns_all() {
        let all = get_all_tools();
        let result = get_tools(None);
        assert_eq!(result.len(), all.len());
    }

    #[test]
    fn test_browser_snapshot_has_no_required_params() {
        let tools = get_all_tools();
        let snap = tools.iter().find(|t| t.name == "browser_snapshot").unwrap();
        let params = snap.parameters.as_object().unwrap();
        let required = params["required"].as_array().unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn test_browser_go_back_has_no_required_params() {
        let tools = get_all_tools();
        let go_back = tools.iter().find(|t| t.name == "browser_go_back").unwrap();
        let params = go_back.parameters.as_object().unwrap();
        let required = params["required"].as_array().unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn test_all_tools_have_valid_json_schema() {
        let tools = get_all_tools();
        for tool in &tools {
            let params = tool
                .parameters
                .as_object()
                .unwrap_or_else(|| panic!("tool '{}' parameters must be an object", tool.name));
            assert_eq!(
                params["type"], "object",
                "tool '{}' schema must have type 'object'",
                tool.name
            );
            assert!(
                params.contains_key("properties"),
                "tool '{}' schema must have properties",
                tool.name
            );
            assert!(
                params.contains_key("required"),
                "tool '{}' schema must have required",
                tool.name
            );
        }
    }
}
