# Requirements shared across projects

The Markdown library remains at the user-level requirements root. Its existing
`store_id`, document IDs, paths, revisions, and test-point links are preserved.
Each document can reference multiple durable project IDs. Opening a project
does not claim existing documents or create an identity file.

## Project identity and membership

The first explicit association, legacy import, or new-document action creates
`.teshi/project.json` with a random UUID. Keep this file when moving the project.
Commit it for clones that represent the same logical project. If `.teshi/` is
ignored, use `git add -f .teshi/project.json`. Do not commit `project.lock`.
An independent fork should remove its copied identity file before linking any
requirements; its next association will create a new identity. Existing
associations of the original project remain in the shared store.

The TUI opens the Current project window when given a project path. Starting
`teshi` without a path opens All projects for library browsing while loading
Feature files from `./features` according to the system UI language. Feature
discovery does not move the project root into the selected language directory.
See [CLI startup rules](cli-usage.md#open-files-tui). In the requirement tree, `o` cycles
Current project, All projects, and No project. `b` toggles the selected saved
document's association with the current project. Unsaved edits must be resolved
before changing scope or membership. New documents belong to the current
project, including when created from another view.

Older metadata without `projects` means no project associations. Teshi never
guesses ownership from titles, paths, iterations, or test-point links. All
projects and No project keep these documents accessible. Iteration filtering
is independent: `--unassigned` means no iteration, whereas
`--unassigned-project` means no project.

```sh
teshi requirements list --json
teshi requirements list --project ../project-b --json
teshi requirements list --all-projects --json
teshi requirements list --unassigned-project --json
teshi requirements link-project doc-1 doc-2 --project . --json
teshi requirements unlink-project doc-1 --project . --json
teshi requirements set-project-status doc-2 --project . --status implemented --json
teshi requirements set-project-status doc-2 --project . --status verified --json
```

`link-project` is idempotent and preserves an existing state. `unlink-project`
removes only that project's association. The document and other projects remain
intact. Status changes require an existing association. Batch operations validate
all documents and commit under the requirement-store lock. Stale Markdown/index
saves cannot overwrite project associations.

The JSON `projects` map stores `planned`, `implemented`, or `verified` separately
for each project ID. These are explicit user records, not inferred test results;
recording `verified` does not run tests. A later requirement edit does not itself
revoke an earlier recorded status. Review and update status when sources change.
Test points and their review/scenario records remain project-local.

## Portable source snapshot

```sh
teshi requirements export-project --project . --output requirements-snapshot.json
```

The snapshot contains the project ID, original store/document identities, content
revisions, full Markdown, and this project's recorded states. It can be read as
ordinary JSON without the original user library and can be committed alongside
test points. Other projects' documents and associations are excluded. The command
refuses to overwrite an existing file; select a new filename for a later snapshot.
Snapshots are read-only evidence copies, not an automatically synchronized library
or an implicit import into another user's store.

## Generation scope

New generation scopes include the selected project window alongside store identity
and iteration. The confirmation dialog displays that window. Once confirmed,
scope is fixed for the session and agent reads validate current membership, so
unlinking a source makes it unavailable to that scoped session. Legacy saved
generation scopes retain their original all-projects meaning.

## Verification

- Core compatibility is exercised by engine tests that read old metadata.
- Engine tests cover moved projects, independent states, stale saves, atomic batch
  failures, malformed identities, and snapshots read after the source store closes.
- `cargo test -p teshi-cli --test requirement_projects --locked` exercises the
  actual CLI, including filters, failed writes, moves, unlinking, and export.
- The real terminal test is `cargo test --manifest-path
  crates/teshi-tui-runner/Cargo.toml --test requirement_projects -- --ignored` with
  `TESHI_SELF_TEST_BIN` pointing to the built CLI. Set
  `TESHI_REQUIREMENT_PROJECT_ARTIFACTS` to retain viewport and disk evidence.
