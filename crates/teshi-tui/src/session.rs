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
    /// Remote ACP `sessionId` for `session/resume` or `session/load`.
    ///
    /// Distinct from Teshi's local [`Session::id`] file name. Absent in legacy
    /// checkpoints and native conversations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_session_id: Option<String>,
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
    /// Title produced by the automatic naming call after a finished turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_title: Option<String>,
    /// Title published by the external ACP agent; outranks the generated one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_title: Option<String>,
    /// Title set by the user with `/rename`; outranks every automatic source
    /// and pauses further automatic refreshes while set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    /// Whether the user marked this conversation as Settled (archived in the sidebar).
    #[serde(default)]
    pub settled: bool,
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
    /// Human-readable title.
    ///
    /// A manual rename wins, then the agent's own title, then the generated
    /// one. Sessions without any stored title keep the historical behavior:
    /// the first user line, or a short session id.
    pub fn display_title(&self) -> String {
        if let Some(title) = self
            .custom_title
            .as_deref()
            .or(self.agent_title.as_deref())
            .or(self.generated_title.as_deref())
            .map(str::trim)
            .filter(|title| !title.is_empty())
        {
            return title.to_string();
        }
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
            acp_session_id: None,
            member_dir: None,
            pending_messages: Default::default(),
            completed_answers: Vec::new(),
            generated_title: None,
            agent_title: None,
            custom_title: None,
            settled: false,
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

    /// Load a session by ID from the storage directory.
    pub fn load_by_id(id: &str) -> Option<Self> {
        let path = Self::storage_dir().join(format!("{id}.json"));
        Self::load_from_file(&path).ok()
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

/// Uncommitted ACP conversation tab (zero local messages) persisted inside
/// [`SidebarWorkspaceState`] so its remote `acp_session_id` survives restart
/// without creating an empty `Session` file in `sessions/*.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingAcpWorkspaceTab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_agent_id: Option<String>,
    pub acp_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    #[serde(default)]
    pub settled: bool,
}

/// Per-project persisted AI sidebar workspace state (open cards, active card, drawer flags).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SidebarWorkspaceState {
    #[serde(default)]
    pub open_session_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_acp_tabs: Vec<PendingAcpWorkspaceTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_pending_acp_index: Option<usize>,
    #[serde(default)]
    pub sidebar_collapsed: bool,
    #[serde(default)]
    pub settled_expanded: bool,
    #[serde(default)]
    pub explore_details_collapsed: bool,
}

impl SidebarWorkspaceState {
    fn project_file(project_root: &std::path::Path) -> PathBuf {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let identity = teshi_engine::ProjectIdentity::new(project_root)
            .unwrap_or_else(|_| teshi_engine::ProjectIdentity::from_root(project_root));
        let mut hasher = DefaultHasher::new();
        identity.root().hash(&mut hasher);
        Session::storage_dir()
            .join("workspaces")
            .join(format!("{:016x}.json", hasher.finish()))
    }

    /// Load persisted sidebar workspace state for `project_root`, if present and readable.
    pub fn load_for_project(project_root: &std::path::Path) -> Option<Self> {
        let json = std::fs::read_to_string(Self::project_file(project_root)).ok()?;
        serde_json::from_str(&json).ok()
    }

    /// Atomically persist sidebar workspace state for `project_root`.
    pub fn save_for_project(project_root: &std::path::Path, state: &Self) -> anyhow::Result<()> {
        let path = Self::project_file(project_root);
        teshi_engine::write_atomic(&path, state)
    }
}

#[cfg(test)]
pub(crate) fn with_temp_session_dir<R>(f: impl FnOnce(&std::path::Path) -> R) -> R {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    struct Restore(Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => unsafe { std::env::set_var(SESSION_DIR_ENV, value) },
                None => unsafe { std::env::remove_var(SESSION_DIR_ENV) },
            }
        }
    }
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = tempfile::tempdir().unwrap();
    let _restore = Restore(std::env::var_os(SESSION_DIR_ENV));
    unsafe { std::env::set_var(SESSION_DIR_ENV, temp.path()) };
    f(temp.path())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    #[test]
    fn sidebar_workspace_state_round_trips_per_project() {
        super::with_temp_session_dir(|_| {
            let proj_a = tempdir().unwrap();
            let proj_b = tempdir().unwrap();
            assert!(super::SidebarWorkspaceState::load_for_project(proj_a.path()).is_none());

            let state_a = super::SidebarWorkspaceState {
                open_session_ids: vec!["s1".into(), "s2".into()],
                active_session_id: Some("s2".into()),
                pending_acp_tabs: vec![super::PendingAcpWorkspaceTab {
                    acp_agent_id: Some("antigravity-acp".into()),
                    acp_session_id: "remote-1".into(),
                    member_dir: None,
                    agent_title: Some("Draft".into()),
                    custom_title: None,
                    settled: false,
                }],
                active_pending_acp_index: Some(0),
                sidebar_collapsed: false,
                settled_expanded: true,
                explore_details_collapsed: true,
            };
            super::SidebarWorkspaceState::save_for_project(proj_a.path(), &state_a).unwrap();

            assert_eq!(
                super::SidebarWorkspaceState::load_for_project(proj_a.path()),
                Some(state_a)
            );
            assert!(super::SidebarWorkspaceState::load_for_project(proj_b.path()).is_none());

            let legacy: super::SidebarWorkspaceState = serde_json::from_value(serde_json::json!({
                "open_session_ids": ["s1"],
                "active_session_id": "s1",
                "sidebar_collapsed": true
            }))
            .unwrap();
            assert!(legacy.pending_acp_tabs.is_empty());
            assert_eq!(legacy.active_pending_acp_index, None);
        });
    }

    #[test]
    fn display_title_prefers_custom_then_agent_then_generated() {
        let mut session: super::Session = serde_json::from_value(serde_json::json!({
            "id": "titled", "created_at": "1", "updated_at": "1", "model_label": null,
            "message_count": 1, "messages": [{
                "role": "User", "content": "first question", "tool_calls": null,
                "tool_call_id": null, "reasoning_content": null, "source": null
            }]
        }))
        .unwrap();
        assert_eq!(session.display_title(), "first question");

        session.generated_title = Some("Generated".into());
        assert_eq!(session.display_title(), "Generated");

        session.agent_title = Some("Agent title".into());
        assert_eq!(session.display_title(), "Agent title");

        session.custom_title = Some("Manual".into());
        assert_eq!(session.display_title(), "Manual");

        // Clearing the manual title falls back to the next source, not the
        // first user line.
        session.custom_title = None;
        assert_eq!(session.display_title(), "Agent title");
        session.agent_title = None;
        assert_eq!(session.display_title(), "Generated");
    }

    #[test]
    fn legacy_sessions_without_titles_still_load() {
        let session: super::Session = serde_json::from_value(serde_json::json!({
            "id": "old", "created_at": "1", "updated_at": "1", "model_label": null,
            "message_count": 0, "messages": []
        }))
        .unwrap();
        assert!(session.generated_title.is_none());
        assert!(session.agent_title.is_none());
        assert!(session.custom_title.is_none());
    }

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
        assert!(session.acp_session_id.is_none());
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
        super::with_temp_session_dir(|dir| {
            assert_eq!(super::Session::storage_dir(), dir);
        });
    }

    #[test]
    fn storage_dir_ignores_empty_environment_override() {
        super::with_temp_session_dir(|_| {
            unsafe { std::env::set_var(super::SESSION_DIR_ENV, "   ") };
            assert_ne!(super::Session::storage_dir(), PathBuf::from("   "));
        });
    }
}
