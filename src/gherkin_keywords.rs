//! Shared Gherkin keyword constants used by both the line-level editor navigation (`bdd_nav`)
//! and the file-level AST parser (`gherkin`).

/// Step keywords in picker cycle order.
pub const STEP_KEYWORDS: &[&str] = &["Given", "When", "Then", "And", "But"];

/// Structural header prefixes; longer prefixes appear first to avoid ambiguous prefix matches.
pub const HEADER_PREFIXES: &[&str] = &[
    "Scenario Outline:",
    "Feature:",
    "Background:",
    "Scenario:",
    "Examples:",
];

/// Headers whose trailing text (after the colon) is editable.
///
/// `Background:` is intentionally excluded — only the keyword is navigable on that line.
pub const HEADER_TITLE_EDIT_PREFIXES: &[&str] =
    &["Scenario Outline:", "Feature:", "Scenario:", "Examples:"];
