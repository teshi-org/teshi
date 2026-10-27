# Scenario checklist

Each scenario is tracked independently. Results refer to recorded runs, not an unconditional claim about later source edits. Unlisted bindings are open.

## features/en-US/browser_profiles.feature

- [ ] Teshi web shows Browser Profiles after startup — open
- [ ] Multiple profiles are not selected automatically — open
- [ ] User selects a browser profile explicitly — open
- [ ] User refreshes the browser profile list — open
- [ ] User can retry connecting Chrome from Browser Profiles — open

## features/en-US/chat_acp_status_row.feature

- [ ] shared activity row — open
- [ ] narrow status priority — open
- [ ] Settings remain accessible when options overflow — open
- [ ] Keep error and interrupted-task recovery visible — open
- [ ] Preserve the native backend status row — open

## features/en-US/chat_focus_hint.feature

- [x] Focus does not add border hints — passed (.teshi/feature-results/chat-focus-feature-20261027/matrix.json)
- [ ] Retain send and newline controls — failed (.teshi/feature-results/chat-focus-feature-20261027/matrix.json)

## features/en-US/chat_profile_title.feature

- [x] default title — passed (.teshi/feature-results/chat-title-feature-20261027/matrix.json)
- [x] custom profile title — passed (.teshi/feature-results/chat-title-feature-20261027/matrix.json)
- [x] A custom profile named Default stays identifiable — passed (.teshi/feature-results/chat-title-feature-20261027/matrix.json)

## features/en-US/chat_runtime_context.feature

- [x] Show task elapsed time across tool work — passed (.teshi/feature-results/chat-runtime-feature-rerun-20261027/matrix.json)
- [x] Unknown usage is not zero — passed (.teshi/feature-results/chat-runtime-feature-rerun-20261027/matrix.json)
- [x] Show reported occupancy and threshold — passed (.teshi/feature-results/chat-runtime-feature-rerun-20261027/matrix.json)

## features/en-US/combined_detail_layout.feature

- [x] Combine supporting details in two columns — passed (.teshi/feature-results/compact-layout-feature-20261027/matrix.json)
- [x] Keep all panes reachable on a small screen — passed (.teshi/feature-results/compact-layout-feature-20261027/matrix.json)

## features/en-US/compact_terminal_layout.feature

- [x] Keep focused content accessible — passed (.teshi/feature-results/compact-layout-feature-20261027/matrix.json)
- [ ] Resize without stale pointer targets — failed (.teshi/feature-results/compact-layout-feature-20261027/matrix.json)

## features/en-US/explore_horizontal_navigation.feature

- [x] Reach long Step text — passed (.teshi/feature-results/horizontal-feature-rerun-20261027/matrix.json)
- [x] Clamp at the content boundary — passed (.teshi/feature-results/horizontal-feature-rerun-20261027/matrix.json)

## features/en-US/explore_step_changes.feature

- [ ] Browse added and removed Steps — open
- [ ] Bound navigation at the first change — open

## features/en-US/external_feature_sync.feature

- [x] Discover additions in the configured scope — passed (.teshi/feature-results/external-context-20261027/matrix.json)
- [ ] Refresh a batch of changes — failed (.teshi/feature-results/external-context-20261027/matrix.json)
- [x] Keep a dirty deleted buffer — passed (.teshi/feature-results/external-context-20261027/matrix.json)
- [x] Defer changes during a run — passed (.teshi/feature-results/external-context-20261027/matrix.json)
- [ ] Retain the project on scan failure — failed (.teshi/feature-results/external-context-20261027/matrix.json)

## features/en-US/footer_actions.feature

- [x] Click a visible action — passed (.teshi/feature-results/footer-feature-final-20261027/matrix.json)
- [x] Clipped actions are not clickable — passed (.teshi/feature-results/footer-feature-final-20261027/matrix.json)

## features/en-US/gherkin_validation_e2e.feature

- [x] Missing Chinese step separator is diagnosed — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Valid source and attachments are accepted — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Valid Chinese source is accepted — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Unrecognized executable text is rejected — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Explicit scope is limited to the selected Feature — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] All scope reports ordered diagnostics — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Warnings do not fail validation — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Legal repeated Given steps remain non-blocking — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Directory run ignores invalid sibling Features — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Conflicting check scopes are rejected — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Unbound refuses an invalid Feature — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Next unbound preserves the active step on validation failure — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] BDD run does not start for an invalid Feature — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Replay entry points fail before target actions — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Repeated validation reports are deterministic — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Daemon warning report is structured and visible — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Daemon validation errors keep a structured report — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/en-US/pointer_feedback.feature

- [x] Highlight a clickable target — passed (.teshi/feature-results/pointer-feature-20261027/matrix.json)
- [ ] Modal coverage blocks underlying feedback — failed (.teshi/feature-results/pointer-feature-20261027/matrix.json)

## features/en-US/replay_screenshot_review.feature

- [ ] View a screenshot for a completed step — open
- [ ] Inspect an available screenshot while replay continues — open
- [ ] Retain the screenshot of a failed step — open
- [ ] Enlarge a step screenshot — open
- [ ] Keep screenshots separate between runs of the same Feature — open
- [ ] Restore screenshot associations after reopening the project — open
- [ ] Report a capture failure without changing the step result — open
- [ ] Explain when the replay backend does not support screenshots — open
- [ ] Explain when a saved image is missing or unreadable — open
- [ ] Keep screenshot evidence scoped to the selected project — open

## features/en-US/requirement_authoring.feature

- [x] Batch assignment moves both documents into an iteration — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Unknown document in a batch leaves existing assignments unchanged — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Clearing iteration returns documents to unassigned — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] File replacement updates the login document body — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Unchanged replacement does not rewrite revision — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Replacement heading synchronizes the stored title — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Replacement without a leading heading is rejected and preserves the document — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Non-interactive edit without a body is rejected — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Blank iteration name is rejected — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/en-US/requirement_creation.feature

- [x] Create from a file with an explicit project — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Create from stdin without implicit associations — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Reject a mismatched title — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Concurrent creation preserves every document — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Failed persistence leaves no partial document — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/en-US/requirement_listing.feature

- [x] Named iteration JSON list includes only assigned documents — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Unassigned JSON list excludes assigned documents — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Empty named iteration still succeeds — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Iteration and unassigned filters cannot be combined — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Unique title shows the checkout document — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Document id shows the login document — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Relative path shows the checkout document — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] JSON show includes document metadata and body — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Duplicate title is rejected as JSON — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Project list shows only linked shared documents — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Shared requirement records independent project delivery states — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Unlinking a project leaves the shared document unassigned — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Portable snapshot contains the linked source documents — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/en-US/requirement_tab.feature

- [x] Teshi without a project path still loads the requirement library — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/en-US/requirement_timestamp_names.feature

- [ ] Name a new document independently of identity — open
- [ ] Preserve historical links — open
- [ ] Avoid equal timestamp collisions — open
- [ ] Clock rollback cannot overwrite history — open

## features/en-US/run_inspect.feature

- [ ] User opens Run from the shell header — open
- [ ] Run lists Gherkin scenarios from the open project — open
- [ ] Refresh reloads the scenario list — open
- [ ] Running a listed scenario shows an event log — open
- [ ] Run inspect surface does not edit Gherkin files — open
- [ ] User can expand redacted secrets after a run — open
- [ ] User returns to Browser Profiles from Run — open

## features/en-US/terminal_title_policy.feature

- [ ] Proposed project and aggregate state — open
- [ ] Preserve shell title until a decision — open

## features/en-US/testpoint_requirement_links.feature

- [x] Listing reports each test point with its links — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Listing reports a test point that has no links — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Adding a link appends a resolved anchor — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Removing a link keeps the other links — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Retargeting a link replaces the previous anchor — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Mutating links returns an approved test point to proposed — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Rewriting a requirement body makes an anchor stale and the point needs review — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Adding a link to a document outside the store is rejected — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Adding a link with an empty range is rejected — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Adding a link past the end of the document is rejected — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Adding a link to an unknown test point is rejected — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] Removing a link that matches nothing is rejected — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/en-US/tui_acp_authoring.feature

- [ ] Generate a feature through human review and Save — open

## features/en-US/tui_acp_long_conversation.feature

- [ ] Complete thirty wrapped conversation turns — open

## features/en-US/tui_acp_model_persistence.feature

- [ ] Fresh sessions restore the confirmed model — open
- [ ] Agent preferences and existing conversations remain independent — open
- [ ] A rejected selection does not replace the saved preference — open
- [ ] An unavailable saved model falls back to the agent default — open
- [ ] A restoration timeout blocks prompts until reconciliation — open

## features/en-US/tui_acp_permission_allow.feature

- [ ] Allowing the second option continues the conversation — open

## features/en-US/tui_acp_permission_reject.feature

- [ ] Rejecting the permission ends the turn — open

## features/en-US/tui_acp_settings.feature

- [ ] Switching the model refreshes the choices that depend on it — open

## features/en-US/tui_acp_wrapped_chat.feature

- [ ] Keep the sixth wrapped reply visible — open

## features/en-US/tui_agent_activity.feature

- [ ] Show reasoning before the final answer (Native) — open
- [ ] Show reasoning before the final answer (ACP) — open
- [ ] Keep the title when an ACP tool update omits it (completed) — open
- [ ] Keep the title when an ACP tool update omits it (failed) — open
- [ ] Show native tool requests and results — open
- [ ] Reserve green reply styling for the completed final answer (Native) — open
- [ ] Reserve green reply styling for the completed final answer (ACP) — open
- [ ] Keep ACP progress and tools in event order — open
- [ ] Preserve progress styling after an interrupted turn (Native, cancellation) — open
- [ ] Preserve progress styling after an interrupted turn (Native, failure) — open
- [ ] Preserve progress styling after an interrupted turn (ACP, cancellation) — open
- [ ] Preserve progress styling after an interrupted turn (ACP, failure) — open
- [ ] Do not promote ACP progress when no final answer follows a tool — open

## features/en-US/tui_agent_selection_memory.feature

- [ ] First launch without a saved selection uses Native — open
- [ ] A new agent uses the last explicitly selected ACP agent — open
- [ ] The new command retains the selected agent with a fresh session — open
- [ ] Restart remembers the most recent explicit ACP selection — open
- [ ] Explicitly selecting Native replaces the saved ACP preference — open
- [ ] Switching conversations preserves their backends and the explicit preference — open
- [ ] Restoring history does not replace the last explicit selection — open
- [ ] A removed preferred ACP agent produces a visible Native fallback — open
- [ ] A connection failure does not silently switch the preferred agent — open

## features/en-US/tui_codebuddy_live_conversation.feature

- [ ] Complete ten real CodeBuddy turns — open

## features/en-US/tui_conversation_history.feature

- [ ] Save an unfinished reply without a lifecycle command (Native) — open
- [ ] Save an unfinished reply without a lifecycle command (ACP) — open
- [ ] Recover a checkpoint after an unexpected termination (Native) — open
- [ ] Recover a checkpoint after an unexpected termination (ACP) — open
- [ ] Restore ACP selection and execution history — open
- [ ] Supply saved context once to a new ACP process — open
- [ ] Restore a legacy conversation without backend or project metadata — open
- [ ] Keep newly saved conversations isolated by project — open
- [ ] Reopen a cancelled conversation without promoting its partial reply (Native) — open
- [ ] Reopen a cancelled conversation without promoting its partial reply (ACP) — open
- [ ] Report a checkpoint failure and recover without losing the conversation (Native) — open
- [ ] Report a checkpoint failure and recover without losing the conversation (ACP) — open
- [ ] Keep native credentials out of saved conversations — open
- [ ] Keep ACP credentials out of saved conversations — open

## features/en-US/tui_editor.feature

- [ ] Navigate between views and exit — open
- [ ] Run, edit, and save a feature — open

## features/en-US/tui_exit_performance.feature

- [ ] Quit promptly when no agent work is pending — open
- [ ] Interrupt pending ACP work when quitting — open
- [ ] Bound shutdown when an ACP agent does not cooperate — open
- [ ] Close multiple idle ACP conversations without serial delays — open
- [ ] Preserve a completed conversation across a prompt exit — open

## features/en-US/tui_project_switch.feature

- [ ] Keyboard switching loads project B — open
- [ ] Pointer selection opens a recent project — open
- [ ] Open path switches to another project — open
- [ ] Selecting the active project is a no-op — open
- [ ] Canceling the picker keeps project A active — open
- [ ] A missing project path preserves project A — open
- [ ] Switching from B back to A reloads A — open
- [ ] Invalid target configuration preserves project A — open
- [ ] A narrow picker keeps the Open path control usable — open
- [ ] Failed preparation preserves edits after choosing Discard — open
- [ ] Stopping a running task switches only after its runner tree exits — open
- [ ] Canceling a switch while ACP stops keeps project A usable — open
- [ ] Switching projects closes a stalled native model stream — open
- [ ] Project session lists and resume stay isolated after switching — open
- [ ] Restarting in project B restores only B's history — open
- [ ] Explicitly loaded legacy history remains unassigned — open
- [ ] A shared global requirement reloads each project's own links — open

## features/en-US/tui_project_switch_boundaries.feature

- [ ] Invalid target encoding preserves an unsaved project — open
- [ ] Unreadable target sources can be repaired before retrying — open
- [ ] Recent-history write failure does not undo successful activation — open
- [ ] Equal project names are distinguished by their paths — open
- [ ] Directory aliases preserve the active project's unsaved edit — open
- [ ] A failed requirement draft save can be cancelled and retried — open
- [ ] A failed test-point save retains edits and review state for retry — open

## features/en-US/tui_project_switch_failures.feature

- [ ] Undo and redo cannot restore the previous project's content — open
- [ ] Recover a failed Feature save before switching — open
- [ ] Reject pending tool permission before switching projects — open
- [ ] Stop ACP tools after the agent exits normally — open
- [ ] Switching projects does not approve a pending test-point review — open
- [ ] A stalled synchronous dispatch times out without changing projects — open
- [ ] Open a project path when recent history is empty — open

## features/en-US/winapp_capability_diagnostics.feature

- [ ] Prefer effective semantic controls — open
- [ ] Diagnose a container-only tree — open
- [ ] Do not infer absence from truncation — open

## features/en-US/winapp_owned_instances.feature

- [ ] Observe redirected second launch — open
- [ ] Stop only the owned instance — open
- [ ] Reject a lost ownership token — open

## features/en-US/winapp_visual_exploration.feature

- [ ] Operate without semantic controls — open
- [ ] Reject stale coordinates — open
- [ ] Refuse unpermitted physical input — open
- [ ] Stop an uncertain sequence — open
- [ ] Reject uncalibrated external success — open
- [ ] Bound external checker execution — open
- [ ] Capture failure preserves the action result — open
- [ ] Stable pixels do not assert business success — open
- [ ] Scroll and drag within an observed client — open

## features/winapp/negative_assertion.feature

- [ ] Missing controls can be asserted absent — open

## features/zh-CN/ACP会话设置.feature

- [ ] 切换模型会刷新依赖它的选项 — open

## features/zh-CN/ACP功能生成.feature

- [ ] 通过人工审核和保存生成功能文件 — open

## features/zh-CN/ACP换行对话.feature

- [ ] 第六轮换行回复保持可见 — open

## features/zh-CN/ACP权限允许对话.feature

- [ ] 允许第二个权限选项后对话继续 — open

## features/zh-CN/ACP权限拒绝对话.feature

- [ ] 拒绝权限请求后回合结束 — open

## features/zh-CN/ACP模型选择持久化.feature

- [ ] 新会话恢复已确认的模型 — open
- [ ] 代理偏好与已有对话互不影响 — open
- [ ] 被拒绝的模型选择不覆盖已保存偏好 — open
- [ ] 已保存模型不可用时使用代理默认模型 — open
- [ ] 恢复超时时阻止消息直至模型状态重新确认 — open

## features/zh-CN/ACP长对话.feature

- [ ] 完成三十轮换行对话 — open

## features/zh-CN/Agent对话持久化.feature

- [ ] 无需生命周期命令即可保存未完成回复 (Native) — open
- [ ] 无需生命周期命令即可保存未完成回复 (ACP) — open
- [ ] 异常终止后恢复检查点 (Native) — open
- [ ] 异常终止后恢复检查点 (ACP) — open
- [ ] 恢复ACP选择和执行历史 — open
- [ ] 向新ACP进程仅补充一次已保存上下文 — open
- [ ] 恢复没有后端或项目元数据的旧版会话 — open
- [ ] 按项目隔离新保存的会话 — open
- [ ] 重启后恢复已取消对话且不将部分回复提升为最终回答 (Native) — open
- [ ] 重启后恢复已取消对话且不将部分回复提升为最终回答 (ACP) — open
- [ ] 报告检查点保存失败并恢复对话 (Native) — open
- [ ] 报告检查点保存失败并恢复对话 (ACP) — open
- [ ] 原生代理凭据不进入已保存对话 — open
- [ ] ACP代理凭据不进入已保存对话 — open

## features/zh-CN/Agent运行过程展示.feature

- [ ] 在最终回答之前展示思考内容 (Native) — open
- [ ] 在最终回答之前展示思考内容 (ACP) — open
- [ ] ACP工具更新缺少标题时保留原工具标题 (completed) — open
- [ ] ACP工具更新缺少标题时保留原工具标题 (failed) — open
- [ ] 展示原生Agent的工具请求和结果 — open
- [ ] 仅在最终回答结束后使用绿色回复样式 (Native) — open
- [ ] 仅在最终回答结束后使用绿色回复样式 (ACP) — open
- [ ] 按事件顺序保留ACP过程发言和工具活动 — open
- [ ] 本轮中断后保留过程回复样式 (Native, 取消) — open
- [ ] 本轮中断后保留过程回复样式 (Native, 失败) — open
- [ ] 本轮中断后保留过程回复样式 (ACP, 取消) — open
- [ ] 本轮中断后保留过程回复样式 (ACP, 失败) — open
- [ ] ACP工具后没有最终回答时不提升过程回复样式 — open

## features/zh-CN/CodeBuddy真实ACP对话.feature

- [ ] 完成十轮真实 CodeBuddy 对话 — open

## features/zh-CN/Gherkin 校验自举.feature

- [x] 诊断中文步骤缺少分隔符 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 接受有效源文本和步骤附件 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 接受有效中文源文本 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 拒绝可执行区域中无法识别的文本 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 明确范围只校验选定的 Feature — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 全部范围报告有序诊断 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 警告不会导致校验失败 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 合法的重复 Given 步骤不会阻断校验 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 目录运行忽略无效兄弟 Feature — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 拒绝冲突的校验范围 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] unbound 拒绝无效 Feature — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 校验失败时 next-unbound 保留活动步骤 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 无效 Feature 不会启动 BDD 运行 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] replay 在目标操作前失败 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 重复校验报告具有确定性 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] daemon 警告报告结构完整且可见 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] daemon 校验错误保留结构化报告 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/zh-CN/TUI退出性能.feature

- [ ] 没有待处理代理任务时及时退出 — open
- [ ] 退出时中断等待中的ACP任务 — open
- [ ] ACP代理不配合时限时退出 — open
- [ ] 多个空闲ACP会话退出时不叠加串行等待 — open
- [ ] 及时退出后仍保留已完成的对话 — open

## features/zh-CN/合并细节布局.feature

- [x] 两列组合辅助细节 — passed (.teshi/feature-results/compact-layout-feature-20261027/matrix.json)
- [x] 小屏所有面板仍可访问 — passed (.teshi/feature-results/compact-layout-feature-20261027/matrix.json)

## features/zh-CN/回放截图查看.feature

- [ ] 查看已完成步骤的截图 — open
- [ ] 回放尚未结束时查看已有截图 — open
- [ ] 保留失败步骤的现场截图 — open
- [ ] 放大查看步骤截图 — open
- [ ] 同一功能的多次运行分别保留截图 — open
- [ ] 重新打开项目后恢复截图关联 — open
- [ ] 截图失败不改变步骤执行结果 — open
- [ ] 说明回放后端不支持截图 — open
- [ ] 说明已保存图片丢失或无法读取 — open
- [ ] 截图证据随当前项目切换 — open

## features/zh-CN/外部功能文件同步.feature

- [x] 发现配置范围内新增文件 — passed (.teshi/feature-results/external-context-20261027/matrix.json)
- [ ] 刷新一批变化 — failed (.teshi/feature-results/external-context-20261027/matrix.json)
- [x] 保留已删除文件的脏缓冲区 — passed (.teshi/feature-results/external-context-20261027/matrix.json)
- [x] 运行期间延后同步 — passed (.teshi/feature-results/external-context-20261027/matrix.json)
- [ ] 扫描失败保留项目 — failed (.teshi/feature-results/external-context-20261027/matrix.json)

## features/zh-CN/小屏终端布局.feature

- [x] 保证聚焦内容可访问 — passed (.teshi/feature-results/compact-layout-feature-20261027/matrix.json)
- [ ] 调整尺寸不残留点击区域 — failed (.teshi/feature-results/compact-layout-feature-20261027/matrix.json)

## features/zh-CN/底部操作.feature

- [x] 点击可见操作 — passed (.teshi/feature-results/footer-feature-final-20261027/matrix.json)
- [x] 截断操作不可点击 — passed (.teshi/feature-results/footer-feature-final-20261027/matrix.json)

## features/zh-CN/步骤横向导航.feature

- [x] 访问长步骤文本 — passed (.teshi/feature-results/horizontal-feature-rerun-20261027/matrix.json)
- [x] 停在内容边界 — passed (.teshi/feature-results/horizontal-feature-rerun-20261027/matrix.json)

## features/zh-CN/测试点需求链接.feature

- [x] 列出会报出每个测试点及其链接 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 列出会报出没有链接的测试点 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 追加链接会新增一条已解析的锚点 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 删除链接会保留其余链接 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 改指向会替换原有锚点 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 变更链接后已批准的测试点回到待审 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 需求正文改写后锚点变陈旧且测试点需复核 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 追加到需求库之外的文档会被拒绝 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 追加空区间会被拒绝 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 追加超出文档末尾的区间会被拒绝 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 向未知测试点追加链接会被拒绝 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 删除匹配不到任何链接时会被拒绝 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/zh-CN/浏览器配置选择.feature

- [ ] teshi web 启动后显示浏览器配置 — open
- [ ] 多个配置不会被自动选中 — open
- [ ] 用户显式选择一个浏览器配置 — open
- [ ] 用户刷新浏览器配置列表 — open
- [ ] 用户可从浏览器配置重试连接 Chrome — open

## features/zh-CN/点击悬停反馈.feature

- [x] 高亮可点击目标 — passed (.teshi/feature-results/pointer-feature-20261027/matrix.json)
- [ ] 弹层阻止底层反馈 — failed (.teshi/feature-results/pointer-feature-20261027/matrix.json)

## features/zh-CN/直接创建需求.feature

- [x] 从文件创建并显式关联项目 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 从标准输入创建且不隐式关联 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 拒绝标题不匹配 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 并发创建保留所有文档 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 保存失败不残留半成品 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/zh-CN/窗口实例生命周期.feature

- [ ] 观察重定向的二次启动 — open
- [ ] 仅停止本次拥有的实例 — open
- [ ] 拒绝丢失的拥有凭证 — open

## features/zh-CN/窗口能力诊断.feature

- [ ] 优先有效语义控件 — open
- [ ] 诊断仅容器树 — open
- [ ] 不从截断推断不存在 — open

## features/zh-CN/窗口视觉探索.feature

- [ ] 无语义控件时操作 — open
- [ ] 拒绝过期坐标 — open
- [ ] 拒绝未许可物理输入 — open
- [ ] 停止结果不确定的序列 — open
- [ ] 拒绝未经校准的外部成功 — open
- [ ] 限制外部检查运行时间 — open
- [ ] 截图失败保留动作结果 — open
- [ ] 像素稳定不代表业务成功 — open
- [ ] 在已观察客户区内滚动和拖动 — open

## features/zh-CN/终端功能编辑.feature

- [ ] 切换视图并退出 — open
- [ ] 执行测试并编辑保存功能文件 — open

## features/zh-CN/终端标题规则.feature

- [ ] 建议的项目与聚合状态 — open
- [ ] 决策前保留Shell标题 — open

## features/zh-CN/聊天ACP状态行.feature

- [ ] 同行活动状态 — open
- [ ] 窄屏状态优先级 — open
- [ ] 选项溢出时仍可访问设置 — open
- [ ] 保留错误和中断任务恢复入口 — open
- [ ] 保留原生后端状态行 — open

## features/zh-CN/聊天耗时与上下文.feature

- [x] 展示包含工具过程的任务耗时 — passed (.teshi/feature-results/chat-runtime-feature-rerun-20261027/matrix.json)
- [x] 未知用量不冒充零 — passed (.teshi/feature-results/chat-runtime-feature-rerun-20261027/matrix.json)
- [x] 展示实际占用与阈值 — passed (.teshi/feature-results/chat-runtime-feature-rerun-20261027/matrix.json)

## features/zh-CN/聊天聚焦提示.feature

- [x] 聚焦不增加边框提示 — passed (.teshi/feature-results/chat-focus-feature-20261027/matrix.json)
- [ ] 保留发送和换行 — failed (.teshi/feature-results/chat-focus-feature-20261027/matrix.json)

## features/zh-CN/聊天配置标题.feature

- [x] 默认标题 — passed (.teshi/feature-results/chat-title-feature-20261027/matrix.json)
- [x] 自定义配置标题 — passed (.teshi/feature-results/chat-title-feature-20261027/matrix.json)
- [x] 名为Default的自定义配置仍可识别 — passed (.teshi/feature-results/chat-title-feature-20261027/matrix.json)

## features/zh-CN/记住上次选择的Agent.feature

- [ ] 首次启动没有已保存选择时使用Native — open
- [ ] 新建Agent沿用最近主动选择的ACP代理 — open
- [ ] 新对话命令保留所选代理并创建独立会话 — open
- [ ] 重启记住最近一次主动选择的ACP代理 — open
- [ ] 主动选择Native替换已保存的ACP偏好 — open
- [ ] 切换对话保留各自后端和主动选择偏好 — open
- [ ] 恢复历史不会替换最近主动选择 — open
- [ ] 已删除的首选ACP代理触发可见的Native回退 — open
- [ ] 连接失败不会静默切换首选代理 — open

## features/zh-CN/运行并检查场景.feature

- [ ] 用户从壳标题打开运行界面 — open
- [ ] 运行界面列出当前项目中的 Gherkin 场景 — open
- [ ] 刷新会重新加载场景列表 — open
- [ ] 运行已列出的场景会显示事件日志 — open
- [ ] 运行检查界面不编辑 Gherkin 文件 — open
- [ ] 运行完成后用户可以展开已脱敏的机密 — open
- [ ] 用户从运行界面返回浏览器配置 — open

## features/zh-CN/逐条步骤差异.feature

- [ ] 浏览新增和删除步骤 — open
- [ ] 首条变化的导航边界 — open

## features/zh-CN/需求列表.feature

- [x] 按命名迭代列出 JSON 时只包含已分配文档 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 未分配 JSON 列表排除已分配文档 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 空的命名迭代仍然成功 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 迭代与未分配过滤器不能同时使用 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 唯一标题显示结账文档 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 文档 id 显示登录文档 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 相对路径显示结账文档 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] JSON 显示包含文档元数据和正文 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 重复标题在 JSON 下被拒绝 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 项目需求列表仅显示已关联的共享文档 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 共享需求分别记录每个项目的交付状态 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 解除项目关联后共享文档成为未关联需求 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 项目快照包含已关联需求的完整正文 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/zh-CN/需求时间戳命名.feature

- [ ] 文件名与身份分离 — open
- [ ] 保留历史关联 — open
- [ ] 避免同时间戳冲突 — open
- [ ] 时钟回拨不覆盖历史 — open

## features/zh-CN/需求标签页.feature

- [x] 无项目路径启动 teshi 仍能加载需求库 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/zh-CN/需求编写.feature

- [x] 批量分配把两份文档划入同一迭代 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 批次中的未知文档不会改动已有分配 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 清除迭代后文档回到未分配 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 文件替换会更新登录文档正文 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 未变化的替换不会改写修订 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 替换正文的标题会同步到存储的标题 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 替换正文缺少开头标题时被拒绝且文档保持不变 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 非交互编辑且未提供正文时被拒绝 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)
- [x] 空白迭代名被拒绝 — passed (.teshi/feature-results/cli-matrix-current-20261027/matrix.json)

## features/zh-CN/项目切换.feature

- [ ] 使用键盘切换并加载项目 B — open
- [ ] 使用指针选择最近项目 — open
- [ ] 使用 Open path 切换到另一个项目 — open
- [ ] 选择当前项目不产生切换 — open
- [ ] 取消选择器后仍保持项目 A — open
- [ ] 缺失项目路径保留项目 A — open
- [ ] 从 B 切回 A 会重新加载 A — open
- [ ] 目标配置损坏时保留项目 A — open
- [ ] 窄窗口选择器仍可使用 Open path — open
- [ ] 选择放弃后准备失败仍保留编辑 — open
- [ ] 停止运行中的任务后等待运行器进程树退出再切换 — open
- [ ] ACP 停止期间取消切换仍可使用项目 A — open
- [ ] 切换项目会关闭阻塞的原生模型流 — open
- [ ] 切换项目后会话列表和恢复保持隔离 — open
- [ ] 在项目 B 重启后只恢复 B 的历史 — open
- [ ] 显式载入的旧会话仍保持未归属 — open
- [ ] 同一全局需求重新载入各项目自己的关联 — open

## features/zh-CN/项目切换失败边界.feature

- [ ] 撤销和重做不能恢复上一个项目的内容 — open
- [ ] 修复功能文件保存失败后再切换 — open
- [ ] 拒绝待处理的工具权限后切换项目 — open
- [ ] 代理正常退出后停止 ACP 工具进程 — open
- [ ] 切换项目不得批准待处理的测试点评审 — open
- [ ] 同步分发停滞时超时并保留当前项目 — open
- [ ] 最近历史为空时通过路径打开项目 — open

## features/zh-CN/项目切换持久化边界.feature

- [ ] 目标编码无效时保留未保存的项目 — open
- [ ] 修复无法读取的目标文件后重试切换 — open
- [ ] 最近历史写入失败不撤销成功的项目激活 — open
- [ ] 通过路径区分同名项目 — open
- [ ] 目录别名保留活动项目的未保存编辑 — open
- [ ] 需求草稿保存失败后可取消并重试 — open
- [ ] 测试点保存失败时保留编辑和评审状态以供重试 — open
