const statusEl = document.getElementById("status");
const connectBtn = document.getElementById("connect");
const labelInput = document.getElementById("profile-label");
const saveLabelBtn = document.getElementById("save-label");
const detailsEl = document.getElementById("details");
const permissionButtons = Array.from(document.querySelectorAll("[data-permission]"));

function sendRuntimeMessage(message) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendMessage(message, (response) => {
      if (chrome.runtime.lastError) {
        reject(new Error(chrome.runtime.lastError.message));
        return;
      }
      resolve(response);
    });
  });
}

async function refreshStatus() {
  try {
    const info = await sendRuntimeMessage({ type: "get_bridge_status" });
    labelInput.value = info.identity?.profile_label || "";
    const suffix = info.identity?.extension_instance_id
      ? info.identity.extension_instance_id.slice(0, 8)
      : "unknown";
    detailsEl.textContent = `Session ${suffix} · extension ${info.extension_version || "unknown"} · protocol ${info.protocol_version || "unknown"}`;
    const permissionStatus = info.optional_permissions || {};
    for (const button of permissionButtons) {
      const key = button.dataset.permission;
      const statusKey = key === "contentSettings" ? "content_settings" : key === "management" ? "extension_management" : key;
      const allowed = Boolean(permissionStatus[statusKey]);
      button.textContent = allowed ? `${button.textContent.replace(/^Allowed: |^Allow /, "")} (allowed)` : button.textContent.replace(/ \(allowed\)$/, "");
      button.disabled = allowed;
    }
    if (info.connected && info.compatible !== false) {
      statusEl.textContent = `Connected${info.identity?.profile_label ? ` · ${info.identity.profile_label}` : ""}`;
      connectBtn.textContent = "Connected";
      connectBtn.disabled = true;
    } else {
      connectBtn.textContent = "Connect to teshi";
      connectBtn.disabled = false;
      if (info.code === "incompatible_browser_session") {
        statusEl.textContent = `Incompatible protocol — broker requires ${info.required_protocol_version}`;
      } else {
        statusEl.textContent = info.error || "Bridge offline — start Connect Chrome in teshi";
      }
    }
  } catch (err) {
    statusEl.textContent = `Extension status unavailable: ${err}`;
    connectBtn.disabled = false;
  }
}

connectBtn.addEventListener("click", async () => {
  connectBtn.disabled = true;
  statusEl.textContent = "Sending heartbeat…";
  try {
    await sendRuntimeMessage({ type: "connect_now" });
    await new Promise((r) => setTimeout(r, 400));
    await refreshStatus();
  } catch (err) {
    statusEl.textContent = `Error: ${err}`;
    connectBtn.disabled = false;
  }
});

for (const button of permissionButtons) {
  button.addEventListener("click", async () => {
    const permission = button.dataset.permission;
    button.disabled = true;
    try {
      const granted = await chrome.permissions.request({ permissions: [permission] });
      statusEl.textContent = granted ? `Allowed ${permission}` : `Permission denied: ${permission}`;
      await sendRuntimeMessage({ type: "connect_now" });
      await refreshStatus();
    } catch (err) {
      statusEl.textContent = `Permission request failed: ${err}`;
      button.disabled = false;
    }
  });
}

saveLabelBtn.addEventListener("click", async () => {
  saveLabelBtn.disabled = true;
  try {
    await sendRuntimeMessage({
      type: "set_profile_label",
      profile_label: labelInput.value,
    });
    await refreshStatus();
  } catch (err) {
    statusEl.textContent = `Could not save profile label: ${err}`;
  } finally {
    saveLabelBtn.disabled = false;
  }
});

void refreshStatus();
