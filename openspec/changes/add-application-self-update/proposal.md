## Why

Teshi publishes release bundles but has no built-in update command or background update discovery. Users must replace installations manually, risking mismatched binaries and bundled resources, especially across nightly builds with the same package version.

## What Changes

- Add a UI-independent `teshi-update` core using GitHub Releases, explicit build identity, a serializable status model, verified downloads, and installation ownership detection.
- Add `teshi update`, `--check`, `--channel stable|nightly`, `--yes`, and `--json` with precise unattended and helper-handoff semantics.
- Update only the Windows per-user setup.exe installation through a silent Inno pass and the journaled helper. Portable ZIP/tar.gz and MSI/WinGet installs are check-only; external-manager guidance remains for explicitly managed installations.
- Add native GPUI update status, manual actions, restart coordination, and automatic checks (stable hourly, nightly every 15 minutes, development disabled).
- Extend release packaging with an update manifest, consistent build metadata, and a bundled helper. Preserve SHA256SUMS validation.
- Keep background checks discovery-only; an explicit `teshi update` command or install action starts the verified upgrade without a second confirmation. Do not introduce background unattended installation.

## Capabilities

### New Capabilities

- `application-self-update`: Release resolution, CLI operations, native update UI, installation transactions, and recovery.

### Modified Capabilities

None.

## Impact

New update core/helper and native UI adapter; shared version metadata; CLI command routing; native desktop settings/lifecycle; release and nightly workflows; WiX packaging; installation documentation. Native filesystem operations stay outside the WASM frontend. Existing users must install the first updater-enabled release manually. No changes to project requirements, credentials, or runtime database formats are intended.

Windows user self-update ships as Inno `teshi-<tag>-x64-setup.exe` (Zed-style: per-user `%LOCALAPPDATA%\Programs\teshi`, no UAC). Portable archives and MSI are not in-app update backends. Linux/macOS in-app updates are leftover until a managed installer exists.
