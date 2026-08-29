//! Shared Gherkin parsing, language support, highlighting, and desktop render payloads.

pub mod api_bdd;
pub mod authoring;
pub mod daemon;
pub mod diff;
pub mod events;
pub mod gherkin;
pub mod gherkin_keywords;
pub mod gherkin_lang;
pub mod highlight;
pub mod http_exchange;
pub mod llm;
pub mod locator;
pub mod markdown;
pub mod mindmap;
pub mod project_settings;
pub mod render;
pub mod step_index;
pub mod venv;

pub use api_bdd::{
    EngineMismatch, EngineMode, is_engine_tag, normalize_tag, resolve_engine_mode,
    scenario_engine_mode, scenario_steps, step_is_api, strip_api_marker, validate_feature_scenario,
    validate_scenario_steps,
};
pub use diff::{ChangeKind, DiffLine, diff_buffers};
pub use gherkin::*;
pub use gherkin_lang::*;
pub use highlight::{HighlightKind, HighlightSpan, StepHighlightState, highlight_line_spans};
pub use http_exchange::{AssertOutcome, HttpExchange, format_exchange_lines};
pub use render::{
    FeatureRenderPayload, RenderBlock, RenderError, RenderExamples, RenderLine, RenderScenario,
    RenderStep, render_feature,
};
pub use step_index::*;
