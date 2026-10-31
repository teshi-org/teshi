# CI test infrastructure

The terminal suite builds once per platform, runs isolated single-threaded
acceptance shards, and verifies their actual execution before the existing
required check can pass. Product startup, discovery and screenshots remain real.

## Successful scheduling baseline

[CI run 41166978257](https://github.com/teshi-org/teshi/actions/runs/41166978257)
passed for `5e62a50cac82d6909051ef8e86517030bf98dcd2` on October 31, 2026.
The complete workflow elapsed time was 16m26s.

| Step | Windows | Linux |
| --- | ---: | ---: |
| Directory Feature replay, including runner build | 368s | 340s |
| Actual terminal regressions | 405s | 369s |
| PTY backend and protocol tests, including compilation | 78s | 73s |
| Build CLI | 51s | 26s |
| Restore Cargo cache | 55s | 31s |

The terminal binary boundaries in `.github/terminal-partitions.json` use the
Windows run's observed binary times: approximately 184s in each group.
These are initial scheduling weights, not predicted timings after optimization.

## Build, shards and platform gates

1. `tui-build` compiles the CLI, runner tests and Windows VT fixture once. It
   obtains executable identities from Cargo JSON, lists every compiled test and
   its ignored status, and runs all non-ignored tests and Rustdoc tests.
2. Directory replay, terminal A and terminal B run on separate runners. Terminal
   shards execute prebuilt binaries with one test thread and perform no Cargo
   compilation or Cargo cache restore/save.
3. The existing `Terminal interface tests (<platform>)` checks are final gates.
   They run even when dependencies fail, and require successful preparation and
   every shard, exact inventories, complete replay evidence and actual files.
4. Nightly continues consuming the success of the entire CI workflow for the
   exact source SHA. Skipped, cancelled, missing and failed prerequisites cannot
   unlock the platform gate.

Ownership checks require every integration-test module to occur exactly once.
Compiled inventories additionally cover platform conditionals, unit-test targets,
test names and ignored acceptance cases. A newly ignored unit/bin test with no
owner fails preparation. Non-ignored Rustdoc cases remain in the build job;
an ignored Rustdoc case cannot silently count as successful execution.

Each execution report binds the source SHA, platform, partition digest and full
compiled-inventory digest. The inventory includes executable hashes and relevant
runner source, fixture, Feature and harness hashes. Changed source or binaries
require a new preparation and execution. The gate compares exact per-binary
test-name multisets and results; an exit code of zero alone is insufficient.
After owned-process cleanup, independent test binaries continue after a test
failure so their success/failure evidence is retained. Failed cases are never
retried, and any failure still rejects the entire shard and platform gate.

The one authenticated CodeBuddy test is explicitly cataloged as manual external
acceptance. Hosted CI excludes it from execution and records `not-run` with its
reason; it does not count as a pass. Its direct invocation now fails when
`TESHI_CODEBUDDY_LIVE=1` is absent, instead of returning a false pass. This changes
the reporting of an already inactive opt-in test; required local terminal
acceptance remains in CI. Run that exact ignored test separately only with
explicit provider authorization and authentication.

Directory expectations come independently from the Rust runner's
`--inventory <feature>`, using Teshi's actual parser. The gate matches every
scenario, Examples row, substituted source step, start/end event and screenshot.
Missing, duplicate, failed or unfinished rows fail. Captures are copied into the
uploaded artifact as well as the application's replay store. The gate requires
nonempty PNG/SVG/text files and validates PNG/SVG signatures.

## Immutable distribution and preparation caches

`terminal-binaries.tar.gz` preserves executable modes and the original checkout
layout, including Cargo's embedded fixture and `CARGO_BIN_EXE` paths. Restoration
requires the same checkout root, source and platform. Archive members, paths and
hashes are verified before overwriting artifacts; restored files are verified
again. Arbitrary paths, links, changed payloads and stale inventories fail closed.

Only the platform build job restores and saves the existing Cargo cache.
Packaging uses gzip level 1, and artifact upload disables a second compression
pass. Build-command, total preparation, package size, packaging time and restore
time remain separate measurements. Compare these with the next run's avoided
compilation before changing cache policies further; hosted transfer time still
needs actual CI evidence.

The directory binding caches one executable hash per Python process, keyed by
resolved path, device/inode, size, modification time and change time. Changed
input invalidates it; changes during hashing fail. Font preparation is shared
inside one renderer process and resets with that process. No directory discovery,
startup, viewport, SVG, pixels, or test outcome is cached.

## Screenshot costs and the async decision

`preview::render_svg` is shared by direct frame capture and scenario evidence.
It records font setup, SVG read/parse, rasterization, PNG encoding and persistence
separately when `TESHI_CAPTURE_TIMINGS` is set.

Only image/font dependencies in the isolated runner's development/test profile
are optimized. Product workspace profiles and test logic remain unchanged.
`teshi-preview-bench <source.svg> <output-directory>` measures three conversions
of the same immutable SVG.

A local 1778x1024 captured frame took about 2.19s per warm conversion before the
dependency optimization and 0.37s afterward. All output pixels matched exactly.
The two dependency-rebuild increments took about 16s each locally. These
observations establish the local conversion benefit, not hosted CI speedups.

PNG encoding then took about 0.094s per frame. It is a small part of complete
replay time; the measured dominant rasterization cost was reduced without a
queue or concurrency change. Async screenshot architecture is therefore deferred
until new full-run measurements show a material remaining bottleneck.

## Conditions and timeout layers

`Terminal::expect_screen` waits for an explicit viewport predicate. Management
pointer operations wait for their actual visible/closed modal or selected row
before their after-action capture. Field clicks wait for the focus marker.
Management fixtures also opt into an atomic completed-frame receipt published
after the product's real draw/flush. Clicks require the owned process's completed
frame and the PTY viewport to agree on the target coordinate and dimensions.
This closes the partial-frame/layout race without sleeping or repeating input;
it is a render receipt, not a claim that unrelated background work has finished.
Receipts are retained alongside management screenshots, including failed waits.
Chat focus waits for Send/Focus controls, profile switching waits for the active
Agent title, and test-point filters wait for the requested filter state.
The registry fetch waits for its business result without repeating the click.

CI self-test launches enforce explicit application, session and ACP-store
overrides. Fixtures own these stores independently of Windows known-folder
lookups; overriding APPDATA/LOCALAPPDATA alone does not isolate every store.
Project-switch fixtures select their intended native backend explicitly.

Evidence helpers do not establish application readiness. Keep waits at the
operation's semantic boundary; this is not a general idle/layout barrier.
Polling intervals and negative observation windows are intentional. The existing
Escape settling delay is a terminal byte-stream protocol requirement.

Directory host writes, flushes and reads run on one I/O worker. Requests use the
minimum of operation, viewport-wait, source-step and Examples-row deadlines.
Timeout/EOF/malformed replies poison the stream, preventing late-reply reuse and
automatic mutation retries.

| Budget | Default |
| --- | ---: |
| `TESHI_DIRECTORY_REQUEST_TIMEOUT_SECONDS` | 30s |
| Viewport wait, shared with each host request | 15s |
| `TESHI_DIRECTORY_STEP_TIMEOUT_SECONDS` | 60s |
| `TESHI_DIRECTORY_CASE_TIMEOUT_SECONDS`, per Examples row | 180s |
| Graceful directory cleanup protocol | 10s total |
| Supervised backend command / CI step | 540s / 10min |
| Supervised shard command / CI step | 1200s / 22min |

Step/row deadlines cover host I/O and polling; outer process/CI limits cover
arbitrary synchronous filesystem calls. Windows uses owned Job Objects and waits
for zero active processes during termination. POSIX uses fresh groups; Linux
also observes and kills identified descendants that created separate sessions
before terminating their parent. No process-name cleanup is used.

Diagnostics retain the original failure, case/step/request, stderr, recent
operations, host/worker state and last successful viewport/capture references.
They never request another frame through a poisoned pipe.

## Measurements and failure evidence

`scripts/run-ci-command.py` writes command output, elapsed/cleanup times, JUnit
and slow-test rankings. Timeout returns 124; startup failure returns 125.
Serial libtest times are observed pipe intervals including teardown, not CPU
time or a substitute for the compiled inventory.

Directory evidence checkpoints each Examples row, including failed steps and
cleanup failures. Execution/wait/capture/cleanup phase totals are exclusive;
nested request durations must not be added again. Each step still gets a fresh,
synchronous capture. Required capture failure fails acceptance.

The manually dispatched `Windows test temp experiment` checks out an exact SHA
in both modes. Default preserves TEMP/TMP; runner mode uses a unique task
directory under RUNNER_TEMP before child processes start. It records original
paths, fresh Python/Rust temp probes, drive metadata and runner image identity.
Drive metadata does not prove physical backing.

Each mode builds outside the measurement window, then runs three sequential
repeats of full directory replay, all required management pointer tests and
terminal backend tests. Mutable fixtures and evidence stay independent.
The summary rejects mismatched SHAs/images or any missing, failed, timed-out or
cleanup-failed sample before reporting medians and ratios.

Hosted Windows temporary-storage and artifact-transfer benefits remain unproven
until those workflows run. Local path probes and local conversion/package
measurements do not replace hosted CI or external-provider acceptance.
