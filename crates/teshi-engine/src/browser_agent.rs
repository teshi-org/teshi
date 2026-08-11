//! Typed operations and contracts for external browser-testing agents.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::sidecar::send_sidecar_command_with_timeout;

/// Current browser-agent schema version.
pub const BROWSER_AGENT_SCHEMA_VERSION: u16 = 1;
/// Current extension-to-broker protocol version.
pub const BROWSER_BROKER_PROTOCOL_VERSION: u16 = 1;
/// Default exclusive lease lifetime in seconds.
pub const DEFAULT_BROWSER_LEASE_TTL_SECS: u64 = 60;

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Stable health state reported for one browser extension session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSessionHealth {
    /// The extension is live and protocol-compatible.
    Ready,
    /// No heartbeat has arrived within the configured lifetime.
    Disconnected,
    /// The extension and broker protocol versions cannot communicate safely.
    Incompatible,
    /// The browser debugger is owned by DevTools or another automation client.
    DebuggerConflict,
    /// The session heartbeat is near or past expiry.
    Stale,
}

/// Stable error codes shared by broker, CLI, and MCP adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAgentErrorCode {
    /// No eligible browser extension is registered.
    BrowserUnavailable,
    /// Component or protocol versions are incompatible.
    IncompatibleBrowserSession,
    /// The selected extension is disconnected.
    BrowserSessionDisconnected,
    /// More than one target exists and no explicit target was supplied.
    AmbiguousBrowserTarget,
    /// Another owner holds the exclusive session lease.
    BrowserSessionBusy,
    /// The selected target or page revision is no longer current.
    StaleBrowserTarget,
    /// A response does not match its pending request or target.
    MismatchedBrowserResponse,
    /// The supplied lease has expired.
    ExpiredBrowserLease,
    /// The supplied lease token is absent or invalid.
    InvalidBrowserLease,
    /// The selected session, window, or tab does not exist.
    BrowserTargetNotFound,
    /// The browser did not respond before the operation timeout.
    BrowserOperationTimeout,
    /// The requested operation or input is invalid.
    InvalidBrowserOperation,
    /// The browser rejected an operation for a component-specific reason.
    BrowserOperationFailed,
}

impl BrowserAgentErrorCode {
    /// Returns the stable snake-case wire value for this code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BrowserUnavailable => "browser_unavailable",
            Self::IncompatibleBrowserSession => "incompatible_browser_session",
            Self::BrowserSessionDisconnected => "browser_session_disconnected",
            Self::AmbiguousBrowserTarget => "ambiguous_browser_target",
            Self::BrowserSessionBusy => "browser_session_busy",
            Self::StaleBrowserTarget => "stale_browser_target",
            Self::MismatchedBrowserResponse => "mismatched_browser_response",
            Self::ExpiredBrowserLease => "expired_browser_lease",
            Self::InvalidBrowserLease => "invalid_browser_lease",
            Self::BrowserTargetNotFound => "browser_target_not_found",
            Self::BrowserOperationTimeout => "browser_operation_timeout",
            Self::InvalidBrowserOperation => "invalid_browser_operation",
            Self::BrowserOperationFailed => "browser_operation_failed",
        }
    }
}

/// Error returned by a typed browser operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserAgentError {
    /// Stable machine-readable error code.
    pub code: BrowserAgentErrorCode,
    /// Actionable user-facing explanation.
    pub message: String,
    /// Non-sensitive recovery metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub recovery: BTreeMap<String, Value>,
}

impl fmt::Display for BrowserAgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for BrowserAgentError {}

impl BrowserAgentError {
    /// Renders the shared machine-readable failure envelope used by CLI and MCP.
    pub fn to_wire_value(&self) -> Value {
        json!({
            "ok": false,
            "schema_version": BROWSER_AGENT_SCHEMA_VERSION,
            "code": self.code.as_str(),
            "error": self.message,
            "recovery": self.recovery,
        })
    }
}

/// Opaque extension identity persisted inside one browser profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionIdentity {
    /// Stable opaque routing identifier.
    pub extension_instance_id: String,
    /// Optional display-only label. Labels are never routing keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_label: Option<String>,
    /// Installed extension version.
    pub extension_version: String,
    /// Extension-to-broker protocol version.
    pub protocol_version: u16,
}

/// Non-sensitive browser component metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserMetadata {
    /// Browser product name.
    pub name: String,
    /// Browser product version when detectable.
    pub version: String,
    /// Operating-system family when detectable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

/// One browser tab advertised by an extension session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserTab {
    /// Browser-local numeric tab identifier.
    pub id: i64,
    /// Browser-local numeric window identifier.
    pub window_id: i64,
    /// Redactable human-readable title.
    #[serde(default)]
    pub title: String,
    /// Redactable page URL.
    #[serde(default)]
    pub url: String,
    /// Whether the tab is active in its window.
    #[serde(default)]
    pub active: bool,
    /// Whether Chromium permits debugger attachment for the tab.
    #[serde(default)]
    pub debuggable: bool,
}

/// One browser window and its current tab inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserWindow {
    /// Browser-local window identifier.
    pub id: i64,
    /// Whether this is the focused browser window.
    #[serde(default)]
    pub focused: bool,
    /// Tabs currently visible to the extension.
    #[serde(default)]
    pub tabs: Vec<BrowserTab>,
}

/// Canonical target for every browser command and result.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BrowserTarget {
    /// Opaque extension instance identifier.
    pub extension_instance_id: String,
    /// Browser-local window identifier scoped by the extension instance.
    pub window_id: i64,
    /// Browser-local tab identifier scoped by the extension instance.
    pub tab_id: i64,
}

/// Public lease state shown in session discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserLeaseSummary {
    /// Display-only owner label.
    pub owner_label: String,
    /// Lease expiry as Unix epoch milliseconds.
    pub expires_at_ms: u64,
}

/// One registered browser-profile extension session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSession {
    /// Version of this serialized contract.
    pub schema_version: u16,
    /// Stable extension identity.
    pub identity: ExtensionIdentity,
    /// Browser product metadata.
    pub browser: BrowserMetadata,
    /// Current health state.
    pub health: BrowserSessionHealth,
    /// Age of the last heartbeat.
    pub last_heartbeat_age_ms: u64,
    /// Current browser windows and tabs.
    #[serde(default)]
    pub windows: Vec<BrowserWindow>,
    /// Current exclusive lease, without its secret token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<BrowserLeaseSummary>,
}

/// Exclusive mutation lease returned to its acquiring caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserLease {
    /// Opaque extension instance protected by the lease.
    pub extension_instance_id: String,
    /// Secret token required for subsequent mutable operations.
    pub lease_token: String,
    /// Display-only owner label.
    pub owner_label: String,
    /// Acquisition timestamp as Unix epoch milliseconds.
    pub acquired_at_ms: u64,
    /// Expiry timestamp as Unix epoch milliseconds.
    pub expires_at_ms: u64,
}

/// Stable page-document revision used to reject stale locator results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageContextRevision(pub String);

/// Frame context for a snapshot element or locator candidate.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LocatorContext {
    /// Frame URL or logical frame path, if the element is not in the main frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    /// Shadow-root host path when the element is inside shadow DOM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_root: Option<String>,
}

/// Accessible, interactive element normalized outside any UI shell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibleElement {
    /// Snapshot-local stable reference.
    pub element_ref: String,
    /// HTML tag name.
    pub tag: String,
    /// Computed or explicit accessible role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Computed accessible name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessible_name: Option<String>,
    /// Associated label text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Placeholder text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Visible text truncated for diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Non-sensitive attributes used for locator generation.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    /// Frame and shadow-root context.
    #[serde(default)]
    pub context: LocatorContext,
}

/// Structured page snapshot used by locator acquisition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserPageSnapshot {
    /// Version of this serialized contract.
    pub schema_version: u16,
    /// Correlated operation request identifier.
    pub request_id: String,
    /// Explicit browser target.
    pub target: BrowserTarget,
    /// Current page-document revision.
    pub page_context_revision: PageContextRevision,
    /// Current page URL.
    pub url: String,
    /// Current page title.
    pub title: String,
    /// Normalized interactive elements.
    #[serde(default)]
    pub elements: Vec<AccessibleElement>,
}

/// Caller-provided intent used to select an element without inventing actions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocatorIntent {
    /// Human-readable purpose such as "submit button".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// Expected visible or accessible text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Expected accessible role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Snapshot-local element reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_ref: Option<String>,
    /// Selected Gherkin step text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gherkin_step: Option<String>,
}

/// Supported Playwright locator strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaywrightLocatorKind {
    /// `page.getByRole(...)`.
    Role,
    /// `page.getByLabel(...)`.
    Label,
    /// `page.getByPlaceholder(...)`.
    Placeholder,
    /// `page.getByTestId(...)`.
    TestId,
    /// Stable attribute-based `page.locator(...)`.
    Attribute,
    /// CSS fallback through `page.locator(...)`.
    Css,
    /// Text locator fallback.
    Text,
}

/// Browser-observed locator verification state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocatorVerificationStatus {
    /// The candidate uniquely resolved to an appropriate element.
    Verified,
    /// The candidate matched more than one element.
    Ambiguous,
    /// The candidate matched no element.
    NotFound,
    /// The candidate resolved but was not visible or actionable.
    NotActionable,
    /// Verification was skipped or not supported.
    Unverified,
    /// The page changed between snapshot and verification.
    StalePageContext,
}

/// One ranked Playwright locator candidate and its evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaywrightLocatorCandidate {
    /// Strategy kind.
    pub kind: PlaywrightLocatorKind,
    /// Structured arguments used to render and verify the locator.
    #[serde(default)]
    pub arguments: BTreeMap<String, Value>,
    /// Directly usable Playwright expression.
    pub expression: String,
    /// Frame or shadow-root context.
    #[serde(default)]
    pub context: LocatorContext,
    /// Browser-observed match count.
    pub match_count: u32,
    /// Whether the resolved element is visible.
    pub visible: bool,
    /// Whether the resolved element is enabled.
    pub enabled: bool,
    /// Verification outcome.
    pub verification: LocatorVerificationStatus,
    /// Relative ranking score; higher is preferred.
    pub score: i32,
    /// Human-readable stability rationale.
    pub stability_rationale: String,
    /// Machine-readable fragility or compatibility warnings.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Ranked and verified Playwright locator result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaywrightLocatorResult {
    /// Version of this serialized contract.
    pub schema_version: u16,
    /// Correlated operation request identifier.
    pub request_id: String,
    /// Explicit browser target.
    pub target: BrowserTarget,
    /// Page revision used for candidate generation and verification.
    pub page_context_revision: PageContextRevision,
    /// Highest-ranked verified candidate, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended: Option<PlaywrightLocatorCandidate>,
    /// All ranked candidates, including rejected alternatives.
    #[serde(default)]
    pub candidates: Vec<PlaywrightLocatorCandidate>,
}

/// Optional screenshot evidence tied to one request and browser target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserEvidenceReference {
    /// Correlated operation request identifier.
    pub request_id: String,
    /// Explicit browser target.
    pub target: BrowserTarget,
    /// Media type of the referenced evidence.
    pub media_type: String,
    /// Local path or opaque inline-reference identifier.
    pub reference: String,
    /// Page revision at capture time.
    pub page_context_revision: PageContextRevision,
}

/// Typed operation executed identically by CLI and MCP adapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum BrowserOperation {
    /// List all registered extension sessions.
    ListBrowserSessions,
    /// List windows and tabs for one extension session.
    ListBrowserTabs {
        /// Opaque extension instance identifier.
        extension_instance_id: String,
    },
    /// Acquire an exclusive instance-level mutation lease.
    AcquireBrowserLease {
        /// Opaque extension instance identifier.
        extension_instance_id: String,
        /// Display-only owner label.
        owner_label: String,
        /// Requested bounded lifetime.
        ttl_secs: u64,
    },
    /// Renew an existing mutation lease.
    RenewBrowserLease {
        /// Opaque extension instance identifier.
        extension_instance_id: String,
        /// Secret token returned by acquisition.
        lease_token: String,
        /// Requested bounded lifetime.
        ttl_secs: u64,
    },
    /// Release an existing mutation lease.
    ReleaseBrowserLease {
        /// Opaque extension instance identifier.
        extension_instance_id: String,
        /// Secret token returned by acquisition.
        lease_token: String,
    },
    /// Acquire a structured page snapshot.
    GetPageSnapshot {
        /// Explicit target.
        target: BrowserTarget,
        /// Valid exclusive lease token.
        lease_token: String,
    },
    /// Generate and verify ranked Playwright locator candidates.
    ResolvePlaywrightLocator {
        /// Explicit target.
        target: BrowserTarget,
        /// Valid exclusive lease token.
        lease_token: String,
        /// Caller-provided element intent.
        intent: LocatorIntent,
        /// Project-configured test-id attributes.
        test_id_attributes: Vec<String>,
    },
    /// Verify a previously generated Playwright locator candidate.
    VerifyPlaywrightLocator {
        /// Explicit target.
        target: BrowserTarget,
        /// Valid exclusive lease token.
        lease_token: String,
        /// Candidate to evaluate.
        candidate: PlaywrightLocatorCandidate,
        /// Expected page revision.
        page_context_revision: PageContextRevision,
    },
    /// Capture request-scoped screenshot evidence.
    CaptureBrowserEvidence {
        /// Explicit target.
        target: BrowserTarget,
        /// Valid exclusive lease token.
        lease_token: String,
        /// Expected page revision.
        page_context_revision: PageContextRevision,
    },
}

impl BrowserOperation {
    /// Returns the stable wire operation name.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::ListBrowserSessions => "list_browser_sessions",
            Self::ListBrowserTabs { .. } => "list_browser_tabs",
            Self::AcquireBrowserLease { .. } => "acquire_browser_lease",
            Self::RenewBrowserLease { .. } => "renew_browser_lease",
            Self::ReleaseBrowserLease { .. } => "release_browser_lease",
            Self::GetPageSnapshot { .. } => "get_page_snapshot",
            Self::ResolvePlaywrightLocator { .. } => "resolve_playwright_locator",
            Self::VerifyPlaywrightLocator { .. } => "verify_playwright_locator",
            Self::CaptureBrowserEvidence { .. } => "capture_browser_evidence",
        }
    }

    /// Serializes the operation into the shared sidecar command envelope.
    pub fn to_sidecar_command(&self, request_id: &str) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or_else(|_| json!({}));
        if let Some(object) = value.as_object_mut() {
            object.insert("cmd".into(), Value::String(self.name().into()));
            object.remove("operation");
            object.insert(
                "schema_version".into(),
                Value::from(BROWSER_AGENT_SCHEMA_VERSION),
            );
            object.insert("request_id".into(), Value::String(request_id.into()));
        }
        value
    }
}

/// Successful typed operation response preserving operation-specific payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserOperationResponse {
    /// Version of this serialized contract.
    pub schema_version: u16,
    /// Stable operation name.
    pub operation: String,
    /// Correlated request identifier.
    pub request_id: String,
    /// Complete operation-specific JSON result.
    pub payload: Value,
}

/// Reusable browser operation client backed by the existing local WebSocket.
#[derive(Debug, Clone)]
pub struct BrowserOperations {
    ws_url: String,
    timeout: Duration,
}

impl BrowserOperations {
    /// Creates a client for one local Teshi browser sidecar WebSocket.
    pub fn new(ws_url: impl Into<String>, timeout: Duration) -> Self {
        Self {
            ws_url: ws_url.into(),
            timeout,
        }
    }

    /// Executes a typed operation and validates the shared success/error envelope.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserAgentError`] when transport fails, the broker rejects the
    /// request, or the response cannot be correlated with the operation.
    pub fn execute(
        &self,
        operation: &BrowserOperation,
    ) -> Result<BrowserOperationResponse, BrowserAgentError> {
        let request_id = next_request_id();
        let command = operation.to_sidecar_command(&request_id);
        let response = send_sidecar_command_with_timeout(&self.ws_url, command, self.timeout)
            .map_err(|message| BrowserAgentError {
                code: if message.to_ascii_lowercase().contains("timed out") {
                    BrowserAgentErrorCode::BrowserOperationTimeout
                } else {
                    BrowserAgentErrorCode::BrowserUnavailable
                },
                message,
                recovery: BTreeMap::new(),
            })?;
        parse_operation_response(operation.name(), &request_id, response)
    }
}

fn next_request_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "browser-agent-{}-{timestamp}-{sequence}",
        std::process::id()
    )
}

fn parse_operation_response(
    expected_operation: &str,
    expected_request_id: &str,
    response: Value,
) -> Result<BrowserOperationResponse, BrowserAgentError> {
    let actual_request_id = response
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if actual_request_id != expected_request_id {
        return Err(BrowserAgentError {
            code: BrowserAgentErrorCode::MismatchedBrowserResponse,
            message: format!(
                "browser response request_id mismatch: expected {expected_request_id}, got {actual_request_id}"
            ),
            recovery: BTreeMap::new(),
        });
    }
    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        let code = response
            .get("code")
            .and_then(Value::as_str)
            .and_then(parse_error_code)
            .unwrap_or(BrowserAgentErrorCode::BrowserOperationFailed);
        let message = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("browser operation failed")
            .to_string();
        let recovery = response
            .get("recovery")
            .and_then(Value::as_object)
            .map(|object| {
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        return Err(BrowserAgentError {
            code,
            message,
            recovery,
        });
    }
    let operation = response
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or(expected_operation)
        .to_string();
    if operation != expected_operation {
        return Err(BrowserAgentError {
            code: BrowserAgentErrorCode::MismatchedBrowserResponse,
            message: format!(
                "browser response operation mismatch: expected {expected_operation}, got {operation}"
            ),
            recovery: BTreeMap::new(),
        });
    }
    Ok(BrowserOperationResponse {
        schema_version: response
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(BROWSER_AGENT_SCHEMA_VERSION),
        operation,
        request_id: actual_request_id.to_string(),
        payload: response,
    })
}

fn parse_error_code(value: &str) -> Option<BrowserAgentErrorCode> {
    Some(match value {
        "browser_unavailable" => BrowserAgentErrorCode::BrowserUnavailable,
        "incompatible_browser_session" => BrowserAgentErrorCode::IncompatibleBrowserSession,
        "browser_session_disconnected" => BrowserAgentErrorCode::BrowserSessionDisconnected,
        "ambiguous_browser_target" => BrowserAgentErrorCode::AmbiguousBrowserTarget,
        "browser_session_busy" => BrowserAgentErrorCode::BrowserSessionBusy,
        "stale_browser_target" | "stale_page_context" => BrowserAgentErrorCode::StaleBrowserTarget,
        "mismatched_browser_response" => BrowserAgentErrorCode::MismatchedBrowserResponse,
        "expired_browser_lease" => BrowserAgentErrorCode::ExpiredBrowserLease,
        "invalid_browser_lease" => BrowserAgentErrorCode::InvalidBrowserLease,
        "browser_target_not_found" => BrowserAgentErrorCode::BrowserTargetNotFound,
        "browser_operation_timeout" => BrowserAgentErrorCode::BrowserOperationTimeout,
        "invalid_browser_operation" => BrowserAgentErrorCode::InvalidBrowserOperation,
        "browser_operation_failed" => BrowserAgentErrorCode::BrowserOperationFailed,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> BrowserTarget {
        BrowserTarget {
            extension_instance_id: "extension-a".into(),
            window_id: 7,
            tab_id: 42,
        }
    }

    #[test]
    fn target_identity_survives_serialization_round_trip() {
        let serialized = serde_json::to_string(&target()).unwrap();
        let restored: BrowserTarget = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored, target());
    }

    #[test]
    fn operation_command_preserves_request_target_and_lease() {
        let operation = BrowserOperation::GetPageSnapshot {
            target: target(),
            lease_token: "lease-secret".into(),
        };
        let command = operation.to_sidecar_command("request-1");
        assert_eq!(command["cmd"], "get_page_snapshot");
        assert_eq!(command["request_id"], "request-1");
        assert_eq!(command["target"]["extension_instance_id"], "extension-a");
        assert_eq!(command["target"]["window_id"], 7);
        assert_eq!(command["target"]["tab_id"], 42);
        assert_eq!(command["lease_token"], "lease-secret");
    }

    #[test]
    fn locator_result_survives_adapter_round_trip() {
        let candidate = PlaywrightLocatorCandidate {
            kind: PlaywrightLocatorKind::Role,
            arguments: BTreeMap::from([
                ("role".into(), json!("button")),
                ("name".into(), json!("Save")),
            ]),
            expression: "page.getByRole('button', { name: 'Save', exact: true })".into(),
            context: LocatorContext::default(),
            match_count: 1,
            visible: true,
            enabled: true,
            verification: LocatorVerificationStatus::Verified,
            score: 100,
            stability_rationale: "unique accessible role and name".into(),
            warnings: vec![],
        };
        let result = PlaywrightLocatorResult {
            schema_version: BROWSER_AGENT_SCHEMA_VERSION,
            request_id: "request-2".into(),
            target: target(),
            page_context_revision: PageContextRevision("revision-1".into()),
            recommended: Some(candidate.clone()),
            candidates: vec![candidate],
        };
        let restored: PlaywrightLocatorResult =
            serde_json::from_value(serde_json::to_value(result).unwrap()).unwrap();
        assert_eq!(restored.request_id, "request-2");
        assert_eq!(restored.target, target());
        assert_eq!(restored.candidates[0].match_count, 1);
    }

    #[test]
    fn stable_error_codes_match_wire_contract() {
        assert_eq!(
            BrowserAgentErrorCode::AmbiguousBrowserTarget.as_str(),
            "ambiguous_browser_target"
        );
        assert_eq!(
            BrowserAgentErrorCode::ExpiredBrowserLease.as_str(),
            "expired_browser_lease"
        );
        assert_eq!(
            parse_error_code("mismatched_browser_response"),
            Some(BrowserAgentErrorCode::MismatchedBrowserResponse)
        );
    }

    #[test]
    fn shared_failure_payload_preserves_cli_and_mcp_scenarios() {
        for code in [
            BrowserAgentErrorCode::AmbiguousBrowserTarget,
            BrowserAgentErrorCode::BrowserSessionBusy,
            BrowserAgentErrorCode::ExpiredBrowserLease,
            BrowserAgentErrorCode::StaleBrowserTarget,
            BrowserAgentErrorCode::BrowserOperationTimeout,
            BrowserAgentErrorCode::IncompatibleBrowserSession,
        ] {
            let error = BrowserAgentError {
                code,
                message: format!("{} test", code.as_str()),
                recovery: BTreeMap::from([("retryable".into(), json!(false))]),
            };
            let payload = error.to_wire_value();
            assert_eq!(payload["ok"], false);
            assert_eq!(payload["code"], code.as_str());
            assert_eq!(payload["recovery"]["retryable"], false);
        }
    }

    #[test]
    fn response_correlation_rejects_wrong_request_id() {
        let error = parse_operation_response(
            "list_browser_sessions",
            "request-a",
            json!({"ok": true, "request_id": "request-b"}),
        )
        .unwrap_err();
        assert_eq!(error.code, BrowserAgentErrorCode::MismatchedBrowserResponse);
    }
}
