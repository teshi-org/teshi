import { isTauri } from "@tauri-apps/api/core";

import type { TeshiRuntimeApi } from "./types";
import { tauriRuntime } from "./tauri";
import { webRuntime } from "./web";

/** True when running inside the Tauri webview. */
export function isTauriHost(): boolean {
  if (typeof window === "undefined") {
    return false;
  }
  // Prefer the official flag; fall back while the webview is still starting.
  return (
    isTauri() ||
    "__TAURI_INTERNALS__" in window ||
    "__TAURI__" in window
  );
}

/**
 * Active platform runtime. Call at use time — do not cache at module scope:
 * the same bundle is served by `teshi web` (HTTP) and the Tauri shell (invoke).
 */
export function getRuntime(): TeshiRuntimeApi {
  return isTauriHost() ? tauriRuntime : webRuntime;
}

export type { TeshiRuntimeApi } from "./types";
