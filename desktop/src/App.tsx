import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { saveWindowState, StateFlags } from "@tauri-apps/plugin-window-state";
import { Toaster, toast } from "sonner";
import { ProjectProvider, useProject } from "./context/ProjectContext";
import { WelcomeScreen } from "./panels/WelcomeScreen";
import { ResizableWorkspace } from "./panels/ResizableWorkspace";
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
    void invoke("finalize_main_window_cmd").catch((err) => {
      console.error("finalize window geometry failed", err);
    });

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
        dispatch({ type: "SET_DOCK_EXPANDED", expanded: true });
      }
    }).then((u) => unsubs.push(u));
    void listen<ActiveStep>("active-step-changed", (event) => {
      dispatch({ type: "SET_ACTIVE_STEP", step: event.payload });
    }).then((u) => unsubs.push(u));
    void listen<FeatureRenderPayload>("feature-refreshed", (event) => {
      dispatch({ type: "REFRESH_FEATURE", payload: event.payload });
    }).then((u) => unsubs.push(u));
    void listen("menu-open-project", () => {
      void pickProject();
    }).then((u) => unsubs.push(u));
    void listen<string>("menu-open-recent", (event) => {
      void openProjectPath(event.payload);
    }).then((u) => unsubs.push(u));

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

    return () => {
      unlistenClose?.();
      unsubs.forEach((u) => u());
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

  return (
    <div className="app-shell">
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
