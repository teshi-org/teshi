export interface HighlightSpan {
  text: string;
  kind: string;
}

export interface RenderStep {
  line_number: number;
  keyword: string;
  text: string;
  keyword_kind: string;
}

export interface RenderScenario {
  name: string;
  kind: string;
  tags: string[];
  line_number: number;
  steps: RenderStep[];
  examples: unknown[];
}

export type RenderBlock =
  | { type: "feature_header"; name: string; tags: string[]; language: string }
  | { type: "background"; steps: RenderStep[] }
  | { type: "scenario"; name: string; kind: string; tags: string[]; line_number: number; steps: RenderStep[]; examples: unknown[] };

export interface RenderLine {
  line_number: number;
  spans: HighlightSpan[];
}

export interface FeatureRenderPayload {
  path: string;
  relative_path: string;
  structured: RenderBlock[];
  raw_lines: RenderLine[];
  error: { message: string; line_number?: number } | null;
}

export interface DirEntry {
  name: string;
  path: string;
  is_dir: boolean;
  is_feature: boolean;
}

export interface BrowserError {
  message: string;
  hint?: string;
}
