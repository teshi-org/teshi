/**
 * teshi-bridge: HTTP heartbeat to local bridge (MV3-safe; no WebSocket).
 */

const DISCOVERY_URL = "http://127.0.0.1:17373/v1/bridge";
const HEARTBEAT_URL = "http://127.0.0.1:17373/v1/bridge/heartbeat";
const RESPONSE_URL = "http://127.0.0.1:17373/v1/bridge/response";
const HEARTBEAT_MS = 1000;
const ALARM_NAME = "teshi-bridge-heartbeat";

const HIGHLIGHT_CONFIG = {
  showInfo: true,
  showStyles: true,
  showRulers: false,
  showExtensionLines: false,
  contentColor: { r: 37, g: 99, b: 235, a: 0.35 },
  borderColor: { r: 37, g: 99, b: 235, a: 0.9 },
};

/** @type {number | null} */
let attachedTabId = null;
let cachedProjectRoot = "";
let heartbeatRunning = false;

async function getActiveTab() {
  const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
  return tabs[0] ?? null;
}

async function detachIfNeeded() {
  if (attachedTabId === null) {
    return;
  }
  try {
    await chrome.debugger.detach({ tabId: attachedTabId });
  } catch {
    // Tab closed or already detached.
  }
  attachedTabId = null;
}

async function attachActiveTab() {
  const tab = await getActiveTab();
  if (!tab?.id) {
    throw new Error("no active tab in the current window");
  }
  if (attachedTabId === tab.id) {
    return tab;
  }
  await detachIfNeeded();
  await chrome.debugger.attach({ tabId: tab.id }, "1.3");
  attachedTabId = tab.id;
  for (const domain of ["Accessibility", "DOM", "Runtime", "Overlay", "Page"]) {
    await chrome.debugger.sendCommand({ tabId: tab.id }, `${domain}.enable`, {});
  }
  return tab;
}

async function cdp(tabId, method, params = {}) {
  return chrome.debugger.sendCommand({ tabId }, method, params);
}

async function collectInteractiveElements(tabId) {
  const { result } = await cdp(tabId, "Runtime.evaluate", {
    expression: `(() => {
      const sel = "button, [role='button'], input, input[type='submit'], select, a[href], [role='link'], textarea";
      const elements = Array.from(document.querySelectorAll(sel));
      return elements.slice(0, 60).map(el => ({
        tag: el.tagName.toLowerCase(),
        text: (el.innerText || el.value || el.getAttribute('aria-label') || '').trim().slice(0, 120),
        id: el.id || null,
        testId: el.getAttribute('data-testid'),
        role: el.getAttribute('role'),
        classes: el.className || null,
      }));
    })()`,
    returnByValue: true,
  });
  return result?.value ?? [];
}

async function getPageSnapshot() {
  const tab = await attachActiveTab();
  let accessibility_tree;
  try {
    accessibility_tree = await cdp(tab.id, "Accessibility.getFullAXTree", {});
  } catch (err) {
    accessibility_tree = { error: String(err) };
  }
  const interactive_elements = await collectInteractiveElements(tab.id);
  return {
    ok: true,
    url: tab.url ?? "",
    title: tab.title ?? "",
    accessibility_tree,
    interactive_elements,
  };
}

async function highlightSelector(selector) {
  if (!selector) {
    return { ok: false, error: "selector is required" };
  }
  const tab = await attachActiveTab();
  await cdp(tab.id, "Overlay.hideHighlight", {});
  const { result } = await cdp(tab.id, "Runtime.evaluate", {
    expression: `document.querySelector(${JSON.stringify(selector)})`,
    returnByValue: false,
  });
  const objectId = result?.objectId;
  if (!objectId) {
    return { ok: false, error: `selector matched no elements: ${selector}` };
  }
  const { nodeId } = await cdp(tab.id, "DOM.requestNode", { objectId });
  if (!nodeId) {
    return { ok: false, error: "could not resolve node id" };
  }
  await cdp(tab.id, "Overlay.highlightNode", {
    highlightConfig: HIGHLIGHT_CONFIG,
    nodeId,
  });
  return { ok: true, selector, node_id: nodeId };
}

async function clearHighlight() {
  if (attachedTabId === null) {
    return { ok: true };
  }
  try {
    await cdp(attachedTabId, "Overlay.hideHighlight", {});
  } catch {
    // ignore
  }
  return { ok: true };
}

async function captureFrame() {
  const tab = await attachActiveTab();
  const shot = await cdp(tab.id, "Page.captureScreenshot", {
    format: "jpeg",
    quality: 65,
  });
  return {
    type: "frame",
    ok: true,
    cmd: "frame_stream",
    data: shot?.data ?? "",
    url: tab.url ?? "",
    title: tab.title ?? "",
  };
}

async function handleCmd(msg) {
  const { cmd, request_id: requestId, selector } = msg;
  let body;
  try {
    if (cmd === "get_page_snapshot") {
      body = await getPageSnapshot();
    } else if (cmd === "highlight_selector") {
      body = await highlightSelector(selector);
    } else if (cmd === "clear_highlight") {
      body = await clearHighlight();
    } else if (cmd === "navigate") {
      body = {
        ok: false,
        error: "navigate is not supported in chrome mode; change URL in Chrome",
      };
    } else {
      body = { ok: false, error: `unknown cmd: ${cmd}` };
    }
  } catch (err) {
    body = {
      ok: false,
      error: String(err),
      hint: "Close DevTools on this tab if attach fails.",
    };
  }
  return { type: "response", request_id: requestId, cmd, ...body };
}

function setBadge(connected) {
  chrome.action.setBadgeText({ text: connected ? "OK" : "" });
  chrome.action.setBadgeBackgroundColor({ color: "#16a34a" });
}

async function ensureProjectRoot() {
  if (cachedProjectRoot) {
    return cachedProjectRoot;
  }
  try {
    const res = await fetch(DISCOVERY_URL);
    if (!res.ok) {
      return "";
    }
    const info = await res.json();
    if (info.mode === "chrome" && info.project_root) {
      cachedProjectRoot = info.project_root;
    }
  } catch {
    // Bridge offline.
  }
  return cachedProjectRoot;
}

async function heartbeatOnce() {
  const projectRoot = await ensureProjectRoot();
  if (!projectRoot) {
    setBadge(false);
    return;
  }
  const tab = await getActiveTab();
  let res;
  try {
    res = await fetch(HEARTBEAT_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        project_root: projectRoot,
        url: tab?.url ?? "",
        title: tab?.title ?? "",
      }),
    });
  } catch {
    setBadge(false);
    return;
  }
  if (!res.ok) {
    setBadge(false);
    cachedProjectRoot = "";
    return;
  }
  let data;
  try {
    data = await res.json();
  } catch {
    setBadge(false);
    return;
  }
  if (!data.ok) {
    setBadge(false);
    // Recovery for project switches: refresh discovery metadata on next tick.
    cachedProjectRoot = "";
    return;
  }
  setBadge(true);
  try {
    const frame = await captureFrame();
    await fetch(RESPONSE_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(frame),
    });
  } catch {
    // Frame streaming is best-effort; heartbeat should still keep connection alive.
  }
  if (data.cmd) {
    const reply = await handleCmd(data.cmd);
    await fetch(RESPONSE_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(reply),
    });
  }
}

async function heartbeatLoop() {
  if (heartbeatRunning) {
    return;
  }
  heartbeatRunning = true;
  try {
    await heartbeatOnce();
  } finally {
    heartbeatRunning = false;
  }
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type === "connect_now") {
    cachedProjectRoot = "";
    void (async () => {
      heartbeatRunning = false;
      await heartbeatOnce();
      sendResponse({ ok: true });
    })();
    return true;
  }
  return false;
});

chrome.runtime.onInstalled.addListener(() => {
  chrome.alarms.create(ALARM_NAME, { periodInMinutes: 1 });
  void heartbeatLoop();
});

chrome.runtime.onStartup.addListener(() => {
  void heartbeatLoop();
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === ALARM_NAME) {
    void heartbeatLoop();
  }
});

chrome.tabs.onActivated.addListener(() => {
  void detachIfNeeded();
  void heartbeatLoop();
});

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (tab.active && changeInfo.url) {
    void heartbeatLoop();
  }
});

chrome.debugger.onDetach.addListener((source) => {
  if (source.tabId === attachedTabId) {
    attachedTabId = null;
  }
});

setInterval(() => {
  void heartbeatLoop();
}, HEARTBEAT_MS);
void heartbeatLoop();
