# Browser broker migration — stage 5.6 handoff

Stage 5.6 is complete. OpenSpec progress is 29/47 tasks; the next task is 6.1.

## Evidence

- Rust real Chrome evidence: `python -m unittest resources.tests.test_browser_two_profile_rust_transport` — 3 tests passed in 104.0s.
- Python comparison baseline: `python -m unittest test_browser_two_profile_python_evidence.BrowserTwoProfilePythonEvidenceTests.test_python_evidence_latency_and_memory_baseline` — 1 test passed in 8.5s.
- Rust evidence is in `.codex-stage5-6-rust-full/`; the focused run is in `.codex-stage5-6-rust-evidence/`.
- Python evidence is in `.codex-stage5-6-python-evidence/`.

The isolated Rust sample recorded 17.58 MiB before evidence and 18.90 MiB at peak. The Python sample recorded 39.0 MiB before evidence and 40.4 MiB at peak. Operation timings are retained in the JSONL artifacts; this is a single Windows comparison sample, not a release performance claim.

The Rust run covered PNG/JPEG screenshots, target-scoped Console retention and redaction, exact-host Network filtering and bounded body detail, broker-offline 500-request backpressure, reconnect, and two-profile cleanup. The offline extension queue remained bounded at 1000 events and below 5 MiB while reporting dropped events.

## Changes and gates

- Added migration-only `--enable-p1-observability` to the internal Rust broker test harness. The default internal broker feature set and production Chrome routing remain unchanged.
- Fixed the Python compatibility path to forward the broker-generated Console `capture_id` to the extension on start/stop; the Python unit suite now passes 79 tests.
- `cargo fmt --all -- --check`, `cargo check --workspace --exclude teshi-web --locked`, strict Clippy, and the full native Rust test suite pass. The first full test invocation had one non-reproducible Windows 10053 connection-abort; the targeted test and complete rerun passed.
- Extension protocol tests: 21 passed; Network capture tests: 19 passed.
- `openspec validate migrate-chrome-browser-broker-to-rust --strict` passes.

No production route switch, old-code deletion, or release action was performed. Continue with 6.1 only after rereading its policy/grant requirements and preserving the current Python production boundary.
