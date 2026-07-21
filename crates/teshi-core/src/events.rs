//! Runtime event payload types (pure data).
//! The event bus and host callbacks live in `teshi-engine`.

/// A single runtime event with a JSON payload.
#[derive(Debug, Clone)]
pub struct RuntimeEvent {
    /// Event name (matches legacy Tauri event names).
    pub name: String,
    /// Serialized payload.
    pub payload: serde_json::Value,
}
