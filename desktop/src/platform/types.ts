import type { FeatureRenderPayload } from "../types";
import type { ActiveStep, PendingLocator } from "../locatorTypes";

/** Platform-agnostic runtime API (Tauri invoke or `teshi web` HTTP). */
export interface TeshiRuntimeApi {
  checkProjectSwitchAllowed(): Promise<boolean>;
  teardownRuntime(): Promise<void>;
  openProject(path: string): Promise<void>;
  getRecentProjects(): Promise<string[]>;
  openProjectDir(): Promise<string | null>;
  getPendingLocator(): Promise<PendingLocator | null>;
  getActiveStep(): Promise<ActiveStep | null>;
  syncActiveStep(featurePath: string, stepLine: number): Promise<ActiveStep>;
  renderFeature(path: string): Promise<FeatureRenderPayload>;
  startBrowserSidecar(
    mode: "embedded" | "chrome",
  ): Promise<{ ws_url: string; mode: string; cdp_endpoint_path?: string }>;
  stopBrowserSidecar(): Promise<void>;
  listDir(path: string): Promise<import("../types").DirEntry[]>;
  spawnTerminal(cols: number, rows: number): Promise<void>;
  stopTerminal(): Promise<void>;
  resizeTerminal(cols: number, rows: number): Promise<void>;
  writeTerminal(data: string): Promise<void>;
  confirmLocator(candidateRank: number, editedValue: string | null): Promise<void>;
  rejectLocator(): Promise<void>;
  confirmStopRuntimeIfBusy(): Promise<boolean>;
  finalizeMainWindow?(): Promise<void>;
  onEvent<T>(event: string, handler: (payload: T) => void): Promise<() => void>;
}
