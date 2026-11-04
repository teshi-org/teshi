# Browser broker migration — stage 8.5 handoff

Stage 8.5 measurements are complete on Windows. Including the separately
completed startup migration task 7.1, OpenSpec progress is 43/47.
The measurements expose a locator performance gap; they do not establish that
all operations are faster or authorize production cutover.

## Reproduction and provenance

```powershell
$env:TESHI_CLI=(Resolve-Path .acceptance-84/msi-root-final/bin/teshi.exe).Path
$env:TESHI_RUST_TRANSPORT_PYTHON_FREE='1'
.venv/Scripts/python.exe -B resources/tests/benchmark_browser_runtime.py `
  --output .acceptance-85/comparison-ready --rounds 2 --samples 10 --burst 500 `
  --recorded-python-baseline .codex-stage5-6-python-evidence/two-profile-p0.jsonl
```

The command passed, exit code 0. For another run, choose a new output directory;
the runner refuses to overwrite evidence. The canonical local results are
`.acceptance-85/comparison-ready/measurements.json`, its four Profile diagnostic
directories, and `.acceptance-85/comparison-ready.log`.

- Windows 11 build 28120; Python 3.13.11, websockets 17.0.1, Playwright 1.62.0,
  Chromium 151.0.7922.34.
- Rust uses the stage 8.4 stable release CLI, version 0.7.10, built from
  `cd10776a2b2fccd6561efc2a60393d9407e0e0c4`; SHA-256
  `250cae4719b7823c1cf7a5979749b7413acb6dca8d4189120e7db383a50b2157`.
  Do not substitute the current `target/release` binaries without checking their
  channel: the stage 8.4 Nightly build used that directory afterward.
- The driver records hashes of itself, the extension background script, and the
  Python service. The benchmark source hash is
  `62cbd62b416e00abf4ad063ddba932c2ff20f07edc97ed94cb9641f2e047e315`.
- Four independent temporary two-Profile fixtures ran in Python/Rust, then
  Rust/Python order. Each implementation has six startup samples, twenty samples
  per operation after two warmups per fixture, and two Network bursts.

Both implementations receive identical direct WebSocket payloads and use the
same synthetic local page server and copied extension. Operation timing includes
connect, the correlated response, and close; response and close measurements are
also recorded separately. Screenshot timing covers capture, transport, artifact
write, and the metadata response, rather than pure socket bandwidth. PNG/JPEG
magic and nonempty payloads, locator identity, fill results, click counts, and
assertions are checked. Navigation is measured separately and waits for published
target readiness before the next operation. Failed actions are never retried.

Startup runs a new broker through its ready announcement and empty-session
WebSocket preflight. OS file caches are warm; Rust reuses only the fixture's
private state. This measures internal process startup, not the ordinary per-user
launcher or cold boot. Rust children have no Python executables on PATH. The
Python broker uses the venv driver's base interpreter and library paths: on
Windows, sampling the venv executable's redirector would measure the wrong
process. The recorded endpoint PID must equal the owned process PID.

## Results

Times are milliseconds, median / nearest-rank p95. Twenty operation samples and
six startup samples are too few for population-level confidence claims.

| Measurement | Python | Rust |
| --- | ---: | ---: |
| Startup | 173.889 / 261.975 | 66.090 / 551.204 |
| Snapshot | 18.696 / 23.959 | 10.340 / 11.586 |
| Locator acquisition | 26.172 / 30.126 | 628.519 / 942.305 |
| Fill | 10.255 / 12.885 | 3.829 / 4.145 |
| Assert | 9.755 / 11.237 | 3.603 / 4.111 |
| Click | 9.002 / 9.882 | 3.112 / 3.629 |
| PNG screenshot | 42.517 / 49.671 | 43.480 / 54.738 |
| JPEG full-page screenshot, quality 75 | 34.494 / 42.379 | 32.273 / 38.510 |
| Navigate | 102.438 / 109.237 | 94.428 / 99.347 |

Working-set memory is sampled every 20 ms from the held owned process handle,
excluding Chromium and the driver. Two seconds of idle samples establish steady
RSS. The OS process peak includes allocations before sampling began.

| Broker memory, median of two fixtures | Python | Rust |
| --- | ---: | ---: |
| Steady working set | 38.15 MiB | 11.97 MiB |
| OS peak working set | 42.96 MiB | 15.82 MiB |
| Sampled peak private bytes, individual fixtures | 35.31 / 34.40 MiB | 7.22 / 7.52 MiB |

Rust locator acquisition is slower in both fixtures. Its response-only median
is 628.236 ms, so the 0.287 ms median connection close does not explain the gap.
Source inspection shows `state.rs` queues the second-stage locator verification
with `session.queue_command`, clears `pending.stream_generation`, and records a
fallback generation. Heartbeat delivery is a likely explanation, not a traced
causal proof. Investigate direct second-stage dispatch and its generation/race
contracts before making locator performance claims. No runtime change was made
in this measurement stage. Rust's startup p95 also exceeds Python's; its maximum
551.204 ms is preserved rather than discarded.

## High-volume Network behavior

Each fixture completed 500 local POST requests in waves of 25 concurrent fetches,
with 32 KiB synthetic responses. Both brokers retained at most 128 requests and
128 KiB; list responses did not include request bodies. Settling includes a
one-second quiet period with a cleared queue and a contiguous acknowledgement.

| Fixture | HTTP complete | Delivery settled | Acked events | Dropped events | Duplicate events | Retained bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Python 1 | 496.322 ms | 5671.825 ms | 1028 | 472 | 1681 | 106684 |
| Rust 1 | 510.282 ms | 1522.028 ms | 1500 | 0 | 0 | 108800 |
| Rust 2 | 492.024 ms | 1521.641 ms | 1500 | 0 | 0 | 109056 |
| Python 2 | 505.654 ms | 5629.471 ms | 1026 | 474 | 809 | 106724 |

All four ended with zero queued events/bytes, pending sequences, and rejected
events; each broker's acknowledgement matched its extension's barrier. Rust
reported no gaps or loss. Python reported saturation loss and deduplicated
resends. HTTP completion is not evidence that every capture event was retained;
retention eviction and transport loss are different measurements. This is one
bounded workload, not a general throughput limit. Broker-offline queue bounds
and reconnect remain supported by the separate stage 5.6/8.2 acceptance evidence.

## Historical baseline and validation boundary

The runner imports the recorded 2026-09-28 stage 5.6 Python baseline and its file
hash into the report. It recorded 39.0 MiB before evidence and a sampled 40.4 MiB
peak. Its one-shot screenshot timings were 64.9 ms PNG / 252.1 ms JPEG, including
CLI subprocess overhead and concurrent Profile work. It did not measure startup
or locator acquisition. Preserve those values as historical context; do not form
direct speed ratios against the new direct-WebSocket samples. The old Rust
artifact also used another build and protocol implementation.

Exploratory failures are retained in `.acceptance-85/probe*`, `comparison`,
`network-diagnostic`, and `comparison-final`. They found the nested Python lease
token, its CSS action contract, the need for explicit async fixture cleanup,
bounded-queue loss invalidating a last-request retention criterion, and target
revision readiness after navigation. These runs are excluded from the table.
The first probe's leftover Python processes were matched to its unique temporary
root before termination; subsequent runs use the fixture's owned cleanup.

The successful run changed no broker implementation, production selection,
existing user Profile, or endpoint. AST parsing, strict OpenSpec validation,
whitespace checks, evidence/provenance checks, scoped credential scanning, and
owned-process cleanup verification pass. Native Rust and WASM gates were not
rerun because this stage changes only the opt-in benchmark, this report, and the
task status; their stage 8.1 evidence remains separate. Stage 8.4 is still partial.
These measurements do not validate production startup selection or Python
removal. Task 7.1 was completed separately in `e08b6db`; task 8.7 remains unchecked.
