# ACP session controls acceptance

## Implemented behavior

- Agent-advertised configuration is retained from new/load responses and idle/active updates, with select groups, boolean values, unknown-type visibility, and legacy model/mode routing.
- Settings connect before prompting, stay scoped to their backend, serialize mutations, retain confirmed state on rejection, refresh dependent choices, and require reconnection after timeout. Reconnection advances the revision to reject old picker selections.
- The bottom TUI row exposes clickable agent configuration and effective client approvals. `/settings`, `Ctrl+,`, `/models`, the model shortcut, and overflow provide keyboard-accessible navigation. Agent modes and client permission policy remain separate.
- Client approval changes affect future permission requests; pending requests retain their explicit decision path. Environment Auto overrides are labeled.

## Automated evidence (Windows, 2026-10-15)

- `cargo test -p teshi-acp --locked`: 21 unit and 12 protocol integration tests passed; two existing live CodeBuddy tests ignored.
- `cargo test -p teshi-agent-runtime --test acp_backend --locked`: 13 tests passed, including model-dependent reasoning options, boolean wire values, rejection recovery, idle updates, timeout invalidation, backend isolation, reconnection, and permission policy changes.
- `cargo test -p teshi-tui -p teshi-agent-runtime -p teshi-acp --locked --lib --tests`: passed; TUI reports 376 passed and one existing ignored test.
- `cargo check --workspace --exclude teshi-web --locked`: passed, including desktop and daemon consumers.
- `cargo clippy -p teshi-acp -p teshi-agent-runtime -p teshi-tui --locked --all-targets --all-features -- -D warnings`: passed.
- Formatting and diff whitespace checks passed. The independent TUI runner test is separately formatted.

## Real ACP agent

Ran `acp-settings` against the installed Codex ACP Node entrypoint using a temporary project. It advertised Mode, Collaboration mode, Model, Reasoning effort, and Fast mode. Model changed from 6 Astra to 6.1 Sol on the same session and was restored. Reasoning effort changed from Low to Medium. Mode was confirmed without broadening permissions. A subsequent text-only prompt returned `ACP_SETTINGS_OK`.

The smoke test sends no file-edit request and rejects unexpected tool permission requests. It does invoke the configured provider. This establishes compatibility with this installed Codex agent, not every ACP provider.

## Real terminal interaction

`acp_settings_support_mouse_keyboard_and_overflow` launches the actual Teshi binary in Windows ConPTY through teshi-tui-runner with an isolated fake ACP process and temporary app data. It opens settings before prompting, sends SGR mouse input to the bottom Model control, selects Beta, observes Thinking becoming Low, resizes to 64 columns, selects the Fast boolean with the keyboard, and observes `Fast: On` in the rendered viewport.

The ordinary CLI build could not overwrite a running `target/debug/teshi.exe`. The existing process was preserved. A separate `target/debug/teshi-acp-controls.exe` was built with `cargo rustc ... -- -o ...` for terminal acceptance. The first test attempt used an incorrect initial-screen assertion (feature title instead of filename); the assertion was corrected to `sample` and the new binary passed. The global installed CLI was not replaced.

## Boundaries

- No full native workspace test/clippy run or WASM validation was performed; affected crates and native workspace compilation were validated.
- Real provider and real terminal checks are complementary: the automated terminal test uses a fake agent; the Codex test uses the production runtime without the terminal frontend.
- Future unrecognized ACP option types remain visible read-only. Only agent-advertised select and boolean types are editable.
- No automatic preference persistence, ACP v2 migration, or reference-project edits are included.
- Existing concurrent authoring, requirements, and installer work remains separate and was not committed by this task.
