import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { getRuntime } from "../platform";
import type { ActiveStep, PendingLocator, StepBindingStatus } from "../locatorTypes";

interface Props {
  activeStep: ActiveStep | null;
  pending: PendingLocator | null;
  stepBindingStatus: StepBindingStatus | undefined;
  onPendingChange: (pending: PendingLocator | null) => void;
  onBindingChanged: () => void;
}

export function LocatorPanel({
  activeStep,
  pending,
  stepBindingStatus,
  onPendingChange,
  onBindingChanged,
}: Props) {
  const [selectedRank, setSelectedRank] = useState<number>(1);
  const [editedValue, setEditedValue] = useState("");
  const [editMode, setEditMode] = useState(false);
  const [autoConfirmSec, setAutoConfirmSec] = useState(60);
  const [countdown, setCountdown] = useState<number | null>(null);
  const highlightTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const autoConfirmTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const acceptingRef = useRef(false);

  useEffect(
    () => () => {
      if (highlightTimerRef.current) {
        clearTimeout(highlightTimerRef.current);
      }
      if (autoConfirmTimerRef.current) {
        clearInterval(autoConfirmTimerRef.current);
      }
    },
    [],
  );

  useEffect(() => {
    void getRuntime()
      .getProjectSettings()
      .then((settings) => {
        setAutoConfirmSec(settings.locator_auto_confirm_sec);
      })
      .catch((e) => {
        console.warn("load project settings failed", e);
      });
  }, []);

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
    if (!pending || mismatch || acceptingRef.current) return;
    acceptingRef.current = true;
    try {
      await getRuntime().confirmLocator(
        selectedRank,
        editMode ? editedValue : null,
      );
      toast.success("Locator saved to step-bindings");
      onPendingChange(null);
      onBindingChanged();
    } catch (e) {
      toast.error(String(e));
    } finally {
      acceptingRef.current = false;
    }
  }, [
    pending,
    mismatch,
    selectedRank,
    editedValue,
    editMode,
    onPendingChange,
    onBindingChanged,
  ]);

  const onReject = useCallback(async () => {
    try {
      await getRuntime().rejectLocator();
      toast.message("Locator proposal rejected");
      onPendingChange(null);
    } catch (e) {
      toast.error(String(e));
    }
  }, [onPendingChange]);

  const onUnbind = useCallback(async () => {
    if (!activeStep) return;
    try {
      await getRuntime().unbindStep(
        activeStep.feature_relative_path,
        activeStep.step_line,
      );
      toast.success("Step binding removed");
      onBindingChanged();
    } catch (e) {
      toast.error(String(e));
    }
  }, [activeStep, onBindingChanged]);

  // Auto-confirm countdown when a pending proposal is ready and aligned with the active step.
  useEffect(() => {
    if (autoConfirmTimerRef.current) {
      clearInterval(autoConfirmTimerRef.current);
      autoConfirmTimerRef.current = null;
    }
    setCountdown(null);

    if (!pending || pending.status !== "pending" || mismatch || autoConfirmSec === 0) {
      return;
    }

    setCountdown(autoConfirmSec);
    autoConfirmTimerRef.current = setInterval(() => {
      setCountdown((prev) => {
        if (prev === null) {
          return null;
        }
        if (prev <= 1) {
          if (autoConfirmTimerRef.current) {
            clearInterval(autoConfirmTimerRef.current);
            autoConfirmTimerRef.current = null;
          }
          void onAccept();
          return 0;
        }
        return prev - 1;
      });
    }, 1000);

    return () => {
      if (autoConfirmTimerRef.current) {
        clearInterval(autoConfirmTimerRef.current);
        autoConfirmTimerRef.current = null;
      }
    };
  }, [pending, mismatch, autoConfirmSec, onAccept]);

  const isBound = stepBindingStatus?.status === "confirmed";

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
          {isBound && !pending && (
            <div className="locator-actions locator-actions--inline">
              <button type="button" data-testid="LocatorUnbind" onClick={() => void onUnbind()}>
                Unbind
              </button>
            </div>
          )}
        </div>
      )}

      {mismatch && (
        <div className="locator-warning">
          Pending proposal targets a different step than the current Gherkin selection. Confirm is
          disabled; reject the proposal or select the matching step.
        </div>
      )}

      {!pending && !isBound && (
        <p className="placeholder">
          Waiting for agent proposal in <code>.teshi/pending-locator.json</code>.
        </p>
      )}

      {pending && (
        <>
          {countdown !== null && autoConfirmSec > 0 && !mismatch && (
            <p className="locator-meta">
              Auto-confirm in {countdown}s (set <code>locator_auto_confirm_sec</code> in{" "}
              <code>.teshi/settings.json</code> to 0 for manual-only)
            </p>
          )}

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
            <button
              type="button"
              className="primary"
              data-testid="LocatorConfirm"
              disabled={Boolean(mismatch)}
              onClick={() => void onAccept()}
            >
              Confirm
            </button>
            <button type="button" onClick={() => setEditMode((v) => !v)}>
              {editMode ? "Cancel Edit" : "Edit"}
            </button>
            <button type="button" data-testid="LocatorReject" onClick={() => void onReject()}>
              Reject
            </button>
          </div>
        </>
      )}
    </div>
  );
}
