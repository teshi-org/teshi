# AI install runbook

Follow this file to install the teshi CLI, load the teshi-bridge Chrome extension, and add the teshi agent skills. Do not download `SKILL.md` from GitHub. Do not automate `chrome://extensions`.

Ask before choosing a teshi-bridge unpack directory. Show `teshi install-skill --dry-run` and wait for the user to confirm before writing skills.

## 1. Install the teshi CLI

Prefer Windows WinGet:

```powershell
winget install teshi-org.teshi
teshi --version
```

If `teshi` is missing from the current shell after WinGet, open a new terminal or refresh `PATH`, then retry `teshi --version`.

If WinGet is unavailable, download the latest GitHub Release archive for this machine (`teshi-v*-x86_64-pc-windows-msvc.zip` on Windows) from https://github.com/teshi-org/teshi/releases, unpack it, and put `teshi` on `PATH`.

If that also fails, build from source:

```bash
git clone https://github.com/teshi-org/teshi.git
cd teshi
cargo build --release --bin teshi
```

Then run the built binary (for example `target/release/teshi --version`).

## 2. Load teshi-bridge (human step)

Ask the user which directory should hold the unpacked Chrome extension.

- If teshi was installed with WinGet/MSI, prefer the bundled copy at `C:\Program Files\teshi\share\teshi-bridge` (or the portable zip's `share/teshi-bridge` next to `teshi.exe`).
- Otherwise download the **same release tag** as `teshi --version`, using `teshi-bridge-vX.Y.Z.zip`, and unpack it into the directory the user named.

Then ask the user to:

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Choose **Load unpacked** and select the `teshi-bridge` directory that contains `manifest.json`. If it is already loaded, click **Reload**.
4. Open the extension popup and connect.
5. Keep at least one `http://` or `https://` tab open. `chrome://` pages are not valid targets.

The agent cannot click those Chrome UI controls. Wait until the user confirms the extension is loaded.

## 3. Install agent skills

Print the plan first:

```bash
teshi install-skill --dry-run
```

Show the output to the user. After they confirm:

- On an interactive TTY: `teshi install-skill`
- From an agent/non-TTY shell: `teshi install-skill --yes`

Do not pass `--yes` until the dry-run has been shown and the user has agreed. Skills are copied from the local teshi share/checkout tree only.

## 4. Verify

```bash
teshi --version
teshi browser sessions
```

`teshi browser sessions` may start the local broker. If no compatible extension session is connected, the output must stay diagnostic and must not crash. Fix CLI/PATH, Python/venv, or extension connection using that diagnostic; do not fetch Skills from GitHub.
