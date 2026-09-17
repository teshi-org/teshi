//! Frontend-independent control state for Teshi's native model/tool loop.
//!
//! The host executes tools and supplies workflow facts; this runtime alone
//! decides whether another model request is allowed.

use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, TryRecvError};
use teshi_agent::pipeline::GenerationStage;
use teshi_core::llm::{ChatMessage, ToolDefinition};
use teshi_engine::llm::{LlmConfig, LlmEvent, LlmHandle, LlmRequest};

pub mod backend;

/// Conversation record shared with frontends and persisted sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatMessage {
    pub role: AiRole,
    pub content: String,
    pub tool_calls: Option<Vec<teshi_core::llm::ToolCall>>,
    pub tool_call_id: Option<String>,
    pub reasoning_content: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiRole {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeTurnState {
    Idle,
    WaitingForModel,
    WaitingForApproval,
    WaitingForHumanReview,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeContinuation {
    RequestModel { allow_tools: bool },
    WaitForApproval,
    WaitForHumanReview,
    StopEmptyProject,
    StopAfterFailure,
    Cancelled,
}

/// Progress emitted to a frontend after the native runtime consumes a model event.
#[derive(Debug)]
pub enum NativeRuntimeEvent {
    MessageChunk {
        content: String,
    },
    Completed {
        model: String,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        finish_reason: Option<String>,
    },
    ToolCalls {
        tool_calls: Vec<teshi_core::llm::ToolCall>,
        assistant_message_index: usize,
        finish_reason: Option<String>,
    },
    Failed(String),
    Ignored,
}

/// The frontend host's result for one Teshi tool call.
pub struct NativeToolExecution {
    pub result: Result<String, String>,
    pub pending_approval: bool,
    pub observation_message: Option<String>,
}

/// One native turn's continuation and cancellation guard. Model transport,
/// prompts, conversation projection, and tool capabilities remain separate.
#[derive(Debug)]
pub struct NativeAgentRuntime {
    pub messages: Vec<AiChatMessage>,
    pub partial_response: String,
    pub llm_handle: Option<LlmHandle>,
    pub llm_rx: Option<Receiver<LlmEvent>>,
    model_config: Option<LlmConfig>,
    state: NativeTurnState,
    loops: u32,
    max_loops: u32,
    final_request_sent: bool,
}

impl Drop for NativeAgentRuntime {
    fn drop(&mut self) {
        if let Some(handle) = &self.llm_handle {
            handle.cancel();
        }
    }
}

impl NativeAgentRuntime {
    pub fn new(max_loops: u32) -> Self {
        Self {
            messages: Vec::new(),
            partial_response: String::new(),
            llm_handle: None,
            llm_rx: None,
            model_config: None,
            state: NativeTurnState::Idle,
            loops: 0,
            max_loops,
            final_request_sent: false,
        }
    }

    pub fn state(&self) -> NativeTurnState {
        self.state
    }

    pub fn attach_model(&mut self, config: LlmConfig) {
        if self.llm_handle.is_some()
            && matches!(
                self.state,
                NativeTurnState::WaitingForModel | NativeTurnState::WaitingForApproval
            )
        {
            self.state = NativeTurnState::Cancelled;
            self.partial_response.clear();
        }
        if let Some(handle) = self.llm_handle.take() {
            handle.cancel();
        }
        self.llm_rx = None;
        self.model_config = Some(config.clone());
        let (handle, rx) = teshi_engine::llm::spawn_llm(config);
        self.llm_handle = Some(handle);
        self.llm_rx = Some(rx);
    }

    fn send_request(&mut self, request: LlmRequest) -> anyhow::Result<()> {
        if !self.accepts_model_event() {
            anyhow::bail!("native agent turn is not waiting for a model response");
        }
        let Some(handle) = &self.llm_handle else {
            self.state = NativeTurnState::Failed;
            anyhow::bail!("native LLM worker is not configured");
        };
        handle
            .send(request)
            .inspect_err(|_| self.state = NativeTurnState::Failed)
    }

    pub fn send_chat(
        &mut self,
        system: Option<String>,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<()> {
        self.send_request(LlmRequest::Chat {
            system,
            messages,
            tools,
        })
    }

    pub fn try_next_event(&mut self) -> Result<Option<NativeRuntimeEvent>, TryRecvError> {
        let Some(rx) = &self.llm_rx else {
            return Ok(None);
        };
        match rx.try_recv() {
            Ok(event) => Ok(Some(self.consume_model_event(event))),
            Err(TryRecvError::Empty) => Ok(None),
            Err(error) => {
                self.fail();
                self.llm_rx = None;
                Err(error)
            }
        }
    }

    pub fn consume_model_event(&mut self, event: LlmEvent) -> NativeRuntimeEvent {
        if !self.accepts_model_event() {
            return NativeRuntimeEvent::Ignored;
        }
        match event {
            LlmEvent::Chunk { content } => {
                self.append_chunk(&content);
                NativeRuntimeEvent::MessageChunk { content }
            }
            LlmEvent::Done {
                full_text,
                reasoning_content,
                model,
                input_tokens,
                output_tokens,
                finish_reason,
            } => {
                self.finish_text(full_text, reasoning_content);
                NativeRuntimeEvent::Completed {
                    model,
                    input_tokens,
                    output_tokens,
                    finish_reason,
                }
            }
            LlmEvent::ToolCallRequest {
                tool_calls,
                reasoning_content,
                finish_reason,
                ..
            } => match self.begin_tool_calls(tool_calls.clone(), reasoning_content) {
                Some(assistant_message_index) => NativeRuntimeEvent::ToolCalls {
                    tool_calls,
                    assistant_message_index,
                    finish_reason,
                },
                None => NativeRuntimeEvent::Failed(
                    "model called tools after the final-response limit".into(),
                ),
            },
            LlmEvent::Error { message } => {
                self.fail();
                NativeRuntimeEvent::Failed(message)
            }
        }
    }

    pub fn start(&mut self) {
        if matches!(
            self.state,
            NativeTurnState::WaitingForModel | NativeTurnState::WaitingForApproval
        ) && let Some(config) = self.model_config.clone()
        {
            self.attach_model(config);
        }
        self.loops = 0;
        self.final_request_sent = false;
        self.partial_response.clear();
        self.state = NativeTurnState::WaitingForModel;
    }

    pub fn append_chunk(&mut self, content: &str) {
        if self.accepts_model_event() {
            self.partial_response.push_str(content);
        }
    }

    pub fn finish_text(&mut self, full_text: String, reasoning_content: Option<String>) {
        if !self.accepts_model_event() {
            return;
        }
        let content = if self.partial_response.is_empty() {
            full_text
        } else {
            std::mem::take(&mut self.partial_response)
        };
        self.messages.push(AiChatMessage {
            role: AiRole::Assistant,
            content,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content,
            source: None,
        });
        self.complete();
    }

    pub fn begin_tool_calls(
        &mut self,
        tool_calls: Vec<teshi_core::llm::ToolCall>,
        reasoning_content: Option<String>,
    ) -> Option<usize> {
        if !self.accepts_model_event() || self.final_request_sent {
            self.state = NativeTurnState::Failed;
            return None;
        }
        self.messages.push(AiChatMessage {
            role: AiRole::Assistant,
            content: std::mem::take(&mut self.partial_response),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            reasoning_content,
            source: None,
        });
        Some(self.messages.len() - 1)
    }

    pub fn record_tool_result(&mut self, tool_call_id: String, result: String) {
        self.messages.push(AiChatMessage {
            role: AiRole::Tool,
            content: result,
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            reasoning_content: None,
            source: None,
        });
    }

    pub fn record_tool_duration(
        &mut self,
        assistant_idx: usize,
        tool_call_id: &str,
        duration_ms: u64,
    ) {
        if let Some(calls) = self
            .messages
            .get_mut(assistant_idx)
            .and_then(|m| m.tool_calls.as_mut())
            && let Some(call) = calls.iter_mut().find(|call| call.id == tool_call_id)
        {
            call.execution_duration_ms = Some(duration_ms);
        }
    }

    /// Run one model-requested tool batch in call order. The supplied host
    /// adapter performs Teshi-specific work; the runtime records each outcome
    /// before it decides whether the model may continue.
    pub fn execute_tool_batch(
        &mut self,
        assistant_idx: usize,
        tool_calls: &[teshi_core::llm::ToolCall],
        mut execute: impl FnMut(&teshi_core::llm::ToolCall) -> NativeToolExecution,
    ) -> bool {
        let mut pending_queued = false;
        for call in tool_calls {
            if !self.accepts_model_event() {
                break;
            }
            let started = std::time::Instant::now();
            let outcome = execute(call);
            match outcome.result {
                Ok(result) => {
                    if outcome.pending_approval {
                        pending_queued = true;
                    } else {
                        self.record_tool_result(call.id.clone(), result);
                    }
                    if let Some(content) = outcome.observation_message {
                        self.messages.push(AiChatMessage {
                            role: AiRole::User,
                            content,
                            tool_calls: None,
                            tool_call_id: None,
                            reasoning_content: None,
                            source: Some("browser_visual_observation".into()),
                        });
                    }
                }
                Err(error) => self.record_tool_result(call.id.clone(), format!("Error: {error}")),
            }
            self.record_tool_duration(
                assistant_idx,
                &call.id,
                started.elapsed().as_millis() as u64,
            );
        }
        pending_queued
    }

    pub fn accepts_model_event(&self) -> bool {
        self.state == NativeTurnState::WaitingForModel
    }

    pub fn complete(&mut self) {
        if self.accepts_model_event() {
            self.state = NativeTurnState::Completed;
        }
    }

    pub fn fail(&mut self) {
        if self.accepts_model_event() {
            self.state = NativeTurnState::Failed;
        }
    }

    pub fn cancel(&mut self) {
        if let Some(handle) = &self.llm_handle {
            handle.cancel();
        }
        self.llm_handle = None;
        self.llm_rx = None;
        self.partial_response.clear();
        self.state = NativeTurnState::Cancelled;
    }

    pub fn after_tools(
        &mut self,
        stage: GenerationStage,
        approval_pending: bool,
        manual_approval: bool,
        project_has_features: bool,
    ) -> NativeContinuation {
        if !self.accepts_model_event() {
            return NativeContinuation::Cancelled;
        }
        if stage.is_human_review_gate() {
            self.state = NativeTurnState::WaitingForHumanReview;
            return NativeContinuation::WaitForHumanReview;
        }
        if approval_pending && manual_approval {
            self.state = NativeTurnState::WaitingForApproval;
            return NativeContinuation::WaitForApproval;
        }
        if !approval_pending && !project_has_features {
            self.state = NativeTurnState::Completed;
            return NativeContinuation::StopEmptyProject;
        }
        self.continue_after_tools()
    }

    pub fn resume_after_approval(&mut self, project_has_features: bool) -> NativeContinuation {
        if self.state != NativeTurnState::WaitingForApproval {
            return NativeContinuation::Cancelled;
        }
        if !project_has_features {
            self.state = NativeTurnState::Completed;
            return NativeContinuation::StopEmptyProject;
        }
        self.continue_after_tools()
    }

    fn continue_after_tools(&mut self) -> NativeContinuation {
        self.state = NativeTurnState::WaitingForModel;
        self.loops += 1;
        if self.loops > self.max_loops {
            if self.final_request_sent {
                self.state = NativeTurnState::Failed;
                return NativeContinuation::StopAfterFailure;
            }
            self.final_request_sent = true;
            return NativeContinuation::RequestModel { allow_tools: false };
        }
        NativeContinuation::RequestModel { allow_tools: true }
    }
}

pub fn max_agent_iterations() -> u32 {
    std::env::var("TESHI_AI_MAX_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn multi_loop_limit_and_final_request() {
        let mut runtime = NativeAgentRuntime::new(2);
        runtime.start();
        for _ in 0..2 {
            assert_eq!(
                runtime.after_tools(GenerationStage::Idle, false, false, true),
                NativeContinuation::RequestModel { allow_tools: true }
            );
        }
        assert_eq!(
            runtime.after_tools(GenerationStage::Idle, false, false, true),
            NativeContinuation::RequestModel { allow_tools: false }
        );
        assert_eq!(
            runtime.after_tools(GenerationStage::Idle, false, false, true),
            NativeContinuation::StopAfterFailure
        );
    }

    #[test]
    fn review_gate_precedes_auto_approval_and_continuation() {
        let mut runtime = NativeAgentRuntime::new(100);
        runtime.start();
        assert_eq!(
            runtime.after_tools(GenerationStage::ReviewingTestPoints, true, false, true),
            NativeContinuation::WaitForHumanReview
        );
        assert!(!runtime.accepts_model_event());
    }

    #[test]
    fn approval_result_resumes_model_event_consumption() {
        let mut runtime = NativeAgentRuntime::new(2);
        runtime.start();
        assert_eq!(
            runtime.after_tools(GenerationStage::Idle, true, true, true),
            NativeContinuation::WaitForApproval
        );
        assert!(!runtime.accepts_model_event());
        assert_eq!(
            runtime.resume_after_approval(true),
            NativeContinuation::RequestModel { allow_tools: true }
        );
        assert!(runtime.accepts_model_event());
    }

    #[test]
    fn cancelled_turn_cannot_continue() {
        let mut runtime = NativeAgentRuntime::new(100);
        runtime.start();
        runtime.cancel();
        assert_eq!(
            runtime.after_tools(GenerationStage::Idle, false, false, true),
            NativeContinuation::Cancelled
        );
        assert!(!runtime.accepts_model_event());
        assert!(matches!(
            runtime.consume_model_event(LlmEvent::Chunk {
                content: "late".into()
            }),
            NativeRuntimeEvent::Ignored
        ));
        assert!(runtime.messages.is_empty());
    }

    #[test]
    fn cancelled_turn_can_reconnect_and_complete_next_turn_without_stale_events() {
        let mut runtime = NativeAgentRuntime::new(3);
        let config = LlmConfig {
            api_key: "test-key".into(),
            base_url: "http://127.0.0.1:1".into(),
            model: "test-model".into(),
            max_tokens: 16,
            temperature: 0.0,
            context_window: None,
            provider: "openai".into(),
            thinking: teshi_engine::model_profile::DeepSeekThinking::High,
            api_style: teshi_engine::model_profile::ApiStyle::ChatCompletions,
            stream: false,
            http_headers: Default::default(),
            chat_options: Default::default(),
        };
        runtime.attach_model(config.clone());
        runtime.start();
        let (old_tx, old_rx) = mpsc::channel();
        runtime.llm_rx = Some(old_rx);
        old_tx
            .send(LlmEvent::Chunk {
                content: "queued stale".into(),
            })
            .unwrap();
        runtime.cancel();
        assert_eq!(runtime.state(), NativeTurnState::Cancelled);
        assert!(
            old_tx
                .send(LlmEvent::Chunk {
                    content: "stale".into()
                })
                .is_err()
        );

        // This is the TUI's order after cancel: start, then reconnect on the missing handle.
        runtime.start();
        runtime.attach_model(config);
        assert_eq!(runtime.state(), NativeTurnState::WaitingForModel);
        runtime.send_chat(None, Vec::new(), None).unwrap();
        let (new_tx, new_rx) = mpsc::channel();
        runtime.llm_rx = Some(new_rx);
        new_tx
            .send(LlmEvent::Done {
                full_text: "second turn".into(),
                reasoning_content: None,
                model: "test-model".into(),
                input_tokens: None,
                output_tokens: None,
                finish_reason: None,
            })
            .unwrap();
        assert!(matches!(
            runtime.try_next_event(),
            Ok(Some(NativeRuntimeEvent::Completed { .. }))
        ));
        assert_eq!(runtime.state(), NativeTurnState::Completed);
        assert_eq!(runtime.messages.len(), 1);
        assert_eq!(runtime.messages[0].content, "second turn");
    }

    #[test]
    fn tool_call_after_final_request_is_rejected_before_execution() {
        let mut runtime = NativeAgentRuntime::new(0);
        runtime.start();
        assert_eq!(
            runtime.after_tools(GenerationStage::Idle, false, false, true),
            NativeContinuation::RequestModel { allow_tools: false }
        );
        let event = runtime.consume_model_event(LlmEvent::ToolCallRequest {
            tool_calls: vec![teshi_core::llm::ToolCall {
                id: "late".into(),
                name: "get_project_info".into(),
                arguments: "{}".into(),
                execution_duration_ms: None,
            }],
            reasoning_content: None,
            input_tokens: None,
            output_tokens: None,
            finish_reason: None,
        });
        assert!(matches!(event, NativeRuntimeEvent::Failed(_)));
        assert_eq!(runtime.state(), NativeTurnState::Failed);
        assert!(runtime.messages.is_empty());
    }

    #[test]
    fn tool_batch_records_success_and_failure_in_call_order() {
        let mut runtime = NativeAgentRuntime::new(3);
        runtime.start();
        let calls = ["one", "two"].map(|id| teshi_core::llm::ToolCall {
            id: id.into(),
            name: "get_project_info".into(),
            arguments: "{}".into(),
            execution_duration_ms: None,
        });
        let assistant_idx = runtime.begin_tool_calls(calls.to_vec(), None).unwrap();
        let pending =
            runtime.execute_tool_batch(assistant_idx, &calls, |call| NativeToolExecution {
                result: if call.id == "one" {
                    Ok("project".into())
                } else {
                    Err("tool failed".into())
                },
                pending_approval: false,
                observation_message: None,
            });
        assert!(!pending);
        assert_eq!(
            runtime.messages.iter().map(|m| m.role).collect::<Vec<_>>(),
            vec![AiRole::Assistant, AiRole::Tool, AiRole::Tool]
        );
        assert_eq!(runtime.messages[1].content, "project");
        assert_eq!(runtime.messages[2].content, "Error: tool failed");
        assert_eq!(runtime.messages[2].tool_call_id.as_deref(), Some("two"));
    }

    #[test]
    fn cancelled_runtime_does_not_invoke_tool_host() {
        let mut runtime = NativeAgentRuntime::new(3);
        runtime.start();
        runtime.cancel();
        let calls = [teshi_core::llm::ToolCall {
            id: "late".into(),
            name: "get_project_info".into(),
            arguments: "{}".into(),
            execution_duration_ms: None,
        }];
        runtime.execute_tool_batch(0, &calls, |_| panic!("cancelled tool executed"));
        assert!(runtime.messages.is_empty());
    }

    #[test]
    fn completion_and_failure_are_terminal() {
        let mut runtime = NativeAgentRuntime::new(100);
        runtime.start();
        runtime.complete();
        assert_eq!(runtime.state(), NativeTurnState::Completed);
        runtime.start();
        runtime.fail();
        assert_eq!(runtime.state(), NativeTurnState::Failed);
    }

    #[test]
    fn transport_error_is_visible_and_terminal() {
        let mut runtime = NativeAgentRuntime::new(100);
        runtime.start();
        assert!(matches!(
            runtime.consume_model_event(LlmEvent::Error { message: "provider failed".into() }),
            NativeRuntimeEvent::Failed(message) if message == "provider failed"
        ));
        assert_eq!(runtime.state(), NativeTurnState::Failed);
        assert!(matches!(
            runtime.consume_model_event(LlmEvent::Chunk {
                content: "late".into()
            }),
            NativeRuntimeEvent::Ignored
        ));
    }

    #[test]
    fn streamed_tool_result_and_followup_keep_conversation_order() {
        let mut runtime = NativeAgentRuntime::new(3);
        runtime.start();
        assert!(matches!(
            runtime.consume_model_event(LlmEvent::Chunk { content: "before ".into() }),
            NativeRuntimeEvent::MessageChunk { content } if content == "before "
        ));
        assert!(matches!(
            runtime.consume_model_event(LlmEvent::Chunk { content: "tool".into() }),
            NativeRuntimeEvent::MessageChunk { content } if content == "tool"
        ));
        let call = teshi_core::llm::ToolCall {
            id: "call-1".into(),
            name: "read_feature".into(),
            arguments: "{}".into(),
            execution_duration_ms: None,
        };
        let idx = runtime
            .begin_tool_calls(vec![call], Some("reasoning".into()))
            .unwrap();
        runtime.record_tool_result("call-1".into(), "result".into());
        runtime.record_tool_duration(idx, "call-1", 12);
        assert_eq!(
            runtime.after_tools(GenerationStage::Idle, false, false, true),
            NativeContinuation::RequestModel { allow_tools: true }
        );
        runtime.append_chunk("after tool");
        runtime.finish_text("after tool".into(), None);
        assert_eq!(
            runtime.messages.iter().map(|m| m.role).collect::<Vec<_>>(),
            vec![AiRole::Assistant, AiRole::Tool, AiRole::Assistant]
        );
        assert_eq!(runtime.messages[0].content, "before tool");
        assert_eq!(
            runtime.messages[0].tool_calls.as_ref().unwrap()[0].execution_duration_ms,
            Some(12)
        );
        assert_eq!(runtime.messages[1].tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(runtime.messages[2].content, "after tool");
    }
}
