# gpui-shell

## Purpose

Shared native/WASM GPUI application shell for Teshi: workspace GPUI pinning, dual entry points (`teshi-desktop` / `teshi-web`), and daemon-hosted static serving for same-origin Path 1 deployments.

## Requirements

### Requirement: Shared GPUI UI crate

The workspace SHALL provide a `teshi-ui` library crate that contains the shared GPUI views for the desktop and web shells. `teshi-ui` MUST NOT depend on `teshi-engine` or `teshi-agent`.

#### Scenario: UI crate is linkable without engine

- **WHEN** `cargo tree -p teshi-ui --depth 1` is inspected
- **THEN** neither `teshi-engine` nor `teshi-agent` appears as a direct dependency

### Requirement: Dual GPUI entry points

The workspace SHALL provide `apps/teshi-desktop` (native GPUI) and `apps/teshi-web` (GPUI WASM with `cdylib`/`rlib`) that both present the same root shell view from `teshi-ui`. The root shell SHALL show a main surface by default (which MAY be empty) and MUST provide access to a settings surface; it MUST NOT use the LLM configuration form as the sole root content.

#### Scenario: Desktop launches shared root

- **WHEN** `teshi-desktop` is started
- **THEN** it opens a native window rendering the `teshi-ui` root shell view on the main surface

#### Scenario: Web entry exports WASM run

- **WHEN** `teshi-web` is built for `wasm32-unknown-unknown` and loaded in a browser page
- **THEN** it initializes GPUI via the web platform (`web_init` / `single_threaded_web` or equivalent) and renders the same `teshi-ui` root shell view

#### Scenario: Default surface is main, not LLM config

- **WHEN** either entry point finishes initial render
- **THEN** the visible root content is the main surface (empty or placeholder), not the LLM configuration form

### Requirement: Empty main surface placeholder

The shared root shell SHALL present a main surface that MAY be empty or a minimal placeholder until product panels are added. The main surface MUST remain the default landing surface after launch.

#### Scenario: Launch lands on empty main

- **WHEN** the application window or WASM shell first becomes visible
- **THEN** the user sees the main surface without required product panels, and can still navigate to settings

### Requirement: Unified GPUI workspace pin

Desktop and web shells SHALL use the same GPUI (and web platform, where required) dependency versions declared in the workspace root.

#### Scenario: Single revision for both targets

- **WHEN** `apps/teshi-desktop/Cargo.toml` and `apps/teshi-web/Cargo.toml` declare GPUI dependencies
- **THEN** both resolve through `[workspace.dependencies]` to the same pinned source/revision

### Requirement: Daemon hosts GPUI web assets (Path 1)

`teshi-daemon` SHALL be able to serve the built GPUI web static assets as its UI `dist`, so the WASM application and `/api/v1` share the same origin.

#### Scenario: Same-origin load

- **WHEN** an operator starts the daemon with the GPUI web `dist` directory configured
- **THEN** requesting the daemon HTTP root (or configured app path) returns the GPUI web shell assets without requiring a separate origin

#### Scenario: API remains on same host

- **WHEN** the GPUI WASM shell runs from that daemon-hosted origin
- **THEN** it can call `/api/v1/*` on the same host and port without cross-origin browser restrictions for that deployment mode
