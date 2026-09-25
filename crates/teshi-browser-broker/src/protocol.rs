//! Typed protocol-v1 records for the existing Chrome extension wire contract.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Wire schema understood by the current Rust browser client.
pub const BROWSER_BROKER_SCHEMA_VERSION: u16 = 1;
/// Public Chrome extension protocol version. Keep in sync with `background.js`.
pub const BROWSER_BROKER_PROTOCOL_VERSION: u16 = 1;
/// Fixed loopback HTTP discovery port retained for existing extensions.
pub const CHROME_DISCOVERY_PORT: u16 = 17_373;
/// Maximum JSON body accepted by broker HTTP routes.
pub const MAX_HTTP_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Maximum WebSocket text/binary message accepted by current v1 clients.
pub const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 72 * 1024 * 1024;
/// Total parsed inbound message and queued frame budget across the broker.
pub const MAX_QUEUED_EVENT_BYTES: usize = 128 * 1024 * 1024;
/// Client control messages are small JSON envelopes; evidence remains separately bounded.
pub const MAX_CONTROL_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
/// Maximum number of concurrent sockets before additional upgrades are refused.
pub const MAX_WEBSOCKET_CONNECTIONS: usize = 64;
/// Maximum queued commands per extension session.
pub const MAX_COMMAND_QUEUE: usize = 256;
/// Maximum exact extension identities a user may pair with the broker.
pub const MAX_TRUSTED_EXTENSION_ORIGINS: usize = 16;
/// Internal local identity proof endpoint; it never accepts or returns the token.
pub const BROWSER_BROKER_IDENTITY_CHALLENGE_PATH: &str = "/v1/bridge/identity";
/// Maximum in-flight network events retained in one extension batch.
pub const MAX_NETWORK_EVENTS_PER_BATCH: usize = 100;

/// A complete target identity. Window and tab IDs are only unique inside one Profile.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserTarget {
    pub extension_instance_id: String,
    pub window_id: i64,
    pub tab_id: i64,
}

/// Feature or permission availability announced by an extension heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureAvailability {
    pub feature: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One browser tab as reported by Chrome extension APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionTab {
    pub id: i64,
    #[serde(default)]
    pub window_id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default, rename = "favIconUrl")]
    pub favicon_url: String,
    #[serde(default)]
    pub debuggable: bool,
}

/// One normal Chrome window and its tabs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionWindow {
    pub id: i64,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub tabs: Vec<ExtensionTab>,
}

/// Heartbeat emitted by the extension. Legacy v0 payloads omit version and identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionHeartbeat {
    #[serde(default)]
    pub schema_version: Option<u16>,
    #[serde(default)]
    pub protocol_version: Option<u16>,
    #[serde(default)]
    pub extension_instance_id: Option<String>,
    #[serde(default)]
    pub profile_label: String,
    #[serde(default)]
    pub extension_version: String,
    #[serde(default)]
    pub features: Vec<FeatureAvailability>,
    #[serde(default)]
    pub supported_actions: Vec<String>,
    #[serde(default)]
    pub supported_operations: Vec<String>,
    #[serde(default)]
    pub optional_permissions: BTreeMap<String, bool>,
    #[serde(default)]
    pub browser: BTreeMap<String, Value>,
    #[serde(default)]
    pub project_root: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub active_window_id: Option<i64>,
    #[serde(default)]
    pub active_tab_id: Option<i64>,
    #[serde(default)]
    pub tabs: Vec<ExtensionTab>,
    #[serde(default)]
    pub windows: Vec<ExtensionWindow>,
    #[serde(default)]
    pub frame_error: String,
}

impl ExtensionHeartbeat {
    /// Returns true only if protocol-v1 target routing is advertised and identified.
    pub fn is_versioned(&self) -> bool {
        self.schema_version == Some(BROWSER_BROKER_SCHEMA_VERSION)
            && self.protocol_version == Some(BROWSER_BROKER_PROTOCOL_VERSION)
            && self
                .extension_instance_id
                .as_deref()
                .is_some_and(|id| !id.trim().is_empty())
    }

    /// Checks one feature without treating an absent feature as available.
    pub fn supports_feature(&self, required: &str) -> bool {
        self.features
            .iter()
            .any(|feature| feature.feature == required && feature.available)
    }
}

/// Typed wrapper for one correlated operation request from CLI, Agent, or MCP.
/// Operation-specific fields are retained in `arguments` at the protocol edge and
/// are converted to typed commands before state mutation/extension dispatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationRequest {
    #[serde(default)]
    pub schema_version: Option<u16>,
    pub request_id: String,
    #[serde(default)]
    pub caller_label: String,
    #[serde(default)]
    pub project_root: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(rename = "cmd")]
    pub operation: String,
    #[serde(default)]
    pub target: Option<BrowserTarget>,
    #[serde(default)]
    pub lease_token: Option<String>,
    #[serde(default)]
    pub required_feature: Option<String>,
    #[serde(flatten)]
    pub arguments: BTreeMap<String, Value>,
}

impl OperationRequest {
    /// Reject malformed or unknown operation names before they reach a target.
    pub fn validate(&self) -> Result<(), BrokerError> {
        if self.request_id.trim().is_empty() || self.request_id.len() > 256 {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "request_id must contain 1 to 256 bytes",
            ));
        }
        if self.operation.trim().is_empty() || self.operation.len() > 128 {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "cmd must contain 1 to 128 bytes",
            ));
        }
        if self
            .schema_version
            .is_some_and(|version| version != BROWSER_BROKER_SCHEMA_VERSION)
        {
            return Err(BrokerError::new(
                BrokerErrorCode::IncompatibleBrowserSession,
                "browser operation schema version is not supported",
            ));
        }
        if !is_supported_operation(&self.operation) {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "unknown browser operation",
            ));
        }
        Ok(())
    }
}

/// Correlated extension operation response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtensionResponse {
    #[serde(rename = "type", default)]
    pub message_type: String,
    #[serde(default)]
    pub schema_version: Option<u16>,
    #[serde(default)]
    pub protocol_version: Option<u16>,
    pub request_id: String,
    #[serde(default, rename = "cmd")]
    pub operation: String,
    #[serde(default)]
    pub extension_instance_id: Option<String>,
    #[serde(default)]
    pub target: Option<BrowserTarget>,
    pub ok: bool,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(flatten)]
    pub result: BTreeMap<String, Value>,
}

impl ExtensionResponse {
    /// Reject responses that cannot be safely associated with this protocol generation.
    pub fn validate(&self) -> Result<(), BrokerError> {
        if self.message_type != "response"
            || self.request_id.trim().is_empty()
            || self.request_id.len() > 256
        {
            return Err(BrokerError::new(
                BrokerErrorCode::BrokerProtocolError,
                "extension response type or request_id is invalid",
            ));
        }
        if self
            .schema_version
            .is_some_and(|version| version != BROWSER_BROKER_SCHEMA_VERSION)
            || self
                .protocol_version
                .is_some_and(|version| version > BROWSER_BROKER_PROTOCOL_VERSION)
        {
            return Err(BrokerError::new(
                BrokerErrorCode::IncompatibleBrowserSession,
                "extension response version is not supported",
            ));
        }
        Ok(())
    }
}

/// Extension hello used to authenticate and bind the preview stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExtensionStreamMessage {
    StreamHello {
        #[serde(default)]
        project_root: Option<String>,
        extension_instance_id: String,
        protocol_version: u16,
        #[serde(default)]
        extension_version: String,
    },
}

/// Metadata prefixed to a TSH1 binary screencast frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewFrameMetadata {
    pub extension_instance_id: String,
    pub window_id: i64,
    pub tab_id: i64,
    #[serde(default)]
    pub url: String,
    pub seq: u64,
}

/// One event in an acknowledged Network capture batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkEvent {
    pub seq: u64,
    #[serde(flatten)]
    pub data: BTreeMap<String, Value>,
}

/// Target-scoped, bounded sequence batch emitted by the extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkBatch {
    #[serde(rename = "type")]
    pub message_type: String,
    pub extension_instance_id: String,
    pub capture_id: String,
    pub target: BrowserTarget,
    pub events: Vec<NetworkEvent>,
    #[serde(default)]
    pub first_seq: Option<u64>,
    #[serde(default)]
    pub last_seq: Option<u64>,
    #[serde(default)]
    pub dropped_events: u64,
    #[serde(default)]
    pub dropped_bytes: u64,
    #[serde(default)]
    pub dropped_events_total: u64,
    #[serde(default)]
    pub dropped_bytes_total: u64,
    #[serde(default)]
    pub termination_reason: Option<String>,
    #[serde(default)]
    pub termination_detail: Option<String>,
    #[serde(default)]
    pub final_sequence: Option<u64>,
    #[serde(default)]
    pub diagnostics: Option<Value>,
}

impl NetworkBatch {
    /// Validate count, scope and sequence envelope before buffering any event.
    pub fn validate(&self) -> Result<(), BrokerError> {
        if self.message_type != "network_batch" {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "expected network_batch message",
            ));
        }
        if self.events.len() > MAX_NETWORK_EVENTS_PER_BATCH {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserResourceLimit,
                "network batch exceeds the event limit",
            ));
        }
        if self.extension_instance_id != self.target.extension_instance_id {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "network batch target does not match its extension instance",
            ));
        }
        if self
            .events
            .windows(2)
            .any(|pair| pair[1].seq <= pair[0].seq)
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network event sequence must be strictly increasing",
            ));
        }
        Ok(())
    }
}

/// Discovery contract consumed by the extension and existing CLI endpoint readers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryResponse {
    pub schema_version: u16,
    pub protocol_version: u16,
    pub mode: String,
    pub transport: String,
    pub command_transport: String,
    pub ws_url: String,
    pub extension_frame_ws_url: String,
    pub discovery_url: String,
    pub broker_pid: u32,
    pub broker_scope: String,
    pub broker_start_id: String,
    pub broker_features: Vec<String>,
    pub extension_connected: bool,
}

/// Per-project compatibility pointer. It deliberately carries no project path or
/// bearer token; trusted native clients load credentials from per-user state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointRecord {
    pub schema_version: u16,
    pub protocol_version: u16,
    pub mode: String,
    pub ws_url: String,
    pub discovery_url: String,
    pub extension_frame_ws_url: String,
    pub broker_pid: u32,
    pub broker_start_id: String,
    pub broker_features: Vec<String>,
    pub bridge: String,
}

/// One-shot client nonce for proving a discovered listener owns the private token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerIdentityChallenge {
    pub nonce: String,
}

/// Proof bound to one nonce and exact broker generation; safe to return locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerIdentityProof {
    pub schema_version: u16,
    pub protocol_version: u16,
    pub broker_pid: u32,
    pub broker_start_id: String,
    pub nonce: String,
    pub proof: String,
}

/// Stable server-side failure codes; unknown operations never reach Chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerErrorCode {
    BrowserUnavailable,
    IncompatibleBrowserSession,
    BrowserSessionDisconnected,
    AmbiguousBrowserTarget,
    BrowserSessionBusy,
    StaleBrowserTarget,
    StaleElementReference,
    MismatchedBrowserResponse,
    ExpiredBrowserLease,
    InvalidBrowserLease,
    BrowserTargetNotFound,
    BrowserOperationTimeout,
    InvalidBrowserOperation,
    BrowserCapabilityUnavailable,
    BrowserCapabilityDenied,
    BrowserArtifactFailure,
    DuplicateBrowserMutation,
    BrowserResourceLimit,
    BrokerAuthenticationFailed,
    BrokerOriginDenied,
    BrokerProtocolError,
}

impl BrokerErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BrowserUnavailable => "browser_unavailable",
            Self::IncompatibleBrowserSession => "incompatible_browser_session",
            Self::BrowserSessionDisconnected => "browser_session_disconnected",
            Self::AmbiguousBrowserTarget => "ambiguous_browser_target",
            Self::BrowserSessionBusy => "browser_session_busy",
            Self::StaleBrowserTarget => "stale_browser_target",
            Self::StaleElementReference => "stale_element_reference",
            Self::MismatchedBrowserResponse => "mismatched_browser_response",
            Self::ExpiredBrowserLease => "expired_browser_lease",
            Self::InvalidBrowserLease => "invalid_browser_lease",
            Self::BrowserTargetNotFound => "browser_target_not_found",
            Self::BrowserOperationTimeout => "browser_operation_timeout",
            Self::InvalidBrowserOperation => "invalid_browser_operation",
            Self::BrowserCapabilityUnavailable => "browser_capability_unavailable",
            Self::BrowserCapabilityDenied => "browser_capability_denied",
            Self::BrowserArtifactFailure => "browser_artifact_failure",
            Self::DuplicateBrowserMutation => "duplicate_browser_mutation",
            Self::BrowserResourceLimit => "browser_resource_limit",
            Self::BrokerAuthenticationFailed => "broker_authentication_failed",
            Self::BrokerOriginDenied => "broker_origin_denied",
            Self::BrokerProtocolError => "broker_protocol_error",
        }
    }
}

/// Structured internal error with a stable public code and bounded recovery hints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrokerError {
    pub code: BrokerErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub recovery: BTreeMap<String, Value>,
}

impl BrokerError {
    pub fn new(code: BrokerErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            recovery: BTreeMap::new(),
        }
    }
}

/// Returns true for a protocol operation currently defined by Teshi's typed browser API.
/// This list is a fail-closed boundary; operation-specific DTO validation follows in
/// the command dispatcher rather than accepting arbitrary JSON command names.
pub fn is_supported_operation(operation: &str) -> bool {
    matches!(
        operation,
        "list_browser_sessions"
            | "list_browser_tabs"
            | "lookup_browser_sessions"
            | "acquire_browser_lease"
            | "renew_browser_lease"
            | "release_browser_lease"
            | "create_browser_capability_grant"
            | "list_browser_capability_grants"
            | "revoke_browser_capability_grant"
            | "expire_browser_capability_grants"
            | "list_browser_privileged_audit"
            | "execute_privileged_javascript"
            | "execute_privileged_cdp"
            | "list_browser_cookies"
            | "access_browser_content_setting"
            | "list_browser_extensions"
            | "get_page_snapshot"
            | "navigate"
            | "go_back"
            | "open_tab"
            | "close_tab"
            | "activate_tab"
            | "create_window"
            | "group_tabs"
            | "resolve_playwright_locator"
            | "verify_playwright_locator"
            | "execute_browser_action"
            | "capture_browser_evidence"
            | "capture_browser_screenshot"
            | "generate_browser_pdf"
            | "start_console_capture"
            | "list_console_events"
            | "clear_console_capture"
            | "stop_console_capture"
            | "start_network_capture"
            | "list_network_requests"
            | "get_network_request_detail"
            | "clear_network_capture"
            | "stop_network_capture"
            | "set_browser_profile_label"
            | "clear_browser_profile_label"
            | "cleanup_browser_artifacts"
    )
}

/// Require a capability to be implemented by this broker generation and
/// advertised by the selected extension Profile before dispatch.
pub fn negotiate_feature(
    feature: &str,
    broker_features: &[String],
    extension: &ExtensionHeartbeat,
) -> Result<(), BrokerError> {
    if feature.trim().is_empty() || feature.len() > 128 {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "required browser feature name is invalid",
        ));
    }
    if !broker_features.iter().any(|available| available == feature)
        || !extension.supports_feature(feature)
    {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserCapabilityUnavailable,
            format!("required browser feature is unavailable: {feature}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixtures() -> Value {
        serde_json::from_str(include_str!(
            "../../../resources/browser_contract_fixtures.json"
        ))
        .expect("shared browser fixtures must be valid JSON")
    }

    #[test]
    fn shared_contract_fixtures_parse_in_typed_rust_records() {
        let fixture = fixtures();
        let old: ExtensionHeartbeat =
            serde_json::from_value(fixture["legacy"]["heartbeat"].clone()).unwrap();
        assert!(!old.is_versioned());
        assert_eq!(old.active_tab_id, Some(42));

        for key in ["p0_only_heartbeat", "p0_p1_heartbeat"] {
            let heartbeat: ExtensionHeartbeat =
                serde_json::from_value(fixture["phased"][key].clone()).unwrap();
            assert!(heartbeat.is_versioned());
            assert!(heartbeat.supports_feature("p0.control"));
        }

        let target: BrowserTarget = serde_json::from_value(
            fixture["migration_contracts"]["authorization"]["owner_profile"]
                .as_str()
                .map(|id| json!({"extension_instance_id": id, "window_id": 7, "tab_id": 42}))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(target.extension_instance_id, "profile-a");
        assert_eq!(CHROME_DISCOVERY_PORT, 17_373);
        assert_eq!(MAX_WEBSOCKET_MESSAGE_BYTES, 75_497_472);
    }

    #[test]
    fn unknown_operation_and_oversized_network_batch_fail_before_dispatch() {
        let unknown: OperationRequest = serde_json::from_value(json!({
            "request_id": "unknown-1",
            "cmd": "execute_undocumented_privileged_operation",
        }))
        .unwrap();
        assert_eq!(
            unknown.validate().unwrap_err().code,
            BrokerErrorCode::InvalidBrowserOperation
        );
        assert!(!is_supported_operation(&unknown.operation));

        let scenario = fixtures()["migration_contracts"]["network_sequence"].clone();
        let target = BrowserTarget {
            extension_instance_id: "profile-a".into(),
            window_id: 7,
            tab_id: 42,
        };
        let events = (0..scenario["oversized_event_count"].as_u64().unwrap())
            .map(|seq| NetworkEvent {
                seq,
                data: BTreeMap::new(),
            })
            .collect();
        let batch = NetworkBatch {
            message_type: "network_batch".into(),
            extension_instance_id: "profile-a".into(),
            capture_id: "capture-a".into(),
            target,
            events,
            first_seq: Some(1),
            last_seq: Some(101),
            dropped_events: 0,
            dropped_bytes: 0,
            dropped_events_total: 0,
            dropped_bytes_total: 0,
            termination_reason: None,
            termination_detail: None,
            final_sequence: None,
            diagnostics: None,
        };
        assert_eq!(
            batch.validate().unwrap_err().code,
            BrokerErrorCode::BrowserResourceLimit
        );
    }

    #[test]
    fn operation_request_retains_existing_v1_envelope_and_rejects_bad_schema() {
        let request: OperationRequest = serde_json::from_value(json!({
            "schema_version": 1,
            "request_id": "request-1",
            "caller_label": "fixture-agent",
            "project_root": "/work/project-a",
            "timeout_ms": 30_000,
            "cmd": "get_page_snapshot",
            "target": {"extension_instance_id": "profile-a", "window_id": 7, "tab_id": 42},
            "lease_token": "fixture-secret",
            "page_context_revision": "rev-1"
        }))
        .unwrap();
        assert!(request.validate().is_ok());
        assert_eq!(request.arguments["page_context_revision"], "rev-1");

        let bad = OperationRequest {
            schema_version: Some(2),
            ..request
        };
        assert_eq!(
            bad.validate().unwrap_err().code,
            BrokerErrorCode::IncompatibleBrowserSession
        );
    }

    #[test]
    fn feature_negotiation_requires_broker_and_extension_support() {
        let fixture = fixtures();
        let extension: ExtensionHeartbeat =
            serde_json::from_value(fixture["phased"]["p0_p1_heartbeat"].clone()).unwrap();
        assert!(
            negotiate_feature(
                "p1.filtered_network_capture",
                &["p0.control".into(), "p1.filtered_network_capture".into(),],
                &extension
            )
            .is_ok()
        );
        assert_eq!(
            negotiate_feature("p1.observability_artifacts", &[], &extension)
                .unwrap_err()
                .code,
            BrokerErrorCode::BrowserCapabilityUnavailable
        );

        let limited: ExtensionHeartbeat =
            serde_json::from_value(fixture["phased"]["p0_only_heartbeat"].clone()).unwrap();
        assert_eq!(
            negotiate_feature(
                "p1.filtered_network_capture",
                &["p1.filtered_network_capture".into()],
                &limited
            )
            .unwrap_err()
            .code,
            BrokerErrorCode::BrowserCapabilityUnavailable
        );
    }
}
