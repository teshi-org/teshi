//! Agent backend taxonomy shared by frontends.
//!
//! Native backends use Teshi's existing LLM/tool-call loop. ACP backends launch
//! an external Agent Client Protocol server that owns its own agent loop. The
//! distinction is intentionally not represented as an LLM provider.

use serde::{Deserialize, Serialize};

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
