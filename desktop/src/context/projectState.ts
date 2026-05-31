import type { FeatureRenderPayload } from "../types";

export type LayoutMode = "normal" | "browserFocus";
export type DockTab = "output" | "logs";

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
  layoutMode: LayoutMode;
  dockExpanded: boolean;
  dockActiveTab: DockTab;
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
  | { type: "SET_LAYOUT_MODE"; mode: LayoutMode }
  | { type: "TOGGLE_DOCK" }
  | { type: "SET_DOCK_TAB"; tab: DockTab }
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
  layoutMode: "normal",
  dockExpanded: false,
  dockActiveTab: "output",
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
    case "SET_LAYOUT_MODE":
      return { ...state, layoutMode: action.mode };
    case "TOGGLE_DOCK":
      return { ...state, dockExpanded: !state.dockExpanded };
    case "SET_DOCK_TAB":
      return {
        ...state,
        dockActiveTab: action.tab,
        dockExpanded: true,
      };
    case "CLOSE_PROJECT":
      return { ...initialProjectState, recentProjects: state.recentProjects };
    default:
      return state;
  }
}
