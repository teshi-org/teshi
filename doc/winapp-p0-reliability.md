# WinApp P0 reliability implementation

Scope: requirement documents doc-16 through doc-19. Doc-1 is the audit index,
not an additional implementation requirement. Baseline: 1bf7eed plus the
existing uncommitted replay-evidence work.

## Contracts and implementation order

1. Bound connect, handshake, send, response and native execution. Transport
   errors carry a stage and stable code. A separate owned worker process hosts
   UIA and capture; its supervisor remains responsive to health and cancellation.
   Terminate only that worker on deadline/cancellation. Never replay an uncertain
   mutation. Recovery must verify the saved native target identity.
2. Probe the authenticated protocol, report stale discovery, authentication,
   blocked execution and invalid targets separately. Rediscover service endpoints
   safely; target loss requires explicit attachment rather than guessing.
3. Build replay plans from Feature steps, Background and expanded Examples.
   Reject missing bindings before effects. Manual cases are explicit, partial
   ranges report omitted work, and neither constitutes a full pass.
4. Persist action responses before screenshots. Every actual run has an isolated
   ID, incremental evidence and one terminal outcome independent of capture.
   Screenshots are optional by default; capture failures remain structured.
   Dry-run produces a plan only. Abrupt process termination retains prior events;
   it must never be interpreted as a successful run.

## Validation checklist

- [x] Transport fault injection: stalled handshake and response, bounded exit.
- [x] Worker timeout/cancellation: health responsiveness, owned-process cleanup,
      recovery, unchanged mutation count and target identity checks.
- [x] Protocol: incorrect token, stale endpoint, restarted service, invalid target.
- [x] Replay: missing Given/When/Then, no bindings, manual, until-line,
      repeated Background and expanded Examples.
- [x] Evidence: all/partial capture failures, first/middle action failure,
      dry-run, unique IDs, readable retained indices and consistent terminals.
- [x] Focused Rust/Python tests and formatting/lint checks.
- [x] Real native-window success/failure replay, or an explicit blocked record.

## Delivery boundaries

Preserve existing worktree changes. Do not commit, push or change requirement
delivery states without a verified result. Fault tests use temporary projects,
local fake servers and owned worker processes. Existing service instances and
user applications are not cleanup targets.


## Implemented behavior

- `sidecar.rs` applies one absolute deadline to DNS, TCP connect, WebSocket
  handshake, send and response reads. Socket timeouts have a stable
  `command_timeout` diagnostic and stage. WinApp CLI errors include structured
  stage/budget metadata and nonzero exits.
- `WinAppSupervisor` owns a disposable subprocess. UIA/capture work stays in
  that process, while authenticated health and cancellation stay responsive.
  Cancellation uses an operation-unique identity obtained from health. Worker
  timeout/exit never replays a mutation. A parent-death watcher exits only the
  worker, leaving application processes alone.
- Target recovery persists HWND, PID and process creation time per project.
  Failed target switches invalidate the old identity. Missing/changed targets
  require explicit attachment. Stale endpoints are rediscovered through the
  selected project's daemon; incompatible protocols and credentials fail closed.
- `winapp_plan.rs` validates every selected source step, including repeated
  Background and expanded Outline rows. Missing/stale bindings and manual cases
  reject execution before effects. `--until-line` reports omitted steps and a
  `partial` terminal outcome. Dry-run emits only a plan.
- Every executed run has a UUID and an isolated evidence directory. Action
  responses are saved before capture, screenshot failures are optional and
  explicit, and both success/failure paths emit `replay_complete`. The screenshot
  index agrees with the run outcome. An OS ownership lock allows readers to
  reconcile a dead writer as interrupted without relying on reusable PIDs.

### Time budgets and cancellation

The transport budget covers all connection/IO phases in one deadline. CLI health
uses 2 seconds. Native requests use their `timeout_ms`, including worker startup
and target restoration, with at most 1 second allowed for worker reap. Existing
execute CLI transport budgeting remains `ceil(timeout_ms / 1000) + 5` seconds;
health probing and a needed daemon/runtime startup are separately bounded setup
phases. This is not a single whole-replay timeout.

`teshi winapp health` exposes the active operation identity. Cancel it with
`teshi winapp cancel --request-id <active_request>`. A successful cancellation
means the executor was stopped, not that application-side effects were undone.
No physical input permission is added by health, recovery, bindings or `--yes`.

## Validation on 2026-10-23

| Gate | Result |
| --- | --- |
| `cargo test -p teshi-engine --lib --locked` | 265 passed |
| `cargo test -p teshi-tui --lib --locked cli:: -- --test-threads=1` | 108 passed |
| Python `test_winapp_*.py` with the freshly built CLI | 119 passed |
| `cargo clippy -p teshi-engine -p teshi-tui --all-targets --locked -- -D warnings` | Passed |
| `cargo fmt --all --check` and `git diff --check` | Passed |
| Release workflow contract and release package inventory tests | Passed; 11 package tests |
| Owned WinForms fixture through CLI, daemon and managed v5 runtime | 18 checks passed |

The fault tests inject stalled handshakes, stalled responses, frame traffic,
blocked executor work, cancellation and a mutation followed by a hang. They
verify bounded results, responsive health, no duplicate mutation, scoped cleanup
and subsequent recovery. CLI fixtures cover missing Given/When/Then, manual and
empty bindings, partial ranges, Outline/Background execution and capture/action
failure combinations.

Live evidence is retained locally under:

- `.teshi/artifacts/winapp-p0-live-20261023-managed-02/report.json`: final managed
  runtime run; real successful and intentionally failing replay records remain
  in its `.teshi/logs/replay-screenshots/` directories. Also verifies a wrong
  credential, daemon/service restart and target exit.
- `.teshi/artifacts/winapp-p0-live-20261023-managed/report.json`: initial failure
  in the test launcher (daemon working directory differed from its project);
  retained, corrected in the launcher, then rerun in the directory above.
- `.teshi/artifacts/winapp-p0-validation-20261023/`: final Python/Rust logs and
  `identity.json` recording exact binary and source hashes.

Final validation binary: `.teshi/teshi-winapp-p0.exe`, SHA-256
`d5cf7532585e1725b3c50c460de90fe0a45b6c5f0ac1fc59a2f792f71b943873`.
The occupied `target/debug/teshi.exe` was not replaced or terminated.

## Runtime delivery

The daemon executes the managed runtime's service, so this protocol change
requires **WinApp runtime v5**. The runtime requirement, build script, release
asset paths and contract checks were updated together. V4 installations are
not reused as v5. Tests staged current service source with a copy of the
existing dependency binaries in a private directory and exercised the real
`TESHI_WINAPP_RUNTIME_DIR` path. Existing user runtimes were not modified.

No installer was built and no release/runtime was published. Production use
requires deploying the matching CLI and v5 runtime. These checks establish
Windows CLI/backend behavior using an owned WinForms target; they do not claim
full workspace, Linux/macOS, elevated UAC, GPUI UI, or arbitrary third-party
provider acceptance. Requirement store delivery states were not changed.
