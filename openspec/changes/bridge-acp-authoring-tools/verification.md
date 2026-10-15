# Verification

## Deterministic checks

- `cargo test -p teshi-acp -p teshi-agent-runtime -p teshi-tui --locked`: passed after review fixes; TUI 396 passed, 1 ignored, plus ACP/client/runtime/integration/doc tests. Ignored tests retain their explicit live-environment gates.
- `cargo test -p teshi-agent-runtime tool_bridge --locked`: 8 passed, including cross-bridge isolation and a subprocess fixture exercising the production STDIO reader and proxy.
- TUI host regression covers requirements, persisted Proposed test points, blocked planning/writing before human approval under Manual/Auto/Bypass, human Continue, accepted plan, Feature creation, scenario insertion, validation, and explicit Save.
- File approval regression covers pending response, accept, reject, rejection of acceptance after cancellation, and write failure after manual acceptance with tool-error delivery and approval cleanup.
- A subprocess proxy regression sends two concurrent requests, cancels one by MCP request ID, confirms the other remains pending, then closes STDIN and verifies that the remaining call closes and the child exits.
- A profile-switch regression exercises the production selection action: no tools to submit_requirements to no tools. Each reconnection advertises the correct catalog and restores conversation history on its first prompt.
- Validation regression confirms unsaved accepted buffer edits are validated rather than older disk content.
- Workspace native compile and Clippy gates exclude the wasm-only teshi-web crate. Formatting and OpenSpec strict validation passed.

## Live provider interoperability

The `acp-authoring-probe` example ran against the installed `codex-acp` and `codebuddy-code` definitions with an isolated temporary working directory and the built preview executable as the STDIO MCP proxy. Both providers called `submit_requirements` followed by `propose_test_points` and completed with `end_turn`.

- Codex after review fixes: host calls at approximately 14.7 and 18.3 seconds.
- CodeBuddy after review fixes: host calls at approximately 8.6 and 11.0 seconds.

The rebuilt CLI proxy was also checked directly against a diagnostic loopback
listener. After cancellation and STDIN EOF, it exited with code 0 and the
listener observed EOF; the previous review reproduction left both alive.

This establishes real provider discovery/call/result interoperability through the executable proxy. The probe uses a non-writing host; full workflow state and file editing are established by the deterministic live-TUI-host tests. Interactive visual acceptance with real providers was not performed. Native workspace-wide tests and the separate WASM smoke were not run for this change.

## Running binary

The existing target/debug/teshi.exe was in use, so it was not replaced or terminated. The updated preview is target/debug/teshi-authoring-preview.exe. Restart using this executable and establish a new ACP connection to use the new MCP descriptor. Existing ACP sessions created by the older executable do not acquire tools retroactively.
