## ADDED Requirements

### Requirement: Multi-profile LLM settings UI

The settings-hosted LLM UI SHALL present a profile list and an editor for the selected profile, with actions to create, clone, delete, activate, and save profiles. The editor MUST expose provider selection among `openai`, `anthropic`, and `deepseek-openai`, plus model id, max context tokens, max output tokens, base URL, API key, streaming toggle, HTTP extra headers, and chat options. API style controls MUST be shown only when provider is `openai`.

#### Scenario: User creates and activates a profile

- **WHEN** the user creates a new profile, fills required fields, saves, and activates it
- **THEN** the profile appears in the list as active and subsequent effective LLM config uses that profile

#### Scenario: Provider change updates labels and default base URL hint

- **WHEN** the user changes provider from `openai` to `deepseek-openai` and the base URL is empty or still equal to the previous provider default
- **THEN** the UI updates field labels for the new provider and applies the DeepSeek default base URL (or placeholder equivalent), without clearing a user-customized base URL that differs from the previous default

#### Scenario: API style hidden for Anthropic

- **WHEN** the selected profile provider is `anthropic`
- **THEN** the API style control is hidden or disabled and save stores chat-completions style semantics for that field

#### Scenario: API key remains masked on reload

- **WHEN** a saved profile with an API key is loaded into the editor
- **THEN** the UI MUST NOT show the full API key in plaintext (masked or empty with a configured indicator); an empty key on save MUST preserve the previously stored key

### Requirement: Daemon model profile HTTP API

`teshi-daemon` SHALL expose HTTP endpoints to list, read, create/update, delete, and activate model profiles against the shared app-data profile store. Public responses MUST mask API keys.

#### Scenario: List profiles

- **WHEN** a client sends `GET /api/v1/llm/profiles`
- **THEN** the daemon returns the profiles with masked keys and indicates which id is active

#### Scenario: Save and activate via HTTP

- **WHEN** a client `PUT`s a profile and `POST`s `/api/v1/llm/profiles/{id}/activate`
- **THEN** subsequent effective LLM resolution uses that profile

#### Scenario: WASM backend uses profile APIs

- **WHEN** the WASM GPUI shell loads and saves model profiles
- **THEN** it uses the daemon profile HTTP APIs via browser HTTP and MUST NOT link `teshi-engine`

## MODIFIED Requirements

### Requirement: LLM configuration UI in GPUI shell

The shared GPUI shell SHALL provide an LLM configuration UI hosted under the settings surface (not as the application default root). The UI MUST support multi-profile editing as specified in the multi-profile LLM settings UI requirement, rather than only a single flat base URL / model / API key form.

#### Scenario: User edits and saves settings

- **WHEN** the user opens settings, edits the selected profile fields, and activates Save
- **THEN** the shell persists the profile through its backend and shows a success or error status

#### Scenario: API key masked on reload

- **WHEN** a profile is loaded into the settings-hosted UI and an API key is already stored
- **THEN** the UI MUST NOT display the full API key in plaintext (masked or empty with a configured indicator)

#### Scenario: LLM form is not the home screen

- **WHEN** the application launches to its default root view
- **THEN** the LLM configuration form is not the sole visible root content; the user reaches it via settings

### Requirement: Daemon LLM config HTTP API

`teshi-daemon` SHALL continue to expose `GET` and `PUT` `/api/v1/llm/config` as a **flat projection of the active model profile** for compatibility (at least `base_url`, `model` / `model_id`, masked or omitted `api_key`, and `api_key_configured`). New clients SHOULD prefer the profile APIs for full fidelity.

#### Scenario: Get config

- **WHEN** a client sends `GET /api/v1/llm/config`
- **THEN** the daemon returns JSON for the active profile projection including `base_url`, model identifier, and a masked or omitted `api_key` plus a boolean indicating whether a key is configured

#### Scenario: Put config

- **WHEN** a client sends `PUT /api/v1/llm/config` with `base_url`, model, and `api_key`
- **THEN** the daemon updates the **active** profile with those fields and returns success

### Requirement: Web backend uses same-origin fetch

The WASM GPUI shell SHALL load and save LLM configuration by calling the daemon HTTP APIs using browser HTTP (`fetch` / GPUI `FetchHttpClient`), not by linking `teshi-engine`. Full profile editing MUST use the profile endpoints; the flat `/api/v1/llm/config` projection MAY still be used for simple active-profile updates.

#### Scenario: WASM save round-trip

- **WHEN** the user saves LLM config in the daemon-hosted GPUI web shell
- **THEN** a subsequent `GET /api/v1/llm/config` reflects the active profile’s saved `base_url` and model and reports the API key as configured

### Requirement: Native backend shares the same store

The native GPUI desktop shell SHALL load and save model profiles against the same user-level app-data profile store semantics as the daemon API so desktop and local web agree on one machine.

#### Scenario: Desktop save visible to daemon GET

- **WHEN** the user saves and activates an LLM profile in `teshi-desktop` and then `GET /api/v1/llm/config` is called against a daemon on the same machine using that store
- **THEN** the response reflects the desktop-active profile’s `base_url` and model and configured key status
