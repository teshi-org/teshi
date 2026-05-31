import type { DockTab } from "../context/projectState";

interface Props {
  expanded: boolean;
  activeTab: DockTab;
  onToggle: () => void;
  onTabChange: (tab: DockTab) => void;
}

const TABS: { id: DockTab; label: string }[] = [
  { id: "output", label: "Output" },
  { id: "logs", label: "Logs" },
];

export function BottomDock({ expanded, activeTab, onToggle, onTabChange }: Props) {
  return (
    <section
      className={`bottom-dock${expanded ? " bottom-dock--expanded" : " bottom-dock--collapsed"}`}
      aria-label="Bottom panel"
    >
      <header className="bottom-dock-tabs">
        {TABS.map(({ id, label }) => (
          <button
            key={id}
            type="button"
            className={activeTab === id ? "active" : ""}
            onClick={() => onTabChange(id)}
          >
            {label}
          </button>
        ))}
        <div className="bottom-dock-tabs-spacer" />
        <button
          type="button"
          className="bottom-dock-toggle"
          onClick={onToggle}
          aria-expanded={expanded}
          title={expanded ? "Collapse panel" : "Expand panel"}
        >
          {expanded ? "▾" : "▴"}
        </button>
      </header>
      {expanded && (
        <div className="bottom-dock-body">
          <p className="placeholder">Coming soon.</p>
        </div>
      )}
    </section>
  );
}
