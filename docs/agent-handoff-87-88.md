# Chrome Rust migration: final local acceptance

## Scope and provenance

Base revision: `3f38d8a760da8a77d97a1ec98d97f23865e7e90a` plus the
stage 8.6/8.7 working-tree changes. Validation ran on Windows and native Linux
under WSL on 2026-10-13. The user explicitly deferred macOS build/archive/native
acceptance on this date; it is not a passing platform result. This exception
allows the remaining migration work to proceed without treating macOS as a
Python-removal prerequisite.

Local logs, artifacts, hashes, and the packaging reproduction driver are under
`target/acceptance-8788/`. Windows/Linux optimized binaries initially carry the
actual default development identity (`0.7.10`, channel `dev`, sequence 0, empty
source/timestamp fields). They are local validation builds, not published stable
releases. Each bundle manifest was generated from its compiled identity; all
shipped executables in each layout were compared. Nightly identity is validated
separately. No Git tag, commit, push, remote CI run, installation, or publication
was performed.

## Production removal and retained consumers

- Chrome has only the Rust launcher. The former exact `python` selector now fails
  before startup with guidance to unset it or use `rust`; invalid selectors also
  fail before launch. Rust errors propagate without fallback.
- `resources/browser_service.py` now exposes only Embedded mode. Chrome bridge
  classes, HTTP/discovery server, authenticated extension transport, TSH1 parsing,
  and the Chrome process entry point were removed from the shipped service.
- The old implementation is retained in
  `resources/tests/chrome_broker_oracle.py` for explicit differential tests and
  benchmarks. Test harnesses reference that path; it is never a production
  runtime selection or a packaged resource. Package inspection rejects it.
- `browser_agent_broker.py` remains a live Embedded dependency: `run_embedded`
  constructs `BrowserSessionBroker`, and Embedded commands use its session,
  lease, reference, locator, capture, and policy helpers. Deleting it would break
  Embedded. `update_participant.py`, API resources, and WinApp resources retain
  their existing consumers. Import closure staging preserves these dependencies.
- Legacy endpoint reading and differential fixtures remain compatible. Wire
  schema/protocol stay at version 1. Documentation and machine-readable runtime
  metadata describe the Rust-only Chrome selection.

## Validation and evidence

| Gate | Result / local evidence |
| --- | --- |
| Native workspace check, test, Clippy, docs | Passed; `check.log`, `test.log`, `clippy.log`, `doc.log` |
| Workspace tests | 1,085 passed, 2 ignored across 47 summaries, including doctests |
| Engine/TUI focused full suites | 223 engine unit + 3 integration; 339 TUI unit + 1 doctest passed; 1 real-broker test ignored |
| Real Chromium, two Profiles and two projects | 4 passed in 82.813 seconds; `chrome-final.log`, `chrome-final/` |
| Python Embedded/oracle HTTP and orchestration | 9 + 32 passed; shared helpers still exercised from the production Embedded module |
| Shared Python session/lease/locator tests | 47 passed |
| Real Embedded Playwright P0 actions | 2 passed |
| WinApp deterministic regression | 57 passed; no real window/UAC claim |
| Extension protocol/Network | 40 passed; `node.log` |
| Package negative/layout tests | 9 passed; stale metadata and accidental oracle shipping rejected |
| Update manifest tests, release/nightly workflow contracts | 5 manifest tests and both workflow checks passed |
| Installed browser-testing smoke | Passed against the rebuilt optimized Windows CLI copied to an isolated installed layout |
| GPUI WASM | Windows build and equivalent distribution smoke passed; `wasm.log`; hosted transport contract passed |
| Strict OpenSpec and whitespace checks | Passed |

Commands:

```text
cargo fmt --all --check
cargo check --workspace --exclude teshi-web --locked
cargo test --workspace --exclude teshi-web --locked
cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings
cargo doc --workspace --exclude teshi-web --locked --no-deps --document-private-items
cargo build --release --locked --bin teshi --bin teshi-update-helper --bin teshi-desktop
cargo test -p teshi-engine -p teshi-tui --locked
python -B -m unittest discover -s resources/tests -p 'test_browser_agent_broker.py' -q
python -B -m unittest discover -s resources/tests -p 'test_browser_service_http.py' -q
python -B -m unittest discover -s resources/tests -p 'test_browser_service_agent_flow.py' -q
python -B -m unittest discover -s resources/tests -p 'test_browser_p0_actions.py' -q
python -B -m unittest discover -s resources/tests -p 'test_winapp*.py' -q
python -B -m unittest resources.tests.test_browser_two_profile_rust_transport -q
python -B scripts/test-browser-agent-package.py
python -B scripts/test_release_package.py
python -B scripts/test_update_manifest.py
python -B scripts/test-release-workflow-contract.py
python -B scripts/test-nightly-workflow-contract.py
python -B scripts/test-hosted-ui-transport.py
node --test extension/teshi-bridge/tests/protocol.test.mjs extension/teshi-bridge/tests/network-capture.test.mjs
./scripts/build-teshi-web.ps1
openspec validate migrate-chrome-browser-broker-to-rust --strict
git diff --check
```

Real Chromium used `TESHI_CLI=target/release/teshi.exe`,
`TESHI_RUST_TRANSPORT_PYTHON_FREE=1`, and a dedicated artifact directory. The
driver uses Python/Playwright; the broker child has no Python on PATH and no
project venv. Temporary extension copies, private broker state, and ephemeral
ports isolate the tests from real user pairing and browser sessions.

The first real Chromium run had 3 passes and one readiness race: heartbeat
reported ready before stream hello populated `stream_generation`. The test now
uses its existing bounded stream-generation observation for both target IDs.
No browser mutation is retried and no assertion is relaxed. The full four-test
rerun passed. The original failure remains in `chrome.log` and `chrome/`.

Rustdoc completed with four existing warnings (unresolved links and a bare URL).
The WASM toolchain reports the existing `proc-macro-error2` future incompatibility
notice. MSI ICE validation reports the existing same-version upgrade warning
ICE61. These are disclosed command output, not waived test or Clippy failures.

## Packaging

| Artifact | Inspection |
| --- | --- |
| Windows MSI | Rebuilt with the normal script and ICE validation; cabinet decompiled to 53 files, including 51 manifest-listed files, the manifest, and license; all listed size/hash values verified |
| Windows EXE | Rebuilt with Inno; all 52 staged files, compiled identities, import closure, bundle hashes, and Python-free broker checked before compilation |
| Windows portable ZIP | Built, CRC checked, extracted, and all 52 files inspected; compiled identities and Python-free broker checked from extraction |
| Linux tar.gz | Built natively in Rocky WSL; 44 staged/extracted files inspected; executable bits, hashes, compiled identities, and Python-free broker checked in Ubuntu WSL |
| Browser-testing ZIP | Built, CRC checked, extracted, and 28 files inspected; only the three live Embedded Python resources retained |
| Windows nightly EXE | CLI/helper rebuilt with matching injected nightly identity; 45-file CLI-only staging, native broker probe, nightly artifact contract, and Inno compilation passed |
| macOS | Explicitly deferred by the user; no build/archive/native acceptance claimed |

The Linux build used `/home/love/.cargo/bin/cargo` as user `love` in Rocky with
`CARGO_TARGET_DIR=target/acceptance-84-linux`:

```text
cargo build --release --locked --bin teshi --bin teshi-update-helper
```

The local nightly CLI/helper build used `TESHI_BUILD_CHANNEL=nightly`,
`TESHI_BUILD_DATE=20261013`, `TESHI_GIT_SHA=3f38d8a760da8a77d97a1ec98d97f23865e7e90a`,
`TESHI_BUILD_TIMESTAMP=2026-10-13T09:40:00Z`, and `TESHI_BUILD_SEQUENCE=1`, with the
same optimized CLI/helper command. The compiled identity reports those exact
values. This is the base SHA plus uncommitted migration changes, not a claim that
the candidate tree equals that commit. The installer is
`nightly-inno/teshi-v0.7.10-nightly.20261013.3f38d8a-x64-setup.exe`; no matching
Git tag or published nightly was created. `target/release` now contains this
nightly CLI/helper, while the tested development-identity binaries are retained
in `msi-root/bin` and the portable/EXE staging roots.

```text
python -B scripts/test-nightly-artifact.py --root target/acceptance-8788/nightly-root
./scripts/build-exe-setup.ps1 -StagingRoot target/acceptance-8788/nightly-root -Tag v0.7.10-nightly.20261013.3f38d8a -Version 0.7.10 -CliOnly -OutputDir target/acceptance-8788/nightly-inno
```

The first packaging-driver invocation in Rocky failed because Python 3.9 lacks
`tomllib`; the unchanged driver subsequently passed in Ubuntu with Python 3.14.
This is a packaging interpreter limitation, not a failed Linux binary test.

Windows MSI reproduction:

```text
./scripts/build-msi.ps1 -SkipBuild -SkipWebBuild -StagingRoot target/acceptance-8788/msi-root -OutputDir target/acceptance-8788/wix
./scripts/build-exe-setup.ps1 -StagingRoot target/acceptance-8788/exe-root -Tag v0.7.10-local-validation -Version 0.7.10 -OutputDir target/acceptance-8788/inno
python -B target/acceptance-8788/package_payloads.py x86_64-pc-windows-msvc
python3 -B target/acceptance-8788/package_payloads.py x86_64-unknown-linux-gnu
```

EXE validation covers compiler inputs and successful compilation, not EXE payload
extraction or installed-machine upgrade behavior. The unchanged WASM frontend was
rebuilt after the MSI staging run; portable/EXE staging uses the rebuilt bundle.

## Invariants, recovery, and performance

Chrome launch never checks Python or a project venv. Profile/window/tab routing,
project/caller leases, origin pairing, private credentials, authenticated broker
identity, and bounded capture behavior remain covered by the native/security
and real Chromium tests. An unrelated listener is left untouched. A broker
restart creates a new generation; old leases and responses are rejected and the
extension reconnects. A failed action is never silently replayed. Embedded and
WinApp continue to use their own runtimes and error guidance.

The [stage 8.5 report](agent-handoff-85.md) remains the recorded performance
measurement: Rust uses less retained working-set memory and has faster medians
for several commands, but locator acquisition was substantially slower, and
startup p95 was worse in that small sample. No new speedup claim is made here;
this removal changes no Rust locator implementation. The test-only Python oracle
remains available to reproduce comparative measurements.

Platform boundaries not exercised here: macOS (explicitly deferred), real native
UI/UAC, installed-machine updates, remote CI/publication, and Linux GUI behavior.
The real Chrome tests and WASM build/static smoke are separate from visual UI
acceptance. Local artifact and native process checks do not imply a published or
installed release.
