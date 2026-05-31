import {
  Group,
  Panel,
  Separator,
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
  const browserPanel = (
    <BrowserPanel
      wsUrl={browserWsUrl}
      running={browserRunning}
      error={browserError}
      hint={browserHint}
      fullscreen={browserFullscreen}
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
      <Panel id="gherkin" defaultSize={30} minSize={200}>
        <div className="layout-panel-shell">
          <GherkinPanel
            relativePath={featurePayload?.relative_path ?? null}
            payload={featurePayload}
            selectedScenarioLine={selectedScenarioLine}
            selectedStepLine={selectedStepLine}
            onSelectScenario={onSelectScenario}
            onSelectStep={onSelectStep}
          />
        </div>
      </Panel>
      <Separator className="resize-handle" />
      <Panel id="browser" defaultSize={45} minSize={200}>
        <div className="layout-panel-shell">{browserPanel}</div>
      </Panel>
      <Separator className="resize-handle" />
      <Panel id="files" defaultSize={25} minSize={200}>
        <div className="layout-panel-shell">
          <FileTreeTerminalPanel
            projectRoot={projectRoot}
            tab={rightTab}
            onTabChange={onRightTabChange}
            onOpenFeature={onOpenFeature}
          />
        </div>
      </Panel>
    </Group>
  );
}
