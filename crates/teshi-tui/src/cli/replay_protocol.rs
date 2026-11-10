//! Runner protocol case events printed by `teshi browser|winapp replay
//! --emit-runner-protocol CASE_ID`.
//!
//! The `[[runners]]` builtin `browser` and `winapp` executors start one replay
//! per scenario and read these events. Each replay reports step events and
//! exactly one terminal case event; the executor owns run framing
//! (`start_run`/`end_run`). Replay evidence is still recorded by the replay's
//! own evidence run.

use serde_json::{Value, json};

/// The browser or WinApp target has no endpoint configured for the project.
pub(crate) const TARGET_UNCONFIGURED: &str = "target_unconfigured";
/// The target is configured but cannot be reached or has nothing attached.
pub(crate) const TARGET_UNAVAILABLE: &str = "target_unavailable";
/// The target cannot run on this operating system.
pub(crate) const PLATFORM_UNSUPPORTED: &str = "platform_unsupported";

/// Writes protocol events for one replayed scenario to stdout.
pub(crate) struct CaseEmitter {
    case_id: String,
}

impl CaseEmitter {
    pub(crate) fn new(case_id: &str) -> Self {
        Self {
            case_id: case_id.to_string(),
        }
    }

    /// Step identity shared with the plan Teshi publishes for this case.
    fn step_id(&self, line: usize) -> String {
        format!("{}:L{line}", self.case_id)
    }

    fn emit(&self, event: Value) {
        println!("{event}");
    }

    pub(crate) fn start_step(&self, line: usize, text: &str) {
        self.emit(json!({"type":"start_step","case_id":self.case_id,
            "step_id":self.step_id(line),"step_line":line,"text":text}));
    }

    pub(crate) fn end_step(&self, line: usize, passed: bool, message: Option<&str>) {
        self.emit(json!({"type":"end_step","case_id":self.case_id,
            "step_id":self.step_id(line),"step_line":line,
            "status":if passed {"passed"} else {"failed"},"message":message}));
    }

    /// Reports the scenario's outcome. Errors raised before any target was
    /// reached are classified by the caller as blocked instead.
    pub(crate) fn finish(&self, result: &anyhow::Result<()>) {
        match result {
            Ok(()) => self.emit(json!({"type":"case_passed","case_id":self.case_id})),
            Err(error) => self.emit(json!({"type":"case_failed","case_id":self.case_id,
                "error":{"message":format!("{error:#}")}})),
        }
    }

    pub(crate) fn blocked(&self, category: &str, reason: &str) {
        self.emit(json!({"type":"case_blocked","case_id":self.case_id,
            "category":category,"reason":reason}));
    }
}

/// Prints replay progress. While stdout carries protocol events, progress goes
/// to stderr so it stays visible as diagnostics without being parsed.
pub(crate) fn progress(protocol: bool, line: &str) {
    if protocol {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}
