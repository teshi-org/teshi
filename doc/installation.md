# Installation

## Windows

| Asset | Contents |
|-------|----------|
| `teshi-vX.Y.Z-x64.msi` / WinGet | Full package: `teshi.exe`, `teshi-desktop.exe`, and web UI assets |
| `teshi-vX.Y.Z-x86_64-pc-windows-msvc.zip` | Portable full bundle (same layout as MSI, no installer) |
| `teshi-desktop-vX.Y.Z-x64.msi` | Tauri desktop shell only (optional if you already use the full MSI) |
| `teshi-bridge-vX.Y.Z.zip` | Chrome extension for locator recording (load unpacked) |

```powershell
winget install teshi-org.teshi
teshi web
teshi desktop
```

## From source

```bash
cargo build --release
```

Requires the Rust toolchain (via [rustup](https://rustup.rs/)). See [Development Guide](development.md) for details.
