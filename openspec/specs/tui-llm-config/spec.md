# tui-llm-config

## Purpose

TUI and CLI surfaces for LLM configuration that read and write the shared engine model-profile store (same store as Desktop and the daemon).

## Requirements

### Requirement: TUI and CLI use the shared model-profile store

The TUI and `teshi` CLI SHALL load, save, activate, and delete LLM model profiles through the shared `teshi-engine` model-profile store. They MUST NOT treat `dirs::config_dir()/teshi/auth.json`, `[providers.*]` in `config.toml`, or `teshi/models/*.toml` as the runtime source of truth after legacy import has run (or when the shared store already has profiles).

#### Scenario: Model panel activates engine profile

- **WHEN** the user activates a profile in the TUI model panel
- **THEN** the shared store active pointer is updated and subsequent TUI LLM calls use `profile_to_llm_config` / `effective_llm_config` for that profile

#### Scenario: teshi auth lists masked profile keys

- **WHEN** the user runs `teshi auth list`
- **THEN** the CLI lists profiles from the shared store with masked API keys and indicates the active profile

#### Scenario: Env fallback when no profile key

- **WHEN** no active profile has a non-empty API key and `TESHI_LLM_API_KEY` is set
- **THEN** the TUI still starts the LLM worker using the env-based config
