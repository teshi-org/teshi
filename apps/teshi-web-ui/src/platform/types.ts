import type { FeatureRenderPayload } from "../types";
import type { ActiveStep, PendingLocator, StepBindingStatus } from "../locatorTypes";

/** Platform-agnostic runtime API (Tauri invoke or `teshi web` HTTP). */
export interface TeshiRuntimeApi {
  checkProjectSwitchAllowed(): Promise<boolean>;
  teardownRuntime(): Promise<void>;
  openProject(path: string): Promise<void>;
  getRecentProjects(): Promise<string[]>;
  openProjectDir(): Promise<string | null>;
  getPendingLocator(): Promise<PendingLocator | null>;
  getStepBindingStatuses(featurePath: string): Promise<StepBindingStatus[]>;
  getActiveStep(): Promise<ActiveStep | null>;
  syncActiveStep(featurePath: string, stepLine: number): Promise<ActiveStep>;
  renderFeature(path: string): Promise<FeatureRenderPayload>;
  startBrowserSidecar(
    mode: "embedded" | "chrome" | "winapp",
  ): Promise<{ ws_url: string; mode: string; cdp_endpoint_path?: string }>;
  stopBrowserSidecar(): Promise<void>;
  listDir(path: string): Promise<import("../types").DirEntry[]>;
  spawnTerminal(cols: number, rows: number): Promise<void>;
  stopTerminal(): Promise<void>;
  resizeTerminal(cols: number, rows: number): Promise<void>;
  writeTerminal(data: string): Promise<void>;
  highlightLocator(selector: string): Promise<void>;
  confirmLocator(candidateRank: number, editedValue: string | null): Promise<void>;
  rejectLocator(): Promise<void>;
  unbindStep(featurePath: string, stepLine: number): Promise<void>;
  getProjectSettings(): Promise<{ locator_auto_confirm_sec: number }>;
  confirmStopRuntimeIfBusy(): Promise<boolean>;
  onEvent<T>(event: string, handler: (payload: T) => void): Promise<() => void>;
  readTextFile(path: string): Promise<string>;
  readFileAsDataUrl(path: string): Promise<string>;
}
