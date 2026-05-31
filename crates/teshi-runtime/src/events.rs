//! Runtime event bus for cross-panel notifications (Tauri emit or WebSocket broadcast).

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::broadcast;

/// Large enough for PTY screen redraw bursts (TUI apps, fast output).
const EVENT_CHANNEL_CAPACITY: usize = 4096;

/// Optional host callback (e.g. Tauri `emit`) for desktop shells.
pub type HostEventCallback = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// A single runtime event with a JSON payload.
#[derive(Debug, Clone)]
pub struct RuntimeEvent {
    /// Event name (matches legacy Tauri event names).
    pub name: String,
    /// Serialized payload.
    pub payload: serde_json::Value,
}

/// Broadcasts runtime events to web clients; optionally forwards to a host callback (Tauri).
#[derive(Clone)]
pub struct RuntimeEvents {
    tx: broadcast::Sender<RuntimeEvent>,
    host: Option<HostEventCallback>,
}

impl RuntimeEvents {
    /// Creates an event bus with optional host forwarding (e.g. Tauri `AppHandle::emit`).
    pub fn new(host: Option<HostEventCallback>) -> Self {
        let (tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self { tx, host }
    }

    /// Subscribes to all runtime events (used by `teshi web` WebSocket).
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.tx.subscribe()
    }

    /// Emits an event to subscribers and the optional host callback.
    pub fn emit<T: Serialize>(&self, name: &str, payload: T) {
        let value = serde_json::to_value(payload).unwrap_or(serde_json::Value::Null);
        if let Some(host) = &self.host {
            host(name, value.clone());
        }
        let _ = self.tx.send(RuntimeEvent {
            name: name.to_string(),
            payload: value,
        });
    }
}
