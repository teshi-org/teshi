## Context

The TUI currently owns `InstalledAgent`, reads and atomically writes `dirs::data_local_dir()/teshi/acp/installed.json`, and uses the records for Registry-installed and custom ACP agents. The hidden `/acp add` command only accepts an ID plus a JSON argv array. The top-level CLI is parsed in `teshi-tui`, so a reusable store cannot remain private to that frontend.

Existing JSON records contain `id`, `name`, `program`, `args`, and a literal `env` map. They must continue to deserialize and launch unchanged. New custom entries need a way to refer to credentials without copying their values into the manifest.

## Goals / Non-Goals

**Goals:**

- Provide discoverable `teshi acp` commands for scripting and manual management.
- Make the existing JSON file a version-tolerant, documented user configuration contract.
- Keep one store shared by CLI, TUI, and Registry installation.
- Resolve declared environment references only when constructing the child command.
- Preserve existing records and the current session-only selection behavior.

**Non-Goals:**

- Desktop or Web management UI.
- Persisting a default selected ACP agent.
- Installing arbitrary packages through the new CLI.
- Probing or authenticating an ACP agent as part of configuration writes.
- Storing credentials or expanding shell command strings.

## Decisions

### Put the shared profile store in `teshi-acp`

`teshi-acp` already owns command construction, executable resolution, Registry metadata, and the ACP process boundary. The persisted profile type and generic CRUD operations will move there; Registry package installation can remain in the TUI for now and call the shared upsert operation. This avoids making the CLI depend on frontend-private code.

An alternative was to expose `teshi-tui::acp_agents`, but that would preserve the wrong dependency direction and make other frontends depend on TUI internals.

### Extend the existing JSON records additively

The path remains `dirs::data_local_dir()/teshi/acp/installed.json`. Existing fields retain their meaning. A defaulted `env_from` map is added, mapping child variable names to host variable names. At launch, referenced values are read from the current process environment and merged over literal `env` values. A missing referenced variable fails closed with the agent ID and variable name but never prints a value.

The CLI accepts `--env NAME` as shorthand for `NAME=NAME` and `--env NAME=SOURCE` for renaming. It does not accept literal secret values. Direct file edits may retain the legacy literal `env` map for compatibility, but documentation directs users to `env_from`.

### Use structured CLI values

The command group is:

- `teshi acp list [--json]`
- `teshi acp show <id> [--json]`
- `teshi acp add <id> --program <PROGRAM> [--name <NAME>] [--arg <ARG>]... [--env <NAME[=SOURCE]>]...`
- `teshi acp remove <id> [--json]`
- `teshi acp config-path`

`add` is an upsert so the same command can repair or update an entry. Program and argument values remain separate; Teshi never asks a shell to parse a command string. Human output is concise, while JSON returns the persisted definition without resolving or exposing environment values.

### Validate the complete file before writes

Loads validate unique, safe IDs and non-empty programs. Mutations load and validate the current file, update one entry in memory, sort deterministically, and atomically replace the file. Invalid manual edits block subsequent use and mutation with a path-qualified error rather than silently dropping records.

## Risks / Trade-offs

- [Users may expect `--env KEY=value` to store a literal value] → Document and validate the option as a reference mapping, with `KEY=SOURCE_ENV` wording in help.
- [Moving the model can disturb Registry installation] → Preserve serialized field names and add focused compatibility and TUI launch tests.
- [A hand-edited file may be malformed] → Return precise validation errors and keep atomic writes; never rewrite a file that failed to load.
- [An executable available during `add` may later disappear] → Resolve and store the executable path at add time as today, then report normal launch errors if it is removed.

## Migration Plan

No eager migration is required. Existing arrays deserialize because `env_from` defaults to empty. The next successful mutation writes the same records with the additive empty field omitted. Rollback remains possible because older Teshi versions ignore no unknown fields only if their serde model permits them; therefore the new field is omitted when empty and users relying on `env_from` must remove those mappings before downgrading.

## Open Questions

None for the CLI/configuration slice. Connection testing and persistent defaults can be proposed separately after this surface is exercised.
