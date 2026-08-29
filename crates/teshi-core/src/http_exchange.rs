//! NDJSON / sidecar event payloads shared by TUI Explore and the GPUI Run surface.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One HTTP attempt emitted after a template `call`.
///
/// Default payloads from the sidecar are already redacted (`redacted: true`).
/// Plaintext is available only via an explicit expand request to the sidecar.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HttpExchange {
    /// Event discriminator; always `http_exchange` on the wire.
    #[serde(rename = "type", default = "http_exchange_type")]
    pub type_name: String,
    /// Stable id for inspector expand.
    pub exchange_id: String,
    /// Runner case id when known.
    #[serde(default)]
    pub case_id: Option<String>,
    /// Gherkin step correlation id when known.
    #[serde(default)]
    pub step_id: Option<String>,
    /// Template path relative to the API template root.
    #[serde(default)]
    pub template: String,
    /// HTTP method after render.
    #[serde(default)]
    pub method: String,
    /// Request URL after render.
    #[serde(default)]
    pub url: String,
    /// Request headers (redacted by default).
    #[serde(default)]
    pub request_headers: Value,
    /// Request body (redacted by default).
    #[serde(default)]
    pub request_body: Value,
    /// HTTP status code.
    #[serde(default)]
    pub status: Option<u16>,
    /// Response headers (redacted by default).
    #[serde(default)]
    pub response_headers: Value,
    /// Response body (redacted by default).
    #[serde(default)]
    pub response_body: Value,
    /// Round-trip duration in milliseconds.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Extract map after the response pass.
    #[serde(default)]
    pub extract: Value,
    /// Per-assertion outcomes.
    #[serde(default)]
    pub asserts: Vec<AssertOutcome>,
    /// Whether secret fields in this payload are masked.
    #[serde(default)]
    pub redacted: bool,
}

fn http_exchange_type() -> String {
    "http_exchange".into()
}

impl HttpExchange {
    /// Parse an `http_exchange` object; unknown extra fields are ignored.
    ///
    /// # Errors
    ///
    /// Returns an error when required fields are missing or the JSON shape is invalid.
    pub fn from_value(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|err| err.to_string())
    }
}

/// One envelope assertion after the response render pass.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssertOutcome {
    /// Assertion name from the envelope.
    #[serde(default)]
    pub name: String,
    /// Whether the assertion passed.
    #[serde(default)]
    pub passed: bool,
    /// Rendered value (string or JSON).
    #[serde(default)]
    pub value: Value,
}

/// Format an exchange for TUI/GPUI inspectors without printing secrets when redacted.
///
/// Sensitive values that are already `***` stay masked. This helper never unmasks.
#[must_use]
pub fn format_exchange_lines(exchange: &HttpExchange, show_bodies: bool) -> Vec<String> {
    let mut lines = vec![
        format!(
            "{} {}  ({})",
            exchange.method, exchange.url, exchange.template
        ),
        format!(
            "status: {}  duration: {} ms  redacted: {}",
            exchange
                .status
                .map(|s| s.to_string())
                .unwrap_or_else(|| "-".into()),
            exchange
                .duration_ms
                .map(|ms| ms.to_string())
                .unwrap_or_else(|| "-".into()),
            exchange.redacted
        ),
    ];
    if !exchange.asserts.is_empty() {
        lines.push("asserts:".into());
        for item in &exchange.asserts {
            let mark = if item.passed { "pass" } else { "FAIL" };
            lines.push(format!("  [{mark}] {}", item.name));
        }
    }
    if show_bodies {
        lines.push("request headers:".into());
        lines.push(pretty_json(&exchange.request_headers));
        lines.push("request body:".into());
        lines.push(pretty_json(&exchange.request_body));
        lines.push("response headers:".into());
        lines.push(pretty_json(&exchange.response_headers));
        lines.push("response body:".into());
        lines.push(pretty_json(&exchange.response_body));
        lines.push("extract:".into());
        lines.push(pretty_json(&exchange.extract));
    }
    lines
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_exchange_does_not_unmask_redacted_values() {
        let exchange = HttpExchange::from_value(&json!({
            "type": "http_exchange",
            "exchange_id": "e1",
            "template": "create_user.json.j2",
            "method": "POST",
            "url": "https://example.test/users",
            "request_headers": {"Authorization": "***", "Accept": "application/json"},
            "request_body": {"name": "Ada", "password": "***"},
            "status": 201,
            "response_headers": {},
            "response_body": {"id": "42"},
            "duration_ms": 12,
            "extract": {"user_id": "42"},
            "asserts": [{"name": "status_ok", "passed": true, "value": "True"}],
            "redacted": true
        }))
        .expect("parse");
        let text = format_exchange_lines(&exchange, true).join("\n");
        assert!(text.contains("***"));
        assert!(!text.to_lowercase().contains("bearer"));
        assert!(text.contains("status_ok"));
        assert!(text.contains("redacted: true"));
    }
}
