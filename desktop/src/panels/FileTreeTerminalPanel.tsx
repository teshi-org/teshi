import React, { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { sanitizePathForTestId } from "../layout/testIdPath";
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

/** Minimum interval (ms) between automatic shell spawns to prevent rapid respawn loops. */
const MIN_SPAWN_INTERVAL_MS = 3_000;
/** If the shell exits this many times within the window, auto-respawn stops. */
const MAX_EXIT_COUNT = 3;
/** Time window (ms) for counting shell exits. */
const EXIT_WINDOW_MS = 10_000;
/** Backoff delays (ms) between respawns after a loop-detected or exit. */
const BACKOFF_DELAYS: number[] = [1_000, 2_000, 4_000, 8_000, 16_000, 30_000];

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
  shellOpsRef: { current: Promise<void> },
  lastSpawnTimeRef: { current: number },
  backoffIndexRef: { current: number },
  force = false,
): Promise<void> {
  // Debounce: skip if the last spawn was too recent (unless forced).
  if (!force && shellSpawnedRef.current) {
    const elapsed = Date.now() - lastSpawnTimeRef.current;
    if (elapsed < MIN_SPAWN_INTERVAL_MS) {
      console.debug(
        `[terminal] ensureShellSpawned skipped (${elapsed}ms since last, min ${MIN_SPAWN_INTERVAL_MS}ms)`,
      );
      return;
    }
  }

  const run = shellOpsRef.current.then(async () => {
    fit.fit();
    const { cols, rows } = normalizeTerminalSize(term.cols, term.rows);
    if (!shellSpawnedRef.current || force) {
      lastSpawnTimeRef.current = Date.now();
      console.debug(`[terminal] spawnTerminal called (cols=${cols}, rows=${rows}, force=${force})`);
      await getRuntime().spawnTerminal(cols, rows);
      shellSpawnedRef.current = true;
      backoffIndexRef.current = 0;
      console.debug("[terminal] spawnTerminal completed");
    } else {
      await syncTerminalSize(fit, term);
    }
  });
  shellOpsRef.current = run.then(
    () => {},
    () => {},
  );
  await run;
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
  const shellOpsRef = useRef<Promise<void>>(Promise.resolve());
  const projectRootRef = useRef(projectRoot);
  const [terminalReady, setTerminalReady] = useState(false);
  // Respawn debounce state
  const lastSpawnTimeRef = useRef(0);
  const lastExitTimeRef = useRef(0);
  const exitCountRef = useRef(0);
  const exitWindowStartRef = useRef(0);
  const backoffIndexRef = useRef(0);
  const autoRespawnPausedRef = useRef(false);

  const loadChildren = useCallback(async (path: string) => {
    return getRuntime().listDir(path);
  }, []);

  useEffect(() => {
    if (tab !== "files") return;
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
  }, [projectRoot, tab, loadChildren]);

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
      shellOpsRef.current = Promise.resolve();
      setTerminalReady(false);
      void getRuntime().stopTerminal().catch((err) => {
        console.error("stop_terminal failed", err);
      });
    };
  }, [projectRoot]);

  // Create xterm once the Terminal tab is first opened (FitAddon needs a visible host).
  // Re-create when projectRoot changes (the previous xterm was disposed by the
  // project-root cleanup effect), even if the Terminal tab hasn't been toggled.
  useEffect(() => {
    if (tab !== "terminal") return;
    if (xtermRef.current && projectRootRef.current === projectRoot) return;
    projectRootRef.current = projectRoot;

    // Dispose any stale xterm before creating a new one (race: the project-root
    // cleanup may not have run before the tab change).
    if (xtermRef.current) {
      xtermRef.current.dispose();
      xtermRef.current = null;
    }

    let disposed = false;

    void (async () => {
      const { Terminal } = await import("@xterm/xterm");
      const { FitAddon } = await import("@xterm/addon-fit");
      await import("@xterm/xterm/css/xterm.css");
      if (disposed || xtermRef.current) return;

      await waitForLayout();
      if (disposed || !terminalRef.current) return;

      const term = new Terminal({
        theme: TERMINAL_THEME,
        fontFamily: "Consolas, 'Cascadia Mono', monospace",
        cursorBlink: true,
      });
      const fit = new FitAddon();
      term.loadAddon(fit);
      term.open(terminalRef.current);
      fit.fit();
      xtermRef.current = term;
      fitRef.current = fit;

      term.onData((data) => {
        void (async () => {
          try {
            const fitNow = fitRef.current;
            const termNow = xtermRef.current;
            if (!shellSpawnedRef.current && fitNow && termNow) {
              await ensureShellSpawned(fitNow, termNow, shellSpawnedRef, shellOpsRef, lastSpawnTimeRef, backoffIndexRef);
            }
            await getRuntime().writeTerminal(data);
          } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            toast.error(message);
            xtermRef.current?.writeln(`\r\n\x1b[33m${message}\x1b[0m`);
          }
        })();
      });
      console.debug("[terminal] xterm created, onData registered");

      if (disposed) {
        term.dispose();
        xtermRef.current = null;
        fitRef.current = null;
        return;
      }

      setTerminalReady(true);
    })();

    return () => {
      disposed = true;
    };
  }, [tab, projectRoot]);

  // PTY → xterm bridge. Re-bind whenever the Terminal tab is shown (exclusive runtime
  // handler prevents duplicate listeners across HMR remounts).
  useEffect(() => {
    if (tab !== "terminal" || !terminalReady) return;

    const term = xtermRef.current;
    if (!term) return;

    let cancelled = false;
    let unlistenOutput: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;
    let unlistenLoop: (() => void) | null = null;

    void (async () => {
      unlistenOutput = await getRuntime().onEvent<string>("terminal-output", (payload) => {
        if (cancelled || xtermRef.current !== term) return;
        term.write(decodeTerminalChunk(payload));
      });
      console.debug("[terminal] terminal-output event listener bound");

      unlistenExit = await getRuntime().onEvent("terminal-exit", () => {
        if (cancelled || xtermRef.current !== term) return;
        shellSpawnedRef.current = false;
        console.debug("[terminal] terminal-exit event received");

        // Track exit frequency for debounce.
        const now = Date.now();
        if (now - exitWindowStartRef.current > EXIT_WINDOW_MS) {
          exitWindowStartRef.current = now;
          exitCountRef.current = 0;
        }
        exitCountRef.current++;
        lastExitTimeRef.current = now;

        if (exitCountRef.current >= MAX_EXIT_COUNT) {
          autoRespawnPausedRef.current = true;
          term.writeln("\r\n\x1b[33mShell exited repeatedly. Auto-restart paused. Click 'Restart shell' to retry.\x1b[0m");
          console.debug("[terminal] auto-respawn paused (exited ${exitCountRef.current}x in window)");
        } else {
          term.writeln("\r\n\x1b[33mShell exited.\x1b[0m");
          // Schedule auto-respawn with backoff.
          const delay = BACKOFF_DELAYS[Math.min(backoffIndexRef.current, BACKOFF_DELAYS.length - 1)];
          backoffIndexRef.current++;
          console.debug(`[terminal] scheduling auto-respawn in ${delay}ms (backoff index ${backoffIndexRef.current - 1})`);
          setTimeout(() => {
            if (cancelled || autoRespawnPausedRef.current) return;
            const fitNow = fitRef.current;
            const termNow = xtermRef.current;
            if (fitNow && termNow) {
              void ensureShellSpawned(fitNow, termNow, shellSpawnedRef, shellOpsRef, lastSpawnTimeRef, backoffIndexRef, true);
            }
          }, delay);
        }
      });

      unlistenLoop = await getRuntime().onEvent("terminal-loop-detected", () => {
        if (cancelled || xtermRef.current !== term) return;
        shellSpawnedRef.current = false;
        console.debug("[terminal] terminal-loop-detected event received");
        term.writeln("\r\n\x1b[33mTerminal output loop detected, restarting...\x1b[0m");

        // Auto-respawn with backoff.
        const delay = BACKOFF_DELAYS[Math.min(backoffIndexRef.current, BACKOFF_DELAYS.length - 1)];
        backoffIndexRef.current++;
        setTimeout(() => {
          if (cancelled) return;
          const fitNow = fitRef.current;
          const termNow = xtermRef.current;
          if (fitNow && termNow) {
            void ensureShellSpawned(fitNow, termNow, shellSpawnedRef, shellOpsRef, lastSpawnTimeRef, backoffIndexRef, true);
          }
        }, delay);
      });

      if (cancelled) {
        unlistenOutput?.();
        unlistenExit?.();
        unlistenLoop?.();
      }
    })();

    return () => {
      cancelled = true;
      unlistenOutput?.();
      unlistenExit?.();
      unlistenLoop?.();
    };
  }, [tab, terminalReady, projectRoot]);

  // Spawn/refit when the Terminal tab is visible; PTY listeners stay on the xterm instance.
  useEffect(() => {
    if (tab !== "terminal" || !terminalReady) return;

    const term = xtermRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;

    let cancelled = false;
    let resizeObserver: ResizeObserver | null = null;

    if (terminalRef.current) {
      resizeObserver = new ResizeObserver(() => {
        const fitNow = fitRef.current;
        const termNow = xtermRef.current;
        if (!fitNow || !termNow) return;
        void ensureShellSpawned(fitNow, termNow, shellSpawnedRef, shellOpsRef, lastSpawnTimeRef, backoffIndexRef).catch((err) => {
          console.error("terminal resize/spawn failed", err);
        });
      });
      resizeObserver.observe(terminalRef.current);
    }

    void (async () => {
      try {
        await waitForLayout();
        if (cancelled) return;
        console.debug("[terminal] initial spawn/refit effect running");
        await ensureShellSpawned(fit, term, shellSpawnedRef, shellOpsRef, lastSpawnTimeRef, backoffIndexRef);
        await syncTerminalSize(fit, term);
        term.focus();
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        toast.error(`Terminal failed: ${message}`);
        term.writeln(`\r\n\x1b[31mFailed to start shell: ${message}\x1b[0m`);
      }
    })();

    return () => {
      cancelled = true;
      resizeObserver?.disconnect();
    };
  }, [tab, terminalReady, projectRoot]);

  // Refit xterm when the panel becomes visible again after browser focus or collapse.
  useEffect(() => {
    if (layoutHidden || layoutCollapsed || tab !== "terminal" || !terminalReady) return;
    const fit = fitRef.current;
    const term = xtermRef.current;
    if (!fit || !term) return;

    requestAnimationFrame(() => {
      console.debug("[terminal] refit effect triggered (layout changed)");
      void ensureShellSpawned(fit, term, shellSpawnedRef, shellOpsRef, lastSpawnTimeRef, backoffIndexRef).catch((err) => {
        console.error("terminal refit/spawn failed", err);
      });
    });
  }, [layoutHidden, layoutCollapsed, tab, terminalReady]);

  // Focus xterm when the user selects the Terminal tab (keystrokes otherwise go nowhere).
  useEffect(() => {
    if (tab !== "terminal" || !terminalReady) return;
    const term = xtermRef.current;
    if (!term) return;
    requestAnimationFrame(() => {
      term.focus();
    });
  }, [tab, terminalReady]);

  const focusTerminal = useCallback(() => {
    xtermRef.current?.focus();
  }, []);

  const restartShell = useCallback(async () => {
    const fit = fitRef.current;
    const term = xtermRef.current;
    if (!fit || !term) {
      toast.error("Terminal not ready yet");
      return;
    }
    try {
      shellOpsRef.current = Promise.resolve();
      await getRuntime().stopTerminal();
      shellSpawnedRef.current = false;
      // Reset debounce state for manual restart.
      exitCountRef.current = 0;
      exitWindowStartRef.current = 0;
      autoRespawnPausedRef.current = false;
      backoffIndexRef.current = 0;
      term.reset();
      await waitForLayout();
      fit.fit();
      console.debug("[terminal] manual restart shell");
      await ensureShellSpawned(fit, term, shellSpawnedRef, shellOpsRef, lastSpawnTimeRef, backoffIndexRef, true);
      term.focus();
      toast.success("Shell restarted");
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      toast.error(`Restart failed: ${message}`);
      term.writeln(`\r\n\x1b[31m${message}\x1b[0m`);
    }
  }, []);

  return (
    <section
      className={`panel side-panel${layoutHidden ? " panel--layout-hidden" : ""}`}
    >
      <header className="panel-header tabs">
        <button
          type="button"
          className={tab === "files" ? "active" : ""}
          data-testid="FileTreeTab"
          onClick={() => onTabChange("files")}
        >
          Files
        </button>
        <button
          type="button"
          className={tab === "terminal" ? "active" : ""}
          data-testid="TerminalTab"
          onClick={() => onTabChange("terminal")}
        >
          Terminal
        </button>
        {tab === "terminal" && (
          <button
            type="button"
            className="terminal-restart-btn"
            title="Restart shell if input stops working"
            onClick={() => void restartShell()}
          >
            Restart shell
          </button>
        )}
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
            {renderTree(rootNode, onClickEntry, projectRoot)}
          </ul>
        )}
        <div
          ref={terminalRef}
          className="terminal-host"
          hidden={tab !== "terminal"}
          onMouseDown={focusTerminal}
          role="presentation"
        />
      </div>
    </section>
  );
}

/** Stable `data-testid` suffix for file-tree nodes (relative path from project root). */
function fileTreeTestId(relativePath: string): string {
  return sanitizePathForTestId(relativePath);
}

function renderTree(
  node: TreeNode,
  onClick: (node: TreeNode) => void,
  projectRoot: string,
): React.ReactNode {
  const relativePath = node.path.startsWith(projectRoot)
    ? node.path.slice(projectRoot.length).replace(/^[/\\]+/, "")
    : node.name;
  const testIdSuffix = fileTreeTestId(relativePath || node.name);
  return (
    <li key={node.path}>
      <button
        type="button"
        className={`tree-item ${node.is_feature ? "feature" : ""}`}
        data-testid={`FileTreeNode-${testIdSuffix}`}
        onClick={() => onClick(node)}
      >
        {node.is_dir ? (node.expanded ? "▾ " : "▸ ") : ""}
        {node.name}
        {node.loading ? " …" : ""}
      </button>
      {node.is_dir && node.expanded && node.children && (
        <ul>{node.children.map((child) => renderTree(child, onClick, projectRoot))}</ul>
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
