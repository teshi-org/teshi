# CLI & Configuration

## Commands

### Open files

```bash
teshi                              # empty buffer
teshi path/to/features/            # open directory of .feature files
teshi path/to/file.feature         # open single .feature file
```

### Run tests

```bash
teshi run path/to/file.feature                    # run all scenarios
teshi run --feature "Login" file.feature           # filter by feature name
teshi run --scenario "Successful login" file.feature # filter by scenario name
teshi run --runner-cmd "behat" --runner-cwd /app  # override runner
```

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

### Format (TOML)

```toml
# Default AI provider
default_provider = "deepseek"

# Provider definitions
[providers.deepseek]
base_url = "https://api.deepseek.com"
model = "deepseek-chat"
api_key = "${auth:deepseek}"   # resolves from ~/.config/teshi/auth.json

[providers.openai]
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key = "${auth:openai}"

# Custom provider (Ollama, etc.)
[providers.ollama]
base_url = "http://localhost:11434/v1"
model = "llama3"
api_key = "ollama"              # Ollama doesn't need a real key

# Runner configuration
[runner]
command = "teshi-runner"
cwd = "."

# LLM settings
[llm]
max_tokens = 4096
temperature = 0.7
```

### Placeholders

- `${auth:provider}` — loads API key from `~/.config/teshi/auth.json`
- `${env:VAR}` — loads from environment variable

API keys should **never** be written directly in `teshi.toml`. Use `teshi auth login` or `${env:VAR}` instead.

---

## Environment variables

| Variable | Effect |
|----------|--------|
| `TESHI_DEFAULT_PROVIDER` | Override default LLM provider |
| `TESHI_LLM_API_KEY` | LLM API key |
| `TESHI_LLM_BASE_URL` | LLM API base URL |
| `TESHI_LLM_MODEL` | LLM model name |
| `TESHI_RUNNER_CMD` | Default test runner command |
| `TESHI_RUNNER_CWD` | Default test runner working directory |
| `TESHI_OPENAI_API_KEY` | Legacy (migrated by `teshi auth migrate`) |

---

## Auth storage

Credentials are stored in `~/.config/teshi/auth.json` with `0600` permissions:

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
