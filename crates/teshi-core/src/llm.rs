//! LLM chat message and tool-call DTOs (pure data).
//! HTTP transport and streaming live in `teshi-tui` and `teshi-engine`.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Provider-neutral content for a chat message.
#[derive(Clone, PartialEq, Eq)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

impl From<String> for MessageContent {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for MessageContent {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl MessageContent {
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Blocks(_) => None,
        }
    }
}

impl fmt::Debug for MessageContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            Self::Blocks(blocks) => f.debug_tuple("Blocks").field(blocks).finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ContentBlock {
    Text { text: String },
    Image { source: ImageSource },
}

impl fmt::Debug for ContentBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text { text } => f.debug_struct("Text").field("text", text).finish(),
            Self::Image { source } => f.debug_struct("Image").field("source", source).finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ImageSource {
    Url(String),
    Data { media_type: String, data: Arc<[u8]> },
}

impl fmt::Debug for ImageSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(url) => f.debug_tuple("Url").field(url).finish(),
            Self::Data { media_type, data } => f
                .debug_struct("Data")
                .field("media_type", media_type)
                .field("bytes", &format_args!("<redacted {} bytes>", data.len()))
                .finish(),
        }
    }
}

/// A structured chat message with role, content, and optional tool fields.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// One of `"user"`, `"assistant"`, `"system"`, or `"tool"`.
    pub role: String,
    /// The message content (may be empty for assistant messages that only
    /// contain tool calls).
    pub content: MessageContent,
    /// Tool calls included in an assistant message.
    pub tool_calls: Option<Vec<ToolCall>>,
    /// The tool call ID this message responds to (for `role: "tool"`).
    pub tool_call_id: Option<String>,
    /// DeepSeek V4 thinking chain — must be preserved across tool-call turns.
    pub reasoning_content: Option<String>,
}

impl ChatMessage {
    pub fn text(role: impl Into<String>, content: impl Into<MessageContent>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_conversions_and_order_are_preserved() {
        assert_eq!(MessageContent::from("hello").text(), Some("hello"));
        assert_eq!(
            MessageContent::from(String::from("world")).text(),
            Some("world")
        );
        let content = MessageContent::Blocks(vec![
            ContentBlock::Text { text: "a".into() },
            ContentBlock::Image {
                source: ImageSource::Url("https://example.test/a.png".into()),
            },
            ContentBlock::Text { text: "b".into() },
        ]);
        assert!(
            matches!(content, MessageContent::Blocks(ref blocks) if matches!(blocks[1], ContentBlock::Image { .. }))
        );
    }

    #[test]
    fn image_debug_redacts_binary_data() {
        let message = ChatMessage::text(
            "user",
            MessageContent::Blocks(vec![ContentBlock::Image {
                source: ImageSource::Data {
                    media_type: "image/png".into(),
                    data: Arc::from([0_u8, 1, 2, 255]),
                },
            }]),
        );
        let debug = format!("{message:?}");
        assert!(debug.contains("<redacted 4 bytes>"));
        assert!(!debug.contains("255"));
    }
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
