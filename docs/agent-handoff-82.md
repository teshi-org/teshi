# Rust Chrome acceptance: two projects and two Profiles

Date: 2026-10-04. Branch: `dev`. Starting HEAD: `2da6f93`.

Subsequent user-authorized ordinary-launcher and real Agent results are in
`docs/agent-handoff-81-83.md`. That report supersedes this run's pending
authorization and remaining stage-8.1/8.3 boundaries below.

OpenSpec change: `migrate-chrome-browser-broker-to-rust`. Task 8.2 is
complete; progress is 39/47, including task 7.5 from the preceding run.

## Implementation and observed effects

`resources/tests/test_browser_two_profile_rust_transport.py` now includes
`test_two_projects_fill_evidence_and_reconnect`. Each temporary project has
its own root, caller, Profile, and lease. The real Rust broker and extension
perform navigation, snapshot, ranked locator verification, fill, text
assertions, click, PNG screenshots, Console capture, Network capture, and
recovery after a broker generation change.

The page input and echo preserve distinct values. Filling Profile A leaves
Profile B empty until its own authorized fill. Cross-project and cross-caller
fill attempts return `invalid_browser_lease` before any write. Clicking the
fixture submit button generates real Console and HTTP events; neither
Profile receives the other's events. Each screenshot is verified beneath
its own project's artifact root. After restart both targets retain their
identity and filled values, old leases are rejected, and fresh leases allow
broker assertions. Playwright prepares Profiles and observes effects; it
does not execute the tested actions.

The optional `TESHI_RUST_TRANSPORT_PYTHON_FREE=1` mode removes Python, pip,
uv, Playwright, and virtual-environment variables from the actual broker
child environment. The external test driver still uses Python/Playwright.
Both project roots have no `.venv`. The same environment produces the
documented missing-Python/install guidance for Embedded.

All browser Profiles, extension copies, HTTP fixtures, and brokers created by
the suites were cleaned up. Existing user processes and pairing state were
not changed. A copied CLI avoids replacement locks on `target/debug/teshi.exe`.

## Commands and evidence

Commands below run from the repository root in PowerShell. Browser execution
ran outside the sandbox so Chromium could launch; native checks used the
sandbox. Evidence directories are untracked local acceptance artifacts.

```powershell
$env:TESHI_CLI = Join-Path (Get-Location) '.acceptance-82/teshi.exe'
$env:TESHI_APP_DATA_DIR = Join-Path (Get-Location) '.acceptance-82/app-data'
$env:TESHI_RUST_TRANSPORT_ARTIFACT_DIR = Join-Path (Get-Location) '.acceptance-82/real-chrome'
./.venv/Scripts/python.exe -m unittest resources.tests.test_browser_two_profile_rust_transport.BrowserTwoProfileRustTransportTests.test_two_projects_fill_evidence_and_reconnect -v
# 1 passed, 23.157 seconds; .acceptance-82/two-projects.log

$env:TESHI_RUST_TRANSPORT_ARTIFACT_DIR = Join-Path (Get-Location) '.acceptance-82/python-free'
$env:TESHI_RUST_TRANSPORT_PYTHON_FREE = '1'
./.venv/Scripts/python.exe -m unittest resources.tests.test_browser_two_profile_rust_transport -v
# 4 passed, 113.950 seconds; .acceptance-82/python-free-suite.log

cargo check --offline --workspace --exclude teshi-web --locked
# passed; .acceptance-82/native-check.log
cargo clippy --offline --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings
# passed; .acceptance-82/native-clippy.log
cargo doc --offline --workspace --exclude teshi-web --locked --no-deps --document-private-items
# completed with four existing rustdoc warnings; .acceptance-82/native-doc.log
cargo fmt --all --check
# passed
openspec validate migrate-chrome-browser-broker-to-rust --strict
# passed
git diff --check
# passed
```

JSONL operation/cleanup evidence and two project screenshots are in
`.acceptance-82/python-free/`. The preceding full native, Python, extension,
and WASM results remain documented in `docs/agent-handoff-75.md`; they were
not rerun or counted as new executions here.

## Remaining boundaries

Task 8.3 remains unchecked: the no-Python run establishes real extension
registration and operations through the isolated internal broker entry
point, but does not establish the ordinary per-user Chrome launcher.
Configured Embedded operation passed in the preceding run.

Task 8.1 remains unchecked, including current real Agent/entry-point
acceptance. Read-only investigation found that the workspace Chrome pointer
does not match the user endpoint. A separate temporary pointer then revealed
that the retained user endpoint advertises only `p0.control`, without
`transport.v1`; public discovery at `127.0.0.1:17373/v1/bridge` refused the
connection. These failures are saved in `.acceptance-82/current-sessions.log`
and `.acceptance-82/native-existing-sessions.log`. No provider call was made,
and neither the original workspace pointer nor private user state was repaired
or overwritten during this investigation.

The production/default Chrome route remains Python. No cutover, Chrome Python
removal, commit, push, packaging, or release was performed.

## Prepared ordinary-launcher and Agent acceptance (not executed)

`resources/tests/test_browser_python_free_launcher.py` provides the concrete
next acceptance run. It refuses to proceed if the fixed discovery port is
occupied. It copies the CLI to a unique temporary path, invokes ordinary
`teshi browser sessions` with the explicit Rust development selector and a
Python-free PATH, and uses the temporary extension's exact origin as an
in-process development override. It does not persist or change durable
pairing. The normal launcher would rotate stale per-user runtime records.
Cleanup holds a Windows process handle and verifies the unique executable
path; it terminates only that test-owned broker. Both the two-project scenario
and broker restart run through this ordinary launcher.

After the second test-owned Profile is closed, the harness would run the
previously ignored native model/tool round trip and the real DeepSeek Agent
test against the remaining fixture page. The mock test can now read its
endpoint from `TESHI_ACCEPTANCE_PROJECT` instead of the ambient checkout.
The live test accepts `TESHI_EXPECT_BROWSER_URL_PATTERN`, so it can verify
the fixture's random HTTP port. The prepared run uses the configured API key
only at runtime and redacts it from output. Only dedicated synthetic fixture
page data is intended for the external `https://api.deepseek.com` provider.

The attempt to start this run was rejected before execution by automatic
approval review: changing user-level Broker runtime records and sending browser
page data to an external model need explicit human authorization. No user
runtime records were rotated and no model request was made. Do not execute
this command until that authorization is supplied:

```powershell
# Reuse TESHI_CLI and TESHI_APP_DATA_DIR from the commands above.
$env:TESHI_RUST_TRANSPORT_ARTIFACT_DIR = Join-Path (Get-Location) '.acceptance-82/normal-launcher'
$env:TESHI_AGENT_ACCEPTANCE_TEST_EXECUTABLE = (Get-Content .acceptance-82/agent-test-executable.txt -Raw).Trim()
./.venv/Scripts/python.exe -m unittest resources.tests.test_browser_python_free_launcher.PythonFreeLauncherTests.test_normal_launcher_and_agent_acceptance -v
```

The test modules pass `py_compile`. The modified Rust tests compile with
`cargo test --offline -p teshi-tui --lib --locked --no-run --message-format=json`.
Fmt and full all-target/all-feature Clippy pass after these source changes;
the latter result is in `.acceptance-82/native-clippy-after-agent-harness.log`.
The ordinary-launcher harness itself has not passed runtime acceptance.
