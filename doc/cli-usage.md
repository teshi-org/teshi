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

Same React UI as desktop, served over loopback HTTP (no Tauri install required):

```bash
teshi web [--project PATH] [--port 1421] [--no-open] [--dist PATH]
```

On Windows, the full MSI and release zip bundle web assets under `share/web/` next to `teshi.exe`.
For development from source, build the frontend first: `cd desktop && npm run build`.

### Native desktop (`teshi desktop` / `teshi-desktop`)

Chrome extension locator workflow and embedded terminal:

```bash
teshi desktop [--project PATH]
teshi desktop path/to/project          # positional shortcut
teshi-desktop --project path/to/project
teshi-desktop path/to/project
```

After installing the full Windows MSI (`teshi-vX.Y.Z-x64.msi`) or release zip, `teshi-desktop.exe` is installed next to `teshi.exe`.

Development: `cargo tauri dev --manifest-path apps/teshi-tauri/Cargo.toml`.

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

For web UI self-test CI, start the embedded sidecar and replay bindings:

```bash
teshi browser serve-embedded --navigate http://127.0.0.1:1421
teshi run tests/feature/web-ui/welcome_smoke.feature
```

See [web-ui-self-test.md](web-ui-self-test.md) and `scripts/run-web-ui-smoke.sh`.

### Browser sidecar (`teshi browser`)

Commands for locator recording, replay, and sidecar health (see [browser-modes.md](browser-modes.md)):

```bash
teshi browser doctor              # TCP + snapshot probe; exit 1 if stale
teshi browser reconnect           # Restart embedded sidecar (refresh cdp-endpoint.json)
teshi browser snapshot            # Page accessibility tree
teshi browser navigate <url>      # Navigate active tab
teshi browser highlight <selector>
teshi browser execute --selector <css> --action <action> [--value-arg <text>]
teshi browser verify --step-line N --selector <css> --action <action> [--value-arg <text>]
teshi browser replay [--until-line N] [--non-interactive] [--yes]
teshi browser serve-embedded [--navigate <url>]
```

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

### Auth management

```bash
teshi auth login                    # interactive: choose provider + enter key
teshi auth login --provider openai  # specify provider
teshi auth list                     # show stored providers (keys masked)
teshi auth remove openai            # delete credentials
teshi auth status                   # show config paths and credential status
teshi auth migrate                  # import from env vars (TESHI_OPENAI_API_KEY, etc.)
```

---

## Configuration files

### Location

| Scope | Path |
|-------|------|
| Global | `~/.teshi/config.toml` |
| Project | `./.teshi/config.toml` |
| Runner | `./teshi.toml` (working directory) |

### Format (TOML)

```toml
# Default AI provider
default_provider = "deepseek"

# Provider definitions
[providers.deepseek]
base_url = "https://api.deepseek.com"
model = "deepseek-chat"
api_key = "${auth:deepseek}"   # resolves from ~/.teshi/auth.json

[providers.openai]
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key = "${auth:openai}"

# Custom provider (Ollama, etc.)
[providers.ollama]
base_url = "http://localhost:11434/v1"
model = "llama3"
api_key = "ollama"              # Ollama doesn't need a real key

# Runner configuration (teshi.toml)
[runner]
cmd = "teshi-runner"
args = ["--bin", "runner"]
cwd = "."

# LLM settings
[llm]
max_tokens = 4096
temperature = 0.7
```

### Placeholders

- `${auth:provider}` — loads API key from `~/.teshi/auth.json`
- `${env:VAR}` — loads from environment variable

API keys should **never** be written directly in config files. Use `teshi auth login` or `${env:VAR}` instead.

---

## Environment variables

| Variable | Default | Effect |
|----------|---------|--------|
| `TESHI_DEFAULT_PROVIDER` | — | Override default LLM provider |
| `TESHI_LLM_API_KEY` | — | LLM API key |
| `TESHI_LLM_BASE_URL` | `https://api.openai.com/v1` | LLM API base URL |
| `TESHI_LLM_MODEL` | `gpt-4o-mini` | LLM model name |
| `TESHI_LLM_MAX_TOKENS` | `1024` | Max tokens per completion |
| `TESHI_LLM_TEMPERATURE` | `0.7` | Sampling temperature |
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

## Auth storage

Credentials are stored in `~/.teshi/auth.json` with `0600` permissions:

```json
{
  "deepseek": {
    "api_key": "sk-abc123...",
    "added_at": "2026-01-15T10:30:00Z"
  },
  "openai": {
    "api_key": "sk-xyz789...",
    "added_at": "2026-02-20T14:00:00Z"
  }
}
```

- Atomic writes via temp file + rename
- Warning displayed if file permissions are not `0600`
- `teshi auth list` masks keys: shows first 4 + last 4 characters only
