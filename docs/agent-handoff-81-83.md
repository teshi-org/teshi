# Rust Chrome migration: stages 8.1 and 8.3 acceptance

Validated on Windows on 2026-10-04 from `dev` HEAD `2da6f93`, with the
uncommitted acceptance changes described below. Tasks 8.1 and 8.3 are now
complete. Together with the preceding two-project task 8.2, progress is 41/47.

## Ordinary launcher and real Agent result

The human user explicitly approved rotating stopped per-user Broker runtime
records and sending dedicated synthetic fixture page data to DeepSeek.
`resources/tests/test_browser_python_free_launcher.py` then passed the complete
ordinary-launcher and Agent scenario in 41.621 seconds. Evidence:
`.acceptance-82/normal-launcher-configured.log` and
`.acceptance-82/normal-launcher-configured/two-profile-rust-transport.jsonl`.

The ordinary `teshi browser sessions` command starts the shared Rust broker
through the explicit development selector. Python, pip, uv, and Playwright
executables are absent from the launcher/broker PATH, Python-related environment
variables are removed, and neither temporary project has a virtual environment.
The extension registers both real Chromium Profiles. The broker performs
navigation, snapshot, locator verification, click/fill/assert, screenshot,
Console and Network capture, project/caller isolation, and generation recovery.
After restart, old leases fail, both targets retain identity, and filled values
remain distinct. Embedded reports its missing-Python/install guidance in the
same environment; configured Embedded functionality also passed separately.
The Python/Playwright acceptance driver remains outside the Python-free runtime.

After closing only Profile B, two explicitly selected Rust tests passed against
Profile A and the same broker generation:

| Test | Result | Evidence under `.acceptance-82/normal-launcher-configured/` |
| --- | --- | --- |
| `app::tests::native_model_browser_tool_round_trip_uses_the_shared_broker --ignored` | 1 passed, 0.09 seconds | `native_model_browser_tool_round_trip_uses_the_shared_broker.log` |
| `app::tests::live_chrome_agent_broker_identity` | 1 passed, 8.92 seconds | `live_chrome_agent_broker_identity.log` |

The second test used the actual DeepSeek service through the production
`dispatch_user_message`/`poll_llm_events` tool loop. It observed the synthetic
page `http://127.0.0.1:1599/project-a`, title `Rust project-a`. The reported
Broker PID was 43004, generation `b1a1bb80e7a4447e96ec117509428c1c`, protocol 1,
and Profile `5d3ff261-c982-47cd-b3ab-ea786c343583`. These match the native baseline.
Credentials and lease tokens were not printed or written into public pointers.

Both test-created broker generations and browser contexts were cleaned up.
The durable pairing file was compared before and after, and is unchanged.
The original workspace endpoint pointer was preserved. Normal startup rotated
the stopped per-user runtime records as authorized; it did not persist the
temporary extension's development Origin into durable pairing.

## Harness repairs and retained failures

The initial attempts exposed acceptance-harness issues rather than unexplained
browser behavior. Their logs remain under `.acceptance-82/`:

- `normal-launcher-pipe-failure.log`: the detached broker retained a captured
  CLI pipe, so waiting for pipe EOF timed out although the CLI exited. Launch
  output now goes to files, and the driver waits for the CLI process itself.
  The orphaned test broker was closed only after its unique executable path was
  verified through a Windows process handle.
- `normal-launcher-agent-setup-failure.log`: an inherited test field was not
  initialized; the wrapper now obtains the surviving Profile identity from the
  current registry.
- `agent-early-exit-failure.log`: the old live test treated frontend Idle as
  completion. It now selects Native explicitly and waits for backend Completed.
- `agent-config-failure.log`: the synthetic test environment had a DeepSeek
  key but omitted the shared `TESHI_LLM_*` configuration required by production
  dispatch. The driver now supplies the existing key only in child memory,
  fixes the destination to DeepSeek, and the test fails fast when configuration
  or submission is missing.

The new Windows launcher class requires
`TESHI_RUN_NORMAL_CHROME_LAUNCHER_ACCEPTANCE=1`. Ordinary test discovery skips
it, preventing accidental user-runtime changes or external provider requests.
No automatic retry of browser mutations or provider requests is embedded in
the scenario. Each rerun followed a diagnosed and corrected cause and used new
temporary projects/Profiles.

## Final suite evidence

| Gate | Result | Evidence under `.acceptance-82/` |
| --- | --- | --- |
| Native workspace, including CLI/MCP/Daemon/Desktop/UI/engine/security and doctests | 1,075 passed; 2 ignored across 47 suite results | `native-workspace-final.log` |
| Python Broker/protocol/security | 47 passed | `python-broker-final.log` |
| Python Service | 41 passed | `python-service-final.log` |
| Python WinApp mocks/fake WebSocket | 57 passed | `winapp-final.log` |
| Configured real Embedded Chromium | 2 passed | `embedded-final.log` |
| Python/Rust shared state differential | 1 passed | `shared-differential-final.log` |
| Extension protocol and Network | 40 passed | `extension-final.log` |
| Hosted UI transport | 14 RPC contract checks passed | `hosted-ui-contract-final.log` |
| Release/Nightly contracts | passed | `release-contract-final.log`, `nightly-contract-final.log` |
| Fmt and all-target/all-feature Clippy | passed | `clippy-final.log` and command output |

The default native workspace run still ignores the real model/browser round
trip; this report supplies its separate explicit pass above. The other ignored
test is the optional live GitHub TLS probe. The gated live Agent test's default
no-op result is not counted as live evidence; its separate real provider pass
above is the evidence.

The separate GPUI WASM distribution smoke passed earlier at this same production
revision: `.acceptance-75-81/web-ui-smoke-unsandboxed.log`. Subsequent Rust changes
are inside the TUI test module; web/frontend product code was not changed.
This run therefore retains that gate rather than claiming a new execution.
`cargo check` also passed earlier, and `cargo doc` completed with four existing
rustdoc link/URL warnings (`AgentDefinitionRaw`, `TreeState`, `from_string`, and
the CLI default GitHub URL), recorded in `.acceptance-82/native-doc.log`.

WinApp unit success does not establish real UIA/WGC/UAC acceptance. Linux/macOS
packaging, comparative performance, Python removal, and release validation
remain for later tasks. The production/default Chrome selector remains Python.
There was no commit, push, release, or production routing cutover.

Final OpenSpec strict validation and `git diff --check` pass. The evidence
scan checked 59 log/JSON artifacts for the configured provider key and raw
bearer/lease-token values and found none; result:
`.acceptance-82/evidence-hygiene-final.json`.

## Reproduction

Browser/Agent execution and the full native tests ran outside the sandbox;
the latter needs the Windows root-certificate store. All test data is isolated.
The wrapper refuses to run when the fixed discovery port already has a listener.
Credentials must already be configured and must never be included in commands,
logs, or committed files.

```powershell
$env:TESHI_CLI = Join-Path (Get-Location) '.acceptance-82/teshi.exe'
$env:TESHI_APP_DATA_DIR = Join-Path (Get-Location) '.acceptance-82/app-data'
$env:TESHI_AGENT_ACCEPTANCE_TEST_EXECUTABLE = (Get-Content .acceptance-82/agent-test-executable.txt -Raw).Trim()
$env:TESHI_RUST_TRANSPORT_ARTIFACT_DIR = Join-Path (Get-Location) '.acceptance-82/normal-launcher-configured'
$env:TESHI_RUN_NORMAL_CHROME_LAUNCHER_ACCEPTANCE = '1'
./.venv/Scripts/python.exe -m unittest resources.tests.test_browser_python_free_launcher.PythonFreeLauncherTests.test_normal_launcher_and_agent_acceptance -v

$env:TESHI_APP_DATA_DIR = Join-Path (Get-Location) '.acceptance-82/native-app-data'
cargo test --offline --workspace --exclude teshi-web --locked -- --test-threads=1
cargo fmt --all --check
cargo clippy --offline --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings
./.venv/Scripts/python.exe -B -m unittest resources.tests.test_browser_contract_differential -q
```
