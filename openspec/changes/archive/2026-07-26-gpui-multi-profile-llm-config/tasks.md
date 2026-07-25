## 1. Profile schema and store

- [x] 1.1 Add `ModelProfile`, `ApiStyle`, and provider constants/defaults module in `teshi-engine` (e.g. `model_profile.rs`)
- [x] 1.2 Implement app-data `model-profiles/` CRUD, active pointer, public masking helpers, and validation for built-in providers
- [x] 1.3 Implement one-time migration from legacy `llm-config.json` into activated `Default` profile with migration marker
- [x] 1.4 Point `effective_llm_config()` at the active profile (keep `TESHI_LLM_*` env fallback when no usable key)
- [x] 1.5 Add unit tests for store CRUD, defaults for empty base URL, migration idempotency, and last-profile delete rules

## 2. Transport: chat completions, stream, HTTP extras

- [x] 2.1 Extend `LlmConfig` with `provider`, `api_style`, `stream`, `http_headers`, `chat_options` and map from `ModelProfile`
- [x] 2.2 Apply HTTP header merge and chat_options shallow merge (core fields win) on chat-completions requests
- [x] 2.3 Implement non-streaming chat-completions path that synthesizes the existing `LlmEvent` sequence
- [x] 2.4 Keep DeepSeek `deepseek-openai` on chat completions with existing `reasoning_content` behavior
- [x] 2.5 Add tests for header injection, chat_options precedence, and stream=false Done emission (mock HTTP where practical)

## 3. Transport: Anthropic and OpenAI Responses

- [x] 3.1 Implement Anthropic Messages request builder (URL, `x-api-key`, `anthropic-version`, body conversion) and SSE/non-stream adapters to `LlmEvent`
- [x] 3.2 Map Anthropic tool use to `LlmEvent::ToolCallRequest` / Done semantics
- [x] 3.3 Implement OpenAI Responses route (`api_style=responses`) with stream and non-stream adapters to `LlmEvent`
- [x] 3.4 Map Responses tool calls into existing tool-call events; fail clearly if unmappable
- [x] 3.5 Add mock-HTTP tests for Anthropic and Responses URL/auth/routing and basic event adaptation

## 4. Backend and daemon APIs

- [x] 4.1 Extend `teshi-ui` backend DTOs/trait with list/get/save/delete/activate profile operations (masked keys)
- [x] 4.2 Implement native desktop backend against the engine profile store
- [x] 4.3 Add daemon routes: `GET/PUT/DELETE /api/v1/llm/profiles`, `GET /api/v1/llm/profiles/{id}`, `POST .../activate`
- [x] 4.4 Rewire `GET/PUT /api/v1/llm/config` as active-profile flat projection (compat)
- [x] 4.5 Implement WASM backend profile calls via same-origin fetch to the new endpoints

## 5. GPUI multi-profile settings UI

- [x] 5.1 Rebuild settings LLM view as profile list + editor (New / Clone / Delete / Activate / Save)
- [x] 5.2 Add Model Options fields: name, provider cycle, API style (openai only), model, max context, max output, base URL, API key, streaming
- [x] 5.3 Implement provider-change label/default base URL behavior without wiping custom URLs
- [x] 5.4 Add Extra Options editors for HTTP headers and chat options (key=value rows or JSON fallback)
- [x] 5.5 Preserve API key mask-on-load and empty-key-preserves-stored behavior; update keybinding help text

## 6. Verification and docs gate

- [x] 6.1 Run `cargo fmt --all`, `cargo clippy --all-targets --all-features -D warnings`, and targeted/engine/UI tests
- [x] 6.2 Manually verify desktop settings profile CRUD/activate and daemon projection round-trip
- [x] 6.3 Confirm openspec delta specs remain accurate after implementation (update only if behavior drifted)
