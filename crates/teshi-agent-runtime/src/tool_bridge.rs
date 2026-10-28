//! Conversation-local MCP transport. Tools execute on the frontend thread.

use std::{
    collections::HashMap,
    io::{BufRead, Read},
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use teshi_core::llm::{ToolCall, ToolDefinition};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt},
    sync::oneshot,
};

const MAX_FRAME: usize = 8 * 1024 * 1024;
const TOKEN_ENV: &str = "TESHI_AUTHORING_BRIDGE_TOKEN";
static NEXT_CALL_ID: AtomicU64 = AtomicU64::new(1);

/// Startup configuration for a live host's MCP proxy.
#[derive(Debug, Clone)]
pub struct ToolBridgeConfig {
    /// Teshi executable implementing the internal STDIO proxy command.
    pub executable: PathBuf,
    /// Tools permitted by the conversation profile.
    pub tools: Vec<ToolDefinition>,
}

/// A tool request awaiting execution on the owning frontend thread.
#[derive(Debug)]
pub struct HostToolRequest {
    /// Validated host tool name, arguments, and bridge-local call identity.
    pub call: ToolCall,
    response: oneshot::Sender<Result<String, String>>,
}

impl HostToolRequest {
    /// Whether the requesting connection is still awaiting this result.
    pub fn is_pending(&self) -> bool {
        !self.response.is_closed()
    }

    /// Complete the MCP call with the host's actual result.
    pub fn respond(self, result: Result<String, String>) {
        let _ = self.response.send(result);
    }
}

/// Owns one authenticated loopback listener and its pending host requests.
pub struct ToolBridge {
    server: teshi_acp::McpServer,
    tools: Arc<RwLock<Vec<ToolDefinition>>>,
    allowed_names: Vec<String>,
    requests: mpsc::Receiver<HostToolRequest>,
    pending: HashMap<String, HostToolRequest>,
    shutdown: Option<oneshot::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for ToolBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolBridge")
            .field("pending_calls", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl ToolBridge {
    /// Start an isolated bridge with an OS-random authentication token.
    ///
    /// # Errors
    /// Returns entropy, socket, runtime, or thread startup failures.
    pub fn start(config: &ToolBridgeConfig) -> Result<Self> {
        let mut entropy = [0_u8; 32];
        getrandom::fill(&mut entropy).map_err(|error| anyhow!("bridge entropy: {error}"))?;
        let token: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let server = teshi_acp::McpServer::Stdio(
            teshi_acp::McpServerStdio::new("teshi-authoring", config.executable.clone())
                .args(vec![
                    "mcp".into(),
                    "authoring-proxy".into(),
                    "--address".into(),
                    address.to_string(),
                ])
                .env(vec![teshi_acp::EnvVariable::new(TOKEN_ENV, token.clone())]),
        );
        let tools = Arc::new(RwLock::new(config.tools.clone()));
        let catalog = Arc::clone(&tools);
        let (requests_tx, requests) = mpsc::sync_channel(32);
        let (shutdown, mut shutdown_rx) = oneshot::channel();
        let worker = thread::Builder::new().name("teshi-authoring-mcp".into()).spawn(move || {
            runtime.block_on(async move {
                let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return; };
                let mut connections = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        biased;
                        _ = &mut shutdown_rx => break,
                        Some(_) = connections.join_next(), if !connections.is_empty() => {},
                        accepted = listener.accept() => {
                            let Ok((socket, _)) = accepted else { break; };
                            if connections.len() >= 32 { continue; }
                            let token = token.clone();
                            let tools = Arc::clone(&catalog);
                            let requests = requests_tx.clone();
                            connections.spawn(async move {
                                let _ = serve_connection(socket, &token, &tools, &requests, &NEXT_CALL_ID).await;
                            });
                        }
                    }
                }
                // Aborting these tasks drops all response receivers, including
                // calls parked on manual approval. No task outlives this runtime.
                connections.shutdown().await;
            });
        })?;
        let allowed_names = config.tools.iter().map(|tool| tool.name.clone()).collect();
        Ok(Self {
            server,
            tools,
            allowed_names,
            requests,
            pending: HashMap::new(),
            shutdown: Some(shutdown),
            worker: Some(worker),
        })
    }

    /// ACP descriptor containing the private proxy environment.
    pub fn server(&self) -> teshi_acp::McpServer {
        self.server.clone()
    }

    /// Update the allowed tool catalog before sending the next turn.
    pub fn set_tools(&self, tools: Vec<ToolDefinition>) {
        if let Ok(mut catalog) = self.tools.write() {
            *catalog = tools
                .into_iter()
                .filter(|tool| self.allowed_names.contains(&tool.name))
                .collect();
        }
    }

    /// Poll the next request without blocking the frontend.
    pub fn try_recv(&self) -> Option<HostToolRequest> {
        self.requests.try_recv().ok()
    }

    /// Hold a call until the existing file approval UI supplies its result.
    pub fn defer(&mut self, request: HostToolRequest) {
        self.pending.insert(request.call.id.clone(), request);
    }

    /// Resolve a deferred file change. Returns whether the ID was pending.
    pub fn resolve(&mut self, id: &str, result: Result<String, String>) -> bool {
        if let Some(request) = self.pending.remove(id) {
            request.respond(result);
            true
        } else {
            false
        }
    }

    /// Whether a deferred call is still connected to its original requester.
    pub fn is_pending(&self, id: &str) -> bool {
        self.pending
            .get(id)
            .is_some_and(HostToolRequest::is_pending)
    }

    /// Reject queued and deferred calls when the current turn is cancelled.
    pub fn cancel_pending(&mut self) {
        for (_, request) in self.pending.drain() {
            request.respond(Err("ACP turn cancelled".into()));
        }
        while let Some(request) = self.try_recv() {
            request.respond(Err("ACP turn cancelled".into()));
        }
    }

    /// Reject calls and stop accepting connections without joining on the UI thread.
    pub(crate) fn request_shutdown(&mut self) {
        self.cancel_pending();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }

    /// Confirm that all bridge connection tasks and the owned runtime exited.
    pub(crate) fn poll_shutdown(&mut self) -> bool {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return false;
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        true
    }
}

impl Drop for ToolBridge {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

async fn serve_connection(
    socket: tokio::net::TcpStream,
    token: &str,
    tools: &RwLock<Vec<ToolDefinition>>,
    requests: &mpsc::SyncSender<HostToolRequest>,
    serial: &AtomicU64,
) -> Result<()> {
    let mut socket = tokio::io::BufReader::new(socket);
    let mut frame = Vec::new();
    let size = tokio::time::timeout(
        Duration::from_secs(10),
        (&mut socket)
            .take((MAX_FRAME + 1) as u64)
            .read_until(b'\n', &mut frame),
    )
    .await??;
    if size == 0 || size > MAX_FRAME || frame.last() != Some(&b'\n') {
        bail!("invalid bridge frame");
    }
    let envelope: Value = serde_json::from_slice(&frame)?;
    if envelope.get("token").and_then(Value::as_str) != Some(token) {
        bail!("bridge authentication failed");
    }
    let request = &envelope["request"];
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let response = if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        rpc_error(id, -32600, "Invalid Request")
    } else {
        match request.get("method").and_then(Value::as_str) {
            Some("initialize") => rpc_result(
                id,
                json!({
                    "protocolVersion": request.pointer("/params/protocolVersion").and_then(Value::as_str)
                        .filter(|version| ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"].contains(version))
                        .unwrap_or("2025-11-25"),
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "teshi-authoring", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "Use these tools for Teshi feature generation. Human test-point review is mandatory. Stop after propose_test_points and wait for the user to continue in Teshi."
                }),
            ),
            Some("ping") => rpc_result(id, json!({})),
            Some("tools/list") => {
                let catalog = tools
                    .read()
                    .map_err(|_| anyhow!("tool catalog unavailable"))?;
                let definitions: Vec<Value> = catalog.iter().map(|tool| json!({"name":tool.name,"description":tool.description,"inputSchema":tool.parameters})).collect();
                rpc_result(id, json!({"tools":definitions}))
            }
            Some("tools/call") => {
                let name = request
                    .pointer("/params/name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let allowed = tools
                    .read()
                    .map_err(|_| anyhow!("tool catalog unavailable"))?
                    .iter()
                    .any(|tool| tool.name == name);
                if !allowed {
                    tool_result(
                        id,
                        Err(format!(
                            "Tool is not available to this conversation: {name}"
                        )),
                    )
                } else {
                    let arguments = request
                        .pointer("/params/arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    if !arguments.is_object() {
                        rpc_error(id, -32602, "Tool arguments must be an object")
                    } else {
                        let (response, receiver) = oneshot::channel();
                        let call = ToolCall {
                            id: format!("teshi-mcp-{}", serial.fetch_add(1, Ordering::Relaxed)),
                            name: name.into(),
                            arguments: arguments.to_string(),
                            execution_duration_ms: None,
                        };
                        match requests.try_send(HostToolRequest { call, response }) {
                            Ok(()) => {
                                // EOF means the proxy/agent disconnected. Drop the
                                // receiver so the UI can discard the stale request.
                                let mut byte = [0_u8; 1];
                                let result = tokio::select! {
                                    result = receiver => result.unwrap_or_else(|_| Err("Teshi host disconnected".into())),
                                    _ = socket.read(&mut byte) => return Ok(()),
                                };
                                tool_result(id, result)
                            }
                            Err(_) => {
                                tool_result(id, Err("Teshi host is unavailable or busy".into()))
                            }
                        }
                    }
                }
            }
            _ => rpc_error(id, -32601, "Method not found"),
        }
    };
    let mut bytes = serde_json::to_vec(&response)?;
    if bytes.len() >= MAX_FRAME {
        bytes = serde_json::to_vec(&rpc_error(
            response["id"].clone(),
            -32603,
            "Tool response exceeds bridge frame limit",
        ))?;
    }
    bytes.push(b'\n');
    socket.get_mut().write_all(&bytes).await?;
    Ok(())
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}
fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn tool_result(id: Value, result: Result<String, String>) -> Value {
    let is_error = result.is_err();
    let text = result.unwrap_or_else(|error| error);
    rpc_result(
        id,
        json!({"content":[{"type":"text","text":text}],"isError":is_error}),
    )
}

/// Forward MCP STDIO requests to a private live-host bridge.
///
/// # Errors
/// Returns invalid-address, missing-token, framing, socket, or STDIO failures.
pub fn run_stdio_proxy(address: SocketAddr) -> Result<()> {
    if !address.ip().is_loopback() {
        bail!("authoring bridge must use loopback");
    }
    let token = std::env::var(TOKEN_ENV).context("missing authoring bridge token")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (input_tx, input_rx) = tokio::sync::mpsc::channel(32);
    // A dedicated OS thread keeps blocking STDIN reads out of Tokio's blocking
    // pool. Dropping the runtime must not wait for an open parent input pipe.
    thread::Builder::new()
        .name("teshi-mcp-stdin".into())
        .spawn(move || {
            let stdin = std::io::stdin();
            let mut input = stdin.lock();
            loop {
                let mut frame = Vec::new();
                let result = (&mut input)
                    .take((MAX_FRAME + 1) as u64)
                    .read_until(b'\n', &mut frame);
                let message = match result {
                    Ok(0) => break,
                    Ok(size) if size <= MAX_FRAME && frame.last() == Some(&b'\n') => Ok(frame),
                    Ok(_) => Err("MCP request exceeds frame limit".to_string()),
                    Err(error) => Err(format!("read MCP STDIO: {error}")),
                };
                let failed = message.is_err();
                if input_tx.blocking_send(message).is_err() || failed {
                    break;
                }
            }
        })?;
    runtime.block_on(proxy_session(address, token, input_rx, tokio::io::stdout()))
}

async fn proxy_session(
    address: SocketAddr,
    token: String,
    mut input: tokio::sync::mpsc::Receiver<Result<Vec<u8>, String>>,
    mut output: impl tokio::io::AsyncWrite + Unpin,
) -> Result<()> {
    let mut calls = tokio::task::JoinSet::new();
    let mut pending = HashMap::<String, tokio::task::AbortHandle>::new();
    loop {
        tokio::select! {
            frame = input.recv() => {
                let Some(frame) = frame else { break; };
                let frame = frame.map_err(|error| anyhow!(error))?;
                let request: Value = match serde_json::from_slice(&frame) {
                    Ok(request) => request,
                    Err(_) => {
                        write_proxy_response(&mut output, &rpc_error(Value::Null, -32700, "Parse error")).await?;
                        continue;
                    }
                };
                if request.get("method").and_then(Value::as_str) == Some("notifications/cancelled") {
                    if let Some(id) = request.pointer("/params/requestId")
                        && let Some(call) = pending.remove(&id.to_string())
                    { call.abort(); }
                    continue;
                }
                let Some(id) = request.get("id") else { continue; };
                let key = id.to_string();
                if pending.contains_key(&key) || pending.len() >= 32 {
                    write_proxy_response(&mut output, &rpc_error(id.clone(), -32600, "Duplicate request ID or too many pending requests")).await?;
                    continue;
                }
                let token = token.clone();
                let request_key = key.clone();
                let abort = calls.spawn(async move {
                    (request_key, tokio::task::id(), proxy_request(address, &token, request).await)
                });
                pending.insert(key, abort);
            }
            Some(completed) = calls.join_next(), if !calls.is_empty() => {
                match completed {
                    Ok((key, task_id, response)) => {
                        // Cancelled requests may reuse an ID. Their aborted task
                        // never produces a response or removes the replacement.
                        if pending.get(&key).is_none_or(|call| call.id() != task_id) { continue; }
                        pending.remove(&key);
                        match response {
                            Ok(bytes) => { output.write_all(&bytes).await?; output.flush().await?; }
                            Err(error) => {
                                let id = serde_json::from_str(&key)?;
                                write_proxy_response(&mut output, &rpc_error(id, -32603, &error.to_string())).await?;
                            }
                        }
                    }
                    Err(error) if error.is_cancelled() => {},
                    Err(error) => return Err(error.into()),
                }
            }
        }
    }
    // EOF is cancellation of the entire MCP connection. Await task destruction
    // so the host observes closed sockets before the proxy returns.
    calls.shutdown().await;
    Ok(())
}

async fn write_proxy_response(
    output: &mut (impl tokio::io::AsyncWrite + Unpin),
    response: &Value,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(response)?;
    bytes.push(b'\n');
    output.write_all(&bytes).await?;
    output.flush().await?;
    Ok(())
}

async fn proxy_request(address: SocketAddr, token: &str, request: Value) -> Result<Vec<u8>> {
    let mut socket = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect(address),
    )
    .await??;
    let mut bytes = serde_json::to_vec(&json!({"token":token,"request":request}))?;
    bytes.push(b'\n');
    tokio::time::timeout(Duration::from_secs(10), socket.write_all(&bytes)).await??;
    let mut response = Vec::new();
    let size = tokio::io::BufReader::new(socket)
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut response)
        .await?;
    if size == 0 || size > MAX_FRAME || response.last() != Some(&b'\n') {
        bail!("authoring bridge disconnected or returned an invalid frame");
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufReader, Write},
        net::TcpStream,
        time::Instant,
    };

    fn bridge() -> ToolBridge {
        ToolBridge::start(&ToolBridgeConfig {
            executable: std::env::current_exe().unwrap(),
            tools: vec![ToolDefinition {
                name: "submit_requirements".into(),
                description: "Submit".into(),
                parameters: json!({"type":"object"}),
            }],
        })
        .unwrap()
    }

    fn connection(bridge: &ToolBridge) -> (SocketAddr, String) {
        let teshi_acp::McpServer::Stdio(server) = bridge.server() else {
            panic!("stdio expected");
        };
        (
            server.args.last().unwrap().parse().unwrap(),
            server.env[0].value.clone(),
        )
    }

    fn call(address: SocketAddr, token: String, request: Value) -> Value {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        serde_json::to_writer(&mut stream, &json!({"token":token,"request":request})).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    }

    fn wait_request(bridge: &ToolBridge) -> HostToolRequest {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(request) = bridge.try_recv() {
                return request;
            }
            assert!(Instant::now() < deadline, "host request not received");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn discovery_and_execution_enforce_catalog_and_authentication() {
        let bridge = bridge();
        let (address, token) = connection(&bridge);
        assert!(!format!("{bridge:?}").contains(&token));
        let discovery = call(
            address,
            token.clone(),
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        );
        assert_eq!(
            discovery["result"]["tools"][0]["name"],
            "submit_requirements"
        );
        let denied = call(
            address,
            token.clone(),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_feature_file"}}),
        );
        assert_eq!(denied["result"]["isError"], true);
        assert!(bridge.try_recv().is_none());
        let malformed = call(
            address,
            token,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"submit_requirements","arguments":[]}}),
        );
        assert_eq!(malformed["error"]["code"], -32602);
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"{\"token\":\"wrong\",\"request\":{\"id\":4,\"method\":\"tools/list\"}}\n")
            .unwrap();
        let mut response = String::new();
        assert_eq!(BufReader::new(stream).read_line(&mut response).unwrap(), 0);
    }

    #[test]
    fn host_results_and_manual_approval_return_to_original_call() {
        let mut bridge = bridge();
        for outcome in [
            Ok("requirements saved".to_string()),
            Err("User rejected the change".to_string()),
        ] {
            let (address, token) = connection(&bridge);
            let caller = thread::spawn(move || {
                call(
                    address,
                    token,
                    json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"submit_requirements","arguments":{"feature_name":"Auth"}}}),
                )
            });
            let request = wait_request(&bridge);
            assert_eq!(request.call.name, "submit_requirements");
            assert!(request.call.arguments.contains("Auth"));
            let id = request.call.id.clone();
            bridge.defer(request);
            assert!(bridge.is_pending(&id));
            assert!(
                !caller.is_finished(),
                "manual approval must keep the result pending"
            );
            assert!(bridge.resolve(&id, outcome.clone()));
            let result = caller.join().unwrap();
            assert_eq!(result["id"], 9);
            assert_eq!(result["result"]["isError"], outcome.is_err());
            assert_eq!(
                result["result"]["content"][0]["text"],
                outcome.unwrap_or_else(|error| error)
            );
        }
    }

    #[test]
    fn dropping_bridge_closes_listener_and_pending_calls() {
        let bridge = bridge();
        let (address, token) = connection(&bridge);
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        serde_json::to_writer(&mut stream, &json!({"token":token,"request":{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"submit_requirements"}}})).unwrap();
        stream.write_all(b"\n").unwrap();
        let request = wait_request(&bridge);
        drop(bridge);
        assert!(!request.is_pending());
        // The accept thread may still be winding down, so poll for the closed
        // listener instead of assuming the drop is instantly observable.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "listener kept accepting after the bridge was dropped"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut response = String::new();
        assert_eq!(BufReader::new(stream).read_line(&mut response).unwrap(), 0);
    }

    #[test]
    fn cancelled_calls_and_tool_catalog_cannot_expand_permissions() {
        let mut bridge = bridge();
        bridge.set_tools(teshi_agent::get_tools(None));
        let (address, token) = connection(&bridge);
        let discovery = call(
            address,
            token.clone(),
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        );
        assert_eq!(discovery["result"]["tools"].as_array().unwrap().len(), 1);
        let caller = thread::spawn(move || {
            call(
                address,
                token,
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"submit_requirements"}}),
            )
        });
        let request = wait_request(&bridge);
        let id = request.call.id.clone();
        bridge.defer(request);
        bridge.cancel_pending();
        assert!(!bridge.is_pending(&id));
        assert_eq!(caller.join().unwrap()["result"]["isError"], true);
    }

    #[test]
    fn disconnected_request_is_not_executable() {
        let bridge = bridge();
        let (address, token) = connection(&bridge);
        let mut stream = TcpStream::connect(address).unwrap();
        serde_json::to_writer(&mut stream, &json!({"token":token,"request":{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"submit_requirements"}}})).unwrap();
        stream.write_all(b"\n").unwrap();
        let request = wait_request(&bridge);
        drop(stream);
        let deadline = Instant::now() + Duration::from_secs(5);
        while request.is_pending() {
            assert!(Instant::now() < deadline, "disconnected call was retained");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn calls_and_tokens_are_isolated_across_bridges() {
        let first = bridge();
        let second = bridge();
        let (first_address, first_token) = connection(&first);
        let (second_address, second_token) = connection(&second);
        assert_ne!(first_token, second_token);
        let one = thread::spawn(move || {
            call(
                first_address,
                first_token,
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"submit_requirements"}}),
            )
        });
        let two = thread::spawn(move || {
            call(
                second_address,
                second_token,
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"submit_requirements"}}),
            )
        });
        let first_request = wait_request(&first);
        let second_request = wait_request(&second);
        assert_ne!(first_request.call.id, second_request.call.id);
        first_request.respond(Ok("first".into()));
        second_request.respond(Ok("second".into()));
        assert_eq!(one.join().unwrap()["result"]["content"][0]["text"], "first");
        assert_eq!(
            two.join().unwrap()["result"]["content"][0]["text"],
            "second"
        );
    }

    #[test]
    fn stdio_proxy_child() {
        if std::env::var_os("TESHI_PROXY_TEST_CHILD").is_none() {
            return;
        }
        let address = std::env::var("TESHI_PROXY_TEST_ADDRESS")
            .unwrap()
            .parse()
            .unwrap();
        run_stdio_proxy(address).unwrap();
    }

    #[test]
    fn stdio_proxy_cancels_matching_call_and_closes_pending_calls_on_eof() {
        use std::process::{Child, Command, Stdio};
        struct OwnedChild(Child);
        impl Drop for OwnedChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let bridge = bridge();
        let (address, token) = connection(&bridge);
        let mut child = OwnedChild(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "tool_bridge::tests::stdio_proxy_child",
                    "--exact",
                    "--nocapture",
                ])
                .env("TESHI_PROXY_TEST_CHILD", "1")
                .env("TESHI_PROXY_TEST_ADDRESS", address.to_string())
                .env(TOKEN_ENV, token)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut input = child.0.stdin.take().unwrap();
        for id in [1, 2] {
            writeln!(input, "{}", json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"submit_requirements","arguments":{"marker":id}}})).unwrap();
        }
        input.flush().unwrap();
        let one = wait_request(&bridge);
        let two = wait_request(&bridge);
        let marker: Value = serde_json::from_str(&one.call.arguments).unwrap();
        let (first, second) = if marker["marker"] == 1 {
            (one, two)
        } else {
            (two, one)
        };
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}})
        )
        .unwrap();
        input.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while first.is_pending() {
            assert!(
                Instant::now() < deadline,
                "STDIO cancellation did not reach the host"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            second.is_pending(),
            "cancellation affected a different call"
        );
        drop(input);
        let deadline = Instant::now() + Duration::from_secs(5);
        while second.is_pending() || child.0.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "STDIO EOF did not close pending host calls and exit"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}
