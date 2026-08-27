## REMOVED Requirements

### Requirement: React web UI for daemon

**Reason**: The daemon web product now uses the shared GPUI WASM shell; retaining a second React implementation creates unsupported duplicate UI behavior.
**Migration**: Build `apps/teshi-web` with `scripts/build-teshi-web.sh` (or the PowerShell equivalent) and serve `apps/teshi-web/dist`.

## ADDED Requirements

### Requirement: GPUI WASM web UI for daemon

The GPUI WASM web UI SHALL reside at `apps/teshi-web/`, share product views through `crates/teshi-ui`, and SHALL be served by `teshi-daemon` as static assets. The retired TypeScript/React application directory `apps/teshi-web-ui/` SHALL NOT exist. There SHALL NOT be a Tauri-based desktop shell in the workspace.

#### Scenario: GPUI web package exists

- **WHEN** the target directory structure is checked
- **THEN** `apps/teshi-web/Cargo.toml` and `apps/teshi-web/web/index.html` exist

#### Scenario: React web package is absent

- **WHEN** the target directory structure is checked
- **THEN** `apps/teshi-web-ui` does NOT exist

#### Scenario: No Tauri app crate

- **WHEN** the root `Cargo.toml` workspace `members` list is read
- **THEN** it does NOT include `apps/teshi-tauri`
