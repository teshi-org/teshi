import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";
import { webcrypto } from "node:crypto";

const testDir = dirname(fileURLToPath(import.meta.url));
const backgroundSource = readFileSync(
  resolve(testDir, "..", "background.js"),
  "utf8",
);

function listenerRegistry() {
  const listeners = [];
  return {
    listeners,
    api: {
      addListener(listener) {
        listeners.push(listener);
      },
    },
  };
}

function loadBackground() {
  const debuggerEvents = listenerRegistry();
  const debuggerDetaches = listenerRegistry();
  const tabActivations = listenerRegistry();
  const tabRemovals = listenerRegistry();
  const commandCalls = [];
  const attachCalls = [];
  const detachCalls = [];
  const tabs = new Map([
    [11, { id: 11, windowId: 1, url: "https://one.example/", active: true, status: "complete" }],
    [22, { id: 22, windowId: 1, url: "https://two.example/", active: false, status: "complete" }],
  ]);
  const noopEvent = listenerRegistry().api;

  class FakeWebSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSED = 3;

    static instances = [];

    constructor(url) {
      this.url = url;
      this.readyState = FakeWebSocket.CONNECTING;
      this.sent = [];
      FakeWebSocket.instances.push(this);
    }

    send(value) {
      this.sent.push(value);
    }

    close() {
      this.readyState = FakeWebSocket.CLOSED;
    }
  }

  const chrome = {
    action: {
      setBadgeText() {},
      setBadgeBackgroundColor() {},
    },
    alarms: {
      create() {},
      onAlarm: noopEvent,
    },
    cookies: { async getAll() { return []; } },
    debugger: {
      onEvent: debuggerEvents.api,
      onDetach: debuggerDetaches.api,
      async attach(target) {
        attachCalls.push(target.tabId);
      },
      async detach(target) {
        detachCalls.push(target.tabId);
      },
      async sendCommand(target, method, params) {
        commandCalls.push({ tabId: target.tabId, method, params });
        if (method === "Network.getRequestPostData") {
          return { postData: "remote-😀-body" };
        }
        return {};
      },
    },
    management: { async getAll() { return []; } },
    permissions: {
      async contains() { return false; },
      onAdded: noopEvent,
      onRemoved: noopEvent,
    },
    runtime: {
      lastError: null,
      getManifest() { return { version: "test" }; },
      onMessage: noopEvent,
      onInstalled: noopEvent,
      onStartup: noopEvent,
    },
    storage: {
      local: {
        async get() { return {}; },
        async set() {},
      },
    },
    tabs: {
      onActivated: tabActivations.api,
      onRemoved: tabRemovals.api,
      async get(tabId) {
        const tab = tabs.get(tabId);
        if (!tab) throw new Error("tab not found");
        return tab;
      },
      async query(query) {
        const values = [...tabs.values()];
        if (query.active) return values.filter((tab) => tab.active);
        return values;
      },
      async update(tabId, update) {
        const tab = tabs.get(tabId);
        Object.assign(tab, update);
        return tab;
      },
      async create() { throw new Error("not used"); },
      async remove(tabId) { tabs.delete(tabId); },
      async group() { return 1; },
    },
    windows: {
      async getAll() { return []; },
      async update() { return {}; },
      async create() { throw new Error("not used"); },
    },
  };

  const context = vm.createContext({
    __TESHI_BRIDGE_TEST__: true,
    chrome,
    navigator: { userAgent: "Chrome/test", platform: "test" },
    crypto: webcrypto,
    URL,
    TextEncoder,
    TextDecoder,
    Uint8Array,
    DataView,
    ArrayBuffer,
    AbortController,
    WebSocket: FakeWebSocket,
    atob,
    fetch: async () => ({ ok: false }),
    setInterval: () => 1,
    clearInterval() {},
    setTimeout: () => 1,
    clearTimeout() {},
    console,
  });
  vm.runInContext(backgroundSource, context, { filename: "background.js" });
  return {
    hooks: context.__teshiBridgeTestHooks,
    FakeWebSocket,
    commandCalls,
    attachCalls,
    detachCalls,
    debuggerDetaches,
    tabActivations,
    tabRemovals,
  };
}

function captureState(overrides = {}) {
  return {
    active: true,
    capture_id: "capture-a",
    target: { extension_instance_id: "profile-a", window_id: 1, tab_id: 11 },
    allowed_hostnames: new Set(["api.example.com"]),
    capture_request_bodies: true,
    max_request_body_bytes: 12,
    matched_request_ids: new Set(),
    next_seq: 1,
    acked_seq: 0,
    sent_through_seq: 0,
    queue: [],
    queue_bytes: 0,
    dropped_events_total: 0,
    dropped_bytes_total: 0,
    termination_reason: "",
    termination_detail: "",
    termination_sent: false,
    termination_acknowledged: false,
    processing: Promise.resolve(),
    ...overrides,
  };
}

test("exact hostname filtering normalizes case and rejects suffix matches", () => {
  const { hooks } = loadBackground();
  const normalized = [...hooks.normalizeAllowedHostnames(["API.Example.com.", "api.example.com"])];
  assert.deepEqual(normalized, ["api.example.com"]);
  const allowed = new Set(normalized);
  assert.equal(hooks.hostnameAllowed("https://api.example.com/v1", allowed), true);
  assert.equal(hooks.hostnameAllowed("https://evil-api.example.com/v1", allowed), false);
  assert.equal(hooks.hostnameAllowed("https://api.example.com.evil/v1", allowed), false);
  assert.throws(() => hooks.normalizeAllowedHostnames(["*.example.com"]));
  assert.throws(() => hooks.normalizeAllowedHostnames([]));
});

test("request bodies are byte bounded and fallback to CDP post data", async () => {
  const { hooks, commandCalls } = loadBackground();
  const bounded = hooks.boundedUtf8RequestBody("ab😀cd", 5);
  assert.equal(bounded.body, "ab");
  assert.equal(bounded.captured_size, 2);
  assert.equal(bounded.original_size, 8);
  assert.equal(bounded.truncated, true);

  const state = captureState({ max_request_body_bytes: 11 });
  const body = await hooks.captureRequestBody(11, { hasPostData: true }, "req-1", state);
  assert.equal(body.body, "remote-😀");
  assert.equal(body.captured_size, 11);
  assert.equal(body.truncated, true);
  assert.ok(commandCalls.some((call) => call.method === "Network.getRequestPostData"));
});

test("nonmatching requests are discarded before request body retrieval", async () => {
  const { hooks, commandCalls } = loadBackground();
  const state = captureState();
  hooks.networkCapturesByTab.set(11, state);

  await hooks.publishNetworkEvent(11, "Network.requestWillBeSent", {
    requestId: "ignored",
    request: {
      url: "https://other.example.com/upload",
      method: "POST",
      hasPostData: true,
    },
  }, state);
  assert.equal(state.queue.length, 0);
  assert.equal(
    commandCalls.filter((call) => call.method === "Network.getRequestPostData").length,
    0,
  );

  await hooks.publishNetworkEvent(11, "Network.requestWillBeSent", {
    requestId: "kept",
    request: {
      url: "https://api.example.com/upload",
      method: "POST",
      hasPostData: true,
    },
  }, state);
  assert.equal(state.queue.length, 1);
  assert.equal(state.queue[0].event.request_body.encoding, "utf8");
  assert.equal(
    commandCalls.filter((call) => call.method === "Network.getRequestPostData").length,
    1,
  );
});

test("debugger roles preserve network attachment while preview moves tabs", async () => {
  const { hooks, attachCalls, detachCalls } = loadBackground();
  const tabOne = { id: 11 };
  const tabTwo = { id: 22 };
  await hooks.acquireDebuggerRole(tabOne, "network", ["Network"]);
  await hooks.acquireDebuggerRole(tabOne, "preview", ["Page"]);
  await hooks.releaseDebuggerRole(11, "preview");
  await hooks.acquireDebuggerRole(tabTwo, "preview", ["Page"]);

  assert.deepEqual(attachCalls, [11, 22]);
  assert.deepEqual(detachCalls, []);
  assert.equal(hooks.debuggerSessions.get(11).roles.has("network"), true);
  assert.equal(hooks.debuggerSessions.get(22).roles.has("preview"), true);

  await hooks.releaseDebuggerRole(11, "network");
  assert.deepEqual(detachCalls, [11]);
});

test("network batches remain queued until a matching contiguous ack", () => {
  const { hooks, FakeWebSocket } = loadBackground();
  const socket = new FakeWebSocket();
  socket.readyState = FakeWebSocket.OPEN;
  hooks.setStreamWebSocketForTest(socket);
  const state = captureState();
  hooks.networkDeliveryStates.set(
    "profile-a:1:11:capture-a",
    state,
  );

  hooks.enqueueNetworkEvent(state, { event_type: "request", request_id: "one" });
  hooks.enqueueNetworkEvent(state, { event_type: "finished", request_id: "one" });
  const batches = socket.sent.map((value) => JSON.parse(value));
  assert.equal(batches[0].type, "network_batch");
  assert.equal(batches[0].capture_id, "capture-a");
  assert.deepEqual(batches[0].target, state.target);
  assert.equal(batches[0].events[0].seq, 1);
  assert.equal(batches[0].dropped_events_total, 0);
  assert.equal(state.queue.length, 2);

  hooks.handleNetworkAck({
    type: "network_ack",
    capture_id: "wrong-capture",
    target: state.target,
    ack_seq: 2,
  });
  assert.equal(state.queue.length, 2);
  hooks.handleNetworkAck({
    type: "network_ack",
    capture_id: "capture-a",
    target: state.target,
    ack_seq: 1,
  });
  assert.equal(state.queue.length, 1);
  assert.equal(state.queue[0].seq, 2);

  hooks.reportNetworkTermination(state, "tab_closed");
  const terminationBatch = JSON.parse(socket.sent.at(-1));
  assert.equal(terminationBatch.type, "network_batch");
  assert.equal(terminationBatch.termination_reason, "tab_closed");
  assert.equal(terminationBatch.final_sequence, 2);
  hooks.handleNetworkAck({
    type: "network_ack",
    capture_id: "capture-a",
    target: state.target,
    ack_seq: 2,
    accepted: true,
  });
  assert.equal(hooks.networkDeliveryStates.size, 0);
});

test("clear establishes a sequence barrier and discards pre-clear delivery", async () => {
  const { hooks } = loadBackground();
  const state = captureState();
  state.matched_request_ids.add("in-flight");
  hooks.networkCapturesByTab.set(11, state);
  hooks.networkDeliveryStates.set("profile-a:1:11:capture-a", state);
  hooks.enqueueNetworkEvent(state, { event_type: "request", request_id: "one" });
  hooks.enqueueNetworkEvent(state, { event_type: "finished", request_id: "one" });

  const result = await hooks.clearNetworkCapture({ window_id: 1, tab_id: 11 });

  assert.equal(result.ok, true);
  assert.equal(result.sequence_barrier, 2);
  assert.equal(state.queue.length, 0);
  assert.equal(state.queue_bytes, 0);
  assert.equal(state.acked_seq, 2);
  assert.equal(state.matched_request_ids.size, 0);
  assert.equal(hooks.networkCapturesByTab.get(11), state);
});

test("a reconnected authenticated socket resends unacknowledged events", async () => {
  const { hooks, FakeWebSocket } = loadBackground();
  const state = captureState();
  hooks.networkDeliveryStates.set("profile-a:1:11:capture-a", state);
  hooks.networkCapturesByTab.set(11, state);
  hooks.enqueueNetworkEvent(state, { event_type: "request", request_id: "retry-me" });
  hooks.setBridgeContextForTest("D:/project", "ws://127.0.0.1/extension/frames");

  const firstConnect = hooks.connectStreamWebSocket();
  const firstSocket = FakeWebSocket.instances.at(-1);
  firstSocket.readyState = FakeWebSocket.OPEN;
  await firstSocket.onopen();
  await firstConnect;
  const firstBatch = firstSocket.sent
    .map((value) => JSON.parse(value))
    .find((message) => message.type === "network_batch");
  assert.equal(firstBatch.events[0].seq, 1);
  assert.equal(state.queue.length, 1);

  firstSocket.readyState = FakeWebSocket.CLOSED;
  firstSocket.onclose();
  const secondConnect = hooks.connectStreamWebSocket();
  const secondSocket = FakeWebSocket.instances.at(-1);
  assert.notEqual(secondSocket, firstSocket);
  secondSocket.readyState = FakeWebSocket.OPEN;
  await secondSocket.onopen();
  await secondConnect;
  const resentBatch = secondSocket.sent
    .map((value) => JSON.parse(value))
    .find((message) => message.type === "network_batch");
  assert.equal(resentBatch.events[0].seq, 1);
  assert.equal(state.queue.length, 1);
});

test("tab activation preserves captures and lifecycle loss is target scoped", async () => {
  const {
    hooks,
    debuggerDetaches,
    tabActivations,
    tabRemovals,
  } = loadBackground();
  await hooks.startNetworkCapture(
    { window_id: 1, tab_id: 11 },
    "capture-one",
    ["one.example"],
    false,
    0,
  );
  await hooks.startNetworkCapture(
    { window_id: 1, tab_id: 22 },
    "capture-two",
    ["two.example"],
    false,
    0,
  );
  tabActivations.listeners[0]({ tabId: 22, windowId: 1 });
  assert.equal(hooks.networkCapturesByTab.size, 2);

  tabRemovals.listeners[0](11);
  assert.equal(hooks.networkCapturesByTab.has(11), false);
  assert.equal(hooks.networkCapturesByTab.has(22), true);
  const captureOne = [...hooks.networkDeliveryStates.values()]
    .find((state) => state.capture_id === "capture-one");
  assert.equal(captureOne.termination_reason, "tab_closed");

  debuggerDetaches.listeners[0]({ tabId: 22 }, "replaced_with_devtools");
  assert.equal(hooks.networkCapturesByTab.has(22), false);
  const captureTwo = [...hooks.networkDeliveryStates.values()]
    .find((state) => state.capture_id === "capture-two");
  assert.equal(captureTwo.termination_reason, "debugger_detached");
  assert.equal(
    captureTwo.termination_detail,
    "replaced_with_devtools",
  );
});
