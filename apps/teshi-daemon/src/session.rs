//! Session-based sandbox isolation for daemon API.
//!
//! Each API request carries an `X-Teshi-Token` header binding it to a role.
//! The daemon's auth middleware checks the token's role against a permission
//! whitelist before allowing any operation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Roles with different permission scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Full access — all API operations allowed.
    Admin,
    /// Limited to read-only snapshot, highlight, and locator proposals.
    AgentRecorder,
    /// Batch runner — can execute scenarios but not modify project structure.
    BatchRunner,
    /// Hosted GPUI Web UI session. This is intentionally separate from Admin;
    /// WebSocket authorization is capability-based and never inferred from a
    /// loopback TCP peer.
    HostedWebUi,
}

impl Role {
    /// Returns `true` if this role may call the given API path.
    ///
    /// Paths not in the whitelist are rejected with HTTP 403.
    pub fn can_execute(&self, path: &str) -> bool {
        match self {
            Role::Admin => true,
            Role::AgentRecorder => matches!(
                path,
                "/api/v1/events"
                    | "/api/v1/locator/active-step"
                    | "/api/v1/locator/pending"
                    | "/api/v1/locator/confirm"
                    | "/api/v1/locator/reject"
                    | "/api/v1/locator/highlight"
                    | "/api/v1/locator/sync-step"
                    | "/api/v1/steps/statuses"
                    | "/api/v1/steps/unbind"
                    | "/api/v1/gherkin/render"
                    | "/api/v1/gherkin/scenarios"
                    | "/api/v1/api/exchange"
                    | "/api/v1/fs/list"
                    | "/api/v1/fs/read"
                    | "/api/v1/settings/project"
            ),
            Role::BatchRunner => matches!(
                path,
                "/api/v1/daemon/run"
                    | "/api/v1/events"
                    | "/api/v1/projects/switch-allowed"
                    | "/api/v1/settings/recent"
                    | "/api/v1/settings/project"
                    | "/api/v1/gherkin/render"
                    | "/api/v1/gherkin/scenarios"
                    | "/api/v1/api/exchange"
                    | "/api/v1/fs/list"
                    | "/api/v1/fs/read"
                    | "/api/v1/steps/statuses"
            ),
            // Hosted pages use the authenticated WebSocket protocols only.
            // Keeping this role out of the legacy REST allowlist prevents a
            // leaked hosted token from reaching raw browser, filesystem, or
            // exchange responses. Local/Admin REST clients remain supported.
            Role::HostedWebUi => false,
        }
    }
}

/// A session binding a unique token to a role with optional metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub role: Role,
    pub created_at_secs: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

/// Thread-safe in-memory session store.
///
/// Tokens are UUIDv4 strings prefixed with `tk_` for easy visual identification.
/// No external database is required — sessions live in process memory.
#[derive(Debug, Clone)]
pub struct SessionStore {
    inner: Arc<Mutex<HashMap<String, Session>>>,
}

impl SessionStore {
    /// Create a new empty session store.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a new session with the given role.
    ///
    /// Returns the generated token.
    pub fn create_session(&self, role: Role, metadata: Option<HashMap<String, String>>) -> String {
        let raw = uuid::Uuid::new_v4().to_string().replace('-', "");
        let token = format!("tk_{raw}");
        let created_at_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let session = Session {
            token: token.clone(),
            role,
            created_at_secs,
            metadata,
        };
        self.inner.lock().unwrap().insert(token.clone(), session);
        token
    }

    /// Create the short-lived session used by the hosted GPUI Web UI.
    ///
    /// A new launcher replaces any previous hosted launch session. This keeps
    /// an old URL from remaining usable after the user starts `teshi web`
    /// again while preserving unrelated local automation sessions.
    pub fn create_hosted_session(&self) -> String {
        let raw = uuid::Uuid::new_v4().to_string().replace('-', "");
        let token = format!("tk_{raw}");
        let created_at_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let session = Session {
            token: token.clone(),
            role: Role::HostedWebUi,
            created_at_secs,
            metadata: None,
        };
        let mut sessions = self.inner.lock().unwrap();
        sessions.retain(|_, session| session.role != Role::HostedWebUi);
        sessions.insert(token.clone(), session);
        token
    }

    /// Look up a session by token. Returns `None` if the token is unknown.
    pub fn get_session(&self, token: &str) -> Option<Session> {
        self.inner.lock().unwrap().get(token).cloned()
    }

    /// Remove a session (logout / cleanup).
    pub fn remove_session(&self, token: &str) {
        self.inner.lock().unwrap().remove(token);
    }

    /// Invalidate every hosted launch session while retaining local sessions.
    ///
    /// This is used when the daemon's project/runtime session is explicitly
    /// torn down. The connected hosted socket observes the removal and closes
    /// without allowing another business request to run.
    pub fn remove_hosted_sessions(&self) {
        self.inner
            .lock()
            .unwrap()
            .retain(|_, session| session.role != Role::HostedWebUi);
    }

    /// Number of active sessions.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// Returns `true` if there are no active sessions.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_can_do_everything() {
        assert!(Role::Admin.can_execute("/api/v1/daemon/shutdown"));
        assert!(Role::Admin.can_execute("/api/v1/whatever"));
    }

    #[test]
    fn agent_recorder_limited_ops() {
        assert!(Role::AgentRecorder.can_execute("/api/v1/locator/confirm"));
        assert!(Role::AgentRecorder.can_execute("/api/v1/locator/highlight"));
        assert!(Role::AgentRecorder.can_execute("/api/v1/events"));
        assert!(!Role::AgentRecorder.can_execute("/api/v1/daemon/shutdown"));
        assert!(!Role::AgentRecorder.can_execute("/api/v1/daemon/run"));
    }

    #[test]
    fn batch_runner_limited_ops() {
        assert!(Role::BatchRunner.can_execute("/api/v1/daemon/run"));
        assert!(Role::BatchRunner.can_execute("/api/v1/events"));
        assert!(!Role::BatchRunner.can_execute("/api/v1/browser/start"));
        assert!(!Role::BatchRunner.can_execute("/api/v1/daemon/shutdown"));
    }

    #[test]
    fn create_and_retrieve_session() {
        let store = SessionStore::new();
        let token = store.create_session(Role::AgentRecorder, None);
        let session = store.get_session(&token).unwrap();
        assert_eq!(session.role, Role::AgentRecorder);
        assert!(session.token.starts_with("tk_"));
    }

    #[test]
    fn remove_session() {
        let store = SessionStore::new();
        let token = store.create_session(Role::Admin, None);
        assert!(store.get_session(&token).is_some());
        store.remove_session(&token);
        assert!(store.get_session(&token).is_none());
    }

    #[test]
    fn unknown_token_returns_none() {
        let store = SessionStore::new();
        assert!(store.get_session("tk_nonexistent").is_none());
    }

    #[test]
    fn hosted_ui_is_not_admin() {
        assert!(!Role::HostedWebUi.can_execute("/api/v1/gherkin/scenarios"));
        assert!(!Role::HostedWebUi.can_execute("/api/v1/browser/sessions"));
        assert!(!Role::HostedWebUi.can_execute("/api/v1/fs/read"));
        assert!(!Role::HostedWebUi.can_execute("/api/v1/api/exchange"));
        assert!(!Role::HostedWebUi.can_execute("/api/v1/daemon/shutdown"));
        assert!(!Role::HostedWebUi.can_execute("/api/v1/_ping"));
    }

    #[test]
    fn new_hosted_launch_replaces_only_previous_hosted_session() {
        let store = SessionStore::new();
        let previous = store.create_hosted_session();
        let local = store.create_session(Role::Admin, None);

        let current = store.create_hosted_session();

        assert_ne!(previous, current);
        assert!(store.get_session(&previous).is_none());
        assert_eq!(store.get_session(&current).unwrap().role, Role::HostedWebUi);
        assert_eq!(store.get_session(&local).unwrap().role, Role::Admin);
    }

    #[test]
    fn teardown_removes_hosted_sessions_but_keeps_local_sessions() {
        let store = SessionStore::new();
        let hosted = store.create_hosted_session();
        let local = store.create_session(Role::AgentRecorder, None);

        store.remove_hosted_sessions();

        assert!(store.get_session(&hosted).is_none());
        assert!(store.get_session(&local).is_some());
    }
}
