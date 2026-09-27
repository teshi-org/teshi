# OpenSpec 5.5 Network Body Access Handoff

- Change: `migrate-chrome-browser-broker-to-rust`
- Branch/commit: `dev` / `cfeb63a`
- Progress: 28/47; task 5.6 is the next unchecked task.
- Implementation: response-body access is an explicit pending-request grant bound to the complete target, broker generation, project, caller, active capture, retained request, lease, and configured byte limit. Request bodies and response bodies are bounded; list/diagnostic/quarantine paths omit raw bodies. Capture/target/lease/session/generation cleanup revokes access and queues extension cleanup where applicable.
- Validation: `cargo test -p teshi-browser-broker --locked` (105 passed); extension Network tests (19 passed); Python broker/service-flow tests (79 passed with `PYTHONPATH=resources/tests`); `cargo fmt --all -- --check`; native workspace `cargo check --workspace --exclude teshi-web --locked`; strict clippy; `openspec validate ... --strict`.
- Real baseline: Rust transport two-Profile harness passed 2/2 for locator/context, disconnect/reconnect, broker restart, identity preservation, and stale-token rejection. It does not cover stage-5 evidence capture yet. The legacy Python P0 harness against the existing 17373 listener failed during preflight with `browser_unavailable: Connection closed normally`; no browser scenario ran and the listener/process were left untouched.
- Next: add/run isolated Rust real-Chrome screenshot, Console, Network, reconnect, large-event, and backpressure evidence with bounded latency/working-set measurements and a Python comparison; do not change production Chrome routing.

