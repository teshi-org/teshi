//! OpenAI Responses API transport, adapted to [`crate::llm::LlmEvent`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::llm::{
    apply_extra_headers, merge_chat_options, ChatMessage, LlmConfig, LlmEvent, ToolCall,
    ToolDefinition,
};

/// Build the Responses endpoint URL under the configured base URL.
pub(crate) fn responses_url(base_url: &str) -> String {
    format!("{}/responses", base_url.trim_end_matches('/'))
}

/// Convert chat history into an OpenAI Responses `input` payload.
pub(crate) fn build_responses_body(
    config: &LlmConfig,
    system: Option<String>,
    messages: &[ChatMessage],
    tools: Option<&[ToolDefinition]>,
) -> Result<Value> {
    let mut input: Vec<Value> = Vec::new();

    if let Some(sys) = system {
        input.push(json!({
            "role": "system",
            "content": sys,
        }));
    }

    for msg in messages {
        match msg.role.as_str() {
            "tool" => {
                let call_id = msg
                    .tool_call_id
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("tool message missing tool_call_id"))?;
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": msg.content,
                }));
            }
            "assistant" if msg.tool_calls.is_some() => {
                if !msg.content.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": msg.content,
                    }));
                }
                for tc in msg.tool_calls.as_ref().unwrap() {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": tc.id,
                        "name": tc.name,
                        "arguments": tc.arguments,
                    }));
                }
            }
            _ => {
                input.push(json!({
                    "role": msg.role,
                    "content": msg.content,
                }));
            }
        }
    }

    let mut body = json!({
        "model": config.model,
        "input": input,
        "stream": config.stream,
        "max_output_tokens": config.max_tokens,
    });

    if let Some(tool_defs) = tools {
        let tools_json: Vec<Value> = tool_defs
            .iter()
            .map(|td| {
                json!({
                    "type": "function",
                    "name": td.name,
                    "description": td.description,
                    "parameters": td.parameters,
                })
            })
            .collect();
        body["tools"] = Value::Array(tools_json);
    }

    let mut core = Map::new();
    core.insert("model".into(), Value::String(config.model.clone()));
    core.insert("input".into(), body["input"].clone());
    core.insert("stream".into(), Value::Bool(config.stream));
    core.insert("max_output_tokens".into(), json!(config.max_tokens));
    if body.get("tools").is_some() {
        core.insert("tools".into(), body["tools"].clone());
    }
    merge_chat_options(&mut body, &config.chat_options, &core);
    Ok(body)
}

/// Run an OpenAI Responses request and emit [`LlmEvent`]s.
pub(crate) async fn responses_request(
    config: &LlmConfig,
    system: Option<String>,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<ToolDefinition>>,
    evt_tx: &Sender<LlmEvent>,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let request_body = match build_responses_body(config, system, &messages, tools.as_deref()) {
        Ok(b) => b,
        Err(e) => {
            let _ = evt_tx.send(LlmEvent::Error {
                message: format!("cannot map chat history to Responses input: {e}"),
            });
            return Ok(());
        }
    };
    let url = responses_url(&config.base_url);

    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(e) => {
            let _ = evt_tx.send(LlmEvent::Error {
                message: format!("failed to build HTTP client: {e}"),
            });
            return Ok(());
        }
    };

    let mut builder = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json");
    if config.stream {
        builder = builder.header("Accept", "text/event-stream");
    }
    builder = apply_extra_headers(builder, &config.http_headers);

    let response = match builder.json(&request_body).send().await {
        Ok(r) => r,
        Err(e) => return Err(anyhow::anyhow!("HTTP request failed: {e}")),
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let err_msg = format!("API returned {status}: {body}");
        if status.is_server_error() {
            return Err(anyhow::anyhow!("{err_msg}"));
        }
        let _ = evt_tx.send(LlmEvent::Error { message: err_msg });
        return Ok(());
    }

    if config.stream {
        read_responses_sse(response, config.model.clone(), evt_tx, cancel).await
    } else {
        let body: Value = response
            .json()
            .await
            .context("parse OpenAI Responses JSON")?;
        match emit_responses_json(&body, evt_tx) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = evt_tx.send(LlmEvent::Error {
                    message: format!("unmappable Responses payload: {e}"),
                });
                Ok(())
            }
        }
    }
}

/// Emit events from a non-streaming Responses JSON body.
pub(crate) fn emit_responses_json(body: &Value, evt_tx: &Sender<LlmEvent>) -> Result<()> {
    let model = body["model"].as_str().unwrap_or("").to_string();
    let input_tokens = body["usage"]["input_tokens"].as_u64().map(|n| n as u32);
    let output_tokens = body["usage"]["output_tokens"].as_u64().map(|n| n as u32);

    let mut full_text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for item in body["output"].as_array().into_iter().flatten() {
        match item["type"].as_str() {
            Some("message") => {
                for part in item["content"].as_array().into_iter().flatten() {
                    if part["type"].as_str() == Some("output_text") {
                        if let Some(t) = part["text"].as_str() {
                            full_text.push_str(t);
                        }
                    }
                }
            }
            Some("function_call") => {
                let id = item["call_id"]
                    .as_str()
                    .or_else(|| item["id"].as_str())
                    .unwrap_or("")
                    .to_string();
                let name = item["name"].as_str().unwrap_or("").to_string();
                let arguments = item["arguments"].as_str().unwrap_or("{}").to_string();
                if name.is_empty() {
                    bail!("Responses function_call missing name");
                }
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                    execution_duration_ms: None,
                });
            }
            Some(other) if other.starts_with("custom") || other == "file_search_call" => {
                bail!("unsupported Responses output type '{other}'");
            }
            _ => {}
        }
    }

    if !tool_calls.is_empty() {
        if !full_text.is_empty() {
            let _ = evt_tx.send(LlmEvent::Chunk { content: full_text });
        }
        let _ = evt_tx.send(LlmEvent::ToolCallRequest {
            tool_calls,
            reasoning_content: None,
        });
        return Ok(());
    }

    if !full_text.is_empty() {
        let _ = evt_tx.send(LlmEvent::Chunk {
            content: full_text.clone(),
        });
    }
    let _ = evt_tx.send(LlmEvent::Done {
        full_text,
        reasoning_content: None,
        input_tokens,
        output_tokens,
        model,
    });
    Ok(())
}

async fn read_responses_sse(
    response: reqwest::Response,
    default_model: String,
    evt_tx: &Sender<LlmEvent>,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    use futures::StreamExt;

    let mut full_text = String::new();
    let mut model_name = default_model;
    let mut input_tokens: Option<u32> = None;
    let mut output_tokens: Option<u32> = None;
    // Responses argument events identify the output item, while tool results
    // must use the function call's call_id. Keep both identities together.
    let mut pending_tools: HashMap<String, (String, String, String)> = HashMap::new();
    let mut item_ids_by_output_index: HashMap<u64, String> = HashMap::new();
    let mut emit_error: Option<String> = None;

    let mut stream = response.bytes_stream();
    let mut buf = String::new();
    const CHUNK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

    'stream: loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = evt_tx.send(LlmEvent::Error {
                message: "Request cancelled by user".into(),
            });
            return Ok(());
        }

        let next_chunk = tokio::time::timeout(CHUNK_TIMEOUT, stream.next()).await;
        let chunk = match next_chunk {
            Ok(Some(Ok(c))) => c,
            Ok(Some(Err(e))) => return Err(anyhow::anyhow!("stream read error: {e}")),
            Ok(None) => break,
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "response stream stalled: no data received for {}s",
                    CHUNK_TIMEOUT.as_secs()
                ));
            }
        };

        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find("\n\n") {
            let event = buf[..pos].to_string();
            buf = buf[pos + 2..].to_string();

            let mut data = String::new();
            for line in event.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("data: ") {
                    data = rest.to_string();
                }
            }
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let v: Value = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let event_type = v["type"].as_str().unwrap_or("");

            match event_type {
                "response.created" | "response.in_progress" => {
                    if let Some(m) = v["response"]["model"].as_str() {
                        model_name = m.to_string();
                    }
                }
                "response.output_text.delta" => {
                    if let Some(text) = v["delta"].as_str() {
                        full_text.push_str(text);
                        let _ = evt_tx.send(LlmEvent::Chunk {
                            content: text.to_string(),
                        });
                    }
                }
                "response.output_item.added" => {
                    let item = &v["item"];
                    if item["type"].as_str() == Some("function_call") {
                        let output_index = v["output_index"].as_u64();
                        let item_id = item["id"]
                            .as_str()
                            .map(str::to_string)
                            .or_else(|| output_index.map(|index| format!("output_index:{index}")))
                            .unwrap_or_default();
                        let call_id = item["call_id"].as_str().unwrap_or("").to_string();
                        let name = item["name"].as_str().unwrap_or("").to_string();
                        if let Some(output_index) = output_index {
                            item_ids_by_output_index.insert(output_index, item_id.clone());
                        }
                        pending_tools.insert(item_id, (call_id, name, String::new()));
                    } else if let Some(t) = item["type"].as_str() {
                        if t != "message" && t != "reasoning" {
                            emit_error =
                                Some(format!("unsupported Responses stream output type '{t}'"));
                        }
                    }
                }
                "response.function_call_arguments.delta" => {
                    let item_id = v["item_id"]
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| {
                            v["output_index"]
                                .as_u64()
                                .and_then(|index| item_ids_by_output_index.get(&index).cloned())
                        })
                        .unwrap_or_default();
                    if let Some(delta) = v["delta"].as_str() {
                        if let Some(entry) = pending_tools.get_mut(&item_id) {
                            entry.2.push_str(delta);
                        }
                    }
                }
                "response.function_call_arguments.done" => {
                    let item_id = v["item_id"]
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| {
                            v["output_index"]
                                .as_u64()
                                .and_then(|index| item_ids_by_output_index.get(&index).cloned())
                        })
                        .unwrap_or_default();
                    if let Some(args) = v["arguments"].as_str() {
                        if let Some(entry) = pending_tools.get_mut(&item_id) {
                            entry.2 = args.to_string();
                        }
                    }
                }
                "response.completed" => {
                    if let Some(u) = v["response"].get("usage") {
                        input_tokens = u["input_tokens"].as_u64().map(|n| n as u32);
                        output_tokens = u["output_tokens"].as_u64().map(|n| n as u32);
                    }
                }
                "response.failed" | "error" => {
                    emit_error = Some(responses_stream_error_message(&v));
                    break 'stream;
                }
                _ => {}
            }
        }
    }

    if let Some(msg) = emit_error {
        let _ = evt_tx.send(LlmEvent::Error { message: msg });
        return Ok(());
    }

    if !pending_tools.is_empty() {
        let tool_calls: Vec<ToolCall> = pending_tools
            .into_iter()
            .map(|(item_id, (call_id, name, arguments))| {
                if name.is_empty() {
                    Err(anyhow::anyhow!(
                        "Responses function_call missing name for item_id {item_id}"
                    ))
                } else {
                    Ok(ToolCall {
                        id: if call_id.is_empty() { item_id } else { call_id },
                        name,
                        arguments: if arguments.is_empty() {
                            "{}".into()
                        } else {
                            arguments
                        },
                        execution_duration_ms: None,
                    })
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let _ = evt_tx.send(LlmEvent::ToolCallRequest {
            tool_calls,
            reasoning_content: None,
        });
    } else {
        let _ = evt_tx.send(LlmEvent::Done {
            full_text,
            reasoning_content: None,
            input_tokens,
            output_tokens,
            model: model_name,
        });
    }
    Ok(())
}

fn responses_stream_error_message(event: &Value) -> String {
    event["response"]["error"]["message"]
        .as_str()
        .or_else(|| event["error"]["message"].as_str())
        .or_else(|| event["message"].as_str())
        .unwrap_or("Responses stream failed")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_profile::{ApiStyle, PROVIDER_OPENAI};
    use std::sync::mpsc;

    fn config() -> LlmConfig {
        LlmConfig {
            api_key: "sk".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4.1".into(),
            max_tokens: 128,
            temperature: 0.7,
            context_window: None,
            provider: PROVIDER_OPENAI.into(),
            api_style: ApiStyle::Responses,
            stream: false,
            http_headers: HashMap::new(),
            chat_options: HashMap::new(),
        }
    }

    #[test]
    fn test_responses_url() {
        assert_eq!(
            responses_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/responses"
        );
    }

    #[test]
    fn test_build_responses_body_maps_tools() {
        let cfg = config();
        let tools = [ToolDefinition {
            name: "lookup".into(),
            description: "d".into(),
            parameters: json!({"type":"object"}),
        }];
        let body = build_responses_body(&cfg, None, &[], Some(&tools)).unwrap();
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "lookup");
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn test_emit_responses_function_call() {
        let (tx, rx) = mpsc::channel();
        let body = json!({
            "model": "gpt-4.1",
            "output": [{
                "type": "function_call",
                "call_id": "fc_1",
                "name": "lookup",
                "arguments": "{\"x\":1}"
            }]
        });
        emit_responses_json(&body, &tx).unwrap();
        match rx.try_recv().unwrap() {
            LlmEvent::ToolCallRequest { tool_calls, .. } => {
                assert_eq!(tool_calls[0].id, "fc_1");
                assert_eq!(tool_calls[0].name, "lookup");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn test_emit_responses_rejects_unsupported_output() {
        let (tx, _rx) = mpsc::channel();
        let body = json!({
            "output": [{ "type": "file_search_call" }]
        });
        assert!(emit_responses_json(&body, &tx).is_err());
    }

    #[tokio::test]
    async fn test_responses_bearer_and_url() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.expect("read");
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = br#"{"id":"1","model":"gpt-4.1","output":[{"type":"message","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                String::from_utf8_lossy(body)
            );
            socket.write_all(resp.as_bytes()).await.expect("write");
            req
        });

        let mut cfg = config();
        cfg.base_url = format!("http://{addr}/v1");
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        responses_request(&cfg, None, vec![], None, &tx, &cancel)
            .await
            .expect("ok");

        let captured = server.await.expect("join");
        assert!(
            captured.contains("POST /v1/responses"),
            "bad path in:\n{captured}"
        );
        assert!(
            captured
                .to_ascii_lowercase()
                .contains("authorization: bearer sk"),
            "missing bearer in:\n{captured}"
        );
        assert!(rx.try_iter().any(|e| matches!(e, LlmEvent::Done { .. })));
    }

    async fn responses_sse_events(payload: &str) -> Vec<LlmEvent> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();
        let payload = payload.to_string();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = vec![0u8; 8192];
            let _ = socket.read(&mut request).await.expect("read");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            socket.write_all(response.as_bytes()).await.expect("write");
        });

        let mut cfg = config();
        cfg.base_url = format!("http://{addr}/v1");
        cfg.stream = true;
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        responses_request(&cfg, None, vec![], None, &tx, &cancel)
            .await
            .expect("request");
        server.await.expect("join");
        rx.try_iter().collect()
    }

    #[tokio::test]
    async fn streaming_arguments_are_keyed_by_item_id() {
        let payload = concat!(
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"item_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"lookup\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"item_1\",\"output_index\":0,\"delta\":\"{\\\"x\\\":\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"item_1\",\"output_index\":0,\"delta\":\"1}\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"item_1\",\"output_index\":0,\"arguments\":\"{\\\"x\\\":1}\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n"
        );
        let events = responses_sse_events(payload).await;
        match events.as_slice() {
            [LlmEvent::ToolCallRequest { tool_calls, .. }] => {
                assert_eq!(tool_calls[0].id, "call_1");
                assert_eq!(tool_calls[0].arguments, r#"{"x":1}"#);
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    #[tokio::test]
    async fn response_failed_emits_error() {
        let payload = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"provider exploded\"}}}\n\n";
        let events = responses_sse_events(payload).await;
        assert!(matches!(
            events.as_slice(),
            [LlmEvent::Error { message }] if message == "provider exploded"
        ));
    }
}
