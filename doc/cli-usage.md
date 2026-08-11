# CLI & Configuration

## Commands

### Open files (TUI)

```bash
teshi                              # scan current directory for .feature files
teshi path/to/features/            # open directory of .feature files
teshi path/to/file.feature         # open single .feature file
```

If no `.feature` files are found, the TUI opens an empty project buffer.

### Browser GUI (`teshi web`)

GPUI WASM workspace UI served over loopback HTTP by the local daemon:

```bash
teshi web [--project PATH] [--port 20253] [--no-open] [--dist PATH]
```

On Windows, the full MSI and release zip bundle web assets under `share/web/` next to `teshi.exe`.
For development from source, build the frontend first:

```bash
bash scripts/build-teshi-web.sh
teshi web --dist apps/teshi-web/dist
```

### Native desktop (`teshi desktop` / `teshi-desktop`)

GPUI desktop shell sharing its browser-session and settings views with GPUI Web:

```bash
teshi desktop [--project PATH]
teshi desktop path/to/project          # positional shortcut
teshi-desktop --project path/to/project
teshi-desktop path/to/project
```

Development: `cargo run -p teshi-desktop`.

### Run tests (headless)

For CI and scripts; streams NDJSON runner events to stdout:

```bash
teshi run                              # all scenarios under current directory
teshi run path/to/file.feature         # all scenarios in one file
teshi run path/to/project/             # all scenarios in a directory tree
teshi run --scenario "Successful login" path/to/file.feature
teshi run --runner-cmd "behat" --runner-cwd /app path/
```

Configure the runner in `teshi.toml` (see below). CLI flags override file and env settings.

For the supported GPUI WASM web UI smoke gate:

```bash
bash scripts/run-web-ui-smoke.sh
```

See [web-ui-self-test.md](web-ui-self-test.md) and `scripts/run-web-ui-smoke.sh`.

### Browser sidecar (`teshi browser`)

Commands for locator recording, replay, and sidecar health (see [browser-modes.md](browser-modes.md)):

```bash
teshi browser doctor              # TCP + snapshot probe; exit 1 if stale
teshi browser reconnect           # Restart embedded sidecar (refresh cdp-endpoint.json)
teshi browser sessions            # Versioned browser-profile discovery JSON
teshi browser tabs --session <id> # Windows/tabs scoped to one profile
teshi browser lease acquire --session <id> --owner <label> [--ttl 60]
teshi browser lease renew --session <id> --lease-token <token> [--ttl 60]
teshi browser lease release --session <id> --lease-token <token>
teshi browser snapshot            # Page accessibility tree
teshi browser navigate <url>      # Navigate active tab
teshi browser highlight <selector>
teshi browser execute --selector <css> --action <action> [--value-arg <text>]
teshi browser verify --step-line N --selector <css> --action <action> [--value-arg <text>]
teshi browser replay [--until-line N] [--non-interactive] [--yes]
teshi browser serve-embedded [--navigate <url>]
teshi browser locator --session <id> --window <id> --tab <id> --lease-token <token> \
  [--purpose <text>] [--role <role>] [--text <text>] [--element-ref <ref>] \
  [--gherkin-step <text>] [--test-id-attribute <name>]
teshi browser locator-verify --session <id> --window <id> --tab <id> \
  --lease-token <token> --page-revision <revision> --candidate-json '<json>'
teshi browser evidence --session <id> --window <id> --tab <id> \
  --lease-token <token> --page-revision <revision>
```

Explicit agent operations require the composite session/window/tab target and an exclusive profile lease. Existing commands may omit them only when one eligible target exists. When several profiles are live, omission fails with `ambiguous_browser_target` and performs no mutation. Machine-readable failures contain a stable `code`, actionable `error`, and non-sensitive `recovery` object and exit non-zero.

Project locator configuration lives in `.teshi/settings.json`; `playwright_test_id_attributes` defaults to `["data-testid"]` and may contain project-specific alternatives.

### Local MCP browser-agent server

```bash
teshi mcp serve --stdio [--project PATH]
```

The newline-delimited JSON-RPC server exposes the same discovery, lease, snapshot, locator, verification, and evidence operations as the CLI. It supports current `server/discover` negotiation and legacy `initialize`, writes protocol messages only to stdout, and operates only against the same-host Teshi broker. The release package includes `.mcp.json` metadata under `share/teshi-browser-testing`.

**Actions** for `execute`, `verify`, and `steps propose --action`:

| Action | Description |
|--------|-------------|
| `click` | Click element |
| `fill` | Fill input (no Enter) |
| `type` | Fill + Enter (xterm terminal) |
| `assert_visible` | Element must be visible |
| `assert_text` | Element text must match `--value-arg` |
| `select` | Select option |
| `press_key` | Press key (e.g. `Enter`) |
| `navigate` | Open URL (`--value-arg` or selector as URL) |
| `open_project` | POST `/api/v1/projects/open` with `--value-arg` absolute path |

Environment:

| Variable | Default | Effect |
|----------|---------|--------|
| `TESHI_BROWSER_AUTO_RECONNECT` | on | Auto reconnect embedded sidecar before browser commands when doctor fails |
| `TESHI_LOCATOR_STRICT` | off | Require prior `browser verify` log before `steps propose` |

---

### LLM profiles (`teshi auth`)

TUI, CLI, Desktop, and the daemon share one model-profile store under the Teshi app data directory:

| Platform | Default path |
|----------|----------------|
| Windows | `%APPDATA%\teshi\model-profiles\` |
| Linux / macOS | `$XDG_DATA_HOME/teshi/model-profiles` (often `~/.local/share/teshi/model-profiles`) |

Override the root with `TESHI_APP_DATA_DIR`. Legacy `%APPDATA%\teshi-desktop` data and older TUI `config.toml` / `auth.json` / `models/*.toml` are imported once automatically.

```bash
teshi auth login                    # interactive: provider + key → profile (activated)
teshi auth login --provider openai  # specify built-in provider id
teshi auth list                     # list profiles (keys masked)
teshi auth remove openai            # clear API key on matching provider profile(s)
teshi auth status                   # show app-data paths and profile status
teshi auth migrate                  # import keys from TESHI_* / OPENAI_* env vars into profiles
```

Built-in provider ids: `openai`, `anthropic`, `deepseek-openai`. For Ollama or other OpenAI-compatible servers, use `openai` with a custom base URL (the login flow offers an Ollama shortcut).

In the TUI, press `m` to open the model panel (same store). `/auth` shows a read-only overview.

---

## Configuration files

### Location

| Scope | Path |
|-------|------|
| LLM profiles (shared) | `<app_data>/teshi/model-profiles/*.json` + `active` pointer |
| Global (legacy / non-LLM) | `<config_dir>/teshi/config.toml` |
| Project | `./.teshi/config.toml` |
| Runner | `./teshi.toml` (working directory) |

### Runner format (`teshi.toml`)

```toml
[runner]
cmd = "teshi-runner"
args = ["--bin", "runner"]
cwd = "."
```

Older `[providers.*]` blocks and `${auth:…}` placeholders in `config.toml` are no longer the runtime LLM source of truth; they are only imported once into model profiles when the shared store is empty.

---

## Environment variables

| Variable | Default | Effect |
|----------|---------|--------|
| `TESHI_APP_DATA_DIR` | OS data dir + `teshi` | Override shared app-data root (profiles, settings, recent) |
| `TESHI_LLM_API_KEY` | — | Fallback LLM API key when no active profile has a key |
| `TESHI_LLM_BASE_URL` | `https://api.openai.com/v1` | Fallback LLM API base URL |
| `TESHI_LLM_MODEL` | `gpt-4o-mini` | Fallback LLM model name |
| `TESHI_LLM_MAX_TOKENS` | `1024` | Max tokens per completion (env fallback) |
| `TESHI_LLM_TEMPERATURE` | `0.7` | Sampling temperature (env fallback) |
| `TESHI_RUNNER_CMD` | — | Override runner command |
| `TESHI_RUNNER_ARGS` | — | Override runner args (space-separated) |
| `TESHI_RUNNER_CWD` | current dir | Override runner working directory |
| `TESHI_CLI` | — | Absolute path to `teshi` binary (Desktop embedded terminal sets this to the dev build) |
| `TESHI_DIAG_PATH` | — | Write diagnostic log to this file path |
| `TESHI_NO_RAW` | — | Disable raw terminal mode |
| `TESHI_NO_ALT` | — | Disable alternate screen |
| `TESHI_OPENAI_API_KEY` | — | Legacy (migrated by `teshi auth migrate`) |

Precedence for runner settings: `teshi.toml` → environment variables → CLI flags.

---

## Step bindings (`teshi steps`) — 0.4.0+

Manage Gherkin step locators under `.teshi/step-bindings/` and `.teshi/active-step.json`.

```bash
teshi steps list --feature test/feature/login.feature
teshi steps unbound --feature test/feature/login.feature
teshi steps select --feature test/feature/login.feature --line 12
teshi steps next-unbound --feature test/feature/login.feature   # JSON output
teshi steps propose --strategy uia --value 'uia:automation_id=Btn' --action click \
  --confidence 0.9 --rationale '...' [--line 12] [--highlight-applied]
teshi steps wait --until confirmed --timeout 60 --auto-confirm
teshi steps confirm | teshi steps reject
teshi steps unbind --feature test/feature/login.feature --line 12
teshi steps resolve --feature test/feature/login.feature [--until-line N]
```

- `--line` on `propose` must match `.teshi/active-step.json` or the command exits 1.
- `--auto-confirm` on `wait`: after timeout, confirms pending locator (default timeout 60s). On step mismatch, auto-rejects and exits 2.
- Project setting `.teshi/settings.json`: `{ "locator_auto_confirm_sec": 60 }` (`0` = manual only).

CLI `select` / `next-unbound` updates `active-step.json`; Desktop watches the file and syncs Gherkin highlight.

---

## WinUI automation (`teshi winapp`) — 0.4.0+

Requires **Connect WinUI3 App** in Desktop (or `teshi web`) and `.teshi/cdp-endpoint.json` with `"mode": "winapp"`.

```bash
teshi winapp list-windows
teshi winapp attach --hwnd 12345
teshi winapp attach --title 'My App'
teshi winapp attach --process-name MyApp.exe
teshi winapp snapshot
teshi winapp highlight 'uia:automation_id=LoginButton'
teshi winapp execute --selector 'uia:automation_id=LoginButton' --action click
teshi winapp replay --feature test/feature/login.feature [--until-line N] [--yes] [--dry-run] \
  [--launch 'C:\path\to\App.exe']
```

`replay` checks that a window is attached before running bindings. Use `attach` or `--launch` when detached.

---

## Export (`teshi export`) — 0.4.0+

Generate standalone test projects from confirmed bindings:

```bash
teshi export --target behave --feature test/feature/login.feature --out ./tests-e2e
```

Writes `behave.ini`, `features/`, `features/environment.py`, `features/steps/`, and `pages/`. Run `behave` from the output directory.

---

## Desktop embedded terminal

When using teshi Desktop, the embedded terminal sets `TESHI_CLI` to the bundled or dev `teshi` binary so agents do not pick up an older MSI on `PATH`. External shells must set `TESHI_CLI` explicitly or ensure `teshi --version` is >= 0.4.0.

See [desktop/README.md](../desktop/README.md) for development setup.

---

## LLM profile storage

Each profile is a JSON file under `<app_data>/teshi/model-profiles/{id}.json`. The active profile id is stored in `model-profiles/active`.

Typical fields: `id`, `name`, `provider`, `api_style`, `model_id`, `base_url`, `api_key`, `max_output_tokens`, `stream`, `http_headers`, `chat_options`.

- Desktop Settings, TUI model panel (`m`), `teshi auth`, and daemon `/api/v1/llm/profiles` all read/write this store.
- Public listings mask API keys; empty key on save preserves the previously stored key.
- When no active profile has a key, runtime falls back to `TESHI_LLM_*`.
