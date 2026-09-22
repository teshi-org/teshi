## 1. Build identity and release contract

- [x] 1.1 Move pure version identity into teshi-core with TUI compatibility re-exports; add explicit channel, full SHA, timestamp and build sequence injection for shipped binaries.
- [x] 1.2 Define versioned external update and embedded bundle manifests, target/layout rules, managed inventory and minimum helper protocol; add schema validation tests.
- [x] 1.3 Generate manifests and SHA256SUMS from actual packaged files without circular hashes; publish validated complete releases from drafts.
- [x] 1.4 Test stable/nightly metadata, same-day builds, equal-SHA releases and missing build identity in packaging fixtures.

## 2. Shared discovery and CLI

- [x] 2.1 Create teshi-update with injectable HTTP/clock/install interfaces, serializable state/events and typed errors; verify no UI dependencies.
- [x] 2.2 Implement paginated GitHub resolution, explicit channel filtering, stable SemVer ordering, nightly sequence comparison and platform compatibility validation.
- [x] 2.3 Implement conditional request cache, cross-process check coordination and rate-limit backoff; test pagination and incomplete/inconsistent releases with mock HTTP.
- [x] 2.4 Resolve installation root and ownership using executable/manifest/MSI evidence; test portable, MSI, explicit external management and unknown builds.
- [x] 2.5 Add update command routing, flags, automatic-install rules, JSON output and exit codes; test --check has no payload mutation and explicit update invocation starts without a second confirmation.

## 3. Download and portable transaction

- [x] 3.1 Implement bounded streaming download, cancellation, HTTPS redirect restrictions, disk-space checks and checksum agreement; test truncation and hash failures.
- [x] 3.2 Implement safe ZIP/tar.gz extraction and inventory validation; test traversal, links, duplicate paths, expansion limits and wrong-target payloads.
- [x] 3.3 Implement durable transaction intent/completion records, managed-file backup/replacement, unknown-file preservation and reverse recovery with injected failures.
- [x] 3.4 Add apps/teshi-update-helper, protocol validation, restricted transaction storage and lock-preserving authenticated handoff; test helper rejection of altered plans.
- [x] 3.5 Register install participants in CLI/desktop/daemon/sidecar startup and implement graceful quiescence; prevent new participants during install without force-killing work.
- [x] 3.6 Add bounded binary identity smoke check, persistent final results, startup recovery and optional desktop restart; test restart failure independently of installation success.
- [x] 3.7 Run real portable upgrade and rollback fixtures on Windows x86_64, Linux x86_64 GNU and macOS ARM64, including locked files and preserved version-named roots.

## 4. Windows setup.exe user updates

- [x] 4.1 Superseded: MSI msiexec coordinator is removed; user updates do not wait on MajorUpgrade VM evidence.
- [x] 4.2 Add InstallKind::Exe, per-user Inno setup (`teshi-<tag>-x64-setup.exe`), and `bin/` bundle layout matching the setup payload.
- [x] 4.3 Apply updates only for EXE installs: silent `/update=true` stages `{app}\install`, then the helper journals the Exe inventory. Portable ZIP/tar.gz and registered MSI are check-only.
- [x] 4.4 Remove helper `--msi-transaction` and msiexec result mapping from the update path. Keep shipping MSI/WinGet as a manual/enterprise package.

## 5. Native desktop experience and distribution

- [x] 5.1 Add native teshi-update-ui Entity adapter and nonblocking event delivery; keep native update dependencies out of the WASM build.
- [x] 5.2 Add manual check, release notes, progress/error display, direct install action and save/exit/restart coordination using shared core states.
- [x] 5.3 Add auto_check/channel settings and hourly/15-minute scheduling with jitter/shared cache; test development/external-manager defaults and rate-limit recovery.
- [x] 5.4 Build and include the native desktop executable and helper in Windows ZIP/MSI; assert packaged inventory and shared version identity in CI and fix stale installation documentation.
- [x] 5.5 Exercise native desktop discovery through install/restart with unsaved work, active daemon runs and a competing CLI update; confirm no false installed status.

## 6. Documentation and release gates

- [x] 6.1 Document CLI flags/exit codes, install matrix, first-release manual bootstrap, checksum trust boundary and separate external skill/extension refresh behavior.
- [x] 6.2 Run focused updater tests plus cargo fmt --all --check and applicable native workspace check/test/clippy/doc gates excluding teshi-web; use the separate WASM smoke gate if affected.
- [ ] 6.3 Validate produced ZIP/MSI/tar.gz/`*-x64-setup.exe` assets against their manifests and checksums; retain helper recovery evidence. Linux/macOS in-app updates remain leftover (`leftovers.md` L1).
- [ ] 6.4 Reconcile implementation with every application-self-update scenario and run strict OpenSpec validation before marking this change complete.

## Leftovers

Deferred work lives in [`leftovers.md`](leftovers.md). Linux/macOS in-app updates (L1) and publication evidence (L2/L3) remain open. Do not reintroduce msiexec auto-update.
