//! Anthropic Messages API transport, adapted to [`crate::llm::LlmEvent`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

use crate::llm::{
    apply_extra_headers, merge_chat_options, ChatMessage, LlmConfig, LlmEvent, ToolCall,
    ToolDefinition,
};

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Build Anthropic Messages URL from a resolved base URL.
pub(crate) fn anthropic_messages_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/messages")
    } else {
        format!("{base}/v1/messages")
    }
}

/// Convert chat messages + tools into an Anthropic Messages request body.
pub(crate) fn build_anthropic_body(
    config: &LlmConfig,
    system: Option<String>,
    messages: &[ChatMessage],
    tools: Option<&[ToolDefinition]>,
) -> Value {
    let mut anthropic_messages: Vec<Value> = Vec::new();
    let mut system_text = system.unwrap_or_default();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                if !system_text.is_empty() {
                    system_text.push('\n');
                }
                system_text.push_str(&msg.content);
            }
            "user" => {
                anthropic_messages.push(json!({
                    "role": "user",
                    "content": msg.content,
                }));
            }
            "assistant" => {
                if let Some(ref tcs) = msg.tool_calls {
                    let mut blocks: Vec<Value> = Vec::new();
                    if !msg.content.is_empty() {
                        blocks.push(json!({ "type": "text", "text": msg.content }));
                    }
                    for tc in tcs {
                        let input: Value = serde_json::from_str(&tc.arguments)
                            .unwrap_or_else(|_| json!({ "raw": tc.arguments }));
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": input,
                        }));
                    }
                    anthropic_messages.push(json!({
                        "role": "assistant",
                        "content": blocks,
                    }));
                } else {
                    anthropic_messages.push(json!({
                        "role": "assistant",
                        "content": msg.content,
                    }));
                }
            }
            "tool" => {
                let tool_use_id = msg.tool_call_id.clone().unwrap_or_default();
                anthropic_messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": msg.content,
                    }],
                }));
            }
            other => {
                anthropic_messages.push(json!({
                    "role": other,
                    "content": msg.content,
                }));
            }
        }
    }

    let mut body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "messages": anthropic_messages,
        "stream": config.stream,
    });
    if !system_text.is_empty() {
        body["system"] = Value::String(system_text);
    }
    if let Some(tool_defs) = tools {
        let tools_json: Vec<Value> = tool_defs
            .iter()
            .map(|td| {
                json!({
                    "name": td.name,
                    "description": td.description,
                    "input_schema": td.parameters,
                })
            })
            .collect();
        body["tools"] = Value::Array(tools_json);
    }

    let mut core = Map::new();
    core.insert("model".into(), Value::String(config.model.clone()));
    core.insert("messages".into(), body["messages"].clone());
    core.insert("max_tokens".into(), json!(config.max_tokens));
    core.insert("stream".into(), Value::Bool(config.stream));
    if body.get("system").is_some() {
        core.insert("system".into(), body["system"].clone());
    }
    if body.get("tools").is_some() {
        core.insert("tools".into(), body["tools"].clone());
    }
    merge_chat_options(&mut body, &config.chat_options, &core);
    body
}

/// Run an Anthropic Messages request and emit [`LlmEvent`]s.
pub(crate) async fn anthropic_messages_request(
    config: &LlmConfig,
    system: Option<String>,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<ToolDefinition>>,
    evt_tx: &Sender<LlmEvent>,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let request_body = build_anthropic_body(config, system, &messages, tools.as_deref());
    let url = anthropic_messages_url(&config.base_url);

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
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
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
        read_anthropic_sse(response, config.model.clone(), evt_tx, cancel).await
    } else {
        let body: Value = response
            .json()
            .await
            .context("parse Anthropic Messages JSON")?;
        emit_anthropic_json(&body, evt_tx);
        Ok(())
    }
}

/// Emit events from a non-streaming Anthropic Messages response.
pub(crate) fn emit_anthropic_json(body: &Value, evt_tx: &Sender<LlmEvent>) {
    let model = body["model"].as_str().unwrap_or("").to_string();
    let input_tokens = body["usage"]["input_tokens"].as_u64().map(|n| n as u32);
    let output_tokens = body["usage"]["output_tokens"].as_u64().map(|n| n as u32);

    let mut full_text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for block in body["content"].as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") => {
                if let Some(t) = block["text"].as_str() {
                    full_text.push_str(t);
                }
            }
            Some("tool_use") => {
                let id = block["id"].as_str().unwrap_or("").to_string();
                let name = block["name"].as_str().unwrap_or("").to_string();
                let arguments = block
                    .get("input")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "{}".into());
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                    execution_duration_ms: None,
                });
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
        return;
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
}

async fn read_anthropic_sse(
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
    // index -> (id, name, partial json)
    let mut tool_blocks: HashMap<usize, (String, String, String)> = HashMap::new();
    let mut stream_error: Option<String> = None;

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

            let mut event_type = String::new();
            let mut data = String::new();
            for line in event.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("event: ") {
                    event_type = rest.to_string();
                } else if let Some(rest) = line.strip_prefix("data: ") {
                    data = rest.to_string();
                }
            }
            if data.is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            match event_type.as_str() {
                "message_start" => {
                    if let Some(m) = v["message"]["model"].as_str() {
                        model_name = m.to_string();
                    }
                    if let Some(u) = v["message"].get("usage") {
                        input_tokens = u["input_tokens"].as_u64().map(|n| n as u32);
                    }
                }
                "content_block_start" => {
                    let index = v["index"].as_u64().unwrap_or(0) as usize;
                    let block = &v["content_block"];
                    if block["type"].as_str() == Some("tool_use") {
                        tool_blocks.insert(
                            index,
                            (
                                block["id"].as_str().unwrap_or("").to_string(),
                                block["name"].as_str().unwrap_or("").to_string(),
                                String::new(),
                            ),
                        );
                    }
                }
                "content_block_delta" => {
                    let index = v["index"].as_u64().unwrap_or(0) as usize;
                    let delta = &v["delta"];
                    if let Some(text) = delta["text"].as_str() {
                        full_text.push_str(text);
                        let _ = evt_tx.send(LlmEvent::Chunk {
                            content: text.to_string(),
                        });
                    }
                    if let Some(partial) = delta["partial_json"].as_str() {
                        if let Some(entry) = tool_blocks.get_mut(&index) {
                            entry.2.push_str(partial);
                        }
                    }
                }
                "message_delta" => {
                    if let Some(u) = v.get("usage") {
                        output_tokens = u["output_tokens"].as_u64().map(|n| n as u32);
                    }
                }
                "error" => {
                    stream_error = Some(
                        v["error"]["message"]
                            .as_str()
                            .or_else(|| v["message"].as_str())
                            .unwrap_or("Anthropic stream error")
                            .to_string(),
                    );
                    break 'stream;
                }
                "message_stop" => {}
                _ => {}
            }
        }
    }

    if let Some(message) = stream_error {
        let _ = evt_tx.send(LlmEvent::Error { message });
        return Ok(());
    }

    if !tool_blocks.is_empty() {
        let mut sorted: Vec<_> = tool_blocks.into_iter().collect();
        sorted.sort_by_key(|(idx, _)| *idx);
        let tool_calls: Vec<ToolCall> = sorted
            .into_iter()
            .map(|(_, (id, name, args))| ToolCall {
                id,
                name,
                arguments: if args.is_empty() { "{}".into() } else { args },
                execution_duration_ms: None,
            })
            .collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_profile::{ApiStyle, PROVIDER_ANTHROPIC};
    use std::sync::mpsc;

    fn config() -> LlmConfig {
        LlmConfig {
            api_key: "sk-ant".into(),
            base_url: "https://api.anthropic.com".into(),
            model: "claude-sonnet-4-5".into(),
            max_tokens: 256,
            temperature: 0.7,
            context_window: None,
            provider: PROVIDER_ANTHROPIC.into(),
            api_style: ApiStyle::ChatCompletions,
            stream: false,
            http_headers: HashMap::new(),
            chat_options: HashMap::new(),
        }
    }

    #[test]
    fn test_anthropic_url_appends_v1_messages() {
        assert_eq!(
            anthropic_messages_url("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn test_build_anthropic_body_with_tools() {
        let cfg = config();
        let tools = [ToolDefinition {
            name: "search".into(),
            description: "Search".into(),
            parameters: json!({"type":"object"}),
        }];
        let body = build_anthropic_body(&cfg, Some("sys".into()), &[], Some(&tools));
        assert_eq!(body["model"], "claude-sonnet-4-5");
        assert_eq!(body["system"], "sys");
        assert_eq!(body["tools"][0]["name"], "search");
        assert!(body["tools"][0].get("input_schema").is_some());
    }

    #[test]
    fn test_emit_anthropic_tool_use() {
        let (tx, rx) = mpsc::channel();
        let body = json!({
            "model": "claude",
            "content": [
                {"type":"text","text":"calling"},
                {"type":"tool_use","id":"tu1","name":"search","input":{"q":"x"}}
            ],
            "usage": {"input_tokens": 1, "output_tokens": 2}
        });
        emit_anthropic_json(&body, &tx);
        let events: Vec<_> = rx.try_iter().collect();
        assert!(matches!(events[0], LlmEvent::Chunk { .. }));
        match &events[1] {
            LlmEvent::ToolCallRequest { tool_calls, .. } => {
                assert_eq!(tool_calls[0].id, "tu1");
                assert_eq!(tool_calls[0].name, "search");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_anthropic_auth_headers_and_url() {
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
            let body = br#"{"id":"1","model":"claude","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                String::from_utf8_lossy(body)
            );
            socket.write_all(resp.as_bytes()).await.expect("write");
            req
        });

        let mut cfg = config();
        cfg.base_url = format!("http://{addr}");
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        anthropic_messages_request(&cfg, None, vec![], None, &tx, &cancel)
            .await
            .expect("ok");

        let captured = server.await.expect("join");
        assert!(
            captured.contains("POST /v1/messages"),
            "bad path in:\n{captured}"
        );
        assert!(
            captured.to_lowercase().contains("x-api-key: sk-ant"),
            "missing x-api-key in:\n{captured}"
        );
        assert!(
            captured.to_lowercase().contains("anthropic-version:"),
            "missing anthropic-version in:\n{captured}"
        );
        assert!(rx.try_iter().any(|e| matches!(e, LlmEvent::Done { .. })));
    }

    #[tokio::test]
    async fn anthropic_stream_error_event_emits_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();
        let payload = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"provider overloaded\"}}\n\n";
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
        cfg.base_url = format!("http://{addr}");
        cfg.stream = true;
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        anthropic_messages_request(&cfg, None, vec![], None, &tx, &cancel)
            .await
            .expect("request");
        server.await.expect("join");

        let events: Vec<_> = rx.try_iter().collect();
        assert!(matches!(
            events.as_slice(),
            [LlmEvent::Error { message }] if message == "provider overloaded"
        ));
    }
}
