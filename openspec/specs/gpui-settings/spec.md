# gpui-settings

## Purpose

Settings navigation and host surface inside the shared GPUI shell, including entry from the main surface and configuration sections (LLM today; more panels later).

## Requirements

### Requirement: Settings surface hosts configuration panels

The shared GPUI shell SHALL provide a settings surface that hosts configuration panels, reachable from the main shell surface and dismissible so the user can return to main.

#### Scenario: Open settings from main

- **WHEN** the user activates the settings entry from the main surface
- **THEN** the shell shows the settings surface and hides the main content

#### Scenario: Return to main from settings

- **WHEN** the user activates the back or close action on the settings surface
- **THEN** the shell shows the main surface again

### Requirement: LLM configuration lives under settings

The settings surface SHALL include the LLM configuration UI (base URL, model, API key, save, and status) as a settings section. The main surface MUST NOT present LLM configuration fields as its primary content.

#### Scenario: Edit LLM config only in settings

- **WHEN** the user opens settings
- **THEN** they can edit and save LLM configuration through the settings-hosted LLM section

#### Scenario: Main has no LLM form

- **WHEN** the shell is showing the main surface
- **THEN** base URL, model, and API key editors are not shown as the main content
