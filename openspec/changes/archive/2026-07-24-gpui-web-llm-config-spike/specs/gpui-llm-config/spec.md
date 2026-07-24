## ADDED Requirements

### Requirement: LLM configuration UI in GPUI shell

The shared GPUI shell SHALL provide an LLM configuration surface with fields for base URL, model, and API key, plus an action to save and a visible configuration status. No other product panels are required for this capability.

#### Scenario: User edits and saves settings

- **WHEN** the user enters a base URL, model, and API key and activates Save
- **THEN** the shell persists the values through its backend and shows a success or error status

#### Scenario: API key masked on reload

- **WHEN** LLM config is loaded into the UI and an API key is already stored
- **THEN** the UI MUST NOT display the full API key in plaintext (masked or empty with a configured indicator)

### Requirement: Daemon LLM config HTTP API

`teshi-daemon` SHALL expose HTTP endpoints to read and update the spike LLM configuration store used by the GPUI web backend.

#### Scenario: Get config

- **WHEN** a client sends `GET /api/v1/llm/config`
- **THEN** the daemon returns JSON including `base_url`, `model`, and a masked or omitted `api_key` plus a boolean indicating whether a key is configured

#### Scenario: Put config

- **WHEN** a client sends `PUT /api/v1/llm/config` with `base_url`, `model`, and `api_key`
- **THEN** the daemon persists the values to the user-level LLM config store and returns success

### Requirement: Web backend uses same-origin fetch

The WASM GPUI shell SHALL load and save LLM configuration by calling the daemon LLM config HTTP API using browser HTTP (`fetch` / GPUI `FetchHttpClient`), not by linking `teshi-engine`.

#### Scenario: WASM save round-trip

- **WHEN** the user saves LLM config in the daemon-hosted GPUI web shell
- **THEN** a subsequent `GET /api/v1/llm/config` reflects the saved `base_url` and `model` and reports the API key as configured

### Requirement: Native backend shares the same store

The native GPUI desktop shell SHALL load and save LLM configuration against the same user-level store semantics as the daemon API so desktop and local web agree on one machine.

#### Scenario: Desktop save visible to daemon GET

- **WHEN** the user saves LLM config in `teshi-desktop` and then `GET /api/v1/llm/config` is called against a daemon on the same machine using that store
- **THEN** the response reflects the desktop-saved `base_url` and `model` and configured key status
