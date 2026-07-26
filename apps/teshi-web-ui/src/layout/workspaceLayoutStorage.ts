import type { Layout } from "react-resizable-panels";

/** localStorage key for the per-project layout map. */
export const WORKSPACE_LAYOUTS_STORAGE_KEY = "teshi.workspaceLayouts.v1";

/** Maximum number of projects to retain layout entries for. */
export const WORKSPACE_LAYOUTS_MAX_ENTRIES = 30;

/** Default three-column percentages when no valid saved layout exists. */
export const DEFAULT_PANEL_LAYOUT = {
  gherkin: 30,
  browser: 45,
  files: 25,
} as const;

/** Default vertical percentages for the main workspace and bottom dock. */
export const DEFAULT_DOCK_LAYOUT = {
  main: 75,
  dock: 25,
} as const;

export type WorkspacePanelLayout = {
  gherkin: number;
  browser: number;
  files: number;
};

export type WorkspaceDockLayout = {
  main: number;
  dock: number;
};

export type WorkspaceDockTab = "locator" | "output" | "logs" | "screenshots";

export type WorkspaceLayoutState = {
  version: 1;
  layout: WorkspacePanelLayout;
  dockLayout: WorkspaceDockLayout;
  gherkinCollapsed: boolean;
  filesCollapsed: boolean;
  dockExpanded: boolean;
  dockActiveTab: WorkspaceDockTab;
  lastUsed: number;
};

type StoredEntry = WorkspaceLayoutState;

type StoredMap = Record<string, StoredEntry>;

/** Sum tolerance when validating that panel percentages add up to 100. */
const LAYOUT_SUM_TOLERANCE = 0.5;

/** Injectable storage for unit tests; defaults to `localStorage` in the browser. */
export interface LayoutStorageBackend {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

function defaultBackend(): LayoutStorageBackend | null {
  try {
    if (typeof localStorage !== "undefined") {
      return localStorage;
    }
  } catch {
    // private mode or disabled storage
  }
  return null;
}

/**
 * Stable key for a project root path (canonical paths from the runtime on Windows
 * are compared case-insensitively).
 */
export function normalizeProjectKey(projectRoot: string): string {
  const normalized = projectRoot.replace(/\\/g, "/");
  if (/^[a-zA-Z]:/.test(normalized)) {
    return normalized.toLowerCase();
  }
  return normalized;
}

/** Returns true when the three panel sizes are positive and sum to ~100%. */
export function validatePanelLayout(layout: WorkspacePanelLayout): boolean {
  const values = [layout.gherkin, layout.browser, layout.files];
  if (
    values.some(
      (v) => typeof v !== "number" || !Number.isFinite(v) || v <= 0 || v > 100,
    )
  ) {
    return false;
  }
  const sum = layout.gherkin + layout.browser + layout.files;
  return Math.abs(sum - 100) <= LAYOUT_SUM_TOLERANCE;
}

/** Returns true when the main/dock sizes are positive and sum to ~100%. */
export function validateDockLayout(layout: WorkspaceDockLayout): boolean {
  const values = [layout.main, layout.dock];
  if (
    values.some(
      (v) => typeof v !== "number" || !Number.isFinite(v) || v <= 0 || v > 100,
    )
  ) {
    return false;
  }
  return Math.abs(layout.main + layout.dock - 100) <= LAYOUT_SUM_TOLERANCE;
}

/** Extracts the main workspace panel layout from a react-resizable-panels layout map. */
export function panelLayoutFromGroupLayout(
  layout: Layout,
): WorkspacePanelLayout | null {
  const gherkin = layout.gherkin;
  const browser = layout.browser;
  const files = layout.files;
  if (
    typeof gherkin !== "number" ||
    typeof browser !== "number" ||
    typeof files !== "number"
  ) {
    return null;
  }
  const candidate = { gherkin, browser, files };
  return validatePanelLayout(candidate) ? candidate : null;
}

/** Extracts the vertical workspace/dock layout from a react-resizable-panels map. */
export function dockLayoutFromGroupLayout(
  layout: Layout,
): WorkspaceDockLayout | null {
  const main = layout.main;
  const dock = layout.dock;
  if (typeof main !== "number" || typeof dock !== "number") {
    return null;
  }
  const candidate = { main, dock };
  return validateDockLayout(candidate) ? candidate : null;
}

function readDockTab(value: unknown): WorkspaceDockTab {
  return value === "output" || value === "logs" || value === "screenshots"
    ? value
    : "locator";
}

function parseStoredMap(raw: string): StoredMap {
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }
    return parsed as StoredMap;
  } catch {
    return {};
  }
}

function readEntry(entry: unknown): WorkspaceLayoutState | null {
  if (!entry || typeof entry !== "object") {
    return null;
  }
  const e = entry as Partial<StoredEntry>;
  if (e.version !== 1 || !e.layout) {
    return null;
  }
  if (!validatePanelLayout(e.layout)) {
    return null;
  }
  const dockLayout =
    e.dockLayout && validateDockLayout(e.dockLayout)
      ? e.dockLayout
      : { ...DEFAULT_DOCK_LAYOUT };
  return {
    version: 1,
    layout: e.layout,
    dockLayout,
    gherkinCollapsed: Boolean(e.gherkinCollapsed),
    filesCollapsed: Boolean(e.filesCollapsed),
    dockExpanded: Boolean(e.dockExpanded),
    dockActiveTab: readDockTab(e.dockActiveTab),
    lastUsed:
      typeof e.lastUsed === "number" && Number.isFinite(e.lastUsed)
        ? e.lastUsed
        : 0,
  };
}

function pruneMap(map: StoredMap): StoredMap {
  const keys = Object.keys(map);
  if (keys.length <= WORKSPACE_LAYOUTS_MAX_ENTRIES) {
    return map;
  }
  const sorted = keys.sort(
    (a, b) => (map[b]?.lastUsed ?? 0) - (map[a]?.lastUsed ?? 0),
  );
  const keep = new Set(sorted.slice(0, WORKSPACE_LAYOUTS_MAX_ENTRIES));
  const pruned: StoredMap = {};
  for (const key of keep) {
    const entry = map[key];
    if (entry) {
      pruned[key] = entry;
    }
  }
  return pruned;
}

/**
 * Loads persisted layout state for a project, or `null` if missing or invalid.
 */
export function loadWorkspaceLayout(
  projectRoot: string,
  backend: LayoutStorageBackend | null = defaultBackend(),
): Omit<WorkspaceLayoutState, "lastUsed"> | null {
  if (!backend) {
    return null;
  }
  const key = normalizeProjectKey(projectRoot);
  let raw: string | null;
  try {
    raw = backend.getItem(WORKSPACE_LAYOUTS_STORAGE_KEY);
  } catch {
    return null;
  }
  if (!raw) {
    return null;
  }
  const entry = readEntry(parseStoredMap(raw)[key]);
  if (!entry) {
    return null;
  }
  return {
    version: 1,
    layout: entry.layout,
    dockLayout: entry.dockLayout,
    gherkinCollapsed: entry.gherkinCollapsed,
    filesCollapsed: entry.filesCollapsed,
    dockExpanded: entry.dockExpanded,
    dockActiveTab: entry.dockActiveTab,
  };
}

/**
 * Persists layout state for a project (LRU-capped map in a single storage entry).
 */
export function saveWorkspaceLayout(
  projectRoot: string,
  state: Partial<Omit<WorkspaceLayoutState, "lastUsed" | "version">>,
  backend: LayoutStorageBackend | null = defaultBackend(),
): void {
  if (!backend) {
    return;
  }
  if (state.layout && !validatePanelLayout(state.layout)) {
    return;
  }
  if (state.dockLayout && !validateDockLayout(state.dockLayout)) {
    return;
  }
  const key = normalizeProjectKey(projectRoot);
  let map: StoredMap = {};
  try {
    const raw = backend.getItem(WORKSPACE_LAYOUTS_STORAGE_KEY);
    if (raw) {
      map = parseStoredMap(raw);
    }
  } catch {
    return;
  }
  const existing = readEntry(map[key]);
  const base: Omit<WorkspaceLayoutState, "version" | "lastUsed"> = existing
    ? {
        layout: existing.layout,
        dockLayout: existing.dockLayout,
        gherkinCollapsed: existing.gherkinCollapsed,
        filesCollapsed: existing.filesCollapsed,
        dockExpanded: existing.dockExpanded,
        dockActiveTab: existing.dockActiveTab,
      }
    : {
        layout: { ...DEFAULT_PANEL_LAYOUT },
        dockLayout: { ...DEFAULT_DOCK_LAYOUT },
        gherkinCollapsed: false,
        filesCollapsed: false,
        dockExpanded: false,
        dockActiveTab: "locator",
      };
  map[key] = {
    version: 1,
    ...base,
    ...state,
    lastUsed: Date.now(),
  };
  map = pruneMap(map);
  try {
    backend.setItem(WORKSPACE_LAYOUTS_STORAGE_KEY, JSON.stringify(map));
  } catch {
    // quota exceeded or storage disabled
  }
}

/** Layout map for `Group` `defaultLayout` (saved or defaults). */
export function defaultLayoutForProject(projectRoot: string): Layout {
  const saved = loadWorkspaceLayout(projectRoot);
  if (saved) {
    return { ...saved.layout };
  }
  return { ...DEFAULT_PANEL_LAYOUT };
}

/** Vertical layout map for the workspace/dock `Group` `defaultLayout`. */
export function defaultDockLayoutForProject(projectRoot: string): Layout {
  const saved = loadWorkspaceLayout(projectRoot);
  if (saved) {
    return { ...saved.dockLayout };
  }
  return { ...DEFAULT_DOCK_LAYOUT };
}
