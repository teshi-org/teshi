import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { getRuntime } from "../platform";
import type { ActiveStep, PendingLocator } from "../locatorTypes";

interface Props {
  activeStep: ActiveStep | null;
  pending: PendingLocator | null;
  onPendingChange: (pending: PendingLocator | null) => void;
}

export function LocatorPanel({
  activeStep,
  pending,
  onPendingChange,
}: Props) {
  const [selectedRank, setSelectedRank] = useState<number>(1);
  const [editedValue, setEditedValue] = useState("");
  const [editMode, setEditMode] = useState(false);
  const highlightTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (highlightTimerRef.current) {
        clearTimeout(highlightTimerRef.current);
      }
    },
    [],
  );

  useEffect(() => {
    if (pending?.candidates.length) {
      const rank = pending.highlight?.candidate_rank ?? pending.candidates[0].rank;
      setSelectedRank(rank);
      const candidate =
        pending.candidates.find((c) => c.rank === rank) ?? pending.candidates[0];
      setEditedValue(candidate.value);
      setEditMode(false);
    }
  }, [pending]);

  const mismatch =
    pending &&
    activeStep &&
    (pending.step_ref.step_line !== activeStep.step_line ||
      pending.step_ref.feature_relative_path !== activeStep.feature_relative_path);

  const scheduleHighlight = useCallback((selector: string) => {
    if (highlightTimerRef.current) {
      clearTimeout(highlightTimerRef.current);
    }
    highlightTimerRef.current = setTimeout(() => {
      highlightTimerRef.current = null;
      void getRuntime()
        .highlightLocator(selector)
        .catch((e) => {
          console.warn("highlight locator failed", e);
          toast.error(String(e));
        });
    }, 150);
  }, []);

  const onAccept = useCallback(async () => {
    if (!pending) return;
    try {
      await getRuntime().confirmLocator(
        selectedRank,
        editMode ? editedValue : null,
      );
      toast.success("Locator saved to step-bindings");
      onPendingChange(null);
    } catch (e) {
      toast.error(String(e));
    }
  }, [pending, selectedRank, editedValue, editMode, onPendingChange]);

  const onReject = useCallback(async () => {
    try {
      await getRuntime().rejectLocator();
      toast.message("Locator proposal rejected");
      onPendingChange(null);
    } catch (e) {
      toast.error(String(e));
    }
  }, [onPendingChange]);

  if (!activeStep && !pending) {
    return (
      <p className="placeholder">
        Select a Gherkin step, then invoke the bdd-locator skill in the terminal agent.
      </p>
    );
  }

  return (
    <div className="locator-panel">
      {activeStep && (
        <div className="locator-step-summary">
          <strong>
            {activeStep.step_keyword} {activeStep.step_text}
          </strong>
          <span className="locator-meta">
            {activeStep.feature_relative_path} · scenario L{activeStep.scenario_line} · step L
            {activeStep.step_line}
          </span>
        </div>
      )}

      {mismatch && (
        <div className="locator-warning">
          Pending proposal targets a different step than the current Gherkin selection.
        </div>
      )}

      {!pending && (
        <p className="placeholder">
          Waiting for agent proposal in <code>.teshi/pending-locator.json</code>.
        </p>
      )}

      {pending && (
        <>
          <ul className="locator-candidates">
            {pending.candidates.map((candidate) => (
              <li key={candidate.rank}>
                <label className="locator-candidate">
                  <input
                    type="radio"
                    name="locator-candidate"
                    checked={selectedRank === candidate.rank}
                    onChange={() => {
                      setSelectedRank(candidate.rank);
                      setEditedValue(candidate.value);
                      scheduleHighlight(candidate.value);
                    }}
                  />
                  <span className="locator-candidate-body">
                    <span className="locator-candidate-title">
                      #{candidate.rank} {candidate.strategy} ·{" "}
                      {(candidate.confidence * 100).toFixed(0)}%
                    </span>
                    <code>{candidate.value}</code>
                    <span className="locator-rationale">{candidate.rationale}</span>
                  </span>
                </label>
              </li>
            ))}
          </ul>

          {editMode && (
            <label className="locator-edit">
              <span>Edit selector</span>
              <input
                type="text"
                value={editedValue}
                onChange={(e) => setEditedValue(e.target.value)}
                spellCheck={false}
              />
            </label>
          )}

          <div className="locator-actions">
            <button type="button" className="primary" onClick={() => void onAccept()}>
              Confirm
            </button>
            <button type="button" onClick={() => setEditMode((v) => !v)}>
              {editMode ? "Cancel Edit" : "Edit"}
            </button>
            <button type="button" onClick={() => void onReject()}>
              Reject
            </button>
          </div>
        </>
      )}
    </div>
  );
}
