## Why

Chrome extension automation currently starts the Python `browser_service.py` process and requires a project virtual environment with `websockets`, even though the extension already performs browser-specific CDP work and Teshi already has a typed Rust client and per-user broker startup coordination. Moving the Chrome broker into Rust removes Python from the Chrome install path while keeping Embedded Playwright and WinApp on their existing runtimes.

## What Changes

- Add a workspace Rust broker runtime that serves the existing loopback HTTP and authenticated WebSocket contracts and owns Chrome session state, target routing, leases, pending requests, locator policy, evidence, captures, grants, and diagnostics.
- Keep the Chrome extension and its Chrome API/CDP implementation in JavaScript; preserve protocol v1, legacy single-session compatibility, endpoint discovery, and existing Feature bindings.
- Make broker process ownership user-wide and project context request-scoped. Bind leases and privileged grants to their project and browser target, and remove project paths and bearer secrets from public discovery and logs.
- Route CLI, daemon, MCP, Agent, Desktop, and Web UI through the existing typed Browser operation boundary and one broker process; Chrome startup must not inspect or invoke Python tooling.
- Add shared contract and negative security tests, real two-Profile E2E coverage, Python-free startup acceptance, performance measurements, and update CI/release/install documentation.
- Retire only the production Chrome Python broker after acceptance; retain Python for Embedded Playwright, WinApp, and migration comparison until each consumer is independently accounted for.

## Capabilities

### New Capabilities

- `rust-chrome-browser-broker`: Rust-owned local Chrome broker transport, process lifecycle, protocol compatibility, and user-session runtime.
- `browser-evidence-capture`: Target-isolated screenshot/preview, Console, Network capture, bounded storage, redaction, and reconnect semantics.

### Modified Capabilities

- `browser-extension-connection`: The extension connects to the Rust broker without a Python environment; discovery, authentication, origins, endpoint metadata, and reconnect remain safe and compatible.
- `multi-browser-session-broker`: Session and lease state are shared per OS user while projects, callers, profiles, windows, and tabs remain explicitly scoped.

## Impact

The change spans a new `teshi-browser-broker` workspace crate; `teshi-engine` startup/client integration; CLI internal broker dispatch; daemon, MCP, Agent, GPUI Desktop/Web adapters; extension protocol tests; the browser contract fixtures; Windows/Linux/macOS package manifests and workflow staging; and browser mode/security documentation. `resources/browser_service.py` remains an Embedded Playwright implementation, and `resources/winapp_service.py` remains the WinApp implementation. Existing HTTP discovery port `17373`, dynamically selected WebSocket port, TSH1 preview frames, protocol v1 message names, and project `.teshi/cdp-endpoint.json` remain compatibility inputs during migration.
