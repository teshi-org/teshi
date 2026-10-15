use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::app::AiChatMessage;

const SESSION_DIR_ENV: &str = "TESHI_SESSION_DIR";

/// A saved AI chat session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub model_label: Option<String>,
    pub message_count: usize,
    pub messages: Vec<AiChatMessage>,
    /// Project owning this conversation; absent in legacy sessions.
    #[serde(default)]
    pub project_root: Option<PathBuf>,
    /// Backend selection only; credentials and transient ACP settings are excluded.
    #[serde(default)]
    pub backend_kind: Option<teshi_agent::backend::AgentBackendKind>,
    /// Installed ACP agent ID used for this conversation.
    #[serde(default)]
    pub acp_agent_id: Option<String>,
}

impl Session {
    /// Directory where session JSON files are stored.
    pub fn storage_dir() -> PathBuf {
        if let Ok(override_dir) = std::env::var(SESSION_DIR_ENV) {
            let trimmed = override_dir.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("teshi")
            .join("sessions")
    }

    /// Create a new session from messages, generating a unique ID.
    pub fn from_messages(messages: Vec<AiChatMessage>, model_label: Option<String>) -> Self {
        let now = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let id = format!("{:x}", now.as_nanos());
        let ts = now.as_millis().to_string();
        let message_count = messages.len();
        Self {
            id,
            created_at: ts.clone(),
            updated_at: ts,
            model_label,
            message_count,
            messages,
            project_root: None,
            backend_kind: None,
            acp_agent_id: None,
        }
    }

    /// Load a session from a JSON file path.
    pub fn load_from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        let session: Session = serde_json::from_str(&json)?;
        Ok(session)
    }

    /// Load all sessions from the storage directory, sorted newest-first.
    pub fn load_all() -> Vec<Self> {
        let dir = Self::storage_dir();
        if !dir.exists() {
            return Vec::new();
        }
        let mut sessions: Vec<Self> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json")
                    && let Ok(session) = Self::load_from_file(&path)
                {
                    sessions.push(session);
                }
            }
        }
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        sessions
    }

    /// Delete a session by its ID.
    pub fn delete(id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::storage_dir().join(format!("{id}.json"));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use tempfile::tempdir;

    #[test]
    fn legacy_sessions_without_metadata_remain_readable() {
        let session: super::Session = serde_json::from_value(serde_json::json!({
            "id": "old", "created_at": "1", "updated_at": "1", "model_label": null,
            "message_count": 0, "messages": []
        }))
        .unwrap();
        assert!(session.project_root.is_none());
        assert!(session.backend_kind.is_none());
        assert!(session.acp_agent_id.is_none());
    }

    #[test]
    fn storage_dir_uses_non_empty_environment_override() {
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = std::env::var_os(super::SESSION_DIR_ENV);
        let temp = tempdir().unwrap();
        unsafe { std::env::set_var(super::SESSION_DIR_ENV, temp.path()) };

        assert_eq!(super::Session::storage_dir(), temp.path());

        match previous {
            Some(value) => unsafe { std::env::set_var(super::SESSION_DIR_ENV, value) },
            None => unsafe { std::env::remove_var(super::SESSION_DIR_ENV) },
        }
    }

    #[test]
    fn storage_dir_ignores_empty_environment_override() {
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = std::env::var_os(super::SESSION_DIR_ENV);
        unsafe { std::env::set_var(super::SESSION_DIR_ENV, "   ") };

        assert_ne!(super::Session::storage_dir(), PathBuf::from("   "));

        match previous {
            Some(value) => unsafe { std::env::set_var(super::SESSION_DIR_ENV, value) },
            None => unsafe { std::env::remove_var(super::SESSION_DIR_ENV) },
        }
    }
}
