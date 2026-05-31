import type { TeshiRuntimeApi } from "./types";
import { tauriRuntime } from "./tauri";
import { webRuntime } from "./web";

/** True when running inside the Tauri webview. */
export function isTauriHost(): boolean {
  return (
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window ||
      "__TAURI__" in window)
  );
}

/** Active platform runtime implementation. */
export function getRuntime(): TeshiRuntimeApi {
  return isTauriHost() ? tauriRuntime : webRuntime;
}

export type { TeshiRuntimeApi } from "./types";
