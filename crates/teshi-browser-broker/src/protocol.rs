//! Typed protocol-v1 records for the existing Chrome extension wire contract.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
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
/// Maximum serialized Network batch accepted by the broker state owner.
pub const MAX_NETWORK_BATCH_BYTES: usize = 4 * 1024 * 1024;
/// Maximum number of out-of-order Network sequence numbers retained per capture.
pub const MAX_NETWORK_PENDING_EVENTS: usize = 2_000;

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

/// Typed locator input accepted by the canonical P0 click operations.
///
/// The outer v1 operation envelope remains intentionally open for compatibility,
/// but the locator element itself is strict.  This keeps unsupported structured
/// candidates from reaching the extension while preserving the existing
/// `execute_browser_action` envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteLocatorElement {
    #[serde(default)]
    pub css: Option<String>,
    #[serde(default, alias = "testId")]
    pub test_id: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub page_context_revision: Option<String>,
}

/// Typed source DTO for the existing `execute_browser_action` operation.
///
/// Rust serializes the resolved value as the extension's existing
/// `execute_locator` command; it does not add a second execution channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteLocatorActionRequest {
    pub action: String,
    pub element: ExecuteLocatorElement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecuteLocatorInput {
    Css(String),
    TestId(String),
    RoleName { role: String, name: String },
    SnapshotReference(String),
}

/// Frame/shadow context retained by a Snapshot element reference.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocatorContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_root: Option<String>,
    /// Keep unknown context fields visible at the protocol boundary. They are
    /// rejected before dispatch instead of silently narrowing the search root.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl LocatorContext {
    pub fn validate_supported(&self) -> Result<(), BrokerError> {
        if self.extra.is_empty() {
            return Ok(());
        }
        let fields = self.extra.keys().cloned().collect::<Vec<_>>().join(", ");
        Err(BrokerError::new(
            BrokerErrorCode::BrowserCapabilityUnavailable,
            format!("unsupported locator context field(s): {fields}"),
        ))
    }
}

/// Candidate kinds understood by the existing Extension execute_locator and
/// verify_playwright_locators implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteLocatorCandidateKind {
    TestId,
    Role,
    Label,
    Placeholder,
    Attribute,
    Css,
    Text,
}

/// Stable argument fields carried by a structured locator.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocatorCandidateArguments {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribute: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A generated locator candidate plus the policy metadata used for ranking.
///
/// Metadata is optional on the wire so the existing direct execute_locator
/// shape remains valid. Rust-generated candidates populate all policy fields;
/// the Extension consumes only kind and arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecuteLocatorCandidate {
    pub kind: ExecuteLocatorCandidateKind,
    pub arguments: LocatorCandidateArguments,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<LocatorContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<LocatorVerificationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability_rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
}

pub type LocatorCandidate = ExecuteLocatorCandidate;
pub type LocatorCandidateKind = ExecuteLocatorCandidateKind;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocatorVerificationStatus {
    #[default]
    Unverified,
    Verified,
    NotFound,
    Ambiguous,
    NotActionable,
    StalePageContext,
}

/// One browser-observed verification result for a candidate expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocatorVerificationResult {
    #[serde(default)]
    pub expression: String,
    #[serde(default, deserialize_with = "deserialize_nonnegative_u32")]
    pub match_count: u32,
    #[serde(default)]
    pub visible: bool,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub stale_page_context: bool,
}

/// Structured caller intent used to select one normalized snapshot element.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocatorIntent {
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default, alias = "elementRef")]
    pub element_ref: Option<String>,
    #[serde(default, alias = "gherkinStep")]
    pub gherkin_step: Option<String>,
}

/// A normalized interactive element. Unknown snapshot fields are retained at
/// the protocol edge; policy code reads only the stable fields below.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotElement {
    pub element_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub accessible_name: Option<String>,
    #[serde(default, rename = "ariaLabel", skip_serializing_if = "Option::is_none")]
    pub aria_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(
        default,
        rename = "shortSelector",
        skip_serializing_if = "Option::is_none"
    )]
    pub short_selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<LocatorContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
    #[serde(skip)]
    context_error: Option<String>,
}

/// Normalized page snapshot used by locator policy and snapshot references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocatorSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_context_revision: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub interactive_elements: Vec<SnapshotElement>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocatorResolution {
    pub element: SnapshotElement,
    pub candidates: Vec<LocatorCandidate>,
}

impl LocatorSnapshot {
    pub fn normalize(value: &Value) -> Result<Self, BrokerError> {
        let object = value.as_object().ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "page snapshot must be a JSON object",
            )
        })?;
        let raw_elements = object
            .get("interactive_elements")
            .and_then(Value::as_array)
            .or_else(|| object.get("elements").and_then(Value::as_array));
        let interactive_elements = raw_elements
            .into_iter()
            .flat_map(|elements| elements.iter().enumerate())
            .filter_map(|(index, element)| SnapshotElement::normalize(index, element))
            .collect();
        let mut extra = object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        for key in [
            "snapshot_id",
            "page_context_revision",
            "url",
            "title",
            "interactive_elements",
            "elements",
        ] {
            extra.remove(key);
        }
        Ok(Self {
            snapshot_id: clean_optional_text(object.get("snapshot_id")),
            page_context_revision: clean_optional_text(object.get("page_context_revision")),
            url: clean_optional_text(object.get("url")).unwrap_or_default(),
            title: clean_optional_text(object.get("title")).unwrap_or_default(),
            interactive_elements,
            extra,
        })
    }

    /// Select the intended element and produce deterministic, policy-ranked
    /// candidates. This is pure; verification is merged separately below.
    pub fn generate_candidates(
        &self,
        intent: &LocatorIntent,
        test_id_attributes: &[String],
    ) -> Result<LocatorResolution, BrokerError> {
        if self.interactive_elements.is_empty() {
            return Err(locator_not_found(
                "page snapshot contains no interactive elements for locator acquisition",
            ));
        }
        let matching = self
            .interactive_elements
            .iter()
            .filter(|element| element.matches_explicit_intent(intent))
            .collect::<Vec<_>>();
        if matching.is_empty() {
            return Err(locator_not_found(
                "locator role, text, or element reference did not match an interactive element in the selected page",
            ));
        }
        let mut ranked_elements = matching
            .into_iter()
            .map(|element| (element.score_intent(intent), element))
            .collect::<Vec<_>>();
        ranked_elements.sort_by_key(|item| std::cmp::Reverse(item.0));
        let (best_score, element) = ranked_elements[0];
        if intent.has_explicit_fields() && best_score <= 0 {
            return Err(locator_not_found(
                "locator intent did not match an interactive element in the selected page",
            ));
        }

        let configured_attributes = if test_id_attributes.is_empty() {
            vec!["data-testid".to_owned()]
        } else {
            let values = test_id_attributes
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if values.is_empty() {
                vec!["data-testid".to_owned()]
            } else {
                values
            }
        };
        let context = element.context.clone().unwrap_or_default();
        element.validate_context()?;
        context.validate_supported()?;
        let role = element.effective_role();
        let accessible_name = element.accessible_name_for_locator();
        let mut candidates = Vec::new();
        if !role.is_empty()
            && let Some(name) = accessible_name.clone()
        {
            candidates.push(LocatorCandidate::role(
                role,
                name,
                context.clone(),
                100,
                "unique accessible role and name",
            ));
        }
        if let Some(label) = element.label.clone() {
            candidates.push(LocatorCandidate::textual(
                ExecuteLocatorCandidateKind::Label,
                "page.getByLabel",
                label,
                context.clone(),
                90,
                "associated form label",
                Vec::new(),
            ));
        }
        if let Some(placeholder) = element.placeholder.clone() {
            candidates.push(LocatorCandidate::textual(
                ExecuteLocatorCandidateKind::Placeholder,
                "page.getByPlaceholder",
                placeholder,
                context.clone(),
                85,
                "stable placeholder text",
                Vec::new(),
            ));
        }
        for attribute in configured_attributes {
            let Some(value) = element.attributes.get(&attribute).cloned() else {
                continue;
            };
            let (kind, expression, rationale) = if attribute == "data-testid" {
                (
                    ExecuteLocatorCandidateKind::TestId,
                    format!("page.getByTestId({})", js_string(&value)),
                    format!("project-configured test-id attribute {attribute}"),
                )
            } else {
                (
                    ExecuteLocatorCandidateKind::Attribute,
                    format!(
                        "page.locator({})",
                        js_string(&format!("[{attribute}={}]", json_string(&value)))
                    ),
                    format!("project-configured test-id attribute {attribute}"),
                )
            };
            candidates.push(LocatorCandidate::attribute(
                kind,
                attribute,
                value,
                expression,
                context.clone(),
                80,
                rationale,
                Vec::new(),
            ));
        }
        for attribute in ["id", "name", "aria-label", "title", "alt"] {
            let Some(value) = element.attributes.get(attribute).cloned() else {
                continue;
            };
            let selector = format!("[{attribute}={}]", json_string(&value));
            candidates.push(LocatorCandidate::attribute(
                ExecuteLocatorCandidateKind::Attribute,
                attribute.to_owned(),
                value,
                format!("page.locator({})", js_string(&selector)),
                context.clone(),
                if attribute == "id" { 70 } else { 65 },
                format!("stable {attribute} attribute fallback"),
                Vec::new(),
            ));
        }
        if let Some(selector) = element.short_selector.clone() {
            let warnings = selector_warnings(&selector);
            candidates.push(LocatorCandidate::css(
                selector,
                context.clone(),
                55 - 8 * warnings.len() as i32,
                "CSS fallback derived from the current DOM",
                warnings,
            ));
        }
        if let Some(text) = element.text.clone() {
            candidates.push(LocatorCandidate::textual(
                ExecuteLocatorCandidateKind::Text,
                "page.getByText",
                text,
                context,
                45,
                "visible text fallback may change with copy or localization",
                vec!["text_content_may_change".into()],
            ));
        }
        if candidates.is_empty() {
            return Err(locator_not_found(
                "the intended element has no supported stable locator attributes",
            ));
        }

        let mut deduplicated = Vec::new();
        for candidate in candidates {
            let expression = candidate.expression.clone().unwrap_or_default();
            if let Some(existing) =
                deduplicated
                    .iter_mut()
                    .find(|existing: &&mut LocatorCandidate| {
                        existing.expression.as_deref().unwrap_or_default() == expression
                    })
            {
                if candidate.score_value() > existing.score_value() {
                    *existing = candidate;
                }
            } else {
                deduplicated.push(candidate);
            }
        }
        deduplicated.sort_by_key(|candidate| std::cmp::Reverse(candidate.score_value()));
        Ok(LocatorResolution {
            element: element.clone(),
            candidates: deduplicated,
        })
    }
}

impl ExecuteLocatorCandidate {
    fn base(
        kind: ExecuteLocatorCandidateKind,
        arguments: LocatorCandidateArguments,
        expression: String,
        context: LocatorContext,
        score: i32,
        rationale: &str,
        warnings: Vec<String>,
    ) -> Self {
        Self {
            kind,
            arguments,
            expression: Some(expression),
            context: Some(context),
            match_count: Some(0),
            visible: Some(false),
            enabled: Some(false),
            verification: Some(LocatorVerificationStatus::Unverified),
            score: Some(score),
            stability_rationale: Some(rationale.into()),
            warnings: Some(warnings),
        }
    }

    fn role(
        role: String,
        name: String,
        context: LocatorContext,
        score: i32,
        rationale: &str,
    ) -> Self {
        Self::base(
            ExecuteLocatorCandidateKind::Role,
            LocatorCandidateArguments {
                role: Some(role.clone()),
                name: Some(name.clone()),
                exact: Some(true),
                ..Default::default()
            },
            format!(
                "page.getByRole({}, {{ name: {}, exact: true }})",
                js_string(&role),
                js_string(&name)
            ),
            context,
            score,
            rationale,
            Vec::new(),
        )
    }

    fn textual(
        kind: ExecuteLocatorCandidateKind,
        method: &str,
        text: String,
        context: LocatorContext,
        score: i32,
        rationale: &str,
        warnings: Vec<String>,
    ) -> Self {
        Self::base(
            kind,
            LocatorCandidateArguments {
                text: Some(text.clone()),
                exact: Some(true),
                ..Default::default()
            },
            format!("{method}({}, {{ exact: true }})", js_string(&text)),
            context,
            score,
            rationale,
            warnings,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn attribute(
        kind: ExecuteLocatorCandidateKind,
        attribute: String,
        value: String,
        expression: String,
        context: LocatorContext,
        score: i32,
        rationale: String,
        warnings: Vec<String>,
    ) -> Self {
        Self::base(
            kind,
            LocatorCandidateArguments {
                attribute: Some(attribute),
                value: Some(value),
                ..Default::default()
            },
            expression,
            context,
            score,
            &rationale,
            warnings,
        )
    }

    fn css(
        selector: String,
        context: LocatorContext,
        score: i32,
        rationale: &str,
        warnings: Vec<String>,
    ) -> Self {
        Self::base(
            ExecuteLocatorCandidateKind::Css,
            LocatorCandidateArguments {
                selector: Some(selector.clone()),
                ..Default::default()
            },
            format!("page.locator({})", js_string(&selector)),
            context,
            score,
            rationale,
            warnings,
        )
    }

    fn score_value(&self) -> i32 {
        self.score.unwrap_or_default()
    }

    pub fn validate_context(&self) -> Result<(), BrokerError> {
        self.context
            .as_ref()
            .map_or(Ok(()), LocatorContext::validate_supported)
    }

    fn is_verified(&self) -> bool {
        self.verification == Some(LocatorVerificationStatus::Verified)
    }

    /// Merge one Extension verification result using the Python status rules.
    pub fn with_verification(&self, result: Option<&LocatorVerificationResult>) -> Self {
        let mut updated = self.clone();
        let match_count = result.map(|value| value.match_count).unwrap_or_default();
        let visible = result.is_some_and(|value| value.visible);
        let enabled = result.is_some_and(|value| value.enabled);
        let status = if result.is_some_and(|value| value.stale_page_context) {
            LocatorVerificationStatus::StalePageContext
        } else if match_count == 0 {
            LocatorVerificationStatus::NotFound
        } else if match_count > 1 {
            LocatorVerificationStatus::Ambiguous
        } else if !visible || !enabled {
            LocatorVerificationStatus::NotActionable
        } else {
            LocatorVerificationStatus::Verified
        };
        updated.match_count = Some(match_count);
        updated.visible = Some(visible);
        updated.enabled = Some(enabled);
        updated.verification = Some(status);
        updated
    }
}

/// Merge Extension results and retain score ordering with verified candidates
/// first, exactly like the Python locator policy.
pub fn apply_locator_verification_results(
    candidates: &[LocatorCandidate],
    verification: &[LocatorVerificationResult],
) -> Vec<LocatorCandidate> {
    let mut merged = candidates
        .iter()
        .map(|candidate| {
            let expression = candidate.expression.as_deref().unwrap_or_default();
            let result = verification
                .iter()
                .rev()
                .find(|item| item.expression == expression);
            candidate.with_verification(result)
        })
        .collect::<Vec<_>>();
    merged.sort_by(|left, right| {
        right
            .is_verified()
            .cmp(&left.is_verified())
            .then_with(|| right.score_value().cmp(&left.score_value()))
    });
    merged
}

fn locator_not_found(message: &str) -> BrokerError {
    BrokerError::new(BrokerErrorCode::BrowserTargetNotFound, message)
}

fn clean_optional_text(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn normalize_attributes(object: &serde_json::Map<String, Value>) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    let raw = object
        .get("attributes")
        .and_then(Value::as_object)
        .filter(|attributes| !attributes.is_empty())
        .or_else(|| object.get("allAttributes").and_then(Value::as_object));
    if let Some(raw) = raw {
        for (name, value) in raw {
            if !value.is_string() && !value.is_number() && !value.is_boolean() {
                continue;
            }
            if let Some(text) = clean_optional_text(Some(value)) {
                attributes.insert(name.clone(), text.chars().take(500).collect());
            }
        }
    }
    for (source, destination) in [
        ("id", "id"),
        ("name", "name"),
        ("testId", "data-testid"),
        ("testid", "data-testid"),
        ("ariaLabel", "aria-label"),
        ("title", "title"),
        ("alt", "alt"),
    ] {
        if let Some(value) = clean_optional_text(object.get(source)) {
            attributes
                .entry(destination.into())
                .or_insert_with(|| value.chars().take(500).collect());
        }
    }
    attributes
}

fn intent_words(value: &str) -> std::collections::BTreeSet<String> {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_lowercase().collect::<String>()
            } else {
                " ".into()
            }
        })
        .collect::<String>();
    normalized
        .split_whitespace()
        .filter(|word| word.chars().count() >= 2)
        .map(str::to_owned)
        .collect()
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"\"".into())
        .replace("</", "<\\/")
}

fn js_string(value: &str) -> String {
    json_string(value)
}

fn selector_warnings(selector: &str) -> Vec<String> {
    let lowered = selector.to_lowercase();
    let mut warnings = Vec::new();
    if lowered.contains(":nth-") || lowered.contains(":first") || lowered.contains(":last") {
        warnings.push("positional_selector".into());
    }
    if selector.matches('>').count() >= 3 || selector.chars().count() > 160 {
        warnings.push("long_dom_path".into());
    }
    if ["sc-", "__", "css-", "emotion-", "_ngcontent"]
        .iter()
        .any(|marker| selector.contains(marker))
    {
        warnings.push("generated_class".into());
    }
    if ["x=", "y=", "coordinate"]
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        warnings.push("coordinate_selector".into());
    }
    warnings
}

fn deserialize_nonnegative_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let parsed = match value {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().map(|value| value.max(0) as u64))
            .or_else(|| number.as_f64().map(|value| value.max(0.0) as u64)),
        Value::String(value) => value.parse::<i64>().ok().map(|value| value.max(0) as u64),
        _ => Some(0),
    }
    .unwrap_or_default();
    u32::try_from(parsed).map_err(|_| D::Error::custom("match_count is too large"))
}

impl LocatorIntent {
    /// Parse the open operation argument without changing the v1 envelope.
    pub fn from_value(value: &Value) -> Self {
        let Some(object) = value.as_object() else {
            return Self::default();
        };
        Self {
            purpose: clean_optional_text(object.get("purpose")),
            text: clean_optional_text(object.get("text")),
            role: clean_optional_text(object.get("role")),
            element_ref: clean_optional_text(
                object
                    .get("element_ref")
                    .or_else(|| object.get("elementRef")),
            ),
            gherkin_step: clean_optional_text(
                object
                    .get("gherkin_step")
                    .or_else(|| object.get("gherkinStep")),
            ),
        }
    }

    fn has_explicit_fields(&self) -> bool {
        [
            self.purpose.as_deref(),
            self.text.as_deref(),
            self.role.as_deref(),
            self.element_ref.as_deref(),
            self.gherkin_step.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| !value.trim().is_empty())
    }
}

impl SnapshotElement {
    /// Match Python _normalize_element while retaining extension fields that
    /// are outside the typed locator policy.
    pub fn normalize(index: usize, value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let mut extra = object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        for key in [
            "element_ref",
            "tag",
            "role",
            "accessible_name",
            "ariaLabel",
            "label",
            "placeholder",
            "text",
            "attributes",
            "shortSelector",
            "context",
            "visible",
            "enabled",
        ] {
            extra.remove(key);
        }

        let element_ref =
            clean_optional_text(object.get("element_ref").or_else(|| object.get("ref")))
                .unwrap_or_else(|| format!("e{}", index + 1));
        let (context, context_error) = match object.get("context") {
            None | Some(Value::Null) => (None, None),
            Some(value) => match serde_json::from_value(value.clone()) {
                Ok(context) => (Some(context), None),
                Err(_) => (
                    None,
                    Some("snapshot element has malformed frame or shadow context".into()),
                ),
            },
        };
        Some(Self {
            element_ref,
            tag: clean_optional_text(object.get("tag")),
            role: clean_optional_text(object.get("role")),
            accessible_name: clean_optional_text(
                object
                    .get("accessible_name")
                    .or_else(|| object.get("computedAccessibleName")),
            ),
            aria_label: clean_optional_text(object.get("ariaLabel")),
            label: clean_optional_text(object.get("label")),
            placeholder: clean_optional_text(object.get("placeholder")),
            text: clean_optional_text(object.get("text")),
            attributes: normalize_attributes(object),
            short_selector: clean_optional_text(
                object
                    .get("shortSelector")
                    .or_else(|| object.get("short_selector")),
            ),
            context,
            visible: object.get("visible").and_then(Value::as_bool),
            enabled: object.get("enabled").and_then(Value::as_bool),
            extra,
            context_error,
        })
    }

    pub fn validate_context(&self) -> Result<(), BrokerError> {
        if let Some(message) = &self.context_error {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserCapabilityUnavailable,
                message,
            ));
        }
        self.context
            .as_ref()
            .map_or(Ok(()), LocatorContext::validate_supported)
    }

    pub fn implicit_role(&self) -> String {
        let tag = self.tag.as_deref().unwrap_or_default().to_ascii_lowercase();
        let input_type = self
            .attributes
            .get("type")
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        match tag.as_str() {
            "button" => "button".into(),
            "input" if matches!(input_type.as_str(), "button" | "submit") => "button".into(),
            "a" => "link".into(),
            "textarea" => "textbox".into(),
            "input" if input_type == "checkbox" => "checkbox".into(),
            "input" if input_type == "radio" => "radio".into(),
            "input" => "textbox".into(),
            "select" => "combobox".into(),
            _ => String::new(),
        }
    }

    fn effective_role(&self) -> String {
        self.role.clone().unwrap_or_else(|| self.implicit_role())
    }

    fn accessible_name_for_locator(&self) -> Option<String> {
        self.accessible_name
            .clone()
            .or_else(|| self.aria_label.clone())
            .or_else(|| self.label.clone())
            .or_else(|| self.text.clone())
    }

    fn intent_haystack(&self) -> String {
        [
            self.accessible_name.as_deref().unwrap_or_default(),
            self.aria_label.as_deref().unwrap_or_default(),
            self.label.as_deref().unwrap_or_default(),
            self.placeholder.as_deref().unwrap_or_default(),
            self.text.as_deref().unwrap_or_default(),
        ]
        .join(" ")
    }

    fn matches_explicit_intent(&self, intent: &LocatorIntent) -> bool {
        if let Some(expected) = intent.element_ref.as_deref()
            && self.element_ref != expected
        {
            return false;
        }
        if let Some(expected) = intent.role.as_deref()
            && self.effective_role().to_lowercase() != expected.to_lowercase()
        {
            return false;
        }
        if let Some(expected) = intent.text.as_deref() {
            let expected = expected.to_lowercase();
            if ![
                self.accessible_name.as_deref(),
                self.aria_label.as_deref(),
                self.label.as_deref(),
                self.placeholder.as_deref(),
                self.text.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|value| value.to_lowercase().contains(&expected))
            {
                return false;
            }
        }
        true
    }

    pub fn score_intent(&self, intent: &LocatorIntent) -> i32 {
        if let Some(expected) = intent.element_ref.as_deref() {
            return if self.element_ref == expected {
                10_000
            } else {
                -10_000
            };
        }
        let mut score = 0;
        if let Some(expected) = intent.role.as_deref() {
            score += if self.effective_role().to_lowercase() == expected.to_lowercase() {
                120
            } else {
                -60
            };
        }
        let haystack = self.intent_haystack();
        if let Some(expected) = intent.text.as_deref() {
            let expected = expected.to_lowercase();
            let haystack_lower = haystack.to_lowercase();
            score += if expected == haystack_lower.trim() {
                160
            } else if haystack_lower.contains(&expected) {
                100
            } else {
                -50
            };
        }
        let context_words = intent_words(
            &[
                intent.purpose.as_deref().unwrap_or_default(),
                intent.gherkin_step.as_deref().unwrap_or_default(),
            ]
            .join(" "),
        );
        if !context_words.is_empty() {
            let element_words = intent_words(&format!("{} {}", self.effective_role(), haystack));
            score += 8 * context_words.intersection(&element_words).count() as i32;
        }
        if self.visible == Some(false) {
            score -= 80;
        }
        score
    }
}

/// Resolved command sent over the existing Extension `execute_locator` path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecuteLocatorCommand {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<ExecuteLocatorCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator_context: Option<LocatorContext>,
    pub action: String,
    pub page_context_revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
}

impl ExecuteLocatorActionRequest {
    /// Parse only the operation-specific fields from the compatibility envelope.
    pub fn from_operation(request: &OperationRequest) -> Result<Self, BrokerError> {
        let action = request.arguments.get("action").cloned().ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "execute_browser_action requires an action",
            )
        })?;
        let raw_element = request.arguments.get("element").cloned().ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "execute_browser_action requires an element object",
            )
        })?;
        if raw_element
            .as_object()
            .is_some_and(|element| element.contains_key("candidate"))
        {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserCapabilityUnavailable,
                "Rust p0.control does not support structured candidate parameters",
            ));
        }
        let element = serde_json::from_value(raw_element).map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "execute_browser_action contains an unsupported locator parameter",
            )
        })?;
        let action = action.as_str().map(str::to_owned).ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "execute_browser_action requires a string action",
            )
        })?;
        Ok(Self { action, element })
    }

    pub fn normalized_action(&self) -> Result<String, BrokerError> {
        normalize_locator_text(&self.action, "action")
    }

    pub fn input(&self) -> Result<ExecuteLocatorInput, BrokerError> {
        let css = normalize_optional_locator_text(self.element.css.as_deref(), "element.css")?;
        let test_id =
            normalize_optional_locator_text(self.element.test_id.as_deref(), "element.test_id")?;
        let role = normalize_optional_locator_text(self.element.role.as_deref(), "element.role")?;
        let name = normalize_optional_locator_text(self.element.name.as_deref(), "element.name")?;
        let reference = normalize_optional_locator_text(
            self.element.reference.as_deref(),
            "element.reference",
        )?;

        if role.is_some() != name.is_some() {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "element.role and element.name must be supplied together",
            ));
        }
        let locator_count = usize::from(css.is_some())
            + usize::from(test_id.is_some())
            + usize::from(role.is_some() && name.is_some())
            + usize::from(reference.is_some());
        if locator_count != 1 {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "exactly one of element.css, element.test_id, element.role/name, or element.reference is required",
            ));
        }
        if let Some(reference) = reference {
            if !is_snapshot_reference(&reference) {
                return Err(BrokerError::new(
                    BrokerErrorCode::InvalidBrowserOperation,
                    "element.reference must be a snapshot alias such as @e1",
                ));
            }
            return Ok(ExecuteLocatorInput::SnapshotReference(reference));
        }
        if let Some(css) = css {
            return Ok(ExecuteLocatorInput::Css(css));
        }
        if let Some(test_id) = test_id {
            return Ok(ExecuteLocatorInput::TestId(test_id));
        }
        Ok(ExecuteLocatorInput::RoleName {
            role: role.expect("role/name count was validated"),
            name: name.expect("role/name count was validated"),
        })
    }

    pub fn page_context_revision(&self) -> Result<String, BrokerError> {
        normalize_locator_text(
            self.element
                .page_context_revision
                .as_deref()
                .unwrap_or_default(),
            "element.page_context_revision",
        )
        .map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::StaleBrowserTarget,
                "execute_browser_action requires the page_context_revision from a current snapshot",
            )
        })
    }

    pub fn snapshot_id(&self) -> Result<Option<String>, BrokerError> {
        normalize_optional_locator_text(self.element.snapshot_id.as_deref(), "element.snapshot_id")
    }
}

fn normalize_locator_text(value: &str, field: &str) -> Result<String, BrokerError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 4096 {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            format!("{field} must contain 1 to 4096 bytes"),
        ));
    }
    Ok(value.to_owned())
}

fn normalize_optional_locator_text(
    value: Option<&str>,
    field: &str,
) -> Result<Option<String>, BrokerError> {
    value
        .map(|value| normalize_locator_text(value, field))
        .transpose()
}

fn is_snapshot_reference(value: &str) -> bool {
    value
        .strip_prefix("@e")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
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

    /// Decode the existing Extension verification array without changing its
    /// response envelope. Missing/non-array data remains an empty result set,
    /// matching the Python policy's conservative default.
    pub fn locator_verification_results(
        &self,
    ) -> Result<Vec<LocatorVerificationResult>, BrokerError> {
        let Some(raw) = self.result.get("verification") else {
            return Ok(Vec::new());
        };
        let Some(items) = raw.as_array() else {
            return Ok(Vec::new());
        };
        items
            .iter()
            .map(|item| {
                serde_json::from_value(item.clone()).map_err(|_| {
                    BrokerError::new(
                        BrokerErrorCode::BrokerProtocolError,
                        "extension locator verification result is malformed",
                    )
                })
            })
            .collect()
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
        if self.capture_id.trim().is_empty() || self.capture_id.len() > 256 {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network batch capture_id is invalid",
            ));
        }
        if self.target.window_id <= 0 || self.target.tab_id <= 0 {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "network batch target is invalid",
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
        if self.events.iter().any(|event| event.seq == 0) {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network event sequence must be positive",
            ));
        }
        if let Some(first_seq) = self.first_seq
            && self.events.first().map(|event| event.seq) != Some(first_seq)
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network batch first_seq does not match its events",
            ));
        }
        if let Some(last_seq) = self.last_seq
            && self.events.last().map(|event| event.seq) != Some(last_seq)
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "network batch last_seq does not match its events",
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
    ElementNotFound,
    MismatchedBrowserResponse,
    ExpiredBrowserLease,
    InvalidBrowserLease,
    BrowserTargetNotFound,
    BrowserOperationTimeout,
    BrowserWaitTimeout,
    BrowserOperationFailed,
    BrowserExecutionUnknown,
    BrowserOperationCancelled,
    BrowserRequestNotFound,
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
            Self::ElementNotFound => "element_not_found",
            Self::MismatchedBrowserResponse => "mismatched_browser_response",
            Self::ExpiredBrowserLease => "expired_browser_lease",
            Self::InvalidBrowserLease => "invalid_browser_lease",
            Self::BrowserTargetNotFound => "browser_target_not_found",
            Self::BrowserOperationTimeout => "browser_operation_timeout",
            Self::BrowserWaitTimeout => "browser_wait_timeout",
            Self::BrowserOperationFailed => "browser_operation_failed",
            Self::BrowserExecutionUnknown => "browser_execution_unknown",
            Self::BrowserOperationCancelled => "browser_operation_cancelled",
            Self::BrowserRequestNotFound => "browser_request_not_found",
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
            | "cancel_browser_request"
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

    #[test]
    fn snapshot_normalization_accepts_elements_aliases_and_preserves_context() {
        let snapshot = LocatorSnapshot::normalize(&json!({
            "snapshot_id": "snapshot-1",
            "page_context_revision": "revision-1",
            "elements": [{
                "ref": "save-button",
                "tag": "button",
                "computedAccessibleName": " Save ",
                "testId": "save",
                "allAttributes": {"class": "css-123", "disabled": false},
                "shortSelector": "button.css-123:nth-of-type(2)",
                "context": {"frame": "checkout-frame", "shadow_root": null},
                "unknown_extension_field": {"kept": true}
            }]
        }))
        .unwrap();
        let element = &snapshot.interactive_elements[0];
        assert_eq!(element.element_ref, "save-button");
        assert_eq!(element.accessible_name.as_deref(), Some("Save"));
        assert_eq!(element.attributes["data-testid"], "save");
        assert_eq!(element.attributes["class"], "css-123");
        assert_eq!(
            element
                .context
                .as_ref()
                .and_then(|context| context.frame.as_deref()),
            Some("checkout-frame")
        );
        assert_eq!(element.extra["unknown_extension_field"]["kept"], true);
    }

    #[test]
    fn candidate_generation_matches_python_order_and_structured_intent_errors() {
        let snapshot = LocatorSnapshot::normalize(&json!({
            "interactive_elements": [
                {
                    "element_ref": "save-button",
                    "tag": "button",
                    "role": "button",
                    "accessible_name": "Save",
                    "text": "Save",
                    "attributes": {"data-testid": "save", "class": "css-123"},
                    "shortSelector": "button.css-123:nth-of-type(2)",
                    "visible": true
                },
                {
                    "element_ref": "email-input",
                    "tag": "input",
                    "label": "Email",
                    "placeholder": "name@example.test",
                    "attributes": {"data-qa": "email-field", "name": "email"},
                    "context": {"frame": "checkout-frame", "shadow_root": null},
                    "visible": true
                }
            ]
        }))
        .unwrap();
        let save = snapshot
            .generate_candidates(
                &LocatorIntent {
                    element_ref: Some("save-button".into()),
                    ..Default::default()
                },
                &[],
            )
            .unwrap();
        assert_eq!(save.candidates[0].kind, ExecuteLocatorCandidateKind::Role);
        assert_eq!(
            save.candidates[0].expression.as_deref(),
            Some("page.getByRole(\"button\", { name: \"Save\", exact: true })")
        );
        let css = save
            .candidates
            .iter()
            .find(|candidate| candidate.kind == ExecuteLocatorCandidateKind::Css)
            .unwrap();
        assert!(
            css.warnings
                .as_ref()
                .unwrap()
                .contains(&"generated_class".into())
        );
        assert!(
            css.warnings
                .as_ref()
                .unwrap()
                .contains(&"positional_selector".into())
        );

        let email = snapshot
            .generate_candidates(
                &LocatorIntent {
                    element_ref: Some("email-input".into()),
                    ..Default::default()
                },
                &["data-qa".into()],
            )
            .unwrap();
        assert_eq!(
            email
                .candidates
                .iter()
                .take(4)
                .map(|candidate| candidate.kind)
                .collect::<Vec<_>>(),
            vec![
                ExecuteLocatorCandidateKind::Role,
                ExecuteLocatorCandidateKind::Label,
                ExecuteLocatorCandidateKind::Placeholder,
                ExecuteLocatorCandidateKind::Attribute
            ]
        );
        assert_eq!(
            email.candidates[0]
                .context
                .as_ref()
                .and_then(|context| context.frame.as_deref()),
            Some("checkout-frame")
        );

        let error = snapshot
            .generate_candidates(
                &LocatorIntent {
                    role: Some("link".into()),
                    text: Some("Save".into()),
                    ..Default::default()
                },
                &[],
            )
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserTargetNotFound);
    }

    #[test]
    fn shared_locator_semantics_fixture_matches_python_policy() {
        let fixture = fixtures()["migration_contracts"]["locator_semantics"].clone();
        let snapshot = LocatorSnapshot::normalize(&fixture["snapshot"]).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let intent = LocatorIntent::from_value(&case["intent"]);
            let test_ids = case["test_id_attributes"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let resolution = snapshot.generate_candidates(&intent, &test_ids).unwrap();
            assert_eq!(
                resolution.element.element_ref,
                case["expected_element_ref"].as_str().unwrap()
            );
            let expected = case["expected_candidates"].as_array().unwrap();
            assert_eq!(resolution.candidates.len(), expected.len());
            for (candidate, expected) in resolution.candidates.iter().zip(expected) {
                let expected_kind: ExecuteLocatorCandidateKind =
                    serde_json::from_value(expected["kind"].clone()).unwrap();
                assert_eq!(candidate.kind, expected_kind);
                assert_eq!(
                    candidate.expression,
                    expected["expression"].as_str().map(str::to_owned)
                );
                assert_eq!(
                    candidate.score,
                    expected["score"].as_i64().map(|value| value as i32)
                );
            }
            if let Some(context) = case.get("expected_context") {
                let actual = resolution.candidates[0].context.as_ref().unwrap();
                assert_eq!(actual.frame, context["frame"].as_str().map(str::to_owned));
                assert_eq!(
                    actual.shadow_root,
                    context["shadow_root"].as_str().map(str::to_owned)
                );
            }
        }

        for case in fixture["error_cases"].as_array().unwrap() {
            let case_snapshot = case.get("snapshot").unwrap_or(&fixture["snapshot"]);
            let case_snapshot = LocatorSnapshot::normalize(case_snapshot).unwrap();
            let error = case_snapshot
                .generate_candidates(&LocatorIntent::from_value(&case["intent"]), &[])
                .unwrap_err();
            assert_eq!(
                error.code.as_str(),
                case["expected_error"].as_str().unwrap()
            );
        }
    }

    #[test]
    fn verification_results_merge_to_stable_structured_statuses() {
        let candidate = ExecuteLocatorCandidate {
            kind: ExecuteLocatorCandidateKind::Role,
            arguments: LocatorCandidateArguments {
                role: Some("button".into()),
                name: Some("Save".into()),
                exact: Some(true),
                ..Default::default()
            },
            expression: Some("page.getByRole(\"button\")".into()),
            context: None,
            match_count: Some(0),
            visible: Some(false),
            enabled: Some(false),
            verification: Some(LocatorVerificationStatus::Unverified),
            score: Some(100),
            stability_rationale: Some("role".into()),
            warnings: Some(Vec::new()),
        };
        let verification = serde_json::from_value::<LocatorVerificationResult>(json!({
            "expression": "page.getByRole(\"button\")",
            "match_count": 1,
            "visible": true,
            "enabled": true
        }))
        .unwrap();
        let merged = apply_locator_verification_results(&[candidate], &[verification]);
        assert_eq!(
            merged[0].verification,
            Some(LocatorVerificationStatus::Verified)
        );
        assert_eq!(merged[0].match_count, Some(1));

        let stale = serde_json::from_value::<LocatorVerificationResult>(json!({
            "expression": "page.getByRole(\"button\")",
            "stale_page_context": true
        }))
        .unwrap();
        assert_eq!(
            apply_locator_verification_results(&merged, &[stale])[0].verification,
            Some(LocatorVerificationStatus::StalePageContext)
        );

        let response: ExtensionResponse = serde_json::from_value(json!({
            "type": "response",
            "request_id": "verify-1",
            "cmd": "verify_playwright_locators",
            "ok": true,
            "verification": [{
                "expression": "page.getByRole(\"button\")",
                "match_count": 2,
                "visible": true,
                "enabled": true
            }]
        }))
        .unwrap();
        assert_eq!(
            response.locator_verification_results().unwrap()[0].match_count,
            2
        );
    }
}
