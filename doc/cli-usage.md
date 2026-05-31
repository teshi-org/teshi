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
The separate `teshi-desktop-vX.Y.Z-x64.msi` installs only the Tauri shell.

Development: `cargo tauri dev --manifest-path desktop/src-tauri/Cargo.toml`.

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

| Variable | Effect |
|----------|--------|
| `TESHI_DEFAULT_PROVIDER` | Override default LLM provider |
| `TESHI_LLM_API_KEY` | LLM API key |
| `TESHI_LLM_BASE_URL` | LLM API base URL |
| `TESHI_LLM_MODEL` | LLM model name |
| `TESHI_RUNNER_CMD` | Override runner command (after `teshi.toml`) |
| `TESHI_RUNNER_ARGS` | Override runner args (space-separated) |
| `TESHI_RUNNER_CWD` | Override runner working directory |
| `TESHI_OPENAI_API_KEY` | Legacy (migrated by `teshi auth migrate`) |

Precedence for runner settings: `teshi.toml` → environment variables → CLI flags.

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
