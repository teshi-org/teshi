## ADDED Requirements

### Requirement: GPUI shell crates in the workspace

The workspace SHALL include `crates/teshi-ui` (shared GPUI views), `apps/teshi-desktop` (native GPUI entry), and `apps/teshi-web` (GPUI WASM entry) as first-class members alongside existing shells. Dependency direction remains: shells may depend on engine/agent/core as appropriate for their target; shared UI must stay platform-capability free.

#### Scenario: Members exist

- **WHEN** the root `Cargo.toml` workspace `members` list is read
- **THEN** it includes `crates/teshi-ui`, `apps/teshi-desktop`, and `apps/teshi-web`

### Requirement: Shared GPUI UI has no engine or agent dependency

`teshi-ui` SHALL depend on GPUI (and pure/shared types as needed) but MUST NOT depend on `teshi-engine` or `teshi-agent`. Platform I/O for the UI SHALL go through a backend abstraction implemented by the entry apps or daemon HTTP client code.

#### Scenario: No engine edge from teshi-ui

- **WHEN** `cargo tree -p teshi-ui --depth 1` is run
- **THEN** `teshi-engine` and `teshi-agent` do not appear

### Requirement: WASM entry has no engine or agent dependency

`apps/teshi-web` SHALL NOT depend on `teshi-engine` or `teshi-agent`. It MAY depend on `teshi-ui`, GPUI web platform crates, and WASM bindgen-related crates.

#### Scenario: Web package tree excludes engine

- **WHEN** `cargo tree -p teshi-web --depth 1` is run for the wasm-oriented package
- **THEN** `teshi-engine` and `teshi-agent` do not appear

### Requirement: Workspace pins GPUI for shells

The root `[workspace.dependencies]` SHALL declare the pinned `gpui` (and `gpui_platform` when required) used by GPUI shells so desktop and web share one revision.

#### Scenario: Workspace lists gpui

- **WHEN** the root `Cargo.toml` `[workspace.dependencies]` section is read
- **THEN** it contains a `gpui` entry referenced by `teshi-desktop` and `teshi-web` via `workspace = true`
