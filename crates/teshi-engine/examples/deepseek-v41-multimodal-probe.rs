//! Opt-in, live DeepSeek V4.1 Flash multimodal wire-contract probe.
//!
//! This is deliberately a raw Chat Completions probe. It does not change
//! Teshi's production message model or parser.

use std::time::Instant;

use anyhow::{bail, Result};
use serde_json::{json, Value};

const ENDPOINT: &str = "https://api.deepseek.com/chat/completions";
const MODEL: &str = "deepseek-flash";
const FIXTURE_B64: &str = include_str!("../../../fixtures/deepseek-v41-ui.png.base64");

#[derive(Debug)]
struct ProbeResult {
    name: &'static str,
    result: String,
    detail: String,
    status: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("TESHI_RUN_LIVE_DEEPSEEK_PROBE").as_deref() != Ok("1") {
        println!("SKIP: set TESHI_RUN_LIVE_DEEPSEEK_PROBE=1 to access the live API");
        return Ok(());
    }
    let Ok(api_key) = std::env::var("DEEPSEEK_API_KEY") else {
        println!("SKIP: DEEPSEEK_API_KEY is not set");
        return Ok(());
    };

    let image = format!("data:image/png;base64,{}", FIXTURE_B64.trim());
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()?;

    let baseline = request(&client, &api_key, baseline_body(false)).await?;
    if !baseline.status {
        print_report(&[baseline]);
        bail!("text baseline failed; image probes were not run");
    }

    let mut results = vec![baseline];
    let a = request(&client, &api_key, image_body(&image, true, false, false)).await?;
    let image_ok = a.status;
    results.push(a);
    if !image_ok {
        results.push(request(&client, &api_key, image_body(&image, false, false, false)).await?);
    }
    results.push(stream_image(&client, &api_key, &image).await?);
    results.push(request(&client, &api_key, image_body(&image, false, true, false)).await?);
    let tool = request(&client, &api_key, image_body(&image, false, false, true)).await?;
    let tool_ok = tool.status && tool.detail.contains("valid_tool_call=true");
    results.push(tool);
    if tool_ok {
        results.push(second_turn(&client, &api_key, &image).await?);
    } else {
        results.push(ProbeResult {
            name: "vision + tool + second turn",
            result: "NOT RUN".into(),
            detail: "Probe 5 did not return a valid tool call".into(),
            status: false,
        });
    }
    results.push(external_url(&client, &api_key).await?);
    results.push(negative(&client, &api_key, "image in system", json!({"role":"system","content":[{"type":"text","text":"Describe the image."},{"type":"image_url","image_url":{"url":image}}]})).await?);
    results.push(negative(&client, &api_key, "image in assistant", json!({"role":"assistant","content":[{"type":"text","text":"I saw this."},{"type":"image_url","image_url":{"url":image}}]})).await?);
    results.push(negative(&client, &api_key, "invalid image data", json!({"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,not-a-png"}}]})).await?);
    print_report(&results);
    Ok(())
}

fn baseline_body(stream: bool) -> Value {
    json!({"model":MODEL,"messages":[{"role":"user","content":"Reply with baseline-ok."}],"stream":stream,"max_tokens":32})
}

fn image_body(image: &str, detail: bool, thinking: bool, tools: bool) -> Value {
    let image_url = if detail {
        json!({"url":image,"detail":"auto"})
    } else {
        json!({"url":image})
    };
    let mut body = json!({"model":MODEL,"messages":[{"role":"user","content":[{"type":"text","text":if tools {"Inspect the screenshot and call report_ui_state. Do not answer in plain text."} else if thinking {"Inspect the screenshot. Determine whether it is a successful test run or an error state. Answer only status=<success|error>."} else {"Read the UI screenshot. Return the visible button label and error code."}},{"type":"image_url","image_url":image_url}]}],"stream":false,"max_tokens":256});
    if thinking {
        body["thinking"] = json!({"type":"enabled"});
        body["reasoning_effort"] = json!("high");
    } else if tools {
        body["thinking"] = json!({"type":"disabled"});
    }
    if tools {
        body["tools"] = json!([{"type":"function","function":{"name":"report_ui_state","description":"Report the visible UI state.","parameters":{"type":"object","properties":{"button_label":{"type":"string"},"error_code":{"type":"string"},"state":{"type":"string","enum":["success","error","unknown"]}},"required":["button_label","error_code","state"]}}}]);
        body["tool_choice"] = json!({"type":"function","function":{"name":"report_ui_state"}});
    }
    body
}

async fn request(client: &reqwest::Client, key: &str, body: Value) -> Result<ProbeResult> {
    let name = if body["messages"][0]["content"].is_array() {
        if body.get("tools").is_some() {
            "vision + tool call"
        } else if body.get("thinking").is_some() {
            "vision + thinking"
        } else {
            "base64 image"
        }
    } else {
        "deepseek-flash text"
    };
    let started = Instant::now();
    let response = client
        .post(ENDPOINT)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or_else(|_| json!({}));
    let detail = if status.is_success() {
        response_detail(
            &value,
            started.elapsed().as_millis(),
            body.get("tools").is_some(),
        )
    } else {
        format!("{}; {}", status, error_detail(&value))
    };
    Ok(ProbeResult {
        name,
        result: if status.is_success() {
            "PASS".into()
        } else {
            "FAIL".into()
        },
        detail,
        status: status.is_success(),
    })
}

async fn stream_image(client: &reqwest::Client, key: &str, image: &str) -> Result<ProbeResult> {
    let mut body = image_body(image, false, false, false);
    body["stream"] = json!(true);
    body["stream_options"] = json!({"include_usage":true});
    let started = Instant::now();
    let response = client
        .post(ENDPOINT)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let v: Value = response.json().await.unwrap_or_default();
        return Ok(ProbeResult {
            name: "streaming vision",
            result: "FAIL".into(),
            detail: format!("{}; {}", status, error_detail(&v)),
            status: false,
        });
    }
    let text = response.text().await?;
    let mut content = 0;
    let mut reasoning = 0;
    let mut finish = None;
    let mut usage = false;
    let mut done = false;
    for line in text.lines().filter_map(|l| l.strip_prefix("data: ")) {
        if line == "[DONE]" {
            done = true;
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            let d = &v["choices"][0]["delta"];
            if d["content"].as_str().is_some() {
                content += 1;
            }
            if d["reasoning_content"].as_str().is_some() {
                reasoning += 1;
            }
            if v["choices"][0]["finish_reason"].is_string() {
                finish = v["choices"][0]["finish_reason"].as_str().map(str::to_owned);
            }
            if v.get("usage").is_some() {
                usage = true;
            }
        }
    }
    Ok(ProbeResult { name:"streaming vision", result:if content>0 && done {"PASS"} else {"FAIL"}.into(), detail:format!("status={status}; delta.content_events={content}; delta.reasoning_content_events={reasoning}; finish_reason={finish:?}; usage_present={usage}; done={done}; latency_ms={}",started.elapsed().as_millis()), status:content>0 && done })
}

async fn second_turn(client: &reqwest::Client, key: &str, image: &str) -> Result<ProbeResult> {
    let first = image_body(image, false, false, true);
    let first_response = client
        .post(ENDPOINT)
        .bearer_auth(key)
        .json(&first)
        .send()
        .await?;
    let first_value: Value = first_response.json().await?;
    let assistant = first_value["choices"][0]["message"].clone();
    if assistant.is_null() {
        return Ok(ProbeResult {
            name: "vision + tool + second turn",
            result: "FAIL".into(),
            detail: "first response missing assistant message".into(),
            status: false,
        });
    }
    let mut messages = vec![first["messages"][0].clone(), assistant.clone()];
    let call = assistant["tool_calls"][0].clone();
    let call_id = call["id"].as_str().unwrap_or("call");
    messages.push(json!({"role":"tool","tool_call_id":call_id,"content":"{\"button_label\":\"RUN TEST\",\"error_code\":\"42\",\"state\":\"error\"}"}));
    messages.push(json!({"role":"user","content":"What action should the test agent take next?"}));
    let mut body = image_body(image, false, false, true);
    body["messages"] = json!(messages);
    let response = client
        .post(ENDPOINT)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or_default();
    let ok = status.is_success() && value["choices"][0]["message"]["content"].is_string();
    Ok(ProbeResult{name:"vision + tool + second turn",result:if ok{"PASS".into()}else{"FAIL".into()},detail:format!("status={status}; assistant_reasoning_present={}; assistant_tool_call_present=true; second_turn_content_present={}",assistant["reasoning_content"].is_string(),value["choices"][0]["message"]["content"].is_string()),status:ok})
}

async fn external_url(client: &reqwest::Client, key: &str) -> Result<ProbeResult> {
    let Some(url) = std::env::var("TESHI_DEEPSEEK_PROBE_IMAGE_URL")
        .ok()
        .filter(|s| s.starts_with("https://"))
    else {
        return Ok(ProbeResult {
            name: "external URL image",
            result: "NOT RUN".into(),
            detail: "no stable public fixture URL supplied via TESHI_DEEPSEEK_PROBE_IMAGE_URL"
                .into(),
            status: false,
        });
    };
    request(client,key,json!({"model":MODEL,"messages":[{"role":"user","content":[{"type":"text","text":"Read the button label and error code."},{"type":"image_url","image_url":{"url":url}}]}],"stream":false,"max_tokens":64})).await.map(|mut r|{r.name="external URL image";r})
}

async fn negative(
    client: &reqwest::Client,
    key: &str,
    name: &'static str,
    message: Value,
) -> Result<ProbeResult> {
    let response = client
        .post(ENDPOINT)
        .bearer_auth(key)
        .json(&json!({"model":MODEL,"messages":[message],"stream":false,"max_tokens":16}))
        .send()
        .await?;
    let status = response.status();
    let v: Value = response.json().await.unwrap_or_default();
    Ok(ProbeResult {
        name,
        result: if status.is_success() {
            "ACCEPTED".into()
        } else {
            "REJECTED".into()
        },
        detail: if status.is_success() {
            format!("status={status}")
        } else {
            format!("{}; {}", status, error_detail(&v))
        },
        status: status.is_success(),
    })
}

fn response_detail(v: &Value, latency: u128, tool_expected: bool) -> String {
    let m = &v["choices"][0]["message"];
    let calls = m["tool_calls"].as_array();
    format!("status=200; model={}; finish_reason={:?}; usage_present={}; content_present={}; reasoning_content_present={}; reasoning_length={}; tool_calls_present={}; valid_tool_call={}; latency_ms={latency}",v["model"].as_str().unwrap_or(""),v["choices"][0]["finish_reason"].as_str(),v.get("usage").is_some(),m["content"].is_string(),m["reasoning_content"].is_string(),m["reasoning_content"].as_str().map_or(0,str::len),calls.is_some_and(|c|!c.is_empty()),tool_expected && calls.is_some_and(|c|!c.is_empty()))
}
fn error_detail(v: &Value) -> String {
    let message = v["error"]["message"].as_str().unwrap_or("");
    let safe_message = if message.to_ascii_lowercase().contains("key")
        || message.to_ascii_lowercase().contains("token")
        || message.to_ascii_lowercase().contains("secret")
    {
        "<redacted authentication detail>"
    } else {
        message
    };
    format!(
        "error_type={:?}; error_code={:?}; error_message={:?}",
        v["error"]["type"].as_str(),
        v["error"]["code"].as_str(),
        safe_message
    )
}
fn print_report(results: &[ProbeResult]) {
    println!("# DeepSeek V4.1 Flash Multimodal Capability Probe\n\nendpoint={ENDPOINT}; model={MODEL}; api_style=Chat Completions\n\n| Capability | Result | Sanitized observation |\n|---|---|---|");
    for r in results {
        println!(
            "| {} | {} | {} |",
            r.name,
            r.result,
            r.detail.replace('|', "\\|")
        );
    }
}
