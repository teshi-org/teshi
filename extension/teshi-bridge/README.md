# teshi-bridge (Chrome extension)

Connects your **Chrome window tabs** to [teshi-desktop](../../desktop/README.md) for BDD locator recording with real login sessions. One active tab is mirrored at a time; teshi-desktop can list tabs and switch which tab is active.

## Icons

Toolbar and store icons are PNGs under `icons/`, generated from the same asset as teshi-desktop (`desktop/src-tauri/icons/icon.ico`). After updating the desktop icon, regenerate sizes (16 / 32 / 48 / 128) and reload the extension in Chrome.

## Install (unpacked)

1. Open Chrome and go to `chrome://extensions`.
2. Enable **Developer mode** (top right).
3. Click **Load unpacked** and select:
   - Installed MSI path (recommended): `C:\Program Files\teshi\share\teshi-bridge`
   - Repo path (development): `extension/teshi-bridge`
4. Pin the extension if you want to see when it is active (optional).

## Use with teshi-desktop

1. In **Google Chrome** (or another Chromium browser), open the tabs you need in one window.
2. In teshi-desktop, open your project and click **Connect Chrome** in the Browser panel.
3. The extension sends an **HTTP heartbeat every second** to `127.0.0.1:17373` (no WebSocket — survives MV3 sleep better). Each heartbeat includes the **current window tab list**.
4. When connected, use the **tab strip** in the Browser panel to switch tabs, or activate tabs in Chrome directly. Optional: click the extension icon → **Connect to teshi** to force one heartbeat.
5. Select a Gherkin step and run the **bdd-locator** agent skill in the terminal.

After changing extension files, click **Reload** on `chrome://extensions` for teshi-bridge.

If connection fails:

- Confirm the extension is enabled.
- Close DevTools on the target tab (debugger attach conflicts).
- Click **Connect Chrome** again after switching tabs.
- Only `http://`, `https://`, and `file://` tabs can be debugged.

## Permissions

- **debugger** — CDP snapshot, screenshot stream, and element highlight on the active tab.
- **tabs** — list tabs in the current window and activate a tab when teshi-desktop requests it.
- **activeTab** — limit scope to user-visible browsing.
- **alarms** — periodic wake for MV3 service worker.
- **127.0.0.1** — local bridge discovery and heartbeat (no remote hosts).

## Protocol (extension ↔ bridge)

Heartbeat `POST /v1/bridge/heartbeat` body includes `project_root`, `url`, `title`, `active_tab_id`, and `tabs[]` (`id`, `title`, `url`, `active`, `favIconUrl`, `debuggable`).

Commands delivered on the heartbeat response (`cmd` field) include `get_page_snapshot`, `highlight_selector`, `clear_highlight`, and `activate_tab` (with `tab_id`). Replies are posted to `/v1/bridge/response`. JPEG frames include `tab_id` for the captured tab.
