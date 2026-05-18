use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::app::AiChatMessage;

/// A saved AI chat session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub model_label: Option<String>,
    pub message_count: usize,
    pub messages: Vec<AiChatMessage>,
}

impl Session {
    /// Directory where session JSON files are stored.
    pub fn storage_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("teshi")
            .join("sessions")
    }

    /// Full path to this session's JSON file.
    pub fn file_path(&self) -> PathBuf {
        Self::storage_dir().join(format!("{}.json", self.id))
    }

    /// Save this session to disk as a JSON file.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let dir = Self::storage_dir();
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(self.file_path(), json)?;
        Ok(())
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
        // Sort by created_at descending (newest first)
        sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
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
