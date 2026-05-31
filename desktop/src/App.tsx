import { useCallback, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { saveWindowState, StateFlags } from "@tauri-apps/plugin-window-state";
import { Toaster, toast } from "sonner";
import { AppChrome } from "./chrome/AppChrome";
import { ProjectProvider, useProject } from "./context/ProjectContext";
import { getRuntime, isTauriHost } from "./platform";
import { WelcomeScreen } from "./panels/WelcomeScreen";
import { ResizableWorkspace } from "./panels/ResizableWorkspace";
import { BottomDock } from "./panels/BottomDock";
import type { BrowserError, FeatureRenderPayload } from "./types";
import type { ActiveStep, PendingLocator } from "./locatorTypes";

const runtime = getRuntime();

function AppShell() {
  const { state, dispatch } = useProject();
  const [browserError, setBrowserError] = useState<string | null>(null);
  const [browserHint, setBrowserHint] = useState<string | null>(null);

  const openProjectPath = useCallback(
    async (path: string) => {
      const ok = await runtime.confirmStopRuntimeIfBusy();
      if (!ok) return;
      await runtime.teardownRuntime();
      await runtime.openProject(path);
      const recent = await runtime.getRecentProjects();
      dispatch({ type: "SET_RECENT", paths: recent });
      dispatch({ type: "SET_PROJECT", root: path });
      setBrowserError(null);
      setBrowserHint(null);
      dispatch({ type: "SET_BROWSER", wsUrl: null, running: false });
      dispatch({ type: "SET_ACTIVE_STEP", step: null });
      dispatch({ type: "SET_PENDING_LOCATOR", pending: null });
      void runtime.getPendingLocator().then((pending) => {
        dispatch({ type: "SET_PENDING_LOCATOR", pending });
      });
      void runtime.getActiveStep().then((step) => {
        dispatch({ type: "SET_ACTIVE_STEP", step });
      });
    },
    [dispatch],
  );

  const pickProject = useCallback(async () => {
    const picked = await runtime.openProjectDir();
    if (picked) {
      await openProjectPath(picked);
    }
  }, [openProjectPath]);

  useEffect(() => {
    void runtime.finalizeMainWindow?.().catch((err) => {
      console.error("finalize window geometry failed", err);
    });

    void runtime.getRecentProjects().then((paths) => {
      dispatch({ type: "SET_RECENT", paths });
    });

    const unsubs: Array<() => void> = [];
    void runtime.onEvent<string>("open-project-cli", (path) => {
      void openProjectPath(path);
    }).then((u) => unsubs.push(u));
    void runtime.onEvent<string[]>("recent-loaded", (paths) => {
      dispatch({ type: "SET_RECENT", paths });
    }).then((u) => unsubs.push(u));
    void runtime
      .onEvent<PendingLocator | null>("pending-locator-changed", (pending) => {
        dispatch({ type: "SET_PENDING_LOCATOR", pending: pending ?? null });
        if (pending?.status === "pending") {
          toast.info("Locator proposal ready for review");
          dispatch({ type: "SET_DOCK_TAB", tab: "locator" });
          dispatch({ type: "SET_DOCK_EXPANDED", expanded: true });
        }
      })
      .then((u) => unsubs.push(u));
    void runtime.onEvent<ActiveStep>("active-step-changed", (step) => {
      dispatch({ type: "SET_ACTIVE_STEP", step });
    }).then((u) => unsubs.push(u));
    void runtime
      .onEvent<FeatureRenderPayload>("feature-refreshed", (payload) => {
        dispatch({ type: "REFRESH_FEATURE", payload });
      })
      .then((u) => unsubs.push(u));

    let unlistenClose: (() => void) | undefined;
    let closing = false;
    if (isTauriHost()) {
      void (async () => {
        unlistenClose = await getCurrentWindow().onCloseRequested(async (event) => {
          if (closing) {
            return;
          }
          event.preventDefault();
          closing = true;
          try {
            const ok = await runtime.confirmStopRuntimeIfBusy();
            if (!ok) {
              closing = false;
              return;
            }
            await runtime.teardownRuntime();
          } catch (err) {
            console.error("shutdown before close failed", err);
          }
          try {
            await saveWindowState(
              StateFlags.SIZE |
                StateFlags.MAXIMIZED |
                StateFlags.FULLSCREEN |
                StateFlags.VISIBLE,
            );
          } catch (err) {
            console.error("save window state failed", err);
          }
          await getCurrentWindow().destroy();
        });
      })();
    }

    return () => {
      unlistenClose?.();
      unsubs.forEach((u) => u());
    };
  }, [dispatch, openProjectPath]);

  const syncActiveStep = useCallback(
    async (stepLine: number) => {
      if (!state.selectedFeaturePath) {
        return;
      }
      try {
        const active = await runtime.syncActiveStep(
          state.selectedFeaturePath,
          stepLine,
        );
        dispatch({ type: "SET_ACTIVE_STEP", step: active });
        dispatch({ type: "SET_DOCK_TAB", tab: "locator" });
      } catch (e) {
        toast.error(String(e));
      }
    },
    [dispatch, state.selectedFeaturePath],
  );

  const openFeature = async (path: string) => {
    const payload = await runtime.renderFeature(path);
    dispatch({ type: "SET_FEATURE", path, payload });
  };

  const startBrowser = async () => {
    setBrowserError(null);
    setBrowserHint(null);
    try {
      const result = await runtime.startBrowserSidecar();
      dispatch({
        type: "SET_BROWSER",
        wsUrl: result.ws_url,
        running: true,
      });
    } catch (e) {
      let err = e as BrowserError;
      if (e instanceof Error && !err.message) {
        try {
          err = JSON.parse(e.message) as BrowserError;
        } catch {
          err = { message: e.message };
        }
      }
      setBrowserError(err.message ?? String(e));
      setBrowserHint(err.hint ?? null);
      dispatch({ type: "SET_BROWSER", wsUrl: null, running: false });
    }
  };

  const stopBrowser = async () => {
    await runtime.stopBrowserSidecar();
    dispatch({ type: "SET_BROWSER", wsUrl: null, running: false });
  };

  const browserFullscreen = state.layoutMode === "browserFullscreen";

  useEffect(() => {
    if (!state.projectRoot || !browserFullscreen) return;

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        dispatch({ type: "SET_LAYOUT_MODE", mode: "normal" });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [state.projectRoot, browserFullscreen, dispatch]);

  const chromeProps = {
    projectRoot: state.projectRoot,
    recentProjects: state.recentProjects,
    hidden: browserFullscreen,
    onOpenProject: () => void pickProject(),
    onOpenRecent: (path: string) => void openProjectPath(path),
  };

  if (!state.projectRoot) {
    return (
      <div className="app-shell">
        <AppChrome {...chromeProps} />
        <WelcomeScreen
          recentProjects={state.recentProjects}
          onOpenProject={() => void pickProject()}
          onOpenRecent={(path) => void openProjectPath(path)}
        />
        <Toaster theme="dark" />
      </div>
    );
  }

  return (
    <div className="app-shell">
      <AppChrome {...chromeProps} />
      <div className="workspace">
        <ResizableWorkspace
          browserFullscreen={browserFullscreen}
          projectRoot={state.projectRoot}
          featurePayload={state.featurePayload}
          selectedScenarioLine={state.selectedScenarioLine}
          selectedStepLine={state.selectedStepLine}
          browserWsUrl={state.browserWsUrl}
          browserRunning={state.browserRunning}
          browserError={browserError}
          browserHint={browserHint}
          rightTab={state.rightTab}
          onSelectScenario={(line) =>
            dispatch({ type: "SELECT_SCENARIO", line })
          }
          onSelectStep={(line) => {
            dispatch({ type: "SELECT_STEP", line });
            if (line !== null) {
              void syncActiveStep(line);
            }
          }}
          onStartBrowser={() => void startBrowser()}
          onStopBrowser={() => void stopBrowser()}
          onToggleBrowserFullscreen={() =>
            dispatch({
              type: "SET_LAYOUT_MODE",
              mode: browserFullscreen ? "normal" : "browserFullscreen",
            })
          }
          onRightTabChange={(tab) => dispatch({ type: "SET_TAB", tab })}
          onOpenFeature={(path) => void openFeature(path)}
        />
        {!browserFullscreen && (
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
        )}
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
