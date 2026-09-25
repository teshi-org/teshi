# Chrome Browser Broker Rust Migration: Baseline Review

Baseline source: `dev` at `6728bfef746679d2f1d825152d0ffb8e0f9e19a2` (2026-09-24).
The implementation plan and module-level migration table are in
[`openspec/changes/migrate-chrome-browser-broker-to-rust/design.md`](../openspec/changes/migrate-chrome-browser-broker-to-rust/design.md).
This report records the pre-Rust behavior boundary and the checks run before changing
which implementation is selected at runtime.

## Current ownership

| Area | Current owner | Migration treatment |
| --- | --- | --- |
| Chrome extension APIs, CDP attach/detach, DOM/accessibility snapshots, locator verification in live frame/shadow context, browser input/navigation, screencast production, Network event buffering and resend | `extension/teshi-bridge/background.js`, `manifest.json` | Keep JavaScript and Chrome APIs. Adjust only a tested protocol/identity handshake if required. |
| Chrome HTTP discovery, WebSocket listeners, extension registration, command and preview-stream dispatch | `ChromeBridge`, `run_chrome`, `run_http_discovery` in `resources/browser_service.py` | Move to Rust. Preserve the fixed discovery port, dynamic WebSocket port and existing wire envelopes. |
| Chrome Profile/window/tab registry, heartbeat liveness, target resolution, exclusive leases, command queues, request correlation/cancellation, stale element references, locator candidate ranking, screenshot/PDF artifacts, Console and Network retention/acks, capability grants, project policy, audit and diagnostics | `resources/browser_agent_broker.py` plus Chrome branches of `resources/browser_service.py` | Move behavior and state to typed Rust modules; do not treat the Python broker as a transport-only shim. |
| Embedded Playwright browser lifecycle and local HTTP transport | `EmbeddedSession` / `run_embedded` in `resources/browser_service.py`, started through `teshi-engine` sidecar/venv helpers | Keep Python and Playwright. Remove only Chrome-specific imports/selection after consumer review. |
| Windows native application automation | `resources/winapp_service.py` and its managed runtime | Keep independent of the Chrome broker migration. |
| Browser operation models/client, user-broker start coordination/version preflight, endpoint read/write, CLI operations, MCP adapters, daemon and GPUI backends | `crates/teshi-engine/src/browser_agent.rs`, `sidecar.rs`; `crates/teshi-tui/src/cli/browser.rs`, `browser_endpoint.rs`; MCP/daemon/UI modules | Reuse these typed client and UI/daemon boundaries. They do not currently provide a Rust broker server. |

Chrome protocol compatibility currently includes HTTP discovery at
`http://127.0.0.1:17373/v1/bridge`, a dynamic authenticated WebSocket URL, the
`/extension/frames` preview stream and TSH1 binary frame metadata. Heartbeats negotiate
protocol/features; commands may travel over the direct WebSocket or the heartbeat queue;
responses correlate by request ID and target. Network capture uses target/capture-scoped
sequenced batches and contiguous acknowledgements. Protocol-v0 implicit targeting is
available only when resolution is unambiguous. Per-project endpoint pointers are written
to `.teshi/cdp-endpoint.json`.

## Findings and migration boundary

The Python Chrome broker owns meaningful automation policy and evidence state: target and
Profile isolation, lease enforcement, command lifecycle, locator strategy, artifact bounds,
capture retention/redaction, authorization and audit. These functions must move; the
extension's Chrome-specific execution primitives should not be duplicated in Rust.

Current Rust already has `BrowserOperation` request/response models and the WebSocket
client, a per-user startup coordinator with schema/protocol/feature checks, project endpoint
handling, and CLI/MCP/Daemon/Desktop/Web adapters. The Chrome startup branch nevertheless
resolves the project `.venv`, imports `websockets`, and launches `browser_service.py`.
Those client surfaces are reusable; the process startup and all server-side broker state
are not.

Packaging stages Python browser resources for current releases. Those resources cannot be
removed as a group: Embedded still imports the shared service module, and WinApp has its
own Python runtime. Chrome-only removal must follow a consumer split and acceptance, not
precede it. `ci.yml` did not run the Python browser-broker or extension Node contract suites
before this change; the real two-Profile acceptance exists but depends on a working local
Chrome broker and headed Chromium.

Review also found defects that are migration gates rather than compatibility promises:

1. Unauthenticated HTTP discovery returns a `ws_url` containing the bearer credential and
   the broker's startup project path. Token checks still protect mutation/WebSocket use,
   and the usual listener is loopback-only, but browser-readable discovery exposes more
   than required.
2. CORS and WebSocket origin checks accept any syntactically valid Chrome extension ID,
   rather than a supported Teshi extension identity.
3. The process is described as per-user, but extension heartbeat and preview-stream hello
   are rejected unless their `project_root` matches the first project's startup root; only
   some command/policy/artifact paths are request-project scoped. A shared process can
   therefore mis-bind a second project.
4. Privileged grant creation receives `interactive_confirmed` and caller identity from the
   request envelope. Client-provided confirmation is not itself a trusted authorization
   decision and needs server-side enforcement in the security phase.
5. Public renewal after lease expiry currently reports `invalid_browser_lease`: session
   lookup clears the expired lease before token validation can report
   `expired_browser_lease`. The migration fixture records the observable public result;
   Rust must keep a stable explicit failure and must never renew or dispatch with that
   token.

The Rust design keeps v1 messages and existing endpoint readers, but it must not preserve
these unsafe or project-confusing behaviors. The extension's packaged/unpacked identity
must be resolved before narrowing credential delivery; do not substitute a permissive
origin fallback.

## Automated behavior baseline

Executed on the current Python implementation before changing runtime selection:

| Command | Result |
| --- | --- |
| `python -m unittest resources.tests.test_browser_agent_broker -q` | 45 tests passed |
| `python -m unittest resources.tests.test_browser_service_http -q` | 6 tests passed |
| `python -m unittest discover -s resources/tests -p 'test_browser_service_agent_flow.py' -q` | 29 tests passed |
| `python -m unittest discover -s resources/tests -p 'test_browser_p2_privileged.py' -k setup_failure -q` | 1 setup-cleanup regression test passed |
| `node --test extension/teshi-bridge/tests/protocol.test.mjs extension/teshi-bridge/tests/network-capture.test.mjs` | 25 tests passed |
| `cargo test -p teshi-engine shared_contract_fixture_preserves_legacy_and_versioned_messages --locked` | 1 Rust fixture-consumer test passed |
| `cargo fmt --all --check`, `git diff --check`, `openspec validate migrate-chrome-browser-broker-to-rust --strict` | All passed |

The existing Python suite asserts target isolation, ambiguous-target rejection, leases,
request races/duplicate dispatch/disconnect cleanup, stale element references, locator
ranking and verification, screenshot/PDF handling, Console and Network filtering/acks,
capability scopes, artifact bounds, and malformed/oversized capture batches. The shared
fixture also pins heartbeat reconnect, lease renewal/release/expiry, duplicate and late
requests, locator outcomes, evidence limits, capability scope/revocation, network sequence
barriers, and malformed transport cases. Python state-transition tests consume these
vectors. The Rust typed fixture consumer currently validates the shared records and selected
negative cases; Rust state-machine differential coverage remains future work.

The headed real-browser attempt did not reach Chromium. The existing
`test_browser_two_profile_p0.py` setup timed out while running `target/debug/teshi.exe
browser sessions`; the P2 acceptance setup hit the same prerequisite. Inspection showed a
saved per-user endpoint referring to a PID that no longer exists and no listener on port
17373. The endpoint was not treated as a live broker and no process was killed. The
P2 test had created `.teshi/browser-policy.json` before setup failed; the exact test-created
file was removed, and the test now restores the prior file (or removes only its own file)
even when async setup raises. The full headed two-Profile behavior remains unverified until
broker startup is repaired and Chrome can be launched.

## Python transport microbenchmark

Command: `python resources/tests/benchmark_browser_command_transport.py` (8 samples).

| Path | Median | p95 | Min | Max |
| --- | ---: | ---: | ---: | ---: |
| Heartbeat polling queue | 752.77 ms | 1335.35 ms | 172.16 ms | 1335.35 ms |
| In-process direct queue claim + one event-loop turn | 0.0038 ms | 0.0177 ms | 0.0034 ms | 0.0177 ms |

This measures queue scheduling in-process, not full extension WebSocket/browser latency,
startup, screenshot transfer or memory. It is only a narrow Python baseline; no general
performance conclusion is supported until equivalent Rust and real-Chrome runs exist.

## Acceptance gaps

- Fix and retest stale user-broker endpoint/crash recovery before the real two-Profile test.
- Expand shared fixtures and stateful consumers for heartbeat reconnect/expiry, lease renewal
  and release, locator failure variants, evidence limits, authorization boundaries, and
  malformed/oversized transport; current Python tests cover many of these but not all from
  the common fixture.
- Resolve deterministic extension identity for credential handoff; add negative discovery,
  origin, stale-token and cross-project authorization tests before Rust transport cutover.
- Run real Chrome two-Profile and two-project isolation, Embedded and WinApp regressions,
  Python-absent packaging acceptance, and the complete performance matrix before removing
  Chrome-only Python selection or source.
