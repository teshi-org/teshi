## Why

Teshi can run ACP conversations but drops session configuration metadata, leaving users unable to select the external agent's model, mode, reasoning effort, or other advertised settings. Provide discoverable controls comparable to Zed, using pocketbot's dynamic configuration approach and Chrys's clickable bottom status bar as references.

## What Changes

- Preserve session configuration from creation, loading, and notifications and support changing advertised options on the existing session.
- Prefer configOptions and retain legacy mode/model compatibility; support grouped selections and boolean options without hard-coded provider values.
- Show clickable controls below the transcript, with keyboard-accessible pickers and an overflow entry for narrow terminals.
- Distinguish agent modes from Teshi tool approval policy and ensure displayed approval settings match the effective policy.
- Handle loading, pending changes, errors, dependent option refresh, and conversation isolation.
- Validate protocol behavior, runtime synchronization, terminal rendering, and real-agent interaction separately.

## Capabilities

### New Capabilities
- `acp-session-controls`: Discovery, state synchronization, mutation, and interactive TUI presentation of external agent session configuration and client approval policy.

### Modified Capabilities
None.

## Impact

Affected crates: teshi-acp, teshi-agent, teshi-agent-runtime, teshi-tui, and TUI test fixtures. Update ACP architecture and interaction documentation. Reference projects remain read-only. Preserve unrelated ongoing authoring and installation changes. No changes to native provider configuration or mandatory test-point review gates.
