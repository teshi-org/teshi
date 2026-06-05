import type { FeatureRenderPayload } from "../types";
import type { ActiveStep, PendingLocator, StepBindingStatus } from "../locatorTypes";
import type { DirEntry } from "../types";
import type { TeshiRuntimeApi } from "./types";

const API = "/api/v1";

async function apiFetch<T>(
  path: string,
  init?: RequestInit,
): Promise<T> {
  const res = await fetch(`${API}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...init?.headers,
    },
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    const msg =
      typeof body === "object" && body && "error" in body
        ? String((body as { error: string }).error)
        : res.statusText;
    throw new Error(msg);
  }
  if (res.status === 204) {
    return undefined as T;
  }
  return res.json() as Promise<T>;
}

let eventsSocket: WebSocket | null = null;
const eventHandlers = new Map<string, Set<(payload: unknown) => void>>();
const terminalExclusiveUnsubs = new Map<string, () => void>();

/** Serializes PTY stdin/resize HTTP calls so keystrokes cannot reorder (breaks TUI redraw). */
let terminalIoChain: Promise<void> = Promise.resolve();

function enqueueTerminalIo<T>(fn: () => Promise<T>): Promise<T> {
  const run = terminalIoChain.then(fn);
  terminalIoChain = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

function ensureEventsSocket(): WebSocket {
  if (eventsSocket && eventsSocket.readyState === WebSocket.OPEN) {
    return eventsSocket;
  }
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  const url = `${proto}//${window.location.host}${API}/events`;
  eventsSocket = new WebSocket(url);
  eventsSocket.onclose = () => {
    if (eventsSocket?.url === url) {
      eventsSocket = null;
    }
  };
  eventsSocket.onmessage = (msg) => {
    try {
      const { event, payload } = JSON.parse(msg.data as string) as {
        event: string;
        payload: unknown;
      };
      const handlers = eventHandlers.get(event);
      if (handlers) {
        handlers.forEach((h) => h(payload));
      }
    } catch {
      // ignore malformed frames
    }
  };
  return eventsSocket;
}

/** Dispatches a host event to local subscribers (fallback when WS is slow or unavailable). */
function emitLocalEvent(event: string, payload: unknown): void {
  eventHandlers.get(event)?.forEach((handler) => handler(payload));
}

/** Loopback HTTP + WebSocket host (`teshi web`). */
export const webRuntime: TeshiRuntimeApi = {
  async checkProjectSwitchAllowed() {
    return apiFetch<boolean>("/projects/switch-allowed");
  },

  async teardownRuntime() {
    await apiFetch<void>("/projects/teardown", { method: "POST" });
  },

  async openProject(path: string) {
    const { root } = await apiFetch<{ root: string }>("/projects/open", {
      method: "POST",
      body: JSON.stringify({ path }),
    });
    emitLocalEvent("project-changed", root);
  },

  async getRecentProjects() {
    return apiFetch<string[]>("/settings/recent");
  },

  async openProjectDir() {
    const path = window.prompt("Enter the absolute path to your BDD project:");
    if (!path?.trim()) {
      return null;
    }
    return path.trim();
  },

  async getPendingLocator() {
    return apiFetch<PendingLocator | null>("/locator/pending");
  },

  async getStepBindingStatuses(featurePath: string) {
    const q = new URLSearchParams({ feature_path: featurePath });
    return apiFetch<StepBindingStatus[]>(`/steps/statuses?${q}`);
  },

  async getActiveStep() {
    return apiFetch<ActiveStep | null>("/locator/active-step");
  },

  async syncActiveStep(featurePath: string, stepLine: number) {
    return apiFetch<ActiveStep>("/locator/sync-step", {
      method: "POST",
      body: JSON.stringify({ feature_path: featurePath, step_line: stepLine }),
    });
  },

  async renderFeature(path: string) {
    return apiFetch<FeatureRenderPayload>("/gherkin/render", {
      method: "POST",
      body: JSON.stringify({ path }),
    });
  },

  async startBrowserSidecar(mode: "embedded" | "chrome" | "winapp") {
    return apiFetch<{ ws_url: string; mode: string }>("/browser/start", {
      method: "POST",
      body: JSON.stringify({ mode }),
    });
  },

  async stopBrowserSidecar() {
    await apiFetch<void>("/browser/stop", { method: "POST" });
  },

  async listDir(path: string) {
    const q = new URLSearchParams({ path });
    return apiFetch<DirEntry[]>(`/fs/list?${q}`);
  },

  async spawnTerminal(cols: number, rows: number) {
    return enqueueTerminalIo(() =>
      apiFetch<void>("/terminal/spawn", {
        method: "POST",
        body: JSON.stringify({ cols, rows }),
      }),
    );
  },

  async stopTerminal() {
    await apiFetch<void>("/terminal/stop", { method: "POST" });
  },

  async resizeTerminal(cols: number, rows: number) {
    return enqueueTerminalIo(() =>
      apiFetch<void>("/terminal/resize", {
        method: "POST",
        body: JSON.stringify({ cols, rows }),
      }),
    );
  },

  async writeTerminal(data: string) {
    return enqueueTerminalIo(() =>
      apiFetch<void>("/terminal/write", {
        method: "POST",
        body: JSON.stringify({ data }),
      }),
    );
  },

  async highlightLocator(selector: string) {
    await apiFetch<void>("/locator/highlight", {
      method: "POST",
      body: JSON.stringify({ selector }),
    });
  },

  async confirmLocator(candidateRank: number, editedValue: string | null) {
    await apiFetch<void>("/locator/confirm", {
      method: "POST",
      body: JSON.stringify({
        candidate_rank: candidateRank,
        edited_value: editedValue,
      }),
    });
  },

  async rejectLocator() {
    await apiFetch<void>("/locator/reject", { method: "POST" });
  },

  async unbindStep(featurePath: string, stepLine: number) {
    await apiFetch<void>("/steps/unbind", {
      method: "POST",
      body: JSON.stringify({ feature_path: featurePath, step_line: stepLine }),
    });
  },

  async getProjectSettings() {
    return apiFetch<{ locator_auto_confirm_sec: number }>("/settings/project");
  },

  async confirmStopRuntimeIfBusy() {
    const e2e =
      (window as Window & { __TESHI_E2E__?: boolean }).__TESHI_E2E__ === true ||
      localStorage.getItem("TESHI_AUTO_TEARDOWN") === "1";
    const allowed = await this.checkProjectSwitchAllowed();
    if (allowed) {
      return true;
    }
    if (e2e) {
      await this.teardownRuntime();
      return true;
    }
    return window.confirm(
      "Browser/Terminal is running. Continuing will stop them.",
    );
  },

  async onEvent<T>(event: string, handler: (payload: T) => void) {
    ensureEventsSocket();
    // Only one PTY output subscriber: Vite HMR can remount the panel without running
    // prior cleanups, which otherwise duplicates xterm.write and doubles characters.
    if (event === "terminal-output" || event === "terminal-exit") {
      terminalExclusiveUnsubs.get(event)?.();
      terminalExclusiveUnsubs.delete(event);
    }
    let set = eventHandlers.get(event);
    if (!set) {
      set = new Set();
      eventHandlers.set(event, set);
    }
    const wrapper = (payload: unknown) => handler(payload as T);
    set.add(wrapper);
    const unlisten = () => {
      set?.delete(wrapper);
      if (terminalExclusiveUnsubs.get(event) === unlisten) {
        terminalExclusiveUnsubs.delete(event);
      }
    };
    if (event === "terminal-output" || event === "terminal-exit") {
      terminalExclusiveUnsubs.set(event, unlisten);
    }
    return unlisten;
  },
};

/**
 * Full automated project open for E2E (`?e2e=1`) and sidecar `open_project`.
 * Mirrors App teardown + open without relying solely on the events WebSocket.
 */
export async function e2eOpenProject(path: string): Promise<string> {
  const allowed = await webRuntime.confirmStopRuntimeIfBusy();
  if (!allowed) {
    throw new Error("project switch cancelled");
  }
  await webRuntime.teardownRuntime();
  const { root } = await apiFetch<{ root: string }>("/projects/open", {
    method: "POST",
    body: JSON.stringify({ path }),
  });
  emitLocalEvent("project-changed", root);
  const recent = await webRuntime.getRecentProjects();
  emitLocalEvent("recent-loaded", recent);
  return root;
}

declare global {
  interface Window {
    __teshiE2eOpenProject?: (path: string) => Promise<string>;
    __teshiE2eWriteTerminal?: (data: string) => Promise<void>;
  }
}

window.__teshiE2eOpenProject = e2eOpenProject;

const e2eWriteTerminal = async (data: string): Promise<void> => {
  await apiFetch<void>("/terminal/write", {
    method: "POST",
    body: JSON.stringify({ data }),
  });
};
window.__teshiE2eWriteTerminal = e2eWriteTerminal;
