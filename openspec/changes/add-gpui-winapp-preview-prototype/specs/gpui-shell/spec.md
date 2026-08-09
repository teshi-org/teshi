## MODIFIED Requirements

### Requirement: Dual GPUI entry points

The workspace SHALL provide `apps/teshi-desktop` (native GPUI) and `apps/teshi-web` (GPUI WASM with `cdylib`/`rlib`) that both present the same root shell view from `teshi-ui`. The root shell SHALL show a main surface containing the shared WinApp preview by default and MUST provide access to a settings surface; it MUST NOT use the LLM configuration form as the sole root content.

#### Scenario: Desktop launches shared root

- **WHEN** `teshi-desktop` is started
- **THEN** it opens a native window rendering the `teshi-ui` root shell view and WinApp preview on the main surface

#### Scenario: Web entry exports WASM run

- **WHEN** `teshi-web` is built for `wasm32-unknown-unknown` and loaded in a browser page
- **THEN** it initializes GPUI via the web platform (`web_init` / `single_threaded_web` or equivalent) and renders the same `teshi-ui` root shell view and WinApp preview

#### Scenario: Default surface is main, not LLM config

- **WHEN** either entry point finishes initial render
- **THEN** the visible root content is the main surface with the preview, not the LLM configuration form

### Requirement: Empty main surface placeholder

The shared root shell SHALL replace its former empty main placeholder with the WinApp preview while keeping the main surface as the default landing surface after launch.

#### Scenario: Launch lands on preview

- **WHEN** the application window or WASM shell first becomes visible
- **THEN** the user sees the WinApp preview connection state on the main surface and can still navigate to settings
