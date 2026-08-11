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
  closeStreamWebSocket();
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
  await detachIfNeeded();
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

async function executeLocator({ selector, action, value, timeoutMs = 5000, target = null }) {
  if (!selector) {
    return { ok: false, error: "selector is required", code: "invalid_selector" };
  }
  const allowed = new Set([
    "click",
    "fill",
    "assert_visible",
    "assert_text",
    "select",
    "press_key",
  ]);
  if (!allowed.has(action)) {
    return {
      ok: false,
      error: `unsupported action: ${action}`,
      code: "unsupported_action",
    };
  }
  if (["fill", "assert_text", "select", "press_key"].includes(action) && value == null) {
    return {
      ok: false,
      error: `value is required for ${action}`,
      code: "missing_value",
    };
  }

  const tab = await attachActiveTab(target);
  const expression = `(() => {
    const selector = ${JSON.stringify(selector)};
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
    return (async () => {
      let el = null;
      while (Date.now() < deadline) {
        el = document.querySelector(selector);
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
      if (action === "click") {
        el.click();
      } else if (action === "fill") {
        el.focus();
        el.value = value;
        el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value }));
        el.dispatchEvent(new Event("change", { bubbles: true }));
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
  return {
    selector,
    action,
    ...(result?.value ?? { ok: false, error: "execute result missing", code: "execute_failed" }),
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

async function activateTab(tabId, windowId = null) {
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
  await chrome.windows.update(existing.windowId, { focused: true }).catch(() => null);
  await chrome.tabs.update(id, { active: true });
  const active = await waitForTabReady(id);
  return {
    ok: true,
    tab_id: id,
    url: active.url ?? "",
    title: active.title ?? "",
  };
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
      body = await executeLocator({
        selector,
        action: msg.action,
        value: msg.value,
        timeoutMs: msg.timeout_ms,
        target,
      });
    } else if (cmd === "activate_tab") {
      body = await activateTab(tabId, target?.window_id ?? msg.window_id);
      if (body.ok) {
        streamSessionTabId = body.tab_id;
      }
    } else if (cmd === "navigate") {
      body = await navigateToUrl(msg.url, msg.timeout_ms, target);
      if (body.ok) {
        streamSessionTabId = body.tab_id;
      }
    } else if (cmd === "verify_playwright_locators") {
      body = await verifyPlaywrightLocators(
        msg.candidates,
        msg.page_context_revision,
        target,
      );
    } else if (cmd === "capture_browser_evidence") {
      body = await captureBrowserEvidence(msg.page_context_revision, target);
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

  let res;
  try {
    res = await fetch(HEARTBEAT_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        schema_version: 1,
        protocol_version: PROTOCOL_VERSION,
        extension_version: chrome.runtime.getManifest().version,
        extension_instance_id: identity.extension_instance_id,
        profile_label: identity.profile_label,
        browser: browserMetadata(),
        project_root: projectRoot,
        url: tab?.url ?? "",
        title: tab?.title ?? "",
        active_window_id: tab?.windowId ?? null,
        active_tab_id: windowTabs.active_tab_id,
        tabs: windowTabs.tabs,
        windows,
        frame_error: "",
      }),
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
  }
});

setInterval(() => {
  void heartbeatLoop();
}, HEARTBEAT_MS);

ensureScreencastDebuggerListener();
void heartbeatLoop({ forceStream: true });
