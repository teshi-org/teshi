# AGENTS.md

## Cursor Cloud specific instructions

`teshi` is a Rust workspace for a terminal-first BDD/Gherkin editor. Standard build/test/run
commands live in `doc/development.md`, `README.md`, and each `Cargo.toml`; general Rust
conventions are in `CLAUDE.md`. Only the non-obvious, environment-specific notes are here.

### Services / products (Linux-runnable)

- `teshi` CLI (`apps/teshi-cli`, binary name `teshi`): the flagship terminal TUI (`teshi <project>`)
  and the entry point for `teshi web`. Runs headless in a real TTY (e.g. tmux).
- `teshi web` (served by `apps/teshi-daemon`): browser GUI. Defaults to `127.0.0.1:20253`.
  Flags: `--project PATH`, `--port`, `--host`, `--no-open`, `--dist PATH`. The HTTP API under
  `/api/v1/*` uses session auth (`POST /api/v1/sessions`); the React app handles this itself.
- `teshi-desktop` (`apps/teshi-desktop`, GPUI) and `teshi-tauri` (`apps/teshi-tauri`) are
  Windows-primary GUI shells and are not runnable headless on this Linux VM.

### Build / lint / test — mirror CI (`.github/workflows/ci.yml`)

Always exclude `teshi-tauri` on Linux; CI does the same. Run from repo root:

- `cargo fmt --all --check`
- `cargo check --workspace --exclude teshi-tauri --locked`
- `cargo test --workspace --exclude teshi-tauri --locked`
- `cargo clippy --workspace --exclude teshi-tauri --locked --all-targets --all-features -- -D warnings`
- `cargo doc --workspace --exclude teshi-tauri --locked --no-deps --document-private-items`

Gotchas:
- Building `teshi-tauri` on Linux is not supported here (needs Node + Tauri host libs); keep it excluded.
- `teshi-desktop` (GPUI) only `cargo check`s in CI, but a full `cargo build`/`cargo build --workspace`
  links GPUI and requires `libxkbcommon-x11` (plus wayland/vulkan dev libs), already present in the VM
  snapshot. If a fresh environment fails to link `teshi-desktop` with `-lxkbcommon-x11`, install
  `libxkbcommon-x11-dev libwayland-dev libvulkan-dev` (and the CI list of xcb/gtk/glib/pango/cairo/atk/graphene dev libs).

### Running `teshi web`

`teshi web` serves prebuilt frontend assets from `apps/teshi-tauri/frontend/dist`. The update script
installs the npm deps, but the `dist` bundle is a build artifact and must be built once before use:

```
npm --prefix apps/teshi-tauri/frontend run build
```

Then: `./target/debug/teshi web --project <dir> --host 127.0.0.1 --port 20253 --no-open`.
Without `dist`, the daemon errors with "frontend dist not found".

### Git hooks

The repo's `.githooks/` are not enabled by default here and are stale (they reference an old
`desktop/` layout that no longer exists). Rely on the CI commands above as the real quality gates
rather than `.githooks/pre-commit`.
