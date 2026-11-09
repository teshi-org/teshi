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
    /// Workspace member directory the conversation ran in; absent when it ran
    /// in the project root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_dir: Option<PathBuf>,
    /// Unsent drafts survive restart, but are never automatically sent on restore.
    #[serde(default)]
    pub pending_messages: std::collections::VecDeque<String>,
    /// Assistant answers that completed a normal turn.
    ///
    /// Sessions written before this list existed stored one `completed_answer`
    /// (an index or `null`); accepting that shape keeps their green bar alive
    /// when an old conversation is loaded.
    #[serde(
        default,
        alias = "completed_answer",
        deserialize_with = "completed_answers_compat"
    )]
    pub completed_answers: Vec<usize>,
}

fn completed_answers_compat<'de, D>(deserializer: D) -> Result<Vec<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Shape {
        One(Option<usize>),
        Many(Vec<usize>),
    }
    Ok(match Shape::deserialize(deserializer)? {
        Shape::One(index) => index.into_iter().collect(),
        Shape::Many(indices) => indices,
    })
}

impl Session {
    /// Human-readable title: the first user line, or a short session id.
    pub fn display_title(&self) -> String {
        let first = self
            .messages
            .iter()
            .find(|message| message.role == crate::app::AiRole::User)
            .map(|message| message.content.as_str())
            .unwrap_or("");
        let line = first.lines().next().unwrap_or("").trim();
        if line.is_empty() {
            let short: String = self.id.chars().take(8).collect();
            format!("Session {short}")
        } else {
            line.chars().take(48).collect()
        }
    }

    /// On-disk size of this session, without loading its messages.
    pub fn file_size(&self) -> Option<u64> {
        std::fs::metadata(Self::storage_dir().join(format!("{}.json", self.id)))
            .ok()
            .map(|metadata| metadata.len())
    }

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
            member_dir: None,
            pending_messages: Default::default(),
            completed_answers: Vec::new(),
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
    fn single_answer_marker_from_older_sessions_still_loads() {
        let session: super::Session = serde_json::from_value(serde_json::json!({
            "id": "old", "created_at": "1", "updated_at": "1", "model_label": null,
            "message_count": 0, "messages": [], "completed_answer": 16
        }))
        .unwrap();
        assert_eq!(session.completed_answers, vec![16]);

        let idle: super::Session = serde_json::from_value(serde_json::json!({
            "id": "old", "created_at": "1", "updated_at": "1", "model_label": null,
            "message_count": 0, "messages": [], "completed_answer": null
        }))
        .unwrap();
        assert!(idle.completed_answers.is_empty());
    }

    #[test]
    fn answer_marker_list_round_trips() {
        let session: super::Session = serde_json::from_value(serde_json::json!({
            "id": "new", "created_at": "1", "updated_at": "1", "model_label": null,
            "message_count": 0, "messages": [], "completed_answers": [1, 3]
        }))
        .unwrap();
        assert_eq!(session.completed_answers, vec![1, 3]);
        let json = serde_json::to_value(&session).unwrap();
        assert_eq!(json["completed_answers"], serde_json::json!([1, 3]));
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
