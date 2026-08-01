# GPUI web LLM config spike (Path 1)

Minimal closed loop: shared GPUI `teshi-ui` (LLM settings only), native `teshi-desktop`, WASM `teshi-web`, and `teshi-daemon` same-origin hosting.

## Scope

- **In:** LLM config UI (base URL, model, API key), `GET/PUT /api/v1/llm/config`, daemon `--dist` serving GPUI assets.
- **Out:** React/Tauri replacement, Hugo `/app` publish, WebSocket events, feature editor, agent chat.

Marketing site remains Hugo (`teshi-org.github.io`). React remains the default production `dist` until a later migration.

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

## Path 1 run (daemon hosts GPUI)

Point the existing web host at the GPUI dist (React default resolver unchanged):

```powershell
cargo run -p teshi-cli -- web --no-open --dist "apps/teshi-web/dist"
```

Open the printed `http://127.0.0.1:<port>/` URL. Save LLM settings in the GPUI form; reload and confirm status shows configured (masked key).

API check:

```powershell
Invoke-RestMethod http://127.0.0.1:<port>/api/v1/llm/config
```

## Desktop

```powershell
cargo run -p teshi-desktop
```

Uses the shared user-level store: `%APPDATA%/teshi/model-profiles/` (or XDG data-home equivalent; override with `TESHI_APP_DATA_DIR`).

## Crate rules

- `teshi-ui` / `teshi-web` must not depend on `teshi-engine` or `teshi-agent`.
- Desktop may use `teshi-engine` only for the native config store backend.
