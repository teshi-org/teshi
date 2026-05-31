import { useCallback, useEffect, useRef, useState } from "react";
import { isTauriHost } from "../platform";

interface Props {
  projectRoot: string | null;
  recentProjects: string[];
  onOpenProject: () => void;
  onOpenRecent: (path: string) => void;
}

export function AppChrome({
  projectRoot,
  recentProjects,
  onOpenProject,
  onOpenRecent,
}: Props) {
  const [fileOpen, setFileOpen] = useState(false);
  const [recentOpen, setRecentOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const desktop = isTauriHost();

  useEffect(() => {
    const onDocClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setFileOpen(false);
        setRecentOpen(false);
      }
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "o") {
        e.preventDefault();
        onOpenProject();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onOpenProject]);

  const windowAction = useCallback(
    async (action: "minimize" | "maximize" | "close") => {
      if (!desktop) {
        return;
      }
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      if (action === "minimize") {
        await win.minimize();
      } else if (action === "maximize") {
        await win.toggleMaximize();
      } else {
        await win.close();
      }
    },
    [desktop],
  );

  const title = projectRoot
    ? projectRoot.replace(/^.*[/\\]/, "") || projectRoot
    : "teshi";

  return (
    <header className="app-chrome" data-tauri-drag-region>
      <div className="app-chrome-left" ref={menuRef}>
        <div className="app-chrome-menus">
          <button
            type="button"
            className="app-chrome-menu-trigger"
            onClick={() => {
              setFileOpen((v) => !v);
              setRecentOpen(false);
            }}
          >
            File
          </button>
          {fileOpen && (
            <div className="app-chrome-dropdown">
              <button type="button" onClick={() => {
                setFileOpen(false);
                onOpenProject();
              }}>
                Open Project…
              </button>
              <button
                type="button"
                className={recentProjects.length === 0 ? "disabled" : ""}
                disabled={recentProjects.length === 0}
                onClick={() => {
                  setRecentOpen((v) => !v);
                }}
              >
                Open Recent ▸
              </button>
              {recentOpen && recentProjects.length > 0 && (
                <div className="app-chrome-submenu">
                  {recentProjects.map((path) => (
                    <button
                      key={path}
                      type="button"
                      title={path}
                      onClick={() => {
                        setFileOpen(false);
                        setRecentOpen(false);
                        onOpenRecent(path);
                      }}
                    >
                      {path}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
        <span className="app-chrome-title" title={projectRoot ?? undefined}>
          {title}
        </span>
      </div>
      {desktop && (
        <div className="app-chrome-controls">
          <button
            type="button"
            className="app-chrome-winbtn"
            aria-label="Minimize"
            onClick={() => void windowAction("minimize")}
          >
            ─
          </button>
          <button
            type="button"
            className="app-chrome-winbtn"
            aria-label="Maximize"
            onClick={() => void windowAction("maximize")}
          >
            □
          </button>
          <button
            type="button"
            className="app-chrome-winbtn app-chrome-winbtn-close"
            aria-label="Close"
            onClick={() => void windowAction("close")}
          >
            ✕
          </button>
        </div>
      )}
    </header>
  );
}
