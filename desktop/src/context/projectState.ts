import type { FeatureRenderPayload } from "../types";

export interface ProjectState {
  projectRoot: string | null;
  selectedFeaturePath: string | null;
  featurePayload: FeatureRenderPayload | null;
  selectedScenarioLine: number | null;
  selectedStepLine: number | null;
  recentProjects: string[];
  browserWsUrl: string | null;
  browserRunning: boolean;
  rightTab: "files" | "terminal";
}

export type ProjectAction =
  | { type: "SET_PROJECT"; root: string }
  | { type: "SET_RECENT"; paths: string[] }
  | { type: "SET_FEATURE"; path: string; payload: FeatureRenderPayload }
  | { type: "REFRESH_FEATURE"; payload: FeatureRenderPayload }
  | { type: "SELECT_SCENARIO"; line: number | null }
  | { type: "SELECT_STEP"; line: number | null }
  | { type: "SET_BROWSER"; wsUrl: string | null; running: boolean }
  | { type: "SET_TAB"; tab: "files" | "terminal" }
  | { type: "CLOSE_PROJECT" };

export const initialProjectState: ProjectState = {
  projectRoot: null,
  selectedFeaturePath: null,
  featurePayload: null,
  selectedScenarioLine: null,
  selectedStepLine: null,
  recentProjects: [],
  browserWsUrl: null,
  browserRunning: false,
  rightTab: "files",
};

export function projectReducer(
  state: ProjectState,
  action: ProjectAction,
): ProjectState {
  switch (action.type) {
    case "SET_PROJECT":
      return {
        ...initialProjectState,
        projectRoot: action.root,
        recentProjects: state.recentProjects,
      };
    case "SET_RECENT":
      return { ...state, recentProjects: action.paths };
    case "SET_FEATURE":
      return {
        ...state,
        selectedFeaturePath: action.path,
        featurePayload: action.payload,
        selectedScenarioLine: null,
        selectedStepLine: null,
      };
    case "REFRESH_FEATURE":
      return { ...state, featurePayload: action.payload };
    case "SELECT_SCENARIO":
      return {
        ...state,
        selectedScenarioLine: action.line,
        selectedStepLine: null,
      };
    case "SELECT_STEP":
      return { ...state, selectedStepLine: action.line };
    case "SET_BROWSER":
      return {
        ...state,
        browserWsUrl: action.wsUrl,
        browserRunning: action.running,
      };
    case "SET_TAB":
      return { ...state, rightTab: action.tab };
    case "CLOSE_PROJECT":
      return { ...initialProjectState, recentProjects: state.recentProjects };
    default:
      return state;
  }
}
