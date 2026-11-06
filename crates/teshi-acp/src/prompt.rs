//! Content blocks carried by an ACP `session/prompt` request.

use serde_json::{Value, json};

/// One entry of a `session/prompt` request's `prompt` array.
///
/// Text is always accepted. Images are only valid when the agent advertised
/// `promptCapabilities.image` during `initialize`; see
/// [`crate::InitializeResult::accepts_images`].
#[derive(Clone, PartialEq, Eq)]
pub enum PromptBlock {
    /// Plain text.
    Text(String),
    /// An inline image, already base64-encoded.
    Image {
        /// MIME type of the decoded bytes, such as `image/png`.
        media_type: String,
        /// Standard base64 of the image bytes, without a `data:` prefix.
        data: String,
    },
}

impl PromptBlock {
    /// The block as ACP wire JSON.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Text(text) => json!({ "type": "text", "text": text }),
            Self::Image { media_type, data } => {
                json!({ "type": "image", "mimeType": media_type, "data": data })
            }
        }
    }

    /// Whether this block is an image.
    pub fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. })
    }
}

impl std::fmt::Debug for PromptBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            // Image payloads are large and may be private screenshots.
            Self::Image { media_type, data } => f
                .debug_struct("Image")
                .field("media_type", media_type)
                .field("base64_len", &data.len())
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocks_serialize_to_acp_content() {
        assert_eq!(
            PromptBlock::Text("hi".into()).to_json(),
            json!({"type": "text", "text": "hi"})
        );
        assert_eq!(
            PromptBlock::Image {
                media_type: "image/png".into(),
                data: "AAEC".into(),
            }
            .to_json(),
            json!({"type": "image", "mimeType": "image/png", "data": "AAEC"})
        );
    }

    #[test]
    fn test_image_debug_omits_payload() {
        let debug = format!(
            "{:?}",
            PromptBlock::Image {
                media_type: "image/png".into(),
                data: "c2VjcmV0".into(),
            }
        );
        assert!(!debug.contains("c2VjcmV0"), "{debug}");
        assert!(debug.contains("base64_len"));
    }
}
