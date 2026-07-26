import type { TeshiRuntimeApi } from "./types";
import { webRuntime } from "./web";

/**
 * Active platform runtime. The React UI is served by `teshi web` (HTTP daemon).
 */
export function getRuntime(): TeshiRuntimeApi {
  return webRuntime;
}

export type { TeshiRuntimeApi } from "./types";
