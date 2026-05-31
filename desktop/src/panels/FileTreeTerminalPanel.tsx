import React, { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { getRuntime } from "../platform";
import type { DirEntry } from "../types";
import { PanelCollapseButton } from "./PanelCollapseButton";

interface TreeNode extends DirEntry {
  expanded?: boolean;
  children?: TreeNode[];
  loading?: boolean;
}

interface Props {
  projectRoot: string;
  tab: "files" | "terminal";
  onTabChange: (tab: "files" | "terminal") => void;
  onOpenFeature: (path: string) => void;
  /** Hide the panel during browser focus mode without unmounting. */
  layoutHidden?: boolean;
  /** Panel width collapsed to zero via the resize handle or chevron. */
  layoutCollapsed?: boolean;
  showCollapseButton?: boolean;
  onToggleCollapse?: () => void;
}

/** VS Code–style dark palette so ANSI SGR colors render correctly in xterm.js. */
const TERMINAL_THEME = {
  background: "#1e1e1e",
  foreground: "#cccccc",
  cursor: "#ffffff",
  // Avoid pure #000000: ConPTY maps it to "default" and strips TUI background colors.
  black: "#0c0c0c",
  red: "#cd3131",
  green: "#0dbc79",
  yellow: "#e5e510",
  blue: "#2472c8",
  magenta: "#bc3fbc",
  cyan: "#11a8cd",
  white: "#e5e5e5",
  brightBlack: "#666666",
  brightRed: "#f14c4c",
  brightGreen: "#23d18b",
  brightYellow: "#f5f543",
  brightBlue: "#3b8eea",
  brightMagenta: "#d670d6",
  brightCyan: "#29b8db",
  brightWhite: "#e5e5e5",
} as const;

const TERMINAL_MIN_COLS = 2;
const TERMINAL_MIN_ROWS = 2;

/** Decode base64 PTY chunks from Rust without corrupting ANSI/truecolor bytes. */
function decodeTerminalChunk(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

function normalizeTerminalSize(cols: number, rows: number): { cols: number; rows: number } {
  return {
    cols: Math.max(cols, TERMINAL_MIN_COLS),
    rows: Math.max(rows, TERMINAL_MIN_ROWS),
  };
}

async function syncTerminalSize(
  fit: import("@xterm/addon-fit").FitAddon,
  term: import("@xterm/xterm").Terminal,
): Promise<void> {
  fit.fit();
  const { cols, rows } = normalizeTerminalSize(term.cols, term.rows);
  await getRuntime().resizeTerminal(cols, rows);
}

async function ensureShellSpawned(
  fit: import("@xterm/addon-fit").FitAddon,
  term: import("@xterm/xterm").Terminal,
  shellSpawnedRef: { current: boolean },
): Promise<void> {
  fit.fit();
  const { cols, rows } = normalizeTerminalSize(term.cols, term.rows);
  if (!shellSpawnedRef.current) {
    await getRuntime().spawnTerminal(cols, rows);
    shellSpawnedRef.current = true;
  } else {
    await syncTerminalSize(fit, term);
  }
}

function waitForLayout(): Promise<void> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => resolve());
  });
}

export function FileTreeTerminalPanel({
  projectRoot,
  tab,
  onTabChange,
  onOpenFeature,
  layoutHidden = false,
  layoutCollapsed = false,
  showCollapseButton = false,
  onToggleCollapse,
}: Props) {
  const [rootNode, setRootNode] = useState<TreeNode | null>(null);
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<import("@xterm/xterm").Terminal | null>(null);
  const fitRef = useRef<import("@xterm/addon-fit").FitAddon | null>(null);
  const shellSpawnedRef = useRef(false);
  const [terminalReady, setTerminalReady] = useState(false);

  const loadChildren = useCallback(async (path: string) => {
    return getRuntime().listDir(path);
  }, []);

  useEffect(() => {
    loadChildren(projectRoot).then((entries) => {
      setRootNode({
        name: projectRoot.split(/[/\\]/).pop() ?? projectRoot,
        path: projectRoot,
        is_dir: true,
        is_feature: false,
        expanded: true,
        children: entries.map((e) => ({ ...e })),
      });
    });
  }, [projectRoot, loadChildren]);

  const toggleDir = async (node: TreeNode) => {
    if (!node.is_dir) return;
    if (node.expanded) {
      setRootNode((prev) => updateNode(prev, node.path, { expanded: false }));
      return;
    }
    setRootNode((prev) =>
      updateNode(prev, node.path, { expanded: true, loading: true }),
    );
    const children = await loadChildren(node.path);
    setRootNode((prev) =>
      updateNode(prev, node.path, {
        expanded: true,
        loading: false,
        children: children.map((c) => ({ ...c })),
      }),
    );
  };

  const onClickEntry = (node: TreeNode) => {
    if (node.is_dir) {
      void toggleDir(node);
      return;
    }
    if (node.is_feature) {
      onOpenFeature(node.path);
      return;
    }
    toast.info(`Selected: ${node.name} (not a feature file)`);
  };

  // Tear down the PTY and xterm when the project changes or the panel unmounts.
  useEffect(() => {
    return () => {
      xtermRef.current?.dispose();
      xtermRef.current = null;
      fitRef.current = null;
      shellSpawnedRef.current = false;
      setTerminalReady(false);
      void getRuntime().stopTerminal().catch((err) => {
        console.error("stop_terminal failed", err);
      });
    };
  }, [projectRoot]);

  // Lazy-init xterm only after the Terminal tab is visible (correct FitAddon measure).
  useEffect(() => {
    if (tab !== "terminal") return;

    let disposed = false;
    let unlistenOutput: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;
    let resizeObserver: ResizeObserver | null = null;
    let term: import("@xterm/xterm").Terminal | null = null;
    let fit: import("@xterm/addon-fit").FitAddon | null = null;

    void (async () => {
      if (xtermRef.current && fitRef.current) {
        await waitForLayout();
        if (disposed) return;
        void syncTerminalSize(fitRef.current, xtermRef.current);
        return;
      }

      const { Terminal } = await import("@xterm/xterm");
      const { FitAddon } = await import("@xterm/addon-fit");
      await import("@xterm/xterm/css/xterm.css");
      if (disposed) return;

      await waitForLayout();
      if (disposed || !terminalRef.current) return;

      term = new Terminal({
        theme: TERMINAL_THEME,
        fontFamily: "Consolas, 'Cascadia Mono', monospace",
        cursorBlink: true,
        windowsMode: true,
      });
      fit = new FitAddon();
      term.loadAddon(fit);
      term.open(terminalRef.current);
      fit.fit();
      xtermRef.current = term;
      fitRef.current = fit;
      setTerminalReady(true);

      unlistenOutput = await getRuntime().onEvent<string>("terminal-output", (payload) => {
        term?.write(decodeTerminalChunk(payload));
      });
      unlistenExit = await getRuntime().onEvent("terminal-exit", () => {
        shellSpawnedRef.current = false;
        term?.writeln("\r\n\x1b[33mShell exited.\x1b[0m");
      });
      if (disposed) return;

      term.onData((data) => {
        void getRuntime().writeTerminal(data);
      });

      if (disposed || !terminalRef.current || !fit || !term) return;

      resizeObserver = new ResizeObserver(() => {
        if (!fit || !term) return;
        void ensureShellSpawned(fit, term, shellSpawnedRef).catch((err) => {
          console.error("terminal resize/spawn failed", err);
        });
      });
      resizeObserver.observe(terminalRef.current);
    })();

    return () => {
      disposed = true;
      resizeObserver?.disconnect();
      unlistenOutput?.();
      unlistenExit?.();
    };
  }, [tab, projectRoot]);

  // Fit, then spawn the PTY at matching cols/rows so the first prompt aligns.
  useEffect(() => {
    if (tab !== "terminal" || !terminalReady) return;
    const fit = fitRef.current;
    const term = xtermRef.current;
    if (!fit || !term) return;

    void (async () => {
      try {
        await waitForLayout();
        await ensureShellSpawned(fit, term, shellSpawnedRef);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        toast.error(`Terminal failed: ${message}`);
        term.writeln(`\r\n\x1b[31mFailed to start shell: ${message}\x1b[0m`);
      }
    })();
  }, [tab, terminalReady, projectRoot]);

  // Refit xterm when the panel becomes visible again after browser focus or collapse.
  useEffect(() => {
    if (layoutHidden || layoutCollapsed || tab !== "terminal" || !terminalReady) return;
    const fit = fitRef.current;
    const term = xtermRef.current;
    if (!fit || !term) return;

    requestAnimationFrame(() => {
      void ensureShellSpawned(fit, term, shellSpawnedRef).catch((err) => {
        console.error("terminal refit/spawn failed", err);
      });
    });
  }, [layoutHidden, layoutCollapsed, tab, terminalReady]);

  return (
    <section
      className={`panel side-panel${layoutHidden ? " panel--layout-hidden" : ""}`}
    >
      <header className="panel-header tabs">
        <button
          type="button"
          className={tab === "files" ? "active" : ""}
          onClick={() => onTabChange("files")}
        >
          Files
        </button>
        <button
          type="button"
          className={tab === "terminal" ? "active" : ""}
          onClick={() => onTabChange("terminal")}
        >
          Terminal
        </button>
        {showCollapseButton && onToggleCollapse && (
          <PanelCollapseButton
            side="right"
            collapsed={false}
            panelLabel="Files"
            onToggle={onToggleCollapse}
          />
        )}
      </header>
      <div className={`panel-body${tab === "terminal" ? " panel-body--terminal" : ""}`}>
        {rootNode && (
          <ul className="file-tree" hidden={tab !== "files"}>
            {renderTree(rootNode, onClickEntry)}
          </ul>
        )}
        <div
          ref={terminalRef}
          className="terminal-host"
          hidden={tab !== "terminal"}
        />
      </div>
    </section>
  );
}

function renderTree(
  node: TreeNode,
  onClick: (node: TreeNode) => void,
): React.ReactNode {
  return (
    <li key={node.path}>
      <button
        type="button"
        className={`tree-item ${node.is_feature ? "feature" : ""}`}
        onClick={() => onClick(node)}
      >
        {node.is_dir ? (node.expanded ? "▾ " : "▸ ") : ""}
        {node.name}
        {node.loading ? " …" : ""}
      </button>
      {node.is_dir && node.expanded && node.children && (
        <ul>{node.children.map((child) => renderTree(child, onClick))}</ul>
      )}
    </li>
  );
}

function updateNode(
  root: TreeNode | null,
  path: string,
  patch: Partial<TreeNode>,
): TreeNode | null {
  if (!root) return null;
  if (root.path === path) {
    return { ...root, ...patch };
  }
  if (root.children) {
    return {
      ...root,
      children: root.children.map((c) => updateNode(c, path, patch) ?? c),
    };
  }
  return root;
}
