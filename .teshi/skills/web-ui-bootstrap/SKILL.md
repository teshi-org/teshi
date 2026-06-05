---
name: web-ui-bootstrap
description: Start teshi web as the SUT and record or replay bindings from teshi desktop (self-bootstrap)
---

# Web UI Bootstrap Skill

Use when testing **teshi's own web UI** from **teshi desktop** (dogfood / self-bootstrap).

## Two browser contexts

| Context | What it is |
|---------|------------|
| **Host IDE** | teshi desktop window (Gherkin, Locator, terminal, Browser panel chrome) |
| **SUT** | `teshi web` / Vite dev server — page inside **Start Embedded** preview |

The SUT is **not** the desktop webview. Start SUT separately and navigate Embedded preview to the SUT URL.

## Prerequisites

1. Project open in teshi desktop (`teshi desktop --project .`).
2. Python venv with Playwright: `pip install -r python/requirements.txt && python -m playwright install chromium`.
3. Compatible CLI: `TESHI_CLI` pointing to teshi >= 0.4.0 when using an external terminal.

## Dev SUT (HMR — recommended while editing React)

| Terminal | Command | Port |
|----------|---------|------|
| 1 API | `teshi web --project . --port 1421 --no-open` | 1421 |
| 2 UI | `cd desktop && npm run dev` | 1420 |

Connect Embedded preview to **`http://127.0.0.1:1420/?e2e=1`** (full URL including `http://`).

Rebuild not required for UI changes; restart terminal 2 if `vite.config.ts` changes.

## Stable SUT (CI / smoke)

```bash
cd desktop && npm run build
teshi web --project . --port 1421 --no-open
```

Embedded preview: `http://127.0.0.1:1421`

## Sidecar health (before snapshot / propose / replay)

```bash
TESHI=${TESHI_CLI:-teshi}
$TESHI browser doctor
# if fail:
$TESHI browser reconnect
$TESHI browser doctor
```

**SUT restart playbook:** after killing/restarting `teshi web` or `npm run dev`, run doctor → reconnect before any browser CLI command.

## Connect Browser panel

1. **Start Embedded**
2. Address bar → SUT URL (`http://127.0.0.1:1420/?e2e=1` or `:1421`)
3. Confirm `.teshi/cdp-endpoint.json` has `"mode": "embedded"`

Then follow **bdd-locator** / **bdd-replay** or the full **agent-web-ui-flow**.

## Selectors (SUT page)

Prefer `[data-testid="..."]`:

| testid | Panel |
|--------|-------|
| `WelcomeOpenProjectButton` | Welcome |
| `WelcomeRecent-{sanitizedPath}` | Welcome recent list |
| `GherkinPanel`, `GherkinStep-{line}` | Gherkin |
| `FileTreeTab`, `TerminalTab`, `FileTreeNode-{relativePath}` | Files / terminal |
| `LocatorConfirm`, `LocatorReject` | Locator |
| `BrowserStartEmbedded`, `BrowserAddressBar`, `BrowserGo` | Host Browser chrome only |

## CI (no desktop UI)

```bash
cd desktop && npm run build
teshi web --port 1421 --no-open &
teshi browser serve-embedded --navigate http://127.0.0.1:1421 &
teshi run tests/feature/web-ui/welcome_smoke.feature
```

See [doc/web-ui-self-test.md](../../doc/web-ui-self-test.md) and **agent-web-ui-flow**.
