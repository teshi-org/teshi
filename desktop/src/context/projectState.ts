import type { FeatureRenderPayload } from "../types";
import type { ActiveStep, PendingLocator, StepBindingStatus } from "../locatorTypes";

export type LayoutMode = "normal" | "browserFullscreen";
export type DockTab = "locator" | "output" | "logs";
export type BrowserSessionMode = "embedded" | "chrome";

export interface ProjectState {
  projectRoot: string | null;
  selectedFeaturePath: string | null;
  featurePayload: FeatureRenderPayload | null;
  selectedScenarioLine: number | null;
  selectedStepLine: number | null;
  activeStep: ActiveStep | null;
  pendingLocator: PendingLocator | null;
  stepBindingStatuses: Record<number, StepBindingStatus>;
  recentProjects: string[];
  browserWsUrl: string | null;
  browserRunning: boolean;
  browserMode: BrowserSessionMode | null;
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
  | { type: "SET_ACTIVE_STEP"; step: ActiveStep | null }
  | { type: "SET_PENDING_LOCATOR"; pending: PendingLocator | null }
  | { type: "SET_STEP_BINDING_STATUSES"; statuses: StepBindingStatus[] }
  | {
      type: "SET_BROWSER";
      wsUrl: string | null;
      running: boolean;
      mode: BrowserSessionMode | null;
    }
  | { type: "SET_TAB"; tab: "files" | "terminal" }
  | { type: "SET_LAYOUT_MODE"; mode: LayoutMode }
  | { type: "TOGGLE_DOCK" }
  | { type: "SET_DOCK_TAB"; tab: DockTab }
  | { type: "SET_DOCK_EXPANDED"; expanded: boolean }
  | { type: "RESTORE_DOCK"; expanded: boolean; activeTab: DockTab }
  | { type: "CLOSE_PROJECT" };

export const initialProjectState: ProjectState = {
  projectRoot: null,
  selectedFeaturePath: null,
  featurePayload: null,
  selectedScenarioLine: null,
  selectedStepLine: null,
  activeStep: null,
  pendingLocator: null,
  stepBindingStatuses: {},
  recentProjects: [],
  browserWsUrl: null,
  browserRunning: false,
  browserMode: null,
  rightTab: "files",
  layoutMode: "normal",
  dockExpanded: false,
  dockActiveTab: "locator",
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
        activeStep: null,
        pendingLocator: null,
        stepBindingStatuses:
          state.featurePayload?.relative_path === action.payload.relative_path
            ? state.stepBindingStatuses
            : {},
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
    case "SET_ACTIVE_STEP":
      return { ...state, activeStep: action.step };
    case "SET_PENDING_LOCATOR":
      return { ...state, pendingLocator: action.pending };
    case "SET_STEP_BINDING_STATUSES":
      return {
        ...state,
        stepBindingStatuses: Object.fromEntries(
          action.statuses.map((status) => [status.step_line, status]),
        ),
      };
    case "SET_BROWSER":
      return {
        ...state,
        browserWsUrl: action.wsUrl,
        browserRunning: action.running,
        browserMode: action.mode,
      };
    case "SET_TAB":
      return { ...state, rightTab: action.tab };
    case "SET_LAYOUT_MODE":
      return { ...state, layoutMode: action.mode };
    case "TOGGLE_DOCK":
      return { ...state, dockExpanded: !state.dockExpanded };
    case "SET_DOCK_TAB":
      return { ...state, dockActiveTab: action.tab };
    case "SET_DOCK_EXPANDED":
      return { ...state, dockExpanded: action.expanded };
    case "RESTORE_DOCK":
      return {
        ...state,
        dockExpanded: action.expanded,
        dockActiveTab: action.activeTab,
      };
    case "CLOSE_PROJECT":
      return { ...initialProjectState, recentProjects: state.recentProjects };
    default:
      return state;
  }
}
