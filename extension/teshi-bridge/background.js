/**
 * teshi-bridge: HTTP heartbeat (metadata + commands) and CDP screencast preview.
 */

const DISCOVERY_URL = "http://127.0.0.1:17373/v1/bridge";
const HEARTBEAT_URL = "http://127.0.0.1:17373/v1/bridge/heartbeat";
const RESPONSE_URL = "http://127.0.0.1:17373/v1/bridge/response";
const PROTOCOL_VERSION = 1;
const IDENTITY_STORAGE_KEY = "teshiBridgeIdentity";
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
const MAX_SCREENSHOT_DIMENSION = 16384;
const MAX_SCREENSHOT_PIXELS = 100000000;
const MAX_ARTIFACT_BYTES = 50 * 1024 * 1024;
const MAX_PRIVILEGED_RESULT_BYTES = 1048576;
function phasedFeatures(optionalPermissions) {
  const permissionFeature = (feature, permission) => optionalPermissions[permission]
    ? { feature, available: true, reason: "grant_required" }
    : { feature, available: false, reason: "permission_not_granted" };
  return [
    { feature: "p0.control", available: true },
    { feature: "p1.observability_artifacts", available: true },
    { feature: "p2.javascript", available: true, reason: "grant_required" },
    { feature: "p2.raw_cdp", available: true, reason: "grant_and_policy_required" },
    permissionFeature("p2.cookies", "cookies"),
    permissionFeature("p2.content_settings", "content_settings"),
    permissionFeature("p2.extension_management", "extension_management"),
  ];
}
const SUPPORTED_ACTIONS = Object.freeze([
  "click",
  "pointer_click",
  "fill",
  "type",
  "assert_visible",
  "assert_text",
  "select",
  "press_key",
  "navigate",
  "upload",
]);
const SUPPORTED_OPERATIONS = Object.freeze([
  "capture_browser_screenshot",
  "generate_browser_pdf",
  "start_console_capture",
  "list_console_events",
  "clear_console_capture",
  "stop_console_capture",
  "start_network_capture",
  "list_network_requests",
  "get_network_request_detail",
  "clear_network_capture",
  "stop_network_capture",
  "execute_privileged_javascript",
  "execute_privileged_cdp",
  "list_browser_cookies",
  "access_browser_content_setting",
  "list_browser_extensions",
]);

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
let cachedBrokerToken = "";
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
/** Tab IDs with broker-owned bounded console capture enabled. */
const consoleCaptureTabIds = new Set();
/** Tab IDs with broker-owned bounded network metadata capture enabled. */
const networkCaptureTabIds = new Set();
let lastBridgeStatus = {
  connected: false,
  error: "Bridge has not been contacted yet.",
};
/** @type {Promise<{extension_instance_id: string, profile_label: string}> | null} */
let identityPromise = null;

function randomInstanceId() {
  if (typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("");
}

async function getExtensionIdentity() {
  if (identityPromise) {
    return identityPromise;
  }
  identityPromise = (async () => {
    const stored = await chrome.storage.local.get(IDENTITY_STORAGE_KEY);
    const value = stored?.[IDENTITY_STORAGE_KEY];
    const extension_instance_id =
      typeof value?.extension_instance_id === "string" && value.extension_instance_id
        ? value.extension_instance_id
        : randomInstanceId();
    const profile_label =
      typeof value?.profile_label === "string" ? value.profile_label.slice(0, 120) : "";
    const identity = { extension_instance_id, profile_label };
    await chrome.storage.local.set({ [IDENTITY_STORAGE_KEY]: identity });
    return identity;
  })();
  return identityPromise;
}

async function setProfileLabel(rawLabel) {
  const identity = await getExtensionIdentity();
  const next = {
    ...identity,
    profile_label: String(rawLabel ?? "").trim().slice(0, 120),
  };
  await chrome.storage.local.set({ [IDENTITY_STORAGE_KEY]: next });
  identityPromise = Promise.resolve(next);
  return next;
}

function browserMetadata() {
  const ua = navigator.userAgent || "";
  const match = ua.match(/(Edg|Chrome|Chromium)\/([^\s]+)/);
  const product = match?.[1] === "Edg" ? "Microsoft Edge" : match?.[1] || "Chromium";
  return {
    name: product,
    version: match?.[2] || "",
    platform: navigator.platform || "",
  };
}

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
      window_id: t.windowId,
      title: (t.title || t.url || "Untitled").slice(0, 200),
      url: t.url ?? "",
      active: Boolean(t.active),
      favIconUrl: t.favIconUrl ?? "",
      debuggable: isTabDebuggable(t.url),
    })),
  };
}

async function listBrowserWindows() {
  const windows = await chrome.windows.getAll({
    populate: true,
    windowTypes: ["normal"],
  });
  return windows.map((windowInfo) => ({
    id: windowInfo.id,
    focused: Boolean(windowInfo.focused),
    tabs: (windowInfo.tabs ?? []).map((tab) => ({
      id: tab.id,
      window_id: tab.windowId,
      title: (tab.title || tab.url || "Untitled").slice(0, 200),
      url: tab.url ?? "",
      active: Boolean(tab.active),
      favIconUrl: tab.favIconUrl ?? "",
      debuggable: isTabDebuggable(tab.url),
    })),
  }));
}

async function resolveCommandTab(target) {
  if (!target || target.tab_id == null) {
    return getActiveTab();
  }
  const tabId = Number(target.tab_id);
  const windowId = Number(target.window_id);
  if (!Number.isInteger(tabId) || !Number.isInteger(windowId)) {
    throw new Error("target requires numeric window_id and tab_id");
  }
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab || tab.windowId !== windowId) {
    throw new Error("target tab is stale or belongs to another browser window");
  }
  if (!isTabDebuggable(tab.url)) {
    throw new Error("target tab cannot be debugged (use an http(s) or file page)");
  }
  return tab;
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

function cacheBrokerToken(info) {
  const rawUrl = String(info?.ws_url || info?.extension_frame_ws_url || "");
  try {
    cachedBrokerToken = new URL(rawUrl).searchParams.get("token") || "";
  } catch {
    cachedBrokerToken = "";
  }
  return Boolean(cachedBrokerToken);
}

async function refreshExtensionFrameWsUrl() {
  try {
    const res = await fetch(DISCOVERY_URL);
    if (!res.ok) {
      return false;
    }
    const info = await res.json();
    if (info.extension_frame_ws_url) {
      cacheBrokerToken(info);
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
    ws.onopen = async () => {
      streamWs = ws;
      streamWsReconnectDelay = STREAM_WS_RECONNECT_BASE_MS;
      const identity = await getExtensionIdentity();
      ws.send(
        JSON.stringify({
          type: "stream_hello",
          project_root: cachedProjectRoot,
          extension_instance_id: identity.extension_instance_id,
          protocol_version: PROTOCOL_VERSION,
          extension_version: chrome.runtime.getManifest().version,
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
        } else if (ack.type === "direct_command" && ack.command) {
          void (async () => {
            const reply = await handleCmd(ack.command);
            if (ws.readyState === WebSocket.OPEN) {
              ws.send(JSON.stringify(reply));
            }
          })();
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

async function publishScreencastFrame(jpegBytes, tabId, url) {
  const identity = await getExtensionIdentity();
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  const meta = {
    extension_instance_id: identity.extension_instance_id,
    window_id: tab?.windowId ?? 0,
    tab_id: tabId,
    url,
    seq: (streamSeq += 1),
  };
  if (streamWs?.readyState === WebSocket.OPEN) {
    streamWs.send(encodeTsh1Frame(meta, jpegBytes));
    return;
  }
  void connectStreamWebSocket();
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
    if (method === "Runtime.consoleAPICalled" || method === "Log.entryAdded") {
      if (consoleCaptureTabIds.has(source.tabId)) {
        void publishConsoleEvent(source.tabId, method, params);
      }
      return;
    }
    if (method.startsWith("Network.") && networkCaptureTabIds.has(source.tabId)) {
      void publishNetworkEvent(source.tabId, method, params);
      return;
    }
    if (method !== "Page.screencastFrame") {
      return;
    }
    if (!screencastActive || source.tabId !== streamSessionTabId) {
      return;
    }
    void handleScreencastFrame(params, source.tabId);
  });
}

function remoteObjectText(value) {
  if (Object.prototype.hasOwnProperty.call(value || {}, "value")) {
    const raw = value.value;
    if (typeof raw === "string") return raw;
    try { return JSON.stringify(raw); } catch { return String(raw); }
  }
  return String(value?.description ?? value?.unserializableValue ?? value?.type ?? "");
}

async function publishConsoleEvent(tabId, method, params) {
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab || !consoleCaptureTabIds.has(tabId)) return;
  const identity = await getExtensionIdentity();
  const entry = method === "Log.entryAdded" ? (params?.entry || {}) : null;
  const stackFrame = params?.stackTrace?.callFrames?.[0];
  const event = entry
    ? {
        timestamp_ms: Number(entry.timestamp || Date.now()),
        level: entry.level || "log",
        text: String(entry.text || ""),
        source: entry.source || "log",
        url: entry.url || "",
        line_number: entry.lineNumber,
      }
    : {
        timestamp_ms: Number(params?.timestamp || Date.now()),
        level: params?.type || "log",
        text: (params?.args || []).map(remoteObjectText).join(" "),
        source: "console-api",
        url: stackFrame?.url || "",
        line_number: stackFrame?.lineNumber,
      };
  try {
    await bridgePost(RESPONSE_URL, {
      type: "console_event",
      extension_instance_id: identity.extension_instance_id,
      target: {
        extension_instance_id: identity.extension_instance_id,
        window_id: tab.windowId,
        tab_id: tabId,
      },
      event,
    });
  } catch {
    // Diagnostic capture is best-effort and must not disrupt page control.
  }
}

async function startConsoleCapture(target = null) {
  ensureScreencastDebuggerListener();
  const tab = await attachActiveTab(target);
  await cdp(tab.id, "Runtime.enable", {});
  await cdp(tab.id, "Log.enable", {});
  consoleCaptureTabIds.add(tab.id);
  return { ok: true, active: true, tab_id: tab.id };
}

async function stopConsoleCapture(target = null) {
  const tab = await resolveCommandTab(target);
  consoleCaptureTabIds.delete(tab.id);
  return { ok: true, active: false, tab_id: tab.id };
}

async function publishNetworkEvent(tabId, method, params) {
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab || !networkCaptureTabIds.has(tabId)) return;
  const identity = await getExtensionIdentity();
  let event = null;
  if (method === "Network.requestWillBeSent") {
    event = {
      event_type: "request",
      request_id: params?.requestId,
      timestamp_ms: Number(params?.wallTime ? params.wallTime * 1000 : Date.now()),
      url: params?.request?.url || "",
      method: params?.request?.method || "",
      resource_type: params?.type || "",
      headers: params?.request?.headers || {},
    };
  } else if (method === "Network.responseReceived") {
    event = {
      event_type: "response",
      request_id: params?.requestId,
      status: params?.response?.status,
      status_text: params?.response?.statusText || "",
      mime_type: params?.response?.mimeType || "",
      protocol: params?.response?.protocol || "",
      from_cache: Boolean(params?.response?.fromDiskCache || params?.response?.fromPrefetchCache),
      headers: params?.response?.headers || {},
    };
  } else if (method === "Network.loadingFinished") {
    event = {
      event_type: "finished",
      request_id: params?.requestId,
      encoded_data_length: params?.encodedDataLength,
    };
  } else if (method === "Network.loadingFailed") {
    event = {
      event_type: "failed",
      request_id: params?.requestId,
      error_text: params?.errorText || "",
      canceled: Boolean(params?.canceled),
    };
  }
  if (!event?.request_id) return;
  try {
    await bridgePost(RESPONSE_URL, {
      type: "network_event",
      extension_instance_id: identity.extension_instance_id,
      target: {
        extension_instance_id: identity.extension_instance_id,
        window_id: tab.windowId,
        tab_id: tabId,
      },
      event,
    });
  } catch {
    // Diagnostic capture is best-effort and must not disrupt page control.
  }
}

async function startNetworkCapture(target = null) {
  ensureScreencastDebuggerListener();
  const tab = await attachActiveTab(target);
  await cdp(tab.id, "Network.enable", {
    maxTotalBufferSize: 10_000_000,
    maxResourceBufferSize: 2_000_000,
    maxPostDataSize: 0,
  });
  networkCaptureTabIds.add(tab.id);
  return { ok: true, active: true, tab_id: tab.id, metadata_only: true };
}

async function getNetworkResponseBody(networkRequestId, target = null) {
  const tab = await attachActiveTab(target);
  if (!networkCaptureTabIds.has(tab.id)) {
    return { ok: false, code: "invalid_browser_operation", error: "network capture is not active" };
  }
  const result = await cdp(tab.id, "Network.getResponseBody", {
    requestId: String(networkRequestId || ""),
  });
  return {
    ok: true,
    body: result?.body || "",
    base64_encoded: Boolean(result?.base64Encoded),
  };
}

async function stopNetworkCapture(target = null) {
  const tab = await resolveCommandTab(target);
  networkCaptureTabIds.delete(tab.id);
  return { ok: true, active: false, tab_id: tab.id };
}

async function stopStreamSession() {
  screencastActive = false;
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
  if (screencastActive && streamSessionTabId === tab.id && !options.force) {
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
    await connectStreamWebSocket();
  } catch (err) {
    await postFrameErrorDebounced(captureErrorMessage(err));
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
  // The same negotiated socket carries correlated direct commands. Keep it
  // open while frames are paused so the in-flight command can return its
  // response without falling back or timing out.
}

async function resumeScreencast() {
  if (!cachedProjectRoot || streamPaused) {
    return;
  }
  const tabId = streamSessionTabId ?? (await getActiveTab())?.id;
  if (tabId == null) {
    return;
  }
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab || !isTabDebuggable(tab.url)) {
    return;
  }
  try {
    await ensureStreamDebugger(tab);
    await chrome.debugger.sendCommand({ tabId }, "Page.startScreencast", {
      format: "jpeg",
      quality: STREAM_JPEG_QUALITY,
      maxWidth: STREAM_BOUNDS_W,
      maxHeight: STREAM_BOUNDS_H,
      everyNthFrame: 1,
    });
    screencastActive = true;
    streamSessionTabId = tabId;
    await connectStreamWebSocket();
  } catch (err) {
    await postFrameErrorDebounced(captureErrorMessage(err));
  }
}

async function attachActiveTab(target = null) {
  const tab = await resolveCommandTab(target);
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
      // makeShortSelector — compact CSS selector generator (ported from teshi-engine)
      function makeShortSelector(el) {
        if (!el || el === document.body || el === document.documentElement) return el ? el.tagName.toLowerCase() : '';
        if (el.id) return '#' + el.id;
        var tid = el.getAttribute('data-testid');
        if (tid) return '[data-testid="' + tid.replace(/"/g,'\\\\"') + '"]';
        var na = el.getAttribute('name');
        if (na) return '[name="' + na.replace(/"/g,'\\\\"') + '"]';
        var aa = el.getAttribute('aria-label');
        if (aa) return '[aria-label="' + aa.replace(/"/g,'\\\\"') + '"]';
        var pa = el.getAttribute('placeholder');
        if (pa) return '[placeholder="' + pa.replace(/"/g,'\\\\"') + '"]';
        var ta = el.getAttribute('title');
        if (ta) return '[title="' + ta.replace(/"/g,'\\\\"') + '"]';
        var href = el.getAttribute('href');
        if (href && el.tagName.toLowerCase() === 'a') return 'a[href*="' + href.replace(/"/g,'\\\\"') + '"]';
        var path = [], cur = el;
        while (cur && cur !== document.body && cur !== document.documentElement) {
          var tag = cur.tagName.toLowerCase(), seg = tag;
          var cls = cur.className, parts = [];
          if (cls && typeof cls === 'string') {
            parts = cls.trim().split(/\\s+/).filter(function(c){
              return c && !/^[a-z]+-[a-z]+-\\d+$/.test(c) && c.indexOf('__')===-1
                && !/^sc-[A-Z]/.test(c) && !/^_[a-z]+_/.test(c)
                && !/^(flex|items-center|justify-|text-|w-full|h-full|relative|absolute|fixed|sticky|z-\\d+|gap-\\d+|p-\\d+|m-\\d+)/.test(c);
            }).slice(0, 2);
          }
          if (parts.length) { seg += '.' + parts.join('.'); }
          else {
            var p = cur.parentElement;
            if (p) {
              var ch = Array.from(p.children);
              var same = ch.filter(function(s){return s.tagName === cur.tagName;});
              if (same.length > 1) seg += ':nth-of-type('+(same.indexOf(cur)+1)+')';
            }
          }
          path.unshift(seg);
          cur = cur.parentElement;
        }
        var vt = (el.innerText || el.textContent || '').trim().substring(0, 60);
        if (vt.length >= 3) return path.join(' > ') + ':has-text("'+vt.replace(/"/g,'\\\\"')+'")';
        return path.join(' > ');
      }

      const sel = "button, [role='button'], input, input[type='submit'], select, a[href], [role='link'], textarea";
      function collect(root, framePath = null, shadowPath = null, output = []) {
        let elements = [];
        try { elements = Array.from(root.querySelectorAll(sel)); } catch { elements = []; }
        for (const el of elements) {
          if (output.length >= 200) break;
          const attrs = {};
          for (const name of (el.getAttributeNames?.() ?? [])) {
            const value = el.getAttribute(name);
            if (value != null && value.length <= 500) attrs[name] = value;
          }
          output.push({
            element_ref: 'e' + (output.length + 1),
            tag: el.tagName.toLowerCase(),
            text: (el.innerText || el.value || el.getAttribute('aria-label') || '').trim().slice(0, 120),
            id: el.id || null,
            testId: el.getAttribute('data-testid'),
            role: el.getAttribute('role'),
            classes: el.className || null,
            shortSelector: makeShortSelector(el),
            ariaLabel: el.getAttribute('aria-label'),
            accessible_name: el.getAttribute('aria-label') || (el.labels?.[0]?.textContent || '').trim() || (el.innerText || el.value || '').trim().slice(0, 120) || null,
            label: (el.labels?.[0]?.textContent || '').trim() || null,
            placeholder: el.getAttribute('placeholder'),
            name: el.getAttribute('name'),
            attributes: attrs,
            visible: Boolean(el.getClientRects().length),
            context: { frame: framePath, shadow_root: shadowPath },
          });
          if (el.shadowRoot) collect(el.shadowRoot, framePath, makeShortSelector(el), output);
        }
        let frames = [];
        try { frames = Array.from(root.querySelectorAll('iframe,frame')); } catch { frames = []; }
        for (const frame of frames) {
          try {
            if (frame.contentDocument) collect(frame.contentDocument, frame.src || frame.name || 'iframe', shadowPath, output);
          } catch { /* Cross-origin frames are represented by AX data only. */ }
        }
        return output;
      }
      return collect(document);
    })()`,
    returnByValue: true,
  });
  return result?.value ?? [];
}

/** Cap AX payload size for heartbeat/CLI responses on heavy SPAs. */
const MAX_AX_NODES = 800;
const MAX_AX_JSON_BYTES = 512 * 1024;

/**
 * Truncate Accessibility.getFullAXTree nodes while preserving agent-usable structure.
 *
 * @param {unknown} tree
 * @returns {unknown}
 */
function truncateAccessibilityTree(tree) {
  if (!tree || typeof tree !== "object") {
    return tree;
  }
  const nodes = /** @type {Record<string, unknown>} */ (tree).nodes;
  if (!Array.isArray(nodes)) {
    return tree;
  }
  if (nodes.length <= MAX_AX_NODES) {
    let encoded = JSON.stringify(tree);
    if (encoded.length <= MAX_AX_JSON_BYTES) {
      return tree;
    }
  }
  const truncatedNodes = nodes.slice(0, MAX_AX_NODES);
  const out = {
    .../** @type {Record<string, unknown>} */ (tree),
    nodes: truncatedNodes,
    truncated: true,
    node_count: nodes.length,
    node_limit: MAX_AX_NODES,
  };
  let encoded = JSON.stringify(out);
  if (encoded.length > MAX_AX_JSON_BYTES) {
    out.nodes = truncatedNodes.slice(0, Math.floor(MAX_AX_NODES / 2));
    encoded = JSON.stringify(out);
    out.truncated = true;
    out.json_bytes = encoded.length;
    out.json_byte_limit = MAX_AX_JSON_BYTES;
  }
  return out;
}

async function pageContextRevision(tabId) {
  const { result } = await cdp(tabId, "Runtime.evaluate", {
    expression: `(() => {
      if (!globalThis.__teshiPageContextRevision) {
        globalThis.__teshiPageContextRevision =
          (globalThis.crypto?.randomUUID?.() || String(Date.now()) + '-' + Math.random());
      }
      return globalThis.__teshiPageContextRevision;
    })()`,
    returnByValue: true,
  });
  return String(result?.value || "");
}

async function getPageSnapshot(target = null) {
  const tab = await attachActiveTab(target);
  let accessibility_tree;
  try {
    const raw = await cdp(tab.id, "Accessibility.getFullAXTree", {});
    accessibility_tree = truncateAccessibilityTree(raw);
  } catch (err) {
    accessibility_tree = { error: String(err) };
  }
  const interactive_elements = await collectInteractiveElements(tab.id);
  const revision = await pageContextRevision(tab.id);
  return {
    ok: true,
    url: tab.url ?? "",
    title: tab.title ?? "",
    tab_id: tab.id,
    window_id: tab.windowId,
    page_context_revision: revision,
    accessibility_tree,
    interactive_elements,
  };
}

async function highlightSelector(selector, target = null) {
  if (!selector) {
    return { ok: false, error: "selector is required" };
  }
  const tab = await attachActiveTab(target);
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

async function clearHighlight(target = null) {
  const targetTab = target ? await attachActiveTab(target) : null;
  const tabId = targetTab?.id ?? attachedTabId;
  if (tabId === null) return { ok: true };
  try {
    await cdp(tabId, "Overlay.hideHighlight", {});
  } catch {
    // ignore
  }
  return { ok: true };
}

function isExplicitNavigableUrl(url) {
  try {
    const parsed = new URL(url);
    return ["http:", "https:", "file:"].includes(parsed.protocol);
  } catch {
    return false;
  }
}

async function navigateToUrl(url, timeoutMs = 15000, target = null) {
  if (!url || !isExplicitNavigableUrl(url)) {
    return {
      ok: false,
      error: "navigate requires an explicit http(s) or file URL",
      code: "invalid_url",
    };
  }
  const tab = await resolveCommandTab(target);
  if (!tab?.id) {
    return { ok: false, error: "no active tab in the current window", code: "no_active_tab" };
  }
  const startedAt = Date.now();
  await chrome.tabs.update(tab.id, { url, active: true });
  const active = await waitForTabReady(tab.id, timeoutMs);
  return {
    ok: true,
    tab_id: tab.id,
    url: active.url ?? url,
    title: active.title ?? "",
    elapsed_ms: Date.now() - startedAt,
    debug_log: {
      event: "extension_navigate",
      tab_id: tab.id,
      requested_url: url,
      final_url: active.url ?? url,
    },
  };
}

async function goBack(timeoutMs = 15000, target = null) {
  const tab = await resolveCommandTab(target);
  await attachActiveTab(target);
  const { result } = await cdp(tab.id, "Runtime.evaluate", {
    expression: "history.length > 1",
    returnByValue: true,
  });
  if (!result?.value) return { ok: false, code: "no_history", error: "tab has no previous history entry" };
  await cdp(tab.id, "Runtime.evaluate", { expression: "history.back()" });
  const active = await waitForTabReady(tab.id, timeoutMs);
  return { ok: true, tab_id: tab.id, window_id: tab.windowId, url: active.url || "", title: active.title || "" };
}

function boundedPrivilegedResult(value, maxResultBytes) {
  const limit = Math.max(1, Math.min(Number(maxResultBytes) || 65536, MAX_PRIVILEGED_RESULT_BYTES));
  const serialized = JSON.stringify(value ?? null);
  const size = new TextEncoder().encode(serialized).length;
  if (size > limit) {
    return {
      ok: false,
      code: "browser_result_too_large",
      error: "privileged browser result exceeds the configured byte limit",
      result_bytes: size,
      max_result_bytes: limit,
    };
  }
  return { ok: true, result: value ?? null, result_bytes: size };
}

async function executePrivilegedJavascript(expression, expectedRevision, timeoutMs, maxResultBytes, target = null) {
  const tab = await attachActiveTab(target);
  const revision = await pageContextRevision(tab.id);
  if (expectedRevision && revision !== String(expectedRevision)) {
    return { ok: false, code: "stale_page_context", error: "page changed before JavaScript execution", page_context_revision: revision };
  }
  const evaluated = await cdp(tab.id, "Runtime.evaluate", {
    expression: String(expression || ""),
    awaitPromise: true,
    returnByValue: true,
    timeout: Math.max(1, Number(timeoutMs) || 5000),
  });
  if (evaluated.exceptionDetails) {
    return { ok: false, code: "browser_javascript_exception", error: "JavaScript execution raised an exception", page_context_revision: revision };
  }
  return { ...boundedPrivilegedResult(evaluated.result?.value, maxResultBytes), page_context_revision: revision };
}

async function executePrivilegedCdp(method, params, expectedRevision, maxResultBytes, target = null) {
  const tab = await attachActiveTab(target);
  const revision = await pageContextRevision(tab.id);
  if (expectedRevision && revision !== String(expectedRevision)) {
    return { ok: false, code: "stale_page_context", error: "page changed before CDP execution", page_context_revision: revision };
  }
  const result = await cdp(tab.id, String(method), params && typeof params === "object" ? params : {});
  return { ...boundedPrivilegedResult(result, maxResultBytes), page_context_revision: revision };
}

async function requireOptionalPermission(permission) {
  if (!await chrome.permissions.contains({ permissions: [permission] })) {
    return {
      ok: false,
      code: "browser_capability_unavailable",
      error: `optional Chromium permission ${permission} is not approved`,
      approval: "extension-popup",
    };
  }
  return null;
}

async function listBrowserCookies(includeValues, maxEntries, maxResultBytes, target = null) {
  const unavailable = await requireOptionalPermission("cookies");
  if (unavailable) return unavailable;
  const tab = await resolveCommandTab(target);
  const parsed = new URL(tab.url);
  if (!['http:', 'https:'].includes(parsed.protocol)) {
    return { ok: false, code: "browser_capability_unavailable", error: "Cookie access requires an HTTP(S) selected tab" };
  }
  const entryLimit = Math.max(1, Math.min(Number(maxEntries) || 200, 500));
  const byteLimit = Math.max(1, Math.min(Number(maxResultBytes) || 262144, MAX_PRIVILEGED_RESULT_BYTES));
  const raw = await chrome.cookies.getAll({ url: tab.url });
  raw.sort((left, right) => `${left.domain}\0${left.path}\0${left.name}\0${left.storeId}`.localeCompare(`${right.domain}\0${right.path}\0${right.name}\0${right.storeId}`));
  const cookies = [];
  let truncated = raw.length > entryLimit;
  for (const item of raw.slice(0, entryLimit)) {
    const cookie = {
      name: String(item.name || "").slice(0, 1024),
      domain: String(item.domain || "").slice(0, 1024),
      path: String(item.path || "").slice(0, 2048),
      secure: Boolean(item.secure),
      http_only: Boolean(item.httpOnly),
      same_site: item.sameSite || "unspecified",
      session: Boolean(item.session),
      expiration_date: Number.isFinite(item.expirationDate) ? item.expirationDate : null,
      store_id: String(item.storeId || "").slice(0, 256),
      partition_key: item.partitionKey && typeof item.partitionKey === "object" ? {
        top_level_site: String(item.partitionKey.topLevelSite || "").slice(0, 2048),
        has_cross_site_ancestor: Boolean(item.partitionKey.hasCrossSiteAncestor),
      } : null,
      value_redacted: !includeValues,
    };
    if (includeValues) cookie.value = String(item.value || "");
    const candidate = [...cookies, cookie];
    if (new TextEncoder().encode(JSON.stringify(candidate)).length > byteLimit) {
      truncated = true;
      break;
    }
    cookies.push(cookie);
  }
  return {
    ok: true,
    origin: parsed.origin,
    cookies,
    count: cookies.length,
    truncated,
    values_included: Boolean(includeValues),
  };
}

const CONTENT_SETTING_APIS = Object.freeze({
  notifications: "notifications",
  popups: "popups",
  geolocation: "location",
  camera: "camera",
  microphone: "microphone",
  automatic_downloads: "automaticDownloads",
});

async function accessBrowserContentSetting(setting, value, target = null) {
  const unavailable = await requireOptionalPermission("contentSettings");
  if (unavailable) return unavailable;
  const apiName = CONTENT_SETTING_APIS[String(setting || "")];
  const api = apiName ? chrome.contentSettings[apiName] : null;
  if (!api) return { ok: false, code: "browser_capability_denied", error: "content setting is not allowlisted" };
  const tab = await resolveCommandTab(target);
  const parsed = new URL(tab.url);
  if (!['http:', 'https:'].includes(parsed.protocol)) {
    return { ok: false, code: "browser_capability_unavailable", error: "content settings require an HTTP(S) selected tab" };
  }
  if (value == null) {
    const current = await api.get({ primaryUrl: tab.url });
    return { ok: true, setting, value: current.setting, origin: parsed.origin, scope: "selected-origin" };
  }
  if (!['allow', 'block', 'ask'].includes(String(value))) {
    return { ok: false, code: "invalid_browser_operation", error: "content setting value must be allow, block, or ask" };
  }
  const primaryPattern = `${parsed.origin}/*`;
  await api.set({ primaryPattern, setting: String(value), scope: "regular" });
  return { ok: true, setting, value: String(value), origin: parsed.origin, scope: "selected-origin" };
}

async function listBrowserExtensions(maxEntries) {
  const unavailable = await requireOptionalPermission("management");
  if (unavailable) return unavailable;
  const limit = Math.max(1, Math.min(Number(maxEntries) || 200, 500));
  const all = await chrome.management.getAll();
  const extensions = all
    .map((item) => ({
      id: item.id,
      name: String(item.name || "").slice(0, 256),
      version: String(item.version || "").slice(0, 128),
      enabled: Boolean(item.enabled),
      type: String(item.type || "").slice(0, 64),
      install_type: String(item.installType || "").slice(0, 64),
    }))
    .sort((left, right) => left.id.localeCompare(right.id));
  return { ok: true, extensions: extensions.slice(0, limit), count: Math.min(extensions.length, limit), truncated: extensions.length > limit, mutations_enabled: false };
}

async function setFileInputFiles(tabId, selector, candidate, locatorContext, files) {
  const { result } = await cdp(tabId, "Runtime.evaluate", {
    expression: `(() => {
      const selector = ${JSON.stringify(selector)};
      const candidate = ${JSON.stringify(candidate)};
      const context = ${JSON.stringify(locatorContext)} || {};
      let root = document;
      if (context.frame) {
        const frame = Array.from(document.querySelectorAll('iframe,frame')).find((item) => item.src === context.frame || item.name === context.frame || item.src?.includes(context.frame));
        if (!frame?.contentDocument) return null;
        root = frame.contentDocument;
      }
      if (context.shadow_root) {
        const host = root.querySelector(context.shadow_root);
        if (!host?.shadowRoot) return null;
        root = host.shadowRoot;
      }
      const name = (el) => (el.getAttribute?.('aria-label') || el.labels?.[0]?.textContent || el.innerText || el.value || el.textContent || '').trim();
      const role = (el) => el.getAttribute?.('role') || ((el.tagName || '').toLowerCase() === 'button' ? 'button' : '');
      let found = [];
      if (!candidate) { try { found = Array.from(root.querySelectorAll(selector)); } catch {} }
      else {
        const args = candidate.arguments || {}, all = Array.from(root.querySelectorAll('*'));
        if (candidate.kind === 'role') found = all.filter((el) => role(el) === args.role && name(el) === String(args.name || ''));
        if (candidate.kind === 'label') found = all.filter((el) => (el.labels?.[0]?.textContent || el.getAttribute?.('aria-label') || '').trim() === String(args.text || ''));
        if (candidate.kind === 'placeholder') found = all.filter((el) => (el.getAttribute?.('placeholder') || '') === String(args.text || ''));
        if (candidate.kind === 'test_id' || candidate.kind === 'attribute') found = all.filter((el) => (el.getAttribute?.(String(args.attribute || 'data-testid')) || '') === String(args.value || ''));
        if (candidate.kind === 'text') found = all.filter((el) => (el.innerText || el.textContent || '').trim() === String(args.text || ''));
        if (candidate.kind === 'css') { try { found = Array.from(root.querySelectorAll(String(args.selector || ''))); } catch {} }
      }
      if (found.length !== 1) return null;
      const element = found[0];
      if ((element.tagName || '').toLowerCase() !== 'input' || String(element.type || '').toLowerCase() !== 'file' || element.disabled) return null;
      return element;
    })()`,
    returnByValue: false,
  });
  if (!result?.objectId) {
    return { ok: false, code: "stale_element_reference", error: "file input is unavailable, ambiguous, or not actionable" };
  }
  await cdp(tabId, "DOM.enable", {});
  const described = await cdp(tabId, "DOM.describeNode", { objectId: result.objectId });
  const backendNodeId = described?.node?.backendNodeId;
  if (!backendNodeId) {
    return { ok: false, code: "stale_element_reference", error: "file input node is unavailable" };
  }
  await cdp(tabId, "DOM.setFileInputFiles", { files, backendNodeId });
  return { ok: true, uploaded_files: files.length };
}

async function executeLocator({ selector, candidate = null, locatorContext = null, expectedRevision = null, action, value, files = [], timeoutMs = 5000, focus = false, target = null }) {
  if (!selector && !candidate) {
    return { ok: false, error: "selector or structured candidate is required", code: "invalid_selector" };
  }
  const allowed = new Set([
    "click",
    "pointer_click",
    "fill",
    "type",
    "assert_visible",
    "assert_text",
    "select",
    "press_key",
    "upload",
  ]);
  if (!allowed.has(action)) {
    return {
      ok: false,
      error: `unsupported action: ${action}`,
      code: "unsupported_action",
    };
  }
  if (["fill", "type", "assert_text", "select", "press_key"].includes(action) && value == null) {
    return {
      ok: false,
      error: `value is required for ${action}`,
      code: "missing_value",
    };
  }

  const tab = await attachActiveTab(target);
  const currentRevision = await pageContextRevision(tab.id);
  if (expectedRevision && currentRevision !== String(expectedRevision)) {
    return {
      ok: false,
      code: "stale_element_reference",
      error: "page changed before the action could be executed",
      page_context_revision: currentRevision,
    };
  }
  const expression = `(() => {
    const selector = ${JSON.stringify(selector)};
    const candidate = ${JSON.stringify(candidate)};
    const locatorContext = ${JSON.stringify(locatorContext)} || {};
    const action = ${JSON.stringify(action)};
    const value = ${JSON.stringify(value ?? "")};
    const timeoutMs = ${Number(timeoutMs) || 5000};
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const visible = (el) => {
      if (!el) return false;
      const style = window.getComputedStyle(el);
      const rect = el.getBoundingClientRect();
      return style.visibility !== "hidden" &&
        style.display !== "none" &&
        Number(style.opacity || "1") > 0 &&
        rect.width > 0 &&
        rect.height > 0;
    };
    const deadline = Date.now() + timeoutMs;
    const implicitRole = (el) => {
      const explicit = el.getAttribute?.('role');
      if (explicit) return explicit;
      const tag = (el.tagName || '').toLowerCase();
      const type = (el.getAttribute?.('type') || '').toLowerCase();
      if (tag === 'button' || (tag === 'input' && ['button','submit','reset'].includes(type))) return 'button';
      if (tag === 'a' && el.hasAttribute('href')) return 'link';
      if (tag === 'textarea') return 'textbox';
      if (tag === 'input') return type === 'checkbox' ? 'checkbox' : type === 'radio' ? 'radio' : 'textbox';
      if (tag === 'select') return 'combobox';
      return '';
    };
    const accessibleName = (el) => (el.getAttribute?.('aria-label') || el.labels?.[0]?.textContent || el.innerText || el.value || el.textContent || '').trim();
    const resolveRoot = () => {
      let root = document;
      if (locatorContext.frame) {
        const frames = Array.from(document.querySelectorAll('iframe,frame'));
        const frame = frames.find((item) => item.src === locatorContext.frame || item.name === locatorContext.frame || item.src?.includes(locatorContext.frame));
        if (!frame?.contentDocument) return null;
        root = frame.contentDocument;
      }
      if (locatorContext.shadow_root) {
        const host = root.querySelector(locatorContext.shadow_root);
        if (!host?.shadowRoot) return null;
        root = host.shadowRoot;
      }
      return root;
    };
    const findMatches = () => {
      const root = resolveRoot();
      if (!root) return [];
      if (!candidate) {
        try { return Array.from(root.querySelectorAll(selector)); } catch { return []; }
      }
      const args = candidate.arguments || {};
      let all = [];
      try { all = Array.from(root.querySelectorAll('*')); } catch { all = []; }
      if (candidate.kind === 'role') return all.filter((el) => implicitRole(el) === args.role && accessibleName(el) === String(args.name || ''));
      if (candidate.kind === 'label') return all.filter((el) => (el.labels?.[0]?.textContent || el.getAttribute?.('aria-label') || '').trim() === String(args.text || ''));
      if (candidate.kind === 'placeholder') return all.filter((el) => (el.getAttribute?.('placeholder') || '') === String(args.text || ''));
      if (candidate.kind === 'test_id' || candidate.kind === 'attribute') return all.filter((el) => (el.getAttribute?.(String(args.attribute || 'data-testid')) || '') === String(args.value || ''));
      if (candidate.kind === 'text') return all.filter((el) => (el.innerText || el.textContent || '').trim() === String(args.text || ''));
      if (candidate.kind === 'css') { try { return Array.from(root.querySelectorAll(String(args.selector || ''))); } catch { return []; } }
      return [];
    };
    return (async () => {
      let el = null;
      while (Date.now() < deadline) {
        const found = findMatches();
        if (found.length > 1) return { ok: false, error: "locator became ambiguous", code: "stale_element_reference", match_count: found.length };
        el = found[0] || null;
        if (visible(el)) break;
        await sleep(100);
      }
      if (!el) {
        return { ok: false, error: "element not found", code: "element_not_found" };
      }
      if (!visible(el)) {
        return { ok: false, error: "element not visible before timeout", code: "not_visible" };
      }
      el.scrollIntoView({ block: "center", inline: "center" });
      if (action === "upload") {
        return { ok: true, upload_ready: true };
      } else if (action === "click") {
        el.click();
      } else if (action === "pointer_click") {
        const rect = el.getBoundingClientRect();
        const x = rect.left + rect.width / 2;
        const y = rect.top + rect.height / 2;
        const hit = document.elementFromPoint(x, y);
        if (!(hit === el || el.contains(hit))) return { ok: false, error: "verified center is obscured", code: "pointer_hit_test_failed", x, y };
        return { ok: true, pointer: { x, y, hit_verified: true } };
      } else if (action === "fill") {
        el.focus();
        el.value = value;
        el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value }));
        el.dispatchEvent(new Event("change", { bubbles: true }));
      } else if (action === "type") {
        el.focus();
        for (const char of value) {
          el.value = (el.value || "") + char;
          el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: char }));
        }
      } else if (action === "assert_visible") {
        return { ok: true };
      } else if (action === "assert_text") {
        const text = (el.innerText || el.textContent || el.value || "").trim();
        if (!text.includes(value)) {
          return { ok: false, error: "text assertion failed", code: "assert_text_failed", actual: text };
        }
      } else if (action === "select") {
        el.value = value;
        el.dispatchEvent(new Event("input", { bubbles: true }));
        el.dispatchEvent(new Event("change", { bubbles: true }));
      } else if (action === "press_key") {
        el.focus();
        for (const type of ["keydown", "keyup"]) {
          el.dispatchEvent(new KeyboardEvent(type, { key: value, bubbles: true }));
        }
      }
      return { ok: true };
    })();
  })()`;
  const { result } = await cdp(tab.id, "Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  const evaluated = result?.value ?? { ok: false, error: "execute result missing", code: "execute_failed" };
  if (action === "upload" && evaluated.ok) {
    const upload = await setFileInputFiles(
      tab.id,
      selector,
      candidate,
      locatorContext,
      Array.isArray(files) ? files : [],
    );
    return {
      selector,
      candidate,
      action,
      page_context_revision: currentRevision,
      ...upload,
    };
  }
  if (action === "pointer_click" && evaluated.ok && evaluated.pointer) {
    if (focus) {
      await chrome.windows.update(tab.windowId, { focused: true }).catch(() => null);
      await chrome.tabs.update(tab.id, { active: true }).catch(() => null);
    }
    const { x, y } = evaluated.pointer;
    await cdp(tab.id, "Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    await cdp(tab.id, "Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
    await cdp(tab.id, "Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
    evaluated.pointer.focus_requested = Boolean(focus);
  }
  return {
    selector,
    candidate,
    action,
    page_context_revision: currentRevision,
    ...evaluated,
  };
}

async function waitForBrowserCondition(wait, timeoutMs, target = null) {
  if (!wait) return null;
  const tab = await resolveCommandTab(target);
  const deadline = Date.now() + Math.max(1, Number(timeoutMs) || 5000);
  while (Date.now() < deadline) {
    let matched = false;
    let observed = null;
    if (wait.kind === "url") {
      const current = await chrome.tabs.get(tab.id);
      observed = current.url || "";
      matched = observed.includes(String(wait.pattern || ""));
    } else if (wait.kind === "visible_text") {
      const { result } = await cdp(tab.id, "Runtime.evaluate", {
        expression: `document.body?.innerText?.includes(${JSON.stringify(String(wait.text || ""))}) || false`,
        returnByValue: true,
      });
      matched = result?.value === true;
    } else if (wait.kind === "page_revision_change") {
      observed = await pageContextRevision(tab.id);
      matched = observed !== String(wait.from || "");
    } else if (wait.kind === "load_complete") {
      const current = await chrome.tabs.get(tab.id);
      observed = current.status;
      matched = current.status === "complete";
    } else if (wait.kind === "element_state") {
      const item = wait.element || {};
      if (item.candidate) {
        const checked = await verifyPlaywrightLocators([item.candidate], null, target);
        const first = checked.verification?.[0] || {};
        observed = first;
        if (wait.state === "visible") matched = first.match_count === 1 && first.visible;
        if (wait.state === "hidden") matched = first.match_count === 0 || !first.visible;
        if (wait.state === "enabled") matched = first.match_count === 1 && first.enabled;
        if (wait.state === "disabled") matched = first.match_count === 1 && !first.enabled;
      } else if (item.css) {
        const { result } = await cdp(tab.id, "Runtime.evaluate", {
          expression: `(() => { const el=document.querySelector(${JSON.stringify(item.css)}); if(!el)return {found:false,visible:false,enabled:false}; const s=getComputedStyle(el),r=el.getBoundingClientRect(); return {found:true,visible:s.display!=='none'&&s.visibility!=='hidden'&&r.width>0&&r.height>0,enabled:!el.disabled&&el.getAttribute('aria-disabled')!=='true'}; })()`,
          returnByValue: true,
        });
        observed = result?.value || {};
        if (wait.state === "visible") matched = observed.found && observed.visible;
        if (wait.state === "hidden") matched = !observed.found || !observed.visible;
        if (wait.state === "enabled") matched = observed.found && observed.enabled;
        if (wait.state === "disabled") matched = observed.found && !observed.enabled;
      }
    }
    if (matched) return { ok: true, condition: wait, observed };
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  return {
    ok: false,
    code: "browser_wait_timeout",
    error: "post-action wait condition timed out",
    condition: wait,
  };
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

async function activateTab(tabId, windowId = null, focusWindow = false) {
  const id = Number(tabId);
  if (!Number.isFinite(id) || id <= 0) {
    return { ok: false, error: "tab_id is required" };
  }
  const existing = await chrome.tabs.get(id).catch(() => null);
  if (!existing) {
    return { ok: false, error: `tab not found: ${id}` };
  }
  if (windowId != null && existing.windowId !== Number(windowId)) {
    return { ok: false, error: "tab does not belong to the requested window", code: "stale_browser_target" };
  }
  if (!isTabDebuggable(existing.url)) {
    return {
      ok: false,
      error: "tab cannot be captured (chrome:// and extension pages are not supported)",
    };
  }
  let windowFocused = false;
  if (focusWindow) {
    windowFocused = Boolean(await chrome.windows.update(existing.windowId, { focused: true }).catch(() => null));
  }
  await chrome.tabs.update(id, { active: true });
  const active = await waitForTabReady(id);
  return {
    ok: true,
    tab_id: id,
    url: active.url ?? "",
    title: active.title ?? "",
    focus_requested: Boolean(focusWindow),
    window_focused: windowFocused,
  };
}

async function openTab(url, active, target = null) {
  if (!isExplicitNavigableUrl(url)) return { ok: false, code: "invalid_url", error: "open_tab requires an explicit URL" };
  const current = await resolveCommandTab(target);
  const created = await chrome.tabs.create({ windowId: current.windowId, url, active: Boolean(active) });
  return { ok: true, new_target: { window_id: created.windowId, tab_id: created.id }, url: created.url || url };
}

async function closeTab(target = null) {
  const current = await resolveCommandTab(target);
  await chrome.tabs.remove(current.id);
  return { ok: true, closed_target: { window_id: current.windowId, tab_id: current.id } };
}

async function createWindow(url, focused) {
  if (!isExplicitNavigableUrl(url)) return { ok: false, code: "invalid_url", error: "create_window requires an explicit URL" };
  const created = await chrome.windows.create({ url, focused: Boolean(focused) });
  const tab = created.tabs?.[0];
  return { ok: true, new_target: { window_id: created.id, tab_id: tab?.id }, url: tab?.url || url };
}

async function groupTabs(tabIds, title, target = null) {
  const current = await resolveCommandTab(target);
  const ids = (tabIds || []).map(Number).filter((id) => Number.isFinite(id));
  for (const id of ids) {
    const tab = await chrome.tabs.get(id);
    if (tab.windowId !== current.windowId) return { ok: false, code: "stale_browser_target", error: "all grouped tabs must belong to the leased window" };
  }
  try {
    const groupId = await chrome.tabs.group({ tabIds: ids, createProperties: { windowId: current.windowId } });
    if (title && chrome.tabGroups) await chrome.tabGroups.update(groupId, { title: String(title) });
    return { ok: true, organized: true, group_id: groupId, window_id: current.windowId, tab_ids: ids };
  } catch (_error) {
    return {
      ok: true,
      organized: false,
      window_id: current.windowId,
      tab_ids: ids,
      warning: {
        code: "tab_group_unavailable",
        message: "tab grouping was unavailable; tabs remain open and ungrouped",
      },
    };
  }
}

async function verifyPlaywrightLocators(
  candidates,
  expectedRevision,
  target = null,
) {
  const tab = await attachActiveTab(target);
  const currentRevision = await pageContextRevision(tab.id);
  if (expectedRevision && currentRevision !== String(expectedRevision)) {
    return {
      ok: false,
      code: "stale_page_context",
      error: "page changed after the locator snapshot was acquired",
      page_context_revision: currentRevision,
      verification: (candidates ?? []).map((candidate) => ({
        expression: candidate.expression,
        match_count: 0,
        visible: false,
        enabled: false,
        stale_page_context: true,
      })),
    };
  }
  const expression = `(() => {
    const candidates = ${JSON.stringify(Array.isArray(candidates) ? candidates : [])};
    const implicitRole = (el) => {
      const explicit = el.getAttribute?.('role');
      if (explicit) return explicit;
      const tag = (el.tagName || '').toLowerCase();
      const type = (el.getAttribute?.('type') || '').toLowerCase();
      if (tag === 'button' || (tag === 'input' && ['button','submit','reset'].includes(type))) return 'button';
      if (tag === 'a' && el.hasAttribute('href')) return 'link';
      if (tag === 'textarea') return 'textbox';
      if (tag === 'input') return type === 'checkbox' ? 'checkbox' : type === 'radio' ? 'radio' : 'textbox';
      if (tag === 'select') return 'combobox';
      return '';
    };
    const accessibleName = (el) => {
      const labelledBy = (el.getAttribute?.('aria-labelledby') || '').trim();
      if (labelledBy) {
        const text = labelledBy.split(/\\s+/).map((id) => document.getElementById(id)?.textContent || '').join(' ').trim();
        if (text) return text;
      }
      return (el.getAttribute?.('aria-label') || el.labels?.[0]?.textContent || el.getAttribute?.('alt') || el.getAttribute?.('title') || el.innerText || el.value || el.textContent || '').trim();
    };
    const collect = (root, output = []) => {
      let elements = [];
      try { elements = Array.from(root.querySelectorAll('*')); } catch { elements = []; }
      for (const el of elements) {
        output.push(el);
        if (el.shadowRoot) collect(el.shadowRoot, output);
      }
      let frames = [];
      try { frames = Array.from(root.querySelectorAll('iframe,frame')); } catch { frames = []; }
      for (const frame of frames) {
        try { if (frame.contentDocument) collect(frame.contentDocument, output); } catch { /* cross origin */ }
      }
      return output;
    };
    const all = collect(document);
    const visible = (el) => {
      const style = getComputedStyle(el);
      const rect = el.getBoundingClientRect();
      return style.display !== 'none' && style.visibility !== 'hidden' && Number(style.opacity || '1') > 0 && rect.width > 0 && rect.height > 0;
    };
    const matches = (candidate) => {
      const args = candidate.arguments || {};
      if (candidate.kind === 'role') {
        return all.filter((el) => implicitRole(el) === args.role && accessibleName(el) === String(args.name || ''));
      }
      if (candidate.kind === 'label') {
        return all.filter((el) => (el.labels?.[0]?.textContent || el.getAttribute?.('aria-label') || '').trim() === String(args.text || ''));
      }
      if (candidate.kind === 'placeholder') {
        return all.filter((el) => (el.getAttribute?.('placeholder') || '') === String(args.text || ''));
      }
      if (candidate.kind === 'test_id' || candidate.kind === 'attribute') {
        return all.filter((el) => (el.getAttribute?.(String(args.attribute || 'data-testid')) || '') === String(args.value || ''));
      }
      if (candidate.kind === 'text') {
        return all.filter((el) => (el.innerText || el.textContent || '').trim() === String(args.text || ''));
      }
      if (candidate.kind === 'css') {
        try { return Array.from(document.querySelectorAll(String(args.selector || ''))); } catch { return []; }
      }
      return [];
    };
    return candidates.map((candidate) => {
      const found = matches(candidate);
      const first = found[0];
      return {
        expression: candidate.expression,
        match_count: found.length,
        visible: Boolean(first && visible(first)),
        enabled: Boolean(first && !first.disabled && first.getAttribute?.('aria-disabled') !== 'true'),
      };
    });
  })()`;
  const { result } = await cdp(tab.id, "Runtime.evaluate", {
    expression,
    returnByValue: true,
  });
  return {
    ok: true,
    page_context_revision: currentRevision,
    verification: Array.isArray(result?.value) ? result.value : [],
  };
}

async function captureBrowserEvidence(expectedRevision, target = null) {
  const tab = await attachActiveTab(target);
  const currentRevision = await pageContextRevision(tab.id);
  if (expectedRevision && currentRevision !== String(expectedRevision)) {
    return {
      ok: false,
      code: "stale_page_context",
      error: "page changed before screenshot evidence could be captured",
      page_context_revision: currentRevision,
    };
  }
  const result = await cdp(tab.id, "Page.captureScreenshot", {
    format: "jpeg",
    quality: 80,
    fromSurface: true,
  });
  return {
    ok: true,
    screenshot: result?.data || "",
    page_context_revision: currentRevision,
    url: tab.url ?? "",
    title: tab.title ?? "",
  };
}

async function captureBrowserScreenshot(
  expectedRevision,
  format = "png",
  quality = null,
  fullPage = false,
  selector = null,
  candidate = null,
  locatorContext = null,
  target = null,
) {
  const tab = await attachActiveTab(target);
  const currentRevision = await pageContextRevision(tab.id);
  if (expectedRevision && currentRevision !== String(expectedRevision)) {
    return {
      ok: false,
      code: "stale_page_context",
      error: "page changed before screenshot capture",
      page_context_revision: currentRevision,
    };
  }
  if (!["png", "jpeg"].includes(format)) {
    return { ok: false, code: "invalid_browser_operation", error: "format must be png or jpeg" };
  }
  const params = { format, fromSurface: true, captureBeyondViewport: Boolean(fullPage) };
  if (fullPage) {
    const metrics = await cdp(tab.id, "Page.getLayoutMetrics", {});
    const size = metrics?.cssContentSize || metrics?.contentSize || {};
    const width = Math.ceil(Number(size.width || 0));
    const height = Math.ceil(Number(size.height || 0));
    if (
      width <= 0 ||
      height <= 0 ||
      width > MAX_SCREENSHOT_DIMENSION ||
      height > MAX_SCREENSHOT_DIMENSION ||
      width * height > MAX_SCREENSHOT_PIXELS
    ) {
      return {
        ok: false,
        code: "browser_artifact_failure",
        error: "full-page screenshot exceeds configured dimension limits",
        width,
        height,
        max_dimension: MAX_SCREENSHOT_DIMENSION,
        max_pixels: MAX_SCREENSHOT_PIXELS,
      };
    }
    params.clip = { x: 0, y: 0, width, height, scale: 1 };
  } else if (selector || candidate) {
    const { result: evaluated } = await cdp(tab.id, "Runtime.evaluate", {
      expression: `(() => {
        const selector = ${JSON.stringify(selector)};
        const candidate = ${JSON.stringify(candidate)};
        const context = ${JSON.stringify(locatorContext)} || {};
        let root = document;
        let frameElement = null;
        if (context.frame) {
          const frame = Array.from(document.querySelectorAll('iframe,frame')).find((item) => item.src === context.frame || item.name === context.frame || item.src?.includes(context.frame));
          if (!frame?.contentDocument) return {ok:false, code:'stale_element_reference', error:'frame is unavailable'};
          frameElement = frame;
          root = frame.contentDocument;
        }
        if (context.shadow_root) {
          const host = root.querySelector(context.shadow_root);
          if (!host?.shadowRoot) return {ok:false, code:'stale_element_reference', error:'shadow root is unavailable'};
          root = host.shadowRoot;
        }
        const name = (el) => (el.getAttribute?.('aria-label') || el.labels?.[0]?.textContent || el.innerText || el.value || el.textContent || '').trim();
        const role = (el) => el.getAttribute?.('role') || ((el.tagName || '').toLowerCase() === 'button' ? 'button' : '');
        let found = [];
        if (!candidate) { try { found = Array.from(root.querySelectorAll(selector)); } catch {} }
        else {
          const args = candidate.arguments || {}, all = Array.from(root.querySelectorAll('*'));
          if (candidate.kind === 'role') found = all.filter((el) => role(el) === args.role && name(el) === String(args.name || ''));
          if (candidate.kind === 'label') found = all.filter((el) => (el.labels?.[0]?.textContent || el.getAttribute?.('aria-label') || '').trim() === String(args.text || ''));
          if (candidate.kind === 'placeholder') found = all.filter((el) => (el.getAttribute?.('placeholder') || '') === String(args.text || ''));
          if (candidate.kind === 'test_id' || candidate.kind === 'attribute') found = all.filter((el) => (el.getAttribute?.(String(args.attribute || 'data-testid')) || '') === String(args.value || ''));
          if (candidate.kind === 'text') found = all.filter((el) => (el.innerText || el.textContent || '').trim() === String(args.text || ''));
          if (candidate.kind === 'css') { try { found = Array.from(root.querySelectorAll(String(args.selector || ''))); } catch {} }
        }
        if (found.length !== 1) return {ok:false, code:'stale_element_reference', error:'element is no longer unique', match_count:found.length};
        found[0].scrollIntoView({block:'center', inline:'center'});
        const rect = found[0].getBoundingClientRect();
        if (rect.width <= 0 || rect.height <= 0) return {ok:false, code:'stale_element_reference', error:'element is not visible'};
        const frameRect = frameElement?.getBoundingClientRect();
        let pageOffsetX = window.scrollX + (frameRect?.left || 0);
        let pageOffsetY = window.scrollY + (frameRect?.top || 0);
        return {ok:true, x:rect.left + pageOffsetX, y:rect.top + pageOffsetY, width:rect.width, height:rect.height};
      })()`,
      returnByValue: true,
    });
    const rect = evaluated?.value || {};
    if (!rect.ok) return rect;
    params.clip = { x: rect.x, y: rect.y, width: rect.width, height: rect.height, scale: 1 };
  }
  if (format === "jpeg") {
    params.quality = Number.isInteger(quality) ? Math.max(0, Math.min(100, quality)) : 80;
  }
  const result = await cdp(tab.id, "Page.captureScreenshot", params);
  const artifactData = result?.data || "";
  const artifactBytes = Math.floor(artifactData.length * 3 / 4);
  if (artifactBytes > MAX_ARTIFACT_BYTES) {
    return {
      ok: false,
      code: "browser_artifact_failure",
      error: "browser artifact exceeds the configured byte limit",
      max_bytes: MAX_ARTIFACT_BYTES,
    };
  }
  return {
    ok: true,
    artifact_data: artifactData,
    format,
    page_context_revision: currentRevision,
  };
}

async function generateBrowserPdf(
  expectedRevision,
  paperFormat = "A4",
  landscape = false,
  scale = 1,
  printBackground = false,
  target = null,
) {
  const tab = await attachActiveTab(target);
  const currentRevision = await pageContextRevision(tab.id);
  if (expectedRevision && currentRevision !== String(expectedRevision)) {
    return { ok: false, code: "stale_page_context", error: "page changed before PDF generation", page_context_revision: currentRevision };
  }
  const papers = {
    a4: { paperWidth: 8.27, paperHeight: 11.69 },
    letter: { paperWidth: 8.5, paperHeight: 11 },
    legal: { paperWidth: 8.5, paperHeight: 14 },
  };
  const paper = papers[String(paperFormat).toLowerCase()];
  if (!paper || Number(scale) < 0.1 || Number(scale) > 2) {
    return { ok: false, code: "invalid_browser_operation", error: "unsupported paper format or scale" };
  }
  try {
    const result = await cdp(tab.id, "Page.printToPDF", {
      ...paper,
      landscape: Boolean(landscape),
      scale: Number(scale),
      printBackground: Boolean(printBackground),
      preferCSSPageSize: false,
    });
    const artifactData = result?.data || "";
    const artifactBytes = Math.floor(artifactData.length * 3 / 4);
    if (artifactBytes > MAX_ARTIFACT_BYTES) {
      return { ok: false, code: "browser_artifact_failure", error: "browser artifact exceeds the configured byte limit", max_bytes: MAX_ARTIFACT_BYTES };
    }
    return { ok: true, artifact_data: artifactData, format: "pdf", page_context_revision: currentRevision };
  } catch (err) {
    return { ok: false, code: "browser_capability_unavailable", error: `PDF generation is unavailable: ${captureErrorMessage(err)}` };
  }
}

async function captureMonitoringSummary(target = null) {
  try {
    const tab = await attachActiveTab(target);
    const revision = await pageContextRevision(tab.id);
    const { result } = await cdp(tab.id, "Runtime.evaluate", {
      expression: `(() => {
        const lines = String(document.body?.innerText || '')
          .split(/\\r?\\n/)
          .map((line) => line.replace(/\\s+/g, ' ').trim())
          .filter(Boolean);
        const unique = [];
        const seen = new Set();
        for (const line of lines) {
          const bounded = line.slice(0, 200);
          if (!seen.has(bounded)) { seen.add(bounded); unique.push(bounded); }
          if (unique.length >= 100) break;
        }
        return {
          url: String(location.href || '').slice(0, 4096),
          title: String(document.title || '').slice(0, 500),
          visible_text: unique,
          truncated: lines.length > unique.length,
        };
      })()`,
      returnByValue: true,
    });
    return {
      ok: true,
      page_context_revision: revision,
      ...(result?.value || {}),
    };
  } catch (err) {
    return { ok: false, error: captureErrorMessage(err) };
  }
}

function diffMonitoringSummaries(before, after) {
  if (!before?.ok || !after?.ok) {
    return { available: false };
  }
  const beforeText = new Set(before.visible_text || []);
  const afterText = new Set(after.visible_text || []);
  const allAdded = [...afterText].filter((line) => !beforeText.has(line));
  const allRemoved = [...beforeText].filter((line) => !afterText.has(line));
  return {
    available: true,
    url_changed: before.url !== after.url,
    before_url: before.url,
    after_url: after.url,
    title_changed: before.title !== after.title,
    before_title: before.title,
    after_title: after.title,
    added_text: allAdded.slice(0, 50),
    removed_text: allRemoved.slice(0, 50),
    added_count: allAdded.length,
    removed_count: allRemoved.length,
    truncated: allAdded.length > 50 || allRemoved.length > 50 || Boolean(before.truncated || after.truncated),
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
  if (!cachedBrokerToken) {
    await refreshBridgeCache();
  }
  if (!cachedBrokerToken) {
    throw new Error("bridge discovery did not provide a command token");
  }
  const authenticatedUrl = new URL(url);
  authenticatedUrl.searchParams.set("token", cachedBrokerToken);
  const body = JSON.stringify(payload);
  let lastErr = null;
  for (let attempt = 0; attempt <= BRIDGE_POST_RETRIES; attempt += 1) {
    try {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), BRIDGE_FETCH_TIMEOUT_MS);
      const res = await fetch(authenticatedUrl, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-Teshi-Broker-Token": cachedBrokerToken,
        },
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
  const identity = await getExtensionIdentity();
  try {
    await bridgePost(RESPONSE_URL, {
      type: "frame_error",
      extension_instance_id: identity.extension_instance_id,
      error: message,
    });
  } catch {
    // Best-effort.
  }
  if (streamWs?.readyState === WebSocket.OPEN) {
    try {
      streamWs.send(JSON.stringify({
        type: "frame_error",
        extension_instance_id: identity.extension_instance_id,
        error: message,
      }));
    } catch {
      // ignore
    }
  }
}

const COMMANDS_PAUSING_STREAM = new Set([
  "get_page_snapshot",
  "highlight_selector",
  "clear_highlight",
  "execute_locator",
  "navigate",
  "verify_playwright_locators",
  "capture_browser_evidence",
  "capture_browser_screenshot",
  "generate_browser_pdf",
]);

async function handleCmd(msg) {
  const { cmd, request_id: requestId, selector, tab_id: tabId } = msg;
  const target = msg.target ?? null;
  const pausesStream = COMMANDS_PAUSING_STREAM.has(cmd);
  if (pausesStream) {
    streamPaused = true;
    await pauseScreencast();
  }
  let body;
  try {
    if (cmd === "get_page_snapshot") {
      body = await getPageSnapshot(target);
    } else if (cmd === "highlight_selector") {
      body = await highlightSelector(selector, target);
    } else if (cmd === "clear_highlight") {
      body = await clearHighlight(target);
    } else if (cmd === "execute_locator") {
      const beforeSummary = msg.monitor ? await captureMonitoringSummary(target) : null;
      body = await executeLocator({
        selector,
        candidate: msg.candidate,
        locatorContext: msg.locator_context,
        expectedRevision: msg.page_context_revision,
        action: msg.action,
        value: msg.value,
        files: msg.files,
        timeoutMs: msg.timeout_ms,
        focus: msg.focus,
        target,
      });
      const actionOutcome = body;
      const waitOutcome = body.ok
        ? await waitForBrowserCondition(msg.wait, msg.timeout_ms, target)
        : null;
      const afterSummary = msg.monitor && body.ok
        ? await captureMonitoringSummary(target)
        : null;
      body = {
        ok: body.ok,
        code: body.ok ? undefined : body.code,
        error: body.ok ? undefined : body.error,
        action_outcome: actionOutcome,
        wait_outcome: waitOutcome,
        page_context_revision: body.page_context_revision,
        monitoring: msg.monitor ? {
          before: beforeSummary,
          after: afterSummary,
          diff: diffMonitoringSummaries(beforeSummary, afterSummary),
        } : undefined,
      };
    } else if (cmd === "activate_tab") {
      body = await activateTab(target?.tab_id ?? tabId, target?.window_id ?? msg.window_id, msg.focus_window);
      if (body.ok) {
        streamSessionTabId = body.tab_id;
      }
    } else if (cmd === "open_tab") {
      body = await openTab(msg.url, msg.active, target);
    } else if (cmd === "close_tab") {
      body = await closeTab(target);
    } else if (cmd === "create_window") {
      body = await createWindow(msg.url, msg.focused);
    } else if (cmd === "group_tabs") {
      body = await groupTabs(msg.tab_ids, msg.title, target);
    } else if (cmd === "navigate") {
      const beforeSummary = msg.monitor ? await captureMonitoringSummary(target) : null;
      body = await navigateToUrl(msg.url, msg.timeout_ms, target);
      if (body.ok) {
        streamSessionTabId = body.tab_id;
      }
      const navigateOutcome = body;
      const navigateWait = body.ok
        ? await waitForBrowserCondition(msg.wait, msg.timeout_ms, target)
        : null;
      const afterSummary = msg.monitor && body.ok
        ? await captureMonitoringSummary(target)
        : null;
      body = {
        ...body,
        action_outcome: navigateOutcome,
        wait_outcome: navigateWait,
        monitoring: msg.monitor ? {
          before: beforeSummary,
          after: afterSummary,
          diff: diffMonitoringSummaries(beforeSummary, afterSummary),
        } : undefined,
      };
    } else if (cmd === "go_back") {
      body = await goBack(msg.timeout_ms, target);
    } else if (cmd === "verify_playwright_locators") {
      body = await verifyPlaywrightLocators(
        msg.candidates,
        msg.page_context_revision,
        target,
      );
    } else if (cmd === "capture_browser_evidence") {
      body = await captureBrowserEvidence(msg.page_context_revision, target);
    } else if (cmd === "capture_browser_screenshot") {
      body = await captureBrowserScreenshot(
        msg.page_context_revision,
        msg.format,
        msg.quality,
        msg.full_page,
        msg.selector,
        msg.candidate,
        msg.locator_context,
        target,
      );
    } else if (cmd === "generate_browser_pdf") {
      body = await generateBrowserPdf(
        msg.page_context_revision,
        msg.paper_format,
        msg.landscape,
        msg.scale,
        msg.print_background,
        target,
      );
    } else if (cmd === "start_console_capture") {
      body = await startConsoleCapture(target);
    } else if (cmd === "stop_console_capture") {
      body = await stopConsoleCapture(target);
    } else if (cmd === "start_network_capture") {
      body = await startNetworkCapture(target);
    } else if (cmd === "get_network_response_body") {
      body = await getNetworkResponseBody(msg.network_request_id, target);
    } else if (cmd === "stop_network_capture") {
      body = await stopNetworkCapture(target);
    } else if (cmd === "execute_privileged_javascript") {
      body = await executePrivilegedJavascript(msg.expression, msg.page_context_revision, msg.timeout_ms, msg.max_result_bytes, target);
    } else if (cmd === "execute_privileged_cdp") {
      body = await executePrivilegedCdp(msg.method, msg.params, msg.page_context_revision, msg.max_result_bytes, target);
    } else if (cmd === "list_browser_cookies") {
      body = await listBrowserCookies(msg.include_values, msg.max_entries, msg.max_result_bytes, target);
    } else if (cmd === "access_browser_content_setting") {
      body = await accessBrowserContentSetting(msg.setting, msg.value, target);
    } else if (cmd === "list_browser_extensions") {
      body = await listBrowserExtensions(msg.max_entries);
    } else {
      body = {
        ok: false,
        code: "invalid_browser_operation",
        error: `unknown cmd: ${cmd}`,
      };
    }
  } catch (err) {
    const message = String(err);
    body = {
      ok: false,
      code: message.toLowerCase().includes("debugger")
        ? "debugger_conflict"
        : "browser_operation_failed",
      error: message,
      hint: "Close DevTools on this tab if attach fails.",
    };
  } finally {
    if (target?.tab_id != null && pausesStream) {
      streamSessionTabId = Number(target.tab_id);
    }
    if (pausesStream) {
      streamPaused = false;
      await resumeScreencast();
    } else if ((cmd === "activate_tab" || cmd === "navigate") && body?.ok) {
      await startStreamSession({ tabId: body.tab_id, force: true });
    }
  }
  const identity = await getExtensionIdentity();
  if (body?.new_target) {
    body.new_target.extension_instance_id = identity.extension_instance_id;
  }
  if (body?.closed_target) {
    body.closed_target.extension_instance_id = identity.extension_instance_id;
  }
  return {
    type: "response",
    schema_version: 1,
    protocol_version: PROTOCOL_VERSION,
    extension_instance_id: identity.extension_instance_id,
    target,
    request_id: requestId,
    cmd,
    ...body,
  };
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
      if (!cacheBrokerToken(info)) {
        cachedProjectRoot = "";
        return false;
      }
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
    await connectStreamWebSocket();
    return;
  }
  await startStreamSession({ tabId: tab.id });
}

async function heartbeatOnce(options = {}) {
  const projectRoot = await ensureProjectRoot();
  if (!projectRoot) {
    setBadge(false);
    lastBridgeStatus = {
      connected: false,
      error: "Local Teshi broker is not running.",
    };
    return;
  }
  const tab = await getActiveTab();
  const windowTabs = await listWindowTabs();
  const windows = await listBrowserWindows();
  const identity = await getExtensionIdentity();
  const optionalPermissions = await optionalPermissionStatus();

  let res;
  try {
    res = await bridgePost(HEARTBEAT_URL, {
      schema_version: 1,
      protocol_version: PROTOCOL_VERSION,
      extension_version: chrome.runtime.getManifest().version,
      extension_instance_id: identity.extension_instance_id,
      profile_label: identity.profile_label,
      browser: browserMetadata(),
      features: phasedFeatures(optionalPermissions),
      supported_actions: SUPPORTED_ACTIONS,
      supported_operations: SUPPORTED_OPERATIONS,
      optional_permissions: optionalPermissions,
      project_root: projectRoot,
      url: tab?.url ?? "",
      title: tab?.title ?? "",
      active_window_id: tab?.windowId ?? null,
      active_tab_id: windowTabs.active_tab_id,
      tabs: windowTabs.tabs,
      windows,
      frame_error: "",
    });
  } catch {
    setBadge(false);
    lastBridgeStatus = {
      connected: false,
      error: "Cannot reach the local Teshi broker on 127.0.0.1:17373.",
    };
    return;
  }
  if (!res.ok) {
    setBadge(false);
    cachedProjectRoot = "";
    lastBridgeStatus = {
      connected: false,
      error: `Broker heartbeat failed with HTTP ${res.status}.`,
    };
    return;
  }
  let data;
  try {
    data = await res.json();
  } catch {
    setBadge(false);
    lastBridgeStatus = {
      connected: false,
      error: "Broker heartbeat returned invalid JSON.",
    };
    return;
  }
  if (!data.ok) {
    setBadge(false);
    cachedProjectRoot = "";
    lastBridgeStatus = {
      connected: false,
      error: data.error || "Broker rejected the extension heartbeat.",
      code: data.code,
      required_protocol_version: data.required_protocol_version,
    };
    return;
  }
  setBadge(true);
  lastBridgeStatus = {
    connected: true,
    extension_instance_id: identity.extension_instance_id,
    profile_label: identity.profile_label,
    compatible: data.compatible !== false,
    required_protocol_version: data.required_protocol_version,
  };

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

async function optionalPermissionStatus() {
  const [cookies, contentSettings, management] = await Promise.all([
    chrome.permissions.contains({ permissions: ["cookies"] }),
    chrome.permissions.contains({ permissions: ["contentSettings"] }),
    chrome.permissions.contains({ permissions: ["management"] }),
  ]);
  return {
    cookies,
    content_settings: contentSettings,
    extension_management: management,
  };
}

chrome.permissions.onAdded.addListener(() => void heartbeatOnce());
chrome.permissions.onRemoved.addListener(() => void heartbeatOnce());

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
  if (message?.type === "get_bridge_status") {
    void (async () => {
      const identity = await getExtensionIdentity();
      sendResponse({
        ok: true,
        identity,
        protocol_version: PROTOCOL_VERSION,
        extension_version: chrome.runtime.getManifest().version,
        optional_permissions: await optionalPermissionStatus(),
        ...lastBridgeStatus,
      });
    })();
    return true;
  }
  if (message?.type === "set_profile_label") {
    void (async () => {
      const identity = await setProfileLabel(message.profile_label);
      await heartbeatOnce();
      sendResponse({ ok: true, identity });
    })();
    return true;
  }
  if (message?.type === "connect_now") {
    cachedProjectRoot = "";
    cachedBrokerToken = "";
    extensionFrameWsUrl = "";
    closeStreamWebSocket();
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
  void getExtensionIdentity();
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
  consoleCaptureTabIds.delete(tabId);
  networkCaptureTabIds.delete(tabId);
  if (tabId === attachedTabId) {
    attachedTabId = null;
  }
  if (tabId === streamSessionTabId) {
    void stopStreamSession();
  }
});

chrome.debugger.onDetach.addListener((source) => {
  consoleCaptureTabIds.delete(source.tabId);
  networkCaptureTabIds.delete(source.tabId);
  if (source.tabId === attachedTabId) {
    attachedTabId = null;
    streamPageEnabled = false;
  }
  if (source.tabId === streamSessionTabId) {
    screencastActive = false;
    streamSessionTabId = null;
  }
});

setInterval(() => {
  void heartbeatLoop();
}, HEARTBEAT_MS);

ensureScreencastDebuggerListener();
void heartbeatLoop({ forceStream: true });
