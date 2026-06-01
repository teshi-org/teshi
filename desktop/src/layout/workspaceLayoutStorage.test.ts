import { describe, expect, it } from "vitest";
import {
  DEFAULT_DOCK_LAYOUT,
  DEFAULT_PANEL_LAYOUT,
  dockLayoutFromGroupLayout,
  loadWorkspaceLayout,
  normalizeProjectKey,
  panelLayoutFromGroupLayout,
  saveWorkspaceLayout,
  validateDockLayout,
  validatePanelLayout,
  WORKSPACE_LAYOUTS_MAX_ENTRIES,
  WORKSPACE_LAYOUTS_STORAGE_KEY,
} from "./workspaceLayoutStorage";
import type { LayoutStorageBackend } from "./workspaceLayoutStorage";

function createMemoryBackend(): LayoutStorageBackend {
  const store = new Map<string, string>();
  return {
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => {
      store.set(key, value);
    },
  };
}

describe("normalizeProjectKey", () => {
  it("lowercases Windows drive paths", () => {
    expect(normalizeProjectKey("C:\\Dev\\MyProject")).toBe("c:/dev/myproject");
  });

  it("preserves POSIX path casing", () => {
    expect(normalizeProjectKey("/home/user/Project")).toBe("/home/user/Project");
  });
});

describe("validatePanelLayout", () => {
  it("accepts layouts summing to 100", () => {
    expect(validatePanelLayout(DEFAULT_PANEL_LAYOUT)).toBe(true);
  });

  it("rejects invalid sums and out-of-range values", () => {
    expect(validatePanelLayout({ gherkin: 50, browser: 50, files: 10 })).toBe(
      false,
    );
    expect(validatePanelLayout({ gherkin: 0, browser: 50, files: 50 })).toBe(
      false,
    );
  });
});

describe("validateDockLayout", () => {
  it("accepts layouts summing to 100", () => {
    expect(validateDockLayout(DEFAULT_DOCK_LAYOUT)).toBe(true);
  });

  it("rejects invalid sums and out-of-range values", () => {
    expect(validateDockLayout({ main: 20, dock: 20 })).toBe(false);
    expect(validateDockLayout({ main: 0, dock: 100 })).toBe(false);
  });
});

describe("panelLayoutFromGroupLayout", () => {
  it("extracts panel ids from group layout", () => {
    expect(
      panelLayoutFromGroupLayout({ gherkin: 30, browser: 45, files: 25 }),
    ).toEqual(DEFAULT_PANEL_LAYOUT);
  });
});

describe("dockLayoutFromGroupLayout", () => {
  it("extracts dock ids from group layout", () => {
    expect(dockLayoutFromGroupLayout({ main: 72, dock: 28 })).toEqual({
      main: 72,
      dock: 28,
    });
  });
});

describe("load and save", () => {
  it("round-trips layout and collapse flags", () => {
    const backend = createMemoryBackend();
    const root = "C:\\Projects\\bdd-app";
    saveWorkspaceLayout(
      root,
      {
        layout: { gherkin: 25, browser: 50, files: 25 },
        dockLayout: { main: 70, dock: 30 },
        gherkinCollapsed: true,
        filesCollapsed: false,
        dockExpanded: true,
        dockActiveTab: "logs",
      },
      backend,
    );
    const loaded = loadWorkspaceLayout(root, backend);
    expect(loaded).toEqual({
      version: 1,
      layout: { gherkin: 25, browser: 50, files: 25 },
      dockLayout: { main: 70, dock: 30 },
      gherkinCollapsed: true,
      filesCollapsed: false,
      dockExpanded: true,
      dockActiveTab: "logs",
    });
  });

  it("loads old entries with default dock fields", () => {
    const backend = createMemoryBackend();
    const root = "/tmp/proj";
    const key = normalizeProjectKey(root);
    backend.setItem(
      WORKSPACE_LAYOUTS_STORAGE_KEY,
      JSON.stringify({
        [key]: {
          version: 1,
          layout: DEFAULT_PANEL_LAYOUT,
          gherkinCollapsed: false,
          filesCollapsed: true,
          lastUsed: 1,
        },
      }),
    );

    expect(loadWorkspaceLayout(root, backend)).toEqual({
      version: 1,
      layout: DEFAULT_PANEL_LAYOUT,
      dockLayout: DEFAULT_DOCK_LAYOUT,
      gherkinCollapsed: false,
      filesCollapsed: true,
      dockExpanded: false,
      dockActiveTab: "locator",
    });
  });

  it("preserves horizontal layout when only dock state changes", () => {
    const backend = createMemoryBackend();
    const root = "/tmp/proj";
    saveWorkspaceLayout(
      root,
      {
        layout: { gherkin: 20, browser: 55, files: 25 },
        gherkinCollapsed: true,
        filesCollapsed: false,
      },
      backend,
    );
    saveWorkspaceLayout(
      root,
      {
        dockLayout: { main: 80, dock: 20 },
        dockExpanded: true,
        dockActiveTab: "output",
      },
      backend,
    );

    expect(loadWorkspaceLayout(root, backend)).toEqual({
      version: 1,
      layout: { gherkin: 20, browser: 55, files: 25 },
      dockLayout: { main: 80, dock: 20 },
      gherkinCollapsed: true,
      filesCollapsed: false,
      dockExpanded: true,
      dockActiveTab: "output",
    });
  });

  it("returns null for invalid stored layout", () => {
    const backend = createMemoryBackend();
    const key = normalizeProjectKey("/tmp/proj");
    backend.setItem(
      WORKSPACE_LAYOUTS_STORAGE_KEY,
      JSON.stringify({
        [key]: {
          version: 1,
          layout: { gherkin: 10, browser: 10, files: 10 },
          gherkinCollapsed: false,
          filesCollapsed: false,
          lastUsed: 1,
        },
      }),
    );
    expect(loadWorkspaceLayout("/tmp/proj", backend)).toBeNull();
  });

  it("prunes entries beyond the LRU cap", () => {
    const backend = createMemoryBackend();
    for (let i = 0; i < WORKSPACE_LAYOUTS_MAX_ENTRIES + 5; i++) {
      saveWorkspaceLayout(
        `/projects/p${i}`,
        {
          layout: DEFAULT_PANEL_LAYOUT,
          gherkinCollapsed: false,
          filesCollapsed: false,
        },
        backend,
      );
    }
    const raw = backend.getItem(WORKSPACE_LAYOUTS_STORAGE_KEY);
    expect(raw).toBeTruthy();
    const map = JSON.parse(raw!) as Record<string, unknown>;
    expect(Object.keys(map).length).toBe(WORKSPACE_LAYOUTS_MAX_ENTRIES);
  });
});
