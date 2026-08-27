# GPUI WASM web shell

Supported web product path: shared GPUI `teshi-ui`, native `teshi-desktop`, WASM `teshi-web`, and `teshi-daemon` same-origin hosting.

## Scope

- **In:** browser-profile discovery and explicit selection, LLM configuration, daemon same-origin APIs, and `--dist` serving GPUI assets.
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

## Path 1 run (daemon hosts GPUI)

The daemon resolves the GPUI dist by default; `--dist` is useful for an explicit build path:

```powershell
cargo run -p teshi-cli -- web --no-open --dist "apps/teshi-web/dist"
```

Open the printed `http://127.0.0.1:<port>/` URL. The initial surface is Browser Profiles; Settings opens the shared LLM profile form.

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
- Desktop may use `teshi-engine` and direct loopback adapters for native platform I/O.
