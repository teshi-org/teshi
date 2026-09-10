# Phase-one migration coverage

This report records the transport boundary implemented by this change. It is
not a REST removal/deprecation plan; every existing `/api/v1/*` route remains
registered and is covered by the existing daemon tests.

## Hosted UI control coverage

| UI operation group | Control method/event or preview path | Implementation evidence |
| --- | --- | --- |
| LLM config and profiles | `llm.get_config`, `llm.set_config`, `llm.list_profiles`, `llm.get_profile`, `llm.save_profile`, `llm.delete_profile`, `llm.activate_profile` | `WasmBackend` and `dispatch_control_request` |
| Browser sessions | `browser.start`, `browser.stop`, `browser.list_sessions`, `browser.activate_tab` | `WasmBackend` and `dispatch_control_request` |
| BDD runs | `bdd.list_scenarios`, `bdd.run`, `terminal.output`/runtime events | `WasmBackend`, dispatcher, ordered control event queue |
| Run exchange | `api.get_exchange` | `WasmBackend` and dispatcher |
| Project/filesystem/Gherkin | `project.*`, `filesystem.*`, `bdd.render_feature`, `bdd.validate_buffer` | daemon control dispatcher; REST remains for legacy clients |
| Locator/steps | `locator.*`, `steps.*` | daemon control dispatcher |
| Terminal | `terminal.spawn`, `terminal.stop`, `terminal.resize`, `terminal.write`; runtime events | daemon control dispatcher and bounded event queue |
| Agent/runtime | authenticated ordered runtime events; no standalone Agent RPC is used by the current hosted shell | control event subscription |
| Browser/WinApp preview | `/ws/preview` with `preview` hello and newest-frame relay | independent preview handshake/relay |

The hosted shell uses one `ControlClient` instance for all shared GPUI views.
It performs manifest compatibility negotiation before any request, keeps
pending requests correlated by id, reconnects only after a fresh hello, and
does not replay non-idempotent requests.

## Security and delivery evidence

- `/ws/control` and `/ws/preview` require exact `Origin: https://teshi.org`.
- The first message is a channel-specific, versioned hello carrying the
  in-memory launch token and UI compatibility identity.
- Hosted sessions are `HostedWebUi`, never implicit loopback `Admin`; daemon
  shutdown is explicitly forbidden to this capability, and the role cannot
  invoke legacy `/api/v1/*` routes.
- A new `teshi web` launch atomically replaces any older hosted launch token;
  explicit runtime/project teardown invalidates hosted tokens while retaining
  unrelated local automation sessions.
- Hosted browser responses/events use an allowlisted public session shape and
  recursively redact sidecar URLs, CDP paths, project roots, and command
  tokens before crossing the control socket. Control filesystem results use
  project-relative paths and deny `.teshi`/credential-like files; exchange
  inspection remains URL/body/header redacted even when an older UI requests
  plaintext.
- Gherkin validation events use the same hosted projection, including
  relative-path conversion for diagnostic scope and paths.
- Preview frame URLs are limited to `http`/`https`; private schemes and
  embedded Windows/Unix absolute paths are normalized before relay.
- Launch fragments are removed with `history.replaceState` before normal
  startup, and the `/app/` shell has restrictive CSP and `no-referrer` policy.
- The Pages workflow builds only the current `/app/` UI from an immutable Teshi
  SHA and validates the generated manifest/artifact before deployment.
- The nightly Windows installer path skips WASM tooling/build/staging and is
  checked for absence of `share/web` and hosted UI payloads.

## Verification status

The focused hosted-transport, release-workflow, nightly-artifact, and
REST-preservation contracts pass, as do repository-wide formatting and the
hosted Hugo workflow contract/YAML checks. The preview load contract records an
intentionally conservative 100 ms local control-response budget while the
preview/event queues are saturated.

The fresh `cargo test -p teshi-daemon --offline` rebuild passes (48 tests), and
the targeted daemon clippy gate passes with `-D warnings`. The WASM
compile/smoke and live Pages/Chromium acceptance remain pending because they
require the hosted deployment/toolchain environment.

The hosted exchange redaction helper tests pass (10 Python tests), and the
focused `teshi-ui` unit tests pass (14 tests). Native workspace `cargo check
--workspace --exclude teshi-web --locked` also passes. A stable-toolchain
WASM check is not a valid gate for this wasm-only crate; it requires the pinned
nightly toolchain used by the Hugo workflow.

The full `cargo test --workspace --exclude teshi-web --locked` run was also
attempted; two pre-existing `teshi-engine` response-stream tests received
`502 Bad Gateway` from their external API dependency. They are unrelated to
this transport change and need a service-available rerun before claiming the
repository-wide test gate is green.

The hosted transport contract audit now extracts all 14 control RPCs used by
the current WASM backend and compares them with the daemon dispatcher. It also
checks every concrete method in the checked operation inventory, the
Browser/Embedded/WinApp preview branches and preview hello, and the absence of
legacy REST requests from hosted business code. The only loader REST request
is the explicitly loopback-gated `?e2e=1` session bootstrap. This audit is
reproducible with `python scripts/test-hosted-ui-transport.py` and is run in
CI.

A live Chromium probe on 2026-09-10 reached the deployed
`https://teshi.org/app/` GPUI WASM shell. A matching nightly-identity local
CLI launched a dynamic loopback daemon and the hosted page removed the
fragment before normal startup. The live protocol probe completed both
`/ws/control` and `/ws/preview` hello handshakes, rejected an untrusted Origin,
rejected a pre-handshake request, rejected an Admin token, invalidated an old
session after replacement, and accepted a fresh reconnect. The browser page
loaded without `/api/v1/*` business resources; the only hosted HTTP resources
were the current `/app/` bundle and manifest.

The deployed manifest currently requires nightly build sequence
`34377301593` and source `bad069e27c5a9df952377ce5fbabbc0cade8a564`; the
local verification binary was rebuilt with that identity for this probe. The
operation-by-operation source/dispatcher audit for task 9.4 is complete. The
real deployed Pages/Chromium operation exercise remains an environment
acceptance concern: the deployed page used during this run did not expose the
current `/app/ui-manifest.json`, and the local development daemon does not
serve that production manifest route. The local daemon protocol and preview
relay tests remain the repeatable repository evidence until Pages is refreshed.

REST deletion, deprecation, or historical UI fallback is deliberately deferred
to a separate future OpenSpec change after that live acceptance evidence.
