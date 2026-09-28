//! Project policy and short-lived privileged browser grants.
//!
//! The authorization state is deliberately separate from transport and browser
//! session state.  Grants are memory-only, bearer tokens are stored as hashes,
//! and every validation repeats the full OS-user, broker-generation, project,
//! caller, Profile, capability, and expiry binding.

use std::collections::{BTreeSet, HashMap};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::protocol::{BrokerError, BrokerErrorCode};

pub const DEFAULT_CAPABILITY_GRANT_TTL_SECS: u64 = 300;
pub const MIN_CAPABILITY_GRANT_TTL_SECS: u64 = 30;
pub const MAX_CAPABILITY_GRANT_TTL_SECS: u64 = 3600;
pub const MAX_POLICY_FILE_BYTES: usize = 64 * 1024;

const POLICY_FILE_NAME: &str = "browser-policy.json";

/// Privileged browser surfaces that can be granted independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    Javascript,
    RawCdp,
    Cookies,
    CookieValues,
    ContentSettings,
    ExtensionManagement,
}

impl Capability {
    pub const ALL: [Self; 6] = [
        Self::Javascript,
        Self::RawCdp,
        Self::Cookies,
        Self::CookieValues,
        Self::ContentSettings,
        Self::ExtensionManagement,
    ];

    pub fn parse(value: &str) -> Result<Self, BrokerError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "javascript" => Ok(Self::Javascript),
            "raw-cdp" => Ok(Self::RawCdp),
            "cookies" => Ok(Self::Cookies),
            "cookie-values" => Ok(Self::CookieValues),
            "content-settings" => Ok(Self::ContentSettings),
            "extension-management" => Ok(Self::ExtensionManagement),
            _ => Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "unknown privileged browser capability",
            )),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Javascript => "javascript",
            Self::RawCdp => "raw-cdp",
            Self::Cookies => "cookies",
            Self::CookieValues => "cookie-values",
            Self::ContentSettings => "content-settings",
            Self::ExtensionManagement => "extension-management",
        }
    }

    pub const fn optional_permission(self) -> Option<&'static str> {
        match self {
            Self::Cookies | Self::CookieValues => Some("cookies"),
            Self::ContentSettings => Some("content_settings"),
            Self::ExtensionManagement => Some("extension_management"),
            Self::Javascript | Self::RawCdp => None,
        }
    }
}

/// Effective project/user policy.  Unknown or malformed policy entries are
/// ignored, leaving the default-deny set unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectPolicy {
    allowed_capabilities: BTreeSet<Capability>,
}

impl ProjectPolicy {
    pub fn allows(&self, capability: Capability) -> bool {
        self.allowed_capabilities.contains(&capability)
    }

    pub fn allowed_capabilities(&self) -> &BTreeSet<Capability> {
        &self.allowed_capabilities
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PolicyDocument {
    privileged: PrivilegedPolicy,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct PrivilegedPolicy {
    allow: Vec<String>,
}

#[derive(Debug)]
struct CapabilityGrant {
    grant_id: String,
    token_hash: [u8; 32],
    capability: Capability,
    extension_instance_id: String,
    project_root: String,
    caller_label: String,
    local_user: String,
    broker_start_id: String,
    issued_wall_time_ms: u64,
    expires_wall_time_ms: u64,
    expires_at: Instant,
    revoked: bool,
}

/// In-memory authorization owner for one broker generation.
#[derive(Debug)]
pub struct AuthorizationState {
    local_user: String,
    grants: HashMap<String, CapabilityGrant>,
}

impl Default for AuthorizationState {
    fn default() -> Self {
        Self::new(current_os_user())
    }
}

impl AuthorizationState {
    pub fn new(local_user: impl Into<String>) -> Self {
        let local_user = clean_text(&local_user.into(), 120);
        Self {
            local_user: if local_user.is_empty() {
                "unknown-user".into()
            } else {
                local_user
            },
            grants: HashMap::new(),
        }
    }

    pub fn grant_count(&self) -> usize {
        self.grants.len()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        &mut self,
        capability: Capability,
        extension_instance_id: &str,
        project_root: &str,
        caller_label: &str,
        broker_start_id: &str,
        ttl_secs: Option<u64>,
        interactive_confirmed: bool,
        non_interactive: bool,
        acknowledged_capability: Option<&str>,
        policy: &ProjectPolicy,
    ) -> Result<Value, BrokerError> {
        let extension_instance_id = clean_text(extension_instance_id, 256);
        let project_root = canonical_project_root(project_root);
        let caller_label = clean_text(caller_label, 120);
        let broker_start_id = clean_text(broker_start_id, 128);
        if extension_instance_id.is_empty()
            || project_root.is_empty()
            || caller_label.is_empty()
            || broker_start_id.is_empty()
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "capability grant scope is incomplete",
            ));
        }
        if non_interactive {
            if !acknowledged_capability
                .is_some_and(|value| value.trim().eq_ignore_ascii_case(capability.as_str()))
            {
                return Err(BrokerError::new(
                    BrokerErrorCode::BrowserCapabilityDenied,
                    "non-interactive grant requires an exact capability acknowledgement",
                ));
            }
            if !policy.allows(capability) {
                return Err(BrokerError::new(
                    BrokerErrorCode::BrowserCapabilityDenied,
                    "effective browser policy denies this non-interactive capability",
                ));
            }
        } else if !interactive_confirmed {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserCapabilityDenied,
                "interactive capability grant requires explicit confirmation",
            ));
        }

        let ttl_secs = ttl_secs
            .unwrap_or(DEFAULT_CAPABILITY_GRANT_TTL_SECS)
            .clamp(MIN_CAPABILITY_GRANT_TTL_SECS, MAX_CAPABILITY_GRANT_TTL_SECS);
        let now = Instant::now();
        let issued_wall_time_ms = unix_ms();
        let expires_wall_time_ms = issued_wall_time_ms.saturating_add(ttl_secs * 1000);
        let token = format!(
            "grant_{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        let grant_id = format!("cap_{}", Uuid::new_v4().simple());
        let grant = CapabilityGrant {
            grant_id: grant_id.clone(),
            token_hash: token_hash(&token),
            capability,
            extension_instance_id,
            project_root,
            caller_label,
            local_user: self.local_user.clone(),
            broker_start_id,
            issued_wall_time_ms,
            expires_wall_time_ms,
            expires_at: now + Duration::from_secs(ttl_secs),
            revoked: false,
        };
        let summary = grant_summary(&grant);
        self.grants.insert(grant_id, grant);
        let mut response = match summary {
            Value::Object(object) => object,
            _ => unreachable!("grant summary is always an object"),
        };
        response.insert("grant_token".into(), Value::String(token));
        Ok(Value::Object(response))
    }

    pub fn list(&mut self, project_root: &str, extension_instance_id: Option<&str>) -> Vec<Value> {
        self.expire(Instant::now());
        let project_root = canonical_project_root(project_root);
        let extension_instance_id = extension_instance_id
            .map(|value| clean_text(value, 256))
            .filter(|value| !value.is_empty());
        self.grants
            .values()
            .filter(|grant| {
                grant.project_root == project_root
                    && extension_instance_id
                        .as_deref()
                        .is_none_or(|value| grant.extension_instance_id == value)
            })
            .map(grant_summary)
            .collect()
    }

    pub fn revoke(&mut self, grant_id: &str, project_root: &str) -> Result<Value, BrokerError> {
        let project_root = canonical_project_root(project_root);
        let grant = self
            .grants
            .get_mut(grant_id.trim())
            .filter(|grant| grant.project_root == project_root)
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::BrowserCapabilityDenied,
                    "capability grant is unavailable",
                )
            })?;
        grant.revoked = true;
        Ok(json!({"grant_id": grant.grant_id, "revoked": true}))
    }

    pub fn expire(&mut self, now: Instant) -> usize {
        let before = self.grants.len();
        self.grants.retain(|_, grant| grant.expires_at > now);
        before.saturating_sub(self.grants.len())
    }

    pub fn validate(
        &mut self,
        token: &str,
        capability: Capability,
        extension_instance_id: &str,
        project_root: &str,
        caller_label: &str,
        broker_start_id: &str,
    ) -> Result<(), BrokerError> {
        let token_hash = token_hash(token.trim());
        let grant_id = self
            .grants
            .iter()
            .find(|(_, grant)| bool::from(grant.token_hash.ct_eq(&token_hash)))
            .map(|(grant_id, _)| grant_id.clone())
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::BrowserCapabilityDenied,
                    "a valid capability grant is required",
                )
            })?;
        let now = Instant::now();
        let grant = self
            .grants
            .get_mut(&grant_id)
            .expect("grant was found above");
        if grant.revoked {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserCapabilityDenied,
                "capability grant was revoked",
            ));
        }
        if grant.expires_at <= now {
            self.grants.remove(&grant_id);
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserCapabilityDenied,
                "capability grant expired",
            ));
        }
        let project_root = canonical_project_root(project_root);
        let caller_label = clean_text(caller_label, 120);
        let extension_instance_id = clean_text(extension_instance_id, 256);
        let broker_start_id = clean_text(broker_start_id, 128);
        if grant.capability != capability
            || grant.extension_instance_id != extension_instance_id
            || grant.project_root != project_root
            || grant.caller_label != caller_label
            || grant.local_user != self.local_user
            || grant.broker_start_id != broker_start_id
        {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserCapabilityDenied,
                "capability grant scope does not match this request",
            ));
        }
        Ok(())
    }
}

/// Load the effective explicit privileged allowlist.  Project policy is read
/// before user policy, and both are additive.  Any filesystem, size, parse,
/// symlink, or schema problem leaves that candidate at default deny.
pub fn load_project_policy(project_root: &str) -> ProjectPolicy {
    let mut policy = ProjectPolicy::default();
    for path in policy_paths(project_root) {
        let Some(document) = read_policy_document(&path) else {
            continue;
        };
        for capability in document.privileged.allow {
            if let Ok(capability) = Capability::parse(&capability) {
                policy.allowed_capabilities.insert(capability);
            }
        }
    }
    policy
}

/// Check the optional Chrome permission announced by the extension heartbeat.
pub fn require_optional_permission(approved: bool, permission: &str) -> Result<(), BrokerError> {
    if approved {
        return Ok(());
    }
    Err(BrokerError::new(
        BrokerErrorCode::BrowserCapabilityUnavailable,
        format!("required Chromium optional permission is not approved: {permission}"),
    ))
}

pub fn capability_for_operation(
    operation: &str,
    _include_cookie_values: bool,
) -> Option<Capability> {
    match operation {
        "execute_privileged_javascript" => Some(Capability::Javascript),
        "execute_privileged_cdp" => Some(Capability::RawCdp),
        "list_browser_cookies" => Some(Capability::Cookies),
        "access_browser_content_setting" => Some(Capability::ContentSettings),
        "list_browser_extensions" => Some(Capability::ExtensionManagement),
        _ => None,
    }
}

pub fn canonical_project_root(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    let path = Path::new(value);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let normalized = fs::canonicalize(&absolute).unwrap_or_else(|_| normalize_path(&absolute));
    let mut result = normalized.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        result.make_ascii_lowercase();
    }
    result
}

fn policy_paths(project_root: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if !project_root.trim().is_empty() {
        paths.push(
            Path::new(project_root)
                .join(".teshi")
                .join(POLICY_FILE_NAME),
        );
    }
    let user_config_root = env::var_os("LOCALAPPDATA")
        .or_else(|| env::var_os("APPDATA"))
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        });
    if let Some(root) = user_config_root {
        paths.push(root.join("teshi").join(POLICY_FILE_NAME));
    }
    paths
}

fn read_policy_document(path: &Path) -> Option<PolicyDocument> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return None;
    }
    if metadata.len() > MAX_POLICY_FILE_BYTES as u64 {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.len() > MAX_POLICY_FILE_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn current_os_user() -> String {
    env::var("USERNAME")
        .or_else(|_| env::var("USER"))
        .map(|value| clean_text(&value, 120))
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown-user".into())
}

fn clean_text(value: &str, max_chars: usize) -> String {
    value.trim().chars().take(max_chars).collect()
}

fn token_hash(token: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().into()
}

fn grant_summary(grant: &CapabilityGrant) -> Value {
    json!({
        "grant_id": grant.grant_id,
        "capability": grant.capability.as_str(),
        "extension_instance_id": grant.extension_instance_id,
        "project_root": grant.project_root,
        "caller_label": grant.caller_label,
        "issued_at_ms": grant.issued_wall_time_ms,
        "expires_at_ms": grant.expires_wall_time_ms,
        "revoked": grant.revoked,
    })
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn policy_is_default_deny_and_loads_project_allowlist() {
        let root = tempdir().unwrap();
        let policy_path = root.path().join(".teshi/browser-policy.json");
        fs::create_dir_all(policy_path.parent().unwrap()).unwrap();
        fs::write(
            &policy_path,
            r#"{"privileged":{"allow":["javascript","unknown"]}}"#,
        )
        .unwrap();
        let policy = load_project_policy(root.path().to_str().unwrap());
        assert!(policy.allows(Capability::Javascript));
        assert!(!policy.allows(Capability::RawCdp));
    }

    #[test]
    fn grants_bind_all_scopes_and_revoke_or_expiry_fails_closed() {
        let root = tempdir().unwrap();
        let root = root.path().to_string_lossy().into_owned();
        let mut state = AuthorizationState::new("user-a");
        let policy = ProjectPolicy {
            allowed_capabilities: [Capability::Javascript].into_iter().collect(),
        };
        let issued = state
            .issue(
                Capability::Javascript,
                "profile-a",
                &root,
                "caller-a",
                "broker-a",
                Some(30),
                false,
                true,
                Some("javascript"),
                &policy,
            )
            .unwrap();
        let token = issued["grant_token"].as_str().unwrap().to_owned();
        state
            .validate(
                &token,
                Capability::Javascript,
                "profile-a",
                &root,
                "caller-a",
                "broker-a",
            )
            .unwrap();
        assert_eq!(
            state
                .validate(
                    &token,
                    Capability::Javascript,
                    "profile-b",
                    &root,
                    "caller-a",
                    "broker-a",
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::BrowserCapabilityDenied
        );
        let grant_id = issued["grant_id"].as_str().unwrap();
        state.revoke(grant_id, &root).unwrap();
        assert!(
            state
                .validate(
                    &token,
                    Capability::Javascript,
                    "profile-a",
                    &root,
                    "caller-a",
                    "broker-a",
                )
                .is_err()
        );
    }

    #[test]
    fn non_interactive_issue_requires_exact_ack_and_policy() {
        let mut state = AuthorizationState::new("user-a");
        let policy = ProjectPolicy::default();
        let error = state
            .issue(
                Capability::RawCdp,
                "profile-a",
                "C:/project-a",
                "caller-a",
                "broker-a",
                None,
                false,
                true,
                Some("raw-cdp"),
                &policy,
            )
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserCapabilityDenied);
    }
}
