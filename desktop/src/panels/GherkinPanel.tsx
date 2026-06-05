import type { HighlightSpan } from "../types";
import type { StepBindingStatus } from "../locatorTypes";
import { PanelCollapseButton } from "./PanelCollapseButton";

const kindClass: Record<string, string> = {
  default: "hl-default",
  comment: "hl-comment",
  header: "hl-header",
  tag: "hl-tag",
  given: "hl-given",
  when: "hl-when",
  then: "hl-then",
  and_but: "hl-and",
  string: "hl-string",
  meta: "hl-meta",
  doc_string: "hl-doc",
};

// keyword_kind 来自后端 StepKeywordType（Given/When/Then/And/But），据此做语法着色。
const stepKeywordClass: Record<string, string> = {
  Given: "hl-given",
  When: "hl-when",
  Then: "hl-then",
  And: "hl-and",
  But: "hl-and",
};

function stepClassFor(keywordKind: string): string {
  return stepKeywordClass[keywordKind] ?? "hl-default";
}

function SpanLine({ spans }: { spans: HighlightSpan[] }) {
  return (
    <>
      {spans.map((span, i) => (
        <span key={i} className={kindClass[span.kind] ?? "hl-default"}>
          {span.text}
        </span>
      ))}
    </>
  );
}

function StepStatusBadge({ status }: { status?: StepBindingStatus }) {
  const value = status?.status ?? "unbound";
  const label =
    value === "confirmed" ? "bound" : value === "pending" ? "pending" : "unbound";
  return <span className={`step-binding-badge step-binding-badge--${value}`}>{label}</span>;
}

interface Props {
  relativePath: string | null;
  payload: import("../types").FeatureRenderPayload | null;
  stepBindingStatuses: Record<number, StepBindingStatus>;
  selectedScenarioLine: number | null;
  selectedStepLine: number | null;
  onSelectScenario: (line: number) => void;
  onSelectStep: (line: number) => void;
  /** Hide the panel during browser focus mode without unmounting. */
  layoutHidden?: boolean;
  showCollapseButton?: boolean;
  onToggleCollapse?: () => void;
}

export function GherkinPanel({
  relativePath,
  payload,
  stepBindingStatuses,
  selectedScenarioLine,
  selectedStepLine,
  onSelectScenario,
  onSelectStep,
  layoutHidden = false,
  showCollapseButton = false,
  onToggleCollapse,
}: Props) {
  return (
    <section
      className={`panel gherkin-panel${layoutHidden ? " panel--layout-hidden" : ""}`}
      data-testid="GherkinPanel"
    >
      <header className="panel-header panel-header--with-collapse">
        {showCollapseButton && onToggleCollapse && (
          <PanelCollapseButton
            side="left"
            collapsed={false}
            panelLabel="Gherkin"
            onToggle={onToggleCollapse}
          />
        )}
        <span className="panel-header-title">
          Gherkin{relativePath ? `: ${relativePath}` : ""}
        </span>
      </header>
      <div className="panel-body">
        {!payload && (
          <p className="placeholder">Select a .feature file from the tree.</p>
        )}
        {payload?.error && (
          <div className="error-banner">{payload.error.message}</div>
        )}
        {payload && (
          <div className="gherkin-structured">
            {payload.structured.map((block, idx) => {
              if (block.type === "feature_header") {
                return (
                  <div key={idx} className="feature-header-block">
                    <h2>{block.name}</h2>
                    {block.tags.length > 0 && (
                      <div className="tags">{block.tags.join(" ")}</div>
                    )}
                    <div className="lang">language: {block.language}</div>
                  </div>
                );
              }
              if (block.type === "background") {
                return (
                  <div key={idx} className="background-block">
                    <h3>Background</h3>
                    {block.steps.map((step) => (
                      <div
                        key={step.line_number}
                        className={`step-line ${
                          selectedStepLine === step.line_number ? "selected" : ""
                        }`}
                        data-testid={`GherkinStep-${step.line_number}`}
                        onClick={() => onSelectStep(step.line_number)}
                      >
                        <span className={stepClassFor(step.keyword_kind)}>
                          {step.keyword}
                        </span>{" "}
                        {step.text}
                        <StepStatusBadge status={stepBindingStatuses[step.line_number]} />
                      </div>
                    ))}
                  </div>
                );
              }
              if (block.type === "scenario") {
                const selected =
                  selectedScenarioLine === block.line_number;
                return (
                  <div
                    key={idx}
                    className={`scenario-block ${selected ? "selected" : ""}`}
                    onClick={() => onSelectScenario(block.line_number)}
                  >
                    <h3>
                      {block.kind === "scenario_outline"
                        ? "Scenario Outline"
                        : "Scenario"}
                      : {block.name}
                    </h3>
                    {block.tags.map((tag) => (
                      <span key={tag} className="hl-tag">
                        {tag}{" "}
                      </span>
                    ))}
                    {block.steps.map((step) => (
                      <div
                        key={step.line_number}
                        className={`step-line ${
                          selectedStepLine === step.line_number ? "selected" : ""
                        }`}
                        data-testid={`GherkinStep-${step.line_number}`}
                        onClick={(e) => {
                          e.stopPropagation();
                          onSelectStep(step.line_number);
                        }}
                      >
                        <span className={stepClassFor(step.keyword_kind)}>
                          {step.keyword}
                        </span>{" "}
                        {step.text}
                        <StepStatusBadge status={stepBindingStatuses[step.line_number]} />
                      </div>
                    ))}
                  </div>
                );
              }
              return null;
            })}
            {payload.error && (
              <pre className="raw-fallback">
                {payload.raw_lines.map((line) => (
                  <div key={line.line_number}>
                    <SpanLine spans={line.spans} />
                  </div>
                ))}
              </pre>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
