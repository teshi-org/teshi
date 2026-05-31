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
  return (
    <div className="welcome">
      <h1>teshi — Desktop</h1>
      <p>BDD recorder and runner shell</p>
      <button type="button" className="primary" onClick={onOpenProject}>
        Open Project
      </button>
      {recentProjects.length > 0 && (
        <div className="recent-list">
          <h2>Recent projects</h2>
          <ul>
            {recentProjects.map((path) => (
              <li key={path}>
                <button type="button" onClick={() => onOpenRecent(path)}>
                  {path}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
