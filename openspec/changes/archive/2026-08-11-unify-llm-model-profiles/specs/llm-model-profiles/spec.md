## MODIFIED Requirements

### Requirement: Named model profiles persist under app data

The system SHALL persist named LLM model profiles as individual JSON files under the Teshi app data `model-profiles/` directory, with a separate active-profile pointer file selecting which profile drives runtime LLM calls. The default Teshi app data directory SHALL be the OS data directory joined with `teshi` (for example `%APPDATA%/teshi` on Windows or the XDG data home equivalent on Linux), overridable via `TESHI_APP_DATA_DIR`. The same store SHALL be the source of truth for TUI, CLI, Desktop, and daemon on one machine.

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

#### Scenario: Default app data directory is teshi

- **WHEN** `TESHI_APP_DATA_DIR` is unset
- **THEN** the profile store resolves under the OS data directory path ending with `teshi/model-profiles`

## ADDED Requirements

### Requirement: One-time migration from legacy teshi-desktop app data

On first resolution of the Teshi app data directory, if the new `teshi` root has not yet recorded a desktop-migration marker and a sibling legacy `teshi-desktop` app data directory exists with usable content, the system SHALL copy `model-profiles/`, `llm-config.json`, `settings.json`, and `recent.json` into the new root (without deleting the legacy directory) and write a migration marker so the copy does not repeat.

#### Scenario: Desktop directory copied once

- **WHEN** `…/teshi` has no desktop-migration marker and `…/teshi-desktop/model-profiles` contains a profile
- **THEN** the first `app_data_dir` resolution copies those profiles into `…/teshi/model-profiles` and writes the marker

#### Scenario: Migration is idempotent

- **WHEN** the desktop-migration marker already exists under `…/teshi`
- **THEN** a later resolution does not re-copy or duplicate profiles from `teshi-desktop`

### Requirement: One-time import from legacy TUI config

On first access to the profile store, after any `llm-config.json` migration, if no profiles exist and a migration marker for TUI import is absent, the system SHALL import legacy TUI model TOML files from the OS config directory `teshi/models/*.toml` when present, and otherwise synthesize profiles from `teshi/config.toml` `[providers.*]` entries plus `teshi/auth.json` keys when those yield a usable API key. Provider id `deepseek` MUST map to `deepseek-openai`. The import MUST write a marker so it does not repeat. Existing engine profiles MUST NOT be overwritten by this import.

#### Scenario: TOML models imported when store empty

- **WHEN** `model-profiles/` is empty and `config_dir/teshi/models/{id}.toml` exists with a model and API key
- **THEN** the first profile-store load creates a matching JSON profile and writes the TUI-import marker

#### Scenario: Providers plus auth.json synthesize profiles

- **WHEN** `model-profiles/` is empty, no TOML models exist, and `auth.json` has a key for provider `openai` referenced by `config.toml`
- **THEN** the first load creates an `openai` profile with that key and writes the TUI-import marker

#### Scenario: Existing profiles skip TUI import

- **WHEN** `model-profiles/` already contains at least one profile
- **THEN** TUI legacy import does not add duplicate profiles from TOML or auth.json
