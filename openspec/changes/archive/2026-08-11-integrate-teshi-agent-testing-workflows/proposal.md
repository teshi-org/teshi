## Why

Teshi already connects a Chrome extension to a local browser sidecar and can expose page snapshots and locator operations. However, an external coding agent cannot yet treat Teshi as a stable source of Playwright locators: the installed workflow is not discoverable, locator results are not expressed as a Playwright-oriented contract, and commands implicitly address the single active browser connection.

The immediate product goal is to let an agent inspect a real signed-in browser profile, obtain and verify robust Playwright locators, and use those results while authoring automation code. The connection model must also evolve now so multiple browser profiles and multiple agents can work in parallel later without replacing the protocol again.

## What Changes

- Make the existing `teshi-bridge` Chromium extension an explicit, installable part of the external-agent locator workflow, with local connection status and actionable setup diagnostics.
- Add structured browser-session and tab discovery so every browser operation targets an explicit extension instance, window, and tab instead of a process-global active browser.
- Add Playwright-oriented locator acquisition that ranks semantic locator candidates, renders usable Playwright expressions, and verifies uniqueness and visibility in the correct page/frame context.
- Replace the single-extension bridge state with a local multi-session broker that can register multiple browser-profile extension instances and route commands, responses, and preview data independently.
- Add session leases and request correlation so multiple agents can reserve different browser sessions and operate them concurrently without cross-profile commands or responses.
- Expose the workflow through machine-readable CLI operations and a thin local STDIO MCP adapter, backed by the same typed operation model.
- Make the GPUI WASM application in `apps/teshi-web` the supported `teshi web` surface for browser-session discovery and selection; remove the retired React application, its runtime resolution, release packaging, and frontend gates.
- Package focused Agent Skills, MCP metadata, browser-extension assets, and compatibility information for use outside the Teshi source checkout.
- Preserve a compatibility path for existing single-browser commands while making ambiguous implicit targeting fail with migration guidance.

## Capabilities

### New Capabilities

- `browser-extension-connection`: Installation, registration, identity, health, and local-only communication for browser-profile extension instances.
- `multi-browser-session-broker`: Concurrent registration, discovery, explicit routing, leasing, and isolation of multiple browser profile sessions.
- `playwright-locator-acquisition`: Structured page inspection plus ranked, rendered, and verified Playwright locator candidates.
- `external-agent-testing-interface`: Machine-readable CLI and local MCP operations through which agents discover browser targets and request locator results.
- `agent-testing-distribution`: Installable Skills/plugin metadata and browser-extension resources required by the locator workflow.
- `gpui-wasm-web-shell`: The official GPUI WASM browser-session surface, its same-origin daemon adapter, and removal of the React application.

### Modified Capabilities

- `module-boundaries`: Replace the former React daemon UI requirement with the GPUI WASM web shell and require the retired React package to be absent.

## Impact

- Affected areas include `extension/teshi-bridge`, the Python browser bridge, `crates/teshi-engine` operation/session models, `crates/teshi-tui` CLI routing, `crates/teshi-ui`, `apps/teshi-web`, `apps/teshi-daemon`, a local STDIO MCP adapter, existing browser locator Skills, release archives/installers, and browser-mode documentation.
- Remove `apps/teshi-web-ui` from the repository; `teshi web`, installers, and release workflows resolve and ship `apps/teshi-web/dist` instead.
- The fixed loopback bridge can remain the single local rendezvous point, but its state changes from one current extension to an instance-indexed session registry.
- `.teshi/cdp-endpoint.json` can remain as a legacy/default discovery file during migration; new operations discover and address sessions through the broker.
- Existing single-profile users continue to work when exactly one eligible session exists. Multiple eligible sessions require an explicit session and tab selection.
- WinApp automation, deterministic CI replay, Behave export hardening, JUnit reporting, and consumer-specific onboarding are intentionally deferred to separate changes.
