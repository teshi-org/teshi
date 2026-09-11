# DeepSeek V4.1 Flash Multimodal Capability Probe

This is a live, opt-in capability probe, not a product integration. It uses
`POST https://api.deepseek.com/chat/completions` with model `deepseek-flash`.

## Run

PowerShell:

```powershell
$env:DEEPSEEK_API_KEY = '...'
$env:TESHI_RUN_LIVE_DEEPSEEK_PROBE = '1'
cargo run -p teshi-engine --example deepseek-v41-multimodal-probe --locked
```

An absent key or opt-in flag produces `SKIP` and exit code 0. Set
`TESHI_DEEPSEEK_PROBE_IMAGE_URL` to run the optional external-URL probe. The
probe never prints the authorization header, key, complete base64 image, or
reasoning text.

## Capability matrix

| Capability | Result | Evidence |
|---|---|---|
| `deepseek-flash` text | PASS | HTTP 200; `finish_reason=stop`; content and `reasoning_content` present |
| base64 image | PASS | HTTP 200; content and `reasoning_content` present; no complete reasoning text retained |
| streaming vision | PASS | HTTP 200; content delta events, reasoning delta events, usage, terminal finish event, and `[DONE]` observed |
| vision + thinking | PASS | HTTP 200; `thinking.type=enabled`, `reasoning_effort=high`; `reasoning_content` present |
| `reasoning_content` | PASS | Present in text, base64-image, streaming, and explicit-thinking responses; only length/field presence recorded |
| vision + tool call | PASS | HTTP 200; `finish_reason=tool_calls`; valid function tool call present; thinking explicitly disabled |
| vision + tool + second turn | PASS | Tool result was appended and a second assistant text response was returned |
| external URL image | NOT RUN | No stable public fixture URL was supplied via `TESHI_DEEPSEEK_PROBE_IMAGE_URL` |
| image in system | REJECTED | HTTP 400 `invalid_request_error`: image in system message unsupported |
| image in assistant | REJECTED | HTTP 400 `invalid_request_error`: image in assistant message unsupported |
| invalid image data | REJECTED | HTTP 400 `invalid_request_error`: invalid base64 data |

## Interpretation

The committed fixture is a small deterministic PNG containing `TESHI`, a
green `RUN TEST` button, and `ERROR 42`. A live run is required before any
row is treated as a V4.1 fact. The live run below is the current evidence;
the probe samples response field names and SSE shape without persisting private
reasoning.

The required vision, thinking, tool-call, and second-turn probes pass, so Teshi
can enter the Multimodal Message Model design/implementation stage. This is not
permission to change the production model in this probe: browser screenshot
capture, Teshi UI support, Responses API support, and production LLM integration
remain out of scope.

## Live result (2026-09-11)

The probe was invoked in one PowerShell process with the explicit opt-in flag,
the real endpoint/model, and `DEEPSEEK_API_KEY` present in that process. The
key value was never printed, persisted, or submitted to this repository.

Observed sanitized results:

- text baseline: `PASS`, HTTP 200, `model=deepseek-flash`, content and
  `reasoning_content` present;
- base64 image: `PASS`, HTTP 200, content and `reasoning_content` present;
- streaming vision: `PASS`, with content and reasoning delta events, usage,
  `finish_reason=stop`, and `[DONE]`;
- vision + thinking: `PASS`, with `thinking.type=enabled` and
  `reasoning_effort=high`;
- vision + tool call: `PASS`, with `thinking.type=disabled`, a valid tool call,
  and `finish_reason=tool_calls`;
- vision + tool + second turn: `PASS`, with a returned assistant text response;
- external URL: `NOT RUN`, because no stable public URL was supplied;
- all three negative probes: `REJECTED` with HTTP 400 and the expected
  unsupported/invalid-input errors.

The current evidence supports the conclusion **YES, proceed to the Multimodal
Message Model stage**, while keeping the implementation uncommitted to this
provider-specific probe contract until the model design is reviewed.

## Candidate wire shapes and evidence boundary

The probe sends the candidate Chat Completions shape
`messages[].content = [{"type":"text","text":"..."},{"type":"image_url","image_url":{"url":"data:image/png;base64,..."}}]`.
The detail variant was also accepted with `image_url.detail=auto`; the probe
does not record the image contents. Thinking uses top-level
`thinking.type=enabled` plus `reasoning_effort=high`. Tool requests use the
normal Chat Completions `tools[].function` schema, forced function
`tool_choice`, and explicitly set `thinking.type=disabled`; omitting the
explicit disabled value was rejected by the provider as incompatible with
`tool_choice`.

The successful streaming response had SSE `data:` JSON events with
`choices[0].delta.content` and `choices[0].delta.reasoning_content`, a finish
event, a usage-bearing event, and terminal `[DONE]`. This records the observed
response shape, not a general SDK guarantee.

The older Vision Exp documentation is retained only as candidate guidance:
its model id, endpoint assumptions, and limitations are not used for
acceptance. This probe uses the current `https://api.deepseek.com` endpoint and
`deepseek-flash`, never falls back to `deepseek-v4-flash` or
`deepseek-v4-flash-vision-exp`, and tests Chat Completions content-part arrays,
streaming, thinking, tools, and multi-turn tool results directly.
