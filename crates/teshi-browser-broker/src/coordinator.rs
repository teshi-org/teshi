//! Coordination for revision-bound browser actions.
//!
//! Locator generation and ranking stay in the protocol/session boundary. This
//! module consumes the already-resolved `ExecuteLocatorCommand` and owns the
//! common command, wait, response, and uncertain-outcome rules for action
//! requests.

use serde_json::{Map, Value};

use crate::protocol::{
    BROWSER_BROKER_PROTOCOL_VERSION, BROWSER_BROKER_SCHEMA_VERSION, BrokerError, BrokerErrorCode,
    BrowserTarget, ExecuteLocatorCommand, ExtensionResponse, OperationRequest,
};

/// Metadata retained while one action is pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActionMetadata {
    pub action: String,
    pub locator: ExecuteLocatorCommand,
    pub may_have_side_effect: bool,
}

/// One fully validated command sent through the existing `execute_locator` path.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActionPlan {
    pub command: Value,
    pub metadata: ActionMetadata,
}

/// Stateless coordinator for the revision-bound action path.
pub(crate) struct BrowserActionCoordinator;

impl BrowserActionCoordinator {
    /// Build one correlated extension command from an existing resolved locator.
    ///
    /// Resolution is deliberately supplied by the caller. In particular, the
    /// 4.2 locator policy remains the owner of candidate generation, ranking,
    /// and snapshot semantics; this method only coordinates its execution.
    pub(crate) fn plan(
        request: &OperationRequest,
        target: &BrowserTarget,
        extension_instance_id: &str,
        locator: &ExecuteLocatorCommand,
    ) -> Result<ActionPlan, BrokerError> {
        if request.operation != "execute_browser_action" {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "the action coordinator only accepts execute_browser_action",
            ));
        }
        let command = Self::build_command(request, target, extension_instance_id, locator)?;
        let action = locator.action.clone();
        Ok(ActionPlan {
            command,
            metadata: ActionMetadata {
                action: action.clone(),
                locator: locator.clone(),
                may_have_side_effect: Self::may_have_side_effect(&action),
            },
        })
    }

    /// Serialize a resolved action through the existing `execute_locator` wire path.
    pub(crate) fn build_command(
        request: &OperationRequest,
        target: &BrowserTarget,
        extension_instance_id: &str,
        locator: &ExecuteLocatorCommand,
    ) -> Result<Value, BrokerError> {
        let wait = normalize_wait(request.arguments.get("wait"), locator)?;
        let mut object = request
            .arguments
            .clone()
            .into_iter()
            .collect::<Map<String, Value>>();
        object.insert("type".into(), Value::String("cmd".into()));
        object.insert("cmd".into(), Value::String("execute_locator".into()));
        object.insert(
            "schema_version".into(),
            Value::from(BROWSER_BROKER_SCHEMA_VERSION),
        );
        object.insert(
            "protocol_version".into(),
            Value::from(BROWSER_BROKER_PROTOCOL_VERSION),
        );
        object.insert(
            "request_id".into(),
            Value::String(request.request_id.clone()),
        );
        object.insert(
            "caller_label".into(),
            Value::String(normalized_caller_label(request)),
        );
        if let Some(project_root) = request.project_root.as_deref() {
            object.insert(
                "project_root".into(),
                Value::String(project_root.to_owned()),
            );
        }
        object.insert(
            "extension_instance_id".into(),
            Value::String(extension_instance_id.to_owned()),
        );
        object.insert(
            "target".into(),
            serde_json::to_value(target).map_err(|_| {
                BrokerError::new(
                    BrokerErrorCode::InvalidBrowserOperation,
                    "browser target could not be serialized",
                )
            })?,
        );
        object.remove("lease_token");
        object.remove("element");
        object.insert("action".into(), Value::String(locator.action.clone()));
        if let Some(selector) = &locator.selector {
            object.insert("selector".into(), Value::String(selector.clone()));
        } else {
            object.remove("selector");
        }
        if let Some(candidate) = &locator.candidate {
            object.insert(
                "candidate".into(),
                serde_json::to_value(candidate).map_err(|_| {
                    BrokerError::new(
                        BrokerErrorCode::InvalidBrowserOperation,
                        "browser locator candidate could not be serialized",
                    )
                })?,
            );
        } else {
            object.remove("candidate");
        }
        if let Some(context) = &locator.locator_context {
            object.insert(
                "locator_context".into(),
                serde_json::to_value(context).map_err(|_| {
                    BrokerError::new(
                        BrokerErrorCode::InvalidBrowserOperation,
                        "browser locator context could not be serialized",
                    )
                })?,
            );
        } else {
            object.remove("locator_context");
        }
        object.insert(
            "page_context_revision".into(),
            Value::String(locator.page_context_revision.clone()),
        );
        if let Some(snapshot_id) = &locator.snapshot_id {
            object.insert("snapshot_id".into(), Value::String(snapshot_id.clone()));
        } else {
            object.remove("snapshot_id");
        }
        match wait {
            Some(wait) => {
                object.insert("wait".into(), wait);
            }
            None => {
                object.remove("wait");
            }
        }
        Ok(Value::Object(object))
    }

    /// Whether a lost transport result can represent an applied side effect.
    pub(crate) fn may_have_side_effect(action: &str) -> bool {
        matches!(
            action,
            "click" | "pointer_click" | "fill" | "type" | "select" | "press_key" | "upload"
        )
    }

    /// Add structured action metadata and map extension failures to broker errors.
    pub(crate) fn finalize_response(
        value: &mut Value,
        response: &ExtensionResponse,
        request_id: &str,
        target: &BrowserTarget,
        metadata: &ActionMetadata,
    ) -> Result<(), BrokerError> {
        if let Value::Object(object) = value {
            object.insert("action".into(), Value::String(metadata.action.clone()));
            object.insert(
                "outcome".into(),
                Value::String(if response.ok { "completed" } else { "failed" }.into()),
            );
            object.insert(
                "page_context_revision".into(),
                Value::String(metadata.locator.page_context_revision.clone()),
            );
            object.insert("locator".into(), locator_summary(&metadata.locator));
        }

        let wait_failed = response.ok
            && value.pointer("/wait_outcome/ok").and_then(Value::as_bool) == Some(false);
        if wait_failed {
            let mut error = action_error(
                BrokerErrorCode::BrowserWaitTimeout,
                "browser action completed but its post-action wait timed out",
                value,
                request_id,
                target,
                metadata,
            );
            error
                .recovery
                .insert("action_executed".into(), Value::Bool(true));
            error.recovery.insert(
                "retry".into(),
                Value::String(
                    "do not retry the action automatically; reconcile the page state first".into(),
                ),
            );
            return Err(error);
        }
        if !response.ok {
            let extension_code = value
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("browser_operation_failed");
            let code = match extension_code {
                "stale_page_context" => BrokerErrorCode::StaleBrowserTarget,
                "stale_element_reference" => BrokerErrorCode::StaleElementReference,
                "element_not_found" => BrokerErrorCode::ElementNotFound,
                "browser_wait_timeout" => BrokerErrorCode::BrowserWaitTimeout,
                "invalid_selector" | "missing_value" => BrokerErrorCode::InvalidBrowserOperation,
                "unsupported_action" => BrokerErrorCode::BrowserCapabilityUnavailable,
                _ => BrokerErrorCode::BrowserOperationFailed,
            };
            return Err(action_error(
                code,
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("browser action failed"),
                value,
                request_id,
                target,
                metadata,
            ));
        }
        Ok(())
    }

    /// Express a terminal transport failure whose action outcome is unknown.
    pub(crate) fn unknown_execution_error(
        request_id: &str,
        target: &BrowserTarget,
        extension_instance_id: &str,
        terminal: &BrokerError,
        metadata: &ActionMetadata,
    ) -> BrokerError {
        let mut error = BrokerError::new(
            BrokerErrorCode::BrowserExecutionUnknown,
            "browser action execution outcome is unknown after transport interruption; do not retry automatically",
        );
        error
            .recovery
            .insert("request_id".into(), Value::String(request_id.into()));
        error.recovery.insert(
            "extension_instance_id".into(),
            Value::String(extension_instance_id.into()),
        );
        error.recovery.insert(
            "target".into(),
            serde_json::to_value(target).unwrap_or(Value::Null),
        );
        error
            .recovery
            .insert("action".into(), Value::String(metadata.action.clone()));
        error.recovery.insert(
            "page_context_revision".into(),
            Value::String(metadata.locator.page_context_revision.clone()),
        );
        error
            .recovery
            .insert("outcome".into(), Value::String("unknown".into()));
        error
            .recovery
            .insert("cause".into(), Value::String(terminal.code.as_str().into()));
        error.recovery.insert(
            "retry".into(),
            Value::String("do_not_retry_automatically".into()),
        );
        error.recovery.insert(
            "reconcile".into(),
            Value::String(
                "request a fresh snapshot and verify application state before any new mutation"
                    .into(),
            ),
        );
        error
    }
}

fn action_error(
    code: BrokerErrorCode,
    message: &str,
    value: &Value,
    request_id: &str,
    target: &BrowserTarget,
    metadata: &ActionMetadata,
) -> BrokerError {
    let mut error = BrokerError::new(code, message);
    error
        .recovery
        .insert("request_id".into(), Value::String(request_id.into()));
    error.recovery.insert(
        "target".into(),
        serde_json::to_value(target).unwrap_or(Value::Null),
    );
    error
        .recovery
        .insert("action".into(), Value::String(metadata.action.clone()));
    error.recovery.insert(
        "page_context_revision".into(),
        Value::String(metadata.locator.page_context_revision.clone()),
    );
    if let Some(code) = value.get("code").and_then(Value::as_str) {
        error
            .recovery
            .insert("extension_code".into(), Value::String(code.into()));
    }
    if let Some(action_outcome) = value.get("action_outcome") {
        error
            .recovery
            .insert("action_outcome".into(), action_outcome.clone());
    }
    if let Some(wait_outcome) = value.get("wait_outcome") {
        error
            .recovery
            .insert("wait_outcome".into(), wait_outcome.clone());
    }
    error.recovery.insert(
        "retry".into(),
        Value::String("do_not_retry_automatically".into()),
    );
    error
}

fn locator_summary(locator: &ExecuteLocatorCommand) -> Value {
    let mut summary = Map::new();
    if let Some(selector) = &locator.selector {
        summary.insert("selector".into(), Value::String(selector.clone()));
    }
    if let Some(candidate) = &locator.candidate {
        summary.insert(
            "candidate".into(),
            serde_json::to_value(candidate).unwrap_or(Value::Null),
        );
    }
    if let Some(snapshot_id) = &locator.snapshot_id {
        summary.insert("snapshot_id".into(), Value::String(snapshot_id.clone()));
    }
    summary.insert(
        "page_context_revision".into(),
        Value::String(locator.page_context_revision.clone()),
    );
    if let Some(context) = &locator.locator_context {
        summary.insert(
            "context".into(),
            serde_json::to_value(context).unwrap_or(Value::Null),
        );
    }
    Value::Object(summary)
}

fn normalize_wait(
    raw: Option<&Value>,
    locator: &ExecuteLocatorCommand,
) -> Result<Option<Value>, BrokerError> {
    let Some(raw) = raw.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let object = raw.as_object().ok_or_else(|| {
        BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "wait must be a typed browser wait object",
        )
    })?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "wait.kind is required",
            )
        })?;
    let mut wait = object.clone();
    match kind {
        "url" => require_wait_text(object, "pattern")?,
        "visible_text" => require_wait_text(object, "text")?,
        "page_revision_change" => require_wait_text(object, "from")?,
        "load_complete" => {}
        "element_state" => {
            let state = object
                .get("state")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| matches!(*value, "visible" | "hidden" | "enabled" | "disabled"))
                .ok_or_else(|| {
                    BrokerError::new(
                        BrokerErrorCode::InvalidBrowserOperation,
                        "wait.element_state requires visible, hidden, enabled, or disabled",
                    )
                })?;
            if !object.get("element").is_some_and(Value::is_object) {
                return Err(BrokerError::new(
                    BrokerErrorCode::InvalidBrowserOperation,
                    "wait.element_state requires an element locator object",
                ));
            }
            wait.insert("element".into(), locator_wait_element(locator));
            wait.insert("state".into(), Value::String(state.into()));
        }
        _ => {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                format!("unsupported browser wait condition: {kind}"),
            ));
        }
    }
    Ok(Some(Value::Object(wait)))
}

fn locator_wait_element(locator: &ExecuteLocatorCommand) -> Value {
    let mut element = Map::new();
    if let Some(selector) = &locator.selector {
        element.insert("css".into(), Value::String(selector.clone()));
    }
    if let Some(candidate) = &locator.candidate {
        element.insert(
            "candidate".into(),
            serde_json::to_value(candidate).unwrap_or(Value::Null),
        );
    }
    element.insert(
        "page_context_revision".into(),
        Value::String(locator.page_context_revision.clone()),
    );
    if let Some(snapshot_id) = &locator.snapshot_id {
        element.insert("snapshot_id".into(), Value::String(snapshot_id.clone()));
    }
    Value::Object(element)
}

fn require_wait_text(object: &Map<String, Value>, key: &str) -> Result<(), BrokerError> {
    let valid = object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty() && value.len() <= 4096);
    if valid {
        Ok(())
    } else {
        Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            format!("wait.{key} must contain 1 to 4096 bytes"),
        ))
    }
}

fn normalized_caller_label(request: &OperationRequest) -> String {
    let caller: String = request.caller_label.trim().chars().take(120).collect();
    if caller.is_empty() {
        "legacy-client".into()
    } else {
        caller
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::protocol::{
        BROWSER_BROKER_PROTOCOL_VERSION, BROWSER_BROKER_SCHEMA_VERSION, ExecuteLocatorCandidate,
        ExecuteLocatorCandidateKind, LocatorCandidateArguments,
    };

    fn target() -> BrowserTarget {
        BrowserTarget {
            extension_instance_id: "profile-a".into(),
            window_id: 7,
            tab_id: 42,
        }
    }

    fn request(wait: Option<Value>) -> OperationRequest {
        let mut value = json!({
            "schema_version": 1,
            "request_id": "action-1",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "execute_browser_action",
            "target": target(),
            "lease_token": "test-lease-token",
            "action": "pointer_click",
            "element": {
                "css": "#save",
                "page_context_revision": "revision-1"
            },
            "timeout_ms": 5000
        });
        if let Some(wait) = wait {
            value["wait"] = wait;
        }
        serde_json::from_value(value).unwrap()
    }

    fn css_locator() -> ExecuteLocatorCommand {
        ExecuteLocatorCommand {
            selector: Some("#save".into()),
            candidate: None,
            locator_context: None,
            action: "pointer_click".into(),
            page_context_revision: "revision-1".into(),
            snapshot_id: None,
        }
    }

    fn snapshot_locator() -> ExecuteLocatorCommand {
        ExecuteLocatorCommand {
            selector: None,
            candidate: Some(ExecuteLocatorCandidate {
                kind: ExecuteLocatorCandidateKind::TestId,
                arguments: LocatorCandidateArguments {
                    attribute: Some("data-testid".into()),
                    value: Some("save".into()),
                    ..Default::default()
                },
                expression: None,
                context: None,
                match_count: None,
                visible: None,
                enabled: None,
                verification: None,
                score: None,
                stability_rationale: None,
                warnings: None,
            }),
            locator_context: None,
            action: "click".into(),
            page_context_revision: "revision-1".into(),
            snapshot_id: Some("snapshot-1".into()),
        }
    }

    #[test]
    fn css_plan_keeps_request_target_revision_and_lease_boundary() {
        let plan = BrowserActionCoordinator::plan(
            &request(Some(json!({"kind": "load_complete"}))),
            &target(),
            "profile-a",
            &css_locator(),
        )
        .unwrap();
        assert_eq!(plan.command["cmd"], "execute_locator");
        assert_eq!(plan.command["request_id"], "action-1");
        assert_eq!(plan.command["target"], json!(target()));
        assert_eq!(plan.command["page_context_revision"], "revision-1");
        assert_eq!(plan.command["wait"]["kind"], "load_complete");
        assert!(plan.command.get("lease_token").is_none());
    }

    #[test]
    fn snapshot_locator_is_reused_for_action_and_element_wait() {
        let mut action_request = request(Some(json!({
            "kind": "element_state",
            "state": "visible",
            "element": {"css": "#save"}
        })));
        action_request
            .arguments
            .insert("action".into(), json!("click"));
        let plan = BrowserActionCoordinator::plan(
            &action_request,
            &target(),
            "profile-a",
            &snapshot_locator(),
        )
        .unwrap();
        assert_eq!(plan.command["candidate"]["kind"], "test_id");
        assert_eq!(plan.command["candidate"]["arguments"]["value"], "save");
        assert_eq!(
            plan.command["wait"]["element"]["candidate"]["kind"],
            "test_id"
        );
        assert_eq!(
            plan.metadata.locator.snapshot_id.as_deref(),
            Some("snapshot-1")
        );
    }

    #[test]
    fn invalid_wait_is_rejected_before_dispatch() {
        let error = BrowserActionCoordinator::plan(
            &request(Some(json!({
                "kind": "element_state",
                "state": "unknown",
                "element": {"css": "#save"}
            }))),
            &target(),
            "profile-a",
            &css_locator(),
        )
        .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::InvalidBrowserOperation);
    }

    #[test]
    fn wait_timeout_is_failure_after_action_completion() {
        let target = target();
        let response = ExtensionResponse {
            message_type: "response".into(),
            schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
            protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
            request_id: "action-1".into(),
            operation: "execute_locator".into(),
            extension_instance_id: Some("profile-a".into()),
            target: Some(target.clone()),
            ok: true,
            code: None,
            error: None,
            result: BTreeMap::from([
                ("action_outcome".into(), json!({"ok": true})),
                (
                    "wait_outcome".into(),
                    json!({"ok": false, "code": "browser_wait_timeout"}),
                ),
            ]),
        };
        let metadata = ActionMetadata {
            action: "click".into(),
            locator: css_locator(),
            may_have_side_effect: true,
        };
        let mut value = serde_json::to_value(&response).unwrap();
        let error = BrowserActionCoordinator::finalize_response(
            &mut value, &response, "action-1", &target, &metadata,
        )
        .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserWaitTimeout);
        assert_eq!(error.recovery["action_executed"], true);
        assert_eq!(
            error.recovery["retry"],
            "do not retry the action automatically; reconcile the page state first"
        );
    }

    #[test]
    fn locator_failures_remain_terminal_failures_with_stable_context() {
        let target = target();
        let metadata = ActionMetadata {
            action: "click".into(),
            locator: css_locator(),
            may_have_side_effect: true,
        };
        let cases = [
            (
                "stale_element_reference",
                BrokerErrorCode::StaleElementReference,
            ),
            ("element_not_found", BrokerErrorCode::ElementNotFound),
            ("not_visible", BrokerErrorCode::BrowserOperationFailed),
            ("element_disabled", BrokerErrorCode::BrowserOperationFailed),
            ("browser_wait_timeout", BrokerErrorCode::BrowserWaitTimeout),
            ("stale_page_context", BrokerErrorCode::StaleBrowserTarget),
            (
                "assert_text_failed",
                BrokerErrorCode::BrowserOperationFailed,
            ),
            (
                "assert_not_exists_failed",
                BrokerErrorCode::BrowserOperationFailed,
            ),
        ];

        for (extension_code, expected_code) in cases {
            let response = ExtensionResponse {
                message_type: "response".into(),
                schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
                protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
                request_id: "action-failure".into(),
                operation: "execute_locator".into(),
                extension_instance_id: Some("profile-a".into()),
                target: Some(target.clone()),
                ok: false,
                code: Some(extension_code.into()),
                error: Some("fixture failure".into()),
                result: BTreeMap::from([(
                    "action_outcome".into(),
                    json!({"ok": false, "match_count": if extension_code == "stale_element_reference" { 2 } else { 0 }}),
                )]),
            };
            let mut value = serde_json::to_value(&response).unwrap();
            let error = BrowserActionCoordinator::finalize_response(
                &mut value,
                &response,
                "action-failure",
                &target,
                &metadata,
            )
            .unwrap_err();

            assert_eq!(error.code, expected_code, "extension code {extension_code}");
            assert_eq!(value["ok"], false);
            assert_eq!(value["outcome"], "failed");
            assert_eq!(error.recovery["extension_code"], extension_code);
            assert_eq!(error.recovery["retry"], "do_not_retry_automatically");
        }
    }
}
