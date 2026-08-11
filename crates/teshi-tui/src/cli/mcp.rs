//! Local STDIO MCP adapter for Teshi browser-agent operations.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use teshi_engine::{
    BROWSER_AGENT_SCHEMA_VERSION, BrowserOperation, BrowserOperations, BrowserTarget,
    DEFAULT_BROWSER_LEASE_TTL_SECS, PageContextRevision, load_project_settings,
};

use super::browser_endpoint::{read_cdp_endpoint, resolve_browser_project_root};
use super::{McpCommand, McpServeArgs};

const CURRENT_MCP_VERSION: &str = "2026-07-28";
const LEGACY_MCP_VERSION: &str = "2025-11-25";
const SERVER_NAME: &str = "teshi-browser-agent";
const TOOL_CACHE_TTL_MS: u64 = 5_000;

/// Handles `teshi mcp ...` commands.
pub fn handle_mcp_command(action: &McpCommand) -> Result<()> {
    match action {
        McpCommand::Serve(args) => serve(args),
    }
}

fn serve(args: &McpServeArgs) -> Result<()> {
    if !args.stdio {
        return Err(anyhow!(
            "no MCP transport selected; run `teshi mcp serve --stdio`"
        ));
    }
    let project_root = resolve_project_root(args.project.as_deref())?;
    serve_stdio(&project_root)
}

fn resolve_project_root(configured: Option<&Path>) -> Result<PathBuf> {
    let start = match configured {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir().context("resolve current directory for MCP server")?,
    };
    let canonical = start
        .canonicalize()
        .with_context(|| format!("canonicalize MCP project root {}", start.display()))?;
    Ok(resolve_browser_project_root(&canonical).unwrap_or(canonical))
}

fn serve_stdio(project_root: &Path) -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.context("read MCP STDIO request")?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => dispatch_request(project_root, &request),
            Err(error) => Some(rpc_error(
                Value::Null,
                -32700,
                "Parse error",
                Some(json!({ "detail": error.to_string() })),
            )),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut stdout, &response).context("serialize MCP response")?;
            stdout.write_all(b"\n").context("write MCP response")?;
            stdout.flush().context("flush MCP response")?;
        }
    }
    Ok(())
}

fn dispatch_request(project_root: &Path, request: &Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str);
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || method.is_none() {
        return id.map(|id| rpc_error(id, -32600, "Invalid Request", None));
    }

    // MCP notifications intentionally produce no response.
    let id = id?;
    let method = method.expect("checked above");
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "server/discover" => Ok(discovery_result()),
        "initialize" => Ok(initialize_result(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": tool_definitions(),
            "ttlMs": TOOL_CACHE_TTL_MS,
            "cacheScope": "session"
        })),
        "tools/call" => call_tool(project_root, &params),
        _ => {
            return Some(rpc_error(
                id,
                -32601,
                "Method not found",
                Some(json!({ "method": method })),
            ));
        }
    };
    Some(match result {
        Ok(result) => rpc_result(id, result),
        Err(error) => rpc_error(
            id,
            -32602,
            "Invalid params",
            Some(json!({ "detail": error.to_string() })),
        ),
    })
}

fn discovery_result() -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": [CURRENT_MCP_VERSION, LEGACY_MCP_VERSION],
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": server_info(),
        "instructions": server_instructions()
    })
}

fn initialize_result(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(LEGACY_MCP_VERSION);
    let negotiated = match requested {
        CURRENT_MCP_VERSION | LEGACY_MCP_VERSION | "2025-06-18" | "2024-11-05" => requested,
        _ => LEGACY_MCP_VERSION,
    };
    json!({
        "protocolVersion": negotiated,
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": server_info(),
        "instructions": server_instructions()
    })
}

fn server_info() -> Value {
    json!({
        "name": SERVER_NAME,
        "title": "Teshi Browser Agent",
        "version": env!("CARGO_PKG_VERSION")
    })
}

fn server_instructions() -> &'static str {
    "This is a same-host browser integration. Install and connect the compatible Teshi Chrome extension, list sessions and tabs, acquire one exclusive profile lease, and pass the explicit target plus lease token to every snapshot, locator, verification, and evidence call. Locator acquisition is observational and does not invent or execute test actions. Release the lease when finished. Browser URLs, titles, page content, and screenshot references are local user data; request only what the task requires. If DevTools owns the debugger, close it or choose another dedicated browser profile."
}

fn call_tool(project_root: &Path, params: &Value) -> Result<Value> {
    let name = required_string(params, "name")?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let (operation, timeout) = parse_tool_operation(project_root, name, &arguments)?;
    let endpoint = match read_cdp_endpoint(project_root) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            return Ok(tool_error(
                "browser_unavailable",
                &error.to_string(),
                json!({
                    "project_root": project_root,
                    "next": "start the Teshi browser sidecar and connect the extension"
                }),
            ));
        }
    };
    let client = BrowserOperations::new(endpoint.ws_url, timeout);
    match client.execute(&operation) {
        Ok(response) => {
            let payload = response.payload;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&payload)?
                }],
                "structuredContent": payload,
                "isError": false
            }))
        }
        Err(error) => Ok(tool_error_payload(error.to_wire_value())),
    }
}

fn tool_error(code: &str, message: &str, recovery: Value) -> Value {
    let payload = json!({
        "ok": false,
        "schema_version": BROWSER_AGENT_SCHEMA_VERSION,
        "code": code,
        "error": message,
        "recovery": recovery
    });
    tool_error_payload(payload)
}

fn tool_error_payload(payload: Value) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
        }],
        "structuredContent": payload,
        "isError": true
    })
}

fn parse_tool_operation(
    project_root: &Path,
    name: &str,
    arguments: &Value,
) -> Result<(BrowserOperation, Duration)> {
    let timeout = Duration::from_millis(
        arguments
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(60_000)
            .clamp(1_000, 300_000),
    );
    let operation = match name {
        "list_browser_sessions" => BrowserOperation::ListBrowserSessions,
        "list_browser_tabs" => BrowserOperation::ListBrowserTabs {
            extension_instance_id: required_string(arguments, "extension_instance_id")?.into(),
        },
        "acquire_browser_lease" => BrowserOperation::AcquireBrowserLease {
            extension_instance_id: required_string(arguments, "extension_instance_id")?.into(),
            owner_label: arguments
                .get("owner_label")
                .and_then(Value::as_str)
                .unwrap_or("mcp-agent")
                .into(),
            ttl_secs: arguments
                .get("ttl_secs")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_BROWSER_LEASE_TTL_SECS),
        },
        "renew_browser_lease" => BrowserOperation::RenewBrowserLease {
            extension_instance_id: required_string(arguments, "extension_instance_id")?.into(),
            lease_token: required_string(arguments, "lease_token")?.into(),
            ttl_secs: arguments
                .get("ttl_secs")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_BROWSER_LEASE_TTL_SECS),
        },
        "release_browser_lease" => BrowserOperation::ReleaseBrowserLease {
            extension_instance_id: required_string(arguments, "extension_instance_id")?.into(),
            lease_token: required_string(arguments, "lease_token")?.into(),
        },
        "get_page_snapshot" => BrowserOperation::GetPageSnapshot {
            target: parse_target(arguments)?,
            lease_token: required_string(arguments, "lease_token")?.into(),
        },
        "resolve_playwright_locator" => {
            let test_id_attributes = match arguments.get("test_id_attributes") {
                Some(value) => serde_json::from_value(value.clone())
                    .context("test_id_attributes must be an array of strings")?,
                None => load_project_settings(project_root)?.playwright_test_id_attributes,
            };
            BrowserOperation::ResolvePlaywrightLocator {
                target: parse_target(arguments)?,
                lease_token: required_string(arguments, "lease_token")?.into(),
                intent: serde_json::from_value(
                    arguments
                        .get("intent")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                )
                .context("intent does not match the locator-intent schema")?,
                test_id_attributes,
            }
        }
        "verify_playwright_locator" => BrowserOperation::VerifyPlaywrightLocator {
            target: parse_target(arguments)?,
            lease_token: required_string(arguments, "lease_token")?.into(),
            candidate: serde_json::from_value(
                arguments
                    .get("candidate")
                    .cloned()
                    .ok_or_else(|| anyhow!("missing required parameter `candidate`"))?,
            )
            .context("candidate does not match the Playwright locator schema")?,
            page_context_revision: PageContextRevision(
                required_string(arguments, "page_context_revision")?.into(),
            ),
        },
        "capture_browser_evidence" => BrowserOperation::CaptureBrowserEvidence {
            target: parse_target(arguments)?,
            lease_token: required_string(arguments, "lease_token")?.into(),
            page_context_revision: PageContextRevision(
                required_string(arguments, "page_context_revision")?.into(),
            ),
        },
        _ => return Err(anyhow!("unknown Teshi MCP tool `{name}`")),
    };
    Ok((operation, timeout))
}

fn parse_target(arguments: &Value) -> Result<BrowserTarget> {
    serde_json::from_value(
        arguments
            .get("target")
            .cloned()
            .ok_or_else(|| anyhow!("missing required parameter `target`"))?,
    )
    .context("target must contain extension_instance_id, window_id, and tab_id")
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("missing required string parameter `{key}`"))
}

fn tool_definitions() -> Vec<Value> {
    let target = target_schema();
    let timeout = json!({
        "type": "integer",
        "minimum": 1000,
        "maximum": 300000,
        "default": 60000
    });
    vec![
        tool(
            "list_browser_sessions",
            "List local browser-extension sessions, health, leases, windows, and tabs.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "list_browser_tabs",
            "List windows and tabs belonging to one opaque extension instance.",
            json!({
                "type": "object",
                "properties": { "extension_instance_id": { "type": "string", "minLength": 1 } },
                "required": ["extension_instance_id"],
                "additionalProperties": false
            }),
        ),
        tool(
            "acquire_browser_lease",
            "Acquire an exclusive, bounded lease for one browser profile.",
            json!({
                "type": "object",
                "properties": {
                    "extension_instance_id": { "type": "string", "minLength": 1 },
                    "owner_label": { "type": "string", "default": "mcp-agent" },
                    "ttl_secs": { "type": "integer", "minimum": 1, "maximum": 300, "default": 60 },
                    "timeout_ms": timeout.clone()
                },
                "required": ["extension_instance_id"],
                "additionalProperties": false
            }),
        ),
        tool(
            "renew_browser_lease",
            "Renew a matching browser-profile lease before it expires.",
            json!({
                "type": "object",
                "properties": {
                    "extension_instance_id": { "type": "string", "minLength": 1 },
                    "lease_token": { "type": "string", "minLength": 1 },
                    "ttl_secs": { "type": "integer", "minimum": 1, "maximum": 300, "default": 60 },
                    "timeout_ms": timeout.clone()
                },
                "required": ["extension_instance_id", "lease_token"],
                "additionalProperties": false
            }),
        ),
        tool(
            "release_browser_lease",
            "Release a matching browser-profile lease.",
            json!({
                "type": "object",
                "properties": {
                    "extension_instance_id": { "type": "string", "minLength": 1 },
                    "lease_token": { "type": "string", "minLength": 1 },
                    "timeout_ms": timeout.clone()
                },
                "required": ["extension_instance_id", "lease_token"],
                "additionalProperties": false
            }),
        ),
        tool(
            "get_page_snapshot",
            "Read a normalized snapshot from one explicitly targeted browser tab.",
            targeted_schema(target.clone(), timeout.clone(), json!({}), &[]),
        ),
        tool(
            "resolve_playwright_locator",
            "Generate, rank, and browser-verify Playwright locator candidates without executing an action.",
            targeted_schema(
                target.clone(),
                timeout.clone(),
                json!({
                    "intent": {
                        "type": "object",
                        "properties": {
                            "purpose": { "type": "string" },
                            "text": { "type": "string" },
                            "role": { "type": "string" },
                            "element_ref": { "type": "string" },
                            "gherkin_step": { "type": "string" }
                        },
                        "additionalProperties": false
                    },
                    "test_id_attributes": {
                        "type": "array",
                        "items": { "type": "string", "minLength": 1 }
                    }
                }),
                &["intent"],
            ),
        ),
        tool(
            "verify_playwright_locator",
            "Re-verify one structured locator candidate against its original page revision.",
            targeted_schema(
                target.clone(),
                timeout.clone(),
                json!({
                    "candidate": { "type": "object" },
                    "page_context_revision": { "type": "string", "minLength": 1 }
                }),
                &["candidate", "page_context_revision"],
            ),
        ),
        tool(
            "capture_browser_evidence",
            "Capture an optional screenshot reference bound to one target and page revision.",
            targeted_schema(
                target,
                timeout,
                json!({
                    "page_context_revision": { "type": "string", "minLength": 1 }
                }),
                &["page_context_revision"],
            ),
        ),
    ]
}

fn target_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "extension_instance_id": { "type": "string", "minLength": 1 },
            "window_id": { "type": "integer" },
            "tab_id": { "type": "integer" }
        },
        "required": ["extension_instance_id", "window_id", "tab_id"],
        "additionalProperties": false
    })
}

fn targeted_schema(
    target: Value,
    timeout: Value,
    extra_properties: Value,
    extra_required: &[&str],
) -> Value {
    let mut properties = serde_json::Map::from_iter([
        ("target".into(), target),
        (
            "lease_token".into(),
            json!({ "type": "string", "minLength": 1 }),
        ),
        ("timeout_ms".into(), timeout),
    ]);
    if let Some(extra) = extra_properties.as_object() {
        properties.extend(extra.clone());
    }
    let mut required = vec![json!("target"), json!("lease_token")];
    required.extend(extra_required.iter().map(|name| json!(name)));
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema
    })
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::browser::locator_operation;
    use crate::cli::{BrowserLocatorArgs, BrowserTargetArgs};

    #[test]
    fn advertises_one_tool_for_every_typed_browser_operation() {
        let names: Vec<_> = tool_definitions()
            .into_iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "list_browser_sessions",
                "list_browser_tabs",
                "acquire_browser_lease",
                "renew_browser_lease",
                "release_browser_lease",
                "get_page_snapshot",
                "resolve_playwright_locator",
                "verify_playwright_locator",
                "capture_browser_evidence",
            ]
        );
    }

    #[test]
    fn current_discovery_and_legacy_initialize_are_both_supported() {
        let discovery = dispatch_request(
            Path::new("."),
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "server/discover" }),
        )
        .unwrap();
        assert_eq!(discovery["result"]["resultType"], "complete");
        assert_eq!(
            discovery["result"]["supportedVersions"][0],
            CURRENT_MCP_VERSION
        );

        let initialize = dispatch_request(
            Path::new("."),
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "initialize",
                "params": { "protocolVersion": "2025-06-18" }
            }),
        )
        .unwrap();
        assert_eq!(initialize["result"]["protocolVersion"], "2025-06-18");
    }

    #[test]
    fn tool_parsing_preserves_explicit_target_and_lease() {
        let (operation, _) = parse_tool_operation(
            Path::new("."),
            "get_page_snapshot",
            &json!({
                "target": {
                    "extension_instance_id": "profile-a",
                    "window_id": 4,
                    "tab_id": 9
                },
                "lease_token": "secret"
            }),
        )
        .unwrap();
        let command = operation.to_sidecar_command("request-a");
        assert_eq!(command["request_id"], "request-a");
        assert_eq!(command["target"]["extension_instance_id"], "profile-a");
        assert_eq!(command["target"]["tab_id"], 9);
        assert_eq!(command["lease_token"], "secret");
    }

    #[test]
    fn target_dependent_tool_fails_closed_without_target() {
        let error = parse_tool_operation(
            Path::new("."),
            "get_page_snapshot",
            &json!({ "lease_token": "secret" }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("target"));
    }

    #[test]
    fn cli_and_mcp_locator_adapters_build_identical_typed_operation() {
        let cli_args = BrowserLocatorArgs {
            target: BrowserTargetArgs {
                session: Some("profile-a".into()),
                window: Some(7),
                tab: Some(42),
                lease_token: Some("lease-a".into()),
            },
            purpose: Some("save changes".into()),
            text: Some("Save".into()),
            role: Some("button".into()),
            element_ref: None,
            gherkin_step: None,
            test_id_attributes: vec!["data-qa".into()],
            timeout_ms: 60_000,
        };
        let cli_operation = locator_operation(Path::new("."), &cli_args).unwrap();
        let (mcp_operation, _) = parse_tool_operation(
            Path::new("."),
            "resolve_playwright_locator",
            &json!({
                "target": {
                    "extension_instance_id": "profile-a",
                    "window_id": 7,
                    "tab_id": 42
                },
                "lease_token": "lease-a",
                "intent": {
                    "purpose": "save changes",
                    "text": "Save",
                    "role": "button"
                },
                "test_id_attributes": ["data-qa"]
            }),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(cli_operation).unwrap(),
            serde_json::to_value(mcp_operation).unwrap()
        );
    }
}
