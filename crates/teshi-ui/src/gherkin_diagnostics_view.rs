//! Shared non-blocking Gherkin diagnostics presentation for GPUI hosts.

use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div, prelude::FluentBuilder, px, rgb,
};
use teshi_core::{DiagnosticSeverity, ValidationDiagnostic, ValidationReport};

use crate::backend::SharedGherkinEditorBackend;

/// A shared GPUI view for a current, possibly incomplete Feature buffer.
///
/// The view never replaces the supplied source with a parsed representation.
/// Validation runs through the host backend and only updates the diagnostic
/// panel when the response belongs to the latest buffer revision.
pub struct GherkinDiagnosticsView {
    backend: SharedGherkinEditorBackend,
    focus_handle: FocusHandle,
    path: Option<String>,
    content: String,
    report: Option<ValidationReport>,
    status: SharedString,
    revision: u64,
}

impl GherkinDiagnosticsView {
    /// Create an empty diagnostics surface. A host supplies a buffer through
    /// [`Self::set_buffer_public`] when its editor selects a Feature.
    pub fn new(
        backend: SharedGherkinEditorBackend,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self {
            backend,
            focus_handle,
            path: None,
            content: String::new(),
            report: None,
            status: "No Feature buffer selected.".into(),
            revision: 0,
        }
    }

    /// Replace the current source snapshot and request fresh diagnostics.
    pub fn set_buffer_public(&mut self, path: String, content: String, cx: &mut Context<Self>) {
        self.path = Some(path);
        self.content = content;
        self.report = None;
        self.revision = self.revision.wrapping_add(1);
        self.status = "Validating Feature buffer…".into();
        self.request_validation(cx);
        cx.notify();
    }

    /// Clear the selected source without affecting the host editor.
    pub fn clear_buffer_public(&mut self, cx: &mut Context<Self>) {
        self.path = None;
        self.content.clear();
        self.report = None;
        self.revision = self.revision.wrapping_add(1);
        self.status = "No Feature buffer selected.".into();
        cx.notify();
    }

    /// Current report, if a buffer has completed validation.
    pub fn report(&self) -> Option<&ValidationReport> {
        self.report.as_ref()
    }

    /// Stable text representation useful to the Web DOM bridge and smoke tests.
    pub fn diagnostics_text(&self) -> String {
        self.report
            .as_ref()
            .map(|report| {
                report
                    .diagnostics
                    .iter()
                    .map(format_diagnostic)
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .unwrap_or_default()
    }

    fn request_validation(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let content = self.content.clone();
        let revision = self.revision;
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let result = backend.validate_feature_buffer(path, content).await;
            let _ = this.update(cx, |view, cx| {
                if view.revision != revision {
                    return;
                }
                match result {
                    Ok(report) => {
                        view.status = validation_status(&report).into();
                        view.report = Some(report);
                    }
                    Err(error) => {
                        view.status = format!("Validation failed: {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Focusable for GherkinDiagnosticsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GherkinDiagnosticsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .id("gherkin-diagnostics")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .bg(rgb(0x1e1e2e))
            .text_color(rgb(0xcdd6f4))
            .track_focus(&self.focus_handle)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(div().text_lg().child("Gherkin diagnostics"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xa6adc8))
                            .child(self.status.clone()),
                    ),
            );

        if let Some(report) = &self.report {
            let path = self.path.as_deref().unwrap_or("<buffer>");
            let source = self.content.lines().enumerate().map(|(index, line)| {
                let line_number = index + 1;
                let has_error = report.diagnostics.iter().any(|diagnostic| {
                    diagnostic.line == line_number
                        && diagnostic.severity == DiagnosticSeverity::Error
                });
                div()
                    .flex()
                    .gap(px(8.))
                    .px(px(4.))
                    .when(has_error, |this| this.bg(rgb(0x4c1d2f)))
                    .child(
                        div()
                            .w(px(44.))
                            .text_color(rgb(0x6c7086))
                            .child(format!("{line_number}")),
                    )
                    .child(div().flex_1().child(line.to_string()))
            });
            let diagnostics = report.diagnostics.iter().map(diagnostic_row);

            root = root
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0xa6adc8))
                        .child(format!("Source: {path}")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .children(source),
                )
                .child(div().flex().flex_col().gap(px(4.)).children(diagnostics));
        } else {
            root = root.child(
                div()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(0x6c7086))
                    .child("Select a Feature buffer to inspect its diagnostics."),
            );
        }

        // Keep the view reactive even when an embedding editor changes the
        // buffer from a GPUI event callback.
        let _ = cx;
        root
    }
}

fn diagnostic_row(diagnostic: &ValidationDiagnostic) -> impl IntoElement {
    let color = match diagnostic.severity {
        DiagnosticSeverity::Error => rgb(0xf38ba8),
        DiagnosticSeverity::Warning => rgb(0xf9e2af),
        DiagnosticSeverity::Suggestion => rgb(0x89dceb),
    };
    div()
        .text_sm()
        .text_color(color)
        .child(format_diagnostic(diagnostic))
}

fn format_diagnostic(diagnostic: &ValidationDiagnostic) -> String {
    let suggestion = diagnostic
        .suggestion
        .as_deref()
        .map(|value| format!(" -> {value}"))
        .unwrap_or_default();
    format!(
        "{}:{}:{} [{}] {}{}",
        diagnostic.path,
        diagnostic.line,
        diagnostic.column,
        diagnostic.code,
        diagnostic.message,
        suggestion,
    )
}

fn validation_status(report: &ValidationReport) -> String {
    format!(
        "{} error(s), {} warning(s), {} suggestion(s)",
        report.summary.errors, report.summary.warnings, report.summary.suggestions
    )
}

#[cfg(test)]
mod tests {
    use super::{format_diagnostic, validation_status};
    use teshi_core::{
        DiagnosticSeverity, ValidationDiagnostic, ValidationReport, ValidationSummary,
    };

    #[test]
    fn diagnostic_text_keeps_location_code_and_suggestion() {
        let diagnostic = ValidationDiagnostic {
            path: "features/login.feature".into(),
            line: 4,
            column: 6,
            severity: DiagnosticSeverity::Error,
            code: "missing_step_separator".into(),
            message: "keyword needs a space".into(),
            suggestion: Some("当 用户登录".into()),
        };
        let text = format_diagnostic(&diagnostic);
        assert!(text.contains("features/login.feature:4:6"));
        assert!(text.contains("[missing_step_separator]"));
        assert!(text.contains("-> 当 用户登录"));
    }

    #[test]
    fn validation_status_exposes_aggregate_counts() {
        let report = ValidationReport {
            scope: vec!["login.feature".into()],
            summary: ValidationSummary {
                errors: 1,
                warnings: 2,
                suggestions: 3,
            },
            diagnostics: Vec::new(),
        };
        assert_eq!(
            validation_status(&report),
            "1 error(s), 2 warning(s), 3 suggestion(s)"
        );
    }
}
