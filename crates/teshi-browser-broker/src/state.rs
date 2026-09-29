//! Single-owner broker state machine.
//!
//! The HTTP/WebSocket transport deliberately has no session or lease authority.
//! This module consumes its typed events on one async task, so target, lease and
//! pending-request transitions are serialized without holding locks over socket
//! or filesystem I/O.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use subtle::ConstantTimeEq;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::authorization::{self, AuthorizationState, Capability, PrivilegedRequest};
use crate::coordinator::{ActionMetadata, BrowserActionCoordinator};
use crate::evidence::{EvidenceStore, NetworkBodyAccess};
use crate::protocol::{
    BROWSER_BROKER_PROTOCOL_VERSION, BROWSER_BROKER_SCHEMA_VERSION, BrokerError, BrokerErrorCode,
    BrowserTarget, ExecuteLocatorActionRequest, ExecuteLocatorCandidate,
    ExecuteLocatorCandidateKind, ExecuteLocatorCommand, ExecuteLocatorInput, ExtensionResponse,
    ExtensionStreamMessage, LocatorCandidateArguments, LocatorContext, LocatorIntent,
    LocatorSnapshot, LocatorVerificationStatus, NetworkBatch, OperationRequest, SnapshotElement,
};
use crate::server::{BrokerEvent, BrokerRuntime};
use crate::session::{BrowserSessionRecord, SessionRegistry};

const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_LEASE_TTL_SECS: u64 = 5;
const DEFAULT_LEASE_TTL_SECS: u64 = 60;
const MAX_LEASE_TTL_SECS: u64 = 3600;
const MAX_PENDING_REQUESTS: usize = 128;
const MAX_QUARANTINED_RESPONSES: usize = 32;
const MAX_RETIRED_REQUESTS: usize = MAX_PENDING_REQUESTS * 8;
const RETIRED_REQUEST_TTL: Duration = Duration::from_secs(600);
const RUST_P0_EXECUTABLE_ACTIONS: [&str; 7] = [
    "click",
    "pointer_click",
    "fill",
    "assert_visible",
    "assert_not_exists",
    "assert_text",
    "upload",
];

#[derive(Debug)]
struct LeaseRecord {
    token: String,
    owner_label: String,
    project_root: String,
    caller_label: String,
    broker_start_id: String,
    acquired_at_ms: u64,
    expires_at_ms: u64,
    expires_at: Instant,
}

#[derive(Debug)]
struct PendingRequest {
    operation: String,
    extension_instance_id: String,
    target: BrowserTarget,
    project_root: String,
    caller_label: String,
    lease_token: Option<String>,
    snapshot_id: Option<String>,
    stream_generation: Option<u64>,
    /// When a direct command falls back to HTTP heartbeat, retain the stream
    /// generation that was allowed to consume it. HTTP responses intentionally
    /// carry no stream generation, so this is separate from the response check.
    fallback_generation: Option<u64>,
    console_capture_id: Option<String>,
    deadline: Instant,
    reply: oneshot::Sender<Result<Value, BrokerError>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocatorWorkflowStage {
    Snapshot,
    Verify,
}

#[derive(Debug, Clone)]
struct LocatorWorkflow {
    intent: LocatorIntent,
    test_id_attributes: Vec<String>,
    stage: LocatorWorkflowStage,
    candidates: Vec<ExecuteLocatorCandidate>,
    element: Option<SnapshotElement>,
    page_context_revision: Option<String>,
    url: String,
    title: String,
}

/// State owner for one user-scoped broker process.
#[derive(Debug, Default)]
pub struct BrokerState {
    pub sessions: SessionRegistry,
    leases: HashMap<String, LeaseRecord>,
    pending: HashMap<String, PendingRequest>,
    /// Request IDs remain reserved for a bounded quarantine window after a
    /// terminal transition. This prevents a late response from an old command
    /// completing a newly reused request ID.
    retired_requests: HashMap<String, Instant>,
    quarantined_responses: Vec<Value>,
    /// Action metadata is separate from `PendingRequest` so the 4.1 pending
    /// DTO remains compatible with callers that construct it in tests/tools.
    action_metadata: HashMap<String, ActionMetadata>,
    /// Requests that reached a transport dispatch boundary and may have run.
    dispatched_actions: HashSet<String>,
    /// Single owner for prepared and published screenshot/PDF artifacts.
    evidence: EvidenceStore,
    /// Explicit response-body grants are short-lived request state. They are
    /// removed with the pending request so a body cannot be fetched after a
    /// timeout, cancellation, lease transition, or capture cleanup.
    network_body_access: HashMap<String, NetworkBodyAccess>,
    /// Operation-specific P2 result limits retained independently of the
    /// compatibility PendingRequest shape.
    privileged_requests: HashMap<String, PrivilegedRequest>,
    /// Memory-only short-lived authorization for privileged browser surfaces.
    authorization: AuthorizationState,
    /// Two-phase locator acquisition state: snapshot, then live extension
    /// verification. The public request ID remains stable across both phases.
    locator_workflows: HashMap<String, LocatorWorkflow>,
}

impl BrokerState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume one transport event. The caller owns the only mutable instance
    /// and must not call this concurrently with another event.
    pub async fn handle(&mut self, event: BrokerEvent, runtime: &BrokerRuntime) {
        self.expire(Instant::now());
        match event {
            BrokerEvent::Heartbeat { payload, reply, .. } => {
                let now = Instant::now();
                let result = match self
                    .sessions
                    .register_heartbeat_with_target_closures(payload, now)
                {
                    Ok((instance_id, closed_targets)) => {
                        for target in closed_targets {
                            self.evidence
                                .terminate_console_target(&target, "target_closed", None);
                            self.revoke_network_body_access_for_target(&target);
                            self.evidence
                                .terminate_network_target(&target, "target_closed", None);
                        }
                        Ok(self.sessions.heartbeat_response(&instance_id, now))
                    }
                    Err(error) => Err(error),
                };
                let mut response = result.unwrap_or_else(error_value);
                self.validate_heartbeat_command(&mut response, runtime);
                let _ = reply.send(response);
            }
            BrokerEvent::Operation { request, reply, .. } => {
                self.handle_operation(request, reply, runtime).await;
            }
            BrokerEvent::ExtensionConnected {
                hello,
                generation,
                reply,
                ..
            } => {
                let response = self.handle_extension_connected(hello, generation);
                let _ = reply.send(response);
            }
            BrokerEvent::ExtensionDisconnected {
                extension_instance_id,
                generation,
            } => self.handle_extension_disconnected(&extension_instance_id, generation),
            BrokerEvent::ExtensionTrustRevoked {
                extension_instance_ids,
            } => self.handle_extension_trust_revoked(&extension_instance_ids),
            BrokerEvent::Unsubscribe { owner_id } => {
                self.sessions.unsubscribe_owner(&owner_id);
            }
            BrokerEvent::SubscriptionActive {
                owner_id,
                target,
                reply,
            } => {
                let _ = reply.send(self.sessions.owner_is_subscribed(&owner_id, &target));
            }
            BrokerEvent::ExtensionResponse {
                extension_instance_id,
                generation,
                response,
                reply,
                ..
            } => {
                let broker_start_id = runtime.endpoint_record().broker_start_id;
                let response_value = self.handle_extension_response_with_broker_generation(
                    &extension_instance_id,
                    generation,
                    Some(&broker_start_id),
                    response,
                );
                if let Some(reply) = reply {
                    let _ = reply.send(response_value);
                }
            }
            BrokerEvent::ExtensionHttpMessage {
                path,
                extension_instance_id,
                generation,
                payload,
                reply,
                ..
            } => {
                if payload.get("type").and_then(Value::as_str) == Some("frame_error")
                    && let (Some(instance_id), Some(error)) = (
                        extension_instance_id.as_deref(),
                        payload.get("error").and_then(Value::as_str),
                    )
                {
                    self.sessions.mark_frame_error(instance_id, error);
                }
                let response = match payload.get("type").and_then(Value::as_str) {
                    Some("console_event") => self.handle_console_event_message(
                        extension_instance_id.as_deref(),
                        generation,
                        &payload,
                    ),
                    Some("console_capture_terminated") => self.handle_console_termination_message(
                        extension_instance_id.as_deref(),
                        generation,
                        &payload,
                    ),
                    _ => json!({
                        "ok": true,
                        "path": path,
                        "extension_instance_id": extension_instance_id,
                    }),
                };
                let _ = reply.send(response);
            }
            BrokerEvent::NetworkBatch {
                batch,
                generation,
                reply,
                ..
            } => {
                let _ = reply.send(self.handle_network_batch(batch, generation));
            }
            BrokerEvent::PreviewFrame {
                target,
                seq,
                url,
                jpeg,
                frame,
                ..
            } => {
                let accepted = self.sessions.update_frame(
                    target.clone(),
                    seq,
                    url,
                    jpeg.to_vec(),
                    Instant::now(),
                );
                match accepted {
                    Ok(true) => {
                        let owner_ids = self.sessions.subscriber_owner_ids(&target);
                        if !owner_ids.is_empty() {
                            let _ = runtime.publish(crate::server::BrokerPublication {
                                target,
                                frame,
                                owner_ids: Arc::from(owner_ids.into_boxed_slice()),
                            });
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        self.sessions
                            .mark_frame_error(&target.extension_instance_id, &error.message);
                    }
                }
            }
            BrokerEvent::FrameError {
                extension_instance_id,
                error,
            } => self
                .sessions
                .mark_frame_error(&extension_instance_id, &error),
            BrokerEvent::Subscribe {
                owner_id,
                extension_instance_id,
                target,
                request_id,
                reply,
            } => {
                let response = self.handle_subscription(
                    &owner_id,
                    &extension_instance_id,
                    target.as_ref(),
                    &request_id,
                );
                let _ = reply.send(response);
            }
        }
    }

    pub(crate) fn tick(&mut self) {
        self.expire(Instant::now());
    }

    fn handle_extension_connected(
        &mut self,
        hello: ExtensionStreamMessage,
        generation: u64,
    ) -> Value {
        let (extension_instance_id, protocol_version) = match hello {
            ExtensionStreamMessage::StreamHello {
                extension_instance_id,
                protocol_version,
                ..
            } => (extension_instance_id, protocol_version),
        };
        let now = Instant::now();
        let result = self
            .sessions
            .ensure_stream_session(&extension_instance_id, protocol_version, now)
            .and_then(|()| {
                self.sessions
                    .attach_stream(&extension_instance_id, generation, now)
            });
        if matches!(&result, Ok(Some(_))) {
            self.fail_pending_for_session(
                &extension_instance_id,
                BrokerError::new(
                    BrokerErrorCode::BrowserSessionDisconnected,
                    "browser extension stream was replaced while the operation was pending",
                ),
            );
        }
        if result.is_ok() {
            self.evidence
                .update_console_stream_generation(&extension_instance_id, generation);
            self.evidence
                .update_network_stream_generation(&extension_instance_id, generation);
            // A reconnect or stream replacement invalidates all old preview
            // subscriptions. UI clients must explicitly re-select the target.
            self.sessions.unsubscribe_instance(&extension_instance_id);
        }
        match result {
            Ok(previous_generation) => json!({
                "type": "stream_hello_ack",
                "ok": true,
                "schema_version": BROWSER_BROKER_SCHEMA_VERSION,
                "protocol_version": BROWSER_BROKER_PROTOCOL_VERSION,
                "extension_instance_id": extension_instance_id,
                "generation": generation,
                "replaced_generation": previous_generation,
            }),
            Err(error) => error_value(error),
        }
    }

    fn validate_heartbeat_command(&mut self, response: &mut Value, runtime: &BrokerRuntime) {
        let Some(command) = response.get("cmd").filter(|value| !value.is_null()) else {
            response["cmd"] = Value::Null;
            return;
        };
        if command.get("cmd").and_then(Value::as_str) == Some("__teshi_stop_console_capture") {
            let target_matches = command
                .get("target")
                .cloned()
                .and_then(|value| serde_json::from_value::<BrowserTarget>(value).ok())
                .is_some_and(|target| {
                    target.extension_instance_id
                        == response
                            .get("extension_instance_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                });
            let capture_id_present = command
                .get("capture_id")
                .and_then(Value::as_str)
                .is_some_and(|capture_id| !capture_id.trim().is_empty());
            if !target_matches || !capture_id_present {
                response["cmd"] = Value::Null;
            }
            return;
        }
        let Some(request_id) = command
            .get("request_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            response["cmd"] = Value::Null;
            return;
        };
        let Some((
            operation,
            instance_id,
            project_root,
            caller_label,
            lease_token,
            fallback_generation,
        )) = self.pending.get(&request_id).map(|pending| {
            (
                pending.operation.clone(),
                pending.extension_instance_id.clone(),
                pending.project_root.clone(),
                pending.caller_label.clone(),
                pending.lease_token.clone(),
                pending.fallback_generation,
            )
        })
        else {
            response["cmd"] = Value::Null;
            return;
        };
        if let Some(expected_generation) = fallback_generation {
            let current_generation = self
                .sessions
                .get(&instance_id)
                .and_then(|session| session.current_stream_generation());
            if current_generation != Some(expected_generation) {
                response["cmd"] = Value::Null;
                if let Some(pending) = self.pending.remove(&request_id) {
                    self.abort_evidence(&request_id, &pending.operation);
                    self.retire_request(&request_id, Instant::now());
                    if let Some(session) = self.sessions.get_mut(&instance_id) {
                        session.remove_queued_command(&request_id);
                    }
                    let _ = pending.reply.send(Err(BrokerError::new(
                        BrokerErrorCode::BrowserSessionDisconnected,
                        "browser extension stream generation changed before heartbeat fallback dispatch",
                    )));
                }
                return;
            }
        }
        if !requires_lease(&operation) {
            self.mark_action_dispatched(&request_id);
            return;
        }
        let validation = lease_token
            .as_deref()
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::InvalidBrowserLease,
                    "queued browser operation no longer has a lease token",
                )
            })
            .and_then(|token| {
                self.validate_lease(
                    &instance_id,
                    token,
                    &project_root,
                    &caller_label,
                    &runtime.endpoint_record().broker_start_id,
                )
                .map(|_| ())
            });
        if let Err(error) = validation {
            response["cmd"] = Value::Null;
            if let Some(pending) = self.pending.remove(&request_id) {
                self.abort_evidence(&request_id, &pending.operation);
                self.retire_request(&request_id, Instant::now());
                let _ = pending.reply.send(Err(error));
            }
        } else {
            self.mark_action_dispatched(&request_id);
        }
    }

    fn handle_extension_disconnected(&mut self, extension_instance_id: &str, generation: u64) {
        if self
            .sessions
            .detach_stream(extension_instance_id, generation)
        {
            self.evidence.terminate_console_session(
                extension_instance_id,
                "stream_disconnected",
                None,
            );
            // Keep Network capture state across a transient stream loss. The
            // extension retains its bounded retry queue and will resend the
            // same capture's unacknowledged events after reconnect; the new
            // stream generation is bound when it attaches.
            self.sessions.unsubscribe_instance(extension_instance_id);
            self.fail_pending_for_session(
                extension_instance_id,
                BrokerError::new(
                    BrokerErrorCode::BrowserSessionDisconnected,
                    "browser extension stream disconnected while the operation was pending",
                ),
            );
        }
    }

    fn handle_extension_trust_revoked(&mut self, extension_instance_ids: &[String]) {
        for extension_instance_id in extension_instance_ids {
            self.leases.remove(extension_instance_id);
            self.queue_console_cleanup(extension_instance_id);
            self.queue_network_cleanup(extension_instance_id);
            self.evidence
                .terminate_console_session(extension_instance_id, "trust_revoked", None);
            self.terminate_network_session(extension_instance_id, "trust_revoked", None);
            self.revoke_network_body_access_for_instance(extension_instance_id);
            self.sessions.unsubscribe_instance(extension_instance_id);
            self.sessions
                .clear_element_references(extension_instance_id);
            self.authorization
                .revoke_for_extension_instance(extension_instance_id);
            self.fail_pending_for_session(
                extension_instance_id,
                BrokerError::new(
                    BrokerErrorCode::BrokerOriginDenied,
                    "browser extension trust was revoked while the operation was pending",
                ),
            );
        }
    }

    fn handle_console_event_message(
        &mut self,
        extension_instance_id: Option<&str>,
        transport_generation: Option<u64>,
        payload: &Value,
    ) -> Value {
        let Some(instance_id) = extension_instance_id else {
            return json!({"ok": false, "accepted": false});
        };
        let Some(target) = payload
            .get("target")
            .cloned()
            .and_then(|value| serde_json::from_value::<BrowserTarget>(value).ok())
        else {
            return json!({"ok": false, "accepted": false});
        };
        let Some(stream_generation) = extension_event_generation(payload, transport_generation)
        else {
            return json!({"ok": false, "accepted": false});
        };
        let accepted = self.evidence.record_console_event(
            instance_id,
            &target,
            payload.get("capture_id").and_then(Value::as_str),
            stream_generation,
            payload.get("event"),
        );
        json!({
            "ok": true,
            "schema_version": BROWSER_BROKER_SCHEMA_VERSION,
            "accepted": accepted,
        })
    }

    fn handle_console_termination_message(
        &mut self,
        extension_instance_id: Option<&str>,
        transport_generation: Option<u64>,
        payload: &Value,
    ) -> Value {
        let Some(instance_id) = extension_instance_id else {
            return json!({"ok": false, "accepted": false});
        };
        let Some(target) = payload
            .get("target")
            .cloned()
            .and_then(|value| serde_json::from_value::<BrowserTarget>(value).ok())
        else {
            return json!({"ok": false, "accepted": false});
        };
        let Some(stream_generation) = extension_event_generation(payload, transport_generation)
        else {
            return json!({"ok": false, "accepted": false});
        };
        let accepted = self.evidence.record_console_termination_event(
            instance_id,
            &target,
            payload.get("capture_id").and_then(Value::as_str),
            stream_generation,
            payload
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("extension_reported"),
            payload.get("detail").and_then(Value::as_str),
        );
        let termination = accepted
            .then(|| self.evidence.latest_console_termination())
            .flatten();
        json!({
            "ok": true,
            "schema_version": BROWSER_BROKER_SCHEMA_VERSION,
            "accepted": accepted,
            "termination": termination,
        })
    }

    #[allow(dead_code)]
    fn handle_extension_response(
        &mut self,
        event_instance_id: &str,
        generation: Option<u64>,
        response: ExtensionResponse,
    ) -> Value {
        self.handle_extension_response_with_broker_generation(
            event_instance_id,
            generation,
            None,
            response,
        )
    }

    fn expected_extension_operation(&self, request_id: &str, operation: &str) -> String {
        if operation == "resolve_playwright_locator"
            && let Some(workflow) = self.locator_workflows.get(request_id)
        {
            return match workflow.stage {
                LocatorWorkflowStage::Snapshot => "get_page_snapshot",
                LocatorWorkflowStage::Verify => "verify_playwright_locators",
            }
            .into();
        }
        extension_operation_for(operation).into()
    }

    fn handle_locator_workflow_response(
        &mut self,
        request_id: &str,
        response: &ExtensionResponse,
    ) -> Value {
        let Some(stage) = self
            .locator_workflows
            .get(request_id)
            .map(|workflow| workflow.stage)
        else {
            return error_value(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "locator workflow is missing for the pending browser request",
            ));
        };
        match stage {
            LocatorWorkflowStage::Snapshot => {
                self.handle_locator_snapshot_response(request_id, response)
            }
            LocatorWorkflowStage::Verify => {
                self.handle_locator_verification_response(request_id, response)
            }
        }
    }

    fn handle_locator_snapshot_response(
        &mut self,
        request_id: &str,
        response: &ExtensionResponse,
    ) -> Value {
        let Some((
            instance_id,
            target,
            project_root,
            caller_label,
            stream_generation,
            fallback_generation,
        )) = self.pending.get(request_id).map(|pending| {
            (
                pending.extension_instance_id.clone(),
                pending.target.clone(),
                pending.project_root.clone(),
                pending.caller_label.clone(),
                pending.stream_generation,
                pending.fallback_generation,
            )
        })
        else {
            return error_value(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "locator snapshot response has no pending request",
            ));
        };
        if !response.ok {
            return self.complete_pending_error(request_id, extension_response_error(response));
        }
        let Some(workflow) = self.locator_workflows.get(request_id).cloned() else {
            return error_value(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "locator snapshot response has no workflow state",
            ));
        };
        let snapshot_value = Value::Object(response.result.clone().into_iter().collect());
        let snapshot = match LocatorSnapshot::normalize(&snapshot_value) {
            Ok(snapshot) => snapshot,
            Err(error) => return self.complete_pending_error(request_id, error),
        };
        let page_context_revision = match snapshot
            .page_context_revision
            .clone()
            .filter(|value| !value.trim().is_empty())
        {
            Some(revision) => revision,
            None => {
                return self.complete_pending_error(
                    request_id,
                    BrokerError::new(
                        BrokerErrorCode::BrokerProtocolError,
                        "browser snapshot did not return page_context_revision",
                    ),
                );
            }
        };
        let resolution =
            match snapshot.generate_candidates(&workflow.intent, &workflow.test_id_attributes) {
                Ok(resolution) => resolution,
                Err(error) => return self.complete_pending_error(request_id, error),
            };
        let mut cached_snapshot = snapshot_value;
        let cache_result = self
            .sessions
            .get_mut(&instance_id)
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::BrowserTargetNotFound,
                    "browser session was not found while caching the locator snapshot",
                )
            })
            .and_then(|session| {
                session.cache_snapshot_references(
                    target.clone(),
                    &mut cached_snapshot,
                    request_id,
                    &project_root,
                    &caller_label,
                    Instant::now(),
                )
            });
        if let Err(error) = cache_result {
            return self.complete_pending_error(request_id, error);
        }
        let command = build_locator_verification_command(
            request_id,
            &target,
            &instance_id,
            &caller_label,
            &project_root,
            &page_context_revision,
            &resolution.candidates,
        );
        let queue_result = self
            .sessions
            .get_mut(&instance_id)
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::BrowserSessionDisconnected,
                    "browser session disconnected before locator verification",
                )
            })
            .and_then(|session| session.queue_command(command));
        if let Err(error) = queue_result {
            return self.complete_pending_error(request_id, error);
        }
        if let Some(workflow) = self.locator_workflows.get_mut(request_id) {
            workflow.stage = LocatorWorkflowStage::Verify;
            workflow.candidates = resolution.candidates;
            workflow.element = Some(resolution.element);
            workflow.page_context_revision = Some(page_context_revision);
            workflow.url = snapshot.url;
            workflow.title = snapshot.title;
        }
        if let Some(pending) = self.pending.get_mut(request_id) {
            pending.stream_generation = None;
            pending.fallback_generation = fallback_generation.or(stream_generation);
        }
        json!({
            "ok": true,
            "request_id": request_id,
            "queued": true,
            "operation": "verify_playwright_locators",
        })
    }

    fn handle_locator_verification_response(
        &mut self,
        request_id: &str,
        response: &ExtensionResponse,
    ) -> Value {
        if !response.ok {
            return self.complete_pending_error(request_id, extension_response_error(response));
        }
        let Some(workflow) = self.locator_workflows.get(request_id).cloned() else {
            return error_value(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "locator verification response has no workflow state",
            ));
        };
        let verification = match response.locator_verification_results() {
            Ok(verification) => verification,
            Err(error) => return self.complete_pending_error(request_id, error),
        };
        let candidates = crate::protocol::apply_locator_verification_results(
            &workflow.candidates,
            &verification,
        );
        let recommended = candidates
            .iter()
            .find(|candidate| candidate.verification == Some(LocatorVerificationStatus::Verified))
            .cloned();
        let Some(page_context_revision) = workflow.page_context_revision else {
            return self.complete_pending_error(
                request_id,
                BrokerError::new(
                    BrokerErrorCode::BrokerProtocolError,
                    "locator workflow has no page_context_revision",
                ),
            );
        };
        let Some(element) = workflow.element else {
            return self.complete_pending_error(
                request_id,
                BrokerError::new(
                    BrokerErrorCode::BrokerProtocolError,
                    "locator workflow has no selected element",
                ),
            );
        };
        let Some((target, extension_instance_id)) = self.pending.get(request_id).map(|pending| {
            (
                pending.target.clone(),
                pending.extension_instance_id.clone(),
            )
        }) else {
            return error_value(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "locator verification response has no pending request",
            ));
        };
        let value = json!({
            "type": "response",
            "schema_version": BROWSER_BROKER_SCHEMA_VERSION,
            "protocol_version": BROWSER_BROKER_PROTOCOL_VERSION,
            "request_id": request_id,
            "operation": "resolve_playwright_locator",
            "ok": true,
            "extension_instance_id": extension_instance_id,
            "target": target,
            "page_context_revision": page_context_revision,
            "url": workflow.url,
            "title": workflow.title,
            "element": element,
            "recommended": recommended,
            "candidates": candidates,
        });
        self.complete_pending_value(request_id, value)
    }

    fn complete_pending_error(&mut self, request_id: &str, error: BrokerError) -> Value {
        let Some(pending) = self.pending.remove(request_id) else {
            return error_value(error);
        };
        self.abort_evidence(request_id, &pending.operation);
        let terminal = self.terminal_pending_error(request_id, &pending, error);
        self.retire_request(request_id, Instant::now());
        if let Some(session) = self.sessions.get_mut(&pending.extension_instance_id) {
            session.remove_queued_command(request_id);
        }
        let value = error_value(terminal.clone());
        let _ = pending.reply.send(Err(terminal));
        value
    }

    fn complete_pending_value(&mut self, request_id: &str, value: Value) -> Value {
        let Some(pending) = self.pending.remove(request_id) else {
            return error_value(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "browser response was already completed",
            ));
        };
        self.retire_request(request_id, Instant::now());
        let _ = pending.reply.send(Ok(value.clone()));
        value
    }

    fn handle_extension_response_with_broker_generation(
        &mut self,
        event_instance_id: &str,
        generation: Option<u64>,
        broker_start_id: Option<&str>,
        response: ExtensionResponse,
    ) -> Value {
        let mut response = response;
        if let Err(error) = response.validate() {
            self.quarantine(&response, "invalid_response");
            return error_value(error);
        }
        let request_id = response.request_id.clone();
        let Some(pending) = self.pending.get(&request_id) else {
            self.quarantine(
                &response,
                if self.retired_requests.contains_key(&request_id) {
                    "late_response"
                } else {
                    "unknown_request_id"
                },
            );
            return error_value(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "no pending browser request matches this response",
            ));
        };
        let instance_matches = event_instance_id == pending.extension_instance_id
            && response.extension_instance_id.as_deref()
                == Some(pending.extension_instance_id.as_str());
        let target_matches = response
            .target
            .as_ref()
            .is_some_and(|target| target == &pending.target);
        let generation_matches = if is_evidence_operation(&pending.operation)
            || is_console_operation(&pending.operation)
            || is_network_operation(&pending.operation)
        {
            match (
                pending.stream_generation,
                pending.fallback_generation,
                generation,
            ) {
                (Some(expected), _, Some(actual)) => expected == actual,
                (None, Some(expected), None) => {
                    self.sessions
                        .get(&pending.extension_instance_id)
                        .and_then(|session| session.current_stream_generation())
                        == Some(expected)
                }
                (None, None, None) => true,
                _ => false,
            }
        } else {
            pending.stream_generation == generation
        };
        let expected_operation = self.expected_extension_operation(&request_id, &pending.operation);
        let operation_matches = response.operation == expected_operation;
        let capture_id_matches = !response.ok
            || pending
                .console_capture_id
                .as_deref()
                .is_none_or(|expected| {
                    response.result.get("capture_id").and_then(Value::as_str) == Some(expected)
                });
        if !instance_matches
            || !target_matches
            || !generation_matches
            || !operation_matches
            || !capture_id_matches
        {
            self.quarantine(
                &response,
                if !capture_id_matches {
                    if is_network_operation(&pending.operation) {
                        "network_capture_id_mismatch"
                    } else {
                        "console_capture_id_mismatch"
                    }
                } else if generation_matches {
                    "target_or_operation_mismatch"
                } else {
                    "stream_generation_mismatch"
                },
            );
            return error_value(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "browser response does not match its pending request",
            ));
        }

        if self.locator_workflows.contains_key(&request_id) {
            return self.handle_locator_workflow_response(&request_id, &response);
        }

        let Some(pending) = self.pending.remove(&request_id) else {
            return error_value(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "browser response was already completed",
            ));
        };
        let action_metadata = self.action_metadata.get(&request_id).cloned();
        let network_body_access = self.network_body_access.get(&request_id).cloned();
        let privileged_request = self.privileged_requests.get(&request_id).cloned();
        self.retire_request(&request_id, Instant::now());

        let console_capture = if pending.operation == "start_console_capture" {
            if !response.ok {
                self.evidence.abort_console_capture(&request_id);
                None
            } else {
                let Some(broker_start_id) = broker_start_id else {
                    self.evidence.abort_console_capture(&request_id);
                    let error = BrokerError::new(
                        BrokerErrorCode::MismatchedBrowserResponse,
                        "console capture response has no broker generation",
                    );
                    let _ = pending.reply.send(Err(error.clone()));
                    return error_value(error);
                };
                let result = self.evidence.commit_console_capture(
                    &request_id,
                    broker_start_id,
                    &pending.project_root,
                    &pending.caller_label,
                    &pending.target,
                    &response,
                );
                match result {
                    Ok(summary) => Some(summary),
                    Err(error) => {
                        if let Some(expected_capture_id) = pending.console_capture_id.as_deref() {
                            let cleanup_capture_id = response
                                .result
                                .get("capture_id")
                                .and_then(Value::as_str)
                                .filter(|capture_id| !capture_id.trim().is_empty())
                                .unwrap_or(expected_capture_id);
                            self.queue_console_stop_command(
                                &pending.extension_instance_id,
                                &pending.target,
                                cleanup_capture_id,
                            );
                        }
                        let _ = pending.reply.send(Err(error.clone()));
                        return error_value(error);
                    }
                }
            }
        } else {
            None
        };
        let network_capture = if pending.operation == "start_network_capture" {
            if !response.ok {
                self.evidence.abort_network_capture(&request_id);
                None
            } else {
                let Some(broker_start_id) = broker_start_id else {
                    self.evidence.abort_network_capture(&request_id);
                    let error = BrokerError::new(
                        BrokerErrorCode::MismatchedBrowserResponse,
                        "network capture response has no broker generation",
                    );
                    let _ = pending.reply.send(Err(error.clone()));
                    return error_value(error);
                };
                let lease_result = pending
                    .lease_token
                    .as_deref()
                    .ok_or_else(|| {
                        BrokerError::new(
                            BrokerErrorCode::InvalidBrowserLease,
                            "network capture response has no lease token",
                        )
                    })
                    .and_then(|token| {
                        self.validate_lease(
                            &pending.extension_instance_id,
                            token,
                            &pending.project_root,
                            &pending.caller_label,
                            broker_start_id,
                        )
                        .map(|_| ())
                    });
                if let Err(error) = lease_result {
                    self.evidence.abort_network_capture(&request_id);
                    if let Some(capture_id) = pending.console_capture_id.as_deref() {
                        self.queue_network_stop_command(
                            &pending.extension_instance_id,
                            &pending.target,
                            capture_id,
                        );
                    }
                    let _ = pending.reply.send(Err(error.clone()));
                    return error_value(error);
                }
                match self.evidence.commit_network_capture(
                    &request_id,
                    broker_start_id,
                    &pending.project_root,
                    &pending.caller_label,
                    &pending.target,
                    &response,
                ) {
                    Ok(summary) => {
                        self.revoke_network_body_access_for_target(&pending.target);
                        Some(summary)
                    }
                    Err(error) => {
                        if let Some(expected_capture_id) = pending.console_capture_id.as_deref() {
                            let cleanup_capture_id = response
                                .result
                                .get("capture_id")
                                .and_then(Value::as_str)
                                .filter(|capture_id| !capture_id.trim().is_empty())
                                .unwrap_or(expected_capture_id);
                            self.queue_network_stop_command(
                                &pending.extension_instance_id,
                                &pending.target,
                                cleanup_capture_id,
                            );
                        }
                        let _ = pending.reply.send(Err(error.clone()));
                        return error_value(error);
                    }
                }
            }
        } else {
            None
        };
        let stopped_console_capture = if pending.operation == "stop_console_capture" && response.ok
        {
            if let Some(_expected_capture_id) = pending.console_capture_id.as_deref()
                && let Some(broker_start_id) = broker_start_id
                && !self.evidence.console_capture_scope_matches(
                    &pending.target,
                    broker_start_id,
                    &pending.project_root,
                    &pending.caller_label,
                )
            {
                let error = BrokerError::new(
                    BrokerErrorCode::InvalidBrowserLease,
                    "console capture scope changed before stop",
                );
                let _ = pending.reply.send(Err(error.clone()));
                return error_value(error);
            }
            Some(
                self.evidence
                    .stop_console_capture(&pending.target, "explicit_stop", None),
            )
        } else {
            None
        };
        let network_transition = if response.ok
            && matches!(
                pending.operation.as_str(),
                "clear_network_capture" | "stop_network_capture"
            ) {
            let Some(broker_start_id) = broker_start_id else {
                let error = BrokerError::new(
                    BrokerErrorCode::MismatchedBrowserResponse,
                    "network capture response has no broker generation",
                );
                let _ = pending.reply.send(Err(error.clone()));
                return error_value(error);
            };
            let Some(capture_id) = pending.console_capture_id.as_deref() else {
                let error = BrokerError::new(
                    BrokerErrorCode::MismatchedBrowserResponse,
                    "network capture response has no capture_id",
                );
                let _ = pending.reply.send(Err(error.clone()));
                return error_value(error);
            };
            let Some(sequence_barrier) = network_response_barrier(&response.result) else {
                let error = BrokerError::new(
                    BrokerErrorCode::MismatchedBrowserResponse,
                    "network capture response has no explicit sequence barrier",
                );
                let _ = pending.reply.send(Err(error.clone()));
                return error_value(error);
            };
            if !self.evidence.network_capture_scope_matches(
                &pending.target,
                Some(capture_id),
                broker_start_id,
                &pending.project_root,
                &pending.caller_label,
            ) {
                let error = BrokerError::new(
                    BrokerErrorCode::InvalidBrowserLease,
                    "network capture scope changed before barrier commit",
                );
                let _ = pending.reply.send(Err(error.clone()));
                return error_value(error);
            }
            if pending.operation == "clear_network_capture" {
                Some(self.evidence.clear_network_capture(
                    &pending.target,
                    capture_id,
                    broker_start_id,
                    &pending.project_root,
                    &pending.caller_label,
                    sequence_barrier,
                ))
            } else {
                Some(
                    self.evidence.stop_network_capture(
                        &pending.target,
                        capture_id,
                        broker_start_id,
                        &pending.project_root,
                        &pending.caller_label,
                        sequence_barrier,
                        response
                            .result
                            .get("termination_reason")
                            .and_then(Value::as_str)
                            .unwrap_or("explicit_stop"),
                    ),
                )
            }
        } else {
            None
        };
        let network_transition = match network_transition {
            Some(Ok(value)) => Some(value),
            Some(Err(error)) => {
                let _ = pending.reply.send(Err(error.clone()));
                return error_value(error);
            }
            None => None,
        };
        if response.ok
            && matches!(
                pending.operation.as_str(),
                "clear_network_capture" | "stop_network_capture"
            )
            && network_transition.is_some()
        {
            self.revoke_network_body_access_for_target(&pending.target);
        }

        let network_body = if pending.operation == "get_network_request_detail" && response.ok {
            let Some(access) = network_body_access.as_ref() else {
                let error = BrokerError::new(
                    BrokerErrorCode::MismatchedBrowserResponse,
                    "network response-body response has no access grant",
                );
                let _ = pending.reply.send(Err(error.clone()));
                return error_value(error);
            };
            let Some(broker_start_id) = broker_start_id else {
                let error = BrokerError::new(
                    BrokerErrorCode::MismatchedBrowserResponse,
                    "network response-body response has no broker generation",
                );
                let _ = pending.reply.send(Err(error.clone()));
                return error_value(error);
            };
            match self.evidence.bound_network_body(
                &pending.target,
                broker_start_id,
                &pending.project_root,
                &pending.caller_label,
                access,
                response.result.get("body"),
                response
                    .result
                    .get("base64_encoded")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ) {
                Ok(value) => Some(value),
                Err(error) => {
                    let _ = pending.reply.send(Err(error.clone()));
                    return error_value(error);
                }
            }
        } else {
            None
        };

        let artifact = if is_evidence_operation(&pending.operation) {
            if !response.ok {
                self.evidence.abort_request(&request_id);
                None
            } else {
                let Some(broker_start_id) = broker_start_id else {
                    self.evidence.abort_request(&request_id);
                    let error = BrokerError::new(
                        BrokerErrorCode::MismatchedBrowserResponse,
                        "browser evidence response has no broker generation",
                    );
                    let _ = pending.reply.send(Err(error.clone()));
                    return error_value(error);
                };
                let lease_result = pending
                    .lease_token
                    .as_deref()
                    .ok_or_else(|| {
                        BrokerError::new(
                            BrokerErrorCode::InvalidBrowserLease,
                            "browser evidence response has no lease token",
                        )
                    })
                    .and_then(|token| {
                        self.validate_lease(
                            &pending.extension_instance_id,
                            token,
                            &pending.project_root,
                            &pending.caller_label,
                            broker_start_id,
                        )
                        .map(|_| ())
                    });
                if let Err(error) = lease_result {
                    self.evidence.abort_request(&request_id);
                    let _ = pending.reply.send(Err(error.clone()));
                    return error_value(error);
                }
                match self.evidence.commit_response(
                    &request_id,
                    broker_start_id,
                    &pending.operation,
                    &pending.project_root,
                    &pending.caller_label,
                    &pending.target,
                    &response,
                ) {
                    Ok(artifact) => Some(artifact),
                    Err(error) => {
                        let _ = pending.reply.send(Err(error.clone()));
                        return error_value(error);
                    }
                }
            }
        } else {
            None
        };
        if let Some(privileged_request) = privileged_request.as_ref() {
            if response.ok
                && let Err(error) = authorization::sanitize_privileged_response(
                    &pending.operation,
                    &mut response.result,
                    privileged_request,
                )
            {
                self.authorization.append_privileged_audit(
                    privileged_request.audit_capability(),
                    &pending.project_root,
                    &pending.caller_label,
                    serde_json::to_value(&pending.target).unwrap_or(Value::Null),
                    &request_id,
                    error.code.as_str(),
                    privileged_request.audit_arguments(),
                );
                let _ = pending.reply.send(Err(error.clone()));
                return error_value(error);
            }
            self.authorization.append_privileged_audit(
                privileged_request.audit_capability(),
                &pending.project_root,
                &pending.caller_label,
                serde_json::to_value(&pending.target).unwrap_or(Value::Null),
                &request_id,
                if response.ok {
                    "succeeded"
                } else {
                    response.code.as_deref().unwrap_or("failed")
                },
                privileged_request.audit_arguments(),
            );
        }
        let mut value = serde_json::to_value(&response).unwrap_or_else(|_| json!({}));
        if let Value::Object(object) = &mut value {
            object.insert("type".into(), Value::String("response".into()));
            object.insert(
                "schema_version".into(),
                Value::from(BROWSER_BROKER_SCHEMA_VERSION),
            );
            object.insert(
                "protocol_version".into(),
                Value::from(BROWSER_BROKER_PROTOCOL_VERSION),
            );
            object.insert("operation".into(), Value::String(pending.operation.clone()));
            object.insert(
                "extension_instance_id".into(),
                Value::String(pending.extension_instance_id.clone()),
            );
            object.insert(
                "target".into(),
                serde_json::to_value(&pending.target).unwrap(),
            );
            if let Some(snapshot_id) = &pending.snapshot_id {
                object.insert("snapshot_id".into(), Value::String(snapshot_id.clone()));
            }
            if let Some(artifact) = &artifact {
                object.insert("artifact".into(), serde_json::to_value(artifact).unwrap());
                match pending.operation.as_str() {
                    "capture_browser_evidence" => {
                        object.remove("screenshot");
                        object.insert(
                            "evidence".into(),
                            json!({
                                "request_id": artifact.request_id,
                                "target": artifact.target,
                                "media_type": artifact.media_type,
                                "reference": artifact.path,
                                "page_context_revision": artifact.page_context_revision,
                                "artifact": artifact,
                            }),
                        );
                    }
                    "generate_browser_pdf" => {
                        object.remove("artifact_data");
                    }
                    _ => {}
                }
            }
            if let Some(capture) = console_capture {
                object.insert("capture".into(), capture);
            }
            if let Some(capture) = stopped_console_capture {
                object.insert("capture".into(), capture);
            }
            if let Some(capture) = network_capture {
                object.insert("capture".into(), capture);
            }
            if let Some(capture) = network_transition {
                object.insert("capture".into(), capture);
            }
            if let Some(Value::Object(body)) = network_body {
                object.remove("body");
                object.remove("base64_encoded");
                object.remove("truncated");
                object.remove("original_size");
                object.remove("returned_size");
                for (key, value) in body {
                    object.insert(key, value);
                }
            }
        }
        if response.ok {
            if pending.operation == "get_page_snapshot" {
                if let Some(session) = self.sessions.get_mut(&pending.extension_instance_id) {
                    let _ = session.cache_snapshot_references(
                        pending.target.clone(),
                        &mut value,
                        &request_id,
                        &pending.project_root,
                        &pending.caller_label,
                        Instant::now(),
                    );
                }
            } else if matches!(pending.operation.as_str(), "navigate" | "close_tab")
                && let Some(session) = self.sessions.get_mut(&pending.extension_instance_id)
            {
                session.clear_element_references(Some(&pending.target));
            }
        }
        let result = action_metadata.as_ref().map_or(Ok(()), |metadata| {
            BrowserActionCoordinator::finalize_response(
                &mut value,
                &response,
                &request_id,
                &pending.target,
                metadata,
            )
        });
        match result {
            Ok(()) => {
                let _ = pending.reply.send(Ok(value));
            }
            Err(error) => {
                let _ = pending.reply.send(Err(error));
            }
        }
        json!({"ok": true, "request_id": request_id})
    }

    fn handle_network_batch(&mut self, batch: NetworkBatch, generation: u64) -> Value {
        let target = batch.target.clone();
        let capture_id = batch.capture_id.clone();
        if let Err(error) = batch.validate() {
            let reason = match error.code {
                BrokerErrorCode::BrowserResourceLimit => "batch_too_large",
                BrokerErrorCode::MismatchedBrowserResponse => "target_mismatch",
                _ => "invalid_events",
            };
            return json!({
                "type": "network_ack",
                "capture_id": capture_id,
                "target": target,
                "ack_seq": 0,
                "acknowledged_sequence": 0,
                "accepted": false,
                "reason": reason,
            });
        }
        let current_generation = self
            .sessions
            .get(&batch.extension_instance_id)
            .and_then(|session| session.current_stream_generation());
        if current_generation != Some(generation) {
            return json!({
                "type": "network_ack",
                "capture_id": capture_id,
                "target": target,
                "ack_seq": 0,
                "acknowledged_sequence": 0,
                "accepted": false,
                "reason": "stream_generation_mismatch",
            });
        }
        self.evidence
            .accept_network_batch(&batch.extension_instance_id, generation, &batch)
    }

    fn handle_subscription(
        &mut self,
        owner_id: &str,
        extension_instance_id: &str,
        requested_target: Option<&BrowserTarget>,
        request_id: &str,
    ) -> Result<Value, BrokerError> {
        let now = Instant::now();
        if requested_target
            .is_some_and(|target| target.extension_instance_id != extension_instance_id)
        {
            return Err(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "subscription target does not match extension_instance_id",
            ));
        }
        let target = if let Some(target) = requested_target {
            self.sessions.resolve_target(Some(target), now).and_then(
                |(resolved_instance_id, target, _)| {
                    if resolved_instance_id != extension_instance_id {
                        Err(BrokerError::new(
                            BrokerErrorCode::MismatchedBrowserResponse,
                            "subscription target does not match extension_instance_id",
                        ))
                    } else {
                        Ok(target)
                    }
                },
            )?
        } else {
            self.sessions
                .require_live(extension_instance_id, now)?
                .current_active_target()
                .ok_or_else(|| {
                    BrokerError::new(
                        BrokerErrorCode::BrowserTargetNotFound,
                        "browser session has no active debuggable target",
                    )
                })?
        };
        self.sessions
            .subscribe_target_for_owner(owner_id.to_owned(), target.clone(), now)?;
        Ok(json!({
            "type": "response",
            "schema_version": BROWSER_BROKER_SCHEMA_VERSION,
            "protocol_version": BROWSER_BROKER_PROTOCOL_VERSION,
            "request_id": request_id,
            "operation": "subscribe_browser_session",
            "ok": true,
            "target": target,
        }))
    }

    async fn handle_operation(
        &mut self,
        request: OperationRequest,
        reply: oneshot::Sender<Result<Value, BrokerError>>,
        runtime: &BrokerRuntime,
    ) {
        // The WebSocket transport validates the envelope before queueing it, but
        // BrokerEvent is also an internal/public typed boundary used by the
        // shared entry points. Re-check here so a caller cannot bypass the
        // operation allowlist or version gate by constructing an event directly.
        if let Err(error) = request.validate() {
            let _ = reply.send(Err(error));
            return;
        }
        if self.pending.contains_key(&request.request_id)
            || self.retired_requests.contains_key(&request.request_id)
        {
            let _ = reply.send(Err(BrokerError::new(
                if is_mutating_operation(&request.operation) {
                    BrokerErrorCode::DuplicateBrowserMutation
                } else {
                    BrokerErrorCode::InvalidBrowserOperation
                },
                if self.pending.contains_key(&request.request_id) {
                    "duplicate browser request_id"
                } else {
                    "request_id is still reserved after a terminal browser request; use a new id"
                },
            )));
            return;
        }
        if self.pending.len() >= MAX_PENDING_REQUESTS {
            let _ = reply.send(Err(BrokerError::new(
                BrokerErrorCode::BrowserSessionBusy,
                "broker has too many pending browser requests; retry later",
            )));
            return;
        }

        let immediate = match request.operation.as_str() {
            "list_browser_sessions" => Some(self.list_sessions_response(runtime)),
            "list_browser_tabs" => Some(self.list_tabs_response(&request)),
            "lookup_browser_sessions" => Some(self.lookup_sessions_response(&request)),
            "set_browser_profile_label" => Some(self.set_profile_label_response(&request)),
            "clear_browser_profile_label" => Some(self.clear_profile_label_response(&request)),
            "acquire_browser_lease" => Some(self.acquire_lease_response(&request, runtime)),
            "renew_browser_lease" => Some(self.renew_lease_response(&request, runtime)),
            "release_browser_lease" => Some(self.release_lease_response(&request, runtime)),
            "create_browser_capability_grant" => {
                Some(self.create_capability_grant_response(&request, runtime))
            }
            "list_browser_capability_grants" => {
                Some(self.list_capability_grants_response(&request))
            }
            "revoke_browser_capability_grant" => {
                Some(self.revoke_capability_grant_response(&request))
            }
            "expire_browser_capability_grants" => Some(self.expire_capability_grants_response()),
            "list_browser_privileged_audit" => Some(self.list_privileged_audit_response(&request)),
            "cancel_browser_request" => Some(self.cancel_request_response(&request)),
            "cleanup_browser_artifacts" => Some(self.cleanup_browser_artifacts_response(&request)),
            _ => None,
        };
        if let Some(result) = immediate {
            let result = result.map(|payload| operation_success(&request, payload));
            self.retire_request(&request.request_id, Instant::now());
            let _ = reply.send(result);
            return;
        }
        self.forward_operation(request, reply, runtime).await;
    }

    fn cleanup_browser_artifacts_response(
        &mut self,
        request: &OperationRequest,
    ) -> Result<Value, BrokerError> {
        let project = required_project_context(request)?;
        let caller = required_caller_context(request)?;
        let paths = request
            .arguments
            .get("paths")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::InvalidBrowserOperation,
                    "paths is required for browser artifact cleanup",
                )
            })?
            .iter()
            .map(|value| {
                value.as_str().map(str::to_owned).ok_or_else(|| {
                    BrokerError::new(
                        BrokerErrorCode::InvalidBrowserOperation,
                        "browser artifact cleanup paths must be strings",
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.evidence.cleanup_managed(&project, &caller, &paths)
    }

    fn create_capability_grant_response(
        &mut self,
        request: &OperationRequest,
        runtime: &BrokerRuntime,
    ) -> Result<Value, BrokerError> {
        let now = Instant::now();
        let (instance_id, _target, _) =
            self.sessions.resolve_target(request.target.as_ref(), now)?;
        let project = required_project_context(request)?;
        let caller = required_caller_context(request)?;
        let lease_token = request_lease_token(request)?;
        let generation = runtime.endpoint_record().broker_start_id;
        self.validate_lease(&instance_id, &lease_token, &project, &caller, &generation)?;
        let capability = Capability::parse(&argument_string(&request.arguments, "capability")?)?;
        let grant = self.authorization.issue(
            capability,
            &instance_id,
            &project,
            &caller,
            &generation,
            request.arguments.get("ttl_secs").and_then(Value::as_u64),
            request
                .arguments
                .get("interactive_confirmed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            request
                .arguments
                .get("non_interactive")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            request
                .arguments
                .get("acknowledged_capability")
                .and_then(Value::as_str),
            &authorization::load_project_policy(&project),
        )?;
        Ok(json!({"grant": grant}))
    }

    fn list_capability_grants_response(
        &mut self,
        request: &OperationRequest,
    ) -> Result<Value, BrokerError> {
        let project = required_project_context(request)?;
        let extension_instance_id =
            argument_string_optional(&request.arguments, "extension_instance_id");
        let grants = self
            .authorization
            .list(&project, extension_instance_id.as_deref());
        Ok(json!({"grants": grants}))
    }

    fn revoke_capability_grant_response(
        &mut self,
        request: &OperationRequest,
    ) -> Result<Value, BrokerError> {
        let project = required_project_context(request)?;
        let grant_id = argument_string(&request.arguments, "grant_id")?;
        self.authorization.revoke(&grant_id, &project)
    }

    fn expire_capability_grants_response(&mut self) -> Result<Value, BrokerError> {
        Ok(json!({
            "expired": self.authorization.expire(Instant::now()),
        }))
    }

    fn list_privileged_audit_response(
        &self,
        request: &OperationRequest,
    ) -> Result<Value, BrokerError> {
        let project = required_project_context(request)?;
        let caller = required_caller_context(request)?;
        let limit = request.arguments.get("limit").and_then(Value::as_u64);
        Ok(json!({
            "records": self
                .authorization
                .list_privileged_audit(&project, &caller, limit),
        }))
    }

    fn record_privileged_audit(
        &mut self,
        request: &OperationRequest,
        project: &str,
        caller: &str,
        outcome: &str,
        arguments: Option<&Value>,
    ) {
        let include_cookie_values = request.operation == "list_browser_cookies"
            && request
                .arguments
                .get("include_values")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        let Some(capability) = authorization::audit_capability_for_operation(
            &request.operation,
            include_cookie_values,
        ) else {
            return;
        };
        let audit_arguments = arguments.cloned().unwrap_or_else(|| {
            authorization::audit_arguments_for_operation(&request.operation, &request.arguments)
        });
        let target = request
            .target
            .as_ref()
            .and_then(|target| serde_json::to_value(target).ok())
            .unwrap_or(Value::Null);
        self.authorization.append_privileged_audit(
            capability,
            project,
            caller,
            target,
            &request.request_id,
            outcome,
            &audit_arguments,
        );
    }

    fn local_console_operation(
        &mut self,
        operation: &str,
        target: &BrowserTarget,
        broker_start_id: &str,
        project: &str,
        caller: &str,
        arguments: &BTreeMap<String, Value>,
    ) -> Result<Value, BrokerError> {
        if !self
            .evidence
            .console_capture_scope_matches(target, broker_start_id, project, caller)
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserLease,
                "console capture scope does not match this project and caller",
            ));
        }
        match operation {
            "list_console_events" => self.evidence.list_console_events(
                target,
                arguments.get("levels"),
                arguments.get("max_age_ms"),
                arguments.get("max_entries"),
                arguments.get("max_bytes"),
            ),
            "clear_console_capture" => self.evidence.clear_console_capture(target),
            _ => Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "unsupported local console operation",
            )),
        }
    }

    fn local_network_operation(
        &mut self,
        operation: &str,
        target: &BrowserTarget,
        broker_start_id: &str,
        project: &str,
        caller: &str,
        arguments: &BTreeMap<String, Value>,
    ) -> Result<Value, BrokerError> {
        match operation {
            "list_network_requests" => self.evidence.list_network_requests(
                target,
                broker_start_id,
                project,
                caller,
                arguments.get("max_age_ms"),
                arguments.get("max_entries"),
                arguments.get("max_bytes"),
            ),
            "get_network_request_detail" => self.evidence.get_network_request_detail(
                target,
                broker_start_id,
                project,
                caller,
                arguments.get("network_request_id").ok_or_else(|| {
                    BrokerError::new(
                        BrokerErrorCode::InvalidBrowserOperation,
                        "network_request_id is required",
                    )
                })?,
                arguments
                    .get("include_body")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ),
            _ => Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "unsupported local network operation",
            )),
        }
    }

    fn list_sessions_response(&mut self, runtime: &BrokerRuntime) -> Result<Value, BrokerError> {
        let now = Instant::now();
        let mut sessions = self.sessions.list_public(now);
        for session in &mut sessions {
            let Some(instance_id) = session
                .pointer("/identity/extension_instance_id")
                .and_then(Value::as_str)
            else {
                continue;
            };
            if let Some(lease) = self.active_lease(instance_id, runtime)
                && let Value::Object(object) = session
            {
                object.insert(
                    "lease".into(),
                    json!({
                        "owner_label": lease.owner_label.clone(),
                        "expires_at_ms": lease.expires_at_ms,
                    }),
                );
            }
        }
        Ok(json!({"sessions": sessions}))
    }

    fn list_tabs_response(&mut self, request: &OperationRequest) -> Result<Value, BrokerError> {
        let instance_id = argument_string(&request.arguments, "extension_instance_id")?;
        self.sessions.list_tabs(&instance_id, Instant::now())
    }

    fn lookup_sessions_response(
        &mut self,
        request: &OperationRequest,
    ) -> Result<Value, BrokerError> {
        let now = Instant::now();
        let extension_instance_id =
            argument_string_optional(&request.arguments, "extension_instance_id");
        let profile_label = argument_string_optional(&request.arguments, "profile_label");
        let browser_name = argument_string_optional(&request.arguments, "browser_name");
        let tab_id = request.arguments.get("tab_id").and_then(Value::as_i64);
        let sessions = self
            .sessions
            .list_public(now)
            .into_iter()
            .filter(|session| {
                extension_instance_id.as_deref().is_none_or(|value| {
                    session
                        .pointer("/identity/extension_instance_id")
                        .and_then(Value::as_str)
                        == Some(value)
                })
            })
            .filter(|session| {
                profile_label.as_deref().is_none_or(|value| {
                    session
                        .pointer("/identity/profile_label")
                        .and_then(Value::as_str)
                        == Some(value)
                })
            })
            .filter(|session| {
                browser_name.as_deref().is_none_or(|value| {
                    session
                        .pointer("/browser/name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.eq_ignore_ascii_case(value))
                })
            })
            .filter(|session| {
                tab_id.is_none_or(|wanted| {
                    session
                        .pointer("/windows")
                        .and_then(Value::as_array)
                        .is_some_and(|windows| {
                            windows.iter().any(|window| {
                                window
                                    .get("tabs")
                                    .and_then(Value::as_array)
                                    .is_some_and(|tabs| {
                                        tabs.iter().any(|tab| {
                                            tab.get("id").and_then(Value::as_i64) == Some(wanted)
                                        })
                                    })
                            })
                        })
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({"sessions": sessions}))
    }

    fn set_profile_label_response(
        &mut self,
        request: &OperationRequest,
    ) -> Result<Value, BrokerError> {
        let instance_id = argument_string(&request.arguments, "extension_instance_id")?;
        let label = argument_string(&request.arguments, "profile_label")?;
        let normalized = self
            .sessions
            .set_profile_label(&instance_id, &label, Instant::now())?;
        Ok(json!({
            "extension_instance_id": instance_id,
            "profile_label": normalized
        }))
    }

    fn clear_profile_label_response(
        &mut self,
        request: &OperationRequest,
    ) -> Result<Value, BrokerError> {
        let instance_id = argument_string(&request.arguments, "extension_instance_id")?;
        self.sessions
            .clear_profile_label(&instance_id, Instant::now())?;
        Ok(json!({"extension_instance_id": instance_id, "profile_label": null}))
    }

    fn acquire_lease_response(
        &mut self,
        request: &OperationRequest,
        runtime: &BrokerRuntime,
    ) -> Result<Value, BrokerError> {
        let instance_id = argument_string(&request.arguments, "extension_instance_id")?;
        self.sessions.require_live(&instance_id, Instant::now())?;
        if let Some(existing) = self.active_lease(&instance_id, runtime) {
            let mut error = BrokerError::new(
                BrokerErrorCode::BrowserSessionBusy,
                "browser session is already leased by another caller",
            );
            error.recovery.insert(
                "owner_label".into(),
                Value::String(existing.owner_label.clone()),
            );
            error
                .recovery
                .insert("expires_at_ms".into(), Value::from(existing.expires_at_ms));
            return Err(error);
        }
        let now = Instant::now();
        let ttl_secs = bounded_ttl(request.arguments.get("ttl_secs"));
        let acquired_at_ms = unix_ms();
        let expires_at_ms = acquired_at_ms.saturating_add(ttl_secs.saturating_mul(1000));
        let token = format!(
            "lease_{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        let record = LeaseRecord {
            token: token.clone(),
            owner_label: argument_string(&request.arguments, "owner_label")?,
            project_root: required_project_context(request)?,
            caller_label: required_caller_context(request)?,
            broker_start_id: runtime.endpoint_record().broker_start_id,
            acquired_at_ms,
            expires_at_ms,
            expires_at: now + Duration::from_secs(ttl_secs),
        };
        self.leases.insert(instance_id.clone(), record);
        Ok(json!({
            "extension_instance_id": instance_id,
            "lease_token": token,
            "owner_label": self.leases[&instance_id].owner_label,
            "acquired_at_ms": acquired_at_ms,
            "expires_at_ms": expires_at_ms,
        }))
    }

    fn renew_lease_response(
        &mut self,
        request: &OperationRequest,
        runtime: &BrokerRuntime,
    ) -> Result<Value, BrokerError> {
        let instance_id = argument_string(&request.arguments, "extension_instance_id")?;
        let token = request_lease_token(request)?;
        let ttl_secs = bounded_ttl(request.arguments.get("ttl_secs"));
        self.sessions.require_live(&instance_id, Instant::now())?;
        let project = required_project_context(request)?;
        let caller = required_caller_context(request)?;
        let generation = runtime.endpoint_record().broker_start_id;
        let record = self.validate_lease(&instance_id, &token, &project, &caller, &generation)?;
        let now = Instant::now();
        let acquired_at_ms = record.acquired_at_ms;
        let owner_label = record.owner_label.clone();
        let expires_at_ms = unix_ms().saturating_add(ttl_secs.saturating_mul(1000));
        record.expires_at = now + Duration::from_secs(ttl_secs);
        record.expires_at_ms = expires_at_ms;
        Ok(json!({
            "extension_instance_id": instance_id,
            "lease_token": token,
            "owner_label": owner_label,
            "acquired_at_ms": acquired_at_ms,
            "expires_at_ms": expires_at_ms,
        }))
    }

    fn release_lease_response(
        &mut self,
        request: &OperationRequest,
        runtime: &BrokerRuntime,
    ) -> Result<Value, BrokerError> {
        let instance_id = argument_string(&request.arguments, "extension_instance_id")?;
        let token = request_lease_token(request)?;
        self.sessions.require_live(&instance_id, Instant::now())?;
        let project = required_project_context(request)?;
        let caller = required_caller_context(request)?;
        let generation = runtime.endpoint_record().broker_start_id;
        self.validate_lease(&instance_id, &token, &project, &caller, &generation)?;
        self.queue_console_cleanup(&instance_id);
        self.queue_network_cleanup(&instance_id);
        self.evidence
            .terminate_console_session(&instance_id, "lease_released", None);
        self.terminate_network_session(&instance_id, "lease_released", None);
        self.leases.remove(&instance_id);
        self.sessions.clear_element_references(&instance_id);
        Ok(json!({"extension_instance_id": instance_id, "released": true}))
    }

    fn cancel_request_response(
        &mut self,
        request: &OperationRequest,
    ) -> Result<Value, BrokerError> {
        let cancelled_request_id = argument_string(&request.arguments, "cancel_request_id")?;
        let project = project_context(request)?;
        let caller = caller_context(request);
        let Some(pending) = self.pending.get(&cancelled_request_id) else {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserRequestNotFound,
                "no pending browser request matches cancel_request_id",
            ));
        };
        if pending.project_root != project || pending.caller_label != caller {
            return Err(BrokerError::new(
                BrokerErrorCode::BrowserRequestNotFound,
                "no pending browser request matches cancel_request_id",
            ));
        }
        let cancellation = self.terminal_pending_error(
            &cancelled_request_id,
            pending,
            BrokerError::new(
                BrokerErrorCode::BrowserOperationCancelled,
                "browser operation was explicitly cancelled; any late response is ignored",
            ),
        );

        let pending = self
            .pending
            .remove(&cancelled_request_id)
            .expect("pending request was checked immediately above");
        self.abort_evidence(&cancelled_request_id, &pending.operation);
        self.retire_request(&cancelled_request_id, Instant::now());
        if let Some(session) = self.sessions.get_mut(&pending.extension_instance_id) {
            session.remove_queued_command(&cancelled_request_id);
        }
        let operation = pending.operation.clone();
        let _ = pending.reply.send(Err(cancellation));
        Ok(json!({
            "cancelled": true,
            "cancel_request_id": cancelled_request_id,
            "operation": operation,
        }))
    }

    async fn forward_operation(
        &mut self,
        request: OperationRequest,
        reply: oneshot::Sender<Result<Value, BrokerError>>,
        runtime: &BrokerRuntime,
    ) {
        let now = Instant::now();
        let lease_required = requires_lease(&request.operation);
        let locator_workflow = if request.operation == "resolve_playwright_locator" {
            Some(locator_workflow_from_request(&request))
        } else {
            None
        };
        let locator_workflow = match locator_workflow {
            Some(Ok(workflow)) => Some(workflow),
            Some(Err(error)) => {
                let _ = reply.send(Err(error));
                return;
            }
            None => None,
        };
        let (project, caller) = match request_scope(&request, lease_required) {
            Ok(scope) => scope,
            Err(error) => {
                let _ = reply.send(Err(error));
                return;
            }
        };
        let mut action_plan = None;
        let mut privileged_request = None;
        let mut upload_files = None;
        let result = self
            .sessions
            .resolve_target(request.target.as_ref(), now)
            .and_then(|(instance_id, target, _)| {
                let (session_supports_control, session_features, supported_operations,
                    optional_permissions) = {
                    let session = self.sessions.get(&instance_id).ok_or_else(|| {
                        BrokerError::new(
                            BrokerErrorCode::BrowserTargetNotFound,
                            "browser session was not found",
                        )
                    })?;
                    (
                        session.supports_feature("p0.control"),
                        session.features().to_vec(),
                        session.supported_operations().to_vec(),
                        session.optional_permissions().clone(),
                    )
                };
                if !runtime
                    .endpoint_record()
                    .broker_features
                    .iter()
                    .any(|feature| feature == "p0.control")
                    || !session_supports_control
                {
                    return Err(BrokerError::new(
                        BrokerErrorCode::BrowserCapabilityUnavailable,
                        "this broker generation provides transport only; browser control is not enabled",
                    ));
                }
                let required_features = required_features_for_operation(&request);
                for required_feature in required_features {
                    if !runtime
                        .endpoint_record()
                        .broker_features
                        .iter()
                        .any(|feature| feature == &required_feature)
                        || !session_features
                            .iter()
                            .any(|feature| feature.feature == required_feature && feature.available)
                    {
                        return Err(BrokerError::new(
                            BrokerErrorCode::BrowserCapabilityUnavailable,
                            format!(
                                "required browser feature is unavailable: {required_feature}"
                            ),
                        ));
                    }
                }
                if requires_extension_operation_advertisement(&request.operation)
                    && !supported_operations
                        .iter()
                        .any(|operation| operation == &request.operation)
                {
                    return Err(BrokerError::new(
                        BrokerErrorCode::BrowserCapabilityUnavailable,
                        format!(
                            "selected browser session does not advertise operation {}",
                            request.operation
                        ),
                    ));
                }
                let generation = runtime.endpoint_record().broker_start_id;
                if lease_required {
                    let token = request.lease_token.as_deref().ok_or_else(|| {
                        BrokerError::new(
                            BrokerErrorCode::InvalidBrowserLease,
                            "browser operation requires a valid lease_token",
                        )
                    })?;
                    self.validate_lease(&instance_id, token, &project, &caller, &generation)?;
                }
                let include_cookie_values = request.operation == "list_browser_cookies"
                    && request
                        .arguments
                        .get("include_values")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                if let Some(capability) = authorization::capability_for_operation(
                    &request.operation,
                    include_cookie_values,
                ) {
                    if let Some(permission) = capability.optional_permission() {
                        authorization::require_optional_permission(
                            optional_permissions
                                .get(permission)
                                .copied()
                                .unwrap_or(false),
                            permission,
                        )?;
                    }
                    let token = request
                        .arguments
                        .get("capability_grant_token")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            BrokerError::new(
                                BrokerErrorCode::BrowserCapabilityDenied,
                                "a valid capability grant is required",
                            )
                        })?;
                    self.authorization.validate(
                        token,
                        capability,
                        &instance_id,
                        &project,
                        &caller,
                        &generation,
                    )?;
                    if capability == Capability::Cookies && include_cookie_values {
                        let value_token = request
                            .arguments
                            .get("value_capability_grant_token")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                BrokerError::new(
                                    BrokerErrorCode::BrowserCapabilityDenied,
                                    "a cookie-values capability grant is required",
                                )
                            })?;
                        self.authorization.validate(
                            value_token,
                            Capability::CookieValues,
                            &instance_id,
                            &project,
                            &caller,
                            &generation,
                        )?;
                    }
                }
                if let Some(prepared) = authorization::prepare_privileged_request(
                    &request.operation,
                    &request.arguments,
                    &project,
                )? {
                    privileged_request = Some(prepared);
                }
                if request.operation == "execute_browser_action" {
                    upload_files = authorization::validate_upload_files(
                        &project,
                        request.arguments.get("action"),
                        request.arguments.get("files"),
                    )?;
                    let session = self.sessions.get_mut(&instance_id).ok_or_else(|| {
                        BrokerError::new(
                            BrokerErrorCode::BrowserTargetNotFound,
                            "browser session was not found",
                        )
                    })?;
                    let locator = resolve_execute_locator(
                        &request,
                        &target,
                        session,
                        &project,
                        &caller,
                        now,
                    )?;
                    action_plan = Some(BrowserActionCoordinator::plan(
                        &request,
                        &target,
                        &instance_id,
                        &locator,
                    )?);
                }
                Ok((instance_id, target))
            });

        let (instance_id, target) = match result {
            Ok(value) => value,
            Err(error) => {
                self.record_privileged_audit(
                    &request,
                    &project,
                    &caller,
                    error.code.as_str(),
                    None,
                );
                let _ = reply.send(Err(error));
                return;
            }
        };
        let broker_start_id = runtime.endpoint_record().broker_start_id;
        if matches!(
            request.operation.as_str(),
            "list_console_events" | "clear_console_capture"
        ) {
            let result = self.local_console_operation(
                &request.operation,
                &target,
                &broker_start_id,
                &project,
                &caller,
                &request.arguments,
            );
            self.retire_request(&request.request_id, Instant::now());
            let _ = reply.send(result.map(|payload| operation_success(&request, payload)));
            return;
        }
        let include_network_body = request.operation == "get_network_request_detail"
            && request
                .arguments
                .get("include_body")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        if request.operation == "list_network_requests"
            || (request.operation == "get_network_request_detail" && !include_network_body)
        {
            let result = self.local_network_operation(
                &request.operation,
                &target,
                &broker_start_id,
                &project,
                &caller,
                &request.arguments,
            );
            self.retire_request(&request.request_id, Instant::now());
            let _ = reply.send(result.map(|payload| operation_success(&request, payload)));
            return;
        }
        let network_body_access = if include_network_body {
            let request_id = match request.arguments.get("network_request_id") {
                Some(request_id) => request_id,
                None => {
                    let _ = reply.send(Err(BrokerError::new(
                        BrokerErrorCode::InvalidBrowserOperation,
                        "network_request_id is required",
                    )));
                    return;
                }
            };
            match self.evidence.prepare_network_body_access(
                &target,
                &broker_start_id,
                &project,
                &caller,
                request_id,
                request.arguments.get("max_body_bytes"),
            ) {
                Ok(access) => Some(access),
                Err(error) => {
                    let _ = reply.send(Err(error));
                    return;
                }
            }
        } else {
            None
        };
        let timeout = request
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_OPERATION_TIMEOUT)
            .clamp(Duration::from_millis(1), Duration::from_secs(300));
        let stream_generation = self
            .sessions
            .get(&instance_id)
            .and_then(|session| session.current_stream_generation());
        let prepared_console = if request.operation == "start_console_capture" {
            match self.evidence.prepare_console_capture(
                &request.request_id,
                &broker_start_id,
                &project,
                &caller,
                &target,
                stream_generation,
                request.arguments.get("levels"),
                request.arguments.get("max_age_ms"),
                request.arguments.get("max_entries"),
                request.arguments.get("max_bytes"),
                request.arguments.get("sensitive_fields"),
            ) {
                Ok(handle) => Some(handle),
                Err(error) => {
                    let _ = reply.send(Err(error));
                    return;
                }
            }
        } else {
            None
        };
        let prepared_network = if request.operation == "start_network_capture" {
            match self.evidence.prepare_network_capture(
                &request.request_id,
                &broker_start_id,
                &project,
                &caller,
                &target,
                stream_generation,
                request.arguments.get("allowed_hostnames"),
                request.arguments.get("capture_request_bodies"),
                request.arguments.get("max_request_body_bytes"),
                request.arguments.get("max_age_ms"),
                request.arguments.get("max_entries"),
                request.arguments.get("max_bytes"),
                request.arguments.get("max_body_bytes"),
                request.arguments.get("sensitive_fields"),
            ) {
                Ok(handle) => Some(handle),
                Err(error) => {
                    let _ = reply.send(Err(error));
                    return;
                }
            }
        } else {
            None
        };
        let capture_id = if request.operation == "start_console_capture" {
            prepared_console
                .as_ref()
                .map(|handle| handle.capture_id.clone())
        } else if request.operation == "stop_console_capture" {
            self.evidence.console_capture_id(&target)
        } else if request.operation == "start_network_capture" {
            prepared_network
                .as_ref()
                .map(|handle| handle.capture_id.clone())
        } else if request.operation == "stop_network_capture"
            || request.operation == "clear_network_capture"
        {
            self.evidence.network_capture_id(&target)
        } else {
            None
        };
        let command = match action_plan.as_ref() {
            Some(plan) => plan.command.clone(),
            None => match build_extension_command_with_capture_id(
                &request,
                &target,
                &instance_id,
                None,
                capture_id.as_deref(),
            ) {
                Ok(command) => command,
                Err(error) => {
                    if prepared_console.is_some() {
                        self.evidence.abort_console_capture(&request.request_id);
                    }
                    if prepared_network.is_some() {
                        self.evidence.abort_network_capture(&request.request_id);
                    }
                    let _ = reply.send(Err(error));
                    return;
                }
            },
        };
        let mut command = command;
        if let Some(files) = upload_files
            && let Value::Object(object) = &mut command
        {
            object.insert(
                "files".into(),
                Value::Array(files.into_iter().map(Value::String).collect()),
            );
        }
        if let Some(privileged) = privileged_request.as_ref()
            && let Value::Object(object) = &mut command
        {
            if let Some(method) = privileged.normalized_method() {
                object.insert("method".into(), Value::String(method.into()));
            }
            if let Some(setting) = privileged.normalized_setting() {
                object.insert("setting".into(), Value::String(setting.into()));
            }
            if let Some(max_entries) = privileged.max_entries() {
                object.insert("max_entries".into(), Value::from(max_entries));
            }
            if matches!(
                request.operation.as_str(),
                "execute_privileged_javascript" | "execute_privileged_cdp" | "list_browser_cookies"
            ) {
                object.insert(
                    "max_result_bytes".into(),
                    Value::from(privileged.max_result_bytes()),
                );
            }
            object.remove("source_kind");
        }
        if is_evidence_operation(&request.operation) {
            let expected_revision = request
                .arguments
                .get("page_context_revision")
                .and_then(Value::as_str);
            let requested_format = request.arguments.get("format").and_then(Value::as_str);
            let broker_start_id = runtime.endpoint_record().broker_start_id;
            if let Err(error) = self.evidence.prepare_request(
                &request.operation,
                &request.request_id,
                &broker_start_id,
                &project,
                &caller,
                &target,
                expected_revision,
                requested_format,
            ) {
                let _ = reply.send(Err(error));
                return;
            }
        }
        if let Some(privileged_request) = privileged_request {
            self.privileged_requests
                .insert(request.request_id.clone(), privileged_request);
        }
        if let Some(locator_workflow) = locator_workflow {
            self.locator_workflows
                .insert(request.request_id.clone(), locator_workflow);
        }
        self.pending.insert(
            request.request_id.clone(),
            PendingRequest {
                operation: request.operation.clone(),
                extension_instance_id: instance_id.clone(),
                target,
                project_root: project.clone(),
                caller_label: caller.clone(),
                lease_token: request.lease_token.clone(),
                snapshot_id: action_plan
                    .as_ref()
                    .and_then(|plan| plan.metadata.locator.snapshot_id.clone()),
                stream_generation,
                fallback_generation: None,
                console_capture_id: capture_id,
                deadline: now + timeout,
                reply,
            },
        );
        if let Some(access) = network_body_access {
            self.network_body_access
                .insert(request.request_id.clone(), access);
        }
        if let Some(plan) = action_plan {
            self.action_metadata
                .insert(request.request_id.clone(), plan.metadata);
        }
        if let Err(error) = runtime
            .send_extension_command_for_generation(&instance_id, stream_generation, command.clone())
            .await
        {
            // A replaced stream must never receive a command belonging to the
            // previous generation. A disconnected sender is safe to fall back
            // to heartbeat because try_send did not enqueue the command; only
            // an explicit generation mismatch means a newer stream is already
            // taking ownership of this session.
            if stream_generation.is_some()
                && error.code == BrokerErrorCode::IncompatibleBrowserSession
            {
                if let Some(pending) = self.pending.remove(&request.request_id) {
                    self.abort_evidence(&request.request_id, &pending.operation);
                    self.retire_request(&request.request_id, Instant::now());
                    let _ = pending.reply.send(Err(error));
                }
                return;
            }
            let queue_result = self
                .sessions
                .get_mut(&instance_id)
                .ok_or_else(|| {
                    BrokerError::new(
                        BrokerErrorCode::BrowserSessionDisconnected,
                        "browser extension session disconnected before dispatch",
                    )
                })
                .and_then(|session| session.restore_command_front(command));
            match queue_result {
                Ok(()) => {
                    // The heartbeat HTTP path has no stream generation. The
                    // command remains pending, but its eventual response is
                    // now correlated to that fallback transport rather than
                    // the failed direct WebSocket generation.
                    if let Some(pending) = self.pending.get_mut(&request.request_id) {
                        pending.stream_generation = None;
                        pending.fallback_generation = stream_generation;
                    }
                }
                Err(error) => {
                    if let Some(pending) = self.pending.remove(&request.request_id) {
                        self.abort_evidence(&request.request_id, &pending.operation);
                        self.retire_request(&request.request_id, Instant::now());
                        let _ = pending.reply.send(Err(error));
                    }
                }
            }
        } else if self.action_metadata.contains_key(&request.request_id) {
            self.dispatched_actions.insert(request.request_id.clone());
        }
    }

    fn active_lease<'a>(
        &'a mut self,
        extension_instance_id: &str,
        runtime: &BrokerRuntime,
    ) -> Option<&'a LeaseRecord> {
        if self
            .leases
            .get(extension_instance_id)
            .is_some_and(|lease| lease.expires_at <= Instant::now())
        {
            self.leases.remove(extension_instance_id);
            self.queue_console_cleanup(extension_instance_id);
            self.queue_network_cleanup(extension_instance_id);
            self.evidence
                .terminate_console_session(extension_instance_id, "lease_expired", None);
            self.terminate_network_session(extension_instance_id, "lease_expired", None);
            self.sessions
                .clear_element_references(extension_instance_id);
        }
        let generation = runtime.endpoint_record().broker_start_id;
        self.leases
            .get(extension_instance_id)
            .filter(|lease| lease.broker_start_id == generation)
    }

    fn validate_lease(
        &mut self,
        extension_instance_id: &str,
        token: &str,
        project_root: &str,
        caller_label: &str,
        broker_start_id: &str,
    ) -> Result<&mut LeaseRecord, BrokerError> {
        if self
            .leases
            .get(extension_instance_id)
            .is_some_and(|lease| lease.expires_at <= Instant::now())
        {
            self.leases.remove(extension_instance_id);
            self.queue_console_cleanup(extension_instance_id);
            self.queue_network_cleanup(extension_instance_id);
            self.evidence
                .terminate_console_session(extension_instance_id, "lease_expired", None);
            self.terminate_network_session(extension_instance_id, "lease_expired", None);
            self.sessions
                .clear_element_references(extension_instance_id);
            return Err(BrokerError::new(
                BrokerErrorCode::ExpiredBrowserLease,
                "browser session lease expired; acquire a new lease",
            ));
        }
        let lease = self.leases.get_mut(extension_instance_id).ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserLease,
                "a valid lease_token is required for this browser operation",
            )
        })?;
        if !constant_time_equal(&lease.token, token)
            || lease.project_root != project_root
            || lease.caller_label != caller_label
            || lease.broker_start_id != broker_start_id
        {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserLease,
                "browser lease scope does not match this request",
            ));
        }
        Ok(lease)
    }

    fn expire(&mut self, now: Instant) {
        self.sessions.expire_stale(now);
        let heartbeat_ttl = self.sessions.heartbeat_ttl();
        let mut capture_instances = self
            .evidence
            .console_capture_instance_ids()
            .into_iter()
            .collect::<HashSet<_>>();
        capture_instances.extend(self.evidence.network_capture_instance_ids());
        for instance_id in capture_instances {
            let alive = self
                .sessions
                .get(&instance_id)
                .is_some_and(|session| session.alive_at(now, heartbeat_ttl));
            if !alive {
                self.evidence
                    .terminate_console_session(&instance_id, "session_disconnected", None);
                self.terminate_network_session(&instance_id, "session_disconnected", None);
            }
        }
        self.retired_requests.retain(|_, retired_at| {
            now.saturating_duration_since(*retired_at) <= RETIRED_REQUEST_TTL
        });
        let expired_leases = self
            .leases
            .iter()
            .filter(|(_, lease)| lease.expires_at <= now)
            .map(|(instance_id, _)| instance_id.clone())
            .collect::<Vec<_>>();
        for instance_id in expired_leases {
            self.leases.remove(&instance_id);
            self.queue_console_cleanup(&instance_id);
            self.queue_network_cleanup(&instance_id);
            self.evidence
                .terminate_console_session(&instance_id, "lease_expired", None);
            self.terminate_network_session(&instance_id, "lease_expired", None);
            self.sessions.clear_element_references(&instance_id);
        }
        let expired = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in expired {
            if let Some(pending) = self.pending.remove(&request_id) {
                self.abort_evidence(&request_id, &pending.operation);
                let terminal = self.terminal_pending_error(
                    &request_id,
                    &pending,
                    BrokerError::new(
                        BrokerErrorCode::BrowserOperationTimeout,
                        "browser operation expired before a response arrived",
                    ),
                );
                self.retire_request(&request_id, now);
                if let Some(session) = self.sessions.get_mut(&pending.extension_instance_id) {
                    session.remove_queued_command(&request_id);
                }
                let _ = pending.reply.send(Err(terminal));
            }
        }
        let heartbeat_ttl = self.sessions.heartbeat_ttl();
        let disconnected = self
            .pending
            .iter()
            .filter(|(_, pending)| {
                self.sessions
                    .get(&pending.extension_instance_id)
                    .is_none_or(|session| !session.alive_at(now, heartbeat_ttl))
            })
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in disconnected {
            if let Some(pending) = self.pending.remove(&request_id) {
                self.abort_evidence(&request_id, &pending.operation);
                let terminal = self.terminal_pending_error(
                    &request_id,
                    &pending,
                    BrokerError::new(
                        BrokerErrorCode::BrowserSessionDisconnected,
                        "browser extension session disconnected while the operation was pending",
                    ),
                );
                self.retire_request(&request_id, now);
                if let Some(session) = self.sessions.get_mut(&pending.extension_instance_id) {
                    session.remove_queued_command(&request_id);
                }
                let _ = pending.reply.send(Err(terminal));
            }
        }
    }

    fn revoke_network_body_access_for_target(&mut self, target: &BrowserTarget) {
        let request_ids = self
            .network_body_access
            .keys()
            .filter(|request_id| {
                self.pending
                    .get(*request_id)
                    .is_some_and(|pending| &pending.target == target)
            })
            .cloned()
            .collect::<Vec<_>>();
        for request_id in request_ids {
            self.network_body_access.remove(&request_id);
        }
    }

    fn revoke_network_body_access_for_instance(&mut self, extension_instance_id: &str) {
        let request_ids = self
            .network_body_access
            .keys()
            .filter(|request_id| {
                self.pending
                    .get(*request_id)
                    .is_some_and(|pending| pending.extension_instance_id == extension_instance_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        for request_id in request_ids {
            self.network_body_access.remove(&request_id);
        }
    }

    fn terminate_network_session(
        &mut self,
        extension_instance_id: &str,
        reason: &str,
        detail: Option<&str>,
    ) -> usize {
        self.revoke_network_body_access_for_instance(extension_instance_id);
        self.evidence
            .terminate_network_session(extension_instance_id, reason, detail)
    }

    fn mark_action_dispatched(&mut self, request_id: &str) {
        if self.action_metadata.contains_key(request_id) {
            self.dispatched_actions.insert(request_id.to_owned());
        }
    }

    fn terminal_pending_error(
        &self,
        request_id: &str,
        pending: &PendingRequest,
        terminal: BrokerError,
    ) -> BrokerError {
        let Some(metadata) = self.action_metadata.get(request_id) else {
            return terminal;
        };
        if !metadata.may_have_side_effect || !self.dispatched_actions.contains(request_id) {
            return terminal;
        }
        BrowserActionCoordinator::unknown_execution_error(
            request_id,
            &pending.target,
            &pending.extension_instance_id,
            &terminal,
            metadata,
        )
    }

    fn retire_request(&mut self, request_id: &str, now: Instant) {
        self.action_metadata.remove(request_id);
        self.dispatched_actions.remove(request_id);
        self.network_body_access.remove(request_id);
        self.privileged_requests.remove(request_id);
        self.locator_workflows.remove(request_id);
        self.retired_requests.insert(request_id.to_owned(), now);
        while self.retired_requests.len() > MAX_RETIRED_REQUESTS {
            let oldest = self
                .retired_requests
                .iter()
                .min_by_key(|(_, retired_at)| **retired_at)
                .map(|(request_id, _)| request_id.to_owned());
            let Some(oldest) = oldest else {
                break;
            };
            self.retired_requests.remove(&oldest);
        }
    }

    fn fail_pending_for_session(&mut self, extension_instance_id: &str, error: BrokerError) {
        let request_ids = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.extension_instance_id == extension_instance_id)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in request_ids {
            if let Some(pending) = self.pending.remove(&request_id) {
                self.abort_evidence(&request_id, &pending.operation);
                let terminal = self.terminal_pending_error(&request_id, &pending, error.clone());
                self.retire_request(&request_id, Instant::now());
                if let Some(session) = self.sessions.get_mut(extension_instance_id) {
                    session.remove_queued_command(&request_id);
                }
                let _ = pending.reply.send(Err(terminal));
            }
        }
    }

    fn abort_evidence(&mut self, request_id: &str, operation: &str) {
        if is_evidence_operation(operation) {
            self.evidence.abort_request(request_id);
        }
        if operation == "start_console_capture" {
            self.evidence.abort_console_capture(request_id);
        }
        if operation == "start_network_capture" {
            self.evidence.abort_network_capture(request_id);
        }
    }

    fn queue_console_cleanup(&mut self, extension_instance_id: &str) {
        let captures = self
            .evidence
            .console_capture_handles_for_session(extension_instance_id);
        for (target, capture_id) in captures {
            self.queue_console_stop_command(extension_instance_id, &target, &capture_id);
        }
    }

    fn queue_console_stop_command(
        &mut self,
        extension_instance_id: &str,
        target: &BrowserTarget,
        capture_id: &str,
    ) {
        let Some(session) = self.sessions.get_mut(extension_instance_id) else {
            return;
        };
        let command = json!({
            "type": "cmd",
            "cmd": "__teshi_stop_console_capture",
            "schema_version": BROWSER_BROKER_SCHEMA_VERSION,
            "protocol_version": BROWSER_BROKER_PROTOCOL_VERSION,
            "request_id": format!("console_cleanup_{}", Uuid::new_v4().simple()),
            "target": target,
            "capture_id": capture_id,
            "suppress_response": true,
        });
        let _ = session.queue_command(command);
    }

    fn queue_network_stop_command(
        &mut self,
        extension_instance_id: &str,
        target: &BrowserTarget,
        capture_id: &str,
    ) {
        let Some(session) = self.sessions.get_mut(extension_instance_id) else {
            return;
        };
        let command = json!({
            "type": "cmd",
            "cmd": "stop_network_capture",
            "schema_version": BROWSER_BROKER_SCHEMA_VERSION,
            "protocol_version": BROWSER_BROKER_PROTOCOL_VERSION,
            "request_id": format!("network_cleanup_{}", Uuid::new_v4().simple()),
            "target": target,
            "capture_id": capture_id,
            "suppress_response": true,
        });
        let _ = session.queue_command(command);
    }

    fn queue_network_cleanup(&mut self, extension_instance_id: &str) {
        let captures = self
            .evidence
            .network_capture_handles_for_session(extension_instance_id);
        let Some(session) = self.sessions.get_mut(extension_instance_id) else {
            return;
        };
        for (target, capture_id) in captures {
            let command = json!({
                "type": "cmd",
                "cmd": "stop_network_capture",
                "schema_version": BROWSER_BROKER_SCHEMA_VERSION,
                "protocol_version": BROWSER_BROKER_PROTOCOL_VERSION,
                "request_id": format!("network_cleanup_{}", Uuid::new_v4().simple()),
                "target": target,
                "capture_id": capture_id,
                "suppress_response": true,
            });
            let _ = session.queue_command(command);
        }
    }

    fn quarantine(&mut self, response: &ExtensionResponse, reason: &str) {
        self.quarantined_responses.push(json!({
            "reason": reason,
            "request_id": response.request_id,
            "extension_instance_id": response.extension_instance_id,
            "target": response.target,
        }));
        if self.quarantined_responses.len() > MAX_QUARANTINED_RESPONSES {
            let drop_count = self.quarantined_responses.len() - MAX_QUARANTINED_RESPONSES;
            self.quarantined_responses.drain(..drop_count);
        }
    }
}

fn build_extension_command(
    request: &OperationRequest,
    target: &BrowserTarget,
    extension_instance_id: &str,
    resolved_locator: Option<&ExecuteLocatorCommand>,
) -> Result<Value, BrokerError> {
    if request.operation == "execute_browser_action"
        && let Some(locator) = resolved_locator
    {
        return BrowserActionCoordinator::build_command(
            request,
            target,
            extension_instance_id,
            locator,
        );
    }
    let mut object = request
        .arguments
        .clone()
        .into_iter()
        .collect::<Map<String, Value>>();
    object.insert("type".into(), Value::String("cmd".into()));
    object.insert(
        "cmd".into(),
        Value::String(extension_operation_for(&request.operation).into()),
    );
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
        Value::String(caller_context(request)),
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
    object.insert("target".into(), serde_json::to_value(target).unwrap());
    object.remove("lease_token");
    object.remove("capability_grant_token");
    object.remove("value_capability_grant_token");
    object.remove("source_kind");
    if request.operation == "execute_browser_action" {
        let fallback_locator;
        let locator = if let Some(locator) = resolved_locator {
            locator
        } else {
            fallback_locator = build_unvalidated_execute_locator(request)?;
            &fallback_locator
        };
        object.insert(
            "cmd".into(),
            Value::String(extension_operation_for(&request.operation).into()),
        );
        if let Some(selector) = &locator.selector {
            object.insert("selector".into(), Value::String(selector.clone()));
        } else {
            object.remove("selector");
        }
        if let Some(candidate) = &locator.candidate {
            object.insert("candidate".into(), serde_json::to_value(candidate).unwrap());
        } else {
            object.remove("candidate");
        }
        if let Some(locator_context) = &locator.locator_context {
            object.insert(
                "locator_context".into(),
                serde_json::to_value(locator_context).unwrap(),
            );
        } else {
            object.remove("locator_context");
        }
        object.insert("action".into(), Value::String(locator.action.clone()));
        object.insert(
            "page_context_revision".into(),
            Value::String(locator.page_context_revision.clone()),
        );
        if let Some(snapshot_id) = &locator.snapshot_id {
            object.insert("snapshot_id".into(), Value::String(snapshot_id.clone()));
        } else {
            object.remove("snapshot_id");
        }
        object.remove("element");
    }
    Ok(Value::Object(object))
}

/// Build the existing extension command envelope and inject only the
/// broker-owned capture ID for Console/Network lifecycle commands.  A
/// caller-supplied `capture_id` is never trusted for start/stop correlation.
fn build_extension_command_with_capture_id(
    request: &OperationRequest,
    target: &BrowserTarget,
    extension_instance_id: &str,
    resolved_locator: Option<&ExecuteLocatorCommand>,
    capture_id: Option<&str>,
) -> Result<Value, BrokerError> {
    let mut command =
        build_extension_command(request, target, extension_instance_id, resolved_locator)?;
    if (is_console_operation(&request.operation) || is_network_operation(&request.operation))
        && let Value::Object(object) = &mut command
    {
        object.remove("capture_id");
        if let Some(capture_id) = capture_id {
            object.insert("capture_id".into(), Value::String(capture_id.to_owned()));
        }
    }
    Ok(command)
}

fn build_unvalidated_execute_locator(
    request: &OperationRequest,
) -> Result<ExecuteLocatorCommand, BrokerError> {
    let dto = ExecuteLocatorActionRequest::from_operation(request)?;
    let action = dto.normalized_action()?;
    if !RUST_P0_EXECUTABLE_ACTIONS.contains(&action.as_str()) {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserCapabilityUnavailable,
            "Rust p0.control supports click, pointer_click, fill, and locator assertions",
        ));
    }
    let page_context_revision = dto.page_context_revision()?;
    let snapshot_id = dto.snapshot_id()?;
    let (selector, candidate) = match dto.input()? {
        ExecuteLocatorInput::Css(selector) => (Some(selector), None),
        ExecuteLocatorInput::TestId(value) => (None, Some(test_id_candidate(value))),
        ExecuteLocatorInput::RoleName { role, name } => {
            (None, Some(role_name_candidate(role, name)))
        }
        ExecuteLocatorInput::SnapshotReference(_) => {
            return Err(BrokerError::new(
                BrokerErrorCode::StaleElementReference,
                "Snapshot element reference must be resolved against its Profile before dispatch",
            ));
        }
    };
    Ok(ExecuteLocatorCommand {
        selector,
        candidate,
        locator_context: None,
        action,
        page_context_revision,
        snapshot_id,
    })
}

fn resolve_execute_locator(
    request: &OperationRequest,
    target: &BrowserTarget,
    session: &mut BrowserSessionRecord,
    project_root: &str,
    caller_label: &str,
    now: Instant,
) -> Result<ExecuteLocatorCommand, BrokerError> {
    let dto = ExecuteLocatorActionRequest::from_operation(request)?;
    let action = dto.normalized_action()?;
    if !RUST_P0_EXECUTABLE_ACTIONS.contains(&action.as_str()) {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserCapabilityUnavailable,
            "Rust p0.control supports click, pointer_click, fill, and locator assertions",
        ));
    }
    if !session
        .supported_actions()
        .iter()
        .any(|supported| supported == &action)
    {
        return Err(BrokerError::new(
            BrokerErrorCode::BrowserCapabilityUnavailable,
            format!(
                "selected browser session does not advertise action {}",
                action
            ),
        ));
    }
    let input = dto.input()?;
    let page_context_revision = dto.page_context_revision()?;
    let mut snapshot_id = dto.snapshot_id()?;
    let (selector, candidate, locator_context) = match input {
        ExecuteLocatorInput::Css(selector) => (Some(selector), None, None),
        ExecuteLocatorInput::TestId(value) => (None, Some(test_id_candidate(value)), None),
        ExecuteLocatorInput::RoleName { role, name } => {
            (None, Some(role_name_candidate(role, name)), None)
        }
        ExecuteLocatorInput::SnapshotReference(alias) => {
            let snapshot_id_value = snapshot_id.clone().ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::StaleElementReference,
                    "snapshot_id is required when executing a Snapshot element reference",
                )
            })?;
            let reference = session.resolve_element_reference(
                target,
                &alias,
                Some(&page_context_revision),
                Some(&snapshot_id_value),
                project_root,
                caller_label,
                now,
            )?;
            let (selector, candidate) = snapshot_locator(&reference.element)?;
            let locator_context = snapshot_locator_context(&reference.context)?.or_else(|| {
                candidate
                    .as_ref()
                    .and_then(|candidate| candidate.context.clone())
            });
            snapshot_id = Some(reference.snapshot_id);
            (selector, candidate, locator_context)
        }
    };
    let current_revision = session
        .current_page_context_revision(target)
        .ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::StaleBrowserTarget,
                "execute_locator requires a snapshot for the selected target",
            )
        })?;
    if current_revision != page_context_revision {
        return Err(BrokerError::new(
            BrokerErrorCode::StaleBrowserTarget,
            "page_context_revision is stale for the selected target",
        ));
    }
    if let Some(snapshot_id) = &snapshot_id
        && session.current_page_snapshot_id(target) != Some(snapshot_id.as_str())
    {
        return Err(BrokerError::new(
            BrokerErrorCode::StaleElementReference,
            "snapshot_id is stale for the selected target",
        ));
    }
    Ok(ExecuteLocatorCommand {
        selector,
        candidate,
        locator_context,
        action,
        page_context_revision,
        snapshot_id,
    })
}

fn test_id_candidate(value: String) -> ExecuteLocatorCandidate {
    ExecuteLocatorCandidate {
        kind: ExecuteLocatorCandidateKind::TestId,
        arguments: LocatorCandidateArguments {
            attribute: Some("data-testid".into()),
            value: Some(value),
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
    }
}

fn role_name_candidate(role: String, name: String) -> ExecuteLocatorCandidate {
    ExecuteLocatorCandidate {
        kind: ExecuteLocatorCandidateKind::Role,
        arguments: LocatorCandidateArguments {
            role: Some(role),
            name: Some(name),
            exact: Some(true),
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
    }
}

fn snapshot_locator(
    element: &Value,
) -> Result<(Option<String>, Option<ExecuteLocatorCandidate>), BrokerError> {
    let normalized = SnapshotElement::normalize(0, element).ok_or_else(|| {
        BrokerError::new(
            BrokerErrorCode::StaleElementReference,
            "Snapshot element reference does not contain a locator object",
        )
    })?;
    normalized.validate_context()?;
    if let Some(raw_candidate) = normalized.extra.get("candidate") {
        let candidate: ExecuteLocatorCandidate = serde_json::from_value(raw_candidate.clone())
            .map_err(|_| {
                BrokerError::new(
                    BrokerErrorCode::StaleElementReference,
                    "Snapshot element candidate is malformed",
                )
            })?;
        candidate.validate_context()?;
        if let (Some(element_context), Some(candidate_context)) =
            (normalized.context.as_ref(), candidate.context.as_ref())
            && element_context != candidate_context
        {
            return Err(BrokerError::new(
                BrokerErrorCode::StaleElementReference,
                "Snapshot element and locator candidate contexts do not match",
            ));
        }
        let selector = (candidate.kind == ExecuteLocatorCandidateKind::Css)
            .then(|| candidate.arguments.selector.clone())
            .flatten();
        return Ok((selector, Some(candidate)));
    }
    let resolution = LocatorSnapshot {
        snapshot_id: None,
        page_context_revision: None,
        url: String::new(),
        title: String::new(),
        interactive_elements: vec![normalized.clone()],
        extra: BTreeMap::new(),
    }
    .generate_candidates(
        &LocatorIntent {
            element_ref: Some(normalized.element_ref.clone()),
            ..Default::default()
        },
        &[],
    )
    .map_err(|error| match error.code {
        BrokerErrorCode::BrowserCapabilityUnavailable => error,
        _ => BrokerError::new(BrokerErrorCode::StaleElementReference, error.message),
    })?;
    let candidate = resolution.candidates.into_iter().next().ok_or_else(|| {
        BrokerError::new(
            BrokerErrorCode::StaleElementReference,
            "Snapshot element reference has no supported locator",
        )
    })?;
    let selector = (candidate.kind == ExecuteLocatorCandidateKind::Css)
        .then(|| candidate.arguments.selector.clone())
        .flatten();
    Ok((selector, Some(candidate)))
}

fn snapshot_locator_context(value: &Value) -> Result<Option<LocatorContext>, BrokerError> {
    if value.is_null() {
        return Ok(None);
    }
    let context: LocatorContext = serde_json::from_value(value.clone()).map_err(|_| {
        BrokerError::new(
            BrokerErrorCode::StaleElementReference,
            "Snapshot element reference has unsupported frame or shadow context",
        )
    })?;
    context.validate_supported()?;
    Ok(Some(context))
}

fn extension_operation_for(operation: &str) -> &str {
    match operation {
        "execute_browser_action" => "execute_locator",
        "resolve_playwright_locator" => "get_page_snapshot",
        "get_network_request_detail" => "get_network_response_body",
        _ => operation,
    }
}

fn locator_workflow_from_request(
    request: &OperationRequest,
) -> Result<LocatorWorkflow, BrokerError> {
    let intent = match request.arguments.get("intent") {
        None | Some(Value::Null) => LocatorIntent::default(),
        Some(value) => serde_json::from_value(value.clone()).map_err(|_| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "locator intent must be a JSON object",
            )
        })?,
    };
    let test_id_attributes = match request.arguments.get("test_id_attributes") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|value| value.trim().to_owned())
                    .ok_or_else(|| {
                        BrokerError::new(
                            BrokerErrorCode::InvalidBrowserOperation,
                            "locator test_id_attributes must contain only strings",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "locator test_id_attributes must be an array",
            ));
        }
    };
    Ok(LocatorWorkflow {
        intent,
        test_id_attributes,
        stage: LocatorWorkflowStage::Snapshot,
        candidates: Vec::new(),
        element: None,
        page_context_revision: None,
        url: String::new(),
        title: String::new(),
    })
}

fn build_locator_verification_command(
    request_id: &str,
    target: &BrowserTarget,
    extension_instance_id: &str,
    caller_label: &str,
    project_root: &str,
    page_context_revision: &str,
    candidates: &[ExecuteLocatorCandidate],
) -> Value {
    json!({
        "type": "cmd",
        "cmd": "verify_playwright_locators",
        "schema_version": BROWSER_BROKER_SCHEMA_VERSION,
        "protocol_version": BROWSER_BROKER_PROTOCOL_VERSION,
        "request_id": request_id,
        "caller_label": caller_label,
        "project_root": project_root,
        "extension_instance_id": extension_instance_id,
        "target": target,
        "page_context_revision": page_context_revision,
        "candidates": candidates,
    })
}

fn operation_success(request: &OperationRequest, payload: Value) -> Value {
    let mut object = Map::from_iter([
        ("type".into(), Value::String("response".into())),
        (
            "schema_version".into(),
            Value::from(BROWSER_BROKER_SCHEMA_VERSION),
        ),
        (
            "protocol_version".into(),
            Value::from(BROWSER_BROKER_PROTOCOL_VERSION),
        ),
        (
            "request_id".into(),
            Value::String(request.request_id.clone()),
        ),
        ("operation".into(), Value::String(request.operation.clone())),
        ("ok".into(), Value::Bool(true)),
    ]);
    if let Value::Object(fields) = payload {
        object.extend(fields);
    }
    Value::Object(object)
}

fn error_value(error: BrokerError) -> Value {
    json!({
        "ok": false,
        "code": error.code.as_str(),
        "error": error.message,
        "recovery": error.recovery,
    })
}

fn extension_response_error(response: &ExtensionResponse) -> BrokerError {
    let code = response
        .code
        .as_deref()
        .and_then(|code| serde_json::from_value::<BrokerErrorCode>(Value::String(code.into())).ok())
        .unwrap_or(BrokerErrorCode::BrowserOperationFailed);
    let message = response
        .error
        .as_deref()
        .unwrap_or("browser extension operation failed")
        .chars()
        .take(4096)
        .collect::<String>();
    BrokerError::new(code, message)
}

fn argument_string(arguments: &BTreeMap<String, Value>, name: &str) -> Result<String, BrokerError> {
    let value = arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                format!("{name} is required"),
            )
        })?;
    Ok(value.chars().take(4096).collect())
}

fn request_lease_token(request: &OperationRequest) -> Result<String, BrokerError> {
    request
        .lease_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(4096).collect())
        .ok_or_else(|| {
            BrokerError::new(
                BrokerErrorCode::InvalidBrowserOperation,
                "lease_token is required",
            )
        })
}

fn argument_string_optional(arguments: &BTreeMap<String, Value>, name: &str) -> Option<String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(4096).collect())
}

fn caller_context(request: &OperationRequest) -> String {
    let caller: String = request.caller_label.trim().chars().take(120).collect();
    if caller.is_empty() {
        "legacy-client".into()
    } else {
        caller
    }
}

fn project_context(request: &OperationRequest) -> Result<String, BrokerError> {
    let project = request
        .project_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(4096).collect::<String>())
        .unwrap_or_else(|| "legacy-project".into());
    Ok(project)
}

fn required_caller_context(request: &OperationRequest) -> Result<String, BrokerError> {
    let caller = caller_context(request);
    if caller == "legacy-client" {
        return Err(BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "caller_label is required for a lease-bound browser operation",
        ));
    }
    Ok(caller)
}

fn required_project_context(request: &OperationRequest) -> Result<String, BrokerError> {
    let project = request
        .project_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(4096).collect::<String>());
    project.ok_or_else(|| {
        BrokerError::new(
            BrokerErrorCode::InvalidBrowserOperation,
            "project_root is required for a lease-bound browser operation",
        )
    })
}

fn request_scope(
    request: &OperationRequest,
    lease_required: bool,
) -> Result<(String, String), BrokerError> {
    if lease_required {
        Ok((
            required_project_context(request)?,
            required_caller_context(request)?,
        ))
    } else {
        Ok((project_context(request)?, caller_context(request)))
    }
}

fn bounded_ttl(value: Option<&Value>) -> u64 {
    value
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_LEASE_TTL_SECS)
        .clamp(MIN_LEASE_TTL_SECS, MAX_LEASE_TTL_SECS)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

/// Resolve the generation attached by the extension to a console event.  A
/// WebSocket transport supplies the authoritative generation out of band;
/// the payload may repeat it for HTTP/diagnostic delivery, but never override
/// a conflicting transport value.  `Some(None)` is intentional for a legacy
/// heartbeat-only stream that has not yet attached a generation.
fn extension_event_generation(
    payload: &Value,
    transport_generation: Option<u64>,
) -> Option<Option<u64>> {
    match payload.get("stream_generation") {
        None | Some(Value::Null) => Some(transport_generation),
        Some(value) => {
            let supplied = value.as_u64()?;
            if transport_generation.is_some_and(|current| current != supplied) {
                None
            } else {
                Some(Some(supplied))
            }
        }
    }
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    left.as_bytes().ct_eq(right.as_bytes()).into()
}

fn requires_lease(operation: &str) -> bool {
    matches!(
        operation,
        "get_page_snapshot"
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
            | "execute_privileged_javascript"
            | "execute_privileged_cdp"
            | "list_browser_cookies"
            | "access_browser_content_setting"
            | "list_browser_extensions"
    )
}

fn required_features_for_operation(request: &OperationRequest) -> Vec<String> {
    let mut required = Vec::new();
    match request.operation.as_str() {
        "get_page_snapshot" | "navigate" | "execute_browser_action" => {
            required.push("p0.control".into());
        }
        "capture_browser_evidence"
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
        | "stop_network_capture" => {
            required.push("p1.observability_artifacts".into());
            if request.operation == "start_network_capture" {
                required.push("p1.filtered_network_capture".into());
                required.push("p1.network_batch_transport".into());
            }
        }
        "execute_privileged_javascript" => required.push("p2.javascript".into()),
        "execute_privileged_cdp" => required.push("p2.raw_cdp".into()),
        "list_browser_cookies" => required.push("p2.cookies".into()),
        "access_browser_content_setting" => required.push("p2.content_settings".into()),
        "list_browser_extensions" => required.push("p2.extension_management".into()),
        _ => {}
    }
    if let Some(feature) = request.required_feature.as_deref() {
        required.push(feature.to_owned());
    }
    required.sort();
    required.dedup();
    required
}

fn requires_extension_operation_advertisement(operation: &str) -> bool {
    matches!(
        operation,
        "capture_browser_evidence"
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
            | "execute_privileged_javascript"
            | "execute_privileged_cdp"
            | "list_browser_cookies"
            | "access_browser_content_setting"
            | "list_browser_extensions"
    )
}

fn is_evidence_operation(operation: &str) -> bool {
    matches!(
        operation,
        "capture_browser_evidence" | "capture_browser_screenshot" | "generate_browser_pdf"
    )
}

fn is_console_operation(operation: &str) -> bool {
    matches!(
        operation,
        "start_console_capture"
            | "list_console_events"
            | "clear_console_capture"
            | "stop_console_capture"
    )
}

fn is_network_operation(operation: &str) -> bool {
    matches!(
        operation,
        "start_network_capture"
            | "list_network_requests"
            | "get_network_request_detail"
            | "clear_network_capture"
            | "stop_network_capture"
    )
}

fn network_response_barrier(result: &BTreeMap<String, Value>) -> Option<u64> {
    ["sequence_barrier", "final_sequence", "last_seq"]
        .iter()
        .find_map(|field| result.get(*field).and_then(Value::as_u64))
}

fn is_mutating_operation(operation: &str) -> bool {
    matches!(
        operation,
        "get_page_snapshot"
            | "resolve_playwright_locator"
            | "verify_playwright_locator"
            | "capture_browser_evidence"
            | "execute_browser_action"
            | "navigate"
            | "activate_tab"
            | "open_tab"
            | "close_tab"
            | "create_window"
            | "group_tabs"
            | "start_console_capture"
            | "clear_console_capture"
            | "stop_console_capture"
            | "start_network_capture"
            | "clear_network_capture"
            | "stop_network_capture"
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::protocol::{
        ExtensionHeartbeat, ExtensionTab, ExtensionWindow, FeatureAvailability, NetworkBatch,
    };
    use base64::Engine as _;

    fn heartbeat() -> ExtensionHeartbeat {
        ExtensionHeartbeat {
            schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
            protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
            extension_instance_id: Some("profile-a".into()),
            profile_label: "Profile A".into(),
            extension_version: "test".into(),
            features: vec![FeatureAvailability {
                feature: "p0.control".into(),
                available: true,
                reason: None,
            }],
            supported_actions: vec!["click".into(), "pointer_click".into()],
            supported_operations: vec![],
            optional_permissions: BTreeMap::new(),
            browser: BTreeMap::new(),
            project_root: Some("/ignored/project".into()),
            url: "https://example.test".into(),
            title: "Example".into(),
            active_window_id: Some(7),
            active_tab_id: Some(42),
            tabs: vec![],
            windows: vec![ExtensionWindow {
                id: 7,
                focused: true,
                tabs: vec![ExtensionTab {
                    id: 42,
                    window_id: 7,
                    title: "Example".into(),
                    url: "https://example.test".into(),
                    active: true,
                    favicon_url: String::new(),
                    debuggable: true,
                }],
            }],
            frame_error: String::new(),
        }
    }

    fn target() -> BrowserTarget {
        BrowserTarget {
            extension_instance_id: "profile-a".into(),
            window_id: 7,
            tab_id: 42,
        }
    }

    fn target_for(instance_id: &str, window_id: i64, tab_id: i64) -> BrowserTarget {
        BrowserTarget {
            extension_instance_id: instance_id.into(),
            window_id,
            tab_id,
        }
    }

    fn extension_response(
        request_id: &str,
        operation: &str,
        target: BrowserTarget,
    ) -> ExtensionResponse {
        ExtensionResponse {
            message_type: "response".into(),
            schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
            protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
            request_id: request_id.into(),
            operation: operation.into(),
            extension_instance_id: Some(target.extension_instance_id.clone()),
            target: Some(target),
            ok: true,
            code: None,
            error: None,
            result: BTreeMap::new(),
        }
    }

    fn evidence_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes
    }

    fn evidence_response(
        request_id: &str,
        target: BrowserTarget,
        payload: &[u8],
    ) -> ExtensionResponse {
        let mut result = BTreeMap::new();
        result.insert("format".into(), Value::String("png".into()));
        result.insert(
            "page_context_revision".into(),
            Value::String("revision-1".into()),
        );
        result.insert(
            "artifact_data".into(),
            Value::String(base64::engine::general_purpose::STANDARD.encode(payload)),
        );
        ExtensionResponse {
            message_type: "response".into(),
            schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
            protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
            request_id: request_id.into(),
            operation: "capture_browser_screenshot".into(),
            extension_instance_id: Some(target.extension_instance_id.clone()),
            target: Some(target),
            ok: true,
            code: None,
            error: None,
            result,
        }
    }

    fn network_batch(target: BrowserTarget, capture_id: &str, events: Value) -> NetworkBatch {
        serde_json::from_value(json!({
            "type": "network_batch",
            "extension_instance_id": target.extension_instance_id,
            "capture_id": capture_id,
            "target": target,
            "events": events,
        }))
        .unwrap()
    }

    fn execute_request(action: &str, selector: &str, revision: &str) -> OperationRequest {
        serde_json::from_value(json!({
            "schema_version": 1,
            "request_id": format!("execute-{action}"),
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "execute_browser_action",
            "target": target(),
            "lease_token": "lease-secret",
            "action": action,
            "element": {
                "css": selector,
                "page_context_revision": revision
            }
        }))
        .unwrap()
    }

    fn execute_element_request(action: &str, element: Value) -> OperationRequest {
        serde_json::from_value(json!({
            "schema_version": 1,
            "request_id": format!("execute-{action}"),
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "execute_browser_action",
            "target": target(),
            "lease_token": "lease-secret",
            "action": action,
            "element": element
        }))
        .unwrap()
    }

    fn dispatched_action_metadata() -> ActionMetadata {
        ActionMetadata {
            action: "click".into(),
            locator: ExecuteLocatorCommand {
                selector: Some("#save".into()),
                candidate: None,
                locator_context: None,
                action: "click".into(),
                page_context_revision: "revision-1".into(),
                snapshot_id: None,
            },
            may_have_side_effect: true,
        }
    }

    #[test]
    fn execute_browser_action_maps_to_css_execute_locator_without_reference_fields() {
        let request = execute_request("pointer_click", "#pointer", "revision-1");
        let command = build_extension_command(&request, &target(), "profile-a", None).unwrap();

        assert_eq!(command["cmd"], "execute_locator");
        assert_eq!(command["action"], "pointer_click");
        assert_eq!(command["selector"], "#pointer");
        assert_eq!(command["page_context_revision"], "revision-1");
        assert_eq!(command["target"], json!(target()));
        assert!(command.get("element").is_none());
        assert!(command.get("lease_token").is_none());
    }

    #[test]
    fn locator_resolution_starts_with_snapshot_and_verifies_candidates_afterward() {
        let request: OperationRequest = serde_json::from_value(json!({
            "request_id": "locator-1",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "resolve_playwright_locator",
            "target": target(),
            "lease_token": "lease-secret",
            "intent": {"role": "textbox", "text": "Name"},
            "test_id_attributes": ["data-testid"]
        }))
        .unwrap();
        let command = build_extension_command(&request, &target(), "profile-a", None).unwrap();
        assert_eq!(command["cmd"], "get_page_snapshot");
        assert!(command.get("lease_token").is_none());

        let verification = build_locator_verification_command(
            "locator-1",
            &target(),
            "profile-a",
            "caller-a",
            "C:/project-a",
            "revision-1",
            &[],
        );
        assert_eq!(verification["cmd"], "verify_playwright_locators");
        assert_eq!(verification["request_id"], "locator-1");
        assert_eq!(verification["page_context_revision"], "revision-1");
        assert_eq!(verification["candidates"], json!([]));
        assert!(!verification.to_string().contains("lease-secret"));
    }

    #[test]
    fn privileged_grant_tokens_never_enter_extension_commands() {
        let request: OperationRequest = serde_json::from_value(json!({
            "request_id": "privileged-command",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "execute_privileged_javascript",
            "target": target(),
            "lease_token": "lease-secret",
            "capability_grant_token": "grant-secret",
            "expression": "document.title"
        }))
        .unwrap();
        let command = build_extension_command(&request, &target(), "profile-a", None).unwrap();
        assert_eq!(command["cmd"], "execute_privileged_javascript");
        assert!(command.get("capability_grant_token").is_none());
        assert!(command.get("value_capability_grant_token").is_none());
    }

    #[tokio::test]
    async fn privileged_response_is_redacted_and_recorded_as_metadata_only() {
        let project = tempfile::tempdir().unwrap();
        let project_root = project.path().to_string_lossy().into_owned();
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state.sessions.attach_stream("profile-a", 7, now).unwrap();
        let request_id = "cookies-state-1";
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            request_id.into(),
            PendingRequest {
                operation: "list_browser_cookies".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: project_root.clone(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: Some(7),
                fallback_generation: None,
                console_capture_id: None,
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );
        let request: OperationRequest = serde_json::from_value(json!({
            "request_id": request_id,
            "caller_label": "caller-a",
            "project_root": project_root,
            "cmd": "list_browser_cookies",
            "target": target(),
            "lease_token": "lease-secret",
            "include_values": false,
            "max_entries": 1
        }))
        .unwrap();
        let prepared = authorization::prepare_privileged_request(
            &request.operation,
            &request.arguments,
            request.project_root.as_deref().unwrap(),
        )
        .unwrap()
        .unwrap();
        state
            .privileged_requests
            .insert(request_id.into(), prepared);
        let mut response = extension_response(request_id, "list_browser_cookies", target());
        response.result.insert(
            "cookies".into(),
            json!([
                {"name": "sid", "value": "secret"},
                {"name": "other", "value": "hidden"}
            ]),
        );
        assert_eq!(
            state.handle_extension_response("profile-a", Some(7), response)["ok"],
            true
        );
        let completed = receiver.await.unwrap().unwrap();
        assert!(completed["cookies"][0].get("value").is_none());
        assert_eq!(completed["cookies"][0]["value_redacted"], true);
        let audit = state.authorization.list_privileged_audit(
            request.project_root.as_deref().unwrap(),
            "caller-a",
            None,
        );
        assert_eq!(audit.len(), 1);
        assert!(!serde_json::to_string(&audit).unwrap().contains("secret"));
    }

    #[test]
    fn typed_execute_locator_inputs_map_to_existing_extension_candidate_shape() {
        let test_id = execute_element_request(
            "click",
            json!({"test_id": "save-button", "page_context_revision": "revision-1"}),
        );
        let test_id_command =
            build_extension_command(&test_id, &target(), "profile-a", None).unwrap();
        assert_eq!(test_id_command["cmd"], "execute_locator");
        assert_eq!(test_id_command["candidate"]["kind"], "test_id");
        assert_eq!(
            test_id_command["candidate"]["arguments"]["attribute"],
            "data-testid"
        );
        assert_eq!(
            test_id_command["candidate"]["arguments"]["value"],
            "save-button"
        );
        assert!(test_id_command.get("selector").is_none());

        let role = execute_element_request(
            "pointer_click",
            json!({
                "role": "button",
                "name": "Save",
                "page_context_revision": "revision-1"
            }),
        );
        let role_command = build_extension_command(&role, &target(), "profile-a", None).unwrap();
        assert_eq!(role_command["cmd"], "execute_locator");
        assert_eq!(role_command["candidate"]["kind"], "role");
        assert_eq!(role_command["candidate"]["arguments"]["role"], "button");
        assert_eq!(role_command["candidate"]["arguments"]["name"], "Save");
        assert!(role_command.get("selector").is_none());
    }

    #[test]
    fn snapshot_reference_resolves_to_locator_and_enforces_scope_metadata() {
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        let mut snapshot = json!({
            "snapshot_id": "snapshot-1",
            "page_context_revision": "revision-1",
            "interactive_elements": [{
                "element_ref": "opaque-save",
                "testId": "save-button",
                "shortSelector": "#save",
                "context": {"frame": null, "shadow_root": null}
            }]
        });
        state
            .sessions
            .get_mut("profile-a")
            .unwrap()
            .cache_snapshot_references(
                target(),
                &mut snapshot,
                "snapshot-1",
                "C:/project-a",
                "caller-a",
                now,
            )
            .unwrap();

        let request = execute_element_request(
            "click",
            json!({
                "reference": "@e1",
                "snapshot_id": "snapshot-1",
                "page_context_revision": "revision-1"
            }),
        );
        let locator = resolve_execute_locator(
            &request,
            &target(),
            state.sessions.get_mut("profile-a").unwrap(),
            "C:/project-a",
            "caller-a",
            now,
        )
        .unwrap();
        assert_eq!(
            locator.candidate.as_ref().unwrap().kind,
            ExecuteLocatorCandidateKind::TestId
        );
        assert_eq!(locator.snapshot_id.as_deref(), Some("snapshot-1"));
        let command =
            build_extension_command(&request, &target(), "profile-a", Some(&locator)).unwrap();
        assert_eq!(command["cmd"], "execute_locator");
        assert_eq!(command["candidate"]["arguments"]["value"], "save-button");
        assert_eq!(command["snapshot_id"], "snapshot-1");
        assert!(command.get("element").is_none());

        let wrong_project = resolve_execute_locator(
            &request,
            &target(),
            state.sessions.get_mut("profile-a").unwrap(),
            "C:/project-b",
            "caller-a",
            now,
        )
        .unwrap_err();
        assert_eq!(wrong_project.code, BrokerErrorCode::StaleElementReference);

        let wrong_snapshot = execute_element_request(
            "click",
            json!({
                "reference": "@e1",
                "snapshot_id": "snapshot-old",
                "page_context_revision": "revision-1"
            }),
        );
        let wrong_snapshot = resolve_execute_locator(
            &wrong_snapshot,
            &target(),
            state.sessions.get_mut("profile-a").unwrap(),
            "C:/project-a",
            "caller-a",
            now,
        )
        .unwrap_err();
        assert_eq!(wrong_snapshot.code, BrokerErrorCode::StaleElementReference);
    }

    #[test]
    fn snapshot_reference_preserves_label_placeholder_attributes_and_context() {
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        let mut snapshot = json!({
            "snapshot_id": "snapshot-label",
            "page_context_revision": "revision-1",
            "interactive_elements": [{
                "element_ref": "opaque-label",
                "tag": "div",
                "label": "Email address",
                "placeholder": "name@example.test",
                "attributes": {
                    "id": "email",
                    "data-qa": "email-field"
                },
                "shortSelector": "#email",
                "context": {
                    "frame": "https://example.test/checkout-frame",
                    "shadow_root": "#checkout-widget"
                }
            }]
        });
        state
            .sessions
            .get_mut("profile-a")
            .unwrap()
            .cache_snapshot_references(
                target(),
                &mut snapshot,
                "snapshot-label",
                "C:/project-a",
                "caller-a",
                now,
            )
            .unwrap();

        let request = execute_element_request(
            "click",
            json!({
                "reference": "@e1",
                "snapshot_id": "snapshot-label",
                "page_context_revision": "revision-1"
            }),
        );
        let locator = resolve_execute_locator(
            &request,
            &target(),
            state.sessions.get_mut("profile-a").unwrap(),
            "C:/project-a",
            "caller-a",
            now,
        )
        .unwrap();
        assert_eq!(
            locator.candidate.as_ref().unwrap().kind,
            ExecuteLocatorCandidateKind::Label
        );
        assert_eq!(
            locator
                .candidate
                .as_ref()
                .unwrap()
                .arguments
                .text
                .as_deref(),
            Some("Email address")
        );
        assert_eq!(
            locator
                .locator_context
                .as_ref()
                .and_then(|context| context.frame.as_deref()),
            Some("https://example.test/checkout-frame")
        );
        assert_eq!(
            locator
                .locator_context
                .as_ref()
                .and_then(|context| context.shadow_root.as_deref()),
            Some("#checkout-widget")
        );
        let command =
            build_extension_command(&request, &target(), "profile-a", Some(&locator)).unwrap();
        assert_eq!(command["cmd"], "execute_locator");
        assert_eq!(command["candidate"]["kind"], "label");
        assert_eq!(command["candidate"]["arguments"]["text"], "Email address");
        assert_eq!(
            command["locator_context"]["frame"],
            "https://example.test/checkout-frame"
        );
        assert_eq!(
            command["locator_context"]["shadow_root"],
            "#checkout-widget"
        );
        assert!(command.get("element").is_none());
    }

    #[test]
    fn snapshot_context_with_unsupported_fields_fails_closed() {
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        let mut snapshot = json!({
            "snapshot_id": "snapshot-unsupported-context",
            "page_context_revision": "revision-1",
            "interactive_elements": [{
                "element_ref": "unsupported",
                "role": "button",
                "accessible_name": "Save",
                "context": {"frame": "checkout", "pierce_shadow": true}
            }]
        });
        let error = state
            .sessions
            .get_mut("profile-a")
            .unwrap()
            .cache_snapshot_references(
                target(),
                &mut snapshot,
                "snapshot-unsupported-context",
                "C:/project-a",
                "caller-a",
                now,
            )
            .unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserCapabilityUnavailable);
    }

    #[test]
    fn execute_browser_action_rejects_unsupported_candidates_and_actions() {
        let mut reference = execute_request("click", "#save", "revision-1");
        reference
            .arguments
            .insert("element".into(), json!({"reference": "@e1"}));
        assert_eq!(
            build_extension_command(&reference, &target(), "profile-a", None)
                .unwrap_err()
                .code,
            BrokerErrorCode::StaleBrowserTarget
        );

        let mut candidate = execute_request("click", "#save", "revision-1");
        candidate.arguments.insert(
            "element".into(),
            json!({
                "candidate": {"kind": "role", "arguments": {"role": "button", "name": "Save"}},
                "page_context_revision": "revision-1"
            }),
        );
        assert_eq!(
            build_extension_command(&candidate, &target(), "profile-a", None)
                .unwrap_err()
                .code,
            BrokerErrorCode::BrowserCapabilityUnavailable
        );

        let mut fill = execute_request("fill", "#save", "revision-1");
        fill.arguments.insert("value".into(), json!("Ada"));
        let fill_command = build_extension_command(&fill, &target(), "profile-a", None).unwrap();
        assert_eq!(fill_command["cmd"], "execute_locator");
        assert_eq!(fill_command["action"], "fill");
        assert_eq!(fill_command["value"], "Ada");

        let unsupported = execute_request("select", "#save", "revision-1");
        assert_eq!(
            build_extension_command(&unsupported, &target(), "profile-a", None)
                .unwrap_err()
                .code,
            BrokerErrorCode::BrowserCapabilityUnavailable
        );
    }

    #[test]
    fn execute_browser_action_requires_current_snapshot_revision_before_dispatch() {
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        let mut snapshot = json!({
            "snapshot_id": "snapshot-1",
            "page_context_revision": "revision-1",
            "interactive_elements": []
        });
        state
            .sessions
            .get_mut("profile-a")
            .unwrap()
            .cache_snapshot_references(
                target(),
                &mut snapshot,
                "snapshot-1",
                "C:/project-a",
                "caller-a",
                now,
            )
            .unwrap();

        assert!(
            resolve_execute_locator(
                &execute_request("click", "#save", "revision-1"),
                &target(),
                state.sessions.get_mut("profile-a").unwrap(),
                "C:/project-a",
                "caller-a",
                now,
            )
            .is_ok()
        );
        assert_eq!(
            resolve_execute_locator(
                &execute_request("click", "#save", "revision-old"),
                &target(),
                state.sessions.get_mut("profile-a").unwrap(),
                "C:/project-a",
                "caller-a",
                now,
            )
            .unwrap_err()
            .code,
            BrokerErrorCode::StaleBrowserTarget
        );
    }

    #[tokio::test]
    async fn execute_locator_response_maps_back_to_canonical_action_and_requires_exact_association()
    {
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state.sessions.attach_stream("profile-a", 7, now).unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "execute-click".into(),
            PendingRequest {
                operation: "execute_browser_action".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: Some(7),
                fallback_generation: None,
                console_capture_id: None,
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );

        let wrong_wire_operation =
            extension_response("execute-click", "execute_browser_action", target());
        assert_eq!(
            state.handle_extension_response("profile-a", Some(7), wrong_wire_operation)["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        assert!(state.pending.contains_key("execute-click"));

        let mut missing_profile = extension_response("execute-click", "execute_locator", target());
        missing_profile.extension_instance_id = None;
        assert_eq!(
            state.handle_extension_response("profile-a", Some(7), missing_profile)["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        assert!(state.pending.contains_key("execute-click"));

        assert_eq!(
            state.handle_extension_response(
                "profile-b",
                Some(7),
                extension_response("execute-click", "execute_locator", target()),
            )["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        assert!(state.pending.contains_key("execute-click"));

        let matching = extension_response("execute-click", "execute_locator", target());
        assert_eq!(
            state.handle_extension_response("profile-a", Some(7), matching)["ok"],
            true
        );
        let completed = receiver.await.unwrap().unwrap();
        assert_eq!(completed["request_id"], "execute-click");
        assert_eq!(completed["operation"], "execute_browser_action");
        assert_eq!(completed["cmd"], "execute_locator");
        assert_eq!(completed["extension_instance_id"], "profile-a");
        assert_eq!(completed["target"], json!(target()));
    }

    #[tokio::test]
    async fn screenshot_response_is_bound_to_lease_generation_and_managed_bytes() {
        let mut config = crate::server::BrokerServerConfig::with_trusted_extension_origins(vec![
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ]);
        config.discovery_addr = "127.0.0.1:0".parse().unwrap();
        let runtime = BrokerRuntime::start(config).await.unwrap();
        let broker_start_id = runtime.endpoint_record().broker_start_id;
        let project = tempfile::tempdir().unwrap();
        let project_root = project.path().to_string_lossy().into_owned();
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state.sessions.attach_stream("profile-a", 7, now).unwrap();
        state.leases.insert(
            "profile-a".into(),
            LeaseRecord {
                token: "lease-secret".into(),
                owner_label: "owner-a".into(),
                project_root: project_root.clone(),
                caller_label: "caller-a".into(),
                broker_start_id: broker_start_id.clone(),
                acquired_at_ms: 1,
                expires_at_ms: u64::MAX,
                expires_at: now + Duration::from_secs(60),
            },
        );
        state
            .evidence
            .prepare_request(
                "capture_browser_screenshot",
                "evidence-state-1",
                &broker_start_id,
                &project_root,
                "caller-a",
                &target(),
                Some("revision-1"),
                Some("png"),
            )
            .unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "evidence-state-1".into(),
            PendingRequest {
                operation: "capture_browser_screenshot".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: project_root.clone(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: Some(7),
                fallback_generation: None,
                console_capture_id: None,
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );

        let payload = evidence_png(2, 3);
        let response = evidence_response("evidence-state-1", target(), &payload);
        assert_eq!(
            state.handle_extension_response_with_broker_generation(
                "profile-a",
                Some(7),
                Some(&broker_start_id),
                response.clone(),
            )["ok"],
            true
        );
        let completed = receiver.await.unwrap().unwrap();
        assert_eq!(completed["artifact"]["size"], payload.len());
        assert_eq!(completed["artifact"]["dimensions"]["pixels"], 6);
        assert!(completed.get("artifact_data").is_some());
        let relative = completed["artifact"]["path"].as_str().unwrap();
        assert!(!relative.contains(['/', '\\']));
        assert_eq!(
            fs::read(
                project
                    .path()
                    .join(".teshi")
                    .join("artifacts")
                    .join("browser")
                    .join(relative)
            )
            .unwrap(),
            payload
        );

        let duplicate = state.handle_extension_response_with_broker_generation(
            "profile-a",
            Some(7),
            Some(&broker_start_id),
            response,
        );
        assert_eq!(
            duplicate["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        runtime.shutdown().await;
    }

    #[test]
    fn lease_required_operation_list_is_fail_closed() {
        assert!(requires_lease("navigate"));
        assert!(requires_lease("execute_privileged_javascript"));
        assert!(!requires_lease("list_browser_sessions"));
    }

    #[test]
    fn expired_pending_request_is_removed_from_heartbeat_fallback_queue() {
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state
            .sessions
            .get_mut("profile-a")
            .unwrap()
            .queue_command(json!({
                "request_id": "timed-out",
                "cmd": "navigate"
            }))
            .unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "timed-out".into(),
            PendingRequest {
                operation: "navigate".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: None,
                fallback_generation: None,
                console_capture_id: None,
                deadline: now - Duration::from_secs(1),
                reply,
            },
        );

        state.expire(now);

        assert!(!state.pending.contains_key("timed-out"));
        assert_eq!(
            state
                .sessions
                .get("profile-a")
                .unwrap()
                .queued_command_count(),
            0
        );
        assert_eq!(
            receiver.blocking_recv().unwrap().unwrap_err().code,
            BrokerErrorCode::BrowserOperationTimeout
        );
    }

    #[tokio::test]
    async fn explicit_cancel_cleans_pending_and_quarantines_late_response() {
        let mut config = crate::server::BrokerServerConfig::with_trusted_extension_origins(vec![
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ]);
        config.discovery_addr = "127.0.0.1:0".parse().unwrap();
        let runtime = BrokerRuntime::start(config).await.unwrap();
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state
            .sessions
            .get_mut("profile-a")
            .unwrap()
            .queue_command(json!({
                "type": "cmd",
                "cmd": "navigate",
                "request_id": "request-cancelled"
            }))
            .unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "request-cancelled".into(),
            PendingRequest {
                operation: "navigate".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: None,
                fallback_generation: None,
                console_capture_id: None,
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );

        let (cancel_reply, cancel_receiver) = oneshot::channel();
        let cancel: OperationRequest = serde_json::from_value(json!({
            "schema_version": 1,
            "request_id": "cancel-1",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "cancel_browser_request",
            "cancel_request_id": "request-cancelled"
        }))
        .unwrap();
        state.handle_operation(cancel, cancel_reply, &runtime).await;

        let cancellation = cancel_receiver.await.unwrap().unwrap();
        assert_eq!(cancellation["cancelled"], true);
        assert_eq!(cancellation["cancel_request_id"], "request-cancelled");
        assert_eq!(
            receiver.await.unwrap().unwrap_err().code,
            BrokerErrorCode::BrowserOperationCancelled
        );
        assert!(!state.pending.contains_key("request-cancelled"));
        assert_eq!(
            state
                .sessions
                .get("profile-a")
                .unwrap()
                .queued_command_count(),
            0
        );

        let late = extension_response("request-cancelled", "navigate", target());
        assert_eq!(
            state.handle_extension_response("profile-a", None, late)["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        assert_eq!(
            state.quarantined_responses.last().unwrap()["reason"],
            "late_response"
        );

        let (reuse_reply, reuse_receiver) = oneshot::channel();
        let reuse: OperationRequest = serde_json::from_value(json!({
            "request_id": "request-cancelled",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "execute_browser_action"
        }))
        .unwrap();
        state.handle_operation(reuse, reuse_reply, &runtime).await;
        assert_eq!(
            reuse_receiver.await.unwrap().unwrap_err().code,
            BrokerErrorCode::DuplicateBrowserMutation
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn response_wins_cancel_race_and_second_completion_is_quarantined() {
        let mut config = crate::server::BrokerServerConfig::with_trusted_extension_origins(vec![
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ]);
        config.discovery_addr = "127.0.0.1:0".parse().unwrap();
        let runtime = BrokerRuntime::start(config).await.unwrap();
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "request-race".into(),
            PendingRequest {
                operation: "navigate".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: None,
                fallback_generation: None,
                console_capture_id: None,
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );

        assert_eq!(
            state.handle_extension_response(
                "profile-a",
                None,
                extension_response("request-race", "navigate", target()),
            )["ok"],
            true
        );
        assert!(receiver.await.unwrap().is_ok());

        let (cancel_reply, cancel_receiver) = oneshot::channel();
        let cancel: OperationRequest = serde_json::from_value(json!({
            "request_id": "cancel-after-response",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "cancel_browser_request",
            "cancel_request_id": "request-race"
        }))
        .unwrap();
        state.handle_operation(cancel, cancel_reply, &runtime).await;
        assert_eq!(
            cancel_receiver.await.unwrap().unwrap_err().code,
            BrokerErrorCode::BrowserRequestNotFound
        );
        assert!(state.pending.is_empty());
        assert_eq!(state.quarantined_responses.len(), 0);

        assert_eq!(
            state.handle_extension_response(
                "profile-a",
                None,
                extension_response("request-race", "navigate", target()),
            )["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        assert_eq!(
            state.quarantined_responses.last().unwrap()["reason"],
            "late_response"
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn disconnect_fails_pending_clears_queue_and_rejects_old_generation_response() {
        let mut config = crate::server::BrokerServerConfig::with_trusted_extension_origins(vec![
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ]);
        config.discovery_addr = "127.0.0.1:0".parse().unwrap();
        let runtime = BrokerRuntime::start(config).await.unwrap();
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state.sessions.attach_stream("profile-a", 7, now).unwrap();
        state
            .sessions
            .get_mut("profile-a")
            .unwrap()
            .queue_command(json!({"request_id": "disconnect-race"}))
            .unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "disconnect-race".into(),
            PendingRequest {
                operation: "navigate".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: Some(7),
                fallback_generation: None,
                console_capture_id: None,
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );

        state
            .handle(
                BrokerEvent::ExtensionDisconnected {
                    extension_instance_id: "profile-a".into(),
                    generation: 7,
                },
                &runtime,
            )
            .await;
        assert_eq!(
            receiver.await.unwrap().unwrap_err().code,
            BrokerErrorCode::BrowserSessionDisconnected
        );
        assert!(state.pending.is_empty());
        assert_eq!(
            state
                .sessions
                .get("profile-a")
                .unwrap()
                .queued_command_count(),
            0
        );

        assert_eq!(
            state.handle_extension_response(
                "profile-a",
                Some(7),
                extension_response("disconnect-race", "navigate", target()),
            )["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        assert_eq!(
            state.quarantined_responses.last().unwrap()["reason"],
            "late_response"
        );
        runtime.shutdown().await;
    }

    #[test]
    fn shared_profile_response_fixture_keeps_rust_completions_isolated() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../resources/browser_contract_fixtures.json"
        ))
        .unwrap();
        let scenario = &fixture["stateful"]["profile_response_race"];
        let request_items = scenario["requests"].as_array().unwrap();
        let mut state = BrokerState::new();
        let now = Instant::now();
        let mut receivers = HashMap::new();

        for item in request_items {
            let instance_id = item["extension_instance_id"].as_str().unwrap();
            let mut payload = heartbeat();
            payload.extension_instance_id = Some(instance_id.into());
            state.sessions.register_heartbeat(payload, now).unwrap();
            let request_id = item["request_id"].as_str().unwrap().to_owned();
            let (reply, receiver) = oneshot::channel();
            state.pending.insert(
                request_id.clone(),
                PendingRequest {
                    operation: "get_page_snapshot".into(),
                    extension_instance_id: instance_id.into(),
                    target: target_for(
                        instance_id,
                        item["window_id"].as_i64().unwrap(),
                        item["tab_id"].as_i64().unwrap(),
                    ),
                    project_root: format!("C:/{instance_id}"),
                    caller_label: format!("agent-{instance_id}"),
                    lease_token: Some(format!("lease-{instance_id}")),
                    snapshot_id: None,
                    stream_generation: None,
                    fallback_generation: None,
                    console_capture_id: None,
                    deadline: now + Duration::from_secs(30),
                    reply,
                },
            );
            receivers.insert(request_id, receiver);
        }

        for request_id in scenario["response_order"].as_array().unwrap() {
            let request_id = request_id.as_str().unwrap();
            let item = request_items
                .iter()
                .find(|item| item["request_id"].as_str() == Some(request_id))
                .unwrap();
            let instance_id = item["extension_instance_id"].as_str().unwrap();
            let target = target_for(
                instance_id,
                item["window_id"].as_i64().unwrap(),
                item["tab_id"].as_i64().unwrap(),
            );
            let response = ExtensionResponse {
                message_type: "response".into(),
                schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
                protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
                request_id: request_id.into(),
                operation: "get_page_snapshot".into(),
                extension_instance_id: Some(instance_id.into()),
                target: Some(target),
                ok: true,
                code: None,
                error: None,
                result: BTreeMap::from([("url".into(), item["result_url"].clone())]),
            };
            assert_eq!(
                state.handle_extension_response(instance_id, None, response)["ok"],
                true
            );
        }

        for item in request_items {
            let request_id = item["request_id"].as_str().unwrap();
            let result = receivers
                .remove(request_id)
                .unwrap()
                .blocking_recv()
                .unwrap()
                .unwrap();
            assert_eq!(
                result["extension_instance_id"],
                item["extension_instance_id"]
            );
            assert_eq!(result["url"], item["result_url"]);
        }
        assert!(state.pending.is_empty());
    }

    #[tokio::test]
    async fn mismatched_response_is_quarantined_and_matching_response_completes_once() {
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "request-1".into(),
            PendingRequest {
                operation: "navigate".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: None,
                fallback_generation: None,
                console_capture_id: None,
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );

        let mismatched = ExtensionResponse {
            message_type: "response".into(),
            schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
            protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
            request_id: "request-1".into(),
            operation: "navigate".into(),
            extension_instance_id: Some("profile-a".into()),
            target: Some(BrowserTarget {
                extension_instance_id: "profile-a".into(),
                window_id: 7,
                tab_id: 99,
            }),
            ok: true,
            code: None,
            error: None,
            result: BTreeMap::new(),
        };
        assert_eq!(
            state.handle_extension_response("profile-a", None, mismatched)["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.quarantined_responses.len(), 1);

        let matching = ExtensionResponse {
            message_type: "response".into(),
            schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
            protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
            request_id: "request-1".into(),
            operation: "navigate".into(),
            extension_instance_id: Some("profile-a".into()),
            target: Some(target()),
            ok: true,
            code: None,
            error: None,
            result: BTreeMap::from([(String::from("url"), json!("https://after.test"))]),
        };
        assert_eq!(
            state.handle_extension_response("profile-a", None, matching)["ok"],
            true
        );
        let completed = receiver.await.unwrap().unwrap();
        assert_eq!(completed["operation"], "navigate");
        assert_eq!(completed["target"], json!(target()));
        assert!(state.pending.is_empty());

        let late = ExtensionResponse {
            message_type: "response".into(),
            schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
            protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
            request_id: "request-1".into(),
            operation: "navigate".into(),
            extension_instance_id: Some("profile-a".into()),
            target: Some(target()),
            ok: true,
            code: None,
            error: None,
            result: BTreeMap::new(),
        };
        assert_eq!(
            state.handle_extension_response("profile-a", None, late)["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        assert_eq!(state.quarantined_responses.len(), 2);
    }

    #[tokio::test]
    async fn successful_snapshot_response_publishes_revision_bound_references() {
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "snapshot-1".into(),
            PendingRequest {
                operation: "get_page_snapshot".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: None,
                fallback_generation: None,
                console_capture_id: None,
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );

        let response = ExtensionResponse {
            message_type: "response".into(),
            schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
            protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
            request_id: "snapshot-1".into(),
            operation: "get_page_snapshot".into(),
            extension_instance_id: Some("profile-a".into()),
            target: Some(target()),
            ok: true,
            code: None,
            error: None,
            result: BTreeMap::from([
                ("page_context_revision".into(), json!("revision-1")),
                (
                    "interactive_elements".into(),
                    json!([{"tag": "button", "context": {"frame": "main"}}]),
                ),
            ]),
        };
        state.handle_extension_response("profile-a", None, response);
        let result = receiver.await.unwrap().unwrap();
        assert_eq!(result["snapshot_id"], "snapshot-1");
        assert_eq!(result["interactive_elements"][0]["ref"], "@e1");
        let reference = state
            .sessions
            .resolve_element_reference(
                &target(),
                "@e1",
                Some("revision-1"),
                Some("snapshot-1"),
                "C:/project-a",
                "caller-a",
                now,
            )
            .unwrap();
        assert_eq!(reference.element["tag"], "button");
    }

    #[tokio::test]
    async fn duplicate_mutating_request_is_rejected_before_target_dispatch() {
        let mut config = crate::server::BrokerServerConfig::with_trusted_extension_origins(vec![
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ]);
        config.discovery_addr = "127.0.0.1:0".parse().unwrap();
        config.broker_features = vec!["p0.control".into()];
        let runtime = BrokerRuntime::start(config).await.unwrap();
        let now = Instant::now();
        let mut state = BrokerState::new();
        let (existing_reply, _existing_receiver) = oneshot::channel();
        state.pending.insert(
            "mutation-1".into(),
            PendingRequest {
                operation: "execute_browser_action".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: None,
                fallback_generation: None,
                console_capture_id: None,
                deadline: now + Duration::from_secs(30),
                reply: existing_reply,
            },
        );
        let (reply, receiver) = oneshot::channel();
        let request: OperationRequest = serde_json::from_value(json!({
            "schema_version": 1,
            "request_id": "mutation-1",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "execute_browser_action"
        }))
        .unwrap();

        state.handle_operation(request, reply, &runtime).await;

        assert_eq!(
            receiver.await.unwrap().unwrap_err().code,
            BrokerErrorCode::DuplicateBrowserMutation
        );
        runtime.shutdown().await;
    }

    #[test]
    fn lease_scope_is_bound_to_project_caller_and_broker_generation() {
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.leases.insert(
            "profile-a".into(),
            LeaseRecord {
                token: "lease-secret".into(),
                owner_label: "agent-a".into(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                broker_start_id: "generation-a".into(),
                acquired_at_ms: 1,
                expires_at_ms: 10_000,
                expires_at: now + Duration::from_secs(30),
            },
        );
        assert!(
            state
                .validate_lease(
                    "profile-a",
                    "lease-secret",
                    "C:/project-a",
                    "caller-a",
                    "generation-a"
                )
                .is_ok()
        );
        assert_eq!(
            state
                .validate_lease(
                    "profile-a",
                    "lease-secret",
                    "C:/project-b",
                    "caller-a",
                    "generation-a"
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::InvalidBrowserLease
        );
        assert_eq!(
            state
                .validate_lease(
                    "profile-a",
                    "lease-secret",
                    "C:/project-a",
                    "caller-a",
                    "generation-b"
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::InvalidBrowserLease
        );
    }

    #[tokio::test]
    async fn renew_and_release_read_the_top_level_lease_token() {
        let mut config = crate::server::BrokerServerConfig::with_trusted_extension_origins(vec![
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ]);
        config.discovery_addr = "127.0.0.1:0".parse().unwrap();
        let runtime = BrokerRuntime::start(config).await.unwrap();
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();

        let acquire: OperationRequest = serde_json::from_value(json!({
            "schema_version": 1,
            "request_id": "lease-acquire",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "acquire_browser_lease",
            "extension_instance_id": "profile-a",
            "owner_label": "caller-a",
            "ttl_secs": 60
        }))
        .unwrap();
        let (acquire_reply, acquire_receiver) = oneshot::channel();
        state
            .handle_operation(acquire, acquire_reply, &runtime)
            .await;
        let acquired = acquire_receiver.await.unwrap().unwrap();
        let token = acquired["lease_token"].as_str().unwrap().to_owned();

        let renew: OperationRequest = serde_json::from_value(json!({
            "schema_version": 1,
            "request_id": "lease-renew",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "renew_browser_lease",
            "extension_instance_id": "profile-a",
            "lease_token": token.clone(),
            "ttl_secs": 60
        }))
        .unwrap();
        let (renew_reply, renew_receiver) = oneshot::channel();
        state.handle_operation(renew, renew_reply, &runtime).await;
        assert_eq!(
            renew_receiver.await.unwrap().unwrap()["ok"].as_bool(),
            Some(true)
        );

        let release: OperationRequest = serde_json::from_value(json!({
            "schema_version": 1,
            "request_id": "lease-release",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "release_browser_lease",
            "extension_instance_id": "profile-a",
            "lease_token": token
        }))
        .unwrap();
        let (release_reply, release_receiver) = oneshot::channel();
        state
            .handle_operation(release, release_reply, &runtime)
            .await;
        assert_eq!(
            release_receiver.await.unwrap().unwrap()["ok"].as_bool(),
            Some(true)
        );
        assert!(!state.leases.contains_key("profile-a"));
        runtime.shutdown().await;
    }

    #[test]
    fn phased_operations_require_advertised_feature_and_operation() {
        let request: OperationRequest = serde_json::from_value(json!({
            "request_id": "network-1",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "start_network_capture"
        }))
        .unwrap();
        assert_eq!(
            required_features_for_operation(&request),
            vec![
                "p1.filtered_network_capture",
                "p1.network_batch_transport",
                "p1.observability_artifacts"
            ]
        );
        assert!(requires_extension_operation_advertisement(
            "start_network_capture"
        ));
        assert!(is_mutating_operation("navigate"));
        assert!(!is_mutating_operation("list_browser_sessions"));
    }

    #[test]
    fn lease_expiry_clears_snapshot_references_before_a_new_owner_can_reuse_them() {
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        let target = target();
        let mut snapshot = json!({
            "snapshot_id": "snapshot-1",
            "page_context_revision": "revision-1",
            "interactive_elements": [{"tag": "button"}]
        });
        state
            .sessions
            .get_mut("profile-a")
            .unwrap()
            .cache_snapshot_references(
                target,
                &mut snapshot,
                "request-1",
                "C:/project-a",
                "caller-a",
                now,
            )
            .unwrap();
        state.leases.insert(
            "profile-a".into(),
            LeaseRecord {
                token: "lease-secret".into(),
                owner_label: "agent-a".into(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                broker_start_id: "generation-a".into(),
                acquired_at_ms: 1,
                expires_at_ms: 2,
                expires_at: now - Duration::from_secs(1),
            },
        );

        state.expire(now);

        assert!(!state.leases.contains_key("profile-a"));
        assert_eq!(
            state
                .sessions
                .get("profile-a")
                .unwrap()
                .element_reference_count(),
            0
        );
    }

    #[test]
    fn dispatched_side_effect_timeout_reports_unknown_execution_outcome() {
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "action-timeout".into(),
            PendingRequest {
                operation: "execute_browser_action".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: Some(7),
                fallback_generation: None,
                console_capture_id: None,
                deadline: now - Duration::from_secs(1),
                reply,
            },
        );
        state
            .action_metadata
            .insert("action-timeout".into(), dispatched_action_metadata());
        state.dispatched_actions.insert("action-timeout".into());

        state.expire(now);

        let error = receiver.blocking_recv().unwrap().unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserExecutionUnknown);
        assert_eq!(error.recovery["outcome"], "unknown");
        assert_eq!(error.recovery["cause"], "browser_operation_timeout");
        assert_eq!(error.recovery["retry"], "do_not_retry_automatically");
        assert!(!state.pending.contains_key("action-timeout"));
        assert!(state.retired_requests.contains_key("action-timeout"));
    }

    #[tokio::test]
    async fn dispatched_action_cancel_is_unknown_and_late_reuse_is_quarantined() {
        let mut config = crate::server::BrokerServerConfig::with_trusted_extension_origins(vec![
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ]);
        config.discovery_addr = "127.0.0.1:0".parse().unwrap();
        let runtime = BrokerRuntime::start(config).await.unwrap();
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "action-cancel".into(),
            PendingRequest {
                operation: "execute_browser_action".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: Some(7),
                fallback_generation: None,
                console_capture_id: None,
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );
        state
            .action_metadata
            .insert("action-cancel".into(), dispatched_action_metadata());
        state.dispatched_actions.insert("action-cancel".into());

        let (cancel_reply, cancel_receiver) = oneshot::channel();
        let cancel: OperationRequest = serde_json::from_value(json!({
            "request_id": "cancel-action-control",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "cancel_browser_request",
            "cancel_request_id": "action-cancel"
        }))
        .unwrap();
        state.handle_operation(cancel, cancel_reply, &runtime).await;

        assert_eq!(cancel_receiver.await.unwrap().unwrap()["cancelled"], true);
        let error = receiver.await.unwrap().unwrap_err();
        assert_eq!(error.code, BrokerErrorCode::BrowserExecutionUnknown);
        assert_eq!(error.recovery["cause"], "browser_operation_cancelled");
        assert_eq!(error.recovery["retry"], "do_not_retry_automatically");
        assert_eq!(
            state.handle_extension_response(
                "profile-a",
                Some(7),
                extension_response("action-cancel", "execute_locator", target()),
            )["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        assert_eq!(
            state.quarantined_responses.last().unwrap()["reason"],
            "late_response"
        );

        let (reuse_reply, reuse_receiver) = oneshot::channel();
        let reuse: OperationRequest = serde_json::from_value(json!({
            "request_id": "action-cancel",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "execute_browser_action"
        }))
        .unwrap();
        state.handle_operation(reuse, reuse_reply, &runtime).await;
        assert_eq!(
            reuse_receiver.await.unwrap().unwrap_err().code,
            BrokerErrorCode::DuplicateBrowserMutation
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn heartbeat_fallback_rejects_a_replaced_stream_generation() {
        let mut config = crate::server::BrokerServerConfig::with_trusted_extension_origins(vec![
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ]);
        config.discovery_addr = "127.0.0.1:0".parse().unwrap();
        let runtime = BrokerRuntime::start(config).await.unwrap();
        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state.sessions.attach_stream("profile-a", 7, now).unwrap();
        state
            .sessions
            .get_mut("profile-a")
            .unwrap()
            .queue_command(json!({"request_id": "fallback-generation"}))
            .unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "fallback-generation".into(),
            PendingRequest {
                operation: "navigate".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: None,
                fallback_generation: Some(7),
                console_capture_id: None,
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );
        state
            .sessions
            .attach_stream("profile-a", 8, now + Duration::from_millis(1))
            .unwrap();

        let mut response = json!({"cmd": {"request_id": "fallback-generation"}});
        state.validate_heartbeat_command(&mut response, &runtime);
        assert!(response["cmd"].is_null());
        assert_eq!(
            receiver.await.unwrap().unwrap_err().code,
            BrokerErrorCode::BrowserSessionDisconnected
        );
        assert!(!state.pending.contains_key("fallback-generation"));
        assert_eq!(
            state
                .sessions
                .get("profile-a")
                .unwrap()
                .queued_command_count(),
            0
        );
        runtime.shutdown().await;
    }

    #[test]
    fn network_store_only_acks_contiguous_processed_events_and_preserves_scope() {
        let target = target();
        let mut state = BrokerState::new();
        let handle = state
            .evidence
            .prepare_network_capture(
                "network-start",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                Some(7),
                Some(&json!(["EXAMPLE.test."])),
                Some(&json!(true)),
                Some(&json!(1024)),
                None,
                None,
                None,
                Some(&json!(1024)),
                None,
            )
            .unwrap();
        let capture_id = handle.capture_id;
        let mut started =
            extension_response("network-start", "start_network_capture", target.clone());
        started.result.insert("active".into(), Value::Bool(true));
        started
            .result
            .insert("capture_id".into(), Value::String(capture_id.clone()));
        state
            .evidence
            .commit_network_capture(
                "network-start",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                &started,
            )
            .unwrap();

        let gap = network_batch(
            target.clone(),
            &capture_id,
            json!([{
                "seq": 2,
                "event": {
                    "event_type": "request",
                    "request_id": "second",
                    "url": "https://example.test/second"
                }
            }]),
        );
        let gap_ack = state.evidence.accept_network_batch("profile-a", 7, &gap);
        assert_eq!(gap_ack["ack_seq"], 0);
        assert_eq!(gap_ack["acknowledged_sequence"], 0);
        assert_eq!(gap_ack["accepted"], true);

        let first = network_batch(
            target.clone(),
            &capture_id,
            json!([{
                "seq": 1,
                "event": {
                    "event_type": "request",
                    "request_id": "first",
                    "url": "https://example.test/api?token=private&safe=visible",
                    "method": "POST",
                    "request_body": {
                        "encoding": "utf8",
                        "body": "x".repeat(2048),
                        "original_size": 2048
                    },
                    "headers": {
                        "Authorization": "Bearer private",
                        "Accept": "application/json"
                    }
                }
            }]),
        );
        let first_ack = state.evidence.accept_network_batch("profile-a", 7, &first);
        assert_eq!(first_ack["ack_seq"], 2);

        let response = network_batch(
            target.clone(),
            &capture_id,
            json!([
                {
                    "seq": 3,
                    "event": {
                        "event_type": "response",
                        "request_id": "first",
                        "status": 200,
                        "headers": {"Set-Cookie": "private", "Content-Type": "application/json"}
                    }
                },
                {
                    "seq": 4,
                    "event": {
                        "event_type": "finished",
                        "request_id": "first",
                        "encoded_data_length": 12
                    }
                },
                {
                    "seq": 5,
                    "event": {
                        "event_type": "response",
                        "request_id": "orphan",
                        "status": 500
                    }
                },
                {
                    "seq": 6,
                    "event": {
                        "event_type": "request",
                        "request_id": "suffix",
                        "url": "https://evil.example.test/"
                    }
                }
            ]),
        );
        let response_ack = state
            .evidence
            .accept_network_batch("profile-a", 7, &response);
        assert_eq!(response_ack["ack_seq"], 6);
        let far_ahead = network_batch(
            target.clone(),
            &capture_id,
            json!([{
                "seq": 6 + crate::protocol::MAX_NETWORK_PENDING_EVENTS as u64 + 1,
                "event": {
                    "event_type": "request",
                    "request_id": "far-ahead",
                    "url": "https://example.test/far-ahead"
                }
            }]),
        );
        let far_ahead_ack = state
            .evidence
            .accept_network_batch("profile-a", 7, &far_ahead);
        assert_eq!(far_ahead_ack["ack_seq"], 6);
        assert_eq!(far_ahead_ack["accepted"], true);
        let listed = state
            .evidence
            .list_network_requests(
                &target,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(listed["requests"].as_array().unwrap().len(), 2);
        assert_eq!(listed["delivery"]["filtered_events"], 1);
        assert!(listed["delivery"]["rejected_events"].as_u64().unwrap() >= 2);
        assert!(listed["requests"].to_string().contains("safe=visible"));
        assert!(!listed["requests"].to_string().contains("request_headers"));
        assert_eq!(
            state
                .evidence
                .list_network_requests(
                    &target,
                    "broker-generation-a",
                    "C:/other-project",
                    "caller-a",
                    None,
                    None,
                    None,
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::InvalidBrowserLease
        );
        assert_eq!(
            state
                .evidence
                .list_network_requests(
                    &target,
                    "broker-generation-a",
                    "C:/project-a",
                    "caller-b",
                    None,
                    None,
                    None,
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::InvalidBrowserLease
        );

        let detail = state
            .evidence
            .get_network_request_detail(
                &target,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &json!("first"),
                false,
            )
            .unwrap();
        let detail_text = detail.to_string();
        assert!(!detail_text.contains("Bearer private"));
        assert!(!detail_text.contains("private"));
        assert!(detail_text.contains("safe=visible"));
        assert_eq!(listed["requests"][0].get("request_body"), None);
        let retained_body = detail["request"]["request_body"]["body"].as_str().unwrap();
        assert_eq!(retained_body.len(), 1024);
        assert_eq!(detail["request"]["request_body"]["captured_size"], 1024);
        assert_eq!(detail["request"]["request_body"]["truncated"], true);

        let access = state
            .evidence
            .prepare_network_body_access(
                &target,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &json!("first"),
                Some(&json!(1024)),
            )
            .unwrap();
        let response_body = state
            .evidence
            .bound_network_body(
                &target,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &access,
                Some(&Value::String("y".repeat(2048))),
                false,
            )
            .unwrap();
        assert_eq!(response_body["body"].as_str().unwrap().len(), 1024);
        assert_eq!(response_body["returned_size"], 1024);
        assert_eq!(response_body["original_size"], 2048);
        assert_eq!(response_body["truncated"], true);
        assert!(response_body.to_string().contains("safe=visible"));

        let duplicate_ack = state.evidence.accept_network_batch("profile-a", 6, &first);
        assert_eq!(duplicate_ack["accepted"], false);
        assert_eq!(duplicate_ack["reason"], "stream_generation_mismatch");
        let duplicate_ack = state.evidence.accept_network_batch("profile-a", 7, &first);
        assert_eq!(duplicate_ack["ack_seq"], 6);
        assert!(
            state
                .evidence
                .list_network_requests(
                    &target,
                    "broker-generation-a",
                    "C:/project-a",
                    "caller-a",
                    None,
                    None,
                    None,
                )
                .unwrap()["delivery"]["duplicate_events"]
                .as_u64()
                .unwrap()
                >= 1
        );

        let cleared = state
            .evidence
            .clear_network_capture(
                &target,
                &capture_id,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                8,
            )
            .unwrap();
        assert_eq!(cleared["sequence_barrier"], 8);
        assert_eq!(cleared["retained_entries"], 0);
        assert_eq!(
            state
                .evidence
                .bound_network_body(
                    &target,
                    "broker-generation-a",
                    "C:/project-a",
                    "caller-a",
                    &access,
                    Some(&Value::String("should-not-be-read".into())),
                    false,
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::BrowserTargetNotFound
        );
        let stopped = state
            .evidence
            .stop_network_capture(
                &target,
                &capture_id,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                8,
                "explicit_stop",
            )
            .unwrap();
        assert_eq!(stopped["active"], false);
        let late = state.evidence.accept_network_batch("profile-a", 7, &first);
        assert_eq!(late["accepted"], true);
        assert_eq!(late["ack_seq"], 8);
    }

    #[test]
    fn network_capture_replacement_keeps_old_unacknowledged_events_out_of_new_capture() {
        let target = target();
        let mut state = BrokerState::new();
        let old_handle = state
            .evidence
            .prepare_network_capture(
                "network-old",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                Some(7),
                Some(&json!(["example.test"])),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let old_capture_id = old_handle.capture_id;
        let mut old_start =
            extension_response("network-old", "start_network_capture", target.clone());
        old_start.result.insert("active".into(), Value::Bool(true));
        old_start
            .result
            .insert("capture_id".into(), Value::String(old_capture_id.clone()));
        state
            .evidence
            .commit_network_capture(
                "network-old",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                &old_start,
            )
            .unwrap();

        let old_gap = network_batch(
            target.clone(),
            &old_capture_id,
            json!([{
                "seq": 2,
                "event": {
                    "event_type": "request",
                    "request_id": "old-unacknowledged",
                    "url": "https://example.test/old"
                }
            }]),
        );
        assert_eq!(
            state
                .evidence
                .accept_network_batch("profile-a", 7, &old_gap)["ack_seq"],
            0
        );

        let new_handle = state
            .evidence
            .prepare_network_capture(
                "network-new",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                Some(7),
                Some(&json!(["example.test"])),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let new_capture_id = new_handle.capture_id;
        let mut new_start =
            extension_response("network-new", "start_network_capture", target.clone());
        new_start.result.insert("active".into(), Value::Bool(true));
        new_start
            .result
            .insert("capture_id".into(), Value::String(new_capture_id.clone()));
        state
            .evidence
            .commit_network_capture(
                "network-new",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                &new_start,
            )
            .unwrap();

        let late_old = state
            .evidence
            .accept_network_batch("profile-a", 7, &old_gap);
        assert_eq!(late_old["accepted"], true);
        assert_eq!(late_old["ack_seq"], 0);
        let fresh = network_batch(
            target.clone(),
            &new_capture_id,
            json!([{
                "seq": 1,
                "event": {
                    "event_type": "request",
                    "request_id": "new-request",
                    "url": "https://example.test/new"
                }
            }]),
        );
        assert_eq!(
            state.evidence.accept_network_batch("profile-a", 7, &fresh)["ack_seq"],
            1
        );
        let listed = state
            .evidence
            .list_network_requests(
                &target,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(listed["requests"].as_array().unwrap().len(), 1);
        assert!(listed["requests"].to_string().contains("new-request"));
        assert!(
            !listed["requests"]
                .to_string()
                .contains("old-unacknowledged")
        );
    }

    #[test]
    fn network_termination_barrier_rejects_events_after_final_sequence() {
        let now = Instant::now();
        let target = target();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state.sessions.attach_stream("profile-a", 7, now).unwrap();
        let handle = state
            .evidence
            .prepare_network_capture(
                "network-terminal",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                Some(7),
                Some(&json!(["example.test"])),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let capture_id = handle.capture_id;
        let mut started =
            extension_response("network-terminal", "start_network_capture", target.clone());
        started.result.insert("active".into(), Value::Bool(true));
        started
            .result
            .insert("capture_id".into(), Value::String(capture_id.clone()));
        state
            .evidence
            .commit_network_capture(
                "network-terminal",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                &started,
            )
            .unwrap();

        let terminal = NetworkBatch {
            message_type: "network_batch".into(),
            extension_instance_id: "profile-a".into(),
            capture_id: capture_id.clone(),
            target: target.clone(),
            events: vec![
                serde_json::from_value(json!({
                    "seq": 1,
                    "event": {
                        "event_type": "request",
                        "request_id": "terminal-request",
                        "url": "https://example.test/terminal"
                    }
                }))
                .unwrap(),
            ],
            first_seq: Some(1),
            last_seq: Some(1),
            dropped_events: 0,
            dropped_bytes: 0,
            dropped_events_total: 0,
            dropped_bytes_total: 0,
            termination_reason: Some("debugger_detached".into()),
            termination_detail: None,
            final_sequence: Some(3),
            diagnostics: None,
        };
        let terminal_ack = state.handle_network_batch(terminal, 7);
        assert_eq!(terminal_ack["ack_seq"], 3);

        let late = network_batch(
            target.clone(),
            &capture_id,
            json!([{
                "seq": 4,
                "event": {
                    "event_type": "request",
                    "request_id": "after-terminal",
                    "url": "https://example.test/after-terminal"
                }
            }]),
        );
        let late_ack = state.handle_network_batch(late, 7);
        assert_eq!(late_ack["ack_seq"], 3);
        let listed = state
            .evidence
            .list_network_requests(
                &target,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(listed["active"], false);
        assert!(listed["requests"].to_string().contains("terminal-request"));
        assert!(!listed["requests"].to_string().contains("after-terminal"));
        assert!(listed["delivery"]["rejected_events"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn network_body_detail_uses_the_extension_response_body_operation() {
        let request: OperationRequest = serde_json::from_value(json!({
            "request_id": "network-body",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "get_network_request_detail",
            "network_request_id": "request-1",
            "include_body": true,
            "max_body_bytes": 1024
        }))
        .unwrap();
        let command =
            build_extension_command_with_capture_id(&request, &target(), "profile-a", None, None)
                .unwrap();
        assert_eq!(command["cmd"], "get_network_response_body");
        assert_eq!(command["network_request_id"], "request-1");
        assert_eq!(command["max_body_bytes"], 1024);
    }

    #[tokio::test]
    async fn network_body_response_is_bounded_and_grant_is_retired() {
        let target = target();
        let mut state = BrokerState::new();
        let handle = state
            .evidence
            .prepare_network_capture(
                "network-body-capture",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                Some(7),
                Some(&json!(["example.test"])),
                Some(&json!(true)),
                Some(&json!(1024)),
                None,
                None,
                None,
                Some(&json!(1024)),
                None,
            )
            .unwrap();
        let capture_id = handle.capture_id;
        let mut started = extension_response(
            "network-body-capture",
            "start_network_capture",
            target.clone(),
        );
        started.result.insert("active".into(), Value::Bool(true));
        started
            .result
            .insert("capture_id".into(), Value::String(capture_id.clone()));
        state
            .evidence
            .commit_network_capture(
                "network-body-capture",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                &started,
            )
            .unwrap();
        let batch = network_batch(
            target.clone(),
            &capture_id,
            json!([{
                "seq": 1,
                "event": {
                    "event_type": "request",
                    "request_id": "request-1",
                    "url": "https://example.test/resource"
                }
            }]),
        );
        assert_eq!(
            state.evidence.accept_network_batch("profile-a", 7, &batch)["ack_seq"],
            1
        );
        let access = state
            .evidence
            .prepare_network_body_access(
                &target,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &json!("request-1"),
                Some(&json!(1024)),
            )
            .unwrap();
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "network-body-response".into(),
            PendingRequest {
                operation: "get_network_request_detail".into(),
                extension_instance_id: "profile-a".into(),
                target: target.clone(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: None,
                snapshot_id: None,
                stream_generation: Some(7),
                fallback_generation: None,
                console_capture_id: None,
                deadline: Instant::now() + Duration::from_secs(30),
                reply,
            },
        );
        state
            .network_body_access
            .insert("network-body-response".into(), access);
        let raw_body = "raw-response-body-".repeat(300);
        let mut response =
            extension_response("network-body-response", "get_network_response_body", target);
        response
            .result
            .insert("body".into(), Value::String(raw_body));
        response
            .result
            .insert("base64_encoded".into(), Value::Bool(false));
        assert_eq!(
            state.handle_extension_response_with_broker_generation(
                "profile-a",
                Some(7),
                Some("broker-generation-a"),
                response,
            )["ok"],
            true
        );
        let completed = receiver.await.unwrap().unwrap();
        assert_eq!(completed["operation"], "get_network_request_detail");
        assert_eq!(completed["body"].as_str().unwrap().len(), 1024);
        assert_eq!(completed["returned_size"], 1024);
        assert_eq!(completed["truncated"], true);
        assert!(completed["request"]["request_id"] == "request-1");
        assert!(
            !state
                .network_body_access
                .contains_key("network-body-response")
        );
        assert!(!state.pending.contains_key("network-body-response"));
    }

    #[test]
    fn network_sequence_matches_shared_python_fixture_boundaries() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../resources/browser_contract_fixtures.json"
        ))
        .unwrap();
        let scenario = &fixture["migration_contracts"]["network_sequence"];
        let accepted_sequences = scenario["accepted_sequences"].as_array().unwrap();
        let first_sequence = accepted_sequences[0].as_u64().unwrap();
        let second_sequence = accepted_sequences[1].as_u64().unwrap();
        let target = target();
        let mut state = BrokerState::new();
        let handle = state
            .evidence
            .prepare_network_capture(
                "network-fixture",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                Some(7),
                Some(&json!(["api.example.test"])),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let capture_id = handle.capture_id;
        let mut started =
            extension_response("network-fixture", "start_network_capture", target.clone());
        started.result.insert("active".into(), Value::Bool(true));
        started
            .result
            .insert("capture_id".into(), Value::String(capture_id.clone()));
        state
            .evidence
            .commit_network_capture(
                "network-fixture",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                &started,
            )
            .unwrap();

        let mut payload = network_batch(
            target.clone(),
            &capture_id,
            json!([
                {
                    "seq": first_sequence,
                    "event": {
                        "event_type": "request",
                        "request_id": "suffix",
                        "url": "https://evilapi.example.test/"
                    }
                },
                {
                    "seq": second_sequence,
                    "event": {
                        "event_type": "request",
                        "request_id": "matching",
                        "url": "https://api.example.test/"
                    }
                }
            ]),
        );
        payload.dropped_events = 3;
        assert_eq!(
            state
                .evidence
                .accept_network_batch("profile-a", 7, &payload)["ack_seq"],
            scenario["duplicate_ack_sequence"]
        );
        assert_eq!(
            state
                .evidence
                .accept_network_batch("profile-a", 7, &payload)["ack_seq"],
            scenario["duplicate_ack_sequence"]
        );
        let listed = state
            .evidence
            .list_network_requests(
                &target,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(listed["requests"].as_array().unwrap().len(), 1);
        assert_eq!(listed["delivery"]["dropped_events"], 3);
        assert_eq!(listed["delivery"]["duplicate_events"], 2);

        let clear_barrier = scenario["clear_barrier_sequence"].as_u64().unwrap();
        state
            .evidence
            .clear_network_capture(
                &target,
                &capture_id,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                clear_barrier,
            )
            .unwrap();
        let late = network_batch(
            target.clone(),
            &capture_id,
            json!([{
                "seq": second_sequence + 1,
                "event": {
                    "event_type": "request",
                    "request_id": "late",
                    "url": "https://api.example.test/late"
                }
            }]),
        );
        assert_eq!(
            state.evidence.accept_network_batch("profile-a", 7, &late)["ack_seq"],
            clear_barrier
        );
        assert!(
            state
                .evidence
                .list_network_requests(
                    &target,
                    "broker-generation-a",
                    "C:/project-a",
                    "caller-a",
                    None,
                    None,
                    None,
                )
                .unwrap()["requests"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let mut terminal = network_batch(target.clone(), &capture_id, json!([]));
        terminal.termination_reason = Some("debugger_detached".into());
        terminal.final_sequence = Some(clear_barrier + 1);
        assert_eq!(
            state
                .evidence
                .accept_network_batch("profile-a", 7, &terminal)["ack_seq"],
            clear_barrier + 1
        );
        assert_eq!(
            state
                .evidence
                .stop_network_capture(
                    &target,
                    &capture_id,
                    "broker-generation-a",
                    "C:/project-a",
                    "caller-a",
                    clear_barrier + 1,
                    "explicit_stop",
                )
                .unwrap()["active"],
            false
        );
        assert_eq!(
            state
                .evidence
                .accept_network_batch("profile-a", 7, &payload)["ack_seq"],
            clear_barrier + 1
        );
    }

    #[test]
    fn network_state_rejects_old_generation_without_mutating_new_capture() {
        let now = Instant::now();
        let target = target();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state.sessions.attach_stream("profile-a", 7, now).unwrap();
        let handle = state
            .evidence
            .prepare_network_capture(
                "network-generation",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                Some(7),
                Some(&json!(["example.test"])),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let capture_id = handle.capture_id;
        let mut started = extension_response(
            "network-generation",
            "start_network_capture",
            target.clone(),
        );
        started.result.insert("active".into(), Value::Bool(true));
        started
            .result
            .insert("capture_id".into(), Value::String(capture_id.clone()));
        state
            .evidence
            .commit_network_capture(
                "network-generation",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                &started,
            )
            .unwrap();
        let batch = network_batch(
            target.clone(),
            &capture_id,
            json!([{
                "seq": 1,
                "event": {
                    "event_type": "request",
                    "request_id": "request-1",
                    "url": "https://example.test/"
                }
            }]),
        );
        let old = state.handle_network_batch(batch.clone(), 6);
        assert_eq!(old["accepted"], false);
        assert_eq!(old["reason"], "stream_generation_mismatch");
        let gap = network_batch(
            target.clone(),
            &capture_id,
            json!([{
                "seq": 2,
                "event": {
                    "event_type": "request",
                    "request_id": "request-2",
                    "url": "https://example.test/two"
                }
            }]),
        );
        let gap_ack = state.handle_network_batch(gap, 7);
        assert_eq!(gap_ack["ack_seq"], 0);

        state.handle_extension_disconnected("profile-a", 7);
        assert_eq!(
            state.evidence.network_capture_id(&target),
            Some(capture_id.clone())
        );
        state
            .sessions
            .attach_stream("profile-a", 8, Instant::now())
            .unwrap();
        state
            .evidence
            .update_network_stream_generation("profile-a", 8);
        let current = state.handle_network_batch(batch, 8);
        assert_eq!(current["ack_seq"], 2);
    }

    #[tokio::test]
    async fn expired_lease_cannot_commit_a_pending_network_start() {
        let now = Instant::now();
        let target = target();
        let mut config = crate::server::BrokerServerConfig::with_trusted_extension_origins(vec![
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ]);
        config.discovery_addr = "127.0.0.1:0".parse().unwrap();
        let runtime = BrokerRuntime::start(config).await.unwrap();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state.sessions.attach_stream("profile-a", 7, now).unwrap();
        let broker_start_id = runtime.endpoint_record().broker_start_id.clone();
        let handle = state
            .evidence
            .prepare_network_capture(
                "network-expired",
                &broker_start_id,
                "C:/project-a",
                "caller-a",
                &target,
                Some(7),
                Some(&json!(["example.test"])),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let capture_id = handle.capture_id;
        let (reply, receiver) = oneshot::channel();
        state.leases.insert(
            "profile-a".into(),
            LeaseRecord {
                token: "lease-expired".into(),
                owner_label: "caller-a".into(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                broker_start_id: broker_start_id.clone(),
                acquired_at_ms: 1,
                expires_at_ms: 2,
                expires_at: now - Duration::from_secs(1),
            },
        );
        state.pending.insert(
            "network-expired".into(),
            PendingRequest {
                operation: "start_network_capture".into(),
                extension_instance_id: "profile-a".into(),
                target: target.clone(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-expired".into()),
                snapshot_id: None,
                stream_generation: Some(7),
                fallback_generation: None,
                console_capture_id: Some(capture_id.clone()),
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );
        let mut response =
            extension_response("network-expired", "start_network_capture", target.clone());
        response.result.insert("active".into(), Value::Bool(true));
        response
            .result
            .insert("capture_id".into(), Value::String(capture_id));
        let _ = state.handle_extension_response_with_broker_generation(
            "profile-a",
            Some(7),
            Some(&broker_start_id),
            response,
        );
        assert_eq!(
            receiver.await.unwrap().unwrap_err().code,
            BrokerErrorCode::ExpiredBrowserLease
        );
        assert!(state.evidence.network_capture_id(&target).is_none());
        assert_eq!(
            state
                .sessions
                .get("profile-a")
                .unwrap()
                .queued_command_count(),
            1
        );
        runtime.shutdown().await;
    }

    #[test]
    fn local_console_list_and_clear_are_scope_bound_and_clear_keeps_capture_active() {
        let target = target();
        let mut state = BrokerState::new();
        let handle = state
            .evidence
            .prepare_console_capture(
                "local-console",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                Some(7),
                Some(&json!(["error"])),
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let capture_id = handle.capture_id;
        let mut response =
            extension_response("local-console", "start_console_capture", target.clone());
        response.ok = true;
        response.result.insert("active".into(), Value::Bool(true));
        response
            .result
            .insert("capture_id".into(), Value::String(capture_id.clone()));
        state
            .evidence
            .commit_console_capture(
                "local-console",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                &response,
            )
            .unwrap();
        assert!(state.evidence.record_console_event(
            "profile-a",
            &target,
            Some(&capture_id),
            Some(7),
            Some(&json!({"level": "error", "text": "kept"})),
        ));
        let listed = state
            .local_console_operation(
                "list_console_events",
                &target,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &BTreeMap::new(),
            )
            .unwrap();
        assert_eq!(listed["returned_entries"], 1);
        assert_eq!(
            state
                .local_console_operation(
                    "list_console_events",
                    &target,
                    "broker-generation-a",
                    "C:/project-b",
                    "caller-a",
                    &BTreeMap::new(),
                )
                .unwrap_err()
                .code,
            BrokerErrorCode::InvalidBrowserLease
        );
        let cleared = state
            .local_console_operation(
                "clear_console_capture",
                &target,
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &BTreeMap::new(),
            )
            .unwrap();
        assert_eq!(cleared["removed_entries"], 1);
        assert!(state.evidence.console_capture_id(&target).is_some());
    }

    #[test]
    fn failed_console_start_response_rolls_back_the_prepared_capture() {
        let now = Instant::now();
        let target = target();
        let mut state = BrokerState::new();
        let handle = state
            .evidence
            .prepare_console_capture(
                "failed-console",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                Some(7),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let capture_id = handle.capture_id;
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "failed-console".into(),
            PendingRequest {
                operation: "start_console_capture".into(),
                extension_instance_id: "profile-a".into(),
                target: target.clone(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: Some(7),
                fallback_generation: None,
                console_capture_id: Some(capture_id),
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );
        let mut failed =
            extension_response("failed-console", "start_console_capture", target.clone());
        failed.ok = false;
        failed.code = Some("browser_debugger_conflict".into());
        failed.error = Some("console debugger role is unavailable".into());
        let ack = state.handle_extension_response_with_broker_generation(
            "profile-a",
            Some(7),
            Some("broker-generation-a"),
            failed,
        );
        assert_eq!(ack["ok"], true);
        assert!(!state.pending.contains_key("failed-console"));
        assert!(state.evidence.console_capture_id(&target).is_none());
        assert_eq!(receiver.blocking_recv().unwrap().unwrap()["ok"], false);
    }

    #[tokio::test]
    async fn console_commands_and_responses_require_broker_capture_id_target_and_generation() {
        let request: OperationRequest = serde_json::from_value(json!({
            "request_id": "console-command",
            "caller_label": "caller-a",
            "project_root": "C:/project-a",
            "cmd": "start_console_capture",
            "capture_id": "caller-controlled"
        }))
        .unwrap();
        let command = build_extension_command_with_capture_id(
            &request,
            &target(),
            "profile-a",
            None,
            Some("console_broker_id"),
        )
        .unwrap();
        assert_eq!(command["capture_id"], "console_broker_id");

        let now = Instant::now();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state.sessions.attach_stream("profile-a", 7, now).unwrap();
        let handle = state
            .evidence
            .prepare_console_capture(
                "console-start",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target(),
                Some(7),
                Some(&json!(["error"])),
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let capture_id = handle.capture_id;
        let (reply, receiver) = oneshot::channel();
        state.pending.insert(
            "console-start".into(),
            PendingRequest {
                operation: "start_console_capture".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: Some(7),
                fallback_generation: None,
                console_capture_id: Some(capture_id.clone()),
                deadline: now + Duration::from_secs(30),
                reply,
            },
        );
        let mut wrong_id = extension_response("console-start", "start_console_capture", target());
        wrong_id.result.insert("active".into(), Value::Bool(true));
        wrong_id
            .result
            .insert("capture_id".into(), Value::String("wrong-capture".into()));
        assert_eq!(
            state.handle_extension_response_with_broker_generation(
                "profile-a",
                Some(7),
                Some("broker-generation-a"),
                wrong_id,
            )["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        assert!(state.pending.contains_key("console-start"));

        let mut old_generation =
            extension_response("console-start", "start_console_capture", target());
        old_generation
            .result
            .insert("active".into(), Value::Bool(true));
        old_generation
            .result
            .insert("capture_id".into(), Value::String(capture_id.clone()));
        assert_eq!(
            state.handle_extension_response_with_broker_generation(
                "profile-a",
                Some(6),
                Some("broker-generation-a"),
                old_generation,
            )["code"],
            BrokerErrorCode::MismatchedBrowserResponse.as_str()
        );
        assert!(state.pending.contains_key("console-start"));

        let mut matching = extension_response("console-start", "start_console_capture", target());
        matching.result.insert("active".into(), Value::Bool(true));
        matching
            .result
            .insert("capture_id".into(), Value::String(capture_id.clone()));
        assert_eq!(
            state.handle_extension_response_with_broker_generation(
                "profile-a",
                Some(7),
                Some("broker-generation-a"),
                matching,
            )["ok"],
            true
        );
        let started = receiver.await.unwrap().unwrap();
        assert_eq!(started["capture"]["capture_id"], capture_id);

        let (stop_reply, stop_receiver) = oneshot::channel();
        state.pending.insert(
            "console-stop".into(),
            PendingRequest {
                operation: "stop_console_capture".into(),
                extension_instance_id: "profile-a".into(),
                target: target(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                lease_token: Some("lease-secret".into()),
                snapshot_id: None,
                stream_generation: Some(7),
                fallback_generation: None,
                console_capture_id: Some(capture_id.clone()),
                deadline: now + Duration::from_secs(30),
                reply: stop_reply,
            },
        );
        let mut stopped = extension_response("console-stop", "stop_console_capture", target());
        stopped
            .result
            .insert("capture_id".into(), Value::String(capture_id));
        assert_eq!(
            state.handle_extension_response_with_broker_generation(
                "profile-a",
                Some(7),
                Some("broker-generation-a"),
                stopped,
            )["ok"],
            true
        );
        let stopped = stop_receiver.await.unwrap().unwrap();
        assert_eq!(stopped["capture"]["termination"]["reason"], "explicit_stop");
        assert!(state.evidence.console_capture_id(&target()).is_none());
    }

    #[test]
    fn console_stream_disconnect_terminates_only_that_profile_capture() {
        let now = Instant::now();
        let target_a = target();
        let target_b = target_for("profile-b", 8, 52);
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state.sessions.attach_stream("profile-a", 7, now).unwrap();

        for (request_id, target, instance_id) in [
            ("disconnect-a", target_a.clone(), "profile-a"),
            ("disconnect-b", target_b.clone(), "profile-b"),
        ] {
            let handle = state
                .evidence
                .prepare_console_capture(
                    request_id,
                    "broker-generation-a",
                    "C:/project-a",
                    "caller-a",
                    &target,
                    Some(7),
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
            let capture_id = handle.capture_id;
            let mut response =
                extension_response(request_id, "start_console_capture", target.clone());
            response.result.insert("active".into(), Value::Bool(true));
            response
                .result
                .insert("capture_id".into(), Value::String(capture_id));
            state
                .evidence
                .commit_console_capture(
                    request_id,
                    "broker-generation-a",
                    "C:/project-a",
                    "caller-a",
                    &target,
                    &response,
                )
                .unwrap();
            assert_eq!(target.extension_instance_id, instance_id);
        }

        state.handle_extension_disconnected("profile-a", 7);

        assert!(state.evidence.console_capture_id(&target_a).is_none());
        assert!(state.evidence.console_capture_id(&target_b).is_some());
        assert_eq!(
            state.evidence.latest_console_termination().unwrap()["termination"]["reason"],
            "stream_disconnected"
        );
    }

    #[test]
    fn lease_expiry_queues_extension_console_cleanup_before_dropping_capture() {
        let now = Instant::now();
        let target = target();
        let mut state = BrokerState::new();
        state.sessions.register_heartbeat(heartbeat(), now).unwrap();
        state.sessions.attach_stream("profile-a", 7, now).unwrap();
        state.leases.insert(
            "profile-a".into(),
            LeaseRecord {
                token: "lease-secret".into(),
                owner_label: "owner-a".into(),
                project_root: "C:/project-a".into(),
                caller_label: "caller-a".into(),
                broker_start_id: "broker-generation-a".into(),
                acquired_at_ms: 1,
                expires_at_ms: 2,
                expires_at: now - Duration::from_secs(1),
            },
        );
        let handle = state
            .evidence
            .prepare_console_capture(
                "lease-expiry-console",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                Some(7),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let capture_id = handle.capture_id;
        let mut response = extension_response(
            "lease-expiry-console",
            "start_console_capture",
            target.clone(),
        );
        response.result.insert("active".into(), Value::Bool(true));
        response
            .result
            .insert("capture_id".into(), Value::String(capture_id.clone()));
        state
            .evidence
            .commit_console_capture(
                "lease-expiry-console",
                "broker-generation-a",
                "C:/project-a",
                "caller-a",
                &target,
                &response,
            )
            .unwrap();

        state.expire(now);

        assert!(state.evidence.console_capture_id(&target).is_none());
        let cleanup = state
            .sessions
            .get_mut("profile-a")
            .unwrap()
            .pop_command()
            .unwrap();
        assert_eq!(cleanup["cmd"], "__teshi_stop_console_capture");
        assert_eq!(cleanup["capture_id"], capture_id);
        assert_eq!(cleanup["suppress_response"], true);
    }
}
