# WinApp exploration contracts

Source implementation only: this delivery has not run tests, replay, PTY or real-window
acceptance. Inspect `teshi --version` and `teshi winapp --help` on the actual executable.
Do not interpret the following examples as evidence of a successful run.

## Owned instances

`teshi winapp instance --file request.json` accepts one JSON object:

```json
{"action":"launch","argv":["C:/fixtures/app.exe","--profile","C:/fixtures/profile-a"]}
```

Keep the returned opaque `instance` token. Subsequent requests use:

```json
{"action":"wait","instance":"<returned token>","wait_ms":5000,"expected_exit_code":0,"expected_windows":0}
```

`status` may assert `expected_alive`, `expected_windows`, and `expected_exit_code`.
`stop` terminates only the original owned process handle and waits up to `wait_ms`
(maximum 10 seconds). Window counts refer to top-level windows of that live instance.
Launch a second instance with the same profile, wait for its exit and inspect the
primary's token separately; use different profiles to check independent lifetimes.
No executable-name cleanup is performed. A supervisor/executor restart loses tokens;
`instance_unknown` requires a new explicit ownership decision, never a PID fallback.
This does not automatically adopt or kill child processes spawned by the application.

## Observation and physical input

```text
teshi winapp visual-plan --file observe.json
```

```json
{"steps":[{"action":"observe","scope":"client"}]}
```

`scope` is `window`, `client`, or `region`; region supplies
`"region":[x,y,width,height]` in physical client pixels. PNG artifacts report dimensions,
client origin, capture origin, DPI, capture backend/fallback, wall-clock time, window
bounds and process incarnation. `observation` is an opaque, executor-local token.
Coordinates are always relative to the client origin, even for a cropped screenshot.
Only the most recent observation is valid; recapture after movement, resize, DPI
changes, reattachment or executor restart. Capturing requires no child UIA selector.

```text
teshi winapp visual-plan --file actions.json --mode foreground --allow-interactive-input
```

```json
{"observation":"<returned token>","steps":[
  {"action":"click","x":120,"y":80},
  {"action":"text","value":"sample"},
  {"action":"scroll","x":200,"y":240,"delta":-120},
  {"action":"drag","x":200,"y":240,"to_x":300,"to_y":240}
]}
```

Only use coordinates observed for the authorized target. `double_click` uses the
same x/y contract. `keys` accepts the existing SendKeys syntax. `resize` takes
physical outer-window `width`/`height`; `maximize` and `restore` affect the target.
All mutations require both permission flags, verified target identity and foreground
activation. Auto/background never imply permission. A mutation may be uncertain
once dispatch begins; never automatically retry it.

Plans contain 1..32 sequential steps and have a 60-second supervisor deadline.
`wait_stable` accepts `wait_ms` (100..5000 ms) and requires consecutive identical
captures; stable does not mean the business result passed. Each completed mutation
attempts a fresh capture. A failed capture is reported separately from the original
`provider_completed` result, and stops further actions. Failures retain earlier
results and journals under `.teshi/artifacts/visual/`. The plan's business result
remains `unverified`; neither a journal nor successful injection creates a binding.
Review the journal before resuming; the API deliberately does not auto-resume a plan.

## External business checks

```text
teshi winapp external-check --file check.json
```

```json
{"argv":["python","check_reflection.py"],"check_timeout_ms":5000,
 "input":{"screenshot":"<captured PNG>","calibration":"<reviewed calibration>"}}
```

Use only an explicitly requested, trusted **read-only** checker. This is a local
process extension, not a sandbox or a replacement input executor. No shell command
string is evaluated. The checker receives one JSON stdin document:
`{"version":1,"input":...}`. It must emit exactly one JSON result with
`business_result` equal to `passed`, `failed`, `blocked`, or `unverified`; a pass
also requires `thresholds_verified: true` and process exit code zero. A declaration
of calibrated thresholds must be backed by separately retained calibration evidence;
Teshi cannot derive application-specific visual formulas from a screenshot.
Nonzero exit, malformed output and deadline expiry fail the CLI operation. Direct
checker execution is bounded to 30 seconds; the transport allows 45 seconds. Timeout
kills the direct checker process; checkers must not spawn unmanaged descendants.
Output/result evidence is stored under `.teshi/artifacts/checks/`.

Use separate checks for reflection color formulas, no-source fallback signals,
repeated-switch survival and cache-file contracts. Do not hard-code unverified
application thresholds in general UIA operations. Steam-specific and multi-DPI
acceptance remains pending; a controlled fixture pass cannot stand in for it.

## Capability routing

A snapshot reports total/returned interactive counts, traversal completeness and
UIA availability. `available` prefers semantic selectors; a complete container-only
tree recommends supported visual observation. Truncation/query failures are uncertain,
not proof of absence. No application-name heuristic or unverified CEF/CDP claim is used.
CLI process exit is a failure signal; `provider_completed`, capture success, and
`business_result: passed` are separate meanings. See the packaged Teshi and WinApp
Regression skills for human confirmation and binding boundaries.
