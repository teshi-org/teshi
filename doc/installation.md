# Installation

Coding agents should follow [AI_INSTALL.md](../AI_INSTALL.md) to install the teshi CLI, load teshi-bridge, and run `teshi install-skill`.

## Windows

| Asset | Contents |
|-------|----------|
| `teshi-vX.Y.Z-x64-setup.exe` | **User install (in-app updates).** Per-user copy under `%LOCALAPPDATA%\Programs\teshi`, no administrator prompt |
| `teshi-vX.Y.Z-x64.msi` / WinGet | Machine-wide package for IT/WinGet. Check-only in `teshi update`; does not self-update |
| `teshi-vX.Y.Z-x86_64-pc-windows-msvc.zip` | Portable full bundle. Check-only; replace the folder manually |
| `teshi-bridge-vX.Y.Z.zip` | Chrome extension for locator recording (load unpacked) |

There is no separate desktop-only installer. The setup.exe, MSI, and ZIP ship the same CLI, desktop, helper, and web assets.

```powershell
# User install with in-app updates (recommended)
# Run teshi-<tag>-x64-setup.exe from GitHub Releases, then:
teshi web
teshi desktop
teshi update

# Machine-wide / WinGet (no in-app replace)
winget install teshi-org.teshi
```

## Linux and macOS

| Asset | Contents |
|-------|----------|
| `teshi-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` | Portable CLI and helper |
| `teshi-vX.Y.Z-aarch64-apple-darwin.tar.gz` | Portable CLI and helper |

Extract to a dedicated directory and keep that directory on `PATH`. These archives cannot self-update; install a newer archive from GitHub Releases when `teshi update --check` reports one.

## Application updates

Shipped release binaries include `teshi update`. The first updater-enabled **setup.exe** must be installed manually; later upgrades on that user install use the command or native desktop **Check for Updates**.

| Installation | Updater action |
|--------------|----------------|
| Windows setup.exe (`%LOCALAPPDATA%\Programs\teshi`, `kind: exe`) | Download the new setup.exe, silent `/update=true`, helper replaces managed files |
| Portable ZIP/tar.gz with `teshi-bundle.json` | Check-only; do not self-update |
| Registered Teshi MSI / WinGet | Check-only; install setup.exe if you want in-app updates |
| Explicit external package-manager marker | Prints the manager guidance; never overwrites files |
| Source/Cargo/`cargo run` builds | Check-only with manual-install guidance |

`teshi update --check` never downloads payloads. Installation requires confirmation or `--yes`. Checksums in `SHA256SUMS` and `update-manifest.json` detect corrupt or mismatched GitHub assets; they are not an independent signature if GitHub itself is compromised.

Updating Teshi does not refresh skills or browser extensions that were copied outside the bundle. Use `teshi install-skill` and the existing extension reload workflow for those.

See [CLI usage](cli-usage.md) for flags and exit codes.

## From source

```bash
cargo build --release
```

Requires the Rust toolchain (via [rustup](https://rustup.rs/)). See [Development Guide](development.md) for details.
