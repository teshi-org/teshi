## Why

Teshi already ships a winget/MSI/Release CLI, a `teshi-bridge` Chrome extension zip, and bundled Skills under `share/teshi-browser-testing`. External coding agents still lack an executable install runbook and a command that copies those Skills into Agent discovery paths. The gap is onboarding, not packaging: a user cannot tell Cursor or Claude one sentence and have the teshi browser toolchain installed.

## What Changes

- Add a root `AI_INSTALL.md` runbook that an agent can follow: install the teshi CLI, guide the user to load `teshi-bridge`, then install Skills from the local install tree.
- Add a one-sentence pointer in `README.md` (and a matching Chinese pointer in `README_zh.md`) plus short links from `doc/installation.md` and `doc/browser-modes.md`.
- Add `teshi install-skill` to copy bundled Skills into `~/.agents/skills/<name>` and create discovery-path symlinks, with `--dry-run` before any write.
- Skills are copied only from the local `share/` tree or the source checkout. Agents MUST NOT download `SKILL.md` from GitHub.

## Capabilities

### New Capabilities

- None. This extends the existing agent-testing distribution contract rather than introducing a separate capability.

### Modified Capabilities

- `agent-testing-distribution`: Add an AI-guided install path (README one-liner + `AI_INSTALL.md` + `teshi install-skill` from the local share tree, with dry-run before writes).

## Impact

- CLI: new `teshi install-skill` subcommand in `crates/teshi-tui`.
- Docs: `AI_INSTALL.md`, README pointers, `doc/installation.md`, `doc/browser-modes.md`.
- Discovery: user-global `~/.agents/skills` plus optional symlinks under Cursor/Claude/Codex/Gemini/agents paths when those parent directories already exist.
- No npm package, no Chrome Web Store publish, no automated `chrome://extensions` clicks, and no WASM/desktop install in this change.
