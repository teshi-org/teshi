# Replay screenshot review

The shared GPUI Run surface lists replay history. Select a run, then a screenshot
event to inspect its image, source step, scenario, outcome, and capture time.
Use **Enlarge image** to open a larger preview and **Close enlarged preview**
to return to the same step. **Refresh** reloads scenarios and run history.

New runs store evidence under the tested project's
`.teshi/logs/replay-screenshots/<run-id>/`. `run.json` is published atomically
after each event; images remain associated with their original run and step.
CLI browser/WinApp replay also maintains an incremental `index.json` inside
that run directory. A failed assertion retains its capture and earlier images.
Capture failure is distinct from the step's execution outcome.

Existing screenshots in the old shared directory are exposed through a
read-only legacy history entry. Their original files are not moved or rewritten.
Legacy indexes do not contain per-step outcomes, so those outcomes remain unknown.

GPUI starts its step walker on a worker thread and polls persisted events while
the run is active. Embedded browser and WinApp sidecars supply viewport captures.
Chrome CLI replay retains its explicit target, lease, and broker capability
checks. A Chrome run started through the GPUI step walker has no explicit target
or lease; screenshot capture is reported as unsupported rather than choosing a
tab implicitly. Targeted Chrome CLI captures can still be reviewed in GPUI.

The hosted UI reads only images associated with the selected project's run.
Run IDs, filenames, canonical containment, private Feature paths, image sizes,
and PNG/JPEG decoding are checked by the host. Project-change notifications
clear the previous selection and reload history; late image responses are ignored.

## Acceptance status

The paired Features are `features/en-US/replay_screenshot_review.feature` and
`features/zh-CN/回放截图查看.feature`. Their ten scenarios per locale remain
pending live UI binding and replay. There is currently no ready browser session
for verified locator acquisition. No confirmed bindings have been fabricated.
Their headers say live bindings remain pending, so the catalog keeps the
`@web-ui` owner and does not fail the release gate for the missing sidecars.
Live replay is still outstanding.

The 2026-10-22 replay attempt used an independently built CLI from this checkout:
all 20 localized scenarios failed at unbound steps. Results and binary/source
hashes are in `.teshi/artifacts/replay-screenshot-review-20261022/verification.json`
and `feature-results/matrix.json` below that directory. Native Clippy, 262 engine
tests, 53 daemon tests, 5 screenshot CLI tests, 10 browser/WinApp CLI tests, and
15 shared GPUI tests passed. The WASM release build and an isolated Windows
equivalent of the web distribution smoke checks also passed; its generated
assets are in `web-dist/` below the same artifact directory.

Engine tests cover incremental publication before run completion, a failed UI
assertion followed by a real WebSocket image response from a fixture sidecar,
run/project isolation, reopening, corrupt/missing files, and legacy compatibility.
Daemon integration tests exercise history/snapshot control requests and private
Feature denial. CLI tests cover retaining earlier images after a later failure.
These checks do not establish rendered GPUI behavior or a passed Feature replay.

## Local screenshot acceptance dock

Open `http://127.0.0.1:20253/?e2e=1&replay_review=1` against a source-built
diagnostic distribution. This explicitly selects an isolated screenshot fixture
backend; ordinary `?e2e=1` retains the existing API fixture and production pages
continue to use the daemon. The screenshot fixture provides successful, active,
failed, unsupported, missing-image, capture-failure, and two-project conditions.
It retains runs while its project is closed/reopened within the harness.

The dock exposes fixture setup controls plus history selection, step selection,
and image enlargement. Those actions invoke the same `ApiRunView` methods as
the rendered controls. Status nodes reflect actual view state, including whether
an image loaded, its selected step/run, step outcome, and enlargement state.
They do not independently calculate a passing UI result. Fixture data does not
prove disk persistence or a real native capture; engine tests cover those
boundaries separately. Bindings still require verification against a rendered UI.

The 2026-10-22 continuation started a separate embedded browser and local daemon
in `.teshi/artifacts/replay-review-acceptance-20261022/project`. The browser
reported `webgpu found no adapters` and the page displayed the GPUI initialization
error. A second isolated attempt with additional SwiftShader/Vulkan flags had
the same result. The flags were compared with Chromium's
[WebGPU test configuration](https://chromium.googlesource.com/chromium/src/+/HEAD/third_party/blink/web_tests/FlagSpecificConfig);
production browser flags were not changed.

The original 20 unbound failures remain intact. The continuation matrix under
`replay-review-acceptance-20261022/feature-results-gpu-probe/matrix.json` records
all 20 scenarios as blocked by WebGPU initialization, with actual browser
snapshots attached. The missing verified bindings remain a separate catalog
failure. Neither result is a passed screenshot acceptance run.

The continuation also tried the full Chromium channel described in
[Playwright's browser documentation](https://playwright.dev/python/docs/browsers),
with the same adapter failure. All experimental launches used the isolated test
project. Test-owned browser/sidecar processes were cleaned up afterward.

The latest GPUI test suite has 21 passing tests, including four tests that drive
the real Run view through GPUI's test context. Drawing is explicitly disabled:
these tests verify asynchronous image loading, selected-step preservation after
enlargement, missing/unsupported captures, history restoration, and rejection
of a late image response after project switching. They are not GPU/render tests.

The current WASM release and generated distribution passed their build/smoke
checks. Its actual browser attempt is retained in
`replay-review-acceptance-20261022/feature-results-current-ui/matrix.json`;
`initialization-failure.jpg` in that same artifact root shows the failed GPUI
startup screen. `gpui-tests.log` records the 21-test result. The continuation's
`verification.json` pins the current source hashes and generated WASM hash.
