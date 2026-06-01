# teshi-desktop

Desktop shell for teshi BDD recorder and runner (Phase 1: panel layout and cross-panel linking).

## Prerequisites

- Rust stable (see repo `rust-toolchain.toml`)
- Node.js 18+ and npm
- Windows (primary dev target; code is structured for cross-platform)

## Project under test

The opened BDD project must have a Python virtual environment at `.venv` or `venv` with:

```bash
python -m venv .venv
.venv\Scripts\pip install -r python/requirements.txt
.venv\Scripts\python -m playwright install chromium
```

For **Connect Chrome**, the venv must include `websockets` (included in `python/requirements.txt`).

**uv-managed venvs** (`uv venv` / `uv sync`, `pyvenv.cfg` contains `uv = ...`):

```bash
uv pip install websockets
# Embedded mode also needs Playwright:
uv pip install -r python/requirements.txt
```

teshi runs Connect Chrome using the base Python from `pyvenv.cfg` `home` plus `PYTHONPATH` to `.venv\Lib\site-packages` (it does not execute the uv trampoline in `.venv\Scripts\python.exe`). Run `uv python install` if that base interpreter is missing. Use an external terminal for `uv pip` if the embedded shell reports os error 448.

(`python/requirements.txt` lives at the repo root as a reference manifest.)

## Development

From this directory:

```bash
npm install
npm run tauri dev
```

Or from the repo root:

```bash
cargo tauri dev --manifest-path desktop/src-tauri/Cargo.toml
```

When `target/debug/teshi.exe` sits next to `teshi-desktop.exe` (typical `cargo tauri dev` layout), the embedded terminal sets `TESHI_CLI` to that binary so agent skills use your dev build instead of an older PATH install. `npm run dev` (via Tauri's `beforeDevCommand`) runs `predev` first to rebuild the CLI so its version matches the repo. Override with `TESHI_CLI` in the environment before launching desktop.

If `TESHI_CLI --version` still looks stale, restart desktop (so the terminal respawns) and confirm:

```powershell
echo $env:TESHI_CLI
& $env:TESHI_CLI --version
```

## Web UI (`teshi web`)

Build the frontend, then start the loopback server from the repo root:

```bash
cd desktop && npm run build && cd ..
cargo run -- web --project C:\path\to\bdd-project
```

Optional flags: `--port 1421`, `--no-open`, `--dist path/to/desktop/dist`.

The browser UI uses the same React app as the Tauri shell; open a project via **File > Open Project** or `--project`.

Equivalent via main CLI wrapper:

```bash
teshi web --project C:\path\to\bdd-project
```

## Layout

- **App chrome:** **File > Open Project…** (`Ctrl+O`) and **Open Recent** in the custom title bar (frameless window on desktop)
- **Panel 1 (left):** Structured Gherkin render for selected `.feature` files
- **Panel 2 (center):** **Connect Chrome** (logged-in tab via unpacked extension from `C:\Program Files\teshi\share\teshi-bridge` or repo `extension/teshi-bridge`) or **Start Embedded** (headless Playwright JPEG stream, 1920×1080); **Disconnect** when connected
- **Panel 3 (right):** Lazy file tree + terminal (tab switch)
- **Bottom dock:** Locator confirmation (linked to selected Gherkin step), plus Output/Logs placeholders; expanded by default on Locator tab
- **Browser fullscreen:** Click the fullscreen icon in the browser panel header to hide side panels and bottom dock; click the exit-fullscreen icon or press **Escape** to restore the layout

## BDD locator workflow

1. Open a project and select a `.feature` file.
2. Click a Gherkin **step** in the left panel (writes `.teshi/active-step.json`).
3. **Connect Chrome** or **Start Embedded** (writes `.teshi/cdp-endpoint.json` with `mode` and `ws_url`). For Chrome, load the unpacked extension first from `C:\Program Files\teshi\share\teshi-bridge` (or repo path for development) — see `extension/teshi-bridge/README.md`.
4. In the embedded terminal, run a Cursor agent and invoke the **bdd-locator** skill (`.teshi/skills/bdd-locator/SKILL.md`).
5. The agent writes `.teshi/pending-locator.json` and highlights the target element via CDP overlay.
6. Confirm or reject the proposal in the **Locator** bottom panel; accepted locators are saved to `{feature}.locators.md`.

Runtime context files under `.teshi/` (except tracked skills) are gitignored local state.

## CLI

```bash
teshi-desktop [--project PATH]
teshi-desktop PATH                    # positional shortcut for --project
teshi desktop [--project PATH]        # spawn via main teshi binary
```

Single-instance: a second launch focuses the existing window and opens the given project path.

## App data

- `%APPDATA%\teshi-desktop\recent.json` — recent projects (max 10)
- `%APPDATA%\teshi-desktop\settings.json` — window size and dialog defaults
- `%APPDATA%\teshi-desktop\logs\` — application logs
- Browser **localStorage** key `teshi.workspaceLayouts.v1` — per-project three-column panel sizes and side-panel collapse state (not in `settings.json`)
