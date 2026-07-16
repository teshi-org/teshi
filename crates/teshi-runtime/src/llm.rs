//! Simple LLM helper for calling OpenAI-compatible chat completions APIs with tool calling.

use std::time::Duration;

use serde_json::{json, Value};

/// Read LLM configuration from environment variables.
///
/// Returns `(api_key, base_url, model)`.
///
/// - `api_key`: from `TESHI_LLM_API_KEY` (fallback: `OPENAI_API_KEY`)
/// - `base_url`: from `TESHI_LLM_BASE_URL` (fallback: `https://api.openai.com/v1`)
/// - `model`: from `TESHI_LLM_MODEL` (fallback: `gpt-4o-mini`)
pub fn llm_config_from_env() -> Result<(String, String, String), String> {
    let api_key = std::env::var("TESHI_LLM_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .map_err(|_| {
            "TESHI_LLM_API_KEY or OPENAI_API_KEY environment variable not set".to_string()
        })?;

    let base_url = std::env::var("TESHI_LLM_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

    let model = std::env::var("TESHI_LLM_MODEL")
        .unwrap_or_else(|_| "gpt-4o-mini".to_string());

    Ok((api_key, base_url, model))
}

/// Call the LLM with a system prompt, user message, and a single tool definition.
///
/// Returns the tool call arguments parsed as a `serde_json::Value`.
pub async fn call_llm_with_tool(
    api_key: &str,
    base_url: &str,
    model: &str,
    system: &str,
    user: &str,
    tool_name: &str,
    tool_description: &str,
    tool_parameters: Value,
) -> Result<Value, String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let body = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ],
        "tools": [{
            "type": "function",
            "function": {
                "name": tool_name,
                "description": tool_description,
                "parameters": tool_parameters
            }
        }],
        "tool_choice": {
            "type": "function",
            "function": {"name": tool_name}
        },
        "temperature": 0.2,
        "max_tokens": 16384
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("LLM HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!(
            "LLM API returned status {}: {}",
            status, error_text
        ));
    }

    let resp_json: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse LLM response: {}", e))?;

    let choices = resp_json["choices"]
        .as_array()
        .ok_or_else(|| "LLM response missing 'choices' array".to_string())?;
    let message = &choices
        .first()
        .ok_or_else(|| "LLM response has empty 'choices'".to_string())?["message"];
    let tool_calls = message["tool_calls"]
        .as_array()
        .ok_or_else(|| "LLM response missing 'tool_calls' — the model may not support tool calling".to_string())?;
    let args_str = tool_calls
        .first()
        .ok_or_else(|| "LLM response has empty 'tool_calls'".to_string())?["function"]["arguments"]
        .as_str()
        .ok_or_else(|| "LLM response missing tool call arguments".to_string())?;

    let args: Value = serde_json::from_str(args_str)
        .map_err(|e| format!("Failed to parse tool call arguments: {}", e))?;

    Ok(args)
}
