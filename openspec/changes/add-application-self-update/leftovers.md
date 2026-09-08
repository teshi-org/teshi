# Leftovers

Recorded 2026-09-08. Updated the same day after the product decision to follow Zed-style Windows setup updates.

## L1. Linux and macOS in-app updates

**Status:** Deferred. Portable tar.gz on Linux/macOS is check-only.

Windows user updates use the per-user `setup.exe`. There is no equivalent managed installer on Linux or macOS yet (`install.sh` / `.app` layout). Until that exists, those platforms check GitHub and tell the user to replace the archive manually.

## L2. Release-asset checksum evidence (task 6.3)

**Status:** Open.

Validate published ZIP / tar.gz / MSI / `*-x64-setup.exe` against `update-manifest.json` and `SHA256SUMS`. MSI remains a WinGet/manual package; it is not an in-app update payload.

## L3. Spec reconciliation (task 6.4)

**Status:** Open after the EXE implementation lands.

Confirm every `application-self-update` scenario matches runtime: EXE installs can apply updates; portable and MSI cannot; no msiexec coordinator.

## Withdrawn

Disposable-VM MSI MajorUpgrade (former task 4.3) is **not** required. In-app MSI upgrades were removed rather than gated. Do not set any MSI install backend.
