import { useCallback, useEffect, useRef, useState } from "react";
import {
  Group,
  Panel,
  Separator,
  usePanelRef,
} from "react-resizable-panels";
import { GherkinPanel } from "./GherkinPanel";
import { BrowserPanel } from "./BrowserPanel";
import { FileTreeTerminalPanel } from "./FileTreeTerminalPanel";
import type { FeatureRenderPayload } from "../types";

interface Props {
  browserFullscreen: boolean;
  projectRoot: string;
  featurePayload: FeatureRenderPayload | null;
  selectedScenarioLine: number | null;
  selectedStepLine: number | null;
  browserWsUrl: string | null;
  browserRunning: boolean;
  browserError: string | null;
  browserHint: string | null;
  rightTab: "files" | "terminal";
  onSelectScenario: (line: number) => void;
  onSelectStep: (line: number) => void;
  onStartBrowser: () => void;
  onStopBrowser: () => void;
  onToggleBrowserFullscreen: () => void;
  onRightTabChange: (tab: "files" | "terminal") => void;
  onOpenFeature: (path: string) => void;
}

export function ResizableWorkspace({
  browserFullscreen,
  projectRoot,
  featurePayload,
  selectedScenarioLine,
  selectedStepLine,
  browserWsUrl,
  browserRunning,
  browserError,
  browserHint,
  rightTab,
  onSelectScenario,
  onSelectStep,
  onStartBrowser,
  onStopBrowser,
  onToggleBrowserFullscreen,
  onRightTabChange,
  onOpenFeature,
}: Props) {
  const gherkinRef = usePanelRef();
  const filesRef = usePanelRef();
  const [gherkinCollapsed, setGherkinCollapsed] = useState(false);
  const [filesCollapsed, setFilesCollapsed] = useState(false);
  const prevFullscreenRef = useRef(browserFullscreen);

  const toggleGherkin = useCallback(() => {
    const panel = gherkinRef.current;
    if (!panel) return;
    if (panel.isCollapsed()) {
      panel.expand();
      setGherkinCollapsed(false);
    } else {
      panel.collapse();
      setGherkinCollapsed(true);
    }
  }, [gherkinRef]);

  const toggleFiles = useCallback(() => {
    const panel = filesRef.current;
    if (!panel) return;
    if (panel.isCollapsed()) {
      panel.expand();
      setFilesCollapsed(false);
    } else {
      panel.collapse();
      setFilesCollapsed(true);
    }
  }, [filesRef]);

  const handleGherkinResize = useCallback(() => {
    setGherkinCollapsed(gherkinRef.current?.isCollapsed() ?? false);
  }, [gherkinRef]);

  const handleFilesResize = useCallback(() => {
    setFilesCollapsed(filesRef.current?.isCollapsed() ?? false);
  }, [filesRef]);

  // New project: restore default three-column layout.
  useEffect(() => {
    gherkinRef.current?.expand();
    filesRef.current?.expand();
    setGherkinCollapsed(false);
    setFilesCollapsed(false);
  }, [projectRoot, gherkinRef, filesRef]);

  // Fullscreen unmounts the resizable group; re-apply collapse after exit.
  useEffect(() => {
    const wasFullscreen = prevFullscreenRef.current;
    prevFullscreenRef.current = browserFullscreen;

    if (wasFullscreen && !browserFullscreen) {
      requestAnimationFrame(() => {
        if (gherkinCollapsed) {
          gherkinRef.current?.collapse();
        }
        if (filesCollapsed) {
          filesRef.current?.collapse();
        }
      });
    }
  }, [browserFullscreen, gherkinCollapsed, filesCollapsed, gherkinRef, filesRef]);

  const browserPanel = (
    <BrowserPanel
      wsUrl={browserWsUrl}
      running={browserRunning}
      error={browserError}
      hint={browserHint}
      fullscreen={browserFullscreen}
      gherkinCollapsed={gherkinCollapsed}
      filesCollapsed={filesCollapsed}
      onToggleGherkin={toggleGherkin}
      onToggleFiles={toggleFiles}
      onStart={onStartBrowser}
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
      id="teshi-main-layout"
      orientation="horizontal"
      className="layout"
    >
      <Panel
        id="gherkin"
        collapsible
        panelRef={gherkinRef}
        defaultSize={30}
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
      <Panel id="browser" defaultSize={45} minSize={200}>
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
        defaultSize={25}
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
