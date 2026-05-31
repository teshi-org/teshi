import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Toaster, toast } from "sonner";
import { ProjectProvider, useProject } from "./context/ProjectContext";
import { WelcomeScreen } from "./panels/WelcomeScreen";
import { GherkinPanel } from "./panels/GherkinPanel";
import { BrowserPanel } from "./panels/BrowserPanel";
import { FileTreeTerminalPanel } from "./panels/FileTreeTerminalPanel";
import { BottomDock } from "./panels/BottomDock";
import type { BrowserError, FeatureRenderPayload } from "./types";
import type { ActiveStep, PendingLocator } from "./locatorTypes";

/** Ask before stopping browser/terminal; uses JS dialog to avoid Rust blocking_show deadlocks. */
async function confirmStopRuntimeIfBusy(): Promise<boolean> {
  const allowed = await invoke<boolean>("check_project_switch_allowed");
  if (allowed) {
    return true;
  }
  const { ask } = await import("@tauri-apps/plugin-dialog");
  return (
    (await ask("Browser/Terminal is running. Continuing will stop them.", {
      title: "Confirm",
      kind: "warning",
    })) ?? false
  );
}

function AppShell() {
  const { state, dispatch } = useProject();
  const [browserError, setBrowserError] = useState<string | null>(null);
  const [browserHint, setBrowserHint] = useState<string | null>(null);

  const openProjectPath = useCallback(
    async (path: string) => {
      const ok = await confirmStopRuntimeIfBusy();
      if (!ok) return;
      await invoke("teardown_runtime");
      await invoke("open_project", { path });
      // open_project 已写入 recent.json，这里重新拉取以刷新下拉列表。
      const recent = await invoke<string[]>("get_recent_projects_cmd");
      dispatch({ type: "SET_RECENT", paths: recent });
      dispatch({ type: "SET_PROJECT", root: path });
      setBrowserError(null);
      setBrowserHint(null);
      dispatch({ type: "SET_BROWSER", wsUrl: null, running: false });
      dispatch({ type: "SET_ACTIVE_STEP", step: null });
      dispatch({ type: "SET_PENDING_LOCATOR", pending: null });
      void invoke<PendingLocator | null>("get_pending_locator_cmd").then((pending) => {
        dispatch({ type: "SET_PENDING_LOCATOR", pending });
      });
      void invoke<ActiveStep | null>("get_active_step_cmd").then((step) => {
        dispatch({ type: "SET_ACTIVE_STEP", step });
      });
    },
    [dispatch],
  );

  const pickProject = useCallback(async () => {
    const picked = await invoke<string | null>("open_project_dir");
    if (picked) {
      await openProjectPath(picked);
    }
  }, [openProjectPath]);

  useEffect(() => {
    void invoke<string[]>("get_recent_projects_cmd").then((paths) => {
      dispatch({ type: "SET_RECENT", paths });
    });

    const unsubs: Array<() => void> = [];
    void listen<string>("open-project-cli", (event) => {
      void openProjectPath(event.payload);
    }).then((u) => unsubs.push(u));
    void listen<string[]>("recent-loaded", (event) => {
      dispatch({ type: "SET_RECENT", paths: event.payload });
    }).then((u) => unsubs.push(u));
    void listen<PendingLocator | null>("pending-locator-changed", (event) => {
      dispatch({ type: "SET_PENDING_LOCATOR", pending: event.payload ?? null });
      if (event.payload?.status === "pending") {
        toast.info("Locator proposal ready for review");
        dispatch({ type: "SET_DOCK_TAB", tab: "locator" });
      }
    }).then((u) => unsubs.push(u));
    void listen<ActiveStep>("active-step-changed", (event) => {
      dispatch({ type: "SET_ACTIVE_STEP", step: event.payload });
    }).then((u) => unsubs.push(u));
    void listen<FeatureRenderPayload>("feature-refreshed", (event) => {
      dispatch({ type: "REFRESH_FEATURE", payload: event.payload });
    }).then((u) => unsubs.push(u));

    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void pickProject();
      }
    };
    window.addEventListener("keydown", onKey);

    // Hold the native close until teardown finishes. Do not call Rust blocking_show()
    // from this handler — it can deadlock on Windows during WM_CLOSE.
    let unlistenClose: (() => void) | undefined;
    let closing = false;
    void (async () => {
      unlistenClose = await getCurrentWindow().onCloseRequested(async (event) => {
        if (closing) {
          return;
        }
        event.preventDefault();
        closing = true;
        try {
          const ok = await confirmStopRuntimeIfBusy();
          if (!ok) {
            closing = false;
            return;
          }
          await invoke("teardown_runtime");
        } catch (err) {
          console.error("shutdown before close failed", err);
        }
        await getCurrentWindow().destroy();
      });
    })();

    // Debounce window resize persistence; skip invalid sizes from startup/minimize glitches.
    const MIN_WINDOW_WIDTH = 1280;
    const MIN_WINDOW_HEIGHT = 720;
    let resizeTimer: ReturnType<typeof setTimeout> | null = null;
    void getCurrentWindow()
      .onResized(async ({ payload }) => {
        const factor = await getCurrentWindow().scaleFactor();
        const logical = payload.toLogical(factor);
        const width = Math.round(logical.width);
        const height = Math.round(logical.height);
        if (width < MIN_WINDOW_WIDTH || height < MIN_WINDOW_HEIGHT) {
          return;
        }
        if (resizeTimer) clearTimeout(resizeTimer);
        resizeTimer = setTimeout(() => {
          void invoke("save_window_settings", { width, height });
        }, 500);
      })
      .then((u) => unsubs.push(u));

    return () => {
      unlistenClose?.();
      unsubs.forEach((u) => u());
      window.removeEventListener("keydown", onKey);
      if (resizeTimer) clearTimeout(resizeTimer);
    };
  }, [dispatch, openProjectPath, pickProject]);

  const syncActiveStep = useCallback(
    async (stepLine: number) => {
      if (!state.selectedFeaturePath) {
        return;
      }
      try {
        const active = await invoke<ActiveStep>("sync_active_step_cmd", {
          featurePath: state.selectedFeaturePath,
          stepLine,
        });
        dispatch({ type: "SET_ACTIVE_STEP", step: active });
        dispatch({ type: "SET_DOCK_TAB", tab: "locator" });
      } catch (e) {
        toast.error(String(e));
      }
    },
    [dispatch, state.selectedFeaturePath],
  );

  const openFeature = async (path: string) => {
    const payload = await invoke<FeatureRenderPayload>("render_feature_cmd", {
      path,
    });
    dispatch({ type: "SET_FEATURE", path, payload });
  };

  const startBrowser = async () => {
    setBrowserError(null);
    setBrowserHint(null);
    try {
      const result = await invoke<{ ws_url: string }>("start_browser_sidecar");
      dispatch({
        type: "SET_BROWSER",
        wsUrl: result.ws_url,
        running: true,
      });
    } catch (e) {
      const err = e as BrowserError;
      setBrowserError(err.message ?? String(e));
      setBrowserHint(err.hint ?? null);
      dispatch({ type: "SET_BROWSER", wsUrl: null, running: false });
    }
  };

  const stopBrowser = async () => {
    await invoke("stop_browser_sidecar");
    dispatch({ type: "SET_BROWSER", wsUrl: null, running: false });
  };

  if (!state.projectRoot) {
    return (
      <>
        <WelcomeScreen
          recentProjects={state.recentProjects}
          onOpenProject={() => void pickProject()}
          onOpenRecent={(path) => void openProjectPath(path)}
        />
        <Toaster theme="dark" />
      </>
    );
  }

  const browserFocus = state.layoutMode === "browserFocus";

  const browserControls = (
    <>
      {!state.browserRunning ? (
        <button type="button" onClick={() => void startBrowser()}>
          Start Browser
        </button>
      ) : (
        <button type="button" onClick={() => void stopBrowser()}>
          Stop Browser
        </button>
      )}
      <span
        className={`status-dot ${state.browserRunning ? "on" : "off"}`}
        title={state.browserRunning ? "Browser running" : "Browser stopped"}
      />
    </>
  );

  return (
    <div className="app-shell">
      <header
        className={`toolbar${browserFocus ? " toolbar--minimal" : ""}`}
      >
        {browserFocus ? (
          <>
            <button
              type="button"
              onClick={() =>
                dispatch({ type: "SET_LAYOUT_MODE", mode: "normal" })
              }
            >
              Exit Focus
            </button>
            <div className="toolbar-spacer" />
            {browserControls}
          </>
        ) : (
          <>
            <button type="button" onClick={() => void pickProject()}>
              Open Project
            </button>
            <select
              value=""
              onChange={(e) => {
                if (e.target.value) void openProjectPath(e.target.value);
              }}
            >
              <option value="">Recent…</option>
              {state.recentProjects.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
            <span className="project-path">{state.projectRoot}</span>
            <div className="toolbar-spacer" />
            {browserControls}
          </>
        )}
      </header>
      <div className="workspace">
        <main
          className={`layout${browserFocus ? " layout--browser-focus" : ""}`}
        >
          {!browserFocus && (
            <GherkinPanel
              relativePath={state.featurePayload?.relative_path ?? null}
              payload={state.featurePayload}
              selectedScenarioLine={state.selectedScenarioLine}
              selectedStepLine={state.selectedStepLine}
              onSelectScenario={(line) =>
                dispatch({ type: "SELECT_SCENARIO", line })
              }
              onSelectStep={(line) => {
                dispatch({ type: "SELECT_STEP", line });
                if (line !== null) {
                  void syncActiveStep(line);
                }
              }}
            />
          )}
          <BrowserPanel
            wsUrl={state.browserWsUrl}
            running={state.browserRunning}
            error={browserError}
            hint={browserHint}
            focusMode={browserFocus}
            onStart={() => void startBrowser()}
            onStop={() => void stopBrowser()}
            onToggleFocus={() =>
              dispatch({
                type: "SET_LAYOUT_MODE",
                mode: browserFocus ? "normal" : "browserFocus",
              })
            }
          />
          {!browserFocus && (
            <FileTreeTerminalPanel
              projectRoot={state.projectRoot}
              tab={state.rightTab}
              onTabChange={(tab) => dispatch({ type: "SET_TAB", tab })}
              onOpenFeature={(path) => void openFeature(path)}
            />
          )}
        </main>
        <BottomDock
          expanded={state.dockExpanded}
          activeTab={state.dockActiveTab}
          activeStep={state.activeStep}
          pendingLocator={state.pendingLocator}
          onToggle={() => dispatch({ type: "TOGGLE_DOCK" })}
          onTabChange={(tab) => dispatch({ type: "SET_DOCK_TAB", tab })}
          onPendingChange={(pending) =>
            dispatch({ type: "SET_PENDING_LOCATOR", pending })
          }
        />
      </div>
      <Toaster theme="dark" />
    </div>
  );
}

export default function App() {
  return (
    <ProjectProvider>
      <AppShell />
    </ProjectProvider>
  );
}
