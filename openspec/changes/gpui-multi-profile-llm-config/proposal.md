## Why

The GPUI settings LLM form is a single flat config (base URL, model, API key) that cannot express real provider choice or chrys-style model profiles. Users need first-class OpenAI, Anthropic, and DeepSeek (OpenAI-compatible) profiles with API style, streaming, and HTTP extras, plus engine transport that actually routes those choices—without waiting for a full TUI/GPUI store unification.

## What Changes

- Replace the single-config GPUI LLM settings form with a **multi-profile** Model Configuration UI (list + editor), hosted under settings: New / Clone / Delete / Activate / Save.
- Support built-in providers: `openai`, `anthropic`, `deepseek-openai` (no GLM).
- Persist named profiles under app data (`model-profiles/`) with an active-profile pointer; one-time migrate existing `llm-config.json` into a Default profile when needed.
- Extend profile fields beyond the spike: provider, API style (OpenAI only: chat completions vs responses), model, max context / max output tokens, base URL, API key, streaming flag, HTTP extra headers, chat options.
- Extend `teshi-engine` LLM transport to honor provider + API style (chat completions, OpenAI Responses, Anthropic Messages), streaming on/off, and HTTP extras, while keeping a unified `LlmEvent` surface.
- Extend daemon HTTP APIs for profile CRUD + activate; keep `GET/PUT /api/v1/llm/config` as a flat projection of the **active** profile for compatibility.
- **Out of scope**: GLM; Vision / skip-TLS / bypass-proxy / HTTP timeout UI; migrating or rewriting TUI `~/.config/teshi/models/`; Python chrys runtime.

## Capabilities

### New Capabilities

- `llm-model-profiles`: Named model profile schema, app-data persistence, active selection, and one-time migration from the legacy single `llm-config.json` store.
- `llm-provider-transport`: Engine routing for OpenAI chat completions, OpenAI Responses, Anthropic Messages, and DeepSeek OpenAI-compatible chat completions, including stream toggle and HTTP extras, emitting existing `LlmEvent` semantics.

### Modified Capabilities

- `gpui-llm-config`: Settings-hosted UI and backends become multi-profile (provider options, API style, streaming, HTTP extras) instead of a single three-field form; daemon profile APIs added while legacy flat config endpoints remain as active-profile projections.

## Impact

- **UI**: `crates/teshi-ui` (`LlmConfigView`, `LlmConfigBackend` DTOs/trait).
- **Engine**: `crates/teshi-engine` (new profile store module; major `llm.rs` transport branching).
- **Apps**: `teshi-desktop`, `teshi-web`, `teshi-daemon` (profile REST + effective config from active profile).
- **Specs**: new `llm-model-profiles` and `llm-provider-transport`; delta for `gpui-llm-config`.
- **Risk**: Anthropic Messages and OpenAI Responses tool-call mapping are the highest complexity areas; must align with existing `LlmEvent::ToolCallRequest` / Done / Chunk contracts.
