import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { saveWindowState, StateFlags } from "@tauri-apps/plugin-window-state";
import {
  Group,
  Panel,
  Separator,
  useGroupRef,
  usePanelRef,
  type Layout,
} from "react-resizable-panels";
import { Toaster, toast } from "sonner";
import { AppChrome } from "./chrome/AppChrome";
import { ProjectProvider, useProject } from "./context/ProjectContext";
import { getRuntime, isTauriHost } from "./platform";
import { WelcomeScreen } from "./panels/WelcomeScreen";
import { RequirementsPage } from "./panels/RequirementsPage";
import { ResizableWorkspace } from "./panels/ResizableWorkspace";
import { BottomDock } from "./panels/BottomDock";
import {
  defaultDockLayoutForProject,
  dockLayoutFromGroupLayout,
  loadWorkspaceLayout,
  saveWorkspaceLayout,
} from "./layout/workspaceLayoutStorage";
import type { BrowserError, FeatureRenderPayload } from "./types";
import type { ActiveStep, PendingLocator } from "./locatorTypes";

const SAVE_DEBOUNCE_MS = 150;

function AppShell() {
  const { state, dispatch } = useProject();
  const [browserError, setBrowserError] = useState<string | null>(null);
  const [browserHint, setBrowserHint] = useState<string | null>(null);
  const selectedFeatureRelativePath = state.featurePayload?.relative_path ?? null;
  const browserFullscreen = state.layoutMode === "browserFullscreen";
  const [showRequirements, setShowRequirements] = useState(true);
  const dockGroupRef = useGroupRef();
  const dockPanelRef = usePanelRef();
  const dockSaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const skipNextDockPersistRef = useRef(false);
  const dockExpandedRef = useRef(state.dockExpanded);
  const dockActiveTabRef = useRef(state.dockActiveTab);
  const projectRootRef = useRef(state.projectRoot);
  const selectedFeatureRelativePathRef = useRef(selectedFeatureRelativePath);

  const refreshStepStatuses = useCallback(
    async (featurePath: string | null) => {
      if (!featurePath) return;
      try {
        const statuses = await getRuntime().getStepBindingStatuses(featurePath);
        dispatch({ type: "SET_STEP_BINDING_STATUSES", statuses });
      } catch (err) {
        console.error("load step binding statuses failed", err);
      }
    },
    [dispatch],
  );

  const openProjectPath = useCallback(
    async (path: string) => {
      const runtime = getRuntime();
      const ok = await runtime.confirmStopRuntimeIfBusy();
      if (!ok) return;
      await runtime.teardownRuntime();
      await runtime.openProject(path);
      const recent = await runtime.getRecentProjects();
      dispatch({ type: "SET_RECENT", paths: recent });
      setBrowserError(null);
      setBrowserHint(null);
      dispatch({ type: "SET_BROWSER", wsUrl: null, running: false, mode: null });
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

  const openProjectPathWithFeedback = useCallback(
    async (path: string) => {
      try {
        await openProjectPath(path);
      } catch (e) {
        console.error("open project failed", e);
        toast.error(e instanceof Error ? e.message : String(e));
      }
    },
    [openProjectPath],
  );

  const pickProject = useCallback(async () => {
    try {
      const picked = await getRuntime().openProjectDir();
      if (picked) {
        await openProjectPathWithFeedback(picked);
      }
    } catch (e) {
      console.error("pick project folder failed", e);
      toast.error(e instanceof Error ? e.message : String(e));
    }
  }, [openProjectPathWithFeedback]);

  const defaultDockLayout = useMemo(
    () =>
      state.projectRoot
        ? defaultDockLayoutForProject(state.projectRoot)
        : { main: 75, dock: 25 },
    [state.projectRoot],
  );

  useEffect(() => {
    dockExpandedRef.current = state.dockExpanded;
  }, [state.dockExpanded]);

  useEffect(() => {
    dockActiveTabRef.current = state.dockActiveTab;
  }, [state.dockActiveTab]);

  useEffect(() => {
    projectRootRef.current = state.projectRoot;
  }, [state.projectRoot]);

  useEffect(() => {
    selectedFeatureRelativePathRef.current = selectedFeatureRelativePath;
  }, [selectedFeatureRelativePath]);

  const openFeature = useCallback(
    async (path: string) => {
      const payload = await getRuntime().renderFeature(path);
      dispatch({ type: "SET_FEATURE", path, payload });
      void refreshStepStatuses(payload.relative_path);
    },
    [dispatch, refreshStepStatuses],
  );

  useEffect(() => {
    if (!state.projectRoot) {
      return;
    }
    const saved = loadWorkspaceLayout(state.projectRoot);
    skipNextDockPersistRef.current = true;
    dispatch({
      type: "RESTORE_DOCK",
      expanded: saved?.dockExpanded ?? false,
      activeTab: saved?.dockActiveTab ?? "locator",
    });
  }, [dispatch, state.projectRoot]);

  useEffect(() => {
    if (!state.projectRoot || browserFullscreen) {
      return;
    }
    requestAnimationFrame(() => {
      if (state.dockExpanded) {
        dockPanelRef.current?.expand();
      } else {
        dockPanelRef.current?.collapse();
      }
    });
  }, [browserFullscreen, dockPanelRef, state.dockExpanded, state.projectRoot]);

  useEffect(() => {
    if (!state.projectRoot) {
      return;
    }
    if (skipNextDockPersistRef.current) {
      skipNextDockPersistRef.current = false;
      return;
    }
    saveWorkspaceLayout(state.projectRoot, {
      dockExpanded: state.dockExpanded,
      dockActiveTab: state.dockActiveTab,
    });
  }, [state.dockActiveTab, state.dockExpanded, state.projectRoot]);

  const persistDockLayout = useCallback(
    (layout: Layout) => {
      if (!state.projectRoot) {
        return;
      }
      const dockLayout = dockLayoutFromGroupLayout(layout);
      if (!dockLayout) {
        return;
      }
      saveWorkspaceLayout(state.projectRoot, {
        dockLayout,
        dockExpanded: dockExpandedRef.current,
        dockActiveTab: dockActiveTabRef.current,
      });
    },
    [state.projectRoot],
  );

  const scheduleDockPersist = useCallback(
    (layout: Layout) => {
      if (dockSaveTimerRef.current) {
        clearTimeout(dockSaveTimerRef.current);
      }
      dockSaveTimerRef.current = setTimeout(() => {
        persistDockLayout(layout);
        dockSaveTimerRef.current = null;
      }, SAVE_DEBOUNCE_MS);
    },
    [persistDockLayout],
  );

  useEffect(
    () => () => {
      if (dockSaveTimerRef.current) {
        clearTimeout(dockSaveTimerRef.current);
      }
    },
    [],
  );

  useEffect(() => {
    const runtime = getRuntime();
    void runtime.finalizeMainWindow?.().catch((err) => {
      console.error("finalize window geometry failed", err);
    });

    void runtime
      .getRecentProjects()
      .then((paths) => {
        dispatch({ type: "SET_RECENT", paths });
      })
      .catch((err) => {
        console.error("load recent projects failed", err);
      });

    const unsubs: Array<() => void> = [];
    void runtime.onEvent<string>("open-project-cli", (path) => {
      void openProjectPathWithFeedback(path);
    }).then((u) => unsubs.push(u));
    void runtime.onEvent<string[]>("recent-loaded", (paths) => {
      dispatch({ type: "SET_RECENT", paths });
    }).then((u) => unsubs.push(u));
    void runtime.onEvent<string>("project-changed", (canonicalRoot) => {
      dispatch({ type: "SET_PROJECT", root: canonicalRoot });
    }).then((u) => unsubs.push(u));
    void runtime
      .onEvent<PendingLocator | null>("pending-locator-changed", (pending) => {
        dispatch({ type: "SET_PENDING_LOCATOR", pending: pending ?? null });
        void refreshStepStatuses(
          pending?.step_ref.feature_relative_path ?? selectedFeatureRelativePath,
        );
        if (pending?.status === "pending") {
          toast.info("Locator proposal ready for review");
          dispatch({ type: "SET_DOCK_TAB", tab: "locator" });
          dispatch({ type: "SET_DOCK_EXPANDED", expanded: true });
        }
      })
      .then((u) => unsubs.push(u));
    void runtime.onEvent<ActiveStep | null>("active-step-changed", (step) => {
      dispatch({ type: "SET_ACTIVE_STEP", step: step ?? null });
      if (!step) {
        return;
      }
      dispatch({ type: "SELECT_SCENARIO", line: step.scenario_line });
      dispatch({ type: "SELECT_STEP", line: step.step_line });
      const root = projectRootRef.current;
      const rel = step.feature_relative_path.replace(/\\/g, "/");
      if (
        root &&
        selectedFeatureRelativePathRef.current !== rel &&
        state.featurePayload?.relative_path !== rel
      ) {
        const abs = `${root.replace(/\\/g, "/").replace(/\/$/, "")}/${rel.replace(/^\.\//, "")}`;
        void openFeature(abs);
      }
      void refreshStepStatuses(step.feature_relative_path);
      dispatch({ type: "SET_DOCK_TAB", tab: "locator" });
      dispatch({ type: "SET_DOCK_EXPANDED", expanded: true });
    }).then((u) => unsubs.push(u));
    void runtime
      .onEvent<FeatureRenderPayload>("feature-refreshed", (payload) => {
        dispatch({ type: "REFRESH_FEATURE", payload });
        void refreshStepStatuses(payload.relative_path);
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
  }, [dispatch, openFeature, openProjectPathWithFeedback, refreshStepStatuses, selectedFeatureRelativePath, state.featurePayload?.relative_path]);

  const syncActiveStep = useCallback(
    async (stepLine: number) => {
      if (!state.selectedFeaturePath) {
        return;
      }
      try {
        const active = await getRuntime().syncActiveStep(
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

  const openFeatureFromTree = async (path: string) => {
    await openFeature(path);
  };

  const startBrowserMode = async (mode: "embedded" | "chrome" | "winapp") => {
    setBrowserError(null);
    setBrowserHint(null);
    try {
      const result = await getRuntime().startBrowserSidecar(mode);
      const sessionMode =
        result.mode === "chrome" || result.mode === "winapp"
          ? result.mode
          : "embedded";
      dispatch({
        type: "SET_BROWSER",
        wsUrl: result.ws_url,
        running: true,
        mode: sessionMode,
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
      dispatch({ type: "SET_BROWSER", wsUrl: null, running: false, mode: null });
    }
  };

  const stopBrowser = async () => {
    await getRuntime().stopBrowserSidecar();
    dispatch({ type: "SET_BROWSER", wsUrl: null, running: false, mode: null });
  };

  // Poll for .teshi/cdp-endpoint.json when browser is not running.
  // This allows `teshi browser serve-embedded` or `--start-embedded` to
  // auto-connect the Desktop/web UI without clicking "Start Embedded".
  // Skipped during E2E self-tests (`?e2e=1`) so locator recording can observe
  // the "Start Embedded" button state rather than jumping to connected mode.
  useEffect(() => {
    if ((window as Window & { __TESHI_E2E__?: boolean }).__TESHI_E2E__) return;
    if (!state.projectRoot || state.browserRunning) return;
    const endpoint = state.projectRoot.replace(/\\/g, "/") + "/.teshi/cdp-endpoint.json";
    const timer = setInterval(async () => {
      try {
        const text = await getRuntime().readTextFile(endpoint);
        const data = JSON.parse(text) as { ws_url?: string; mode?: string };
        if (data.ws_url && data.mode === "embedded") {
          clearInterval(timer);
          // Let the runtime manage the sidecar lifecycle — it will
          // re-adopt or restart the existing embedded browser.
          startBrowserMode("embedded");
        }
      } catch {
        // file not yet written — continue polling
      }
    }, 2000);
    return () => clearInterval(timer);
  }, [state.projectRoot, state.browserRunning, dispatch]);

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
    onOpenProject: () => void pickProject(),
    onOpenRecent: (path: string) => void openProjectPathWithFeedback(path),
  };

  const workspacePanel = (
    <ResizableWorkspace
      browserFullscreen={browserFullscreen}
      projectRoot={state.projectRoot!}
      featurePayload={state.featurePayload}
      stepBindingStatuses={state.stepBindingStatuses}
      selectedScenarioLine={state.selectedScenarioLine}
      selectedStepLine={state.selectedStepLine}
      browserWsUrl={state.browserWsUrl}
      browserRunning={state.browserRunning}
      browserMode={state.browserMode}
      browserError={browserError}
      browserHint={browserHint}
      rightTab={state.rightTab}
      onSelectScenario={(line) => dispatch({ type: "SELECT_SCENARIO", line })}
      onSelectStep={(line) => {
        dispatch({ type: "SELECT_STEP", line });
        if (line !== null) {
          void syncActiveStep(line);
        }
      }}
      onConnectChrome={() => void startBrowserMode("chrome")}
      onConnectWinApp={() => void startBrowserMode("winapp")}
      onStartEmbedded={() => void startBrowserMode("embedded")}
      onStopBrowser={() => void stopBrowser()}
      onToggleBrowserFullscreen={() =>
        dispatch({
          type: "SET_LAYOUT_MODE",
          mode: browserFullscreen ? "normal" : "browserFullscreen",
        })
      }
      onRightTabChange={(tab) => dispatch({ type: "SET_TAB", tab })}
      onOpenFeature={(path) => void openFeatureFromTree(path)}
    />
  );

  return (
    <div className="app-shell">
      {state.projectRoot && (
        <button
          className="mode-toggle"
          onClick={() => setShowRequirements((prev) => !prev)}
          title={showRequirements ? "Switch to Workspace" : "Switch to Requirements"}
        >
          {showRequirements ? "🔍 Workspace" : "📋 Requirements"}
        </button>
      )}
      {!state.projectRoot ? (
        <>
          <AppChrome {...chromeProps} />
          <WelcomeScreen
            recentProjects={state.recentProjects}
            onOpenProject={() => void pickProject()}
            onOpenRecent={(path) => void openProjectPathWithFeedback(path)}
          />
        </>
      ) : (
        <>
          <div style={{ display: showRequirements ? "flex" : "none", flex: 1, flexDirection: "column" }}>
            <RequirementsPage />
          </div>
          <div style={{ display: showRequirements ? "none" : "flex", flex: 1, flexDirection: "column" }}>
            <AppChrome {...chromeProps} />
            {browserFullscreen ? (
              <div className="workspace">{workspacePanel}</div>
            ) : (
              <Group
                key={state.projectRoot}
                id="teshi-workspace-dock-layout"
                orientation="vertical"
                className="workspace"
                groupRef={dockGroupRef}
                defaultLayout={defaultDockLayout}
                onLayoutChanged={scheduleDockPersist}
              >
                <Panel id="main" defaultSize={defaultDockLayout.main} minSize={200}>
                  {workspacePanel}
                </Panel>
                <Separator className="resize-handle resize-handle--horizontal" />
                <Panel
                  id="dock"
                  collapsible
                  collapsedSize="33px"
                  panelRef={dockPanelRef}
                  defaultSize={defaultDockLayout.dock}
                  minSize={120}
                >
                  <BottomDock
                    expanded={state.dockExpanded}
                    activeTab={state.dockActiveTab}
                    activeStep={state.activeStep}
                    pendingLocator={state.pendingLocator}
                    stepBindingStatuses={state.stepBindingStatuses}
                    projectRoot={state.projectRoot}
                    onToggle={() => dispatch({ type: "TOGGLE_DOCK" })}
                    onTabChange={(tab) => dispatch({ type: "SET_DOCK_TAB", tab })}
                    onPendingChange={(pending) => {
                      dispatch({ type: "SET_PENDING_LOCATOR", pending });
                      void refreshStepStatuses(selectedFeatureRelativePath);
                    }}
                    onBindingChanged={() => {
                      void refreshStepStatuses(selectedFeatureRelativePath);
                    }}
                  />
                </Panel>
              </Group>
            )}
          </div>
        </>
      )}
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
