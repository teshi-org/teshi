interface Props {
  /** Which workspace edge the controlled panel sits on. */
  side: "left" | "right";
  /** Whether the panel is currently collapsed. */
  collapsed: boolean;
  /** Accessible panel name, e.g. "Gherkin" or "Files". */
  panelLabel: string;
  onToggle: () => void;
}

function ChevronLeftIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
      <path
        fill="currentColor"
        d="M10.5 3.5 6 8l4.5 4.5-.7.7L4.6 8l5.2-5.2z"
      />
    </svg>
  );
}

function ChevronRightIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
      <path
        fill="currentColor"
        d="M5.5 3.5 10 8l-4.5 4.5.7.7L11.4 8 6.2 2.8z"
      />
    </svg>
  );
}

/** VS Code / Cursor-style sidebar collapse chevron. */
export function PanelCollapseButton({
  side,
  collapsed,
  panelLabel,
  onToggle,
}: Props) {
  const action = collapsed ? "Expand" : "Collapse";
  const ariaLabel = `${action} ${panelLabel} panel`;

  // Chevron direction is fixed per workspace edge; it does not flip when the
  // control moves from a side panel header onto the center panel edge.
  const icon = side === "left" ? <ChevronLeftIcon /> : <ChevronRightIcon />;

  return (
    <button
      type="button"
      className="panel-collapse-btn"
      onClick={onToggle}
      title={ariaLabel}
      aria-label={ariaLabel}
    >
      {icon}
    </button>
  );
}
