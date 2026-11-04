# Chrome Rust migration: stage 8.4 packaging evidence

## Status and scope

Source revision used for the artifact evidence: `cd10776a2b2fccd6561efc2a60393d9407e0e0c4` on `dev`.
At the original stage 8.4 verification, OpenSpec was **41/47 complete**.
See [migration tasks](../openspec/changes/migrate-chrome-browser-broker-to-rust/tasks.md)
for current progress. Task 8.4 is partial: Windows and Linux
artifacts have local evidence, but an actual macOS archive and the final
Chrome Python removal boundary remain unverified. No installation, Git tag,
push, release publication, pairing change, or production routing change was
performed. All generated files and raw logs remain under `.acceptance-84/`;
Linux compilation uses the separate ignored `target/acceptance-84-linux/`.

## Changes

- Repair `scripts/build-msi.ps1` to include the CLI, update helper, Desktop,
  API resources, transitive Python imports, and hashed bundle manifest.
  It accepts prebuilt binary/dist inputs, rejects nonempty staging roots,
  compares compiled identities, checks every native tool exit code, and writes
  WiX fragments under its output directory instead of the source tree.
- Add `scripts/package_runtime.py` to resolve resource-owned imports from
  actual entry points. The standalone browser-testing package now includes
  the previously missing `update_participant.py`.
- Add `scripts/test-release-package.py` for source/resource inventory,
  version/protocol/extension identity, binary format, channel layout, local
  state exclusion, and bundle size/hash validation. Native execution is an
  explicit option. Its broker probe uses an ephemeral loopback port, private
  temporary state, no project venv, and a PATH containing only the staged bin
  directory; it terminates only its own Popen process.
- Probe the staged Embedded entry point without checkout PYTHONPATH. Keep
  the Python oracle tests separate from the packaged Rust process probe.
- Add seven negative/layout tests and wire inventory gates into CI and
  release staging. Define the nightly Desktop flag in its own PowerShell
  step, rather than depending on another step's variables.

## Local build and inspection evidence

| Artifact/layout | Result | Evidence |
| --- | --- | --- |
| Windows release CLI + helper | Built with `--offline --release --locked`; 4m 16s | `release-cli-build.log` |
| Windows release Desktop | Built with matching identity; 3m 15s | `release-desktop-build-final.log` |
| Full Windows MSI | Complete script and ICE validation passed; final cabinet extracted and checked | `msi-script-verified.log`, `msi-script-payload-inspection.json` |
| Full Windows EXE | Inno compiler succeeded; 52-file source inventory and compiled identities checked | `exe-build.log`, `full-exe-inspection.log` |
| Windows portable ZIP | Built, CRC checked, extracted, inventory/hash/native broker probe passed | `payload-inspection.json` |
| Windows installer-only layout | 45 files; no Desktop or hosted WASM payload; Inno EXE build succeeded | `nightly-exe-build.log`, `payload-inspection.json` |
| Standalone browser-testing ZIP | 28 files; built, CRC checked, extracted, dependencies checked | `payload-inspection.json`, `browser-agent-package-final.log` |
| Linux x86_64 GNU archive | Built in Rocky WSL; 44 files, archive executable bits and extracted hashes checked | `linux-release-build-online.log`, `linux-package-inspection.json` |
| Extracted Linux native runtime | CLI/helper identity checks and Python-free Rust broker probe passed | `linux-native-probe.json` |
| Local Windows nightly identity | CLI/helper rebuilt for nightly sequence 2; 45-file staging and Rust probe passed; Inno EXE built | `nightly-release-build.log`, `nightly-actual-inspection.json`, `nightly-actual-exe-build.log` |

The MSI cabinet contains **53 files**: all 52 staged bundle files plus the
separately verified license. CLI, helper, and Desktop identities agree on
version `0.7.10`, stable channel, source SHA, timestamp, and sequence `1`.
Linux CLI/helper match that stable identity. Windows EXE evidence covers
compiler input and successful compilation; EXE payload extraction and an
installed-machine run were not performed.

The local nightly installer is named
`teshi-v0.7.10-nightly.20261004.cd10776-x64-setup.exe`. It carries the compiled
nightly channel and sequence `2`, not a stable executable renamed as nightly.
Its build took **3m 07s**. No corresponding Git tag or GitHub release exists
as a result of this work. The earlier `packagecheck` EXE is only a layout
fixture built from stable binaries.

The first MSI linker attempt failed because the sandbox could not access
Windows Installer for ICE checks. The same linker command passed with service
access. Existing WiX warnings `CNDL1077` (redundant formatted ARP property) and
`ICE61` (same-version upgrades) remain visible. Cabinet decompilation reports
existing UI table foreign-key warnings; file extraction and checks passed.
The initial local fragment variable failure was fixed by supplying all four
absolute WiX roots. A Windows sharing violation after copying a binary is
handled only by a bounded retry of the read-only identity query.
The complete repaired MSI script subsequently passed in a fresh staging root
(`msi-script-verified.log`). An intervening cabinet build sharing violation is
preserved in `msi-script-final.log`; a fresh staging/output run succeeded.

Ubuntu WSL has no Rust on PATH. Rocky WSL has Rust `1.97.1`; its first offline
build lacked the `zip` index/dependency. A `--locked` build fetched the missing
dependencies and succeeded in **3m 45s** without changing `Cargo.lock`.
This is native Rocky/Linux evidence, not a Linux CI matrix or macOS result.

## Retained Python resources

| Resource | Current live consumer |
| --- | --- |
| `browser_service.py` | Embedded Playwright and the temporary production Python Chrome path |
| `browser_agent_broker.py` | Imports from `browser_service.py`, including Embedded helpers; migration oracle and temporary Chrome runtime |
| `update_participant.py` | Browser, WinApp, and API sidecar lifetime coordination |
| `winapp_service.py` | Windows WinApp runtime; absent from Linux portable staging |
| `api_service.py`, `teshi_api.py` | Independent API runtime; preserved outside Chrome removal scope |

The standalone browser-testing runtime closure contains exactly
`browser_service.py`, `browser_agent_broker.py`, and `update_participant.py`.
The WinApp runtime builder separately copies `winapp_service.py` and
`update_participant.py`; its production archive build was not repeated here.
Removing the broker now would break a live import. Retire it only after task
8.7 splits/clears those imports and production selection, then rerun packaging.

## Validation and remaining gates

Stable Windows binary build commands:

```text
cargo build --offline --release --locked --bin teshi --bin teshi-update-helper
cargo build --offline --release --locked --bin teshi-desktop
```

All three used the same injected stable identity, source SHA above, timestamp
`2026-10-04T10:07:26Z`, and sequence `1`. The Linux command was:

```text
cargo build --release --locked --bin teshi --bin teshi-update-helper
```

It ran inside Rocky WSL with the same identity and
`CARGO_TARGET_DIR=/mnt/d/Dev/Rust/teshi/dev/target/acceptance-84-linux`.
Native verification ran `python3 -B .acceptance-84/probe_linux.py` in WSL.
The rebuilt nightly CLI/helper used the first Windows command with the same
source/timestamp, nightly channel, build date `20261004`, and sequence `2`.

The complete MSI entry point verification used:

```powershell
./scripts/build-msi.ps1 -SkipBuild -SkipWebBuild `
  -BinaryDir .acceptance-84/msi-root-final/bin `
  -StagingRoot .acceptance-84/msi-script-verified `
  -OutputDir .acceptance-84/wix-script-verified
```

```text
python -B scripts/test_release_package.py                 # 7 passed
python -B scripts/test_update_manifest.py                 # 5 passed
TESHI_CLI=<release exe> python -B scripts/test-browser-agent-package.py
python -B scripts/test-release-workflow-contract.py
python -B scripts/test-nightly-workflow-contract.py
python -B scripts/test-nightly-artifact.py --root .acceptance-84/nightly-layout
python -B scripts/test-nightly-artifact.py --root .acceptance-84/nightly-actual
cargo fmt --all --check
openspec validate migrate-chrome-browser-broker-to-rust --strict
git diff --check
```

Python AST checks and both edited workflow YAML parses passed. Artifact
checksums are saved in `.acceptance-84/artifact-sha256.json`. Structural tests
use short fake executable headers, including the macOS fixture; they are not
cross-platform builds. The previous stage 8.1 full native regression is not
claimed as a new run in this packaging-only change.
The ten-file change scope passed a known-runtime-secret and high-confidence
credential-pattern scan. Generated packages, private runtime state, and old
acceptance artifacts are outside the Git change scope. The index was empty at
the time of the original stage 8.4 verification.
An additional scan of 169 files across extracted/staged Windows, Linux,
nightly, and standalone package payloads found no known runtime secrets or
high-confidence private-key/service-token patterns. This is a pattern scan,
not a proof that arbitrary sensitive values cannot exist.

Still required: actual macOS build/archive/native inspection, final Python
resource convergence after acceptance, installed-machine/EXE extraction
evidence where required, and final release-equivalent validation. The local
nightly layout is not evidence of a published Nightly or a remote CI run.
Task 7.1 was subsequently completed in `e08b6db`. The evidence above does not
validate that later startup-selection change; consult the migration tasks for
the current remaining work.
