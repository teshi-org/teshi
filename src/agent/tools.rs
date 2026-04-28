//! Tool definitions registered with the LLM for function calling.
//!
//! Each tool is described by its name, natural-language description, and JSON Schema
//! parameters. The full list is returned by [`get_tools`] and passed to the LLM at
//! chat-request time.

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
    ]
}
