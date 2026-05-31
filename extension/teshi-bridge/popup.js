const statusEl = document.getElementById("status");
const connectBtn = document.getElementById("connect");

const DISCOVERY_URL = "http://127.0.0.1:17373/v1/bridge";
const HEARTBEAT_URL = "http://127.0.0.1:17373/v1/bridge/heartbeat";

async function refreshStatus() {
  try {
    const res = await fetch(DISCOVERY_URL);
    if (!res.ok) {
      statusEl.textContent = "Bridge offline — click Connect Chrome in teshi-desktop";
      return;
    }
    const info = await res.json();
    if (info.extension_connected) {
      statusEl.textContent = `Connected · ${info.page_url || "active tab"}`;
      connectBtn.textContent = "Connected";
      connectBtn.disabled = true;
    } else {
      connectBtn.textContent = "Connect to teshi";
      connectBtn.disabled = false;
      if (info.page_url) {
        statusEl.textContent = "Bridge online, extension not linked — click Connect";
      } else {
        statusEl.textContent = "Bridge online — click Connect (keep app tab active in Chrome)";
      }
    }
  } catch {
    statusEl.textContent = "Bridge offline — start Connect Chrome in teshi-desktop";
    connectBtn.disabled = false;
  }
}

connectBtn.addEventListener("click", async () => {
  connectBtn.disabled = true;
  statusEl.textContent = "Sending heartbeat…";
  try {
    const discovery = await fetch(DISCOVERY_URL);
    if (!discovery.ok) {
      statusEl.textContent = "Bridge offline — use Connect Chrome in teshi-desktop first";
      connectBtn.disabled = false;
      return;
    }
    const info = await discovery.json();
    const tab = await chrome.tabs.query({ active: true, currentWindow: true });
    const active = tab[0];
    const hb = await fetch(HEARTBEAT_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        project_root: info.project_root ?? "",
        url: active?.url ?? "",
        title: active?.title ?? "",
      }),
    });
    const result = await hb.json();
    if (!hb.ok || !result.ok) {
      statusEl.textContent = result.error ?? "Heartbeat failed";
      connectBtn.disabled = false;
      return;
    }
    chrome.runtime.sendMessage({ type: "connect_now" });
    await new Promise((r) => setTimeout(r, 400));
    await refreshStatus();
  } catch (err) {
    statusEl.textContent = `Error: ${err}`;
    connectBtn.disabled = false;
  }
});

void refreshStatus();
