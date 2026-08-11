## MODIFIED Requirements

### Requirement: Native backend shares the same store

The native GPUI desktop shell SHALL load and save model profiles against the same user-level Teshi app-data profile store (`…/teshi/model-profiles`, or `TESHI_APP_DATA_DIR`) as the daemon API so desktop, local web, TUI, and CLI agree on one machine.

#### Scenario: Desktop save visible to daemon GET

- **WHEN** the user saves and activates an LLM profile in `teshi-desktop` and then `GET /api/v1/llm/config` is called against a daemon on the same machine using that store
- **THEN** the response reflects the desktop-active profile’s `base_url` and model and configured key status

#### Scenario: Desktop save visible to TUI active profile

- **WHEN** the user activates a profile in `teshi-desktop` and then the TUI loads the active profile from the shared store
- **THEN** the TUI uses that profile’s provider, model, and configured key for LLM calls
