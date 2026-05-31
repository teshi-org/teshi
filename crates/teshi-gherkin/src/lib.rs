//! Shared Gherkin parsing, language support, highlighting, and desktop render payloads.

pub mod gherkin;
pub mod gherkin_keywords;
pub mod gherkin_lang;
pub mod highlight;
pub mod render;

pub use gherkin::*;
pub use gherkin_lang::*;
pub use highlight::{HighlightKind, HighlightSpan, StepHighlightState, highlight_line_spans};
pub use render::{
    FeatureRenderPayload, RenderBlock, RenderError, RenderExamples, RenderLine, RenderScenario,
    RenderStep, render_feature,
};
