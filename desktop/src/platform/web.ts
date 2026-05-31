import type { FeatureRenderPayload } from "../types";
import type { ActiveStep, PendingLocator } from "../locatorTypes";
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

function ensureEventsSocket(): WebSocket {
  if (eventsSocket && eventsSocket.readyState === WebSocket.OPEN) {
    return eventsSocket;
  }
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  const url = `${proto}//${window.location.host}${API}/events`;
  eventsSocket = new WebSocket(url);
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

/** Loopback HTTP + WebSocket host (`teshi web`). */
export const webRuntime: TeshiRuntimeApi = {
  async checkProjectSwitchAllowed() {
    return apiFetch<boolean>("/projects/switch-allowed");
  },

  async teardownRuntime() {
    await apiFetch<void>("/projects/teardown", { method: "POST" });
  },

  async openProject(path: string) {
    await apiFetch<void>("/projects/open", {
      method: "POST",
      body: JSON.stringify({ path }),
    });
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

  async startBrowserSidecar() {
    return apiFetch<{ ws_url: string }>("/browser/start", { method: "POST" });
  },

  async stopBrowserSidecar() {
    await apiFetch<void>("/browser/stop", { method: "POST" });
  },

  async listDir(path: string) {
    const q = new URLSearchParams({ path });
    return apiFetch<DirEntry[]>(`/fs/list?${q}`);
  },

  async spawnTerminal() {
    await apiFetch<void>("/terminal/spawn", { method: "POST" });
  },

  async stopTerminal() {
    await apiFetch<void>("/terminal/stop", { method: "POST" });
  },

  async resizeTerminal(cols: number, rows: number) {
    await apiFetch<void>("/terminal/resize", {
      method: "POST",
      body: JSON.stringify({ cols, rows }),
    });
  },

  async writeTerminal(data: string) {
    await apiFetch<void>("/terminal/write", {
      method: "POST",
      body: JSON.stringify({ data }),
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

  async confirmStopRuntimeIfBusy() {
    const allowed = await this.checkProjectSwitchAllowed();
    if (allowed) {
      return true;
    }
    return window.confirm(
      "Browser/Terminal is running. Continuing will stop them.",
    );
  },

  async onEvent<T>(event: string, handler: (payload: T) => void) {
    ensureEventsSocket();
    let set = eventHandlers.get(event);
    if (!set) {
      set = new Set();
      eventHandlers.set(event, set);
    }
    const wrapper = (payload: unknown) => handler(payload as T);
    set.add(wrapper);
    return () => {
      set?.delete(wrapper);
    };
  },
};
