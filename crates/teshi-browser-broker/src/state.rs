//! Single-owner broker state machine.
//!
//! The HTTP/WebSocket transport deliberately has no session or lease authority.
//! This module consumes its typed events on one async task, so target, lease and
//! pending-request transitions are serialized without holding locks over socket
//! or filesystem I/O.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use subtle::ConstantTimeEq;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::protocol::{
    BROWSER_BROKER_PROTOCOL_VERSION, BROWSER_BROKER_SCHEMA_VERSION, BrokerError, BrokerErrorCode,
    BrowserTarget, ExtensionResponse, ExtensionStreamMessage, NetworkBatch, OperationRequest,
};
use crate::server::{BrokerEvent, BrokerRuntime};
use crate::session::SessionRegistry;

const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_LEASE_TTL_SECS: u64 = 5;
const DEFAULT_LEASE_TTL_SECS: u64 = 60;
const MAX_LEASE_TTL_SECS: u64 = 3600;
const MAX_PENDING_REQUESTS: usize = 128;
const MAX_QUARANTINED_RESPONSES: usize = 32;
const MAX_RETIRED_REQUESTS: usize = MAX_PENDING_REQUESTS * 8;
const RETIRED_REQUEST_TTL: Duration = Duration::from_secs(600);

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
    stream_generation: Option<u64>,
    deadline: Instant,
    reply: oneshot::Sender<Result<Value, BrokerError>>,
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
                let result = self
                    .sessions
                    .register_heartbeat(payload, now)
                    .map(|instance_id| self.sessions.heartbeat_response(&instance_id, now));
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
            BrokerEvent::ExtensionResponse {
                extension_instance_id,
                generation,
                response,
                reply,
                ..
            } => {
                let response_value =
                    self.handle_extension_response(&extension_instance_id, generation, response);
                if let Some(reply) = reply {
                    let _ = reply.send(response_value);
                }
            }
            BrokerEvent::ExtensionHttpMessage {
                path,
                extension_instance_id,
                payload,
                reply,
                ..
            } => {
                if payload.get("type").and_then(Value::as_str) == Some("frame_error") {
                    if let (Some(instance_id), Some(error)) = (
                        extension_instance_id.as_deref(),
                        payload.get("error").and_then(Value::as_str),
                    ) {
                        self.sessions.mark_frame_error(instance_id, error);
                    }
                }
                let _ = reply.send(json!({
                    "ok": true,
                    "path": path,
                    "extension_instance_id": extension_instance_id,
                }));
            }
            BrokerEvent::NetworkBatch { batch, reply, .. } => {
                let _ = reply.send(self.handle_network_batch(batch));
            }
            BrokerEvent::PreviewFrame {
                target,
                seq,
                url,
                jpeg,
                ..
            } => {
                let _ = self
                    .sessions
                    .update_frame(target, seq, url, jpeg.to_vec(), Instant::now());
            }
            BrokerEvent::FrameError {
                extension_instance_id,
                error,
            } => self
                .sessions
                .mark_frame_error(&extension_instance_id, &error),
            BrokerEvent::Subscribe {
                extension_instance_id,
                request_id,
                reply,
            } => {
                let response = self.handle_subscription(&extension_instance_id, &request_id);
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
        match result {
            Ok(previous_generation) => json!({
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
        let Some(request_id) = response
            .get("cmd")
            .filter(|value| !value.is_null())
            .and_then(|command| command.get("request_id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            response["cmd"] = Value::Null;
            return;
        };
        let Some((operation, instance_id, project_root, caller_label, lease_token)) =
            self.pending.get(&request_id).map(|pending| {
                (
                    pending.operation.clone(),
                    pending.extension_instance_id.clone(),
                    pending.project_root.clone(),
                    pending.caller_label.clone(),
                    pending.lease_token.clone(),
                )
            })
        else {
            response["cmd"] = Value::Null;
            return;
        };
        if !requires_lease(&operation) {
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
                self.retire_request(&request_id, Instant::now());
                let _ = pending.reply.send(Err(error));
            }
        }
    }

    fn handle_extension_disconnected(&mut self, extension_instance_id: &str, generation: u64) {
        if self
            .sessions
            .detach_stream(extension_instance_id, generation)
        {
            self.fail_pending_for_session(
                extension_instance_id,
                BrokerError::new(
                    BrokerErrorCode::BrowserSessionDisconnected,
                    "browser extension stream disconnected while the operation was pending",
                ),
            );
        }
    }

    fn handle_extension_response(
        &mut self,
        event_instance_id: &str,
        generation: Option<u64>,
        response: ExtensionResponse,
    ) -> Value {
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
        let response_instance = response
            .extension_instance_id
            .as_deref()
            .unwrap_or(event_instance_id);
        let target_matches = response
            .target
            .as_ref()
            .is_none_or(|target| target == &pending.target);
        let generation_matches = pending.stream_generation == generation;
        if response_instance != pending.extension_instance_id
            || !target_matches
            || !generation_matches
            || (!response.operation.is_empty() && response.operation != pending.operation)
        {
            self.quarantine(
                &response,
                if generation_matches {
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

        let Some(pending) = self.pending.remove(&request_id) else {
            return error_value(BrokerError::new(
                BrokerErrorCode::MismatchedBrowserResponse,
                "browser response was already completed",
            ));
        };
        self.retire_request(&request_id, Instant::now());
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
            } else if matches!(pending.operation.as_str(), "navigate" | "close_tab") {
                if let Some(session) = self.sessions.get_mut(&pending.extension_instance_id) {
                    session.clear_element_references(Some(&pending.target));
                }
            }
        }
        let _ = pending.reply.send(Ok(value));
        json!({"ok": true, "request_id": request_id})
    }

    fn handle_network_batch(&mut self, batch: NetworkBatch) -> Value {
        if let Err(error) = batch.validate() {
            return error_value(error);
        }
        let ack_seq = batch
            .last_seq
            .or_else(|| batch.events.last().map(|event| event.seq))
            .unwrap_or_default();
        json!({
            "ok": true,
            "type": "network_ack",
            "extension_instance_id": batch.extension_instance_id,
            "capture_id": batch.capture_id,
            "ack_seq": ack_seq,
        })
    }

    fn handle_subscription(
        &mut self,
        extension_instance_id: &str,
        request_id: &str,
    ) -> Result<Value, BrokerError> {
        let now = Instant::now();
        let target = self
            .sessions
            .require_live(extension_instance_id, now)?
            .current_active_target()
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::BrowserTargetNotFound,
                    "browser session has no active debuggable target",
                )
            })?;
        self.sessions.subscribe_target(target.clone(), now)?;
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
            "cancel_browser_request" => Some(self.cancel_request_response(&request)),
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
            if let Some(lease) = self.active_lease(instance_id, runtime) {
                if let Value::Object(object) = session {
                    object.insert(
                        "lease".into(),
                        json!({
                            "owner_label": lease.owner_label.clone(),
                            "expires_at_ms": lease.expires_at_ms,
                        }),
                    );
                }
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
        let token = argument_string(&request.arguments, "lease_token")?;
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
        let token = argument_string(&request.arguments, "lease_token")?;
        self.sessions.require_live(&instance_id, Instant::now())?;
        let project = required_project_context(request)?;
        let caller = required_caller_context(request)?;
        let generation = runtime.endpoint_record().broker_start_id;
        self.validate_lease(&instance_id, &token, &project, &caller, &generation)?;
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

        let pending = self
            .pending
            .remove(&cancelled_request_id)
            .expect("pending request was checked immediately above");
        self.retire_request(&cancelled_request_id, Instant::now());
        if let Some(session) = self.sessions.get_mut(&pending.extension_instance_id) {
            session.remove_queued_command(&cancelled_request_id);
        }
        let operation = pending.operation.clone();
        let cancellation = BrokerError::new(
            BrokerErrorCode::BrowserOperationCancelled,
            "browser operation was explicitly cancelled; any late response is ignored",
        );
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
        let (project, caller) = match request_scope(&request, lease_required) {
            Ok(scope) => scope,
            Err(error) => {
                let _ = reply.send(Err(error));
                return;
            }
        };
        let result = self
            .sessions
            .resolve_target(request.target.as_ref(), now)
            .and_then(|(instance_id, target, _)| {
                let session = self.sessions.get(&instance_id).ok_or_else(|| {
                    BrokerError::new(
                        BrokerErrorCode::BrowserTargetNotFound,
                        "browser session was not found",
                    )
                })?;
                if !runtime
                    .endpoint_record()
                    .broker_features
                    .iter()
                    .any(|feature| feature == "p0.control")
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
                        || !session.supports_feature(&required_feature)
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
                    && !session
                        .supported_operations()
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
                Ok((instance_id, target))
            });

        let (instance_id, target) = match result {
            Ok(value) => value,
            Err(error) => {
                let _ = reply.send(Err(error));
                return;
            }
        };
        let command = build_extension_command(&request, &target, &instance_id);
        let timeout = request
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_OPERATION_TIMEOUT)
            .clamp(Duration::from_millis(1), Duration::from_secs(300));
        let stream_generation = self
            .sessions
            .get(&instance_id)
            .and_then(|session| session.current_stream_generation());
        self.pending.insert(
            request.request_id.clone(),
            PendingRequest {
                operation: request.operation.clone(),
                extension_instance_id: instance_id.clone(),
                target,
                project_root: project.clone(),
                caller_label: caller.clone(),
                lease_token: request.lease_token.clone(),
                stream_generation,
                deadline: now + timeout,
                reply,
            },
        );
        if runtime
            .send_extension_command(&instance_id, command.clone())
            .await
            .is_err()
        {
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
                    }
                }
                Err(error) => {
                    if let Some(pending) = self.pending.remove(&request.request_id) {
                        self.retire_request(&request.request_id, Instant::now());
                        let _ = pending.reply.send(Err(error));
                    }
                }
            }
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
                self.retire_request(&request_id, now);
                if let Some(session) = self.sessions.get_mut(&pending.extension_instance_id) {
                    session.remove_queued_command(&request_id);
                }
                let _ = pending.reply.send(Err(BrokerError::new(
                    BrokerErrorCode::BrowserOperationTimeout,
                    "browser operation expired before a response arrived",
                )));
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
                self.retire_request(&request_id, now);
                if let Some(session) = self.sessions.get_mut(&pending.extension_instance_id) {
                    session.remove_queued_command(&request_id);
                }
                let _ = pending.reply.send(Err(BrokerError::new(
                    BrokerErrorCode::BrowserSessionDisconnected,
                    "browser extension session disconnected while the operation was pending",
                )));
            }
        }
    }

    fn retire_request(&mut self, request_id: &str, now: Instant) {
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
                self.retire_request(&request_id, Instant::now());
                if let Some(session) = self.sessions.get_mut(extension_instance_id) {
                    session.remove_queued_command(&request_id);
                }
                let _ = pending.reply.send(Err(BrokerError {
                    code: error.code,
                    message: error.message.clone(),
                    recovery: error.recovery.clone(),
                }));
            }
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
) -> Value {
    let mut object = request
        .arguments
        .clone()
        .into_iter()
        .collect::<Map<String, Value>>();
    object.insert("type".into(), Value::String("cmd".into()));
    object.insert("cmd".into(), Value::String(request.operation.clone()));
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
    Value::Object(object)
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
            | "cleanup_browser_artifacts"
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
        "capture_browser_screenshot"
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
        "capture_browser_screenshot"
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
    use super::*;
    use crate::protocol::{ExtensionHeartbeat, ExtensionTab, ExtensionWindow, FeatureAvailability};

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
            supported_actions: vec![],
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
                stream_generation: None,
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
                stream_generation: None,
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
                stream_generation: None,
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
                stream_generation: Some(7),
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
                    stream_generation: None,
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
                stream_generation: None,
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
                stream_generation: None,
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
                stream_generation: None,
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
}
