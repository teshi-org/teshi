# teshi web UI self-test (bootstrap)

teshi can dogfood its own GUI by treating **teshi desktop** as the test IDE and **teshi web** on loopback as the application under test (AUT). Both hosts load the same React bundle from `desktop/dist`.

## Two browser contexts

| Context | Role | URL / surface |
|---------|------|----------------|
| **Host (IDE)** | Recording shell: Gherkin, Locator, embedded terminal, Browser panel controls | teshi desktop (Tauri) or teshi web when used as IDE |
| **SUT (AUT)** | Page under test inside the Browser panel preview | `http://127.0.0.1:1420` (Vite dev) or `http://127.0.0.1:1421` (stable dist) |

Do not confuse them:

- **Start Embedded** in the Browser panel attaches Playwright to the **SUT** preview stream.
- `teshi web --no-open` in the terminal starts the **SUT API/runtime** on port 1421.

## One-command bootstrap (recommended)

From the repo root, a single command starts the full dev stack and opens a live status dashboard (version, health, duplicate-instance warnings):

```powershell
pip install -r scripts/requirements-dev.txt
python scripts/bootstrap_dev.py --project . --build
```

Shorthand:

```powershell
py scripts/bootstrap_dev.py .
```

The script builds **teshi** + **teshi-desktop**, starts **Vite** (`:1420`), **teshi-desktop**, **teshi web** (`:1421`), and **serve-embedded**. It avoids `npm run tauri dev` predev so Windows does not lock `teshi.exe`. Do **not** run `Stop-Process -Name teshi` before bootstrap; use `--stop-existing` only.

Logs: `.teshi/logs/bootstrap-*.log`

Flags: `--mode separate` (desktop exe + npm instead of tauri dev), `--no-embedded`, `--api-port`, `--ui-port`, `--stop-existing` (stop locked debug binaries before build/start).

See also [`.teshi/skills/web-ui-bootstrap/SKILL.md`](../.teshi/skills/web-ui-bootstrap/SKILL.md).

## Dev workflow (HMR — manual)

| Terminal | Command |
|----------|---------|
| 1 | `teshi web --project . --port 1421 --no-open` |
| 2 | `cd desktop && npm run dev` (Vite on 1420) |

1. Open the repo: `teshi desktop --project .`
2. Start both terminals above.
3. Browser panel: **Start Embedded** → navigate to `http://127.0.0.1:1420/?e2e=1` (include `http://` and `?e2e=1` for automation teardown).
4. Health check before record/replay:

```bash
teshi browser doctor || teshi browser reconnect
teshi browser doctor
```

5. Record bindings with **bdd-locator** (RVP verification) or follow **agent-web-ui-flow**.
6. Replay: `teshi browser replay --non-interactive --yes`

Use stable `[data-testid="..."]` selectors (see `apps/teshi-web-ui/src/panels/` and **web-ui-bootstrap** skill).

## Stable workflow (CI / smoke)

1. `cd desktop && npm run build`
2. `teshi web --project . --port 1421 --no-open`
3. Embedded preview: `http://127.0.0.1:1421`

## Sidecar lifecycle

After killing/restarting `teshi web` or Vite dev server:

```bash
teshi browser doctor
teshi browser reconnect   # embedded mode only
teshi browser doctor
```

Set `TESHI_BROWSER_AUTO_RECONNECT=0` to disable automatic reconnect before browser CLI commands.

## CI headless workflow

1. Build the frontend: `cd desktop && npm run build`
2. Start the web host: `teshi web --port 1421 --no-open` (add `--project .` for project-panel scenarios)
3. Start the embedded sidecar: `teshi browser serve-embedded --navigate http://127.0.0.1:1421`
4. Run scenarios: `teshi run tests/feature/web-ui/`

Ensure `.teshi/cdp-endpoint.json` exists (written by `serve-embedded`) and step-bindings are committed under `.teshi/step-bindings/`.

## Test assets

| Path | Purpose |
|------|---------|
| `tests/feature/web-ui/*.feature` | Gherkin scenarios for the web UI |
| `.teshi/step-bindings/tests__feature__web-ui__*.json` | Confirmed DOM bindings |
| `.teshi/skills/agent-web-ui-flow/SKILL.md` | End-to-end external agent playbook |
| `.teshi/skills/web-ui-bootstrap/SKILL.md` | Host + SUT setup |
| `.teshi/skills/bdd-feature-author/SKILL.md` | Write `.feature` files |
| `.teshi/skills/bdd-locator/SKILL.md` | RVP + `steps propose` |
| `.teshi/skills/bdd-replay/SKILL.md` | Replay validation |

## Out of scope

- Tauri shell–specific behavior (native file dialog, window chrome, `invoke` paths)
- WinApp UIA against `teshi-desktop.exe` (WinApp mode targets WinUI3 apps under test)

Web UI E2E covers the shared React panels used by both desktop and web hosts.
