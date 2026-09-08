## Context

Verified against the local dev checkout on 2026-09-08:

- `crates/teshi-tui/src/cli/mod.rs` has no update command. `VersionInfo` and CI build environment tracking currently live in teshi-tui.
- `nightly.yml` creates `v<semver>-nightly.<YYYYMMDD>.<short-sha>`; the date alone cannot order multiple builds on one day.
- `release.yml` emits Windows x86_64 ZIP/MSI, Linux x86_64 GNU tar.gz, macOS ARM64 tar.gz, and SHA256SUMS. Portable packages have an executable at their root; MSI uses bin/ and share/. Current packaging copies the CLI and resources but does not actually stage a desktop executable despite an outdated workflow comment and installation documentation.
- WiX uses perMachine installation and AllowSameVersionUpgrades. MSI version ordering alone cannot protect nightly build ordering.
- Zed provides useful state/UI/helper patterns, but its auto_update crate itself imports GPUI. Teshi will implement an independent core rather than copy that dependency structure.

References: [Zed updater source](https://github.com/zed-industries/zed/blob/main/crates/auto_update/src/auto_update.rs), [GitHub Releases API](https://docs.github.com/en/rest/releases/releases). The design does not depend on reproducing Zed installer internals.

## Goals / Non-Goals

**Goals:** Working CLI upgrades and native desktop discovery; exact channel/build selection; complete bundle consistency; recoverable replacement; clear installation ownership; reusable non-GPUI core.

**Non-Goals:** Zed Cloud, unattended installation/restart, delta patches, arbitrary repositories, downgrades, browser-triggered host upgrades, forced shutdown of unrelated processes, changing already installed external skills or browser profiles, **in-app MSI/Windows Installer upgrades**, and in-app updates of portable ZIP/tar.gz trees. Bundled skill updates do not automatically rerun install-skill. MSI remains a WinGet/manual package only.

## Decisions

### 1. Components and boundaries

- `crates/teshi-update`: GitHub source, release model/comparison, download/verification, install planning, transaction journal and serializable events. No GPUI, TUI, daemon, or desktop dependency. Inject HTTP, clock and installation interfaces for tests.
- `apps/teshi-update-helper`: executable using the update core's local installer routines; no network access. Lives under apps to match repository binary conventions.
- `crates/teshi-update-ui`: native GPUI adapter observing core events through an Entity. Excluded from WASM integration. Native desktop composes the adapter; no UI download/install implementation.
- Move pure build identity to teshi-core and inject build-time identity consistently in consuming binaries; retain teshi-tui re-exports. Network/filesystem operations remain in teshi-update, never teshi-core.
- A per-installation OS lock and persistent transaction record coordinate separate processes. CLI and desktop do not assume a shared in-memory singleton. Background polling uses a shared last-check/ETag cache and a short check lock to avoid duplicate requests.

Alternative rejected: implementing everything in cli/update.rs prevents reuse and makes Windows process handoff inseparable from UI code.

### 2. Release identity and publication contract

Add a versioned `update-manifest.json` release asset and an embedded bundle manifest. Include schema version, semver, explicit channel, full git SHA, UTC build timestamp, numeric CI build sequence, target triple, bundle layout/version, minimum updater protocol, and exact asset names/sizes/SHA256 values. The bundle manifest inventories managed paths and contains installation kind; packaging supplies MSI versus portable metadata separately. Avoid circular archive hashing: the external manifest hashes archives, while the embedded manifest inventories payload files. SHA256SUMS covers archives and the external manifest.

Explicit stable metadata replaces the ambiguity of missing TESHI_BUILD_CHANNEL; missing or inconsistent identity is development/unknown, with automatic updates disabled. Support existing tag formats for display, but require complete update metadata for installable candidates. The first updater-enabled release is installed manually.

Use GitHub Releases with pagination, excluding drafts and matching explicit channel. Stable selects the greatest valid stable SemVer rather than assuming API listing order means SemVer order. Nightly selects the greatest build sequence within the nightly channel, checks full SHA identity, and refuses a lower sequence. SHA inequality signals different content, not newer content. Equal SHA means no update; equal sequence with different SHA is inconsistent metadata. A higher nightly sequence cannot lower base SemVer. Validate candidate platform and manifest before presenting an installable update. No fallback from a newer incompatible release to an older candidate without explaining the incompatibility.

Pin release ID and asset IDs for a transaction. Require immutable publication: build and validate all artifacts as a draft, publish only when complete, and avoid replacing published assets. Use ETag/conditional requests and Retry-After/rate-limit backoff. Do not silently substitute stable for nightly or another CPU architecture.

Channel switches require `--channel` and confirmation; reject lower base SemVer. A same-base nightly-to-stable switch is an explicit channel change, not a normal version comparison. Persist the channel only after successful installation. Arbitrary version pinning and downgrade overrides are deferred.

### 3. CLI and status contract

```
teshi update                         # check, show plan, confirm, download and install/handoff
teshi update --check                 # metadata only; no archive/helper/install
teshi update --channel nightly       # explicit channel switch
teshi update --yes                   # skip Teshi confirmation; not OS elevation
teshi update --check --json          # one machine-readable result
```

Without --yes, non-interactive installation fails with a clear confirmation-required error before downloading. --json suppresses interactive prompts; an installation additionally requires --yes. JSON output is one final object; progress goes to stderr. Include current/target identity, installation kind, result, transaction ID, and reboot/restart requirements. Exit 0 means the requested operation completed (including an available update in --check), 1 means failure, 2 means invalid arguments, and 3 means a helper accepted a pending installation. Pending MUST NOT be printed as installed.

Status model: Idle, Checking, UpToDate, UpdateAvailable, Downloading, Verifying, Staged, WaitingForExit, Installing, ReadyToRestart, Completed, Blocked, Errored. Include transaction identity and typed error codes. Staged means validated payload only; ReadyToRestart means replacement actually succeeded and an application restart remains. MSI reboot-required is a separate flag. CLI users inspect the persisted previous transaction result on the next invocation; desktop restoration reads it on startup.

### 4. Ownership and supported installation matrix

| Installation | Action |
| --- | --- |
| Windows per-user Inno setup (`kind: exe`, `%LOCALAPPDATA%\Programs\teshi`) | Download `teshi-<tag>-x64-setup.exe`; silent `/update=true` stages `{app}\install`; helper journals managed files after participants exit |
| Manifest-marked portable ZIP/tar.gz | Check only; explain that portable trees are not self-updating |
| Registered Teshi MSI / WinGet | Check only; explain that in-app updates require the setup.exe user install |
| Explicit external package-manager ownership marker | Explain the manager command; do not overwrite or invoke it automatically |
| Source/Cargo build, unknown layout, unmarked installation, unsupported target | Check only and explain manual installation |

Do not classify installation ownership by PATH or the presence of winget alone. WinGet uses the same MSI and cannot reliably be distinguished from direct MSI installation without provenance; registered MSI is MSI-owned unless an explicit external-management marker says otherwise. Do not invent a reliable WinGet provenance heuristic.

The installation root derives from the canonical executable path and validated manifest, not the current working directory. Never convert MSI into an EXE or portable tree by overwriting Program Files. Never apply ZIP/tar.gz replacement to any installation kind.

### 5. Download, staging and transaction

Use bounded streaming downloads, HTTPS and a constrained GitHub asset redirect policy, cancellation, timeouts, disk-space checks and exact SHA256SUMS/manifest agreement. SHA256 detects corruption and mismatched assets; it is not an independent signature if GitHub is compromised. Signed update metadata is a future hardening option, not a claim of this implementation.

Reject archive traversal, absolute paths, duplicate normalized entries, links/reparse points, device names, and excessive expansion. Validate expected root, executable, target and payload inventory. Stage on the installation volume. Keep transaction/helper files outside paths being replaced with restrictive permissions and protocol validation.

For EXE updates, Inno writes the new tree beside the running binaries (`{app}\install`) so locked executables are not overwritten. After participants exit, the helper applies the same journaled managed-file transaction used in fixtures: move inventoried files, preserve unknown/user and Inno uninstall files, fail on unknown-file collisions. The installation lock remains held through handoff. Persist journal steps with recoverable intent/completion records. On failure undo in reverse order. On startup detect incomplete transactions and recover before starting bundled services.

The helper waits for the initiating process handle and other registered Teshi processes using this installation. Desktop asks to save work and close before handoff. Active test runs and uncooperative processes block installation; do not force-kill them. Daemons/sidecars register install identity and acknowledge graceful shutdown; hold participant locks so new starts cannot race an installation. Bound file-lock retries; unexpected locks lead to rollback and an actionable error. Restart Manager forced shutdown is deferred.

Helper performs a bounded new-binary version/protocol smoke check without starting UI or services before committing and deleting backups. Failure rolls back. Persist success before optional restart; restart failure is reported separately from installation failure. Desktop restart restores approved project/window context; CLI does not reopen a TUI automatically.

Do not launch msiexec. Elevation is not required: the user install lives under `%LOCALAPPDATA%\Programs\teshi` (`PrivilegesRequired=lowest`). Inno `/update=true` must not recreate shortcuts or mutate PATH. After success verify installed identity and remove the staging `install` directory.

### 6. Automatic checks and GPUI

Native desktop checks at startup when due and then hourly for stable / every 15 minutes for nightly, with jitter, persisted cache and failure backoff. Development and external-manager installations do not poll automatically. Settings provide auto_check (default true for supported release installations) and channel. No polling task is started for short-lived CLI invocations or the WASM client.

UI provides Check for Updates, release notes, Download and Install, progress/error state and restart/save coordination. Auto-check only discovers; download, install and restart require user actions. Disabling checks cancels scheduled checking and does not abort an already explicitly requested installation. UI observes events without blocking the GPUI foreground executor.

## Risks / Trade-offs

- [Nightly identity ambiguity] → Require full metadata and numeric sequence; do not order by short SHA or day.
- [Mixed versions during replacement] → Participant locks, quiescence, journal recovery and post-install smoke check.
- [Portable/MSI self-update ownership] → Check-only; only the Inno user install is Teshi-owned for replacement.
- [GitHub outages or anonymous rate limits] → Conditional requests, persisted polling state, backoff and actionable retry errors; normal application startup still works.
- [Packaging drift] → Assert actual bundle inventories in CI, including the setup.exe payload and `kind: exe` manifest.
- [Repository compromise] → Checksums share the release trust boundary; independent signing remains outside this change.

## Migration Plan

1. Implement identity/manifests and core discovery; retain existing display compatibility.
2. Implement CLI, helper journal and recovery; keep portable fixtures as the replacement engine.
3. Ship per-user Inno setup.exe as the only in-app Windows backend; stop MSI and portable replacement.
4. Integrate native GPUI polling/actions and include desktop/helper in Windows bundles.
5. Publish the first updater-enabled release through the normal release process. Existing users install setup.exe manually; later Windows user upgrades use `teshi update`.

Each implementation phase has its own reviewable validation, but the change is complete only when CLI, the Windows setup path and native UI acceptance pass. Linux/macOS remain check-only (`leftovers.md`). Documentation must explain that externally copied skills/extensions require their existing refresh workflow. Binary rollback does not roll back user databases; this change introduces no database migrations.

## Open Questions

The remaining product decision is Linux/macOS managed installs (`leftovers.md` L1). Windows defaults are check-and-notify for the per-user setup.exe, explicit installation, and no msiexec auto-update. Portable ZIP/tar.gz and MSI stay visible as downloadable packages.
