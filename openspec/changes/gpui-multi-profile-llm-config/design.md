## Context

GPUI settings currently host a single-form LLM config (`base_url`, `model`, `api_key`) persisted as `llm-config.json` under app data, with daemon `GET/PUT /api/v1/llm/config`. The engine speaks only OpenAI-compatible streaming `/chat/completions` (with DeepSeek `reasoning_content` support). The TUI already has a separate multi-profile store under `~/.config/teshi/models/`, but that store is not shared with GPUI and lacks Anthropic / API style / HTTP extras.

Product reference for UX and field shape: Chrys model configuration (providers `openai` / `anthropic` / `deepseek-openai`, API style for OpenAI, streaming, HTTP extra headers and chat options). Implementation stays in Teshi GPUI + `teshi-engine`, not Chrys/Python.

## Goals / Non-Goals

**Goals:**

- Multi-profile LLM configuration in GPUI Settings with Activate semantics.
- First-class providers: OpenAI, Anthropic, DeepSeek (OpenAI-compatible).
- Profile-driven transport: chat completions, OpenAI Responses, Anthropic Messages; stream on/off; HTTP extras.
- Shared app-data profile store used by desktop, daemon, and WASM backends.
- One-time migration from legacy `llm-config.json`.
- Preserve masked API key behavior and a compatible flat active-profile projection on legacy config endpoints.

**Non-Goals:**

- GLM provider.
- Vision, skip-TLS, bypass-proxy, or HTTP timeout/connection options UI.
- Migrating or rewriting TUI `~/.config/teshi/models/` in this change.
- Replacing agent chat UI or inventing a new event protocol beyond existing `LlmEvent`.

## Decisions

### 1. Profile store location: app data, not TUI config dir

**Choice:** Store profiles at `{app_data_dir}/model-profiles/{id}.json` plus `{app_data_dir}/model-profiles/active`.

**Why:** Matches the existing GPUI/daemon trust model (`llm-config.json` under the same app data root), keeps WASM/desktop/daemon on one store, and avoids coupling this change to TUI profile schema migration.

**Alternatives considered:** Reuse `~/.config/teshi/models/` — rejected for this change to avoid breaking TUI mid-flight; schema lives in `teshi-engine` so a later unification is possible.

### 2. Provider IDs match Chrys naming

**Choice:** `openai`, `anthropic`, `deepseek-openai` with defaults:

| Provider | Default base URL |
|----------|------------------|
| `openai` | `https://api.openai.com/v1` |
| `anthropic` | `https://api.anthropic.com` |
| `deepseek-openai` | `https://api.deepseek.com` |

**Why:** Aligns with the referenced Chrys UX and client factory naming; DeepSeek stays explicitly OpenAI-compatible.

### 3. Profile schema (engine-owned)

**Choice:** `ModelProfile` fields: `id`, `name`, `provider`, `api_style`, `model_id`, `max_context_tokens`, `max_output_tokens`, `base_url`, `api_key`, `stream` (default `true`), `http_headers` (object/map), `chat_options` (object/map).

**Why:** Covers Chrys Model Options + Extra Options needed for transport; omits Connection Options for this change. Empty `base_url` resolves via provider default at effective-config time. `api_style` is meaningful only for `openai`; other providers force chat-completions semantics (Anthropic uses Messages transport regardless of stored style).

### 4. Transport router in `teshi-engine`

**Choice:** Extend `LlmConfig` with provider, api_style, stream, headers, chat_options. Route:

- `openai` + `chat_completions` or `deepseek-openai` → `/chat/completions` (DeepSeek keeps `reasoning_content`).
- `openai` + `responses` → `/responses` with adapter to `LlmEvent`.
- `anthropic` → `/v1/messages` with `x-api-key` + `anthropic-version`, adapter to `LlmEvent`.

HTTP extras: merge profile headers into the request; shallow-merge `chat_options` into the JSON body with core fields taking precedence. Non-stream: one-shot JSON response synthesized into the same `LlmEvent` sequence.

**Why:** Keeps upper layers on one event surface; avoids dual agent loops.

**Alternatives considered:** Separate client crates per provider — deferred; start as modules inside `teshi-engine` for faster iteration.

### 5. GPUI UI shape

**Choice:** Rebuild settings LLM section as list + form (left profiles, right editor), chrys-inspired field grouping, GPUI-native controls (no Textual). Select-like fields cycle with Space when focused. Headers/chat options: editable key=value rows (JSON textarea acceptable fallback if row editor is too heavy).

**Why:** Fits existing settings surface and keyboard model (`LlmConfigView` key context).

### 6. Daemon API compatibility

**Choice:** Add `/api/v1/llm/profiles` CRUD + activate. Keep `GET/PUT /api/v1/llm/config` as a flat projection of the **active** profile (`base_url`, `model`, masked key, plus new fields where useful) so older web clients do not hard-fail.

**Why:** Soft migration path for WASM backend evolution.

### 7. Legacy migration

**Choice:** On first profile-store access, if no profiles exist and `llm-config.json` has usable data, create profile `Default`, copy fields, activate it, leave the old file in place, write a small migration marker under `model-profiles/`.

**Why:** No user data loss; reversible by deleting the new directory if needed.

## Risks / Trade-offs

- [OpenAI Responses / Anthropic tool-call fidelity] → Mitigation: define acceptance against existing `LlmEvent::ToolCallRequest` / Chunk / Done; add mock-HTTP unit tests for URL, auth headers, and tool mapping; prioritize chat completions + Anthropic text/tools before polishing Responses edge cases.
- [Dual profile stores (GPUI app data vs TUI)] → Mitigation: document explicitly; engine schema is the future shared type; no silent cross-writes this change.
- [Non-stream path regressions] → Mitigation: synthesize the same event sequence; integration test Done/ToolCall without SSE.
- [API key still plaintext on disk] → Mitigation: unchanged local-trust model; never log keys; continue masked public snapshots.
- [GPUI form complexity] → Mitigation: ship functional list/form with cycling selects before polish; key=value extras over full KV widget if needed.

## Migration Plan

1. Ship profile store + migration + chat-completions path with stream/headers (behavior-preserving for migrated Default).
2. Ship GPUI multi-profile UI + daemon profile APIs; wire `effective_llm_config` to active profile.
3. Ship Anthropic Messages adapter.
4. Ship OpenAI Responses adapter.
5. Rollback: revert to previous release; users can delete `model-profiles/` and fall back to env/`llm-config.json` only if code paths retain env fallback—keep `TESHI_LLM_*` env fallback when no active profile/key exists.

## Open Questions

- None blocking: Responses tool mapping details will follow OpenAI’s current SSE event shapes during implementation; if a tool shape cannot map cleanly, fail the request with a clear error rather than silently dropping tools.
