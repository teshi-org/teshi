# Installation

Coding agents should follow [AI_INSTALL.md](../AI_INSTALL.md) to install the teshi CLI, load teshi-bridge, and run `teshi install-skill`.

## Windows

| Asset | Contents |
|-------|----------|
| `teshi-vX.Y.Z-x64-setup.exe` | **User install (in-app updates).** Per-user copy under `%LOCALAPPDATA%\Programs\teshi`, no administrator prompt |
| `teshi-vX.Y.Z-x64.msi` | Machine-wide package for IT deployments. Check-only in `teshi update`; does not self-update |
| `teshi-vX.Y.Z-x86_64-pc-windows-msvc.zip` | Portable full bundle. Check-only; replace the folder manually |
| `teshi-bridge-vX.Y.Z.zip` | Chrome extension for locator recording (load unpacked) |

There is no separate desktop-only installer. The setup.exe, MSI, and ZIP ship the same CLI, desktop, helper, and web assets.

```powershell
# User install with in-app updates (recommended)
# Run teshi-<tag>-x64-setup.exe from GitHub Releases, then:
teshi web
teshi desktop
teshi update

# Machine-wide MSI (no in-app replace; run from an elevated shell)
msiexec /i teshi-<tag>-x64.msi /qn
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
| Registered Teshi MSI | Check-only; upgrade using a newer MSI or contact your administrator |
| Explicit external package-manager marker | Prints the manager guidance; never overwrites files |
| Source/Cargo/`cargo run` builds | Check-only with manual-install guidance |

`teshi update --check` never downloads payloads. `teshi update` starts installation automatically after a verified candidate is found; `--yes` remains a compatibility flag and does not bypass operating-system or process-safety checks. Checksums in `SHA256SUMS` and `update-manifest.json` detect corrupt or mismatched GitHub assets; they are not an independent signature if GitHub itself is compromised.

Downloads use a persistent per-user cache at `%LOCALAPPDATA%\teshi\updates\payloads`
on Windows, keyed by release ID, asset ID and SHA256. Partial transfers resume on the next
attempt, and completed payloads are revalidated before reuse. Installation failures do
not delete verified downloads. `teshi update --retry-install` retries the cached candidate
offline; `teshi update --status` explains the last failed phase and whether a cached
payload was retained. Ordinary `update` can still contact GitHub for newer metadata.

Payloads are retained until explicitly removed; there is no automatic eviction in this
version. To reclaim disk space, remove obsolete entry directories under `payloads` only
when no updater is running. Removing a cache entry requires downloading it again; do not
remove installation `.teshi-update/pending.json` or transaction journals to clear cache.

Closing the TUI or browser does not necessarily stop its background daemon or MCP server.
The updater now requests graceful shutdown of registered daemons from the same install
and reports remaining processes. Use `teshi daemon stop` from that daemon's project, or
explicitly run `teshi update --force-close` / `teshi update --retry-install --force-close`
to permit termination after the graceful-exit deadline. Force close can lose unsaved work;
it targets verified process generations in that installation, not other Teshi copies.
The setup.exe-only self-update eligibility rules above still apply.

MSI installations skip the updater's installation-level process and daemon registration.
Running `teshi web` and stopping its daemon do not require write access to the MSI
installation directory. Project daemon manifests and logs remain in the project's
`.teshi` directory (or the user-level daemon directory when no project is selected).

In-app setup uses `/SUPPRESSMSGBOXES` and `/SP-` as well as `/VERYSILENT`, so
ordinary installer failures return an error instead of waiting on a hidden dialog.
Installer-driven application closing is disabled; the helper owns the verified,
installation-scoped close policy described above.

### Proxies and TLS

Update checks and downloads use reqwest's proxy discovery:

- Environment variables `HTTPS_PROXY`/`https_proxy`, `ALL_PROXY`/`all_proxy`, and `NO_PROXY`/`no_proxy` (the more specific HTTPS variable wins over `ALL_PROXY`).
- When those variables are unset, Teshi also reads the static Windows **Internet Settings** proxy (`ProxyEnable`/`ProxyServer`/`ProxyOverride`) and the equivalent macOS manual proxy. Process-level environment variables still take precedence.
- Desktop launched from Explorer only sees user or machine environment variables that exist at logon. Set a persistent `HTTPS_PROXY` or the Windows system proxy if GitHub is not reachable directly.
- PAC/WPAD automatic proxy scripts and NTLM/Kerberos integrated authentication are not implemented. Use an explicit `http://proxy-host:port` URL, including Basic credentials in the proxy URL when the proxy requires them.
- Local browser-bridge requests from Desktop and `teshi web` ignore proxies (`no_proxy`) so loopback traffic is never sent to a corporate proxy.

On Windows, update TLS verifies GitHub (or a TLS-inspecting proxy) with a snapshot of the Windows root store and does **not** contact Microsoft CRL/OCSP endpoints. Certificate chain, hostname, expiry, and signatures are still checked. Teshi does not disable TLS verification. If an intercepting proxy re-signs GitHub with an enterprise CA, install that CA in the Windows trust store; otherwise the update client rejects the connection.

Updating Teshi does not refresh skills or browser extensions that were copied outside the bundle. Use `teshi install-skill` and the existing extension reload workflow for those.

See [CLI usage](cli-usage.md) for flags and exit codes.

## From source

```bash
cargo build --release
```

Requires the Rust toolchain (via [rustup](https://rustup.rs/)). See [Development Guide](development.md) for details.
