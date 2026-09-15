use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{Mutex, mpsc, oneshot},
};

use crate::error::{AcpError, AcpResult};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(u64),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsonRpcMessage {
    Request {
        id: JsonRpcId,
        method: String,
        params: Value,
    },
    Response {
        id: JsonRpcId,
        result: Value,
    },
    Error {
        id: Option<JsonRpcId>,
        error: JsonRpcError,
    },
    Notification {
        method: String,
        params: Value,
    },
}

pub fn parse_line(line: &str) -> AcpResult<Option<JsonRpcMessage>> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.trim().is_empty() {
        return Ok(None);
    }
    let value: Value =
        serde_json::from_str(line).map_err(|e| AcpError::ProtocolDecode(e.to_string()))?;
    parse_value(value).map(Some)
}

pub fn parse_value(value: Value) -> AcpResult<JsonRpcMessage> {
    let obj = value
        .as_object()
        .ok_or_else(|| AcpError::ProtocolDecode("JSON-RPC message must be an object".into()))?;
    if obj.get("jsonrpc") != Some(&Value::String("2.0".into())) {
        return Err(AcpError::ProtocolViolation("jsonrpc must equal 2.0".into()));
    }
    if let Some(method) = obj.get("method").and_then(Value::as_str) {
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        if let Some(id) = obj.get("id") {
            Ok(JsonRpcMessage::Request {
                id: parse_id(id)?,
                method: method.to_string(),
                params,
            })
        } else {
            Ok(JsonRpcMessage::Notification {
                method: method.to_string(),
                params,
            })
        }
    } else if let Some(error) = obj.get("error") {
        let id = obj
            .get("id")
            .filter(|v| !v.is_null())
            .map(parse_id)
            .transpose()?;
        let error: JsonRpcError = serde_json::from_value(error.clone())
            .map_err(|e| AcpError::ProtocolDecode(e.to_string()))?;
        Ok(JsonRpcMessage::Error { id, error })
    } else if obj.contains_key("result") {
        let id = obj
            .get("id")
            .ok_or_else(|| AcpError::ProtocolViolation("response missing id".into()))
            .and_then(parse_id)?;
        Ok(JsonRpcMessage::Response {
            id,
            result: obj.get("result").cloned().unwrap_or(Value::Null),
        })
    } else {
        Err(AcpError::ProtocolViolation(
            "message is neither request, response, nor notification".into(),
        ))
    }
}

fn parse_id(value: &Value) -> AcpResult<JsonRpcId> {
    if let Some(n) = value.as_u64() {
        return Ok(JsonRpcId::Number(n));
    }
    if let Some(s) = value.as_str() {
        return Ok(JsonRpcId::String(s.to_string()));
    }
    Err(AcpError::ProtocolViolation("invalid JSON-RPC id".into()))
}

pub fn request(id: JsonRpcId, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

pub fn response(id: JsonRpcId, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error_response(id: Option<JsonRpcId>, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

pub async fn write_message<W: AsyncWrite + Unpin>(writer: &mut W, value: &Value) -> AcpResult<()> {
    let mut bytes =
        serde_json::to_vec(value).map_err(|e| AcpError::ProtocolDecode(e.to_string()))?;
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .await
        .map_err(|_| AcpError::ProcessExited)?;
    writer.flush().await.map_err(|_| AcpError::ProcessExited)
}

pub async fn read_next<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> AcpResult<Option<JsonRpcMessage>> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| AcpError::ProtocolDecode(e.to_string()))?;
        if n == 0 {
            return Ok(None);
        }
        if let Some(msg) = parse_line(&line)? {
            return Ok(Some(msg));
        }
    }
}

pub struct JsonRpcPeer<W> {
    writer: Arc<Mutex<W>>,
    pending: Arc<Mutex<HashMap<JsonRpcId, oneshot::Sender<AcpResult<Value>>>>>,
    next_id: Arc<std::sync::atomic::AtomicU64>,
}

impl<W> Clone for JsonRpcPeer<W> {
    fn clone(&self) -> Self {
        Self {
            writer: Arc::clone(&self.writer),
            pending: Arc::clone(&self.pending),
            next_id: Arc::clone(&self.next_id),
        }
    }
}

impl<W: AsyncWrite + Unpin + Send + 'static> JsonRpcPeer<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer: Arc::new(Mutex::new(writer)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    pub async fn request(&self, method: &str, params: Value) -> AcpResult<Value> {
        let (_id, rx) = self.send_request(method, params).await?;
        rx.await.unwrap_or(Err(AcpError::ProcessExited))
    }

    pub async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: std::time::Duration,
    ) -> AcpResult<Value> {
        let (id, rx) = self.send_request(method, params).await?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(result) => result.unwrap_or(Err(AcpError::ProcessExited)),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(AcpError::Timeout(timeout))
            }
        }
    }

    pub(crate) async fn send_request(
        &self,
        method: &str,
        params: Value,
    ) -> AcpResult<(JsonRpcId, oneshot::Receiver<AcpResult<Value>>)> {
        let id = JsonRpcId::Number(
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        );
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        let msg = request(id.clone(), method, params);
        let write_result = {
            let mut writer = self.writer.lock().await;
            write_message(&mut *writer, &msg).await
        };
        if let Err(e) = write_result {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }
        Ok((id, rx))
    }

    pub async fn notify(&self, method: &str, params: Value) -> AcpResult<()> {
        let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let mut writer = self.writer.lock().await;
        write_message(&mut *writer, &msg).await
    }

    pub async fn close(&self) -> AcpResult<()> {
        self.writer
            .lock()
            .await
            .shutdown()
            .await
            .map_err(|_| AcpError::ProcessExited)
    }

    pub async fn respond(&self, id: JsonRpcId, result: Value) -> AcpResult<()> {
        let mut writer = self.writer.lock().await;
        write_message(&mut *writer, &response(id, result)).await
    }

    pub async fn respond_error(
        &self,
        id: Option<JsonRpcId>,
        code: i64,
        message: impl Into<String>,
    ) -> AcpResult<()> {
        let mut writer = self.writer.lock().await;
        write_message(&mut *writer, &error_response(id, code, message)).await
    }

    pub async fn complete_response(&self, id: &JsonRpcId, value: AcpResult<Value>) -> bool {
        if let Some(tx) = self.pending.lock().await.remove(id) {
            let _ = tx.send(value);
            true
        } else {
            false
        }
    }

    pub async fn fail_all(&self, err: AcpError) {
        let mut pending = self.pending.lock().await;
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(match &err {
                AcpError::ProcessExited => AcpError::ProcessExited,
                AcpError::Cancelled => AcpError::Cancelled,
                _ => AcpError::ProtocolViolation(err.to_string()),
            }));
        }
    }
}

pub async fn reader_loop<R, W>(
    reader: R,
    peer: JsonRpcPeer<W>,
    inbound: mpsc::Sender<JsonRpcMessage>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut reader = BufReader::new(reader);
    loop {
        match read_next(&mut reader).await {
            Ok(Some(JsonRpcMessage::Response { id, result })) => {
                let _ = peer.complete_response(&id, Ok(result)).await;
            }
            Ok(Some(JsonRpcMessage::Error {
                id: Some(id),
                error,
            })) => {
                let response_error = if error.code == -32000 {
                    AcpError::AuthenticationRequired
                } else {
                    AcpError::ProtocolViolation(error.message)
                };
                let _ = peer.complete_response(&id, Err(response_error)).await;
            }
            Ok(Some(msg)) => {
                if inbound.send(msg).await.is_err() {
                    break;
                }
            }
            Ok(None) => {
                peer.fail_all(AcpError::ProcessExited).await;
                break;
            }
            Err(e) => {
                peer.fail_all(e).await;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn parses_complete_and_empty_and_malformed() {
        assert!(parse_line("\n").unwrap().is_none());
        assert!(matches!(
            parse_line(r#"{"jsonrpc":"2.0","method":"x"}"#).unwrap(),
            Some(JsonRpcMessage::Notification { .. })
        ));
        assert!(parse_line("not-json").is_err());
        assert!(parse_line("[]").is_err());
    }

    #[tokio::test]
    async fn reads_split_and_multiple_lines() {
        let (mut w, r) = tokio::io::duplex(1024);
        let task = tokio::spawn(async move {
            w.write_all(br#"{"jsonrpc":"2.0","method":"a"}"#)
                .await
                .unwrap();
            w.write_all(b"\n").await.unwrap();
            w.write_all(
                br#"{"jsonrpc":"2.0","method":"b"}
"#,
            )
            .await
            .unwrap();
        });
        let mut reader = BufReader::new(r);
        let a = read_next(&mut reader).await.unwrap().unwrap();
        let b = read_next(&mut reader).await.unwrap().unwrap();
        task.await.unwrap();
        assert!(matches!(a, JsonRpcMessage::Notification { method, .. } if method == "a"));
        assert!(matches!(b, JsonRpcMessage::Notification { method, .. } if method == "b"));
    }

    #[tokio::test]
    async fn timed_out_request_is_removed_from_pending() {
        let (client_io, _server_io) = tokio::io::duplex(4096);
        let (_r, w) = tokio::io::split(client_io);
        let peer = JsonRpcPeer::new(w);

        assert!(matches!(
            peer.request_with_timeout("slow", Value::Null, std::time::Duration::from_millis(1))
                .await,
            Err(AcpError::Timeout(_))
        ));
        assert!(peer.pending.lock().await.is_empty());
    }

    #[tokio::test]
    async fn correlates_out_of_order_and_unknown() {
        let (client_io, mut server_io) = tokio::io::duplex(4096);
        let (r, w) = tokio::io::split(client_io);
        let peer = JsonRpcPeer::new(w);
        let (tx, _rx) = mpsc::channel(4);
        tokio::spawn(reader_loop(r, peer.clone(), tx));
        let a = tokio::spawn({
            let peer = peer.clone();
            async move { peer.request("a", Value::Null).await.unwrap() }
        });
        let b = tokio::spawn({
            let peer = peer.clone();
            async move { peer.request("b", Value::Null).await.unwrap() }
        });
        let mut lines = BufReader::new(&mut server_io);
        let mut l1 = String::new();
        let mut l2 = String::new();
        lines.read_line(&mut l1).await.unwrap();
        lines.read_line(&mut l2).await.unwrap();
        let v1: Value = serde_json::from_str(&l1).unwrap();
        let v2: Value = serde_json::from_str(&l2).unwrap();
        let id1 = v1["id"].clone();
        let id2 = v2["id"].clone();
        drop(lines);
        server_io
            .write_all(format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":2}}\n", id2).as_bytes())
            .await
            .unwrap();
        server_io
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":999,\"result\":0}\n")
            .await
            .unwrap();
        server_io
            .write_all(format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":1}}\n", id1).as_bytes())
            .await
            .unwrap();
        assert_eq!(a.await.unwrap(), json!(1));
        assert_eq!(b.await.unwrap(), json!(2));
    }
}
