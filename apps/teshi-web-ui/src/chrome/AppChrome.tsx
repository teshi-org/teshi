import { useEffect, useRef, useState } from "react";

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

  const title = projectRoot
    ? projectRoot.replace(/^.*[/\\]/, "") || projectRoot
    : "Teshi";

  return (
    <header className="app-chrome">
      <div className="app-chrome-menus app-chrome-no-drag" ref={menuRef}>
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
            <button
              type="button"
              onClick={() => {
                setFileOpen(false);
                onOpenProject();
              }}
            >
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
      <div className="app-chrome-drag">
        <span className="app-chrome-title" title={projectRoot ?? undefined}>
          {title}
        </span>
      </div>
    </header>
  );
}
