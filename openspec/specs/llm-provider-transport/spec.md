# llm-provider-transport

## Purpose

Provider-aware LLM request routing in `teshi-engine`: OpenAI chat completions / Responses, Anthropic Messages, DeepSeek OpenAI-compatible paths, streaming toggle, and HTTP extras merged into outbound requests.

## Requirements

### Requirement: Provider-aware request routing

`teshi-engine` SHALL route LLM requests based on the effective profile provider and API style while emitting the existing `LlmEvent` surface (`Chunk`, `ToolCallRequest`, `Done`, and existing error/completion semantics).

#### Scenario: OpenAI chat completions route

- **WHEN** effective config has `provider` `openai` and `api_style` `chat_completions`
- **THEN** the engine calls `{base_url}/chat/completions` with Bearer auth

#### Scenario: DeepSeek OpenAI-compatible route

- **WHEN** effective config has `provider` `deepseek-openai`
- **THEN** the engine calls `{base_url}/chat/completions` with Bearer auth and preserves DeepSeek `reasoning_content` round-trip behavior already required for tool turns

#### Scenario: OpenAI Responses route

- **WHEN** effective config has `provider` `openai` and `api_style` `responses`
- **THEN** the engine calls the OpenAI Responses endpoint under the configured base URL and adapts streamed or non-streamed results into `LlmEvent` values

#### Scenario: Anthropic Messages route

- **WHEN** effective config has `provider` `anthropic`
- **THEN** the engine calls `{base_url}/v1/messages` using Anthropic API key header conventions (`x-api-key` and `anthropic-version`) and adapts results into `LlmEvent` values

### Requirement: Streaming toggle

The engine SHALL honor the profile `stream` flag. When `stream` is true, it MUST use the provider’s streaming protocol. When `stream` is false, it MUST perform a non-streaming request and still produce an equivalent `LlmEvent` sequence culminating in `Done` (and tool-call events when tools are requested).

#### Scenario: Non-streaming completion still emits Done

- **WHEN** a chat request runs with `stream` false and the provider returns a successful full response
- **THEN** consumers receive `Done` with the full text (and tool-call events if present) without requiring SSE

### Requirement: HTTP extras applied to outbound requests

The engine SHALL merge profile `http_headers` into the outbound HTTP request headers and shallow-merge `chat_options` into the JSON request body. Core required fields (model, messages/input, stream, max tokens, tools) MUST take precedence over conflicting `chat_options` keys.

#### Scenario: Custom header sent

- **WHEN** the active profile includes HTTP header `X-Test` = `1`
- **THEN** the outbound LLM HTTP request includes `X-Test: 1`

#### Scenario: Chat option merge does not override model

- **WHEN** `chat_options` contains a `model` key different from the profile `model_id`
- **THEN** the request body uses the profile `model_id`
