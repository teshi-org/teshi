## Context

Teshi already packages the locator Skill, MCP metadata, and `teshi-bridge` inside MSI/Release `share/` trees. External agents still copy Skills by hand (or fail to find them). This change adds an agent-executable runbook and a CLI that installs from that local tree into user-global Skill discovery paths.

The first version targets the Agent toolchain (CLI + Chrome extension + Skills), not a full product bootstrap of Desktop/WASM/Python.

## Goals / Non-Goals

**Goals:**
- Give coding agents a single English runbook (`AI_INSTALL.md`) reached from README.
- Install bundled Skills from the local share/checkout tree into `~/.agents/skills/<name>` with optional discovery-path symlinks.
- Require a dry-run (and interactive confirm, or `--yes` when stdin is not a TTY) before writing files.
- Keep Skill bytes version-locked to the installed CLI by never fetching `SKILL.md` from the network.

**Non-Goals:**
- Publishing `@teshi/cli` or any npm optional-binary package.
- Automating `chrome://extensions` Load unpacked / Reload.
- Bundling `teshi-desktop`, GPUI WASM rebuilds, or Python/venv bootstrap into this change.
- Installing MCP config or the Codex plugin.
- Downloading Skills from GitHub raw URLs.

## Decisions

- **Runbook over installer script.** Agents already execute shell commands. A root `AI_INSTALL.md` plus a README one-liner matches how they discover install steps. A new npm wrapper would duplicate winget/Release and fight GitHub English/install policy.
- **CLI lives in `teshi-tui`.** `teshi install-skill` is a new `Command` variant dispatched from `crates/teshi-tui/src/lib.rs`, implemented in `crates/teshi-tui/src/cli/install_skill.rs`, beside existing non-TUI subcommands.
- **Source resolution is local-only, first match wins.**
  1. `current_exe()` directory: `share/teshi-browser-testing/skills` then `../share/teshi-browser-testing/skills` (MSI `bin/` vs portable zip layout).
  2. Walk up from the executable looking for `agent-packages/teshi-browser-testing/skills` (cargo/dev tree).
  3. Otherwise fail with a message that points at winget/Release, not GitHub Skill URLs.
- **Install set.** Always copy packaged `playwright-locator`. If the resolved tree also exposes repo `skills/bdd-feature` and `skills/winapp-regression`, copy those too. On name collision, the packaged copy wins so two `playwright-locator` trees cannot clobber each other.
- **Layout.** Canonical files go to `~/.agents/skills/<name>`. If a parent already exists, create a symlink at `~/.cursor/skills/<name>`, `~/.claude/skills/<name>`, `~/.codex/skills/<name>`, `~/.config/agents/skills/<name>`, and `~/.gemini/skills/<name>`. Missing parents are skipped, not created, so unused Agent products stay untouched.
- **No overwrite of real directories.** If a discovery-path target exists and is not a symlink, skip it even with `--yes` and print a warning. Existing symlinks may be replaced to point at the canonical entity. The canonical `~/.agents/skills/<name>` directory is created or updated because it is the managed copy.
- **Confirm gates.** `--dry-run` prints the plan and writes nothing. A TTY without `--yes` prompts yes/no. Non-TTY without `--yes` errors. `--yes` is for scripts and for agents after the user has confirmed the dry-run in chat.
- **Windows symlink failures are errors, not success.** If `symlink_dir` fails (missing Developer Mode / elevation), the command reports an actionable hint. Canonical copy can still succeed; skipped links are listed.

## Risks / Trade-offs

- [Windows directory symlinks need Developer Mode or elevation] → Surface the OS error with a Developer Mode / Administrator hint; do not silently report success.
- [Agent shells are rarely a TTY] → `AI_INSTALL.md` sequences `--dry-run`, user confirmation in chat, then `--yes`.
- [Repo `skills/playwright-locator` and the packaged Skill share a name but different contracts] → Packaged copy always wins on collision.
- [User-global install vs project-vendored `.agents/skills`] → This change adds the global path; project vendoring in the existing spec remains valid.

## Migration Plan

Ship the CLI and docs together. Existing manual copies under a consumer repo `.agents/skills` continue to work. Re-running `teshi install-skill` updates the managed `~/.agents` copy and refreshes symlinks; it never replaces a real directory at a discovery path.

Rollback is removing the subcommand and docs; no on-disk schema is introduced.

## Open Questions

None. Chrome extension directory and Skill confirmation remain human gates, as in the install runbook.
