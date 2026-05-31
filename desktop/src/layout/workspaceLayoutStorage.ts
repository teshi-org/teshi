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

export type WorkspacePanelLayout = {
  gherkin: number;
  browser: number;
  files: number;
};

export type WorkspaceLayoutState = {
  version: 1;
  layout: WorkspacePanelLayout;
  gherkinCollapsed: boolean;
  filesCollapsed: boolean;
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
  return {
    version: 1,
    layout: e.layout,
    gherkinCollapsed: Boolean(e.gherkinCollapsed),
    filesCollapsed: Boolean(e.filesCollapsed),
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
    gherkinCollapsed: entry.gherkinCollapsed,
    filesCollapsed: entry.filesCollapsed,
  };
}

/**
 * Persists layout state for a project (LRU-capped map in a single storage entry).
 */
export function saveWorkspaceLayout(
  projectRoot: string,
  state: Omit<WorkspaceLayoutState, "lastUsed" | "version"> & {
    layout: WorkspacePanelLayout;
  },
  backend: LayoutStorageBackend | null = defaultBackend(),
): void {
  if (!backend) {
    return;
  }
  if (!validatePanelLayout(state.layout)) {
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
  map[key] = {
    version: 1,
    layout: state.layout,
    gherkinCollapsed: state.gherkinCollapsed,
    filesCollapsed: state.filesCollapsed,
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
