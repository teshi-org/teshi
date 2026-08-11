## 1. Engine app-data path and migrations

- [x] 1.1 Change `default_app_data_dir` to `…/teshi` and add one-time copy from `teshi-desktop` with marker
- [x] 1.2 Add TUI legacy import (TOML models + config.toml/auth.json) with marker; map provider ids
- [x] 1.3 Prefer `chat_options.temperature` in `profile_to_llm_config` when present
- [x] 1.4 Export migration helpers as needed; add unit tests for path migrate + TUI import idempotency

## 2. TUI / CLI on shared store

- [x] 2.1 Rewire `spawn_llm_if_configured` / activate paths to engine profiles + `effective_llm_config`
- [x] 2.2 Point model panel CRUD/activate at `teshi_engine::model_profile`; remove TUI TOML profile persistence
- [x] 2.3 Rewrite `teshi auth` over shared profiles (list/login/remove/status); stop writing live `auth.json`
- [x] 2.4 Simplify `llm.rs` to use engine `LlmConfig` without forcing `PROVIDER_OPENAI`

## 3. Docs and specs

- [x] 3.1 Update `doc/cli-usage.md`, `doc/architecture.md`, `doc/user-guide.md` for unified store paths
- [x] 3.2 Keep OpenSpec change deltas accurate; sync main specs when archiving

## 4. Quality gates

- [x] 4.1 `cargo fmt --all`
- [x] 4.2 `cargo test -p teshi-engine -p teshi-tui --locked` (and related)
- [x] 4.3 `cargo clippy -p teshi-engine -p teshi-tui --locked --all-targets --all-features -- -D warnings`
