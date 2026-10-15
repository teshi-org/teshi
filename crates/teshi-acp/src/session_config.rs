//! Agent-advertised session settings, independent of any frontend.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Wire operation required by a discovered setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigRoute {
    /// Modern session configuration API.
    Config,
    /// Legacy session mode API.
    Mode,
    /// Legacy session model API.
    Model,
}

/// One named value, optionally belonging to an agent-supplied group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigChoice {
    /// Value sent to the agent.
    pub value: Value,
    /// User-facing name.
    pub name: String,
    /// Optional explanatory text.
    pub description: Option<String>,
    /// Optional group heading.
    pub group: Option<String>,
}

/// A setting and its confirmed value. Unknown types are retained read-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigOption {
    /// Agent-supplied identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Help text.
    pub description: Option<String>,
    /// Semantic category; unknown categories remain supported.
    pub category: Option<String>,
    /// Advertised option type.
    pub kind: String,
    /// Last confirmed value.
    pub current: Value,
    /// Advertised values or the two boolean values.
    pub choices: Vec<ConfigChoice>,
    /// Mutation endpoint for this option.
    pub route: ConfigRoute,
}

impl ConfigOption {
    /// Display the selected value using its name where available.
    pub fn value_label(&self) -> String {
        self.choices
            .iter()
            .find(|c| c.value == self.current)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| {
                self.current
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| self.current.to_string())
            })
    }

    /// Whether a value is supported by this advertised option.
    pub fn accepts(&self, value: &Value) -> bool {
        matches!(self.kind.as_str(), "select" | "boolean")
            && self.choices.iter().any(|choice| &choice.value == value)
    }
}

/// Normalized options for one remote session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Agent order is retained, including unfamiliar categories.
    pub options: Vec<ConfigOption>,
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn choices(raw: &Value, group: Option<String>) -> Vec<ConfigChoice> {
    raw.as_array()
        .into_iter()
        .flatten()
        .flat_map(|item| {
            if let Some(nested) = item.get("options") {
                return choices(nested, string(item, "name"));
            }
            let Some(value) = item.get("value").filter(|v| v.is_string()) else {
                return vec![];
            };
            vec![ConfigChoice {
                value: value.clone(),
                name: string(item, "name")
                    .unwrap_or_else(|| value.as_str().unwrap_or_default().into()),
                description: string(item, "description"),
                group: group.clone(),
            }]
        })
        .collect()
}

impl SessionConfig {
    /// Read settings from a new/load session response.
    pub fn from_response(response: &Value) -> Self {
        let mut state = Self::default();
        state.replace_modern(response.get("configOptions"));
        for (key, category, available, current, id_key, route) in [
            (
                "modes",
                "mode",
                "availableModes",
                "currentModeId",
                "id",
                ConfigRoute::Mode,
            ),
            (
                "models",
                "model",
                "availableModels",
                "currentModelId",
                "modelId",
                ConfigRoute::Model,
            ),
        ] {
            if state
                .options
                .iter()
                .any(|o| o.category.as_deref() == Some(category) || o.id == category)
            {
                continue;
            }
            let Some(raw) = response.get(key) else {
                continue;
            };
            let values: Vec<_> = raw
                .get(available)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| {
                    let id = string(item, id_key)?;
                    Some(ConfigChoice {
                        value: Value::String(id.clone()),
                        name: string(item, "name").unwrap_or(id),
                        description: string(item, "description"),
                        group: None,
                    })
                })
                .collect();
            if values.is_empty() {
                continue;
            }
            state.options.push(ConfigOption {
                id: format!("legacy:{category}"),
                name: if category == "mode" { "Mode" } else { "Model" }.into(),
                description: None,
                category: Some(category.into()),
                kind: "select".into(),
                current: raw.get(current).cloned().unwrap_or(Value::Null),
                choices: values,
                route,
            });
        }
        state
    }

    /// Replace the full modern list when present, retaining non-overlapping legacy controls.
    pub fn replace_modern(&mut self, raw: Option<&Value>) {
        let Some(items) = raw.and_then(Value::as_array) else {
            return;
        };
        let legacy: Vec<_> = self
            .options
            .iter()
            .filter(|o| o.route != ConfigRoute::Config)
            .cloned()
            .collect();
        self.options = items
            .iter()
            .filter_map(|item| {
                let id = string(item, "id").filter(|id| !id.is_empty())?;
                let kind = string(item, "type").unwrap_or_else(|| "unknown".into());
                let values = match kind.as_str() {
                    "select" => choices(&item["options"], None),
                    "boolean" => [false, true]
                        .into_iter()
                        .map(|v| ConfigChoice {
                            value: Value::Bool(v),
                            name: if v { "On" } else { "Off" }.into(),
                            description: None,
                            group: None,
                        })
                        .collect(),
                    _ => vec![],
                };
                Some(ConfigOption {
                    name: string(item, "name").unwrap_or_else(|| id.clone()),
                    id,
                    description: string(item, "description"),
                    category: string(item, "category"),
                    kind,
                    current: item.get("currentValue").cloned().unwrap_or(Value::Null),
                    choices: values,
                    route: ConfigRoute::Config,
                })
            })
            .collect();
        for option in legacy {
            if !self.options.iter().any(|o| {
                o.category == option.category || Some(o.id.as_str()) == option.category.as_deref()
            }) {
                self.options.push(option);
            }
        }
    }

    /// Apply a notification only to its owning session.
    pub fn apply_update(&mut self, session_id: &str, params: &Value) -> bool {
        if params.get("sessionId").and_then(Value::as_str) != Some(session_id) {
            return false;
        }
        let update = &params["update"];
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("config_option_update") => {
                self.replace_modern(update.get("configOptions"));
                true
            }
            Some("current_mode_update") => {
                if let Some(value) = update.get("currentModeId") {
                    for option in &mut self.options {
                        if option.route == ConfigRoute::Mode {
                            option.current = value.clone();
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn grouped_boolean_unknown_and_legacy_options_are_retained() {
        let state = SessionConfig::from_response(&json!({"configOptions":[
            {"id":"model","name":"Model","category":"model","type":"select","currentValue":"a","options":[{"group":"g","name":"Provider","options":[{"value":"a","name":"A"}]}]},
            {"id":"fast","name":"Fast","type":"boolean","currentValue":false},
            {"id":"future","name":"Future","type":"range","currentValue":3}],
            "models":{"currentModelId":"old","availableModels":[{"modelId":"old","name":"Old"}]},
            "modes":{"currentModeId":"ask","availableModes":[{"id":"ask","name":"Ask"}]}}));
        assert_eq!(state.options.len(), 4);
        assert_eq!(
            state.options[0].choices[0].group.as_deref(),
            Some("Provider")
        );
        assert!(state.options[1].accepts(&json!(true)));
        assert!(!state.options[1].accepts(&json!("true")));
        assert!(!state.options[2].accepts(&json!(3)));
        assert_eq!(state.options[3].route, ConfigRoute::Mode);
    }

    #[test]
    fn updates_replace_choices_and_ignore_other_sessions() {
        let mut state = SessionConfig::from_response(
            &json!({"configOptions":[{"id":"effort","type":"select","currentValue":"high","options":[{"value":"high","name":"High"}]}]}),
        );
        let update = json!({"sessionId":"other","update":{"sessionUpdate":"config_option_update","configOptions":[]}});
        assert!(!state.apply_update("ours", &update));
        assert_eq!(state.options.len(), 1);
        assert!(state.apply_update("other", &update));
        assert!(state.options.is_empty());
    }
}
