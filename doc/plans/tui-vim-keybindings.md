# TUI Vim and arrow-key consistency

## Contract

- Normal mode owns commands; Insert mode accepts all printable characters.
- Arrow keys remain available alongside Vim navigation. Text editors use arrows
  for the cursor, lists use Up/Down, and trees retain their collapse/expand behavior.
- `Ctrl+W` followed by `h/j/k/l` or arrows moves pane focus. Existing Tab and
  non-text pane-navigation keys remain compatible. Remove the shadowed When binding.
- `Esc` leaves input or closes the current modal layer. Input mode and available
  commands must be visible, including on narrow terminals.
- Review approvals remain explicit human actions. Editing test-point meaning
  continues to reset its review state; keyboard changes must preserve that contract.
- Model/session deletion requires an explicit confirmation for keyboard and mouse.

## Implementation

- [x] Give Test Points details Normal/Insert modes and Unicode-safe cursor editing.
- [x] Keep printable `j/k` in model forms; add cursor editing and Ctrl+S submission.
- [x] Add arrow-key pane chords, consistent saving, first/last and page navigation.
- [x] Use `j/k` and arrows for AI history in Normal, `i`/Enter for input, and
      `[`/`]` for previous/next agent conversation.
- [x] Confirm model/session deletion; make quit `y/n` decisions unambiguous.
- [x] Update mode indicators, popup hints, and the complete keybinding reference.
- [x] Run unit/integration checks and real Windows PTY regressions against this
      checkout's binary with isolated stores; retain viewport and persistence evidence.

## Scope

This is a bounded Vim subset, not a full Vim emulator. Preserve existing Feature
structural editing, requirement iteration/group keys, project switching, ACP
human review, and unrelated working-tree changes. No commit or push is included.


## Validation evidence (Windows, 2026-10-22)

The candidate was built from working-tree sources on HEAD
`cb57764a735a89f54e280e90d049d82ae23571a5`. The existing `target/debug/teshi.exe`
process was preserved. The independent `target/debug/teshi-keybindings.exe` is
pinned by binary SHA256 and source hashes in
`.teshi/artifacts/vim-keybindings-20261022/build.json`; its cached embedded build
identity is not used as proof of the checkout revision.

- TUI unit tests: 496 passed, 2 environment-dependent tests ignored.
- TUI all-target/all-feature Clippy with `-D warnings`: passed.
- Native workspace formatting and formatting of changed runner tests: passed.
- Real PTY: interaction 6, management 16, Test Points 6, Requirements Vim 8,
  and AI controls 14; all 50 passed, with normal/narrow viewport and disk checks.
- English and Chinese TUI Feature adapters: 2 Rust harness tests passed,
  replaying both scenarios per locale and checking saved Feature content.
- Rustdoc completed with two pre-existing warnings in `editor_buffer.rs` and
  `cli/mod.rs`; these unrelated lines were preserved.
- Runner default suite: 23 passed, 124 ignored (PTY/environment gates run separately as listed above).
- Final runner all-target Clippy with `-D warnings` passed.
- Python fixture syntax checks passed.

Logs and captured TXT/SVG/persisted-data evidence live under
`.teshi/artifacts/vim-keybindings-20261022/`. Initial failing PTY attempts and the
baseline reproduction remain available. The existing cross-document Vim fixture
omitted project membership for its second document; it failed against the existing
binary too. Adding that membership restored all eight regressions without changing
product filtering.

## Shortcut migrations

- AI Normal: `j/k` scroll history; `[`/`]` switch Agent conversations.
- Feature Normal: `u`/`Ctrl+R` undo/redo; F5 replaces the old background-run key.
- `Ctrl+W` is only the pane prefix; choose When through the keyword picker.
- Test Points details require `i`/Enter before typing. Esc/Enter commits;
  Ctrl+S saves while retaining Insert when the selected point remains visible.
- Model/session `dd`, Delete, and pointer delete actions open confirmation.
  Failures keep the dialog, stable target ID, and an explanatory error.

These checks establish scoped Windows behavior, not full native workspace,
Linux/macOS, hosted WASM, or live provider acceptance. No global installation,
process restart, commit, or push was performed.
