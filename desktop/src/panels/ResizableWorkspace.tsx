import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Group,
  Panel,
  Separator,
  useGroupRef,
  usePanelRef,
  type Layout,
} from "react-resizable-panels";
import { GherkinPanel } from "./GherkinPanel";
import { BrowserPanel } from "./BrowserPanel";
import { FileTreeTerminalPanel } from "./FileTreeTerminalPanel";
import type { FeatureRenderPayload } from "../types";
import type { BrowserSessionMode } from "../context/projectState";
import {
  defaultLayoutForProject,
  loadWorkspaceLayout,
  panelLayoutFromGroupLayout,
  saveWorkspaceLayout,
} from "../layout/workspaceLayoutStorage";

interface Props {
  browserFullscreen: boolean;
  projectRoot: string;
  featurePayload: FeatureRenderPayload | null;
  selectedScenarioLine: number | null;
  selectedStepLine: number | null;
  browserWsUrl: string | null;
  browserRunning: boolean;
  browserMode: BrowserSessionMode | null;
  browserError: string | null;
  browserHint: string | null;
  rightTab: "files" | "terminal";
  onSelectScenario: (line: number) => void;
  onSelectStep: (line: number) => void;
  onConnectChrome: () => void;
  onStartEmbedded: () => void;
  onStopBrowser: () => void;
  onToggleBrowserFullscreen: () => void;
  onRightTabChange: (tab: "files" | "terminal") => void;
  onOpenFeature: (path: string) => void;
}

const SAVE_DEBOUNCE_MS = 150;

function applyPanelCollapse(
  panelRef: ReturnType<typeof usePanelRef>,
  collapsed: boolean,
) {
  if (collapsed) {
    panelRef.current?.collapse();
  } else {
    panelRef.current?.expand();
  }
}

export function ResizableWorkspace({
  browserFullscreen,
  projectRoot,
  featurePayload,
  selectedScenarioLine,
  selectedStepLine,
  browserWsUrl,
  browserRunning,
  browserMode,
  browserError,
  browserHint,
  rightTab,
  onSelectScenario,
  onSelectStep,
  onConnectChrome,
  onStartEmbedded,
  onStopBrowser,
  onToggleBrowserFullscreen,
  onRightTabChange,
  onOpenFeature,
}: Props) {
  const groupRef = useGroupRef();
  const gherkinRef = usePanelRef();
  const filesRef = usePanelRef();
  const savedForProject = useMemo(
    () => loadWorkspaceLayout(projectRoot),
    [projectRoot],
  );
  const [gherkinCollapsed, setGherkinCollapsed] = useState(
    () => savedForProject?.gherkinCollapsed ?? false,
  );
  const [filesCollapsed, setFilesCollapsed] = useState(
    () => savedForProject?.filesCollapsed ?? false,
  );
  const gherkinCollapsedRef = useRef(gherkinCollapsed);
  const filesCollapsedRef = useRef(filesCollapsed);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const prevFullscreenRef = useRef(browserFullscreen);

  const defaultLayout = useMemo(
    () => defaultLayoutForProject(projectRoot),
    [projectRoot],
  );

  useEffect(() => {
    gherkinCollapsedRef.current = gherkinCollapsed;
  }, [gherkinCollapsed]);

  useEffect(() => {
    filesCollapsedRef.current = filesCollapsed;
  }, [filesCollapsed]);

  const persistLayout = useCallback(
    (layout: Layout) => {
      const panelLayout = panelLayoutFromGroupLayout(layout);
      if (!panelLayout) {
        return;
      }
      saveWorkspaceLayout(projectRoot, {
        layout: panelLayout,
        gherkinCollapsed: gherkinCollapsedRef.current,
        filesCollapsed: filesCollapsedRef.current,
      });
    },
    [projectRoot],
  );

  const schedulePersist = useCallback(
    (layout: Layout) => {
      if (saveTimerRef.current) {
        clearTimeout(saveTimerRef.current);
      }
      saveTimerRef.current = setTimeout(() => {
        persistLayout(layout);
        saveTimerRef.current = null;
      }, SAVE_DEBOUNCE_MS);
    },
    [persistLayout],
  );

  const persistCurrentLayout = useCallback(() => {
    const layout = groupRef.current?.getLayout();
    if (layout) {
      persistLayout(layout);
    }
  }, [groupRef, persistLayout]);

  const onLayoutChanged = useCallback(
    (layout: Layout) => {
      schedulePersist(layout);
    },
    [schedulePersist],
  );

  // Restore or reset side-panel collapse when the opened project changes.
  useEffect(() => {
    const saved = loadWorkspaceLayout(projectRoot);
    setGherkinCollapsed(saved?.gherkinCollapsed ?? false);
    setFilesCollapsed(saved?.filesCollapsed ?? false);

    requestAnimationFrame(() => {
      if (saved) {
        applyPanelCollapse(gherkinRef, saved.gherkinCollapsed);
        applyPanelCollapse(filesRef, saved.filesCollapsed);
      } else {
        gherkinRef.current?.expand();
        filesRef.current?.expand();
      }
    });
  }, [projectRoot, gherkinRef, filesRef]);

  useEffect(
    () => () => {
      if (saveTimerRef.current) {
        clearTimeout(saveTimerRef.current);
      }
    },
    [],
  );

  const toggleGherkin = useCallback(() => {
    const panel = gherkinRef.current;
    if (!panel) return;
    if (panel.isCollapsed()) {
      panel.expand();
      setGherkinCollapsed(false);
      gherkinCollapsedRef.current = false;
    } else {
      panel.collapse();
      setGherkinCollapsed(true);
      gherkinCollapsedRef.current = true;
    }
    persistCurrentLayout();
  }, [gherkinRef, persistCurrentLayout]);

  const toggleFiles = useCallback(() => {
    const panel = filesRef.current;
    if (!panel) return;
    if (panel.isCollapsed()) {
      panel.expand();
      setFilesCollapsed(false);
      filesCollapsedRef.current = false;
    } else {
      panel.collapse();
      setFilesCollapsed(true);
      filesCollapsedRef.current = true;
    }
    persistCurrentLayout();
  }, [filesRef, persistCurrentLayout]);

  const handleGherkinResize = useCallback(() => {
    const collapsed = gherkinRef.current?.isCollapsed() ?? false;
    setGherkinCollapsed(collapsed);
    gherkinCollapsedRef.current = collapsed;
    const layout = groupRef.current?.getLayout();
    if (layout) {
      schedulePersist(layout);
    }
  }, [gherkinRef, groupRef, schedulePersist]);

  const handleFilesResize = useCallback(() => {
    const collapsed = filesRef.current?.isCollapsed() ?? false;
    setFilesCollapsed(collapsed);
    filesCollapsedRef.current = collapsed;
    const layout = groupRef.current?.getLayout();
    if (layout) {
      schedulePersist(layout);
    }
  }, [filesRef, groupRef, schedulePersist]);

  // Fullscreen unmounts the resizable group; re-apply collapse after exit.
  useEffect(() => {
    const wasFullscreen = prevFullscreenRef.current;
    prevFullscreenRef.current = browserFullscreen;

    if (wasFullscreen && !browserFullscreen) {
      requestAnimationFrame(() => {
        applyPanelCollapse(gherkinRef, gherkinCollapsed);
        applyPanelCollapse(filesRef, filesCollapsed);
      });
    }
  }, [browserFullscreen, gherkinCollapsed, filesCollapsed, gherkinRef, filesRef]);

  const browserPanel = (
    <BrowserPanel
      wsUrl={browserWsUrl}
      running={browserRunning}
      mode={browserMode}
      error={browserError}
      hint={browserHint}
      fullscreen={browserFullscreen}
      gherkinCollapsed={gherkinCollapsed}
      filesCollapsed={filesCollapsed}
      onToggleGherkin={toggleGherkin}
      onToggleFiles={toggleFiles}
      onConnectChrome={onConnectChrome}
      onStartEmbedded={onStartEmbedded}
      onStop={onStopBrowser}
      onToggleFullscreen={onToggleBrowserFullscreen}
    />
  );

  if (browserFullscreen) {
    return (
      <main className="layout layout--browser-fullscreen">
        <div className="layout-panel-shell">{browserPanel}</div>
        <GherkinPanel
          layoutHidden
          relativePath={featurePayload?.relative_path ?? null}
          payload={featurePayload}
          selectedScenarioLine={selectedScenarioLine}
          selectedStepLine={selectedStepLine}
          onSelectScenario={onSelectScenario}
          onSelectStep={onSelectStep}
        />
        <FileTreeTerminalPanel
          layoutHidden
          projectRoot={projectRoot}
          tab={rightTab}
          onTabChange={onRightTabChange}
          onOpenFeature={onOpenFeature}
        />
      </main>
    );
  }

  return (
    <Group
      key={projectRoot}
      id="teshi-main-layout"
      orientation="horizontal"
      className="layout"
      groupRef={groupRef}
      defaultLayout={defaultLayout}
      onLayoutChanged={onLayoutChanged}
    >
      <Panel
        id="gherkin"
        collapsible
        panelRef={gherkinRef}
        defaultSize={defaultLayout.gherkin}
        minSize={200}
        onResize={handleGherkinResize}
      >
        <div className="layout-panel-shell">
          <GherkinPanel
            relativePath={featurePayload?.relative_path ?? null}
            payload={featurePayload}
            selectedScenarioLine={selectedScenarioLine}
            selectedStepLine={selectedStepLine}
            onSelectScenario={onSelectScenario}
            onSelectStep={onSelectStep}
            showCollapseButton
            onToggleCollapse={toggleGherkin}
          />
        </div>
      </Panel>
      <Separator
        className={`resize-handle${gherkinCollapsed ? " resize-handle--hidden" : ""}`}
        disabled={gherkinCollapsed}
      />
      <Panel id="browser" defaultSize={defaultLayout.browser} minSize={200}>
        <div className="layout-panel-shell">{browserPanel}</div>
      </Panel>
      <Separator
        className={`resize-handle${filesCollapsed ? " resize-handle--hidden" : ""}`}
        disabled={filesCollapsed}
      />
      <Panel
        id="files"
        collapsible
        panelRef={filesRef}
        defaultSize={defaultLayout.files}
        minSize={200}
        onResize={handleFilesResize}
      >
        <div className="layout-panel-shell">
          <FileTreeTerminalPanel
            projectRoot={projectRoot}
            tab={rightTab}
            onTabChange={onRightTabChange}
            onOpenFeature={onOpenFeature}
            layoutCollapsed={filesCollapsed}
            showCollapseButton
            onToggleCollapse={toggleFiles}
          />
        </div>
      </Panel>
    </Group>
  );
}
