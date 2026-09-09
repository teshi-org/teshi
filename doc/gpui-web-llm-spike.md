# GPUI WASM web shell

Supported web product path: shared GPUI `teshi-ui`, native `teshi-desktop`, WASM
`teshi-web`, and the loopback daemon connected to the hosted `teshi.org` shell.

## Scope

- **In:** browser-profile discovery, explicit selection, LLM configuration, and
  the hosted control/preview WebSocket adapters. `--dist` is diagnostic only.
- **Out:** Hugo `/app` publish, the full feature editor, and agent chat.

Marketing remains Hugo (`teshi-org.github.io`). The retired React/Vite application has been removed and is not served or shipped by `teshi web`.

## Prerequisites

- Rust **stable** for desktop / daemon.
- Rust **nightly** + `wasm32-unknown-unknown` for `teshi-web` (GPUI web / `wasm_thread`).
- `wasm-bindgen-cli` **0.2.126** (`cargo install wasm-bindgen-cli --version 0.2.126 --locked`).

## Build WASM dist

```powershell
# Windows
.\scripts\build-teshi-web.ps1
```

```bash
# Unix
bash ./scripts/build-teshi-web.sh
```

Output: `apps/teshi-web/dist/`.

## Hosted run

The daemon no longer hosts the production GPUI bundle. Launch the hosted shell:

```powershell
cargo run -p teshi-cli -- web
```

The CLI opens `https://teshi.org/app/#port=<port>&token=<session-token>` and
keeps the token out of logs. The initial surface is Browser Profiles; Settings
opens the shared LLM profile form.

For source/artifact-only checks:

```powershell
python scripts/test-hosted-ui-transport.py
```

## Desktop

```powershell
cargo run -p teshi-desktop
```

Uses the shared user-level store: `%APPDATA%/teshi/model-profiles/` (or XDG data-home equivalent; override with `TESHI_APP_DATA_DIR`).

## Crate rules

- `teshi-ui` / `teshi-web` must not depend on `teshi-engine` or `teshi-agent`.
- Desktop may use `teshi-engine` and direct loopback adapters for native platform I/O.
