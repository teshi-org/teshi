## Context

Desktop/daemon already share `teshi-engine` JSON model profiles under `%APPDATA%/teshi-desktop/model-profiles/`. TUI keeps a parallel stack: `dirs::config_dir()/teshi/config.toml` providers, `auth.json`, and `models/*.toml`. Chrys unifies all surfaces on one `ModelProfile` store; Teshi should match that shape.

## Goals / Non-Goals

**Goals:**

- One on-disk profile store for TUI, CLI, Desktop, and daemon.
- Neutral app-data root `…/teshi` (not `teshi-desktop`).
- One-time migrations from desktop dir and legacy TUI TOML/auth.
- TUI LLM spawn and model/`teshi auth` UX edit the shared store.
- Preserve `TESHI_LLM_*` fallback when no usable profile key exists.

**Non-Goals:**

- OS keyring / `{{ENV_VAR}}` templates in profile files.
- Free-form provider registry (`[providers.ollama]` as a first-class id).
- Changing daemon HTTP API shapes.
- React web-ui redesign (already uses daemon profile APIs).

## Decisions

1. **SSOT = engine `ModelProfile` JSON** under `{app_data}/model-profiles/`. TUI drops its local `profiles::ModelProfile` type as the persistence layer.
2. **App data dir** = `dirs::data_dir()/teshi`, override via `TESHI_APP_DATA_DIR`. On first access, if the new root lacks migrated data and `…/teshi-desktop` exists, copy known artifacts (`model-profiles/`, `llm-config.json`, `settings.json`, `recent.json`, logs optional) and write a marker `.migrated-from-teshi-desktop`.
3. **Legacy TUI import** runs after desktop-dir migration and after `ensure_migrated` (llm-config.json). Only when the profile store has zero profiles (or none with a usable key, per marker `.migrated-from-tui-config`): import `config_dir/teshi/models/*.toml` and/or synthesize profiles from `config.toml` providers + `auth.json`. Map `deepseek` → `deepseek-openai`; unknown provider names with a custom base URL → `openai`.
4. **Runtime resolution order** for TUI: active engine profile with non-empty key via `effective_llm_config()` / `profile_to_llm_config`; else `TESHI_LLM_*`. Do not prefer `AppConfig.providers` after migration.
5. **`teshi auth`**: reimplement over profile CRUD (list masked, login sets key on matching/creating profile by provider, remove clears key or deletes, status shows app-data paths). Stop writing `auth.json` as the live store.
6. **Temperature**: TUI form may keep a temperature field mapped into `chat_options["temperature"]` for round-trip; engine `profile_to_llm_config` should prefer that option when present (small engine fix) so activation preserves user temperature.

## Risks / Trade-offs

- **Path rename** may surprise existing Desktop installs if migration fails; mitigate with copy-not-delete and marker + tests.
- **Provider id rename** (`deepseek` → `deepseek-openai`) can confuse users; document in CLI help and migration notes.
- **Dual TUI types during transition**: replace references carefully to avoid leftover TOML writes.
- Windows `data_dir` and `config_dir` both often map to Roaming; keep engine on `data_dir()/teshi` and only *read* legacy `config_dir()/teshi` for import.

## Migration Plan

1. Change `default_app_data_dir` → `teshi`.
2. Add `ensure_app_data_migrated()` called from `app_data_dir()`.
3. Add `ensure_tui_legacy_imported()` called from `ensure_migrated()` / profile list entrypoints.
4. Rewire TUI + `teshi auth`.
5. Update docs and main OpenSpec specs on archive.

## Open Questions

None — Chrys-aligned decisions locked by product direction.
