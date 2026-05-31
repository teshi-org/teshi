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
import type { BrowserError, FeatureRenderPayload } from "./types";

function AppShell() {
  const { state, dispatch } = useProject();
  const [browserError, setBrowserError] = useState<string | null>(null);
  const [browserHint, setBrowserHint] = useState<string | null>(null);

  const openProjectPath = useCallback(
    async (path: string) => {
      const ok = await invoke<boolean>("confirm_teardown");
      if (!ok) return;
      await invoke("teardown_runtime");
      await invoke("open_project", { path });
      dispatch({ type: "SET_PROJECT", root: path });
      setBrowserError(null);
      setBrowserHint(null);
      dispatch({ type: "SET_BROWSER", wsUrl: null, running: false });
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

    void getCurrentWindow().onCloseRequested(async (event) => {
      const ok = await invoke<boolean>("confirm_teardown");
      if (!ok) {
        event.preventDefault();
        return;
      }
      await invoke("teardown_runtime");
    });

    return () => {
      unsubs.forEach((u) => u());
      window.removeEventListener("keydown", onKey);
    };
  }, [dispatch, openProjectPath, pickProject]);

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

  return (
    <div className="app-shell">
      <header className="toolbar">
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
      </header>
      <main className="layout">
        <GherkinPanel
          relativePath={state.featurePayload?.relative_path ?? null}
          payload={state.featurePayload}
          selectedScenarioLine={state.selectedScenarioLine}
          selectedStepLine={state.selectedStepLine}
          onSelectScenario={(line) =>
            dispatch({ type: "SELECT_SCENARIO", line })
          }
          onSelectStep={(line) => dispatch({ type: "SELECT_STEP", line })}
        />
        <BrowserPanel
          wsUrl={state.browserWsUrl}
          running={state.browserRunning}
          error={browserError}
          hint={browserHint}
          onStart={() => void startBrowser()}
          onStop={() => void stopBrowser()}
        />
        <FileTreeTerminalPanel
          projectRoot={state.projectRoot}
          tab={state.rightTab}
          onTabChange={(tab) => dispatch({ type: "SET_TAB", tab })}
          onOpenFeature={(path) => void openFeature(path)}
        />
      </main>
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
