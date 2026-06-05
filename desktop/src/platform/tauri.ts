import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { FeatureRenderPayload } from "../types";
import type { ActiveStep, PendingLocator, StepBindingStatus } from "../locatorTypes";
import type { DirEntry } from "../types";
import type { TeshiRuntimeApi } from "./types";

const terminalExclusiveUnsubs = new Map<string, () => void>();

/** Tauri desktop host using `invoke` and `listen`. */
export const tauriRuntime: TeshiRuntimeApi = {
  async checkProjectSwitchAllowed() {
    return invoke<boolean>("check_project_switch_allowed_cmd");
  },

  async teardownRuntime() {
    await invoke("teardown_runtime");
  },

  async openProject(path: string) {
    await invoke("open_project", { path });
  },

  async getRecentProjects() {
    return invoke<string[]>("get_recent_projects_cmd");
  },

  async openProjectDir() {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({
      directory: true,
      multiple: false,
      title: "Open Project",
    });
    if (picked === null) {
      return null;
    }
    return Array.isArray(picked) ? (picked[0] ?? null) : picked;
  },

  async getPendingLocator() {
    return invoke<PendingLocator | null>("get_pending_locator_cmd");
  },

  async getStepBindingStatuses(featurePath: string) {
    return invoke<StepBindingStatus[]>("get_step_binding_statuses_cmd", {
      featurePath,
    });
  },

  async getActiveStep() {
    return invoke<ActiveStep | null>("get_active_step_cmd");
  },

  async syncActiveStep(featurePath: string, stepLine: number) {
    return invoke<ActiveStep>("sync_active_step_cmd", {
      featurePath,
      stepLine,
    });
  },

  async renderFeature(path: string) {
    return invoke<FeatureRenderPayload>("render_feature_cmd", { path });
  },

  async startBrowserSidecar(mode: "embedded" | "chrome" | "winapp") {
    return invoke<{ ws_url: string; mode: string; cdp_endpoint_path: string }>(
      "start_browser_sidecar",
      { mode },
    );
  },

  async stopBrowserSidecar() {
    await invoke("stop_browser_sidecar");
  },

  async listDir(path: string) {
    return invoke<DirEntry[]>("list_dir", { path });
  },

  async spawnTerminal(cols: number, rows: number) {
    await invoke("spawn_terminal", { cols, rows });
  },

  async stopTerminal() {
    await invoke("stop_terminal");
  },

  async resizeTerminal(cols: number, rows: number) {
    await invoke("resize_terminal", { cols, rows });
  },

  async writeTerminal(data: string) {
    await invoke("write_terminal", { data });
  },

  async highlightLocator(selector: string) {
    await invoke("highlight_locator_cmd", { selector });
  },

  async confirmLocator(candidateRank: number, editedValue: string | null) {
    await invoke("confirm_locator_cmd", {
      candidateRank,
      editedValue,
    });
  },

  async rejectLocator() {
    await invoke("reject_locator_cmd");
  },

  async unbindStep(featurePath: string, stepLine: number) {
    await invoke("unbind_step_cmd", { featurePath, stepLine });
  },

  async getProjectSettings() {
    return invoke<{ locator_auto_confirm_sec: number }>("get_project_settings_cmd");
  },

  async confirmStopRuntimeIfBusy() {
    return invoke<boolean>("confirm_teardown");
  },

  async finalizeMainWindow() {
    await invoke("finalize_main_window_cmd");
  },

  async onEvent<T>(event: string, handler: (payload: T) => void) {
    if (event === "terminal-output" || event === "terminal-exit") {
      terminalExclusiveUnsubs.get(event)?.();
      terminalExclusiveUnsubs.delete(event);
    }
    const unlisten = await listen<T>(event, (e) => handler(e.payload));
    const wrapped = () => {
      unlisten();
      if (terminalExclusiveUnsubs.get(event) === wrapped) {
        terminalExclusiveUnsubs.delete(event);
      }
    };
    if (event === "terminal-output" || event === "terminal-exit") {
      terminalExclusiveUnsubs.set(event, wrapped);
    }
    return wrapped;
  },
};
