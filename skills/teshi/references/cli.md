# CLI workflows

## Executable and capabilities

PowerShell:

```powershell
$teshiExe = if ($env:TESHI_CLI) { $env:TESHI_CLI } else { 'teshi' }
& $teshiExe --version
& $teshiExe --help
& $teshiExe requirements --help
```

Bash:

```bash
teshi_bin="${TESHI_CLI:-teshi}"
"$teshi_bin" --version
"$teshi_bin" --help
"$teshi_bin" requirements --help
```

Examples below use `teshi` as shorthand for that resolved executable. If a command is missing, report the capability mismatch; do not silently switch binaries or edit internal storage to emulate it. For a requested source-development run, build `cargo build -p teshi-cli --locked` and use the resulting binary explicitly.

## Requirement library

Discover the store and stable document IDs before editing:

```text
teshi requirements path
teshi requirements list --json
teshi requirements list --iteration "Sprint 12" --json
teshi requirements show doc-12 --json
teshi requirements edit doc-12 --file body.md
teshi requirements set-iteration doc-12 doc-13 --iteration "Sprint 12"
teshi requirements clear-iteration doc-12
```

These commands depend on the installed version; check help first. Do not infer create/delete/search commands from the existence of list/edit.

- Store resolution: `--requirements-root PATH`, then non-empty `TESHI_REQUIREMENTS_DIR`, then the platform app-data requirements directory. The store is shared across projects; test points stay project-local in `testpoints/testpoints.json`.
- References resolve by ID, store-relative path, then unique exact title. Prefer IDs from list output. Ambiguous titles fail with candidates; do not choose one arbitrarily.
- `list --json` includes `store_id`, `store_path`, and `documents`; `show --json` includes metadata and `body`. Use `--json` only where help advertises it, and check process exit status as well as output.
- `edit --file` supplies the complete Markdown body. Non-interactive input may also use stdin; avoid opening an editor from an agent process. Write through CLI instead of editing `_teshi.json` or requirement files directly.
- On a revision conflict, re-read and reconcile the requested change. Do not automatically retry with `--force`. Read back the resulting body or iteration to verify the write.
- Legacy migration is a separate task: `requirements import-project --dry-run` previews it; `--yes` performs it when authorized. Routine reading/editing does not require migration.

## Headless BDD

```text
teshi run path/to/project
teshi run path/to/file.feature --scenario "Successful login"
teshi run path/to/project --runner-cmd behave --runner-cwd path/to/project
teshi steps list --feature path/to/file.feature
teshi steps unbound --feature path/to/file.feature
```

`run` streams NDJSON events. Inspect scenario/step outcomes and process status; do not treat the first event as completion. Explicit runner configuration (CLI or `teshi.toml [runner]`) takes precedence over live-daemon and Python-engine discovery. A feature file alone does not provide executable step implementations.

For bindings, follow [Gherkin bindings and replay](bindings.md): select the step, inspect/verify the requested target, propose the binding, honor the applicable confirmation mode, then replay and inspect results.

## Browser and native Windows targets

Browser workflow: `browser sessions` → `browser tabs --session <id>` → acquire a lease → inspect/verify → execute the requested action → verify the result → release the lease. Carry the session/window/tab identity and lease token through calls; labels are not durable identity. Revision-bound element references must be refreshed after page changes. Use the browser specialist for exact targeting and operation-specific grants.

WinApp workflow: inspect `winapp --help`, discover windows, attach the intended target, then inspect/verify/replay through the native specialist. A WinApp screenshot panel does not establish a browser target. Do not select an unrelated window just because it is available.

## Other command families

Read the family's help when the task needs it:

| Family | Purpose and completion evidence |
|--------|---------------------------------|
| `api` | HTTP BDD sidecar; `doctor` checks health and discovery, `exchange <id>` inspects redacted results. Request plaintext only when the task needs sensitive details. |
| `daemon` | Project service lifecycle; inspect available status/start/stop commands before changing the running service. |
| `terminal` | Terminal sidecar control; target the requested terminal and inspect output/exit state. |
| `export` | Export confirmed bindings; inspect generated artifacts and report whether they were actually executed. |
| `record`, `generate`, `trace` | Recording, artifact generation, and trace inspection; discover supported subcommands and output paths. |
| `mcp` | Expose local integrations when the user requests MCP setup. Existing CLI tasks need no transport migration. |
| `auth` | Credential management; avoid printing credential values into task reports. |

In a source checkout, `doc/cli-usage.md` provides extended examples and `teshi.toml` configuration. Prefer the running executable's help when its interface differs.
