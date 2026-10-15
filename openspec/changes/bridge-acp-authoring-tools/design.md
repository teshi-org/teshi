## Context

The ACP backend forwards system instructions as text but discards native tool definitions. The existing browser MCP adapter does not expose authoring or share TUI workflow state.

## Goals / Non-Goals

Goals: give ACP sessions live access to permitted Teshi tools, with unchanged test-point and file approval semantics. Non-goals: implement external agents' filesystem/terminal tools or bypass mandatory review.

## Decisions

Use ACP's typed STDIO MCP server configuration. The current executable runs a hidden STDIO proxy that forwards JSON-RPC to a random loopback port authenticated with an ephemeral OS-random token supplied through the MCP child environment. A per-backend bridge owns the listener and channels. Tool calls are polled and executed on the UI thread through the existing AgentHost, keeping workflow state authoritative. Manual file changes retain their response until accepted or rejected. Profile permissions filter discovery and execution. Pending calls fail when the bridge is dropped; the listener is stopped and joined.

The proxy avoids depending on agent support for HTTP MCP. A standalone authoring server would duplicate workflow state and lose live review integration. Native tool events cannot substitute for MCP because the external agent owns continuation.

The proxy reads STDIO independently of outstanding bridge calls. Cancellation
aborts the matching connection; EOF closes every outstanding connection. Manual
approval failures complete the waiting MCP response with an error and clear
the failed UI proposal. Changing profile permissions cancels the previous
turn and reconnects ACP with a fresh catalog, so agents rediscover both added
and removed tools without requiring MCP list-change notifications.

## Risks / Trade-offs

- Agent MCP naming differs: prompt guidance identifies the Teshi MCP server and underlying tool names.
- A stale tool request could outlive cancellation: execute only while the owning backend is running and reject calls after cancellation.
- Agents may lack working MCP support: deterministic protocol tests establish wiring; actual Codex and CodeBuddy runs remain a separate acceptance gate.
- The generation state is currently shared by the TUI: calls reuse the existing behavior rather than introducing another state model.
