/**
 * teshi-bridge: HTTP heartbeat (metadata + commands) and CDP screencast preview.
 */

const DISCOVERY_URL = "http://127.0.0.1:17373/v1/bridge";
const HEARTBEAT_URL = "http://127.0.0.1:17373/v1/bridge/heartbeat";
const RESPONSE_URL = "http://127.0.0.1:17373/v1/bridge/response";
const FRAME_UPLOAD_URL = "http://127.0.0.1:17373/v1/bridge/frame";
/** Metadata heartbeat (tabs, URL) — no screenshot. */
const HEARTBEAT_MS = 1500;
/** Target ~10 fps via screencastFrameAck throttling. */
const SCREENCAST_MIN_INTERVAL_MS = 100;
const STREAM_ERROR_DEBOUNCE_MS = 5000;
const TAB_EVENT_DEBOUNCE_MS = 400;
const ALARM_NAME = "teshi-bridge-heartbeat";
const STREAM_BOUNDS_W = 1920;
const STREAM_BOUNDS_H = 1080;
const STREAM_JPEG_QUALITY = 70;
const BRIDGE_POST_RETRIES = 2;
const BRIDGE_FETCH_TIMEOUT_MS = 15_000;
const STREAM_WS_RECONNECT_BASE_MS = 500;
const STREAM_WS_RECONNECT_MAX_MS = 8000;
const FRAME_MAGIC_BYTES = new Uint8Array([0x54, 0x53, 0x48, 0x31]);

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
let extensionFrameWsUrl = "";
let heartbeatRunning = false;
let pendingStreamRestart = false;
let tabEventTimer = null;
/** Pause preview while CDP locator commands run on the active tab. */
let streamPaused = false;
let lastStreamErrorPostedAt = 0;
/** Page domain enabled on attachedTabId for stream capture. */
let streamPageEnabled = false;
/** @type {WebSocket | null} */
let streamWs = null;
let streamWsReconnectDelay = STREAM_WS_RECONNECT_BASE_MS;
let screencastActive = false;
/** @type {number | null} */
let streamSessionTabId = null;
let streamSeq = 0;
let lastScreencastPublishAt = 0;
let screencastListenerRegistered = false;
/** When true, send raw JPEG via POST /v1/bridge/frame instead of WebSocket. */
let streamUseHttpFallback = false;
/** @type {ReturnType<typeof setInterval> | null} */
let httpFallbackIntervalId = null;

function isTabDebuggable(url) {
  if (!url) {
    return false;
  }
  try {
    const parsed = new URL(url);
    return (
      parsed.protocol === "http:" ||
      parsed.protocol === "https:" ||
      parsed.protocol === "file:"
    );
  } catch {
    return false;
  }
}

async function getActiveTab() {
  const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
  return tabs[0] ?? null;
}

async function listWindowTabs() {
  const tabs = await chrome.tabs.query({ currentWindow: true });
  const active = tabs.find((t) => t.active);
  return {
    active_tab_id: active?.id ?? null,
    tabs: tabs.map((t) => ({
      id: t.id,
      title: (t.title || t.url || "Untitled").slice(0, 200),
      url: t.url ?? "",
      active: Boolean(t.active),
      favIconUrl: t.favIconUrl ?? "",
      debuggable: isTabDebuggable(t.url),
    })),
  };
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
  streamPageEnabled = false;
}

async function ensureStreamDebugger(tab) {
  if (!tab?.id) {
    throw new Error("no active tab in the current window");
  }
  if (attachedTabId !== tab.id) {
    await detachIfNeeded();
    await chrome.debugger.attach({ tabId: tab.id }, "1.3");
    attachedTabId = tab.id;
    streamPageEnabled = false;
  }
  if (!streamPageEnabled) {
    await chrome.debugger.sendCommand({ tabId: tab.id }, "Page.enable", {});
    streamPageEnabled = true;
  }
}

function base64ToBytes(b64) {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

function encodeTsh1Frame(meta, jpegBytes) {
  const metaJson = new TextEncoder().encode(JSON.stringify(meta));
  const packet = new Uint8Array(8 + metaJson.length + jpegBytes.length);
  packet.set(FRAME_MAGIC_BYTES, 0);
  const view = new DataView(packet.buffer);
  view.setUint32(4, metaJson.length, true);
  packet.set(metaJson, 8);
  packet.set(jpegBytes, 8 + metaJson.length);
  return packet;
}

function closeStreamWebSocket() {
  if (streamWs) {
    try {
      streamWs.close();
    } catch {
      // ignore
    }
    streamWs = null;
  }
}

function clearHttpFallbackInterval() {
  if (httpFallbackIntervalId !== null) {
    clearInterval(httpFallbackIntervalId);
    httpFallbackIntervalId = null;
  }
}

async function refreshExtensionFrameWsUrl() {
  try {
    const res = await fetch(DISCOVERY_URL);
    if (!res.ok) {
      return false;
    }
    const info = await res.json();
    if (info.extension_frame_ws_url) {
      extensionFrameWsUrl = String(info.extension_frame_ws_url);
      return true;
    }
  } catch {
    // Bridge offline.
  }
  return false;
}

async function connectStreamWebSocket() {
  if (!extensionFrameWsUrl || !cachedProjectRoot) {
    return false;
  }
  if (streamWs?.readyState === WebSocket.OPEN) {
    return true;
  }
  closeStreamWebSocket();
  return new Promise((resolve) => {
    let settled = false;
    const ws = new WebSocket(extensionFrameWsUrl);
    ws.binaryType = "arraybuffer";
    ws.onopen = () => {
      streamWs = ws;
      streamWsReconnectDelay = STREAM_WS_RECONNECT_BASE_MS;
      ws.send(
        JSON.stringify({
          type: "stream_hello",
          project_root: cachedProjectRoot,
        }),
      );
      if (!settled) {
        settled = true;
        resolve(true);
      }
    };
    ws.onmessage = (event) => {
      if (typeof event.data !== "string") {
        return;
      }
      try {
        const ack = JSON.parse(event.data);
        if (ack.type === "stream_hello_ack" && ack.ok === false) {
          void postFrameErrorDebounced(
            ack.error || "extension stream hello rejected",
          );
        }
      } catch {
        // ignore
      }
    };
    ws.onclose = () => {
      streamWs = null;
      if (!settled) {
        settled = true;
        resolve(false);
      }
      if (screencastActive && cachedProjectRoot && !streamPaused) {
        setTimeout(() => {
          void connectStreamWebSocket();
        }, streamWsReconnectDelay);
        streamWsReconnectDelay = Math.min(
          streamWsReconnectDelay * 2,
          STREAM_WS_RECONNECT_MAX_MS,
        );
      }
    };
    ws.onerror = () => {
      if (!settled) {
        settled = true;
        resolve(false);
      }
    };
  });
}

async function postFrameHttpFallback(jpegBytes, tabId, url) {
  const u = new URL(FRAME_UPLOAD_URL);
  u.searchParams.set("project_root", cachedProjectRoot);
  u.searchParams.set("tab_id", String(tabId));
  u.searchParams.set("url", url);
  const res = await fetch(u.toString(), {
    method: "POST",
    headers: { "Content-Type": "image/jpeg" },
    body: jpegBytes,
  });
  if (!res.ok) {
    throw new Error(`frame upload HTTP ${res.status}`);
  }
}

async function publishScreencastFrame(jpegBytes, tabId, url) {
  const meta = {
    tab_id: tabId,
    url,
    seq: (streamSeq += 1),
  };
  if (streamUseHttpFallback) {
    await postFrameHttpFallback(jpegBytes, tabId, url);
    return;
  }
  if (streamWs?.readyState === WebSocket.OPEN) {
    streamWs.send(encodeTsh1Frame(meta, jpegBytes));
    return;
  }
  await postFrameHttpFallback(jpegBytes, tabId, url);
}

async function handleScreencastFrame(params, tabId) {
  if (streamPaused || !screencastActive || streamSessionTabId !== tabId) {
    return;
  }
  const sessionId = params.sessionId;
  const now = Date.now();
  if (now - lastScreencastPublishAt < SCREENCAST_MIN_INTERVAL_MS) {
    if (sessionId) {
      try {
        await chrome.debugger.sendCommand(
          { tabId },
          "Page.screencastFrameAck",
          { sessionId },
        );
      } catch {
        // ignore
      }
    }
    return;
  }
  lastScreencastPublishAt = now;
  const jpegBytes = base64ToBytes(params.data || "");
  if (!jpegBytes.length) {
    return;
  }
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  try {
    await publishScreencastFrame(jpegBytes, tabId, tab?.url ?? "");
  } catch (err) {
    await postFrameErrorDebounced(captureErrorMessage(err));
  }
  if (sessionId) {
    try {
      await chrome.debugger.sendCommand(
        { tabId },
        "Page.screencastFrameAck",
        { sessionId },
      );
    } catch {
      // ignore
    }
  }
}

function ensureScreencastDebuggerListener() {
  if (screencastListenerRegistered) {
    return;
  }
  screencastListenerRegistered = true;
  chrome.debugger.onEvent.addListener((source, method, params) => {
    if (method !== "Page.screencastFrame") {
      return;
    }
    if (!screencastActive || source.tabId !== streamSessionTabId) {
      return;
    }
    void handleScreencastFrame(params, source.tabId);
  });
}

async function stopStreamSession() {
  screencastActive = false;
  clearHttpFallbackInterval();
  closeStreamWebSocket();
  streamUseHttpFallback = false;
  if (streamSessionTabId !== null) {
    try {
      await chrome.debugger.sendCommand(
        { tabId: streamSessionTabId },
        "Page.stopScreencast",
        {},
      );
    } catch {
      // ignore
    }
  }
  streamSessionTabId = null;
  lastScreencastPublishAt = 0;
}

async function runHttpFallbackTick(tabId) {
  if (!screencastActive || streamPaused || streamSessionTabId !== tabId) {
    return;
  }
  const now = Date.now();
  if (now - lastScreencastPublishAt < SCREENCAST_MIN_INTERVAL_MS) {
    return;
  }
  try {
    const tab = await chrome.tabs.get(tabId);
    await ensureStreamDebugger(tab);
    const result = await chrome.debugger.sendCommand(
      { tabId },
      "Page.captureScreenshot",
      {
        format: "jpeg",
        quality: STREAM_JPEG_QUALITY,
        fromSurface: true,
      },
    );
    const raw = result?.data;
    if (!raw) {
      return;
    }
    lastScreencastPublishAt = now;
    const jpegBytes = base64ToBytes(raw);
    await publishScreencastFrame(jpegBytes, tabId, tab.url ?? "");
  } catch (err) {
    await postFrameErrorDebounced(captureErrorMessage(err));
  }
}

async function startHttpFallbackSession(tab) {
  streamUseHttpFallback = true;
  screencastActive = true;
  streamSessionTabId = tab.id;
  await ensureStreamDebugger(tab);
  clearHttpFallbackInterval();
  httpFallbackIntervalId = setInterval(() => {
    void runHttpFallbackTick(tab.id);
  }, SCREENCAST_MIN_INTERVAL_MS);
  void runHttpFallbackTick(tab.id);
}

async function startStreamSession(options = {}) {
  ensureScreencastDebuggerListener();
  if (!cachedProjectRoot) {
    return;
  }
  let tabId = options.tabId;
  if (tabId == null) {
    const active = await getActiveTab();
    tabId = active?.id;
  }
  if (tabId == null) {
    return;
  }
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab || !isTabDebuggable(tab.url)) {
    await stopStreamSession();
    return;
  }
  await refreshExtensionFrameWsUrl();
  if (!extensionFrameWsUrl) {
    await postFrameErrorDebounced(
      "bridge missing extension_frame_ws_url — reconnect teshi Chrome mode",
    );
    return;
  }
  if (
    screencastActive &&
    streamSessionTabId === tab.id &&
    !options.force &&
    !streamUseHttpFallback
  ) {
    await connectStreamWebSocket();
    return;
  }
  await stopStreamSession();
  try {
    await ensureStreamDebugger(tab);
    await chrome.debugger.sendCommand({ tabId: tab.id }, "Page.startScreencast", {
      format: "jpeg",
      quality: STREAM_JPEG_QUALITY,
      maxWidth: STREAM_BOUNDS_W,
      maxHeight: STREAM_BOUNDS_H,
      everyNthFrame: 1,
    });
    screencastActive = true;
    streamSessionTabId = tab.id;
    streamUseHttpFallback = false;
    await connectStreamWebSocket();
  } catch (err) {
    const msg = captureErrorMessage(err);
    try {
      await startHttpFallbackSession(tab);
      await postFrameErrorDebounced(
        `Screencast unavailable (${msg}); using HTTP frame fallback.`,
      );
    } catch (fallbackErr) {
      await postFrameErrorDebounced(
        `Preview failed: ${msg}. ${captureErrorMessage(fallbackErr)}`,
      );
    }
  }
}

async function pauseScreencast() {
  if (streamSessionTabId === null) {
    return;
  }
  try {
    await chrome.debugger.sendCommand(
      { tabId: streamSessionTabId },
      "Page.stopScreencast",
      {},
    );
  } catch {
    // ignore
  }
  clearHttpFallbackInterval();
  closeStreamWebSocket();
}

async function resumeScreencast() {
  if (!cachedProjectRoot || streamPaused) {
    return;
  }
  const tabId = streamSessionTabId ?? (await getActiveTab())?.id;
  if (tabId == null) {
    return;
  }
  await startStreamSession({ tabId, force: true });
}

async function attachActiveTab() {
  const tab = await getActiveTab();
  if (!tab?.id) {
    throw new Error("no active tab in the current window");
  }
  if (!isTabDebuggable(tab.url)) {
    throw new Error(
      "active tab cannot be debugged (use an http(s) page, not chrome://)",
    );
  }
  if (attachedTabId === tab.id) {
    for (const domain of ["Accessibility", "DOM", "Runtime", "Overlay", "Page"]) {
      try {
        await chrome.debugger.sendCommand({ tabId: tab.id }, `${domain}.enable`, {});
      } catch {
        // Domain may already be enabled.
      }
    }
    streamPageEnabled = true;
    return tab;
  }
  await detachIfNeeded();
  await chrome.debugger.attach({ tabId: tab.id }, "1.3");
  attachedTabId = tab.id;
  for (const domain of ["Accessibility", "DOM", "Runtime", "Overlay", "Page"]) {
    await chrome.debugger.sendCommand({ tabId: tab.id }, `${domain}.enable`, {});
  }
  streamPageEnabled = true;
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
    tab_id: tab.id,
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

async function waitForTabReady(tabId, timeoutMs = 3500) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const tab = await chrome.tabs.get(tabId);
    if (tab.active && tab.status === "complete") {
      return tab;
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  return chrome.tabs.get(tabId);
}

async function activateTab(tabId) {
  const id = Number(tabId);
  if (!Number.isFinite(id) || id <= 0) {
    return { ok: false, error: "tab_id is required" };
  }
  const existing = await chrome.tabs.get(id).catch(() => null);
  if (!existing) {
    return { ok: false, error: `tab not found: ${id}` };
  }
  if (!isTabDebuggable(existing.url)) {
    return {
      ok: false,
      error: "tab cannot be captured (chrome:// and extension pages are not supported)",
    };
  }
  await chrome.tabs.update(id, { active: true });
  const active = await waitForTabReady(id);
  return {
    ok: true,
    tab_id: id,
    url: active.url ?? "",
    title: active.title ?? "",
  };
}

function formatError(err) {
  if (err instanceof Error) {
    return err.message ? `${err.name}: ${err.message}` : err.name;
  }
  return String(err);
}

function captureErrorMessage(err) {
  const raw = chrome.runtime.lastError?.message ?? formatError(err);
  return raw.replace(/^(Error:\s*)+/i, "").trim();
}

async function postFrameErrorDebounced(error) {
  const now = Date.now();
  if (now - lastStreamErrorPostedAt < STREAM_ERROR_DEBOUNCE_MS) {
    return;
  }
  lastStreamErrorPostedAt = now;
  await postFrameError(error);
}

async function bridgePost(url, payload) {
  const body = JSON.stringify(payload);
  let lastErr = null;
  for (let attempt = 0; attempt <= BRIDGE_POST_RETRIES; attempt += 1) {
    try {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), BRIDGE_FETCH_TIMEOUT_MS);
      const res = await fetch(url, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body,
        signal: controller.signal,
      });
      clearTimeout(timer);
      if (!res.ok) {
        throw new Error(`bridge HTTP ${res.status} for ${url}`);
      }
      return res;
    } catch (err) {
      lastErr = err;
      if (attempt < BRIDGE_POST_RETRIES) {
        await new Promise((resolve) => setTimeout(resolve, 120 * (attempt + 1)));
      }
    }
  }
  throw lastErr instanceof Error ? lastErr : new Error(String(lastErr));
}

async function postFrameError(error) {
  const message =
    typeof error === "string" ? error : captureErrorMessage(error);
  try {
    await bridgePost(RESPONSE_URL, {
      type: "frame_error",
      error: message,
    });
  } catch {
    // Best-effort.
  }
  if (streamWs?.readyState === WebSocket.OPEN) {
    try {
      streamWs.send(
        JSON.stringify({ type: "frame_error", error: message }),
      );
    } catch {
      // ignore
    }
  }
}

const COMMANDS_PAUSING_STREAM = new Set([
  "get_page_snapshot",
  "highlight_selector",
  "clear_highlight",
]);

async function handleCmd(msg) {
  const { cmd, request_id: requestId, selector, tab_id: tabId } = msg;
  const pausesStream = COMMANDS_PAUSING_STREAM.has(cmd);
  if (pausesStream) {
    streamPaused = true;
    await pauseScreencast();
  }
  let body;
  try {
    if (cmd === "get_page_snapshot") {
      body = await getPageSnapshot();
    } else if (cmd === "highlight_selector") {
      body = await highlightSelector(selector);
    } else if (cmd === "clear_highlight") {
      body = await clearHighlight();
    } else if (cmd === "activate_tab") {
      body = await activateTab(tabId);
      if (body.ok) {
        streamSessionTabId = body.tab_id;
      }
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
  } finally {
    if (pausesStream) {
      streamPaused = false;
      await resumeScreencast();
    } else if (cmd === "activate_tab" && body?.ok) {
      await startStreamSession({ tabId: body.tab_id, force: true });
    }
  }
  return { type: "response", request_id: requestId, cmd, ...body };
}

function setBadge(connected) {
  chrome.action.setBadgeText({ text: connected ? "OK" : "" });
  chrome.action.setBadgeBackgroundColor({ color: "#16a34a" });
}

async function refreshBridgeCache() {
  try {
    const res = await fetch(DISCOVERY_URL);
    if (!res.ok) {
      return false;
    }
    const info = await res.json();
    if (info.mode === "chrome" && info.project_root) {
      cachedProjectRoot = info.project_root;
      if (info.extension_frame_ws_url) {
        extensionFrameWsUrl = String(info.extension_frame_ws_url);
      }
      return true;
    }
  } catch {
    // Bridge offline.
  }
  return false;
}

async function ensureProjectRoot() {
  if (cachedProjectRoot) {
    return cachedProjectRoot;
  }
  await refreshBridgeCache();
  return cachedProjectRoot;
}

async function ensureStreamForActiveTab() {
  if (!cachedProjectRoot || streamPaused) {
    return;
  }
  const tab = await getActiveTab();
  if (!tab?.id || !isTabDebuggable(tab.url)) {
    return;
  }
  if (screencastActive && streamSessionTabId === tab.id) {
    if (!streamUseHttpFallback) {
      await connectStreamWebSocket();
    }
    return;
  }
  await startStreamSession({ tabId: tab.id });
}

async function heartbeatOnce(options = {}) {
  const projectRoot = await ensureProjectRoot();
  if (!projectRoot) {
    setBadge(false);
    return;
  }
  const tab = await getActiveTab();
  const windowTabs = await listWindowTabs();

  let res;
  try {
    res = await fetch(HEARTBEAT_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        project_root: projectRoot,
        url: tab?.url ?? "",
        title: tab?.title ?? "",
        active_tab_id: windowTabs.active_tab_id,
        tabs: windowTabs.tabs,
        frame_error: "",
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
    cachedProjectRoot = "";
    return;
  }
  setBadge(true);

  if (data.cmd) {
    const reply = await handleCmd(data.cmd);
    try {
      await bridgePost(RESPONSE_URL, reply);
    } catch {
      // Best-effort.
    }
  }
  if (data.stream_restart || data.force_capture || options.forceStream) {
    await startStreamSession({ force: true });
  } else {
    await ensureStreamForActiveTab();
  }
}

function scheduleTabSwitchCapture() {
  if (tabEventTimer !== null) {
    clearTimeout(tabEventTimer);
  }
  tabEventTimer = setTimeout(() => {
    tabEventTimer = null;
    void (async () => {
      const tab = await getActiveTab();
      if (tab?.id) {
        await stopStreamSession();
        await startStreamSession({ tabId: tab.id, force: true });
      }
      void heartbeatLoop();
    })();
  }, TAB_EVENT_DEBOUNCE_MS);
}

async function heartbeatLoop(options = {}) {
  if (options.forceStream) {
    pendingStreamRestart = true;
  }
  if (heartbeatRunning) {
    return;
  }
  heartbeatRunning = true;
  try {
    do {
      const force = pendingStreamRestart || options.forceStream;
      pendingStreamRestart = false;
      await heartbeatOnce({ forceStream: force });
      options = {};
    } while (pendingStreamRestart);
  } finally {
    heartbeatRunning = false;
  }
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type === "connect_now") {
    cachedProjectRoot = "";
    extensionFrameWsUrl = "";
    void (async () => {
      heartbeatRunning = false;
      await refreshBridgeCache();
      await stopStreamSession();
      await heartbeatOnce({ forceStream: true });
      sendResponse({ ok: true });
    })();
    return true;
  }
  return false;
});

chrome.runtime.onInstalled.addListener(() => {
  chrome.alarms.create(ALARM_NAME, { periodInMinutes: 1 });
  void heartbeatLoop({ forceStream: true });
});

chrome.runtime.onStartup.addListener(() => {
  void heartbeatLoop({ forceStream: true });
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === ALARM_NAME) {
    void heartbeatLoop();
  }
});

chrome.tabs.onActivated.addListener((activeInfo) => {
  if (attachedTabId !== null && attachedTabId !== activeInfo.tabId) {
    void detachIfNeeded();
  }
  scheduleTabSwitchCapture();
});

chrome.tabs.onRemoved.addListener((tabId) => {
  if (tabId === attachedTabId) {
    attachedTabId = null;
  }
  if (tabId === streamSessionTabId) {
    void stopStreamSession();
  }
});

chrome.debugger.onDetach.addListener((source) => {
  if (source.tabId === attachedTabId) {
    attachedTabId = null;
    streamPageEnabled = false;
  }
  if (source.tabId === streamSessionTabId) {
    screencastActive = false;
    streamSessionTabId = null;
    closeStreamWebSocket();
    clearHttpFallbackInterval();
  }
});

setInterval(() => {
  void heartbeatLoop();
}, HEARTBEAT_MS);

ensureScreencastDebuggerListener();
void heartbeatLoop({ forceStream: true });
