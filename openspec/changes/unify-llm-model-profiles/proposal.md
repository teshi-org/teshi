## Why

TUI/CLI and Desktop/daemon keep separate LLM provider stores (TOML + `auth.json` vs app-data JSON model profiles). Users must configure the same credentials twice, provider ids diverge (`deepseek` vs `deepseek-openai`), and TUI hardcodes OpenAI chat-completions even when profiles exist. Unifying on the engine `ModelProfile` store (Chrys-style) gives one source of truth across all surfaces.

## What Changes

- **BREAKING** (path): default app-data root changes from `…/teshi-desktop` to `…/teshi`, with one-time migration of existing desktop data.
- Make engine `model-profiles/` the single LLM configuration source for TUI, CLI, Desktop, and daemon.
- TUI model panel (`m`), `/auth`, and `teshi auth` read/write the same profile store (masked keys); stop treating `auth.json` and `[providers.*]` as the runtime truth.
- One-time import of legacy TUI `config.toml` providers + `auth.json` and `~/.config/teshi/models/*.toml` into the shared store when the target is empty.
- Keep `TESHI_LLM_*` as fallback when no usable active profile key exists.
- Document the unified paths and CLI; update OpenSpec main specs for shared store + TUI consumers.
- Custom / Ollama setups use `provider=openai` + custom `base_url` (no free-form provider registry).

## Capabilities

### New Capabilities

- (none)

### Modified Capabilities

- `llm-model-profiles`: App-data root is `teshi` (not `teshi-desktop`); one-time migrate from legacy desktop dir; one-time import from TUI TOML/auth; store is shared by TUI/CLI as well as Desktop/daemon.
- `gpui-llm-config`: Native/daemon shared-store requirement refers to the neutral `teshi` app-data directory.
- `tui-llm-config`: TUI/CLI MUST resolve and edit LLM settings via the shared engine model-profile store (new capability documenting TUI surface behavior).

## Impact

- **Engine**: `app_data.rs`, `model_profile.rs`, new migration helpers; exports used by TUI.
- **TUI**: `app.rs` LLM spawn/activate, `llm.rs`, `profiles/`, `auth/`, `cli/auth.rs`, config provider path demoted.
- **Desktop/daemon**: benefit from path rename + migration; APIs unchanged in shape.
- **Docs**: `doc/cli-usage.md`, `doc/architecture.md`, `doc/user-guide.md`.
- **Out of scope**: OS keyring; `{{ENV_VAR}}` templates; free-form `[providers.*]` registry; React web-ui schema changes (already via daemon).
