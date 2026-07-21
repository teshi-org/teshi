//! LLM chat message and tool-call DTOs (pure data).
//! HTTP transport and streaming live in `teshi-tui` and `teshi-engine`.

use serde::{Deserialize, Serialize};

/// A structured chat message with role, content, and optional tool fields.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// One of `"user"`, `"assistant"`, `"system"`, or `"tool"`.
    pub role: String,
    /// The message content (may be empty for assistant messages that only
    /// contain tool calls).
    pub content: String,
    /// Tool calls included in an assistant message.
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The tool call ID this message responds to (for `role: "tool"`).
    pub tool_call_id: Option<String>,
    /// DeepSeek V4 thinking chain — must be preserved across tool-call turns.
    pub reasoning_content: Option<String>,
}

/// A tool definition conforming to OpenAI's function-calling JSON Schema format.
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    /// The name of the function (a-z, A-Z, 0-9, underscores, dashes).
    pub name: String,
    /// A description of what the function does.
    pub description: String,
    /// JSON Schema for the function parameters.
    pub parameters: serde_json::Value,
}

/// A tool call request returned by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique identifier for this tool call.
    pub id: String,
    /// The name of the function to call.
    pub name: String,
    /// JSON-encoded arguments for the function.
    pub arguments: String,
    /// Execution duration in milliseconds, set after tool completes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_duration_ms: Option<u64>,
}
