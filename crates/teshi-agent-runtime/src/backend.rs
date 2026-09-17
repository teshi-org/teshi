//! Application-facing backend composition. ACP is intentionally unavailable
//! until its execution adapter is implemented.

use std::sync::mpsc::TryRecvError;
use teshi_agent::backend::{
    AgentBackendDecision, AgentBackendEvent, AgentBackendKind, AgentBackendStatus, AgentToolOutcome,
};
use teshi_agent::pipeline::GenerationStage;
use teshi_core::llm::{ChatMessage, ToolCall, ToolDefinition};
use teshi_engine::llm::LlmConfig;

use crate::{
    AiChatMessage, NativeAgentRuntime, NativeContinuation, NativeRuntimeEvent, NativeToolExecution,
    NativeTurnState,
};

#[derive(Debug)]
pub struct NativeAgentBackend {
    runtime: NativeAgentRuntime,
}

impl NativeAgentBackend {
    pub fn new(max_loops: u32) -> Self {
        Self {
            runtime: NativeAgentRuntime::new(max_loops),
        }
    }
}

#[derive(Debug, Default)]
pub struct AcpAgentBackend {
    messages: Vec<AiChatMessage>,
    partial_response: String,
}

/// One owned backend per conversation. Enum dispatch keeps the two possible
/// implementations visible to the compiler without shared mutable UI state.
#[derive(Debug)]
pub enum AgentBackendRuntime {
    Native(Box<NativeAgentBackend>),
    Acp(AcpAgentBackend),
}

impl AgentBackendRuntime {
    /// Temporarily releases the backend while AgentHost calls mutate the
    /// application. The replacement has the same selected kind.
    pub fn take_for_host_call(&mut self) -> Self {
        let replacement = Self::new(self.kind(), crate::max_agent_iterations());
        std::mem::replace(self, replacement)
    }
    /// Test fixture hook; production model events stay inside the Native adapter.
    #[doc(hidden)]
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn attach_model_event_receiver_for_tests(
        &mut self,
        rx: std::sync::mpsc::Receiver<teshi_engine::llm::LlmEvent>,
    ) {
        if let Self::Native(backend) = self {
            backend.runtime.llm_rx = Some(rx);
        }
    }
    pub fn new(kind: AgentBackendKind, max_loops: u32) -> Self {
        match kind {
            AgentBackendKind::Native => Self::Native(Box::new(NativeAgentBackend::new(max_loops))),
            AgentBackendKind::Acp => Self::Acp(AcpAgentBackend::default()),
        }
    }

    pub fn kind(&self) -> AgentBackendKind {
        match self {
            Self::Native(_) => AgentBackendKind::Native,
            Self::Acp(_) => AgentBackendKind::Acp,
        }
    }

    pub fn status(&self) -> AgentBackendStatus {
        match self {
            Self::Native(backend) => match backend.runtime.state() {
                NativeTurnState::Idle => AgentBackendStatus::Idle,
                NativeTurnState::WaitingForModel => AgentBackendStatus::Running,
                NativeTurnState::WaitingForApproval => AgentBackendStatus::WaitingForPermission,
                NativeTurnState::WaitingForHumanReview => AgentBackendStatus::WaitingForHumanReview,
                NativeTurnState::Completed => AgentBackendStatus::Completed,
                NativeTurnState::Failed => AgentBackendStatus::Failed,
                NativeTurnState::Cancelled => AgentBackendStatus::Cancelled,
            },
            Self::Acp(_) => AgentBackendStatus::Unavailable,
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Native(backend) if backend.runtime.llm_handle.is_some())
    }

    pub fn attach_native_model(&mut self, config: LlmConfig) -> anyhow::Result<()> {
        match self {
            Self::Native(backend) => {
                backend.runtime.attach_model(config);
                Ok(())
            }
            Self::Acp(_) => anyhow::bail!("ACP agent backend is not implemented"),
        }
    }

    pub fn start(&mut self) -> anyhow::Result<()> {
        match self {
            Self::Native(backend) => {
                backend.runtime.start();
                Ok(())
            }
            Self::Acp(_) => anyhow::bail!("ACP agent backend is not implemented"),
        }
    }

    /// Submit the next turn with Teshi's projected conversation and available
    /// host capabilities. Native translates this into its model request.
    pub fn submit_turn(
        &mut self,
        system: Option<String>,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<()> {
        match self {
            Self::Native(backend) => backend.runtime.send_chat(system, messages, tools),
            Self::Acp(_) => anyhow::bail!("ACP agent backend is not implemented"),
        }
    }

    pub fn cancel(&mut self) {
        if let Self::Native(backend) = self {
            backend.runtime.cancel();
        }
    }

    pub fn try_next_event(&mut self) -> Result<Option<AgentBackendEvent>, TryRecvError> {
        match self {
            Self::Native(backend) => backend
                .runtime
                .try_next_event()
                .map(|event| event.map(Into::into)),
            Self::Acp(_) => Ok(None),
        }
    }

    pub fn execute_tool_batch(
        &mut self,
        assistant_idx: usize,
        tool_calls: &[ToolCall],
        mut execute: impl FnMut(&ToolCall) -> AgentToolOutcome,
    ) -> bool {
        match self {
            Self::Native(backend) => {
                backend
                    .runtime
                    .execute_tool_batch(assistant_idx, tool_calls, |call| {
                        let outcome = execute(call);
                        NativeToolExecution {
                            result: outcome.result,
                            pending_approval: outcome.pending_permission,
                            observation_message: outcome.observation_message,
                        }
                    })
            }
            Self::Acp(_) => false,
        }
    }

    pub fn after_tools(
        &mut self,
        stage: GenerationStage,
        approval_pending: bool,
        manual_approval: bool,
        project_has_features: bool,
    ) -> AgentBackendDecision {
        match self {
            Self::Native(backend) => backend
                .runtime
                .after_tools(
                    stage,
                    approval_pending,
                    manual_approval,
                    project_has_features,
                )
                .into(),
            Self::Acp(_) => AgentBackendDecision::StopAfterFailure,
        }
    }

    pub fn resume_after_approval(&mut self, project_has_features: bool) -> AgentBackendDecision {
        match self {
            Self::Native(backend) => backend
                .runtime
                .resume_after_approval(project_has_features)
                .into(),
            Self::Acp(_) => AgentBackendDecision::StopAfterFailure,
        }
    }

    pub fn messages(&self) -> &[AiChatMessage] {
        match self {
            Self::Native(backend) => &backend.runtime.messages,
            Self::Acp(backend) => &backend.messages,
        }
    }

    pub fn messages_mut(&mut self) -> &mut Vec<AiChatMessage> {
        match self {
            Self::Native(backend) => &mut backend.runtime.messages,
            Self::Acp(backend) => &mut backend.messages,
        }
    }

    pub fn partial_response(&self) -> &str {
        match self {
            Self::Native(backend) => &backend.runtime.partial_response,
            Self::Acp(backend) => &backend.partial_response,
        }
    }

    pub fn partial_response_mut(&mut self) -> &mut String {
        match self {
            Self::Native(backend) => &mut backend.runtime.partial_response,
            Self::Acp(backend) => &mut backend.partial_response,
        }
    }
}

impl From<NativeRuntimeEvent> for AgentBackendEvent {
    fn from(value: NativeRuntimeEvent) -> Self {
        match value {
            NativeRuntimeEvent::MessageChunk { content } => Self::MessageChunk { content },
            NativeRuntimeEvent::Completed {
                model,
                input_tokens,
                output_tokens,
                finish_reason,
            } => Self::Completed {
                model,
                input_tokens,
                output_tokens,
                finish_reason,
            },
            NativeRuntimeEvent::ToolCalls {
                tool_calls,
                assistant_message_index,
                finish_reason,
            } => Self::ToolActivity {
                tool_calls,
                assistant_message_index,
                finish_reason,
            },
            NativeRuntimeEvent::Failed(message) => Self::Failed(message),
            NativeRuntimeEvent::Ignored => Self::Ignored,
        }
    }
}

impl From<NativeContinuation> for AgentBackendDecision {
    fn from(value: NativeContinuation) -> Self {
        match value {
            NativeContinuation::RequestModel { allow_tools } => Self::Continue { allow_tools },
            NativeContinuation::WaitForApproval => Self::WaitForPermission,
            NativeContinuation::WaitForHumanReview => Self::WaitForHumanReview,
            NativeContinuation::StopEmptyProject => Self::StopEmptyProject,
            NativeContinuation::StopAfterFailure => Self::StopAfterFailure,
            NativeContinuation::Cancelled => Self::Cancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use teshi_engine::llm::LlmEvent;

    fn native() -> AgentBackendRuntime {
        AgentBackendRuntime::new(AgentBackendKind::Native, 1)
    }

    #[test]
    fn plain_completion_is_backend_event() {
        let mut backend = native();
        let (tx, rx) = mpsc::channel();
        backend.attach_model_event_receiver_for_tests(rx);
        backend.start().unwrap();
        tx.send(LlmEvent::Done {
            full_text: "answer".into(),
            reasoning_content: None,
            model: "mock".into(),
            input_tokens: Some(2),
            output_tokens: Some(1),
            finish_reason: None,
        })
        .unwrap();
        assert!(matches!(
            backend.try_next_event().unwrap(),
            Some(AgentBackendEvent::Completed {
                input_tokens: Some(2),
                output_tokens: Some(1),
                ..
            })
        ));
        assert_eq!(backend.messages()[0].content, "answer");
    }

    #[test]
    fn completion_and_stream_order_are_backend_events() {
        let mut backend = native();
        let (tx, rx) = mpsc::channel();
        backend.attach_model_event_receiver_for_tests(rx);
        backend.start().unwrap();
        for content in ["A", "B"] {
            tx.send(LlmEvent::Chunk {
                content: content.into(),
            })
            .unwrap();
        }
        tx.send(LlmEvent::Done {
            full_text: "AB".into(),
            reasoning_content: None,
            model: "mock".into(),
            input_tokens: None,
            output_tokens: None,
            finish_reason: None,
        })
        .unwrap();
        assert!(
            matches!(backend.try_next_event().unwrap(), Some(AgentBackendEvent::MessageChunk { content }) if content == "A")
        );
        assert!(
            matches!(backend.try_next_event().unwrap(), Some(AgentBackendEvent::MessageChunk { content }) if content == "B")
        );
        assert!(matches!(
            backend.try_next_event().unwrap(),
            Some(AgentBackendEvent::Completed { .. })
        ));
        assert_eq!(backend.messages()[0].content, "AB");
        assert_eq!(backend.status(), AgentBackendStatus::Completed);
    }

    #[test]
    fn tool_host_result_precedes_continuation_and_review_stops_it() {
        let mut backend = native();
        let (tx, rx) = mpsc::channel();
        backend.attach_model_event_receiver_for_tests(rx);
        backend.start().unwrap();
        tx.send(LlmEvent::ToolCallRequest {
            tool_calls: vec![ToolCall {
                id: "one".into(),
                name: "propose_test_points".into(),
                arguments: "{}".into(),
                execution_duration_ms: None,
            }],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            finish_reason: None,
        })
        .unwrap();
        let Some(AgentBackendEvent::ToolActivity {
            tool_calls,
            assistant_message_index,
            ..
        }) = backend.try_next_event().unwrap()
        else {
            panic!("expected host tool activity")
        };
        let mut called = false;
        backend.execute_tool_batch(assistant_message_index, &tool_calls, |_| {
            called = true;
            AgentToolOutcome {
                result: Ok("proposal saved".into()),
                pending_permission: false,
                observation_message: None,
            }
        });
        assert!(called);
        assert_eq!(backend.messages()[0].role, crate::AiRole::Assistant);
        assert_eq!(backend.messages()[1].role, crate::AiRole::Tool);
        assert_eq!(backend.messages()[1].content, "proposal saved");
        assert_eq!(
            backend.after_tools(GenerationStage::ReviewingTestPoints, false, false, true),
            AgentBackendDecision::WaitForHumanReview
        );
        assert_eq!(backend.status(), AgentBackendStatus::WaitingForHumanReview);
        assert!(backend.try_next_event().unwrap().is_none());
    }

    #[test]
    fn cancellation_discards_old_events_and_next_turn_works() {
        let mut backend = native();
        let (old_tx, old_rx) = mpsc::channel();
        backend.attach_model_event_receiver_for_tests(old_rx);
        backend.start().unwrap();
        backend.cancel();
        assert!(
            old_tx
                .send(LlmEvent::Chunk {
                    content: "stale".into(),
                })
                .is_err()
        );
        assert!(backend.try_next_event().unwrap().is_none());
        let (tx, rx) = mpsc::channel();
        backend.attach_model_event_receiver_for_tests(rx);
        backend.start().unwrap();
        tx.send(LlmEvent::Done {
            full_text: "fresh".into(),
            reasoning_content: None,
            model: "mock".into(),
            input_tokens: None,
            output_tokens: None,
            finish_reason: None,
        })
        .unwrap();
        assert!(matches!(
            backend.try_next_event().unwrap(),
            Some(AgentBackendEvent::Completed { .. })
        ));
        assert_eq!(backend.messages()[0].content, "fresh");
    }

    #[test]
    fn native_loop_limit_still_requests_one_final_response() {
        let mut backend = AgentBackendRuntime::new(AgentBackendKind::Native, 0);
        backend.start().unwrap();
        assert_eq!(
            backend.after_tools(GenerationStage::Idle, false, false, true),
            AgentBackendDecision::Continue { allow_tools: false }
        );
        assert_eq!(
            backend.after_tools(GenerationStage::Idle, false, false, true),
            AgentBackendDecision::StopAfterFailure
        );
    }

    #[test]
    fn acp_selection_is_explicitly_unavailable() {
        let mut backend = AgentBackendRuntime::new(AgentBackendKind::Acp, 1);
        assert_eq!(backend.kind(), AgentBackendKind::Acp);
        assert_eq!(backend.status(), AgentBackendStatus::Unavailable);
        assert!(
            backend
                .start()
                .unwrap_err()
                .to_string()
                .contains("not implemented")
        );
        assert!(
            backend
                .submit_turn(None, vec![], None)
                .unwrap_err()
                .to_string()
                .contains("not implemented")
        );
    }
}
