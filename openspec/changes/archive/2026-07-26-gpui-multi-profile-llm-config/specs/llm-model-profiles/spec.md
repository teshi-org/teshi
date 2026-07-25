## ADDED Requirements

### Requirement: Named model profiles persist under app data

The system SHALL persist named LLM model profiles as individual JSON files under the Teshi app data `model-profiles/` directory, with a separate active-profile pointer file selecting which profile drives runtime LLM calls.

#### Scenario: Save creates or updates a profile file

- **WHEN** a client saves a profile with a stable `id`
- **THEN** the profile is written to `model-profiles/{id}.json` and a subsequent list/load returns the saved fields (API key masked on public reads)

#### Scenario: Activate selects runtime profile

- **WHEN** a client activates profile `id`
- **THEN** the active pointer refers to that `id` and `effective_llm_config` resolves from that profile

#### Scenario: Delete refuses to leave zero profiles when one remains active

- **WHEN** the user deletes a non-active profile
- **THEN** the profile file is removed and other profiles remain available
- **WHEN** the user deletes the active profile and at least one other profile exists
- **THEN** the system activates another remaining profile
- **WHEN** the user attempts to delete the last remaining profile
- **THEN** the system rejects the delete or recreates a minimal default so at least one profile remains

### Requirement: Built-in providers and defaults

Each profile SHALL use one of the built-in provider ids `openai`, `anthropic`, or `deepseek-openai`. Empty `base_url` MUST resolve to the provider default base URL at effective-config time (`https://api.openai.com/v1`, `https://api.anthropic.com`, `https://api.deepseek.com` respectively).

#### Scenario: Empty base URL uses provider default

- **WHEN** an active profile has `provider` `deepseek-openai` and empty `base_url`
- **THEN** effective config uses `https://api.deepseek.com` as the base URL

#### Scenario: Unknown provider rejected on save

- **WHEN** a client attempts to save a profile with a provider id outside the built-in set
- **THEN** the save fails with a validation error

### Requirement: Profile field set for transport options

A model profile SHALL include: `id`, `name`, `provider`, `api_style`, `model_id`, `max_context_tokens`, `max_output_tokens`, `base_url`, `api_key`, `stream`, `http_headers`, and `chat_options`. `api_style` MUST be `chat_completions` or `responses`. For non-`openai` providers, effective transport MUST treat style as chat-completions semantics (Anthropic uses Messages) regardless of stored style.

#### Scenario: OpenAI responses style stored

- **WHEN** a profile with `provider` `openai` and `api_style` `responses` is saved and activated
- **THEN** effective config reports `api_style` `responses`

#### Scenario: Streaming default

- **WHEN** a new profile is created without an explicit stream flag
- **THEN** `stream` defaults to true

### Requirement: One-time migration from legacy llm-config.json

On first access to the profile store, if no profiles exist and legacy `llm-config.json` contains a usable configuration, the system SHALL create a profile named `Default`, copy base URL / model / API key, activate it, leave the legacy file in place, and record a migration marker under `model-profiles/`.

#### Scenario: Legacy config imported once

- **WHEN** app data has `llm-config.json` with an API key and an empty `model-profiles/` directory
- **THEN** the first profile-store load creates and activates `Default` with those values and writes a migration marker so a later load does not duplicate the profile
