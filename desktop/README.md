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

## Layout

- **File menu (native):** **File > Open Project…** (`Ctrl+O`) and **File > Open Recent** with dynamic recent projects
- **Panel 1 (left):** Structured Gherkin render for selected `.feature` files
- **Panel 2 (center):** Playwright Chromium JPEG stream (1920×1080 viewport); **Start Browser** / **Stop Browser** in the panel header
- **Panel 3 (right):** Lazy file tree + terminal (tab switch)
- **Bottom dock:** Locator confirmation (linked to selected Gherkin step), plus Output/Logs placeholders; expanded by default on Locator tab
- **Browser fullscreen:** Click the fullscreen icon in the browser panel header to hide side panels and bottom dock; click the exit-fullscreen icon or press **Escape** to restore the layout

## BDD locator workflow

1. Open a project and select a `.feature` file.
2. Click a Gherkin **step** in the left panel (writes `.teshi/active-step.json`).
3. **Start Browser** (writes `.teshi/cdp-endpoint.json` with CDP endpoint).
4. In the embedded terminal, run a Cursor agent and invoke the **bdd-locator** skill (`.teshi/skills/bdd-locator/SKILL.md`).
5. The agent writes `.teshi/pending-locator.json` and highlights the target element via CDP overlay.
6. Confirm or reject the proposal in the **Locator** bottom panel; accepted locators are saved to `{feature}.locators.md`.

Runtime context files under `.teshi/` (except tracked skills) are gitignored local state.

## CLI

```bash
teshi-desktop
teshi-desktop --project C:\path\to\project
```

Single-instance: a second launch focuses the existing window and can pass `--project`.

## App data

- `%APPDATA%\teshi-desktop\recent.json` — recent projects (max 10)
- `%APPDATA%\teshi-desktop\settings.json` — window size and dialog defaults
- `%APPDATA%\teshi-desktop\logs\` — application logs
