# teshi-bridge (Chrome extension)

Connects your **active Chrome tab** to [teshi-desktop](../../desktop/README.md) for BDD locator recording with real login sessions.

## Install (developer / unpacked)

1. Open Chrome and go to `chrome://extensions`.
2. Enable **Developer mode** (top right).
3. Click **Load unpacked** and select this directory: `extension/teshi-bridge`.
4. Pin the extension if you want to see when it is active (optional).

## Use with teshi-desktop

1. In **Google Chrome**, open and activate the tab you want to test (logged-in app).
2. In teshi-desktop, open your project and click **Connect Chrome** in the Browser panel.
3. The extension sends an **HTTP heartbeat every second** to `127.0.0.1:17373` (no WebSocket — survives MV3 sleep better).
4. teshi-desktop should show **extension: connected** while Chrome stays open on your tab. Optional: click the extension icon → **Connect to teshi** to force one heartbeat.
5. Select a Gherkin step and run the **bdd-locator** agent skill in the terminal.

After changing extension files, click **Reload** on `chrome://extensions` for teshi-bridge.

If connection fails:

- Confirm the extension is enabled.
- Close DevTools on the target tab (debugger attach conflicts).
- Click **Connect Chrome** again after switching tabs.

## Permissions

- **debugger** — CDP snapshot and element highlight on the active tab only.
- **activeTab** — limit scope to the tab you are viewing.
- **127.0.0.1** — local bridge discovery and WebSocket (no remote hosts).
