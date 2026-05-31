import { describe, expect, it } from "vitest";
import {
  DEFAULT_PANEL_LAYOUT,
  loadWorkspaceLayout,
  normalizeProjectKey,
  panelLayoutFromGroupLayout,
  saveWorkspaceLayout,
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

describe("panelLayoutFromGroupLayout", () => {
  it("extracts panel ids from group layout", () => {
    expect(
      panelLayoutFromGroupLayout({ gherkin: 30, browser: 45, files: 25 }),
    ).toEqual(DEFAULT_PANEL_LAYOUT);
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
        gherkinCollapsed: true,
        filesCollapsed: false,
      },
      backend,
    );
    const loaded = loadWorkspaceLayout(root, backend);
    expect(loaded).toEqual({
      version: 1,
      layout: { gherkin: 25, browser: 50, files: 25 },
      gherkinCollapsed: true,
      filesCollapsed: false,
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
