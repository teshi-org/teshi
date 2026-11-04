#![cfg(feature = "contract-oracle")]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use teshi_browser_broker::protocol::{
    BROWSER_BROKER_PROTOCOL_VERSION, BROWSER_BROKER_SCHEMA_VERSION, BrowserTarget,
    ExtensionHeartbeat, ExtensionResponse, ExtensionTab, ExtensionWindow, FeatureAvailability,
    OperationRequest,
};
use teshi_browser_broker::{BrokerEvent, BrokerRuntime, BrokerServerConfig, BrokerState};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};
use tokio::time::sleep;

fn event_budget() -> OwnedSemaphorePermit {
    Arc::new(Semaphore::new(1))
        .try_acquire_owned()
        .expect("one event budget permit")
}

fn target(instance_id: &str, window_id: i64, tab_id: i64) -> BrowserTarget {
    BrowserTarget {
        extension_instance_id: instance_id.to_owned(),
        window_id,
        tab_id,
    }
}

fn heartbeat(instance_id: &str, window_id: i64, tab_id: i64) -> ExtensionHeartbeat {
    let url = format!("https://{instance_id}.example.test/");
    let tab = ExtensionTab {
        id: tab_id,
        window_id,
        title: instance_id.to_owned(),
        url: url.clone(),
        active: true,
        favicon_url: String::new(),
        debuggable: true,
    };
    ExtensionHeartbeat {
        schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
        protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
        extension_instance_id: Some(instance_id.to_owned()),
        profile_label: format!("Profile {instance_id}"),
        extension_version: "0.7.9".into(),
        features: vec![FeatureAvailability {
            feature: "p0.control".into(),
            available: true,
            reason: None,
        }],
        supported_actions: vec!["click".into(), "navigate".into()],
        supported_operations: Vec::new(),
        optional_permissions: BTreeMap::new(),
        browser: BTreeMap::from([
            ("name".into(), json!("Chromium")),
            ("version".into(), json!("140")),
            ("platform".into(), json!("test")),
        ]),
        project_root: None,
        url: url.clone(),
        title: instance_id.to_owned(),
        active_window_id: Some(window_id),
        active_tab_id: Some(tab_id),
        tabs: vec![tab.clone()],
        windows: vec![ExtensionWindow {
            id: window_id,
            focused: true,
            tabs: vec![tab],
        }],
        frame_error: String::new(),
    }
}

fn operation(
    request_id: &str,
    operation_name: &str,
    caller: &str,
    project: &str,
    target_value: Option<BrowserTarget>,
    lease_token: Option<String>,
    arguments: BTreeMap<String, Value>,
) -> OperationRequest {
    operation_with_timeout(
        request_id,
        operation_name,
        caller,
        project,
        target_value,
        lease_token,
        None,
        arguments,
    )
}

#[allow(clippy::too_many_arguments)]
fn operation_with_timeout(
    request_id: &str,
    operation_name: &str,
    caller: &str,
    project: &str,
    target_value: Option<BrowserTarget>,
    lease_token: Option<String>,
    timeout_ms: Option<u64>,
    arguments: BTreeMap<String, Value>,
) -> OperationRequest {
    OperationRequest {
        schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
        request_id: request_id.into(),
        caller_label: caller.into(),
        project_root: Some(project.into()),
        timeout_ms,
        operation: operation_name.into(),
        target: target_value,
        lease_token,
        required_feature: None,
        arguments,
    }
}

async fn heartbeat_event(
    state: &mut BrokerState,
    runtime: &BrokerRuntime,
    payload: ExtensionHeartbeat,
) -> Value {
    let (reply, receiver) = oneshot::channel();
    state
        .handle(
            BrokerEvent::Heartbeat {
                payload,
                reply,
                _budget: event_budget(),
            },
            runtime,
        )
        .await;
    receiver.await.expect("heartbeat response")
}

async fn operation_event(
    state: &mut BrokerState,
    runtime: &BrokerRuntime,
    request: OperationRequest,
) -> oneshot::Receiver<Result<Value, teshi_browser_broker::protocol::BrokerError>> {
    let (reply, receiver) = oneshot::channel();
    state
        .handle(
            BrokerEvent::Operation {
                request,
                reply,
                _budget: event_budget(),
            },
            runtime,
        )
        .await;
    receiver
}

async fn extension_response_event(
    state: &mut BrokerState,
    runtime: &BrokerRuntime,
    instance_id: &str,
    response: ExtensionResponse,
) -> Value {
    extension_response_event_at_generation(state, runtime, instance_id, None, response).await
}

async fn extension_response_event_at_generation(
    state: &mut BrokerState,
    runtime: &BrokerRuntime,
    instance_id: &str,
    generation: Option<u64>,
    response: ExtensionResponse,
) -> Value {
    let (reply, receiver) = oneshot::channel();
    state
        .handle(
            BrokerEvent::ExtensionResponse {
                extension_instance_id: instance_id.into(),
                generation,
                response,
                reply: Some(reply),
                _budget: event_budget(),
            },
            runtime,
        )
        .await;
    receiver.await.expect("extension response acknowledgement")
}

async fn connect_event(
    state: &mut BrokerState,
    runtime: &BrokerRuntime,
    instance_id: &str,
    generation: u64,
) -> Value {
    let (reply, receiver) = oneshot::channel();
    state
        .handle(
            BrokerEvent::ExtensionConnected {
                hello: teshi_browser_broker::protocol::ExtensionStreamMessage::StreamHello {
                    project_root: None,
                    extension_instance_id: instance_id.into(),
                    protocol_version: BROWSER_BROKER_PROTOCOL_VERSION,
                    extension_version: "0.7.9".into(),
                },
                generation,
                reply,
                _budget: event_budget(),
            },
            runtime,
        )
        .await;
    receiver.await.expect("extension connect acknowledgement")
}

fn lease_request(instance_id: &str, caller: &str, project: &str) -> OperationRequest {
    operation(
        &format!("lease-{instance_id}"),
        "acquire_browser_lease",
        caller,
        project,
        None,
        None,
        BTreeMap::from([
            ("extension_instance_id".into(), json!(instance_id)),
            ("owner_label".into(), json!(caller)),
            ("ttl_secs".into(), json!(30)),
        ]),
    )
}

fn snapshot_response(
    request_id: &str,
    instance_id: &str,
    window_id: i64,
    tab_id: i64,
    url: &str,
) -> ExtensionResponse {
    ExtensionResponse {
        message_type: "response".into(),
        schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
        protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
        request_id: request_id.into(),
        operation: "get_page_snapshot".into(),
        extension_instance_id: Some(instance_id.into()),
        target: Some(target(instance_id, window_id, tab_id)),
        ok: true,
        code: None,
        error: None,
        result: BTreeMap::from([("url".into(), json!(url))]),
    }
}

async fn acquire_lease_token(
    state: &mut BrokerState,
    runtime: &BrokerRuntime,
    instance_id: &str,
    caller: &str,
    project: &str,
) -> Result<String, String> {
    let receiver =
        operation_event(state, runtime, lease_request(instance_id, caller, project)).await;
    let result = receiver
        .await
        .map_err(|_| "lease response channel closed".to_owned())?
        .map_err(|error| error.code.as_str().to_owned())?;
    result["lease_token"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "lease response did not contain a token".to_owned())
}

async fn prepare_rust_pending(
    state: &mut BrokerState,
    runtime: &BrokerRuntime,
    instance_id: &str,
    request_id: &str,
    project: &str,
    caller: &str,
    timeout_ms: Option<u64>,
) -> Result<
    (
        oneshot::Receiver<Result<Value, teshi_browser_broker::protocol::BrokerError>>,
        String,
    ),
    String,
> {
    heartbeat_event(state, runtime, heartbeat(instance_id, 7, 42)).await;
    let lease_token = acquire_lease_token(state, runtime, instance_id, caller, project).await?;
    let receiver = operation_event(
        state,
        runtime,
        operation_with_timeout(
            request_id,
            "get_page_snapshot",
            caller,
            project,
            Some(target(instance_id, 7, 42)),
            Some(lease_token.clone()),
            timeout_ms,
            BTreeMap::new(),
        ),
    )
    .await;
    let _ = heartbeat_event(state, runtime, heartbeat(instance_id, 7, 42)).await;
    Ok((receiver, lease_token))
}

async fn profile_response_race(
    state: &mut BrokerState,
    state_fixture: &Value,
    runtime: &BrokerRuntime,
) -> Result<Value, String> {
    let scenario = &state_fixture["profile_response_race"];
    let requests = scenario["requests"]
        .as_array()
        .ok_or_else(|| "profile_response_race.requests must be an array".to_owned())?;
    let mut leases = BTreeMap::new();
    let mut receivers = BTreeMap::new();

    for item in requests {
        let instance_id = item["extension_instance_id"]
            .as_str()
            .ok_or_else(|| "request extension_instance_id is missing".to_owned())?;
        let window_id = item["window_id"]
            .as_i64()
            .ok_or_else(|| "request window_id is missing".to_owned())?;
        let tab_id = item["tab_id"]
            .as_i64()
            .ok_or_else(|| "request tab_id is missing".to_owned())?;
        let caller = format!("agent-{instance_id}");
        let project = format!("project-{instance_id}");
        heartbeat_event(state, runtime, heartbeat(instance_id, window_id, tab_id)).await;
        let lease_receiver = operation_event(
            state,
            runtime,
            lease_request(instance_id, &caller, &project),
        )
        .await;
        let lease_result = lease_receiver
            .await
            .map_err(|_| "lease response channel closed".to_owned())?
            .map_err(|error| error.code.as_str().to_owned())?;
        let lease_token = lease_result["lease_token"]
            .as_str()
            .ok_or_else(|| "lease response did not contain a token".to_owned())?
            .to_owned();
        leases.insert(instance_id.to_owned(), lease_token.clone());

        let request_id = item["request_id"]
            .as_str()
            .ok_or_else(|| "request_id is missing".to_owned())?;
        let request = operation(
            request_id,
            "get_page_snapshot",
            &caller,
            &project,
            Some(target(instance_id, window_id, tab_id)),
            Some(lease_token),
            BTreeMap::new(),
        );
        receivers.insert(
            request_id.to_owned(),
            operation_event(state, runtime, request).await,
        );
    }

    for request_id in scenario["response_order"]
        .as_array()
        .ok_or_else(|| "response_order must be an array".to_owned())?
    {
        let request_id = request_id
            .as_str()
            .ok_or_else(|| "response_order item must be a string".to_owned())?;
        let item = requests
            .iter()
            .find(|item| item["request_id"].as_str() == Some(request_id))
            .ok_or_else(|| format!("response {request_id} has no request"))?;
        let instance_id = item["extension_instance_id"]
            .as_str()
            .ok_or_else(|| "response extension_instance_id is missing".to_owned())?;
        let window_id = item["window_id"]
            .as_i64()
            .ok_or_else(|| "response window_id is missing".to_owned())?;
        let tab_id = item["tab_id"]
            .as_i64()
            .ok_or_else(|| "response tab_id is missing".to_owned())?;
        let response = ExtensionResponse {
            message_type: "response".into(),
            schema_version: Some(BROWSER_BROKER_SCHEMA_VERSION),
            protocol_version: Some(BROWSER_BROKER_PROTOCOL_VERSION),
            request_id: request_id.into(),
            operation: "get_page_snapshot".into(),
            extension_instance_id: Some(instance_id.into()),
            target: Some(target(instance_id, window_id, tab_id)),
            ok: true,
            code: None,
            error: None,
            result: BTreeMap::from([("url".into(), item["result_url"].clone())]),
        };
        extension_response_event(state, runtime, instance_id, response).await;
    }

    let mut completed = Vec::new();
    for item in requests {
        let request_id = item["request_id"]
            .as_str()
            .ok_or_else(|| "request_id is missing".to_owned())?;
        let value = receivers
            .remove(request_id)
            .ok_or_else(|| format!("missing receiver for {request_id}"))?
            .await
            .map_err(|_| format!("operation {request_id} channel closed"))?
            .map_err(|error| error.code.as_str().to_owned())?;
        completed.push(json!({
            "request_id": request_id,
            "extension_instance_id": value["extension_instance_id"],
            "url": value["url"],
        }));
    }
    completed.sort_by(|left, right| {
        left["request_id"]
            .as_str()
            .cmp(&right["request_id"].as_str())
    });

    let mut queued_commands = BTreeMap::new();
    for instance_id in leases.keys() {
        let _ = heartbeat_event(state, runtime, heartbeat(instance_id, 7, 42)).await;
        let count = state
            .sessions
            .get(instance_id)
            .map(|session| session.queued_command_count())
            .unwrap_or_default();
        queued_commands.insert(instance_id.clone(), count);
    }
    Ok(json!({
        "completed": completed,
        "queued_commands": queued_commands,
        "session_count": state.sessions.len(),
    }))
}

async fn lease_scope_oracle(
    state: &mut BrokerState,
    state_fixture: &Value,
    runtime: &BrokerRuntime,
) -> Result<Value, String> {
    let scenario = &state_fixture["lease_scope_isolation"];
    heartbeat_event(state, runtime, heartbeat("profile-a", 7, 42)).await;
    heartbeat_event(state, runtime, heartbeat("profile-b", 7, 42)).await;
    let owner = &scenario["owner"];
    let owner_profile = owner["extension_instance_id"]
        .as_str()
        .ok_or_else(|| "lease owner profile is missing".to_owned())?;
    let owner_project = owner["project_root"]
        .as_str()
        .ok_or_else(|| "lease owner project is missing".to_owned())?;
    let owner_caller = owner["caller_label"]
        .as_str()
        .ok_or_else(|| "lease owner caller is missing".to_owned())?;
    let lease_token =
        acquire_lease_token(state, runtime, owner_profile, owner_caller, owner_project).await?;

    let mut errors = Vec::new();
    for mismatch in scenario["mismatches"]
        .as_array()
        .ok_or_else(|| "lease mismatches must be an array".to_owned())?
    {
        let name = mismatch["name"]
            .as_str()
            .ok_or_else(|| "lease mismatch name is missing".to_owned())?;
        let instance_id = mismatch["extension_instance_id"]
            .as_str()
            .ok_or_else(|| "lease mismatch profile is missing".to_owned())?;
        let project = mismatch["project_root"]
            .as_str()
            .ok_or_else(|| "lease mismatch project is missing".to_owned())?;
        let caller = mismatch["caller_label"]
            .as_str()
            .ok_or_else(|| "lease mismatch caller is missing".to_owned())?;
        let receiver = operation_event(
            state,
            runtime,
            operation(
                &format!("lease-scope-{name}"),
                "get_page_snapshot",
                caller,
                project,
                Some(target(instance_id, 7, 42)),
                Some(lease_token.clone()),
                BTreeMap::new(),
            ),
        )
        .await;
        let error = receiver
            .await
            .map_err(|_| format!("lease mismatch {name} channel closed"))?
            .expect_err("lease mismatch unexpectedly authorized");
        errors.push(json!({"name": name, "error": error.code.as_str()}));
    }
    let queued_commands = ["profile-a", "profile-b"]
        .into_iter()
        .map(|instance_id| {
            state
                .sessions
                .get(instance_id)
                .map(|session| session.queued_command_count())
                .unwrap_or_default()
        })
        .sum::<usize>();
    Ok(json!({
        "errors": errors,
        "queued_commands": queued_commands,
        "session_count": state.sessions.len(),
    }))
}

fn cancel_operation(
    request_id: &str,
    cancel_request_id: &str,
    caller: &str,
    project: &str,
) -> OperationRequest {
    operation(
        request_id,
        "cancel_browser_request",
        caller,
        project,
        None,
        None,
        BTreeMap::from([("cancel_request_id".into(), json!(cancel_request_id))]),
    )
}

fn result_code(result: Result<Value, teshi_browser_broker::protocol::BrokerError>) -> String {
    match result {
        Ok(value) => {
            if value.get("ok").and_then(Value::as_bool) == Some(true) {
                "ok".into()
            } else {
                value["code"].as_str().unwrap_or("unknown").into()
            }
        }
        Err(error) => error.code.as_str().into(),
    }
}

async fn cancel_response_oracle(
    state_fixture: &Value,
    runtime: &BrokerRuntime,
) -> Result<Value, String> {
    let scenario = &state_fixture["cancel_response_race"];
    let instance_id = scenario["extension_instance_id"]
        .as_str()
        .ok_or_else(|| "cancel profile is missing".to_owned())?;
    let project = scenario["project_root"]
        .as_str()
        .ok_or_else(|| "cancel project is missing".to_owned())?;
    let caller = scenario["caller_label"]
        .as_str()
        .ok_or_else(|| "cancel caller is missing".to_owned())?;
    let window_id = scenario["window_id"]
        .as_i64()
        .ok_or_else(|| "cancel window is missing".to_owned())?;
    let tab_id = scenario["tab_id"]
        .as_i64()
        .ok_or_else(|| "cancel tab is missing".to_owned())?;
    let mut response_state = BrokerState::new();
    let (response_receiver, _) = prepare_rust_pending(
        &mut response_state,
        runtime,
        instance_id,
        scenario["response_first_request_id"]
            .as_str()
            .ok_or_else(|| "response-first request is missing".to_owned())?,
        project,
        caller,
        None,
    )
    .await?;
    let response_id = scenario["response_first_request_id"].as_str().unwrap();
    let _ = extension_response_event(
        &mut response_state,
        runtime,
        instance_id,
        snapshot_response(
            response_id,
            instance_id,
            window_id,
            tab_id,
            scenario["response_url"].as_str().unwrap(),
        ),
    )
    .await;
    let response_result = result_code(
        response_receiver
            .await
            .map_err(|_| "response-first channel closed".to_owned())?,
    );
    let cancel_after_response_receiver = operation_event(
        &mut response_state,
        runtime,
        cancel_operation("cancel-after-response", response_id, caller, project),
    )
    .await;
    let cancel_after_response = result_code(
        cancel_after_response_receiver
            .await
            .map_err(|_| "cancel-after-response channel closed".to_owned())?,
    );

    let mut cancel_state = BrokerState::new();
    let (cancel_receiver, _) = prepare_rust_pending(
        &mut cancel_state,
        runtime,
        instance_id,
        scenario["cancel_first_request_id"]
            .as_str()
            .ok_or_else(|| "cancel-first request is missing".to_owned())?,
        project,
        caller,
        None,
    )
    .await?;
    let cancel_id = scenario["cancel_first_request_id"].as_str().unwrap();
    let cancel_reply = operation_event(
        &mut cancel_state,
        runtime,
        cancel_operation("cancel-first-command", cancel_id, caller, project),
    )
    .await
    .await
    .map_err(|_| "cancel-first acknowledgement channel closed".to_owned())?;
    let cancel_ack = match &cancel_reply {
        Ok(value) if value["cancelled"] == true => "cancelled".to_owned(),
        _ => result_code(cancel_reply),
    };
    let cancelled_operation = result_code(
        cancel_receiver
            .await
            .map_err(|_| "cancelled operation channel closed".to_owned())?,
    );
    let late_ack = extension_response_event(
        &mut cancel_state,
        runtime,
        instance_id,
        snapshot_response(
            cancel_id,
            instance_id,
            window_id,
            tab_id,
            scenario["response_url"].as_str().unwrap(),
        ),
    )
    .await;
    let late_response = late_ack["code"].as_str().unwrap_or("ok");

    Ok(json!({
        "response_first": {
            "operation": response_result,
            "cancel": cancel_after_response,
        },
        "cancel_first": {
            "operation": cancelled_operation,
            "cancel": cancel_ack,
            "late_response": late_response,
        },
    }))
}

async fn timeout_oracle(state_fixture: &Value, runtime: &BrokerRuntime) -> Result<Value, String> {
    let scenario = &state_fixture["timeout_late_response"];
    let instance_id = scenario["extension_instance_id"]
        .as_str()
        .ok_or_else(|| "timeout profile is missing".to_owned())?;
    let request_id = scenario["request_id"]
        .as_str()
        .ok_or_else(|| "timeout request is missing".to_owned())?;
    let project = scenario["project_root"]
        .as_str()
        .ok_or_else(|| "timeout project is missing".to_owned())?;
    let caller = scenario["caller_label"]
        .as_str()
        .ok_or_else(|| "timeout caller is missing".to_owned())?;
    let timeout_ms = scenario["timeout_ms"]
        .as_u64()
        .ok_or_else(|| "timeout_ms is missing".to_owned())?;
    let mut state = BrokerState::new();
    let (receiver, _) = prepare_rust_pending(
        &mut state,
        runtime,
        instance_id,
        request_id,
        project,
        caller,
        Some(timeout_ms),
    )
    .await?;
    sleep(Duration::from_millis(timeout_ms + 20)).await;
    let _ = heartbeat_event(&mut state, runtime, heartbeat(instance_id, 7, 42)).await;
    let timeout_result = result_code(
        receiver
            .await
            .map_err(|_| "timeout operation channel closed".to_owned())?,
    );
    let late_ack = extension_response_event(
        &mut state,
        runtime,
        instance_id,
        snapshot_response(request_id, instance_id, 7, 42, "https://late.example.test/"),
    )
    .await;
    let late_response = late_ack["code"].as_str().unwrap_or("ok");
    let queued_commands = state
        .sessions
        .get(instance_id)
        .map(|session| session.queued_command_count())
        .unwrap_or_default();
    Ok(json!({
        "timeout": timeout_result,
        "late_response": late_response,
        "queued_commands": queued_commands,
    }))
}

async fn generation_oracle(
    state_fixture: &Value,
    runtime: &BrokerRuntime,
) -> Result<Value, String> {
    let scenario = &state_fixture["generation_reconnect"];
    let instance_id = scenario["extension_instance_id"]
        .as_str()
        .ok_or_else(|| "generation profile is missing".to_owned())?;
    let request_id = scenario["request_id"]
        .as_str()
        .ok_or_else(|| "generation request is missing".to_owned())?;
    let project = scenario["project_root"]
        .as_str()
        .ok_or_else(|| "generation project is missing".to_owned())?;
    let caller = scenario["caller_label"]
        .as_str()
        .ok_or_else(|| "generation caller is missing".to_owned())?;
    let old_generation = scenario["old_generation"]
        .as_u64()
        .ok_or_else(|| "old generation is missing".to_owned())?;
    let new_generation = scenario["new_generation"]
        .as_u64()
        .ok_or_else(|| "new generation is missing".to_owned())?;
    let mut state = BrokerState::new();
    heartbeat_event(&mut state, runtime, heartbeat(instance_id, 7, 42)).await;
    let mut old_sink = runtime
        .contract_attach_extension_stream(instance_id, old_generation)
        .await;
    let old_connect = connect_event(&mut state, runtime, instance_id, old_generation).await;
    let lease_token =
        acquire_lease_token(&mut state, runtime, instance_id, caller, project).await?;
    let receiver = operation_event(
        &mut state,
        runtime,
        operation(
            request_id,
            "get_page_snapshot",
            caller,
            project,
            Some(target(instance_id, 7, 42)),
            Some(lease_token.clone()),
            BTreeMap::new(),
        ),
    )
    .await;
    let _ = old_sink
        .recv()
        .await
        .ok_or_else(|| "old generation stream did not receive command".to_owned())?;
    state
        .handle(
            BrokerEvent::ExtensionDisconnected {
                extension_instance_id: instance_id.into(),
                generation: old_generation,
            },
            runtime,
        )
        .await;
    let disconnected = result_code(
        receiver
            .await
            .map_err(|_| "disconnected operation channel closed".to_owned())?,
    );
    let old_response = extension_response_event_at_generation(
        &mut state,
        runtime,
        instance_id,
        Some(old_generation),
        snapshot_response(
            request_id,
            instance_id,
            7,
            42,
            "https://old-generation.example.test/",
        ),
    )
    .await;
    let old_generation_response = old_response["code"].as_str().unwrap_or("ok");

    let _new_sink = runtime
        .contract_attach_extension_stream(instance_id, new_generation)
        .await;
    let new_connect = connect_event(&mut state, runtime, instance_id, new_generation).await;
    let reused_receiver = operation_event(
        &mut state,
        runtime,
        operation(
            request_id,
            "get_page_snapshot",
            caller,
            project,
            Some(target(instance_id, 7, 42)),
            Some(lease_token),
            BTreeMap::new(),
        ),
    )
    .await;
    let reused_request = result_code(
        reused_receiver
            .await
            .map_err(|_| "reused request channel closed".to_owned())?,
    );
    runtime.contract_detach_extension_stream(instance_id).await;
    Ok(json!({
        "disconnect": disconnected,
        "old_generation_response": old_generation_response,
        "reused_request": reused_request,
        "reconnected": old_connect["ok"] == true && new_connect["ok"] == true,
        "session_count": state.sessions.len(),
    }))
}

async fn queue_fairness_oracle(
    state_fixture: &Value,
    runtime: &BrokerRuntime,
) -> Result<Value, String> {
    let scenario = &state_fixture["queue_fairness"];
    let blocked_profile = scenario["blocked_profile"]
        .as_str()
        .ok_or_else(|| "blocked profile is missing".to_owned())?;
    let healthy_profile = scenario["healthy_profile"]
        .as_str()
        .ok_or_else(|| "healthy profile is missing".to_owned())?;
    let project = scenario["project_root"]
        .as_str()
        .ok_or_else(|| "queue project is missing".to_owned())?;
    let caller = scenario["caller_label"]
        .as_str()
        .ok_or_else(|| "queue caller is missing".to_owned())?;
    let mut state = BrokerState::new();
    heartbeat_event(&mut state, runtime, heartbeat(blocked_profile, 7, 42)).await;
    heartbeat_event(&mut state, runtime, heartbeat(healthy_profile, 7, 42)).await;
    let blocked_lease =
        acquire_lease_token(&mut state, runtime, blocked_profile, caller, project).await?;
    let healthy_lease = acquire_lease_token(
        &mut state,
        runtime,
        healthy_profile,
        "caller-b",
        "project-b",
    )
    .await?;
    let mut filler_index = 0;
    loop {
        let result = state
            .sessions
            .get_mut(blocked_profile)
            .ok_or_else(|| "blocked profile disappeared".to_owned())?
            .queue_command(json!({"request_id": format!("queue-filler-{filler_index}")}));
        if result.is_err() {
            break;
        }
        filler_index += 1;
    }
    let blocked_receiver = operation_event(
        &mut state,
        runtime,
        operation(
            scenario["blocked_request_id"]
                .as_str()
                .ok_or_else(|| "blocked request is missing".to_owned())?,
            "get_page_snapshot",
            caller,
            project,
            Some(target(blocked_profile, 7, 42)),
            Some(blocked_lease),
            BTreeMap::new(),
        ),
    )
    .await;
    let blocked_error = result_code(
        blocked_receiver
            .await
            .map_err(|_| "blocked operation channel closed".to_owned())?,
    );

    let healthy_id = scenario["healthy_request_id"]
        .as_str()
        .ok_or_else(|| "healthy request is missing".to_owned())?;
    let healthy_receiver = operation_event(
        &mut state,
        runtime,
        operation(
            healthy_id,
            "get_page_snapshot",
            "caller-b",
            "project-b",
            Some(target(healthy_profile, 7, 42)),
            Some(healthy_lease),
            BTreeMap::new(),
        ),
    )
    .await;
    let _ = heartbeat_event(&mut state, runtime, heartbeat(healthy_profile, 7, 42)).await;
    let _ = extension_response_event(
        &mut state,
        runtime,
        healthy_profile,
        snapshot_response(
            healthy_id,
            healthy_profile,
            7,
            42,
            scenario["healthy_url"]
                .as_str()
                .ok_or_else(|| "healthy URL is missing".to_owned())?,
        ),
    )
    .await;
    let healthy_value = healthy_receiver
        .await
        .map_err(|_| "healthy operation channel closed".to_owned())?
        .map_err(|error| error.code.as_str().to_owned())?;
    let healthy_dispatched = healthy_value["url"] == scenario["healthy_url"];
    let healthy_queue_empty = state
        .sessions
        .get(healthy_profile)
        .map(|session| session.queued_command_count() == 0)
        .unwrap_or(false);
    Ok(json!({
        "blocked_error": blocked_error,
        "healthy_dispatched": healthy_dispatched,
        "healthy_queue_empty": healthy_queue_empty,
    }))
}

async fn ambiguous_implicit_target(
    state: &mut BrokerState,
    state_fixture: &Value,
    runtime: &BrokerRuntime,
) -> Result<Value, String> {
    let scenario = &state_fixture["ambiguous_implicit_target"];
    for item in scenario["targets"]
        .as_array()
        .ok_or_else(|| "targets must be an array".to_owned())?
    {
        let instance_id = item["extension_instance_id"]
            .as_str()
            .ok_or_else(|| "target extension_instance_id is missing".to_owned())?;
        let window_id = item["window_id"]
            .as_i64()
            .ok_or_else(|| "target window_id is missing".to_owned())?;
        let tab_id = item["tab_id"]
            .as_i64()
            .ok_or_else(|| "target tab_id is missing".to_owned())?;
        heartbeat_event(state, runtime, heartbeat(instance_id, window_id, tab_id)).await;
    }
    let receiver = operation_event(
        state,
        runtime,
        operation(
            "ambiguous-request",
            "get_page_snapshot",
            "agent-ambiguous",
            "project-ambiguous",
            None,
            None,
            BTreeMap::new(),
        ),
    )
    .await;
    let error = receiver
        .await
        .map_err(|_| "ambiguous response channel closed".to_owned())?
        .expect_err("implicit target must be rejected");
    let queued_commands = scenario["targets"]
        .as_array()
        .map(|targets| {
            targets
                .iter()
                .map(|item| {
                    item["extension_instance_id"]
                        .as_str()
                        .and_then(|instance_id| {
                            state
                                .sessions
                                .get(instance_id)
                                .map(|session| session.queued_command_count())
                        })
                        .unwrap_or_default()
                })
                .sum::<usize>()
        })
        .unwrap_or_default();
    Ok(json!({
        "error": error.code.as_str(),
        "queued_commands": queued_commands,
        "session_count": state.sessions.len(),
    }))
}

async fn run_oracles(fixture: &Value, runtime: &BrokerRuntime) -> Result<Value, String> {
    let mut profile_state = BrokerState::new();
    let profile_response_race =
        profile_response_race(&mut profile_state, &fixture["stateful"], runtime).await?;
    let mut ambiguous_state = BrokerState::new();
    let ambiguous_implicit_target =
        ambiguous_implicit_target(&mut ambiguous_state, &fixture["stateful"], runtime).await?;
    let mut lease_state = BrokerState::new();
    let lease_scope_isolation =
        lease_scope_oracle(&mut lease_state, &fixture["stateful"], runtime).await?;
    let cancel_response = cancel_response_oracle(&fixture["stateful"], runtime).await?;
    let timeout = timeout_oracle(&fixture["stateful"], runtime).await?;
    let generation = generation_oracle(&fixture["stateful"], runtime).await?;
    let queue_fairness = queue_fairness_oracle(&fixture["stateful"], runtime).await?;
    Ok(json!({
        "profile_response_race": profile_response_race,
        "ambiguous_implicit_target": ambiguous_implicit_target,
        "lease_scope_isolation": lease_scope_isolation,
        "cancel_response_race": cancel_response,
        "timeout_late_response": timeout,
        "generation_reconnect": generation,
        "queue_fairness": queue_fairness,
    }))
}

#[tokio::main]
async fn main() {
    let fixture_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "resources/browser_contract_fixtures.json".into());
    let fixture: Value = match fs::read_to_string(&fixture_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
    {
        Some(fixture) => fixture,
        None => {
            eprintln!("cannot read contract fixture: {fixture_path}");
            std::process::exit(2);
        }
    };
    let mut config = BrokerServerConfig::new("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    config.discovery_addr = "127.0.0.1:0".parse().expect("loopback address");
    config.broker_features = vec!["transport.v1".into(), "p0.control".into()];
    let runtime = match BrokerRuntime::start(config).await {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("cannot start test-only broker runtime: {error:?}");
            std::process::exit(2);
        }
    };

    let output = match run_oracles(&fixture, &runtime).await {
        Ok(output) => output,
        Err(error) => {
            eprintln!("contract oracle failed: {error}");
            runtime.shutdown().await;
            std::process::exit(2);
        }
    };
    runtime.shutdown().await;
    println!("{}", serde_json::to_string(&output).expect("oracle JSON"));
}
