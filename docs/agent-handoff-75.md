# Browser broker migration — stage 7.5 regression

Validated on Windows on 2026-10-04, starting from `dev` commit `2da6f93`.
Stage 7.5 is complete; this report does not establish the stage-8 acceptance gates.

## Environment repair

The project virtual environment runs Python 3.13.11 but contained Python 3.14
native wheels for Pillow 12.3.0, NumPy 2.5.2, and websockets 17.0.1. Reinstalled
the same package versions with Python 3.13 wheels. Package versions, project
requirements, and the managed WinApp runtime were not changed. Previous files
are retained under `.acceptance-75-81/dependency-backup/`.

`pip check` passes. Pillow, NumPy, OpenCV, and WindowsCapture import successfully.

## Regression evidence

Commands use `.venv/Scripts/python.exe -B`:

| Command | Result | Evidence |
| --- | --- | --- |
| `-m unittest discover -s resources/tests -p 'test_winapp*.py' -q` | 57 passed | `.acceptance-75-81/winapp-regression-final.log` |
| `-m unittest resources.tests.test_browser_p0_actions -q` | 2 passed | `.acceptance-75-81/embedded-regression-final.log` |
| `-m unittest discover -s resources/tests -p 'test_browser_agent_broker.py' -q` | 47 passed | `.acceptance-75-81/python-broker.log` |
| `-m unittest discover -s resources/tests -p 'test_browser_service*.py' -q` | 41 passed | `.acceptance-75-81/python-service.log` |

Embedded tests launch isolated real Chromium and verify DOM/pointer input,
fill, select, key input, waits, and structured negative assertions. WinApp
tests use mocks or a fake WebSocket sidecar: they verify input policy, integrity
checks, lossless capture, visual assertions, CLI JSON, and replay contracts.
They do not establish manual UIA, WGC, or UAC acceptance against a real window.

With Python absent from the child PATH and no project virtual environment,
`teshi browser serve-embedded --project <isolated-project>` exits with the
expected Python-environment error and requirements-install hint. Evidence:
`.acceptance-75-81/embedded-no-python-isolated.log`. The configured Embedded
runtime remains functional in the real Chromium regression above.

## Stage-8 preparation

- Extension protocol/Network tests: 40 passed.
- Release, Nightly, and hosted UI transport contracts pass.
- The initial native workspace run failed two daemon-spawn tests because the
  sandbox cannot write the default application-data migration marker. Using
  the supported `TESHI_APP_DATA_DIR` override in an isolated directory makes
  both tests pass. Evidence: `.acceptance-75-81/daemon-spawn-isolated.log`.
- The complete native workspace test command passes outside the sandbox with
  isolated application data: `cargo test --offline --workspace --exclude
  teshi-web --locked -- --test-threads=1`. It reports 1,075 passed and 2 ignored
  across 47 suite results, including doctests. The sandboxed attempt failed
  Windows root-certificate-store access; the same TLS tests pass outside it.
  Evidence: `.acceptance-75-81/native-workspace-test-unsandboxed.log`.
- The ignored tests are the durable-paired real Chrome model/browser round trip
  and an optional live GitHub TLS probe. The separately gated
  `live_chrome_agent_broker_identity` test returns without live acceptance unless
  `TESHI_RUN_LIVE_CHROME_AGENT_E2E=1`; that gate was not enabled in this run.
- Python/Rust shared state differential: 1 passed. Evidence:
  `.acceptance-75-81/shared-differential.log`.
- `bash scripts/run-web-ui-smoke.sh` passes with Git Bash outside the sandbox.
  The existing WASM distribution was backed up before rebuilding. Evidence:
  `.acceptance-75-81/web-ui-smoke-unsandboxed.log`.
- The existing isolated Rust/real-Chromium two-Profile suite passes all 3 tests
  in 65.006 seconds, including locator contexts, evidence/backpressure, and
  reconnect/isolation. Cleanup completed for test-created resources. Evidence:
  `.acceptance-75-81/real-chrome-regression.log` and
  `.acceptance-75-81/real-chrome/two-profile-rust-transport.jsonl`.
- An isolated copy of the internal Rust broker starts in 0.877 seconds with
  Python, pip, and uv absent from its PATH and no project virtual environment.
  Embedded still reports its expected missing-Python guidance in that same
  environment. This is an internal-startup probe, not default-launcher or real
  extension-registration acceptance. Evidence:
  `.acceptance-75-81/python-free-result.json`.

## Remaining boundaries

OpenSpec progress is 38/47. Stages 8.1, 8.2, and 8.3 remain unchecked:

- Full live entry-point acceptance, including the gated real Agent scenario,
  was not rerun as part of the default workspace tests.
- The existing three real-Chrome tests use one project root and do not provide
  the combined two-project, two-Profile fill coverage required by 8.2.
- The no-Python internal-broker probe does not establish the default Chrome
  launcher and real-extension registration required by 8.3.

Build validation must not replace `target/debug/teshi.exe` while an acceptance
broker is running from it; use separate binaries or run these gates serially.

The production/default Chrome route remains Python. No cutover, Chrome Python
removal, commit, push, packaging, or release was performed.
