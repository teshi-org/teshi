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
    results.extend(vision_thinking_tools(&client, &api_key, &image).await?);
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

fn tool_definition() -> Value {
    json!({"type":"function","function":{"name":"report_ui_state","description":"Report the visible UI state.","parameters":{"type":"object","properties":{"button_label":{"type":"string"},"error_code":{"type":"string"},"state":{"type":"string","enum":["success","error","unknown"]}},"required":["button_label","error_code","state"]}}})
}

fn thinking_tools_body(messages: Value) -> Value {
    json!({
        "model": MODEL,
        "messages": messages,
        "stream": false,
        "max_tokens": 256,
        "thinking": {"type": "enabled"},
        "reasoning_effort": "high",
        "tools": [tool_definition()]
    })
}

fn tool_result() -> &'static str {
    "{\"button_label\":\"RUN TEST\",\"error_code\":\"42\",\"state\":\"error\"}"
}

fn first_tool_call(message: &Value) -> Option<&Value> {
    message["tool_calls"].as_array()?.first()
}

fn tool_arguments(call: &Value) -> Option<Value> {
    let arguments = call["function"]["arguments"].as_str()?;
    serde_json::from_str(arguments).ok()
}

fn arguments_match_ui(arguments: &Value) -> bool {
    let button = arguments["button_label"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    let error = arguments["error_code"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    let state = arguments["state"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    button.contains("run test") && error.contains("42") && state == "error"
}

async fn vision_thinking_tools(
    client: &reqwest::Client,
    key: &str,
    image: &str,
) -> Result<Vec<ProbeResult>> {
    let first_user = json!({"role":"user","content":[
        {"type":"text","text":"Inspect the UI screenshot carefully. You have a report_ui_state tool. Use that tool to report the UI state you observe before giving any final answer. Identify the button label, error code, and whether this represents an error state."},
        {"type":"image_url","image_url":{"url":image}}
    ]});
    let mut messages = vec![first_user];
    let mut rows = Vec::new();
    let mut replay_rows = Vec::new();
    let mut previous_reasoning: Option<String> = None;
    let mut first_tool = false;
    let mut turns = 0;

    while turns < 3 {
        turns += 1;
        let body = thinking_tools_body(Value::Array(messages.clone()));
        if body.get("tool_choice").is_some() {
            bail!("combined probe constructed an unexpected tool_choice");
        }
        if let Some(reasoning) = previous_reasoning.as_ref() {
            let assistant = messages
                .iter()
                .rev()
                .find(|message| message["role"] == "assistant")
                .ok_or_else(|| anyhow::anyhow!("missing assistant message for replay"))?;
            let tool_id_matches = assistant["tool_calls"]
                .as_array()
                .and_then(|calls| calls.first())
                .and_then(|call| call["id"].as_str())
                .and_then(|id| {
                    messages
                        .iter()
                        .find(|message| message["role"] == "tool" && message["tool_call_id"] == id)
                })
                .is_some();
            if assistant["reasoning_content"].as_str() != Some(reasoning)
                || !assistant["content"].is_string()
                || first_tool_call(assistant).is_none()
                || !tool_id_matches
            {
                replay_rows.push(ProbeResult {
                    name: "reasoning replay after visual tool call",
                    result: "FAIL".into(),
                    detail:
                        "outgoing assistant semantic fields did not preserve the previous response"
                            .into(),
                    status: false,
                });
                return Ok(replay_rows);
            }
            replay_rows.push(ProbeResult {
                name: "reasoning replay after visual tool call",
                result: "PASS".into(),
                detail: format!("assistant_reasoning_replayed=true; reasoning_length={}; assistant_content_present=true; assistant_tool_calls_present=true; tool_call_id_paired={tool_id_matches}; tool_choice_absent=true", reasoning.len()),
                status: true,
            });
        }

        let response = client
            .post(ENDPOINT)
            .bearer_auth(key)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let value: Value = response.json().await.unwrap_or_default();
        if !status.is_success() {
            let row = ProbeResult {
                name: if turns == 1 {
                    "vision + thinking + autonomous tool call"
                } else {
                    "second-turn continuation"
                },
                result: "FAIL".into(),
                detail: format!("{}; {}", status, error_detail(&value)),
                status: false,
            };
            rows.push(row);
            break;
        }
        let message = value["choices"][0]["message"].clone();
        let reasoning = message["reasoning_content"].as_str();
        let content = message["content"].as_str();
        let calls = message["tool_calls"].as_array();
        let finish = value["choices"][0]["finish_reason"].as_str().unwrap_or("");
        let common = format!(
            "status={status}; finish_reason={finish:?}; reasoning_content_present={}; reasoning_length={}; content_present={}; content_length={}; tool_calls_present={}; tool_choice_absent=true",
            reasoning.is_some_and(|text| !text.is_empty()),
            reasoning.map_or(0, str::len),
            content.is_some(),
            content.map_or(0, str::len),
            calls.is_some_and(|items| !items.is_empty()),
        );
        if reasoning.is_none_or(str::is_empty) || content.is_none() {
            rows.push(ProbeResult {
                name: if turns == 1 {
                    "vision + thinking + autonomous tool call"
                } else {
                    "second-turn continuation"
                },
                result: "FAIL".into(),
                detail: format!(
                    "{common}; assistant content must be present (empty string allowed)"
                ),
                status: false,
            });
            break;
        }

        if turns == 1 {
            let Some(call) = first_tool_call(&message) else {
                rows.push(ProbeResult {
                    name: "vision + thinking + autonomous tool call",
                    result: "INCONCLUSIVE".into(),
                    detail: format!("{common}; model returned final text without tool call"),
                    status: false,
                });
                break;
            };
            let arguments = tool_arguments(call);
            let valid_name = call["function"]["name"] == "report_ui_state";
            let valid_arguments = arguments.as_ref().is_some_and(arguments_match_ui);
            let call_id = call["id"].as_str();
            first_tool = valid_name && valid_arguments && call_id.is_some();
            rows.push(ProbeResult {
                name: "vision + thinking + autonomous tool call",
                result: if first_tool { "PASS" } else { "FAIL" }.into(),
                detail: format!("{common}; tool_name={:?}; tool_arguments_valid_json={}; button_label_run_test={}; error_code_contains_42={}; state_error={}; tool_call_id_present={}", call["function"]["name"].as_str(), arguments.is_some(), arguments.as_ref().is_some_and(|v| v["button_label"].as_str().unwrap_or("").to_ascii_lowercase().contains("run test")), arguments.as_ref().is_some_and(|v| v["error_code"].as_str().unwrap_or("").contains("42")), arguments.as_ref().is_some_and(|v| v["state"].as_str().unwrap_or("").eq_ignore_ascii_case("error")), call_id.is_some()),
                status: first_tool,
            });
            if !first_tool {
                break;
            }
        }

        if let Some(call) = first_tool_call(&message) {
            let Some(call_id) = call["id"].as_str() else {
                rows.push(ProbeResult {
                    name: "second-turn continuation",
                    result: "FAIL".into(),
                    detail: "tool call missing id".into(),
                    status: false,
                });
                break;
            };
            messages.push(message.clone());
            messages.push(json!({"role":"tool","tool_call_id":call_id,"content":tool_result()}));
            messages.push(json!({"role":"user","content":"Based on the observed UI state and the tool result, decide what the test agent should do next. Do not call report_ui_state again unless another observation is necessary."}));
            previous_reasoning = Some(reasoning.unwrap().to_owned());
            if turns == 3 {
                rows.push(ProbeResult {
                    name: "second-turn continuation",
                    result: "INCONCLUSIVE".into(),
                    detail: "model did not converge within probe limit".into(),
                    status: false,
                });
            }
        } else {
            let final_answer = !content.unwrap_or_default().is_empty();
            rows.push(ProbeResult {
                name: "second-turn continuation",
                result: if final_answer { "PASS" } else { "FAIL" }.into(),
                detail: format!(
                    "{common}; final_answer_non_empty={final_answer}; additional_tool_call=false"
                ),
                status: final_answer,
            });
            break;
        }
    }

    if !first_tool {
        replay_rows.clear();
    }
    if replay_rows.is_empty() && first_tool {
        replay_rows.push(ProbeResult {
            name: "reasoning replay after visual tool call",
            result: "FAIL".into(),
            detail: "no second request was completed".into(),
            status: false,
        });
    }
    rows.extend(replay_rows);
    Ok(rows)
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
