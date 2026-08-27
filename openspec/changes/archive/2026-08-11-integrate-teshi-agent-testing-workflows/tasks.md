## 1. Contracts and Compatibility Baseline

- [x] 1.1 Inventory the existing extension heartbeat, discovery, command, response, frame, `.teshi/cdp-endpoint.json`, and browser CLI contracts and capture single-session compatibility fixtures.
- [x] 1.2 Define versioned types and schemas for extension identity, browser session, window/tab target, request correlation, lease, page-context revision, and locator result.
- [x] 1.3 Define stable error codes for unavailable, incompatible, disconnected, ambiguous, busy, stale, mismatched, and expired browser targets.
- [x] 1.4 Add serialization and contract tests proving target and request identity survive every extension, broker, operation, CLI, and MCP boundary.

## 2. Extension Identity and Multi-Session Broker

- [x] 2.1 Persist a random extension instance identifier in profile-local extension storage and add an optional user-editable display label.
- [x] 2.2 Add instance identity, extension/protocol versions, browser metadata, windows, and tabs to heartbeat, response, and frame messages while accepting legacy messages during migration.
- [x] 2.3 Replace global extension state with an instance-indexed broker registry containing health, last heartbeat, tab inventory, command queue, pending requests, stream state, and lease state.
- [x] 2.4 Route commands, responses, preview frames, and diagnostics by composite instance/window/tab target and unique request identifier.
- [x] 2.5 Expire stale instances, bound queues, fail pending requests, and recover leases when a browser profile disconnects.
- [x] 2.6 Add protocol compatibility preflight and actionable extension popup/broker diagnostics for disconnected, incompatible, debugger-conflict, and stale states.
- [x] 2.7 Add broker tests with multiple fake extension instances, colliding tab IDs, delayed responses, disconnects, and malformed cross-session responses.

## 3. Session Discovery, Targeting, and Leases

- [x] 3.1 Add reusable operations to list browser sessions, list windows/tabs, inspect health, and return non-sensitive selection metadata.
- [x] 3.2 Add acquire, renew, and release operations for exclusive instance-level leases with opaque tokens, owner labels, TTLs, and bounded recovery.
- [x] 3.3 Require explicit session/tab targets and lease tokens for locator acquisition and browser mutation operations.
- [x] 3.4 Preserve implicit targeting only when exactly one eligible target exists and return `ambiguous_browser_target` without mutation otherwise.
- [x] 3.5 Add `--session`, `--tab`, and lease inputs, or equivalent structured selectors, to existing browser CLI commands without changing unambiguous single-session defaults.
- [x] 3.6 Add concurrency tests proving different agents can operate different profiles in parallel while same-session mutation is rejected.

## 4. Playwright Locator Acquisition

- [x] 4.1 Normalize the browser snapshot into a reusable page, frame, shadow-context, accessible-element, and page-revision model outside UI shells.
- [x] 4.2 Define locator intent inputs that can identify an element by user-supplied purpose, text, role, current element reference, or selected Gherkin step without inventing test actions.
- [x] 4.3 Generate and rank `getByRole`, `getByLabel`, `getByPlaceholder`, configured `getByTestId`, stable attribute, and CSS candidates.
- [x] 4.4 Penalize or reject positional selectors, generated classes, long DOM paths, coordinates, ambiguous text, and other fragile strategies with machine-readable reasons.
- [x] 4.5 Render valid Playwright expressions and return structured arguments, context, match count, state, stability rationale, warnings, and alternatives.
- [x] 4.6 Verify recommended candidates in the explicitly targeted tab/frame and return stale-page-context or unverified results when the document changes.
- [x] 4.7 Add project configuration for test-id attribute names with documented defaults.
- [x] 4.8 Add fixture-page tests for roles, labels, placeholders, test IDs, duplicate names, iframes, shadow DOM, dynamic replacement, and CSS fallback.

## 5. External Agent Interface

- [x] 5.1 Move effectful browser session, snapshot, locator, verification, and lease logic behind reusable typed operations that do not depend on Desktop, Web, or TUI selection state.
- [x] 5.2 Add documented JSON output for session discovery, lease management, snapshot, locator resolution, and locator verification with stable non-zero exit behavior for operation errors.
- [x] 5.3 Add a thin `teshi mcp serve --stdio` adapter exposing the same browser operations and result schemas.
- [x] 5.4 Add MCP server instructions covering extension setup, explicit target selection, lease ownership, same-host limits, privacy, and non-destructive locator acquisition.
- [x] 5.5 Add CLI/MCP parity tests for successful locator acquisition, ambiguous targets, busy leases, expired sessions, stale pages, timeouts, and incompatible extensions.
- [x] 5.6 Add agent-consumable evidence references for optional screenshots without mixing frames between sessions or requests.

## 6. Skill, Extension, and Release Distribution

- [x] 6.1 Consolidate a focused Playwright locator Skill that performs broker health, extension compatibility, session selection, lease acquisition, snapshot, resolution, verification, and lease release.
- [x] 6.2 Remove source-checkout-only references so the Skill and all supporting documentation work from repository-local and installed package layouts.
- [x] 6.3 Add plugin metadata declaring compatible CLI, broker protocol, extension, Chromium, operating-system, and optional MCP versions.
- [x] 6.4 Include the compatible extension bundle and installation guidance in installers and release archives that advertise browser locator support.
- [x] 6.5 Document dedicated profile setup, display labels, debugger/DevTools conflicts, session selection, lease recovery, and multi-agent allocation.
- [x] 6.6 Add package smoke tests that install outside the source checkout, discover the Skill, resolve all references, check versions, and connect a fake extension.

## 7. Migration and End-to-End Validation

- [x] 7.1 Keep `.teshi/cdp-endpoint.json` and existing browser commands working through a default-target compatibility adapter when only one eligible session exists.
- [x] 7.2 Add a shared GPUI browser-session panel used by Desktop and GPUI WASM Web that displays broker identity and never silently selects across multiple profiles.
- [x] 7.3 Add an end-to-end single-profile test covering extension registration through verified Playwright locator output.
- [x] 7.4 Add an end-to-end multi-profile test with at least two extension instances and two concurrent mock agents acquiring distinct leases and locators.
- [x] 7.5 Validate that URLs, titles, page data, screenshots, request results, and debugger commands do not cross session boundaries in real-browser and GPUI WASM flows.
- [x] 7.6 Run extension tests, GPUI UI tests, GPUI WASM build, Rust format/check/test gates excluding native `teshi-web`, strict OpenSpec validation, and release package smoke tests; record known pre-existing failures separately.
- [x] 7.7 Add same-origin daemon adapters for browser session inventory and tab activation while keeping the browser broker bound to loopback.
- [x] 7.8 Make `apps/teshi-web/dist` the only supported `teshi web` development and release bundle; delete `apps/teshi-web-ui` and remove its daemon fallback, installer/release builds, docs, hooks, ignores, and supported frontend gates.
- [x] 7.9 Fix real-browser validation defects in dynamic sidecar port discovery, complete HTTP body reads, hyphen-leading lease-token parsing, locator intent matching, and CLI error exits.
- [x] 7.10 Manually validate the built GPUI WASM UI with three real Chrome profiles, including explicit selection, tab activation, disconnect behavior, and cross-session isolation.
