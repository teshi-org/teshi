## Why

ACP agents receive Teshi's feature generation instructions without the tools required to follow them. Codex conversations repeatedly stop before test-point review because those tools are absent.

## What Changes

- Attach a conversation-specific STDIO MCP server to ACP sessions.
- Route advertised Teshi tools to the live TUI host and return actual execution results.
- Preserve human test-point review, profile tool permissions, and file approval.
- Close the bridge when its backend is replaced or stopped.

## Capabilities

### New Capabilities
- `acp-authoring-tools`: Live host tool access for external ACP agents.

### Modified Capabilities

None.

## Impact

Teshi ACP session creation, agent runtime, TUI tool dispatch, and an internal MCP STDIO proxy. Native tools and external agents' own tool loops retain their existing ownership.
