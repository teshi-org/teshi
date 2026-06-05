import {
  formatOpenProjectShortcut,
  formatRecentProjectEntry,
} from "../layout/recentProjectDisplay";
import { welcomeRecentTestId } from "../layout/testIdPath";

const WELCOME_RECENT_LIMIT = 5;

interface Props {
  recentProjects: string[];
  onOpenProject: () => void;
  onOpenRecent: (path: string) => void;
}

export function WelcomeScreen({
  recentProjects,
  onOpenProject,
  onOpenRecent,
}: Props) {
  const visibleRecentProjects = recentProjects.slice(0, WELCOME_RECENT_LIMIT);

  return (
    <div className="welcome">
      <div className="welcome-content">
        <p className="welcome-intro">Choose an option below to get started</p>
        <div className="welcome-actions">
          <button
            type="button"
            className="welcome-action"
            data-testid="WelcomeOpenProjectButton"
            onClick={onOpenProject}
          >
            <span>Open Project</span>
            <kbd className="welcome-shortcut">{formatOpenProjectShortcut()}</kbd>
          </button>
        </div>
        {visibleRecentProjects.length > 0 && (
          <section className="welcome-recent">
            <h2 className="welcome-recent-title">Recent projects</h2>
            <ul className="welcome-recent-list">
              {visibleRecentProjects.map((path) => {
                const { name, parent } = formatRecentProjectEntry(path);
                return (
                  <li key={path}>
                    <button
                      type="button"
                      className="welcome-recent-item"
                      data-testid={welcomeRecentTestId(path)}
                      onClick={() => onOpenRecent(path)}
                    >
                      <span className="welcome-recent-name">{name}</span>
                      {parent ? (
                        <span className="welcome-recent-parent">{parent}</span>
                      ) : null}
                    </button>
                  </li>
                );
              })}
            </ul>
          </section>
        )}
      </div>
    </div>
  );
}
