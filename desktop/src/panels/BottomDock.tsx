import type { DockTab } from "../context/projectState";
import type { ActiveStep, PendingLocator } from "../locatorTypes";
import { LocatorPanel } from "./LocatorPanel";

interface Props {
  expanded: boolean;
  activeTab: DockTab;
  activeStep: ActiveStep | null;
  pendingLocator: PendingLocator | null;
  stepBindingStatuses: Record<number, import("../locatorTypes").StepBindingStatus>;
  onToggle: () => void;
  onTabChange: (tab: DockTab) => void;
  onPendingChange: (pending: PendingLocator | null) => void;
  onBindingChanged: () => void;
}

const TABS: { id: DockTab; label: string }[] = [
  { id: "locator", label: "Locator" },
  { id: "output", label: "Output" },
  { id: "logs", label: "Logs" },
];

export function BottomDock({
  expanded,
  activeTab,
  activeStep,
  pendingLocator,
  stepBindingStatuses,
  onToggle,
  onTabChange,
  onPendingChange,
  onBindingChanged,
}: Props) {
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
            {id === "locator" && pendingLocator?.status === "pending" ? " •" : ""}
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
          {activeTab === "locator" && (
            <LocatorPanel
              activeStep={activeStep}
              pending={pendingLocator}
              stepBindingStatus={
                activeStep ? stepBindingStatuses[activeStep.step_line] : undefined
              }
              onPendingChange={onPendingChange}
              onBindingChanged={onBindingChanged}
            />
          )}
          {activeTab === "output" && (
            <p className="placeholder">Runner output will appear here.</p>
          )}
          {activeTab === "logs" && (
            <p className="placeholder">Application logs are written under AppData.</p>
          )}
        </div>
      )}
    </section>
  );
}
