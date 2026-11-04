# Chrome browser broker — stage 6.5 security review

Review status: complete for the Rust broker implementation before production
routing changes. The production Chrome selection remains Python-backed; this
review does not approve a route switch, deletion, packaging, or release.

## Scope and decision

The review covered discovery, listener and endpoint permissions, sensitive data
handling in logs and audit records, project/caller/Profile request scoping,
legacy protocol compatibility, and the server-side checks that run before an
extension command is dispatched.

The Rust broker passes this pre-cutover review after one defense-in-depth fix:
the state owner now re-validates every `BrokerEvent::Operation` envelope. The
WebSocket boundary already performed that check, but the typed state boundary
must not rely on a transport caller to enforce the operation allowlist and
schema gate. `security_negative.rs` proves that direct state events for an
unknown operation or incompatible schema are rejected without dispatch.

## Review matrix

| Area | Server-side control | Evidence |
| --- | --- | --- |
| Discovery | Loopback listener and exact `Host`; public GET is project-neutral and credential-free; credential-bearing POST requires the browser-supplied exact paired extension Origin; untrusted Origins are denied; CORS is returned only for paired Origins. | `server.rs` discovery/preflight handlers; broker discovery and hostile-origin tests |
| Endpoint permissions | Both listeners bind to `127.0.0.1`; mutations and WebSockets require the generation token; token comparison is constant-time; extension streams require an exact configured `chrome-extension://` Origin; paired origins are bounded to 16. | `server.rs` listener/auth helpers; `credential.rs` origin validation; server and integration negative tests |
| Native listener identity | Public endpoint metadata contains no bearer token or project path. Native reuse can use the nonce-bound HMAC identity proof, which binds schema, protocol, PID, and broker start identity without sending the bearer token. | `protocol.rs` endpoint/proof DTOs; `credential.rs`; identity-proof tests |
| Extension permissions | Required manifest permissions are explicit. Privileged Chromium permissions remain optional and popup-gesture-only; the Rust state owner checks the permission announced by the authenticated heartbeat and separately checks the capability grant. | `extension/teshi-bridge/manifest.json`; `protocol.test.mjs`; `state.rs` privileged pre-dispatch path |
| Legacy compatibility | Protocol-v0 stream hello is rejected. The retained legacy heartbeat/session path is still subject to the current lease, target, feature, optional-permission, and capability-grant checks. | `security_negative.rs` protocol-v0 test |
| Sensitive data | Bearer and grant tokens are removed before extension dispatch; private credential `Debug` redacts the bearer; audit records retain only bounded metadata and filter by project/caller; sensitive audit keys are redacted; Network listings omit bodies; body access is short-lived and revoked on lifecycle cleanup. | `credential.rs`, `authorization.rs`, `state.rs`, `evidence.rs`; existing redaction/body-grant tests |
| Request scoping | Complete opaque target identity is required for explicit routing. Leases bind Profile, project, caller, and broker generation. Grants additionally bind OS user, capability, and expiry. Pending responses require request ID, operation, Profile, complete target, stream generation, and capture ID where applicable. | `session.rs`, `state.rs`, `authorization.rs`; lease/response/disconnect/race tests |
| Actual dispatch gate | The state owner checks the supported operation, broker/session features, extension-advertised operations, lease, optional Chrome permission, capability grant, operation-specific bounds, upload canonicalization, and evidence scope before queueing a command. The state owner now repeats the envelope validation independently of transport. | `state.rs` `handle_operation`/`forward_operation`; direct state negative test |
| Logging and failure output | No broker source log records project paths, tokens, cookies, page bodies, or capture bodies. The listener-exit log records only the bounded listener error. Public endpoint and error projections omit private credential material. | `server.rs`, `credential.rs`, `authorization.rs`, `state.rs` source audit and tests |

## Validation run

- `cargo test -p teshi-browser-broker --test security_negative --locked -- --nocapture`: 5 passed.
- `cargo fmt --all -- --check`: passed after the state-owner fix.
- Earlier stage-6.4 gates remain valid: native workspace check/test/clippy,
  extension protocol and Network tests, `git diff --check`, and strict
  OpenSpec validation.

## Remaining acceptance boundaries

This review does not replace the later gates for all Teshi entry points, real
two-Profile/two-project Chrome behavior, Python-free startup, Windows ACL and
packaging inspection, performance measurement, or final release-equivalent
validation. Until those gates are complete, the Rust broker remains migration
code and the Python production route and old source remain in place.
