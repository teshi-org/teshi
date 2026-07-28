//! Shared Gherkin parsing, language support, highlighting, and desktop render payloads.

pub mod authoring;
pub mod daemon;
pub mod diff;
pub mod events;
pub mod gherkin;
pub mod gherkin_keywords;
pub mod gherkin_lang;
pub mod highlight;
pub mod llm;
pub mod locator;
pub mod markdown;
pub mod mindmap;
pub mod project_settings;
pub mod render;
pub mod step_index;
pub mod venv;

pub use diff::{ChangeKind, DiffLine, diff_buffers};
pub use gherkin::*;
pub use gherkin_lang::*;
pub use highlight::{HighlightKind, HighlightSpan, StepHighlightState, highlight_line_spans};
pub use render::{
    FeatureRenderPayload, RenderBlock, RenderError, RenderExamples, RenderLine, RenderScenario,
    RenderStep, render_feature,
};
pub use step_index::*;
