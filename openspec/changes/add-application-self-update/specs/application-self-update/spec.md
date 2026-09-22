## ADDED Requirements

### Requirement: Shared native update core
Teshi SHALL provide a native update core independent of GPUI and TUI, exposing serializable states and typed results to CLI and native desktop. Update I/O MUST NOT enter teshi-core or WASM.

#### Scenario: Shared results
- **WHEN** CLI and desktop check the same installation and release source
- **THEN** they produce the same candidate identity and installation eligibility using the shared core

### Requirement: Release identity and channel selection
The updater SHALL resolve complete non-draft GitHub releases for the selected channel and exact target, validating versioned update metadata and asset identity. Stable SHALL use SemVer ordering. Nightly SHALL use a numeric build sequence and full commit identity, rejecting older sequences, inconsistent identities and lower base versions. Switching channels SHALL require explicit intent and SHALL reject lower base versions.

#### Scenario: Same-day nightly update
- **WHEN** a nightly has the same SemVer and date but a higher build sequence and a different full SHA
- **THEN** it is offered as an update

#### Scenario: Different older commit
- **WHEN** a nightly candidate has a different SHA but an older build sequence
- **THEN** it is not offered as newer

#### Scenario: Missing target or metadata
- **WHEN** the newest release lacks valid metadata or the current target's asset
- **THEN** the updater reports incompatibility without installing another architecture or silently downgrading

### Requirement: CLI update behavior
Teshi SHALL expose update, --check, --channel stable|nightly, --yes and --json. Check mode SHALL fetch metadata only and SHALL NOT download archives or modify installed payloads. An explicit `teshi update` invocation SHALL begin installation automatically after a verified candidate is found; `--yes` SHALL remain accepted for compatibility and SHALL NOT be required in interactive, non-interactive or JSON modes. JSON SHALL be a single final object with progress on stderr. Exit codes SHALL be 0 for completed requests, 1 for failure, 2 for argument errors and 3 for accepted pending helper installation.

#### Scenario: Automated discovery
- **WHEN** `teshi update --check --json` finds a newer release
- **THEN** it returns exit 0 and update_available with current/target identity and eligibility without staging payloads

#### Scenario: Helper handoff
- **WHEN** installation requires the initiating CLI to exit
- **THEN** the command reports pending and a transaction ID with exit 3, and persists the later result for the next invocation

### Requirement: Installation ownership
The updater SHALL resolve the canonical executable root and validated installation metadata. It SHALL apply in-app replacement only for Windows per-user setup (`kind: exe`) installations. It SHALL NOT replace files in portable ZIP/tar.gz trees or registered MSI/WinGet installations. It MUST NOT infer external ownership from the presence of a package-manager executable.

#### Scenario: EXE installation
- **WHEN** the running executable belongs to a validated `kind: exe` setup root
- **THEN** update planning selects the Windows setup.exe asset and never applies ZIP, tar.gz or MSI payloads

#### Scenario: Portable archive
- **WHEN** the running executable belongs to a `kind: portable` bundle
- **THEN** checks may describe a newer release but installation is blocked with guidance to use setup.exe on Windows or to replace the archive manually elsewhere

#### Scenario: MSI installation
- **WHEN** the running executable belongs to the registered Teshi MSI root
- **THEN** checks may describe a newer release but installation is blocked; ZIP replacement is never applied

#### Scenario: Unknown source build
- **WHEN** a build lacks a validated installation manifest
- **THEN** update checks can describe a release but installation is blocked with manual guidance

### Requirement: Verified complete bundles
The updater SHALL validate exact archive size/hash against release metadata and SHA256SUMS before extraction, then validate a bounded archive and managed-file inventory. It SHALL reject unsafe paths and links. Release CI SHALL publish complete metadata/assets together and package the helper and native desktop binary in Windows bundles used for desktop updating.

#### Scenario: Corrupt or escaping archive
- **WHEN** checksum verification fails or an archive entry escapes the staging root
- **THEN** installation fails before changing any installed payload

#### Scenario: Partial publication
- **WHEN** a release is missing required bundle metadata or checksums
- **THEN** it is not treated as installable

### Requirement: Recoverable EXE replacement
Windows setup updates SHALL stage the new tree with a silent installer without overwriting running executables, then use an installation lock, process quiescence, a durable transaction journal, managed-file backup and reverse rollback. They SHALL preserve unknown files, Inno uninstall metadata and user data, block unknown-file collisions, and recover interrupted transactions before bundled services start. The helper SHALL verify the new binary identity before committing success. Portable fixtures MAY exercise the same journal without enabling portable `can_install`.

#### Scenario: Mid-transaction failure
- **WHEN** a managed-file move fails after earlier moves succeeded
- **THEN** the helper restores the prior managed bundle and records the failure, retaining recoverable backups if rollback itself cannot finish

#### Scenario: Process crash
- **WHEN** startup finds an unfinished transaction
- **THEN** recovery runs before normal services and prevents use of a mixed-version bundle

#### Scenario: User-owned file collision
- **WHEN** a new payload path would overwrite a file outside the previous managed inventory
- **THEN** the transaction is blocked without removing that file

### Requirement: Process coordination
Updates SHALL coordinate all Teshi participants for the same installation across processes, reject concurrent installations and defer while work cannot exit gracefully. The updater MUST NOT invoke Windows Installer or record MSI reboot-required as an in-app result.

#### Scenario: Busy installation
- **WHEN** another update or active non-quiescent Teshi run holds the installation
- **THEN** the updater reports blocked and makes no replacement

### Requirement: Native desktop automatic discovery
Native desktop SHALL default to check-and-notify for supported release installations, hourly for stable and every 15 minutes for nightly, with a shared cache, jitter and rate-limit backoff. Development and explicitly externally managed installations SHALL NOT poll automatically. CLI startup and WASM SHALL NOT schedule checks.

#### Scenario: Available desktop update
- **WHEN** a scheduled check discovers a newer release
- **THEN** the UI offers installation without automatically downloading, installing or restarting

#### Scenario: Rate limited
- **WHEN** GitHub reports a rate limit
- **THEN** the updater backs off without preventing application startup

### Requirement: Desktop update lifecycle
The native UI SHALL observe shared update states and provide manual checking, release notes, download/install progress, errors and restart coordination. Staged SHALL mean verified pending payload; ReadyToRestart SHALL only follow successful replacement. Unsaved work SHALL be resolved before exit. Restart failure SHALL be distinguished from installation failure.

#### Scenario: Unsaved edits
- **WHEN** installation needs desktop exit while edits are unsaved
- **THEN** the user can save or cancel before helper handoff and no work is silently discarded

#### Scenario: Replacement succeeds but launch fails
- **WHEN** the new bundle passes validation but desktop restart fails
- **THEN** the result reports installed with restart failure rather than falsely reporting installation rollback
