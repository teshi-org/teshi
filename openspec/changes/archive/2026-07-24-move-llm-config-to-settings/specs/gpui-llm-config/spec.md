## MODIFIED Requirements

### Requirement: LLM configuration UI in GPUI shell

The shared GPUI shell SHALL provide an LLM configuration UI with fields for base URL, model, and API key, plus an action to save and a visible configuration status. That UI MUST be hosted under the settings surface, not as the application's default root / main surface. No other product panels are required for this capability.

#### Scenario: User edits and saves settings

- **WHEN** the user opens settings, enters a base URL, model, and API key, and activates Save
- **THEN** the shell persists the values through its backend and shows a success or error status

#### Scenario: API key masked on reload

- **WHEN** LLM config is loaded into the settings-hosted UI and an API key is already stored
- **THEN** the UI MUST NOT display the full API key in plaintext (masked or empty with a configured indicator)

#### Scenario: LLM form is not the home screen

- **WHEN** the application launches to its default root view
- **THEN** the LLM configuration form is not the sole visible root content; the user reaches it via settings
