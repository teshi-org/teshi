import React, { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import type { DirEntry } from "../types";

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
}

async function syncTerminalSize(
  fit: import("@xterm/addon-fit").FitAddon,
  term: import("@xterm/xterm").Terminal,
): Promise<void> {
  fit.fit();
  const cols = term.cols;
  const rows = term.rows;
  if (cols > 0 && rows > 0) {
    await invoke("resize_terminal", { cols, rows });
  }
}

export function FileTreeTerminalPanel({
  projectRoot,
  tab,
  onTabChange,
  onOpenFeature,
}: Props) {
  const [rootNode, setRootNode] = useState<TreeNode | null>(null);
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<import("@xterm/xterm").Terminal | null>(null);

  const loadChildren = useCallback(async (path: string) => {
    return invoke<DirEntry[]>("list_dir", { path });
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

  useEffect(() => {
    if (tab !== "terminal" || xtermRef.current) return;

    let disposed = false;
    let unlistenOutput: (() => void) | null = null;
    let unlistenExit: (() => void) | null = null;
    let resizeObserver: ResizeObserver | null = null;
    let term: import("@xterm/xterm").Terminal | null = null;
    let fit: import("@xterm/addon-fit").FitAddon | null = null;

    void (async () => {
      const { Terminal } = await import("@xterm/xterm");
      const { FitAddon } = await import("@xterm/addon-fit");
      await import("@xterm/xterm/css/xterm.css");
      const { listen } = await import("@tauri-apps/api/event");
      if (disposed) return;

      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      });
      if (disposed || !terminalRef.current) return;

      term = new Terminal({
        theme: { background: "#1e1e1e", foreground: "#d4d4d4" },
        fontFamily: "Consolas, monospace",
      });
      fit = new FitAddon();
      term.loadAddon(fit);
      term.open(terminalRef.current);
      fit.fit();
      xtermRef.current = term;

      unlistenOutput = await listen<string>("terminal-output", (event) => {
        term?.write(event.payload);
      });
      unlistenExit = await listen("terminal-exit", () => {
        term?.writeln("\r\n\x1b[33mShell exited.\x1b[0m");
      });
      if (disposed) return;

      term.onData((data) => {
        void invoke("write_terminal", { data });
      });

      try {
        await invoke("spawn_terminal");
        await syncTerminalSize(fit, term);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        toast.error(`Terminal failed: ${message}`);
        term.writeln(`\r\n\x1b[31mFailed to start shell: ${message}\x1b[0m`);
      }

      if (disposed || !terminalRef.current || !fit || !term) return;

      resizeObserver = new ResizeObserver(() => {
        if (!fit || !term) return;
        void syncTerminalSize(fit, term);
      });
      resizeObserver.observe(terminalRef.current);
    })();

    return () => {
      disposed = true;
      resizeObserver?.disconnect();
      unlistenOutput?.();
      unlistenExit?.();
      term?.dispose();
      xtermRef.current = null;
      void invoke("stop_terminal").catch((err) => {
        console.error("stop_terminal failed", err);
      });
    };
  }, [tab]);

  return (
    <section className="panel side-panel">
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
      </header>
      <div className="panel-body">
        {tab === "files" && rootNode && (
          <ul className="file-tree">{renderTree(rootNode, onClickEntry)}</ul>
        )}
        {tab === "terminal" && <div ref={terminalRef} className="terminal-host" />}
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
