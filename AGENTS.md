# AGENTS.md

## Cursor Cloud specific instructions

`teshi` is a Rust workspace for a terminal-first BDD/Gherkin editor. Standard build/test/run
commands live in `doc/development.md`, `README.md`, and each `Cargo.toml`; general Rust
conventions are in `CLAUDE.md`. Only the non-obvious, environment-specific notes are here.

### Services / products (Linux-runnable)

- `teshi` CLI (`apps/teshi-cli`, binary name `teshi`): the flagship terminal TUI
  (`teshi <project>`, e.g. `teshi . --recursive`) and the entry point for `teshi web`.
  Runs in a real TTY (e.g. tmux). `1`/`2`/`3` switch the Explore/MindMap/AI tabs;
  `e` opens the file editor, `s` saves. See `doc/keybindings.md`.
- `teshi web` (served by `apps/teshi-daemon`): browser GUI. Defaults to `127.0.0.1:20253`.
  It serves the prebuilt **GPUI WASM** frontend from `apps/teshi-web/dist` (see below).
  Flags: `--project PATH`, `--port`, `--host`, `--no-open`, `--dist PATH`.
- `teshi-desktop` (`apps/teshi-desktop`, GPUI) is a Windows-primary native shell; only
  `cargo check`s on this Linux VM (a full build links GPUI — see gotcha below).
- `teshi-web` (`apps/teshi-web`) is the only web frontend and the official **wasm32-only**
  GPUI-in-browser shell. The retired React/Vite application has been removed. Build it with
  `scripts/build-teshi-web.sh`. It does **not** compile on
  the native host and must be excluded from native workspace commands (see below). Building it
  needs the nightly toolchain + `wasm32-unknown-unknown` target + the `wasm-bindgen` CLI, none of
  which are installed by default.

### Build / lint / test (native Linux)

Run from repo root. On native Linux you must `--exclude teshi-web` (it is wasm-only):

- `cargo fmt --all --check`
- `cargo check --workspace --exclude teshi-web --locked`
- `cargo test --workspace --exclude teshi-web --locked`
- `cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings`
- `cargo doc --workspace --exclude teshi-web --locked --no-deps --document-private-items`

Gotchas:
- `teshi-desktop` (GPUI) only `cargo check`s in CI, but a full `cargo build -p teshi-desktop`
  links GPUI and requires `libxkbcommon-x11` (plus wayland/vulkan dev libs), already present in the
  VM snapshot. If a fresh environment fails to link with `-lxkbcommon-x11`, install
  `libxkbcommon-x11-dev libwayland-dev libvulkan-dev` (and the CI list of
  xcb/gtk/glib/pango/cairo/atk/graphene dev libs).

### Known pre-existing failures on `main` (not environment issues)

CI (`.github/workflows/ci.yml`) runs `cargo check --workspace --locked` **without** excluding
`teshi-web`, so it currently fails at the check step and never reaches the later gates. As a result
the following latent defects exist on `main` and are unrelated to environment setup:

- `cargo check --workspace` (no exclude) fails: `teshi-web` uses `gpui_platform::web_init` /
  `single_threaded_web`, both gated behind the `wasm` target. Use `--exclude teshi-web` on native.
- `teshi-engine` test `llm_responses::tests::test_responses_bearer_and_url` asserts the header name
  `Authorization`, but hyper/reqwest emit the canonical lowercase `authorization:`, so it fails
  (value `Bearer sk` is correct; only the header-name case differs).
- `teshi-tui` clippy `items_after_test_module` fires with `--all-targets`, failing the clippy gate.

Do not "fix" these as part of environment setup; call them out if they block a task.

### Running `teshi web`

The `dist` bundle is a build artifact and must be built once before use:

```
bash scripts/build-teshi-web.sh
```

Then: `./target/debug/teshi web --project <dir> --host 127.0.0.1 --port 20253 --no-open --dist apps/teshi-web/dist`.
Without `dist`, the daemon errors with "GPUI WASM dist not found". The daemon auto-resolves
`apps/teshi-web/dist` when run from the repo root, so `--dist` is optional there.

### Git hooks

The repo's `.githooks/` are not enabled by default here and are stale. Rely on the commands above
as the real quality gates rather than `.githooks/pre-commit`.
