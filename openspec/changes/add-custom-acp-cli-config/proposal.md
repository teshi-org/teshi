## Why

Teshi can already launch a custom ACP agent, but registering one requires an undiscoverable TUI slash command and the persisted record is treated as an internal install manifest. Users need a scriptable command surface and a documented, safely editable configuration contract before richer UI management is added.

## What Changes

- Add `teshi acp` commands to list, inspect, add, update, remove, and locate user-level ACP agent definitions.
- Promote the existing user-level ACP manifest to a documented configuration contract while preserving compatibility with already-installed Registry agents and custom entries.
- Support display names, structured program arguments, and runtime environment-variable references for custom agents without persisting referenced secret values.
- Make the TUI and CLI consume the same ACP configuration implementation.
- Add stable human-readable output and `--json` output for automation.

## Capabilities

### New Capabilities

- `custom-acp-configuration`: User-level custom ACP agent definitions, CLI lifecycle management, safe environment resolution, and shared TUI consumption.

### Modified Capabilities

None.

## Impact

- Moves the reusable ACP agent manifest model and storage operations out of the TUI-only module into `teshi-acp`.
- Extends the `teshi` command tree with an `acp` command group.
- Keeps the current user data location and existing JSON records readable.
- Adds documentation for direct configuration-file editing and CLI examples.
- Does not add Desktop/Web management UI or persist the ACP selected for a conversation.
