## 1. ACP tool transport

- [x] 1.1 Add typed MCP server configuration to session creation.
- [x] 1.2 Implement an authenticated per-backend loopback bridge and internal STDIO proxy with bounded framing and cleanup.

## 2. Live TUI integration

- [x] 2.1 Advertise profile-permitted tools and dispatch calls through the owning AgentHost.
- [x] 2.2 Preserve test-point gates, manual file approval, cancellation, and tool-result delivery.

## 3. Verification and documentation

- [x] 3.1 Add deterministic MCP discovery/call/error/isolation/cleanup and live-host review regression tests.
- [x] 3.2 Run focused tests, formatting, compile, and Clippy checks; document the capability and remaining acceptance limits.
- [x] 3.3 Verify real Codex and CodeBuddy discover and call Teshi generation tools without writing unrelated project files.

## 4. Review fixes

- [x] 4.1 Return manual file-approval failures to MCP callers and clean up the failed approval.
- [x] 4.2 Handle STDIO cancellation and EOF while calls await host approval; test through the proxy transport.
- [x] 4.3 Reconnect ACP with the current profile's tool catalog when profile permissions change.
- [x] 4.4 Run regressions and quality gates and refresh the preview executable and verification notes.

## 5. Executable TUI acceptance

- [x] 5.1 Add paired English/Chinese executable `.feature` coverage with a real PTY runner for ACP requirements, human test-point approval, file-change approval, continuation, and Save-to-disk verification.
