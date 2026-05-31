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

- **Toolbar:** Open project, recent projects, browser controls; switches to a minimal bar in browser focus mode
- **Panel 1 (left):** Structured Gherkin render for selected `.feature` files
- **Panel 2 (center):** Playwright Chromium JPEG stream (1920×1080 viewport), started manually
- **Panel 3 (right):** Lazy file tree + terminal (tab switch)
- **Bottom dock:** Collapsible tab bar (Output, Logs) — placeholder content in Phase 1; collapsed by default
- **Browser focus:** Use **Focus** in the browser panel header to hide side panels and expand the browser; **Exit Focus** in the minimal toolbar restores the three-column layout

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
