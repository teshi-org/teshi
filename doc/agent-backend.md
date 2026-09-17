# Agent backend boundary

## Current request path

Before this boundary, a user message or generation request entered the TUI,
which built the system prompt, projected conversation and project context, and
submitted a chat request directly through `NativeAgentRuntime`. That runtime
sent the request to the native model worker in `teshi-engine`. Model chunks,
completion, errors, and tool calls returned through its event receiver. The
TUI executed requested Teshi tools through `AgentHost`, supplied results to
the runtime, and matched native continuation decisions. A
`propose_test_points` call advanced the Teshi generation stage to
`ReviewingTestPoints`; the runtime paused before another model request. Human
approval then resumed the workflow.

## Boundary now

Each TUI conversation owns one `AgentBackendRuntime`. `AgentBackendKind` is
chosen at conversation construction. The enum dispatches commands to the
selected implementation and reports Teshi-level status, events, and decisions.
The TUI no longer matches native events or continuation types. It still owns
prompt construction, context projection, and its current AgentHost adapter;
those moves are separate work.

`NativeAgentBackend` delegates to the existing `NativeAgentRuntime`. It maps
native events and decisions into the backend contract without changing the
native model/tool loop. The backend retains model handles, event receivers,
conversation records, and cancellation state. Frontend code has access to
conversation history and partial text through backend accessors, not native
runtime fields. The old receiver is dropped on cancellation, so late events
cannot reach a later turn. No shared mutable application object or lock is
held by the backend.

`AcpAgentBackend` is an unavailable stub. Selecting it returns an explicit
error on start or chat submission; it does not use the native implementation.
The next execution phase can connect this adapter to `teshi-acp` without
putting ACP protocol messages in the TUI or in `teshi-agent`.

## Product ownership

`AgentBackend` answers who runs the agent loop. Native means Teshi; ACP means
an external agent. It does not select a model provider, define Teshi tools,
or decide whether test points require review.

`AgentHost` supplies Teshi operations such as reading requirements, changing
feature files, running tests, and browser actions. The TUI currently adapts
those calls to the host. A future external agent may receive equivalent
capabilities through a separate bridge.

Teshi Workflow owns stages and the mandatory human test-point review gate.
Tool execution updates the workflow stage before the backend receives the
continuation facts. `WaitingForPermission` concerns whether an operation may
run; `WaitingForHumanReview` concerns the product's required review. These
states are separate. Auto or bypass permission mode cannot approve test
points. This boundary lets another frontend use the same backend event and
decision vocabulary while retaining the same workflow rules.
