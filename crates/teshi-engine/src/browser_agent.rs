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
/// Default privileged browser capability grant lifetime in seconds.
pub const DEFAULT_BROWSER_CAPABILITY_GRANT_TTL_SECS: u64 = 300;

/// Stable rollout phase for negotiated browser protocol features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserFeaturePhase {
    /// Safe browser-control primitives.
    P0,
    /// Bounded observability and artifact operations.
    P1,
    /// Explicitly granted privileged operations.
    P2,
}

/// Extensible typed identifier for one independently negotiated feature.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BrowserFeatureId(pub String);

impl BrowserFeatureId {
    /// Complete safe P0 control loop.
    pub const P0_CONTROL: &'static str = "p0.control";
    /// Bounded P1 observability and artifact surface.
    pub const P1_OBSERVABILITY_ARTIFACTS: &'static str = "p1.observability_artifacts";
    /// P2 arbitrary JavaScript execution.
    pub const P2_JAVASCRIPT: &'static str = "p2.javascript";
    /// P2 allowlisted raw CDP execution.
    pub const P2_RAW_CDP: &'static str = "p2.raw_cdp";
    /// P2 Cookie metadata/value access.
    pub const P2_COOKIES: &'static str = "p2.cookies";
    /// P2 content-setting access.
    pub const P2_CONTENT_SETTINGS: &'static str = "p2.content_settings";
    /// P2 extension-management access.
    pub const P2_EXTENSION_MANAGEMENT: &'static str = "p2.extension_management";

    /// Construct an identifier while preserving forward-compatible unknown values.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Return the stable wire identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Infer the rollout phase from the stable identifier prefix.
    pub fn phase(&self) -> Option<BrowserFeaturePhase> {
        if self.0.starts_with("p0.") {
            Some(BrowserFeaturePhase::P0)
        } else if self.0.starts_with("p1.") {
            Some(BrowserFeaturePhase::P1)
        } else if self.0.starts_with("p2.") {
            Some(BrowserFeaturePhase::P2)
        } else {
            None
        }
    }
}

/// Public availability of one negotiated protocol feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserFeatureAvailability {
    /// Stable extensible feature identifier.
    pub feature: BrowserFeatureId,
    /// Whether the selected extension/backend can execute the feature now.
    pub available: bool,
    /// Non-sensitive reason when the feature is unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Stable browser actions advertised during discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    /// Synthetic DOM activation.
    Click,
    /// CDP-backed pointer activation.
    PointerClick,
    /// Replace a form control's value.
    Fill,
    /// Enter text using the backend's typing semantics.
    Type,
    /// Select an option.
    Select,
    /// Dispatch a named key.
    PressKey,
    /// Assert visibility without mutation.
    AssertVisible,
    /// Assert text without mutation.
    AssertText,
    /// Navigate the selected tab.
    Navigate,
    /// Upload an explicitly supplied file.
    Upload,
}

/// Non-sensitive capability metadata returned in session discovery.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserCapabilities {
    /// Independently negotiated phased features.
    #[serde(default)]
    pub features: Vec<BrowserFeatureAvailability>,
    /// Actions implemented by the selected backend.
    #[serde(default)]
    pub supported_actions: Vec<BrowserAction>,
    /// Independently advertised browser operations implemented by the backend.
    #[serde(default)]
    pub supported_operations: Vec<String>,
    /// Optional Chromium permission availability; never contains grant tokens.
    #[serde(default)]
    pub optional_permissions: BTreeMap<String, bool>,
}

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
    /// A snapshot-local element reference is expired or belongs to another context.
    StaleElementReference,
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
    /// The selected backend does not advertise the requested action.
    UnsupportedBrowserAction,
    /// A browser or extension capability is not implemented or permitted by Chromium.
    BrowserCapabilityUnavailable,
    /// A privileged operation lacks an effective explicit grant.
    BrowserCapabilityDenied,
    /// An action completed but its typed post-condition timed out.
    BrowserWaitTimeout,
    /// A requested browser artifact could not be created or validated.
    BrowserArtifactFailed,
    /// A mutating request identifier was already accepted.
    DuplicateBrowserMutation,
    /// A bounded privileged result exceeded the caller-selected limit.
    BrowserResultTooLarge,
    /// Privileged JavaScript completed with an exception.
    BrowserJavascriptException,
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
            Self::StaleElementReference => "stale_element_reference",
            Self::MismatchedBrowserResponse => "mismatched_browser_response",
            Self::ExpiredBrowserLease => "expired_browser_lease",
            Self::InvalidBrowserLease => "invalid_browser_lease",
            Self::BrowserTargetNotFound => "browser_target_not_found",
            Self::BrowserOperationTimeout => "browser_operation_timeout",
            Self::InvalidBrowserOperation => "invalid_browser_operation",
            Self::UnsupportedBrowserAction => "unsupported_browser_action",
            Self::BrowserCapabilityUnavailable => "browser_capability_unavailable",
            Self::BrowserCapabilityDenied => "browser_capability_denied",
            Self::BrowserWaitTimeout => "browser_wait_timeout",
            Self::BrowserArtifactFailed => "browser_artifact_failure",
            Self::DuplicateBrowserMutation => "duplicate_browser_mutation",
            Self::BrowserResultTooLarge => "browser_result_too_large",
            Self::BrowserJavascriptException => "browser_javascript_exception",
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
            "recovery": sanitize_public_value(Value::Object(
                self.recovery.clone().into_iter().collect()
            )),
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
    /// Public feature, action, and optional-permission availability.
    #[serde(default)]
    pub capabilities: BrowserCapabilities,
    /// Current exclusive lease, without its secret token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<BrowserLeaseSummary>,
}

/// Shared metadata surrounding one typed browser operation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserRequestEnvelope {
    /// Version of the typed agent schema.
    pub schema_version: u16,
    /// Extension/broker protocol version expected by the caller.
    pub protocol_version: u16,
    /// Unique correlation identifier.
    pub request_id: String,
    /// Display-only caller identity used for diagnostics and policy.
    pub caller_label: String,
    /// Caller-side operation deadline in milliseconds.
    pub timeout_ms: u64,
    /// Request-scoped project context; the user-session broker does not own a project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    /// Typed operation carrying any explicit target, lease, and page revision.
    pub operation: BrowserOperation,
}

impl BrowserRequestEnvelope {
    /// Build a request envelope without exposing secrets in discovery metadata.
    pub fn new(
        request_id: impl Into<String>,
        caller_label: impl Into<String>,
        timeout: Duration,
        operation: BrowserOperation,
    ) -> Self {
        Self {
            schema_version: BROWSER_AGENT_SCHEMA_VERSION,
            protocol_version: BROWSER_BROKER_PROTOCOL_VERSION,
            request_id: request_id.into(),
            caller_label: caller_label.into(),
            timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            project_root: None,
            operation,
        }
    }

    /// Attach non-secret project context to this request.
    pub fn with_project_root(mut self, project_root: impl Into<String>) -> Self {
        self.project_root = Some(project_root.into());
        self
    }

    /// Flatten the envelope into the existing sidecar wire format.
    pub fn to_sidecar_command(&self) -> Value {
        let mut value = serde_json::to_value(&self.operation).unwrap_or_else(|_| json!({}));
        if let Some(object) = value.as_object_mut() {
            object.insert("cmd".into(), Value::String(self.operation.name().into()));
            object.remove("operation");
            object.insert("schema_version".into(), Value::from(self.schema_version));
            object.insert(
                "protocol_version".into(),
                Value::from(self.protocol_version),
            );
            object.insert("request_id".into(), Value::String(self.request_id.clone()));
            object.insert(
                "caller_label".into(),
                Value::String(self.caller_label.clone()),
            );
            object.insert("timeout_ms".into(), Value::from(self.timeout_ms));
            if let Some(project_root) = &self.project_root {
                object.insert("project_root".into(), Value::String(project_root.clone()));
            }
        }
        value
    }
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
    /// Backend-internal opaque routing identity retained for compatibility.
    #[serde(default)]
    pub element_ref: String,
    /// Compact presentation alias bound to the snapshot, revision, and target.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
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

/// Broker-owned revision-bound element reference record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserElementReferenceRecord {
    pub reference: String,
    pub target: BrowserTarget,
    pub snapshot_id: String,
    pub page_context_revision: PageContextRevision,
    #[serde(default)]
    pub context: LocatorContext,
    pub created_at_ms: u64,
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
    /// Stable identity of the snapshot that issued compact references.
    pub snapshot_id: String,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserScreenshotFormat {
    Png,
    Jpeg,
}

/// Exactly-one element input accepted by canonical browser actions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrowserElementInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<PlaywrightLocatorCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub css: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_context_revision: Option<PageContextRevision>,
}

impl BrowserElementInput {
    /// Reject missing or ambiguous element inputs before any mutation.
    pub fn validate(&self) -> Result<(), &'static str> {
        let count = usize::from(self.reference.is_some())
            + usize::from(self.candidate.is_some())
            + usize::from(self.css.is_some());
        match count {
            1 => Ok(()),
            0 => Err("one of reference, candidate, or css is required"),
            _ => Err("reference, candidate, and css are mutually exclusive"),
        }
    }
}

/// Bounded post-action wait condition shared by every browser backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserWaitCondition {
    Url {
        pattern: String,
    },
    VisibleText {
        text: String,
    },
    ElementState {
        element: Box<BrowserElementInput>,
        state: BrowserElementState,
    },
    PageRevisionChange {
        from: PageContextRevision,
    },
    LoadComplete,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserElementState {
    Visible,
    Hidden,
    Enabled,
    Disabled,
}

/// Console severity retained by bounded P1 diagnostic capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserConsoleLevel {
    Debug,
    Log,
    Info,
    Warn,
    Error,
}

/// Exact privileged scope authorized by a short-lived P2 grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserPrivilegedCapability {
    Javascript,
    RawCdp,
    Cookies,
    CookieValues,
    ContentSettings,
    ExtensionManagement,
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
    LookupBrowserSessions {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extension_instance_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        browser_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<i64>,
    },
    SetBrowserProfileLabel {
        extension_instance_id: String,
        profile_label: String,
    },
    ClearBrowserProfileLabel {
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
    /// Create one short-lived, fully bound P2 capability grant.
    CreateBrowserCapabilityGrant {
        target: BrowserTarget,
        lease_token: String,
        capability: BrowserPrivilegedCapability,
        ttl_secs: u64,
        #[serde(default)]
        interactive_confirmed: bool,
        #[serde(default)]
        non_interactive: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        acknowledged_capability: Option<BrowserPrivilegedCapability>,
    },
    /// List non-secret active grant metadata for this project.
    ListBrowserCapabilityGrants {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extension_instance_id: Option<String>,
    },
    /// Revoke one grant using its public identifier.
    RevokeBrowserCapabilityGrant {
        grant_id: String,
    },
    /// Remove expired grants immediately.
    ExpireBrowserCapabilityGrants,
    /// List bounded metadata-only privileged audit records.
    ListBrowserPrivilegedAudit {
        limit: u64,
    },
    /// Execute bounded arbitrary JavaScript under a matching explicit grant.
    ExecutePrivilegedJavascript {
        target: BrowserTarget,
        lease_token: String,
        capability_grant_token: String,
        expression: String,
        source_kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page_context_revision: Option<PageContextRevision>,
        timeout_ms: u64,
        max_result_bytes: u64,
    },
    /// Execute one policy-allowlisted page-scoped raw CDP method.
    ExecutePrivilegedCdp {
        target: BrowserTarget,
        lease_token: String,
        capability_grant_token: String,
        method: String,
        #[serde(default)]
        params: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page_context_revision: Option<PageContextRevision>,
        max_result_bytes: u64,
    },
    /// List Cookies scoped to the selected tab; values require a second grant.
    ListBrowserCookies {
        target: BrowserTarget,
        lease_token: String,
        capability_grant_token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value_capability_grant_token: Option<String>,
        #[serde(default)]
        include_values: bool,
        max_entries: u64,
        max_result_bytes: u64,
    },
    /// Read or set one allowlisted content setting for the selected tab origin.
    AccessBrowserContentSetting {
        target: BrowserTarget,
        lease_token: String,
        capability_grant_token: String,
        setting: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    /// List bounded, non-sensitive extension metadata.
    ListBrowserExtensions {
        target: BrowserTarget,
        lease_token: String,
        capability_grant_token: String,
        max_entries: u64,
    },
    /// Acquire a structured page snapshot.
    GetPageSnapshot {
        /// Explicit target.
        target: BrowserTarget,
        /// Valid exclusive lease token.
        lease_token: String,
    },
    /// Navigate one explicit leased target.
    NavigateBrowser {
        target: BrowserTarget,
        lease_token: String,
        url: String,
        timeout_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait: Option<BrowserWaitCondition>,
        /// Capture one bounded before/after summary around this single mutation.
        #[serde(default)]
        monitor: bool,
    },
    GoBackBrowser {
        target: BrowserTarget,
        lease_token: String,
        timeout_ms: u64,
    },
    OpenBrowserTab {
        target: BrowserTarget,
        lease_token: String,
        url: String,
        #[serde(default)]
        active: bool,
    },
    CloseBrowserTab {
        target: BrowserTarget,
        lease_token: String,
    },
    ActivateBrowserTab {
        target: BrowserTarget,
        lease_token: String,
        #[serde(default)]
        focus_window: bool,
    },
    CreateBrowserWindow {
        target: BrowserTarget,
        lease_token: String,
        url: String,
        #[serde(default)]
        focused: bool,
    },
    GroupBrowserTabs {
        target: BrowserTarget,
        lease_token: String,
        tab_ids: Vec<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
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
    /// Execute one canonical action against exactly one element input.
    ExecuteBrowserAction {
        target: BrowserTarget,
        lease_token: String,
        action: BrowserAction,
        element: BrowserElementInput,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        /// Explicit local files used only by the upload action.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait: Option<Box<BrowserWaitCondition>>,
        #[serde(default = "default_browser_action_timeout_ms")]
        timeout_ms: u64,
        #[serde(default)]
        focus: bool,
        /// Capture one bounded before/after summary around this single mutation.
        #[serde(default)]
        monitor: bool,
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
    /// Capture a P1 viewport screenshot into managed project artifact storage.
    CaptureBrowserScreenshot {
        target: BrowserTarget,
        lease_token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page_context_revision: Option<PageContextRevision>,
        format: BrowserScreenshotFormat,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quality: Option<u8>,
        #[serde(default)]
        full_page: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element: Option<Box<BrowserElementInput>>,
    },
    /// Generate a target-scoped PDF into managed project artifact storage.
    GenerateBrowserPdf {
        target: BrowserTarget,
        lease_token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page_context_revision: Option<PageContextRevision>,
        paper_format: String,
        #[serde(default)]
        landscape: bool,
        scale: f64,
        #[serde(default)]
        print_background: bool,
    },
    /// Start bounded console capture for one explicitly leased target.
    StartBrowserConsoleCapture {
        target: BrowserTarget,
        lease_token: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        levels: Vec<BrowserConsoleLevel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_age_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_entries: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_bytes: Option<u64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        sensitive_fields: Vec<String>,
    },
    /// List a bounded filtered view of captured console events.
    ListBrowserConsoleEvents {
        target: BrowserTarget,
        lease_token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        levels: Option<Vec<BrowserConsoleLevel>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_age_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_entries: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_bytes: Option<u64>,
    },
    /// Clear retained console events while leaving capture active.
    ClearBrowserConsoleCapture {
        target: BrowserTarget,
        lease_token: String,
    },
    /// Stop target-scoped console capture and discard its in-memory events.
    StopBrowserConsoleCapture {
        target: BrowserTarget,
        lease_token: String,
    },
    /// Start bounded metadata-only network capture for one leased target.
    StartBrowserNetworkCapture {
        target: BrowserTarget,
        lease_token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_age_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_entries: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_bytes: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_body_bytes: Option<u64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        sensitive_fields: Vec<String>,
    },
    /// List bounded network request/response metadata without bodies.
    ListBrowserNetworkRequests {
        target: BrowserTarget,
        lease_token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_age_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_entries: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_bytes: Option<u64>,
    },
    /// Return metadata and optionally an explicitly requested bounded body.
    GetBrowserNetworkRequestDetail {
        target: BrowserTarget,
        lease_token: String,
        network_request_id: String,
        #[serde(default)]
        include_body: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_body_bytes: Option<u64>,
    },
    /// Clear retained network metadata while leaving capture active.
    ClearBrowserNetworkCapture {
        target: BrowserTarget,
        lease_token: String,
    },
    /// Stop network capture and discard its retained metadata.
    StopBrowserNetworkCapture {
        target: BrowserTarget,
        lease_token: String,
    },
    /// Explicitly remove only caller-named managed browser artifacts.
    CleanupBrowserArtifacts {
        paths: Vec<String>,
    },
}

const fn default_browser_action_timeout_ms() -> u64 {
    5_000
}

impl BrowserOperation {
    /// Returns the stable wire operation name.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::ListBrowserSessions => "list_browser_sessions",
            Self::ListBrowserTabs { .. } => "list_browser_tabs",
            Self::LookupBrowserSessions { .. } => "lookup_browser_sessions",
            Self::SetBrowserProfileLabel { .. } => "set_browser_profile_label",
            Self::ClearBrowserProfileLabel { .. } => "clear_browser_profile_label",
            Self::AcquireBrowserLease { .. } => "acquire_browser_lease",
            Self::RenewBrowserLease { .. } => "renew_browser_lease",
            Self::ReleaseBrowserLease { .. } => "release_browser_lease",
            Self::CreateBrowserCapabilityGrant { .. } => "create_browser_capability_grant",
            Self::ListBrowserCapabilityGrants { .. } => "list_browser_capability_grants",
            Self::RevokeBrowserCapabilityGrant { .. } => "revoke_browser_capability_grant",
            Self::ExpireBrowserCapabilityGrants => "expire_browser_capability_grants",
            Self::ListBrowserPrivilegedAudit { .. } => "list_browser_privileged_audit",
            Self::ExecutePrivilegedJavascript { .. } => "execute_privileged_javascript",
            Self::ExecutePrivilegedCdp { .. } => "execute_privileged_cdp",
            Self::ListBrowserCookies { .. } => "list_browser_cookies",
            Self::AccessBrowserContentSetting { .. } => "access_browser_content_setting",
            Self::ListBrowserExtensions { .. } => "list_browser_extensions",
            Self::GetPageSnapshot { .. } => "get_page_snapshot",
            Self::NavigateBrowser { .. } => "navigate",
            Self::GoBackBrowser { .. } => "go_back",
            Self::OpenBrowserTab { .. } => "open_tab",
            Self::CloseBrowserTab { .. } => "close_tab",
            Self::ActivateBrowserTab { .. } => "activate_tab",
            Self::CreateBrowserWindow { .. } => "create_window",
            Self::GroupBrowserTabs { .. } => "group_tabs",
            Self::ResolvePlaywrightLocator { .. } => "resolve_playwright_locator",
            Self::VerifyPlaywrightLocator { .. } => "verify_playwright_locator",
            Self::ExecuteBrowserAction { .. } => "execute_browser_action",
            Self::CaptureBrowserEvidence { .. } => "capture_browser_evidence",
            Self::CaptureBrowserScreenshot { .. } => "capture_browser_screenshot",
            Self::GenerateBrowserPdf { .. } => "generate_browser_pdf",
            Self::StartBrowserConsoleCapture { .. } => "start_console_capture",
            Self::ListBrowserConsoleEvents { .. } => "list_console_events",
            Self::ClearBrowserConsoleCapture { .. } => "clear_console_capture",
            Self::StopBrowserConsoleCapture { .. } => "stop_console_capture",
            Self::StartBrowserNetworkCapture { .. } => "start_network_capture",
            Self::ListBrowserNetworkRequests { .. } => "list_network_requests",
            Self::GetBrowserNetworkRequestDetail { .. } => "get_network_request_detail",
            Self::ClearBrowserNetworkCapture { .. } => "clear_network_capture",
            Self::StopBrowserNetworkCapture { .. } => "stop_network_capture",
            Self::CleanupBrowserArtifacts { .. } => "cleanup_browser_artifacts",
        }
    }

    /// Serializes the operation into the shared sidecar command envelope.
    pub fn to_sidecar_command(&self, request_id: &str) -> Value {
        BrowserRequestEnvelope::new(
            request_id,
            "teshi-engine",
            Duration::from_secs(60),
            self.clone(),
        )
        .to_sidecar_command()
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
    caller_label: String,
    project_root: Option<String>,
}

impl BrowserOperations {
    /// Creates a client for one local Teshi browser sidecar WebSocket.
    pub fn new(ws_url: impl Into<String>, timeout: Duration) -> Self {
        Self {
            ws_url: ws_url.into(),
            timeout,
            caller_label: "teshi-engine".into(),
            project_root: None,
        }
    }

    /// Set the non-secret caller label included in typed request envelopes.
    pub fn with_caller_label(mut self, caller_label: impl Into<String>) -> Self {
        self.caller_label = caller_label.into();
        self
    }

    /// Set request-scoped project context without making it broker ownership.
    pub fn with_project_root(mut self, project_root: impl Into<String>) -> Self {
        self.project_root = Some(project_root.into());
        self
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
        let mut envelope = BrowserRequestEnvelope::new(
            &request_id,
            &self.caller_label,
            self.timeout,
            operation.clone(),
        );
        if let Some(project_root) = &self.project_root {
            envelope = envelope.with_project_root(project_root);
        }
        let command = envelope.to_sidecar_command();
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
            recovery: sanitize_public_map(recovery),
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
        "stale_element_reference" => BrowserAgentErrorCode::StaleElementReference,
        "mismatched_browser_response" => BrowserAgentErrorCode::MismatchedBrowserResponse,
        "expired_browser_lease" => BrowserAgentErrorCode::ExpiredBrowserLease,
        "invalid_browser_lease" => BrowserAgentErrorCode::InvalidBrowserLease,
        "browser_target_not_found" => BrowserAgentErrorCode::BrowserTargetNotFound,
        "browser_operation_timeout" => BrowserAgentErrorCode::BrowserOperationTimeout,
        "invalid_browser_operation" => BrowserAgentErrorCode::InvalidBrowserOperation,
        "unsupported_browser_action" => BrowserAgentErrorCode::UnsupportedBrowserAction,
        "browser_capability_unavailable" => BrowserAgentErrorCode::BrowserCapabilityUnavailable,
        "browser_capability_denied" => BrowserAgentErrorCode::BrowserCapabilityDenied,
        "browser_wait_timeout" => BrowserAgentErrorCode::BrowserWaitTimeout,
        "browser_artifact_failure" | "browser_artifact_failed" => {
            BrowserAgentErrorCode::BrowserArtifactFailed
        }
        "duplicate_browser_mutation" => BrowserAgentErrorCode::DuplicateBrowserMutation,
        "browser_result_too_large" => BrowserAgentErrorCode::BrowserResultTooLarge,
        "browser_javascript_exception" => BrowserAgentErrorCode::BrowserJavascriptException,
        "browser_operation_failed" => BrowserAgentErrorCode::BrowserOperationFailed,
        _ => return None,
    })
}

fn sanitize_public_map(values: BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    match sanitize_public_value(Value::Object(values.into_iter().collect())) {
        Value::Object(object) => object.into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

fn sanitize_public_value(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .filter_map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase().replace('-', "_");
                    let secret = normalized == "lease_token"
                        || normalized == "capability_grant"
                        || normalized == "capability_grant_token"
                        || normalized.ends_with("_secret");
                    (!secret).then(|| (key, sanitize_public_value(value)))
                })
                .collect(),
        ),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(sanitize_public_value).collect())
        }
        other => other,
    }
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
        assert_eq!(command["caller_label"], "teshi-engine");
        assert_eq!(command["timeout_ms"], 60_000);
    }

    #[test]
    fn phased_feature_identifiers_are_typed_and_forward_compatible() {
        let known = BrowserFeatureId::new(BrowserFeatureId::P1_OBSERVABILITY_ARTIFACTS);
        let future = BrowserFeatureId::new("p3.future_feature");
        assert_eq!(known.phase(), Some(BrowserFeaturePhase::P1));
        assert_eq!(future.phase(), None);
        assert_eq!(
            serde_json::to_value(known).unwrap(),
            json!("p1.observability_artifacts")
        );
    }

    #[test]
    fn canonical_element_input_requires_exactly_one_strategy() {
        let none = BrowserElementInput::default();
        assert!(none.validate().is_err());
        let reference = BrowserElementInput {
            reference: Some("@e1".into()),
            ..Default::default()
        };
        assert!(reference.validate().is_ok());
        let ambiguous = BrowserElementInput {
            reference: Some("@e1".into()),
            css: Some("#save".into()),
            ..Default::default()
        };
        assert!(ambiguous.validate().is_err());
    }

    #[test]
    fn shared_request_envelope_preserves_context() {
        let envelope = BrowserRequestEnvelope::new(
            "request-envelope",
            "codex-a",
            Duration::from_millis(12_345),
            BrowserOperation::VerifyPlaywrightLocator {
                target: target(),
                lease_token: "lease-private".into(),
                candidate: PlaywrightLocatorCandidate {
                    kind: PlaywrightLocatorKind::Css,
                    arguments: BTreeMap::from([("selector".into(), json!("#save"))]),
                    expression: "page.locator('#save')".into(),
                    context: LocatorContext::default(),
                    match_count: 1,
                    visible: true,
                    enabled: true,
                    verification: LocatorVerificationStatus::Verified,
                    score: 50,
                    stability_rationale: "fixture".into(),
                    warnings: vec![],
                },
                page_context_revision: PageContextRevision("revision-a".into()),
            },
        );
        let command = envelope.to_sidecar_command();
        assert_eq!(command["request_id"], "request-envelope");
        assert_eq!(command["caller_label"], "codex-a");
        assert_eq!(command["timeout_ms"], 12_345);
        assert_eq!(command["target"]["tab_id"], 42);
        assert_eq!(command["lease_token"], "lease-private");
        assert_eq!(command["page_context_revision"], "revision-a");
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
        for code in [
            BrowserAgentErrorCode::StaleElementReference,
            BrowserAgentErrorCode::UnsupportedBrowserAction,
            BrowserAgentErrorCode::BrowserCapabilityUnavailable,
            BrowserAgentErrorCode::BrowserCapabilityDenied,
            BrowserAgentErrorCode::BrowserWaitTimeout,
            BrowserAgentErrorCode::BrowserArtifactFailed,
            BrowserAgentErrorCode::DuplicateBrowserMutation,
        ] {
            assert_eq!(parse_error_code(code.as_str()), Some(code));
        }
    }

    #[test]
    fn public_discovery_and_errors_do_not_serialize_secret_tokens() {
        let session = BrowserSession {
            schema_version: BROWSER_AGENT_SCHEMA_VERSION,
            identity: ExtensionIdentity {
                extension_instance_id: "profile-a".into(),
                profile_label: Some("agent-a".into()),
                extension_version: "0.7.9".into(),
                protocol_version: BROWSER_BROKER_PROTOCOL_VERSION,
            },
            browser: BrowserMetadata {
                name: "Chromium".into(),
                version: "140".into(),
                platform: Some("Windows".into()),
            },
            health: BrowserSessionHealth::Ready,
            last_heartbeat_age_ms: 10,
            windows: vec![],
            capabilities: BrowserCapabilities {
                features: vec![BrowserFeatureAvailability {
                    feature: BrowserFeatureId::new(BrowserFeatureId::P0_CONTROL),
                    available: false,
                    reason: Some("partial_control_surface".into()),
                }],
                supported_actions: vec![BrowserAction::Click],
                supported_operations: vec![],
                optional_permissions: BTreeMap::from([("cookies".into(), false)]),
            },
            lease: Some(BrowserLeaseSummary {
                owner_label: "agent-a".into(),
                expires_at_ms: 123,
            }),
        };
        let discovery = serde_json::to_string(&session).unwrap();
        assert!(!discovery.contains("lease_token"));
        assert!(!discovery.contains("capability_grant"));

        let error = BrowserAgentError {
            code: BrowserAgentErrorCode::BrowserCapabilityDenied,
            message: "grant required".into(),
            recovery: BTreeMap::from([
                ("lease_token".into(), json!("lease-private")),
                (
                    "nested".into(),
                    json!({"capability_grant_token": "grant-private", "retryable": false}),
                ),
            ]),
        };
        let wire = error.to_wire_value().to_string();
        assert!(!wire.contains("lease-private"));
        assert!(!wire.contains("grant-private"));
        assert!(wire.contains("retryable"));
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
