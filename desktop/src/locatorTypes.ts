export interface ActiveStep {
  feature_relative_path: string;
  scenario_line: number;
  scenario_name: string;
  step_line: number;
  step_keyword: string;
  step_text: string;
  updated_at: string;
}

export interface LocatorCandidate {
  rank: number;
  strategy: string;
  value: string;
  action: string;
  confidence: number;
  rationale: string;
}

export interface PendingLocator {
  step_ref: ActiveStep;
  candidates: LocatorCandidate[];
  highlight?: {
    candidate_rank: number;
    applied: boolean;
  };
  status: string;
}

export interface StepBindingStatus {
  step_line: number;
  step_text_normalized: string;
  status: "confirmed" | "pending" | string;
  source: "binding" | "pending" | "script" | string;
}
