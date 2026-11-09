//! Conversation-local presentation state and bounded streaming-rate samples.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use crate::app::{AiChatMessage, AiRole};
use teshi_core::llm::ToolCall;

/// Transient controls stay with their conversation, including unsent queued drafts.
#[derive(Debug, Default)]
pub(crate) struct ChatState {
    /// Changes invalidate cached layouts without touching durable message contents.
    pub revision: u64,
    /// Per-turn overrides of the default collapsed completed-process view.
    pub turns: HashMap<usize, bool>,
    /// Reasoning messages explicitly opened by the user.
    pub thoughts: HashSet<usize>,
    /// Individual external tool groups opened inside an expanded turn.
    pub groups: HashSet<usize>,
    /// Messages waiting for a successful completion, in submission order.
    pub queue: VecDeque<String>,
    /// Only a normal completion permits automatically submitting the next draft.
    pub queue_ready: bool,
    /// Most recent visible output or tool update, used by the inline wait indicator.
    pub last_activity: Option<Instant>,
    /// Bounded recent estimates; provider billing usage remains separate.
    pub rate: OutputRate,
    /// Sent-prompt recall position; `None` while the live draft is shown.
    pub recall: Option<Recall>,
}

/// Up/Down position in [`sent_prompts`], plus the draft to return to.
#[derive(Debug, Clone)]
pub(crate) struct Recall {
    /// Index into the list returned by [`sent_prompts`].
    pub index: usize,
    /// The interrupted draft with image placeholders expanded to mentions.
    pub draft: String,
}

/// Prompts sent in this conversation, oldest first, for composer recall.
///
/// Compaction digests carry a source and are skipped; consecutive repeats
/// collapse so holding Up never stalls on one prompt.
pub(crate) fn sent_prompts(messages: &[AiChatMessage]) -> Vec<&str> {
    let mut prompts: Vec<&str> = Vec::new();
    for message in messages
        .iter()
        .filter(|m| m.role == AiRole::User && m.source.is_none())
    {
        let text = message.content.as_str();
        if !text.trim().is_empty() && prompts.last() != Some(&text) {
            prompts.push(text);
        }
    }
    prompts
}

/// A rolling estimate based on visible text, never reported as provider usage.
#[derive(Debug, Default)]
pub(crate) struct OutputRate {
    samples: VecDeque<(Instant, f64)>,
}

impl OutputRate {
    /// Record a text delta with a bounded sample count, coalescing bursts.
    pub fn record(&mut self, text: &str, now: Instant) {
        let tokens = text
            .chars()
            .map(|c| if c.is_ascii() { 0.25 } else { 1.0 })
            .sum();
        if let Some((time, count)) = self.samples.back_mut()
            && now.saturating_duration_since(*time) < Duration::from_millis(100)
        {
            *count += tokens;
        } else {
            self.samples.push_back((now, tokens));
        }
        while self
            .samples
            .front()
            .is_some_and(|(time, _)| now.saturating_duration_since(*time) > Duration::from_secs(3))
        {
            self.samples.pop_front();
        }
    }

    /// Recent approximate tokens per second, hidden after two seconds without text.
    pub fn per_second(&self, now: Instant) -> Option<f64> {
        let last = self.samples.back()?.0;
        if now.saturating_duration_since(last) > Duration::from_secs(2) {
            return None;
        }
        let first = self.samples.front()?.0;
        let seconds = now.saturating_duration_since(first).as_secs_f64().max(0.5);
        Some(self.samples.iter().map(|(_, n)| n).sum::<f64>() / seconds)
    }

    /// A new turn or tool execution starts a fresh measurement window.
    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// One user turn and the final answer that stays visible when its process folds.
pub(crate) struct Turn {
    pub start: usize,
    pub end: usize,
    pub answer: Option<usize>,
}

/// Derive boundaries from durable user messages; incomplete turns have no foldable answer.
pub(crate) fn turns(messages: &[AiChatMessage]) -> Vec<Turn> {
    let starts: Vec<_> = messages
        .iter()
        .enumerate()
        .filter_map(|(i, m)| (m.role == AiRole::User).then_some(i))
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &start)| {
            let end = starts.get(n + 1).copied().unwrap_or(messages.len());
            let answer = end.checked_sub(1).filter(|&i| i > start).filter(|&i| {
                let m = &messages[i];
                m.role == AiRole::Assistant
                    && m.source.is_none()
                    && !m.content.is_empty()
                    && m.tool_calls.as_ref().is_none_or(|calls| calls.is_empty())
            });
            Turn { start, end, answer }
        })
        .collect()
}

fn scalar(text: &str) -> String {
    text.chars()
        .take(512)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn argument<'a>(args: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        args.get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
    })
}

/// A stable tool category from a native name or an explicitly advertised ACP kind.
pub(crate) fn category(name: &str) -> &'static str {
    match name {
        "read"
        | "read_file"
        | "read_files"
        | "list_directory"
        | "list_files"
        | "list_features"
        | "read_feature"
        | "get_requirements"
        | "get_project_info"
        | "get_workspace_info"
        | "get_feature_content"
        | "list_requirement_documents"
        | "read_requirement_document" => "reads",
        "search" | "search_files" | "grep" | "glob" | "search_features" => "searches",
        "edit"
        | "write_file"
        | "edit_file"
        | "apply_patch"
        | "create_feature"
        | "update_feature"
        | "submit_requirements"
        | "insert_scenario"
        | "update_step"
        | "create_feature_file"
        | "delete_scenario"
        | "rename_scenario"
        | "reorder_steps" => "edits",
        "execute" | "execute_command" | "run_command" | "shell" | "bash" | "pwsh"
        | "run_scenarios" | "run_tests" => "commands",
        "fetch" | "web_fetch" | "web_search" => "web calls",
        _ => "other calls",
    }
}

fn describe(name: &str, args: &serde_json::Value, path: Option<&str>) -> Option<String> {
    let path = path.or_else(|| {
        argument(
            args,
            &[
                "path",
                "file_path",
                "file",
                "feature_path",
                "file_name",
                "directory",
            ],
        )
    });
    let subject = match category(name) {
        "reads" => format!("Read {}", scalar(path?)),
        "edits" => format!("Edited {}", scalar(path?)),
        "searches" => {
            let pattern = scalar(argument(args, &["pattern", "query", "text"])?);
            match path {
                Some(p) => format!("Searched for {pattern} in {}", scalar(p)),
                None => format!("Searched for {pattern}"),
            }
        }
        "commands" => format!(
            "Ran {}",
            scalar(argument(args, &["command", "cmd", "script"])?)
        ),
        // External web URLs may contain credentials: leave their agent-supplied title alone.
        _ => return None,
    };
    Some(subject)
}

/// Explain a native call from its arguments, falling back to its actual tool name.
pub(crate) fn native_summary(call: &ToolCall) -> String {
    let args = serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
    let path = argument(&args, &["file_path", "file_name", "path", "feature_path"]).map(scalar);
    let scenario = argument(&args, &["scenario_name", "old_name"]).map(scalar);
    let specific = match call.name.as_str() {
        "get_project_info" => Some("Read project overview".into()),
        "list_requirement_documents" => Some("Listed requirement documents".into()),
        "read_requirement_document" => Some(format!(
            "Read requirement {}",
            scalar(argument(&args, &["document_id", "id"]).unwrap_or("document"))
        )),
        "create_feature_file" => path.as_ref().map(|p| format!("Created {p}")),
        "insert_scenario" | "delete_scenario" | "rename_scenario" | "reorder_steps"
        | "update_step" => scenario.map(|scenario| {
            let verb = match call.name.as_str() {
                "insert_scenario" => "Added scenario",
                "delete_scenario" => "Deleted scenario",
                "rename_scenario" => "Renamed scenario",
                "reorder_steps" => "Reordered steps in",
                _ => "Updated step in",
            };
            let location = path.as_ref().map(|p| format!(" ({p})")).unwrap_or_default();
            format!("{verb} {scenario}{location}")
        }),
        "run_tests" => Some(
            path.map(|p| format!("Ran tests in {p}"))
                .unwrap_or_else(|| "Ran project tests".into()),
        ),
        "search_features" => Some(format!(
            "Searched features{}",
            argument(&args, &["scenario_name", "step_content", "tag", "pattern"])
                .map(|p| format!(": {}", scalar(p)))
                .unwrap_or_default()
        )),
        "get_workspace_info" => Some("Inspected workspace".into()),
        "submit_requirements" => Some("Submitted requirement draft".into()),
        "propose_test_points" => Some("Proposed test points".into()),
        "generate_plan" => Some("Prepared feature generation plan".into()),
        "validate_feature" => Some(
            path.map(|p| format!("Validated {p}"))
                .unwrap_or_else(|| "Validated generated feature".into()),
        ),
        "observe_page" => Some("Observed browser page".into()),
        "browser_snapshot" => Some("Read browser snapshot".into()),
        "browser_click" => Some("Clicked browser element".into()),
        "browser_type" => Some("Entered browser text".into()),
        "browser_assert" => Some("Checked browser expectation".into()),
        "browser_go_back" => Some("Navigated browser back".into()),
        _ => None,
    };
    specific
        .or_else(|| describe(&call.name, &args, None))
        .unwrap_or_else(|| format!("Called {}", scalar(&call.name)))
}

/// Explain an external tool only when the agent provided structured facts.
pub(crate) fn external_summary(message: &AiChatMessage) -> String {
    let details = message.activity.as_ref();
    details
        .and_then(|d| {
            describe(
                d.kind.as_deref().unwrap_or(""),
                d.input.as_ref().unwrap_or(&serde_json::Value::Null),
                d.locations.first().map(String::as_str),
            )
        })
        .unwrap_or_else(|| scalar(&message.content))
}

/// Summarize process calls and reserve visible space for unsuccessful outcomes.
pub(crate) fn process_summary(messages: &[AiChatMessage]) -> String {
    let failed_calls: HashSet<&str> = messages
        .iter()
        .filter(|m| m.role == AiRole::Tool && m.content.starts_with("Error:"))
        .filter_map(|m| m.tool_call_id.as_deref())
        .collect();
    let mut counts = [0usize; 6];
    let mut failed = 0;
    let mut interrupted = 0;
    for message in messages {
        if message.source.as_deref() == Some("acp_activity") {
            let kind = message
                .activity
                .as_ref()
                .and_then(|d| d.kind.as_deref())
                .unwrap_or("");
            counts[category_index(category(kind))] += 1;
            match message.reasoning_content.as_deref() {
                Some("failed") => failed += 1,
                Some("cancelled" | "canceled") => interrupted += 1,
                _ => {}
            }
        } else if let Some(calls) = &message.tool_calls {
            for call in calls {
                counts[category_index(category(&call.name))] += 1;
                if failed_calls.contains(call.id.as_str()) {
                    failed += 1;
                }
            }
        }
    }
    let total: usize = counts.iter().sum();
    let detail = counts
        .iter()
        .zip([
            ("read", "reads"),
            ("search", "searches"),
            ("edit", "edits"),
            ("command", "commands"),
            ("web call", "web calls"),
            ("other call", "other calls"),
        ])
        .filter(|(n, _)| **n > 0)
        .map(|(n, (one, many))| format!("{n} {}", if *n == 1 { one } else { many }))
        .collect::<Vec<_>>()
        .join(" · ");
    let mut label = if total == 0 {
        "Activity".into()
    } else {
        format!(
            "{total} tool {}: {detail}",
            if total == 1 { "call" } else { "calls" }
        )
    };
    if failed > 0 {
        label.push_str(&format!(" · {failed} failed"));
    }
    if interrupted > 0 {
        label.push_str(&format!(" · {interrupted} interrupted"));
    }
    label
}

fn category_index(name: &str) -> usize {
    match name {
        "reads" => 0,
        "searches" => 1,
        "edits" => 2,
        "commands" => 3,
        "web calls" => 4,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_summaries_use_real_teshi_tool_arguments_and_keep_unknown_tools_honest() {
        for (name, args, expected) in [
            (
                "get_feature_content",
                r#"{"file_path":"features/login.feature"}"#,
                "Read features/login.feature",
            ),
            (
                "insert_scenario",
                r#"{"file_path":"login.feature","scenario_name":"Account locked"}"#,
                "Added scenario Account locked (login.feature)",
            ),
            (
                "run_tests",
                r#"{"feature_path":"login.feature"}"#,
                "Ran tests in login.feature",
            ),
            (
                "read_requirement_document",
                r#"{"document_id":"doc-55"}"#,
                "Read requirement doc-55",
            ),
            ("unknown_tool", "{}", "Called unknown_tool"),
        ] {
            let call = ToolCall {
                id: "call".into(),
                name: name.into(),
                arguments: args.into(),
                execution_duration_ms: None,
            };
            assert_eq!(native_summary(&call), expected);
        }
    }

    #[test]
    fn rate_is_bounded_unicode_aware_and_expires() {
        let mut rate = OutputRate::default();
        let start = Instant::now();
        rate.record("abcd中文", start);
        assert_eq!(rate.per_second(start + Duration::from_secs(1)), Some(3.0));
        assert_eq!(rate.per_second(start + Duration::from_secs(3)), None);
        for n in 0..1000 {
            rate.record("a", start + Duration::from_millis(n * 10));
        }
        assert!(rate.samples.len() <= 31);
        rate.clear();
        assert_eq!(rate.per_second(start), None);
    }
}
