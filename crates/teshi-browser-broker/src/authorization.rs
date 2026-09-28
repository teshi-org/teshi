//! Project policy and short-lived privileged browser grants.
//!
//! The authorization state is deliberately separate from transport and browser
//! session state.  Grants are memory-only, bearer tokens are stored as hashes,
//! and every validation repeats the full OS-user, broker-generation, project,
//! caller, Profile, capability, and expiry binding.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::protocol::{BrokerError, BrokerErrorCode};

pub const DEFAULT_CAPABILITY_GRANT_TTL_SECS: u64 = 300;
pub const MIN_CAPABILITY_GRANT_TTL_SECS: u64 = 30;
pub const MAX_CAPABILITY_GRANT_TTL_SECS: u64 = 3600;
pub const MAX_POLICY_FILE_BYTES: usize = 64 * 1024;
pub const MAX_PRIVILEGED_SCRIPT_BYTES: usize = 1024 * 1024;
pub const MAX_PRIVILEGED_RESULT_BYTES: usize = 1024 * 1024;
pub const MAX_PRIVILEGED_CDP_PARAMS_BYTES: usize = 256 * 1024;
pub const MAX_PRIVILEGED_COOKIE_ENTRIES: usize = 500;
pub const MAX_EXTENSION_METADATA_ENTRIES: usize = 500;
pub const MAX_BROWSER_UPLOAD_FILES: usize = 20;
pub const MAX_BROWSER_UPLOAD_FILE_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_BROWSER_UPLOAD_TOTAL_BYTES: u64 = 250 * 1024 * 1024;
pub const MAX_PRIVILEGED_AUDIT_RECORDS: usize = 1000;
pub const DEFAULT_PRIVILEGED_AUDIT_LIMIT: usize = 100;

const DEFAULT_PRIVILEGED_RESULT_BYTES: usize = 65_536;
const DEFAULT_PRIVILEGED_COOKIE_ENTRIES: usize = 200;
const DEFAULT_EXTENSION_METADATA_ENTRIES: usize = 200;
const REDACTION_MARKER: &str = "[REDACTED]";
const ALLOWED_CONTENT_SETTINGS: [&str; 6] = [
    "notifications",
    "popups",
    "geolocation",
    "camera",
    "microphone",
    "automatic_downloads",
];
const SENSITIVE_AUDIT_FIELDS: [&str; 18] = [
    "authorization",
    "cookie",
    "set-cookie",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "token",
    "access-token",
    "refresh-token",
    "password",
    "passwd",
    "secret",
    "expression",
    "params",
    "body",
    "artifact_data",
    "files",
    "value",
];

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
    allowed_raw_cdp_methods: BTreeSet<String>,
}

impl ProjectPolicy {
    pub fn allows(&self, capability: Capability) -> bool {
        self.allowed_capabilities.contains(&capability)
    }

    pub fn allowed_capabilities(&self) -> &BTreeSet<Capability> {
        &self.allowed_capabilities
    }

    pub fn allows_raw_cdp_method(&self, method: &str) -> bool {
        self.allowed_raw_cdp_methods.contains(method)
    }

    pub fn allowed_raw_cdp_methods(&self) -> &BTreeSet<String> {
        &self.allowed_raw_cdp_methods
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
    raw_cdp_methods: Vec<String>,
}

/// Validated, operation-specific security limits retained while one request is
/// in flight. The audit projection intentionally contains metadata only.
#[derive(Debug, Clone)]
pub struct PrivilegedRequest {
    capability: Capability,
    audit_capability: Capability,
    max_result_bytes: usize,
    max_entries: Option<usize>,
    include_cookie_values: bool,
    normalized_method: Option<String>,
    normalized_setting: Option<String>,
    audit_arguments: Value,
}

impl PrivilegedRequest {
    pub fn capability(&self) -> Capability {
        self.capability
    }

    pub fn audit_capability(&self) -> Capability {
        self.audit_capability
    }

    pub fn max_result_bytes(&self) -> usize {
        self.max_result_bytes
    }

    pub fn max_entries(&self) -> Option<usize> {
        self.max_entries
    }

    pub fn include_cookie_values(&self) -> bool {
        self.include_cookie_values
    }

    pub fn normalized_method(&self) -> Option<&str> {
        self.normalized_method.as_deref()
    }

    pub fn normalized_setting(&self) -> Option<&str> {
        self.normalized_setting.as_deref()
    }

    pub fn audit_arguments(&self) -> &Value {
        &self.audit_arguments
    }
}

#[derive(Debug, Clone)]
struct PrivilegedAuditRecord {
    timestamp_ms: u64,
    capability: Capability,
    project_root: String,
    caller_label: String,
    target: Value,
    request_id: String,
    outcome: String,
    arguments: Value,
}

impl PrivilegedAuditRecord {
    fn public_summary(&self) -> Value {
        json!({
            "timestamp_ms": self.timestamp_ms,
            "capability": self.capability.as_str(),
            "caller_label": self.caller_label,
            "target": self.target,
            "request_id": self.request_id,
            "outcome": self.outcome,
            "arguments": self.arguments,
        })
    }
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
    privileged_audit: Vec<PrivilegedAuditRecord>,
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
            privileged_audit: Vec::new(),
        }
    }

    pub fn grant_count(&self) -> usize {
        self.grants.len()
    }

    pub fn audit_count(&self) -> usize {
        self.privileged_audit.len()
    }

    /// Append one bounded metadata-only record. Project roots are retained
    /// only for server-side scope filtering and never enter the public record.
    #[allow(clippy::too_many_arguments)]
    pub fn append_privileged_audit(
        &mut self,
        capability: Capability,
        project_root: &str,
        caller_label: &str,
        target: Value,
        request_id: &str,
        outcome: &str,
        arguments: &Value,
    ) {
        self.privileged_audit.push(PrivilegedAuditRecord {
            timestamp_ms: unix_ms(),
            capability,
            project_root: canonical_project_root(project_root),
            caller_label: clean_text(caller_label, 120),
            target: sanitize_audit_value(&target, 0),
            request_id: clean_text(request_id, 160),
            outcome: clean_text(outcome, 80),
            arguments: sanitize_audit_value(arguments, 0),
        });
        if self.privileged_audit.len() > MAX_PRIVILEGED_AUDIT_RECORDS {
            let drop_count = self.privileged_audit.len() - MAX_PRIVILEGED_AUDIT_RECORDS;
            self.privileged_audit.drain(..drop_count);
        }
    }

    pub fn list_privileged_audit(
        &self,
        project_root: &str,
        caller_label: &str,
        limit: Option<u64>,
    ) -> Vec<Value> {
        let project_root = canonical_project_root(project_root);
        let caller_label = clean_text(caller_label, 120);
        let limit = limit
            .map(|value| value.clamp(1, MAX_PRIVILEGED_AUDIT_RECORDS as u64) as usize)
            .unwrap_or(DEFAULT_PRIVILEGED_AUDIT_LIMIT);
        let records = self
            .privileged_audit
            .iter()
            .filter(|record| {
                record.project_root == project_root && record.caller_label == caller_label
            })
            .collect::<Vec<_>>();
        let start = records.len().saturating_sub(limit);
        records[start..]
            .iter()
            .map(|record| record.public_summary())
            .collect()
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
        for method in document.privileged.raw_cdp_methods {
            let method = clean_text(&method, 256);
            if !method.is_empty() {
                policy.allowed_raw_cdp_methods.insert(method);
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

pub fn audit_capability_for_operation(
    operation: &str,
    include_cookie_values: bool,
) -> Option<Capability> {
    if operation == "list_browser_cookies" && include_cookie_values {
        Some(Capability::CookieValues)
    } else {
        capability_for_operation(operation, include_cookie_values)
    }
}

/// Validate operation-specific P2 arguments before any extension dispatch.
/// This is intentionally independent from grant validation so malformed or
/// over-broad input cannot use a valid grant to reach Chrome.
pub fn prepare_privileged_request(
    operation: &str,
    arguments: &BTreeMap<String, Value>,
    project_root: &str,
) -> Result<Option<PrivilegedRequest>, BrokerError> {
    let Some(capability) = capability_for_operation(
        operation,
        arguments
            .get("include_values")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    ) else {
        return Ok(None);
    };
    let include_cookie_values = operation == "list_browser_cookies"
        && arguments
            .get("include_values")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let audit_capability = audit_capability_for_operation(operation, include_cookie_values)
        .expect("capability_for_operation returned Some");
    let (max_result_bytes, max_entries, normalized_method, normalized_setting, audit_arguments) =
        match operation {
            "execute_privileged_javascript" => {
                let expression = arguments
                    .get("expression")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let source_bytes = expression.len();
                if expression.is_empty() || source_bytes > MAX_PRIVILEGED_SCRIPT_BYTES {
                    let mut error = BrokerError::new(
                        BrokerErrorCode::InvalidBrowserOperation,
                        "JavaScript source is empty or exceeds the configured byte limit",
                    );
                    error.recovery.insert(
                        "max_source_bytes".into(),
                        Value::from(MAX_PRIVILEGED_SCRIPT_BYTES),
                    );
                    return Err(error);
                }
                let max_result_bytes = bounded_result_bytes(
                    arguments.get("max_result_bytes"),
                    DEFAULT_PRIVILEGED_RESULT_BYTES,
                );
                (
                    max_result_bytes,
                    None,
                    None,
                    None,
                    json!({
                        "source": arguments
                            .get("source_kind")
                            .and_then(Value::as_str)
                            .map(|value| clean_text(value, 20))
                            .filter(|value| !value.is_empty())
                            .unwrap_or_else(|| "inline".into()),
                        "source_bytes": source_bytes,
                        "max_result_bytes": max_result_bytes,
                    }),
                )
            }
            "execute_privileged_cdp" => {
                let method = arguments
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let policy = load_project_policy(project_root);
                let method = validate_raw_cdp_method(&policy, method)?;
                let params = arguments
                    .get("params")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new()));
                if !params.is_object() {
                    return Err(BrokerError::new(
                        BrokerErrorCode::InvalidBrowserOperation,
                        "CDP params must be an object",
                    ));
                }
                let parameter_bytes = serde_json::to_vec(&params)
                    .map(|value| value.len())
                    .map_err(|_| {
                        BrokerError::new(
                            BrokerErrorCode::InvalidBrowserOperation,
                            "CDP params could not be serialized",
                        )
                    })?;
                if parameter_bytes > MAX_PRIVILEGED_CDP_PARAMS_BYTES {
                    let mut error = BrokerError::new(
                        BrokerErrorCode::InvalidBrowserOperation,
                        "CDP parameters exceed the configured byte limit",
                    );
                    error.recovery.insert(
                        "max_parameter_bytes".into(),
                        Value::from(MAX_PRIVILEGED_CDP_PARAMS_BYTES),
                    );
                    return Err(error);
                }
                let max_result_bytes = bounded_result_bytes(
                    arguments.get("max_result_bytes"),
                    DEFAULT_PRIVILEGED_RESULT_BYTES,
                );
                let mut parameter_keys = params
                    .as_object()
                    .expect("params object checked above")
                    .keys()
                    .map(|key| clean_text(key, 120))
                    .collect::<Vec<_>>();
                parameter_keys.sort();
                parameter_keys.truncate(128);
                (
                    max_result_bytes,
                    None,
                    Some(method.clone()),
                    None,
                    json!({
                        "method": method,
                        "parameter_keys": parameter_keys,
                        "parameter_bytes": parameter_bytes,
                        "max_result_bytes": max_result_bytes,
                    }),
                )
            }
            "list_browser_cookies" => {
                let max_entries = bounded_count(
                    arguments.get("max_entries"),
                    DEFAULT_PRIVILEGED_COOKIE_ENTRIES,
                    MAX_PRIVILEGED_COOKIE_ENTRIES,
                );
                let max_result_bytes =
                    bounded_result_bytes(arguments.get("max_result_bytes"), 262_144);
                (
                    max_result_bytes,
                    Some(max_entries),
                    None,
                    None,
                    json!({
                        "include_values": include_cookie_values,
                        "scope": "selected-tab-url",
                        "max_entries": max_entries,
                        "max_result_bytes": max_result_bytes,
                    }),
                )
            }
            "access_browser_content_setting" => {
                let setting = normalize_content_setting(arguments.get("setting"))?;
                validate_content_setting_value(arguments.get("value"))?;
                (
                    DEFAULT_PRIVILEGED_RESULT_BYTES,
                    None,
                    None,
                    Some(setting.clone()),
                    json!({
                        "setting": setting,
                        "operation": if arguments.get("value").is_some_and(|value| !value.is_null()) { "set" } else { "get" },
                    }),
                )
            }
            "list_browser_extensions" => {
                let max_entries = bounded_count(
                    arguments.get("max_entries"),
                    DEFAULT_EXTENSION_METADATA_ENTRIES,
                    MAX_EXTENSION_METADATA_ENTRIES,
                );
                (
                    DEFAULT_PRIVILEGED_RESULT_BYTES,
                    Some(max_entries),
                    None,
                    None,
                    json!({
                        "operation": "list-metadata",
                        "max_entries": max_entries,
                    }),
                )
            }
            _ => return Ok(None),
        };
    Ok(Some(PrivilegedRequest {
        capability,
        audit_capability,
        max_result_bytes,
        max_entries,
        include_cookie_values,
        normalized_method,
        normalized_setting,
        audit_arguments,
    }))
}

pub fn validate_raw_cdp_method(
    policy: &ProjectPolicy,
    method: &str,
) -> Result<String, BrokerError> {
    let normalized = clean_text(method, 256);
    let Some((domain, name)) = normalized.split_once('.') else {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "CDP method must be Domain.method",
        ));
    };
    if !is_ascii_identifier(domain) || !is_ascii_identifier(name) {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "CDP method must be Domain.method",
        ));
    }
    let blocked_domains = [
        "Browser",
        "Target",
        "SystemInfo",
        "FileSystem",
        "IO",
        "Runtime",
        "Debugger",
    ];
    let blocked_methods = [
        "Page.setDownloadBehavior",
        "Browser.setDownloadBehavior",
        "Storage.clearDataForOrigin",
        "DOM.setFileInputFiles",
        "Page.addScriptToEvaluateOnNewDocument",
    ];
    if blocked_domains.contains(&domain)
        || blocked_methods.contains(&normalized.as_str())
        || normalized.to_ascii_lowercase().contains("cookie")
    {
        let mut error = BrokerError::new(
            BrokerErrorCode::BrowserCapabilityDenied,
            "CDP method can escape the selected page or access a separately gated surface",
        );
        error
            .recovery
            .insert("method".into(), Value::String(normalized));
        return Err(error);
    }
    if !policy.allows_raw_cdp_method(&normalized) {
        let mut error = BrokerError::new(
            BrokerErrorCode::BrowserCapabilityDenied,
            "CDP method is not allowlisted by effective project policy",
        );
        error
            .recovery
            .insert("method".into(), Value::String(normalized));
        return Err(error);
    }
    Ok(normalized)
}

pub fn sanitize_privileged_response(
    operation: &str,
    result: &mut BTreeMap<String, Value>,
    request: &PrivilegedRequest,
) -> Result<(), BrokerError> {
    if operation == "list_browser_cookies" {
        let mut cookie_count = None;
        let mut cookie_truncated = false;
        if let Some(Value::Array(cookies)) = result.get_mut("cookies") {
            if let Some(max_entries) = request.max_entries {
                if cookies.len() > max_entries {
                    cookies.truncate(max_entries);
                    cookie_truncated = true;
                }
                cookie_count = Some(cookies.len());
            }
            if !request.include_cookie_values {
                for cookie in cookies.iter_mut().filter_map(Value::as_object_mut) {
                    cookie.remove("value");
                    cookie.insert("value_redacted".into(), Value::Bool(true));
                }
            }
        }
        if cookie_truncated {
            result.insert("truncated".into(), Value::Bool(true));
        }
        if let Some(cookie_count) = cookie_count {
            result.insert("count".into(), Value::from(cookie_count));
        }
        if !request.include_cookie_values {
            result.insert("values_included".into(), Value::Bool(false));
        }
    } else if operation == "list_browser_extensions" {
        let mut extension_count = None;
        let mut extension_truncated = false;
        if let Some(Value::Array(extensions)) = result.get_mut("extensions")
            && let Some(max_entries) = request.max_entries
        {
            if extensions.len() > max_entries {
                extensions.truncate(max_entries);
                extension_truncated = true;
            }
            extension_count = Some(extensions.len());
        }
        if extension_truncated {
            result.insert("truncated".into(), Value::Bool(true));
        }
        if let Some(extension_count) = extension_count {
            result.insert("count".into(), Value::from(extension_count));
        }
        result.insert("mutations_enabled".into(), Value::Bool(false));
    }
    let encoded = serde_json::to_vec(&Value::Object(
        result.clone().into_iter().collect::<Map<String, Value>>(),
    ))
    .map_err(|_| {
        BrokerError::new(
            BrokerErrorCode::BrowserOperationFailed,
            "privileged browser result could not be serialized",
        )
    })?;
    if encoded.len() > request.max_result_bytes {
        let mut error = BrokerError::new(
            BrokerErrorCode::BrowserResourceLimit,
            "privileged browser result exceeds the configured byte limit",
        );
        error
            .recovery
            .insert("result_bytes".into(), Value::from(encoded.len()));
        error.recovery.insert(
            "max_result_bytes".into(),
            Value::from(request.max_result_bytes),
        );
        return Err(error);
    }
    Ok(())
}

pub fn validate_upload_files(
    project_root: &str,
    action: Option<&Value>,
    files: Option<&Value>,
) -> Result<Option<Vec<String>>, BrokerError> {
    let action = action.and_then(Value::as_str).unwrap_or_default();
    let Some(raw_files) = files else {
        if action == "upload" {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "upload requires at least one explicit file",
            ));
        }
        return Ok(None);
    };
    let has_files =
        !raw_files.is_null() && raw_files.as_array().is_none_or(|items| !items.is_empty());
    if action != "upload" {
        if has_files {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "files are accepted only for the upload browser action",
            ));
        }
        return Ok(None);
    }
    let files = raw_files.as_array().ok_or_else(|| {
        BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "upload requires at least one explicit file",
        )
    })?;
    if files.is_empty() {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "upload requires at least one explicit file",
        ));
    }
    if files.len() > MAX_BROWSER_UPLOAD_FILES {
        let mut error = BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "upload exceeds the configured file-count limit",
        );
        error
            .recovery
            .insert("max_files".into(), Value::from(MAX_BROWSER_UPLOAD_FILES));
        return Err(error);
    }
    let root = fs::canonicalize(project_root).map_err(|_| {
        BrokerError::new(
            BrokerErrorCode::BrowserCapabilityDenied,
            "upload project root is unavailable",
        )
    })?;
    if !root.is_dir() {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserCapabilityDenied,
            "upload project root is not a directory",
        ));
    }
    let mut total_bytes = 0_u64;
    let mut resolved = Vec::with_capacity(files.len());
    for (index, raw_path) in files.iter().enumerate() {
        let raw_path = raw_path.as_str().ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "upload file paths must be strings",
            )
        })?;
        let candidate = Path::new(raw_path);
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            root.join(candidate)
        };
        let path = fs::canonicalize(&candidate).map_err(|_| {
            let mut error = BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "upload file is missing or inaccessible",
            );
            error
                .recovery
                .insert("file_index".into(), Value::from(index));
            error
        })?;
        if path.strip_prefix(&root).is_err() {
            let mut error = BrokerError::new(
                BrokerErrorCode::BrowserCapabilityDenied,
                "upload file is outside the authorized project root",
            );
            error
                .recovery
                .insert("file_index".into(), Value::from(index));
            error
                .recovery
                .insert("policy".into(), Value::String("project_root_only".into()));
            return Err(error);
        }
        let metadata = fs::metadata(&path).map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "upload file is inaccessible",
            )
        })?;
        if !metadata.is_file() {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "upload target is not a regular file",
            ));
        }
        if metadata.len() > MAX_BROWSER_UPLOAD_FILE_BYTES {
            let mut error = BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "upload file exceeds the configured byte limit",
            );
            error.recovery.insert(
                "max_file_bytes".into(),
                Value::from(MAX_BROWSER_UPLOAD_FILE_BYTES),
            );
            return Err(error);
        }
        total_bytes = total_bytes.saturating_add(metadata.len());
        if total_bytes > MAX_BROWSER_UPLOAD_TOTAL_BYTES {
            let mut error = BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "upload files exceed the configured total byte limit",
            );
            error.recovery.insert(
                "max_total_bytes".into(),
                Value::from(MAX_BROWSER_UPLOAD_TOTAL_BYTES),
            );
            return Err(error);
        }
        resolved.push(path_to_string(&path));
    }
    Ok(Some(resolved))
}

pub fn audit_arguments_for_operation(
    operation: &str,
    arguments: &BTreeMap<String, Value>,
) -> Value {
    match operation {
        "execute_privileged_javascript" => json!({
            "source": arguments
                .get("source_kind")
                .and_then(Value::as_str)
                .map(|value| clean_text(value, 20))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "inline".into()),
            "source_bytes": arguments
                .get("expression")
                .and_then(Value::as_str)
                .map(str::len)
                .unwrap_or_default(),
        }),
        "execute_privileged_cdp" => {
            let parameter_keys = arguments
                .get("params")
                .and_then(Value::as_object)
                .map(|params| {
                    let mut keys = params
                        .keys()
                        .map(|key| clean_text(key, 120))
                        .collect::<Vec<_>>();
                    keys.sort();
                    keys.truncate(128);
                    keys
                })
                .unwrap_or_default();
            json!({
                "method": arguments
                    .get("method")
                    .and_then(Value::as_str)
                    .map(|value| clean_text(value, 256))
                    .unwrap_or_default(),
                "parameter_keys": parameter_keys,
            })
        }
        "list_browser_cookies" => json!({
            "include_values": arguments
                .get("include_values")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "scope": "selected-tab-url",
        }),
        "access_browser_content_setting" => json!({
            "setting": arguments
                .get("setting")
                .and_then(Value::as_str)
                .map(|value| clean_text(value, 80))
                .unwrap_or_default(),
            "operation": if arguments.get("value").is_some_and(|value| !value.is_null()) { "set" } else { "get" },
        }),
        "list_browser_extensions" => json!({"operation": "list-metadata"}),
        _ => json!({}),
    }
}

fn bounded_count(value: Option<&Value>, default: usize, maximum: usize) -> usize {
    value
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .map(|value| value.min(maximum as u64) as usize)
        .unwrap_or(default)
}

fn bounded_result_bytes(value: Option<&Value>, default: usize) -> usize {
    value
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .map(|value| value.min(MAX_PRIVILEGED_RESULT_BYTES as u64) as usize)
        .unwrap_or(default)
}

fn normalize_content_setting(value: Option<&Value>) -> Result<String, BrokerError> {
    let normalized = value
        .and_then(Value::as_str)
        .map(|value| clean_text(value, 80).to_ascii_lowercase().replace('-', "_"))
        .unwrap_or_default();
    if ALLOWED_CONTENT_SETTINGS.contains(&normalized.as_str()) {
        return Ok(normalized);
    }
    let mut error = BrokerError::new(
        BrokerErrorCode::BrowserCapabilityDenied,
        "content setting is not in the supported allowlist",
    );
    error.recovery.insert(
        "allowed_settings".into(),
        Value::Array(
            ALLOWED_CONTENT_SETTINGS
                .iter()
                .map(|value| Value::String((*value).into()))
                .collect(),
        ),
    );
    Err(error)
}

fn validate_content_setting_value(value: Option<&Value>) -> Result<(), BrokerError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    if value
        .as_str()
        .is_some_and(|value| matches!(value, "allow" | "block" | "ask"))
    {
        return Ok(());
    }
    Err(BrokerError::new(
        BrokerErrorCode::InvalidBrowserOperation,
        "content setting value must be allow, block, or ask",
    ))
}

fn is_ascii_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|character| character.is_ascii_alphanumeric())
}

fn sanitize_audit_value(value: &Value, depth: usize) -> Value {
    if depth >= 4 {
        return Value::String("[TRUNCATED]".into());
    }
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .take(256)
                .map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase().replace('_', "-");
                    let value = if is_sensitive_audit_field(&normalized) {
                        Value::String(REDACTION_MARKER.into())
                    } else {
                        sanitize_audit_value(value, depth + 1)
                    };
                    (key.chars().take(256).collect(), value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(128)
                .map(|item| sanitize_audit_value(item, depth + 1))
                .collect(),
        ),
        Value::String(value) => Value::String(value.chars().take(4096).collect()),
        _ => value.clone(),
    }
}

fn is_sensitive_audit_field(value: &str) -> bool {
    SENSITIVE_AUDIT_FIELDS.contains(&value)
        || value.contains("token")
        || value.contains("password")
        || value.contains("passwd")
        || value.contains("secret")
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
    let mut result = path_to_string(&normalized).replace('\\', "/");
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

fn path_to_string(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).into()
    }
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
            allowed_raw_cdp_methods: BTreeSet::new(),
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

    #[test]
    fn raw_cdp_policy_and_operation_gates_are_fail_closed() {
        let root = tempdir().unwrap();
        let policy_path = root.path().join(".teshi/browser-policy.json");
        fs::create_dir_all(policy_path.parent().unwrap()).unwrap();
        fs::write(
            &policy_path,
            r#"{"privileged":{"allow":["raw-cdp"],"raw_cdp_methods":["Page.getLayoutMetrics"]}}"#,
        )
        .unwrap();
        let root = root.path().to_string_lossy().into_owned();
        let mut args = BTreeMap::new();
        args.insert(
            "method".into(),
            Value::String("Page.getLayoutMetrics".into()),
        );
        args.insert("params".into(), json!({"include": "metadata"}));
        let prepared = prepare_privileged_request("execute_privileged_cdp", &args, &root)
            .unwrap()
            .unwrap();
        assert_eq!(prepared.normalized_method(), Some("Page.getLayoutMetrics"));
        assert_eq!(
            prepared.audit_arguments()["parameter_keys"],
            json!(["include"])
        );

        args.insert("method".into(), Value::String("Target.createTarget".into()));
        let blocked =
            prepare_privileged_request("execute_privileged_cdp", &args, &root).unwrap_err();
        assert_eq!(blocked.code, BrokerErrorCode::BrowserCapabilityDenied);
    }

    #[test]
    fn privileged_results_redact_cookie_values_and_bound_metadata() {
        let args = BTreeMap::from([
            ("include_values".into(), Value::Bool(false)),
            ("max_entries".into(), Value::from(1_u64)),
        ]);
        let request = prepare_privileged_request("list_browser_cookies", &args, "C:/project")
            .unwrap()
            .unwrap();
        let mut result = BTreeMap::from([(
            "cookies".into(),
            json!([
                {"name": "sid", "value": "secret"},
                {"name": "other", "value": "also-secret"}
            ]),
        )]);
        sanitize_privileged_response("list_browser_cookies", &mut result, &request).unwrap();
        assert_eq!(result["cookies"].as_array().unwrap().len(), 1);
        assert!(result["cookies"][0].get("value").is_none());
        assert_eq!(result["cookies"][0]["value_redacted"], Value::Bool(true));
        assert_eq!(result["truncated"], Value::Bool(true));

        let mut extension_result =
            BTreeMap::from([("extensions".into(), json!([{"id": "one"}, {"id": "two"}]))]);
        let extension_args = BTreeMap::from([(String::from("max_entries"), Value::from(1_u64))]);
        let extension_request =
            prepare_privileged_request("list_browser_extensions", &extension_args, "C:/project")
                .unwrap()
                .unwrap();
        sanitize_privileged_response(
            "list_browser_extensions",
            &mut extension_result,
            &extension_request,
        )
        .unwrap();
        assert_eq!(extension_result["extensions"].as_array().unwrap().len(), 1);
        assert_eq!(extension_result["mutations_enabled"], Value::Bool(false));
    }

    #[test]
    fn upload_files_are_canonicalized_and_project_bound() {
        let root = tempdir().unwrap();
        let upload = root.path().join("upload.txt");
        fs::write(&upload, "fixture").unwrap();
        let action = Value::String("upload".into());
        let files = json!(["upload.txt"]);
        let resolved =
            validate_upload_files(root.path().to_str().unwrap(), Some(&action), Some(&files))
                .unwrap()
                .unwrap();
        assert_eq!(resolved, vec![upload.to_string_lossy().into_owned()]);

        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("private.txt");
        fs::write(&outside_file, "private").unwrap();
        let denied = validate_upload_files(
            root.path().to_str().unwrap(),
            Some(&action),
            Some(&json!([outside_file.to_string_lossy()])),
        )
        .unwrap_err();
        assert_eq!(denied.code, BrokerErrorCode::BrowserCapabilityDenied);
        assert!(
            !denied
                .message
                .contains(outside_file.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn audit_records_are_bounded_scoped_and_redacted() {
        let root = tempdir().unwrap();
        let root = root.path().to_string_lossy().into_owned();
        let mut state = AuthorizationState::new("user-a");
        for index in 0..(MAX_PRIVILEGED_AUDIT_RECORDS + 5) {
            state.append_privileged_audit(
                Capability::Javascript,
                &root,
                "caller-a",
                json!({"extension_instance_id": "profile-a"}),
                &format!("request-{index}"),
                "denied",
                &json!({
                    "expression": "40 + 2",
                    "capability_grant_token": "grant-secret",
                    "authorization": "Bearer secret"
                }),
            );
        }
        assert_eq!(state.audit_count(), MAX_PRIVILEGED_AUDIT_RECORDS);
        let listed = state.list_privileged_audit(&root, "caller-a", Some(1));
        let serialized = serde_json::to_string(&listed).unwrap();
        assert!(!serialized.contains("40 + 2"));
        assert!(!serialized.contains("grant-secret"));
        assert!(serialized.contains(REDACTION_MARKER));
        assert!(
            state
                .list_privileged_audit(&root, "caller-b", None)
                .is_empty()
        );
    }
}
