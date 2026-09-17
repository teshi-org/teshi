//! Agent backend taxonomy shared by frontends.
//!
//! Native backends use Teshi's existing LLM/tool-call loop. ACP backends launch
//! an external Agent Client Protocol server that owns its own agent loop. The
//! distinction is intentionally not represented as an LLM provider.

use serde::{Deserialize, Serialize};
use teshi_core::llm::ToolCall;

/// Product-level state of one agent turn. Permission and mandatory workflow
/// review are distinct states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentBackendStatus {
    Idle,
    Running,
    WaitingForPermission,
    WaitingForHumanReview,
    Completed,
    Failed,
    Cancelled,
    Unavailable,
}

/// A decision after Teshi has executed a batch of host tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentBackendDecision {
    Continue { allow_tools: bool },
    WaitForPermission,
    WaitForHumanReview,
    StopEmptyProject,
    StopAfterFailure,
    Cancelled,
}

/// Events seen by a frontend. Neither provider nor ACP wire messages cross
/// this boundary.
#[derive(Debug)]
pub enum AgentBackendEvent {
    MessageChunk {
        content: String,
    },
    Completed {
        model: String,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        finish_reason: Option<String>,
    },
    ToolActivity {
        tool_calls: Vec<ToolCall>,
        assistant_message_index: usize,
        finish_reason: Option<String>,
    },
    /// Progress from a tool owned and executed by the external ACP agent.
    ExternalToolActivity {
        title: String,
        status: Option<String>,
    },
    Failed(String),
    Ignored,
}

/// Result of one AgentHost operation, supplied by the application after a
/// backend requests a tool. Permission remains separate from workflow review.
pub struct AgentToolOutcome {
    pub result: Result<String, String>,
    pub pending_permission: bool,
    pub observation_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentBackendKind {
    Native,
    Acp,
}

impl AgentBackendKind {
    pub fn owns_agent_loop(self) -> &'static str {
        match self {
            AgentBackendKind::Native => "teshi",
            AgentBackendKind::Acp => "external_agent",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{approval::ApprovalMode, pipeline::GenerationStage};

    #[test]
    fn acp_is_not_a_native_llm_backend() {
        assert_eq!(AgentBackendKind::Native.owns_agent_loop(), "teshi");
        assert_eq!(AgentBackendKind::Acp.owns_agent_loop(), "external_agent");
    }

    #[test]
    fn approval_modes_do_not_bypass_human_test_point_review_gate() {
        for mode in [
            ApprovalMode::Manual,
            ApprovalMode::Auto,
            ApprovalMode::Bypass,
        ] {
            assert!(
                GenerationStage::ReviewingTestPoints.is_human_review_gate(),
                "{mode:?} must not change review-gate semantics"
            );
        }
    }
}
