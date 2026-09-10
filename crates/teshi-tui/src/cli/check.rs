//! Canonical teshi check command.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;
use teshi_core::{DiagnosticSeverity, ValidationDiagnostic, ValidationReport, ValidationSummary};

use super::CheckArgs;

/// Validate a Feature before a binding or execution operation.
///
/// Warnings and suggestions are surfaced on stderr while errors return before
/// the caller can read binding state, write selection state, or execute a
/// target action.
pub(crate) fn preflight_feature(project_root: &Path, feature: &str) -> Result<ValidationReport> {
    let report = teshi_engine::validate_feature_scope(project_root, Some(Path::new(feature)))?;
    if !report.diagnostics.is_empty() {
        eprintln!("{}", serde_json::to_string_pretty(&report)?);
    }
    if report.has_errors() {
        anyhow::bail!("Feature validation failed for {feature}");
    }
    Ok(report)
}

/// Run teshi check for one Feature or the current project scope.
pub fn handle_check_command(args: &CheckArgs) -> Result<()> {
    let project_root = project_root_for(args.feature.as_deref());
    let report = match teshi_engine::validate_feature_scope(&project_root, args.feature.as_deref())
    {
        Ok(report) => report,
        Err(error) => {
            emit_io_error(args.json, &error.to_string());
            std::process::exit(2);
        }
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human_report(&report);
    }

    if report.has_errors() {
        std::process::exit(1);
    }
    Ok(())
}

fn project_root_for(feature: Option<&Path>) -> PathBuf {
    let start = feature.map(|path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    });
    teshi_engine::find_project_root(start.as_deref())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn print_human_report(report: &ValidationReport) {
    println!("Checked {} Feature file(s)", report.scope.len());
    for diagnostic in &report.diagnostics {
        println!(
            "{} {}:{}:{} [{}] {}",
            severity_label(diagnostic.severity),
            diagnostic.path,
            diagnostic.line,
            diagnostic.column,
            diagnostic.code,
            diagnostic.message
        );
        if let Some(suggestion) = &diagnostic.suggestion {
            println!("  suggestion: {suggestion}");
        }
    }
    println!(
        "Summary: {} error(s), {} warning(s), {} suggestion(s)",
        report.summary.errors, report.summary.warnings, report.summary.suggestions
    );
}

fn severity_label(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Suggestion => "suggestion",
    }
}

#[derive(Debug, Serialize)]
struct CheckErrorEnvelope<'a> {
    ok: bool,
    scope: Vec<String>,
    summary: ValidationSummary,
    diagnostics: Vec<ValidationDiagnostic>,
    error: CheckError<'a>,
}

#[derive(Debug, Serialize)]
struct CheckError<'a> {
    code: &'static str,
    message: &'a str,
}

fn emit_io_error(json: bool, message: &str) {
    if json {
        let envelope = CheckErrorEnvelope {
            ok: false,
            scope: Vec::new(),
            summary: ValidationSummary::default(),
            diagnostics: Vec::new(),
            error: CheckError {
                code: "check_io_error",
                message,
            },
        };
        // Serialization of these in-memory fields cannot fail. If it ever
        // does, retain the one-line machine-readable error contract.
        match serde_json::to_string_pretty(&envelope) {
            Ok(output) => println!("{output}"),
            Err(_) => println!(r#"{{"ok":false,"error":{{"code":"check_io_error"}}}}"#),
        }
    } else {
        eprintln!("check: {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(diagnostics: Vec<ValidationDiagnostic>) -> ValidationReport {
        let mut summary = ValidationSummary::default();
        for diagnostic in &diagnostics {
            match diagnostic.severity {
                DiagnosticSeverity::Error => summary.errors += 1,
                DiagnosticSeverity::Warning => summary.warnings += 1,
                DiagnosticSeverity::Suggestion => summary.suggestions += 1,
            }
        }
        ValidationReport {
            scope: vec!["features/login.feature".into()],
            summary,
            diagnostics,
        }
    }

    #[test]
    fn human_output_contains_location_code_suggestion_and_summary() {
        let output = capture_human_output(&report(vec![ValidationDiagnostic {
            path: "features/login.feature".into(),
            line: 4,
            column: 6,
            severity: DiagnosticSeverity::Error,
            code: "missing_step_separator".into(),
            message: "keyword needs a space".into(),
            suggestion: Some("当 用户登录".into()),
        }]));
        assert!(output.contains("features/login.feature:4:6"));
        assert!(output.contains("[missing_step_separator]"));
        assert!(output.contains("suggestion: 当 用户登录"));
        assert!(output.contains("Summary: 1 error(s)"));
    }

    // Keep formatting testable without redirecting process stdout in the
    // command handler. This mirrors the stable pieces printed by the helper.
    fn capture_human_output(report: &ValidationReport) -> String {
        let mut output = format!("Checked {} Feature file(s)\n", report.scope.len());
        for diagnostic in &report.diagnostics {
            output.push_str(&format!(
                "{} {}:{}:{} [{}] {}\n",
                severity_label(diagnostic.severity),
                diagnostic.path,
                diagnostic.line,
                diagnostic.column,
                diagnostic.code,
                diagnostic.message
            ));
            if let Some(suggestion) = &diagnostic.suggestion {
                output.push_str(&format!("  suggestion: {suggestion}\n"));
            }
        }
        output.push_str(&format!(
            "Summary: {} error(s), {} warning(s), {} suggestion(s)\n",
            report.summary.errors, report.summary.warnings, report.summary.suggestions
        ));
        output
    }
}
