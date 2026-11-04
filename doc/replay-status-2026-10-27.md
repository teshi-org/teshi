# 回放处理记录 — 2026-10-27

本次对仓库中新增（未跟踪）的 feature 文件进行回放尝试。处理原则：能跑的跑，跑不了的标 blocker；回放过程中发现的 bug 一律只记录、不修改 feature 文件（本记录也未修改任何 feature）。

## 结论速览

- 新增 feature 共 34 个（en-US 17 + zh-CN 17，双语对照）。
- 1 对 CLI 功能已成功回放：requirement_creation / 直接创建需求（doc-23），当前源码下 5/5 通过（双语）。
- 其余 32 个（26 TUI + 6 WinApp）回放被 blocker 阻断：缺少回放录制（TUI 的 .tui.json / WinApp 的 UIA 绑定）。这些文件头注释均为 authored only; not bound, executed or accepted。
- 未发现功能性 bug：唯一可回放的功能在当前源码下完全通过。

## 回放方式与证据

### CLI（已通过）
源码构建 runner 驱动当前 teshi：
  TESHI_BIN=$PWD/target/debug/teshi.exe ./target/debug/teshi-requirement-cli-runner.exe < request.json
- en-US：end_run passed 5 failed 0（5 场景全过）
- zh-CN：end_run passed 5 failed 0（5 场景全过）

### TUI（blocker：缺 .tui.json）
直接调用 tui-runner 报 系统找不到指定的文件。 (os error 2)，即找不到 <name>.tui.json。对全部 26 个 TUI 新功能均不存在该文件（Glob 确认）。

### WinApp（blocker：缺 UIA 绑定）
  teshi winapp replay --feature <f> --dry-run --non-interactive
返回 replay_plan 中 executable_steps 为 0，gaps 每项 status=unbound reason=missing_binding。6 个 WinApp 新功能均无 .bindings.json（Glob 确认）。

## 逐文件结果

[类型 CLI=已通过 / TUI=缺.tui.json / WinApp=缺 UIA 绑定]
en-US: requirement_creation.feature CLI 5/5 (doc-23)
en-US: chat_acp_status_row.feature TUI blocker (doc-26)
en-US: chat_focus_hint.feature TUI blocker (doc-29)
en-US: chat_profile_title.feature TUI blocker (doc-25)
en-US: chat_runtime_context.feature TUI blocker (doc-27)
en-US: combined_detail_layout.feature TUI blocker (doc-36)
en-US: compact_terminal_layout.feature TUI blocker (doc-31)
en-US: explore_horizontal_navigation.feature TUI blocker (doc-33)
en-US: explore_step_changes.feature TUI blocker (doc-32)
en-US: external_feature_sync.feature TUI blocker (doc-30)
en-US: footer_actions.feature TUI blocker (doc-34)
en-US: pointer_feedback.feature TUI blocker (doc-35)
en-US: requirement_timestamp_names.feature TUI blocker (doc-24)
en-US: terminal_title_policy.feature TUI blocker (doc-28 产品决策未定)
en-US: winapp_capability_diagnostics.feature WinApp blocker (doc-22)
en-US: winapp_owned_instances.feature WinApp blocker (doc-20)
en-US: winapp_visual_exploration.feature WinApp blocker (doc-21)
zh-CN: 直接创建需求.feature CLI 5/5 (doc-23)
zh-CN: 聊天ACP状态行.feature TUI blocker (doc-26)
zh-CN: 聊天聚焦提示.feature TUI blocker (doc-29)
zh-CN: 聊天配置标题.feature TUI blocker (doc-25)
zh-CN: 聊天耗时与上下文.feature TUI blocker (doc-27)
zh-CN: 合并细节布局.feature TUI blocker (doc-36)
zh-CN: 小屏终端布局.feature TUI blocker (doc-31)
zh-CN: 步骤横向导航.feature TUI blocker (doc-33)
zh-CN: 逐条步骤差异.feature TUI blocker (doc-32)
zh-CN: 外部功能文件同步.feature TUI blocker (doc-30)
zh-CN: 底部操作.feature TUI blocker (doc-34)
zh-CN: 点击悬停反馈.feature TUI blocker (doc-35)
zh-CN: 需求时间戳命名.feature TUI blocker (doc-24)
zh-CN: 终端标题规则.feature TUI blocker (doc-28 产品决策未定)
zh-CN: 窗口能力诊断.feature WinApp blocker (doc-22)
zh-CN: 窗口实例生命周期.feature WinApp blocker (doc-20)
zh-CN: 窗口视觉探索.feature WinApp blocker (doc-21)

## 发现的问题 / 记录

以下均为缺口/阻塞/待办，非 feature 内容错误；未改动任何 feature 文件。

1. [BLOCKER-普遍] 32 个新功能缺回放录制。26 个 TUI 功能缺 <name>.tui.json，6 个 WinApp 功能缺 UIA 绑定（.bindings.json）。每个文件头均标注 authored only; not bound, executed or accepted。回放需要先完成 runner wiring（录制终端会话/绑定原生控件），该工作通常在 Windows 桌面端 teshi-desktop 录制完成，命令行只能运行已有录制，没有 TUI 录制模式。

2. [ISSUE] 预构建 runner 已过期。.teshi/feature-runtime/bin-v5/teshi-requirement-cli-runner.exe 直接对 requirement_creation 的 5 个场景全部报 unimplemented Given step；只有从源码 cargo build -p teshi-requirement-cli-runner 重新构建的 runner 才能正确回放（已验证 5/5）。feature-runtime 中发布的 runner 产物与源码不同步，直接用它回放会假失败。

3. [PENDING-非bug] 部分功能描述的行为尚未实现或决策未定，即使补齐录制也无法通过：
   - terminal_title_policy/终端标题规则（doc-28）：头注释 Proposed behavior; product decision pending。
   - winapp_*（doc-20/21/22）：原生窗口验收仍未验证。
   - external_feature_sync（doc-30）：专用回放仍缺失。
   - compact_terminal_layout/explore_*/footer_actions/pointer_feedback/requirement_timestamp_names/combined_detail_layout（doc-24/31/33-36）：逐场景回放仍缺失。

4. [无功能 bug] 唯一可回放的功能 requirement_creation（双语）在当前源码下均 5/5 通过，没有发现缺陷。

## 后续建议

- TUI 功能：在 Windows 桌面端 teshi-desktop 录制每个场景的 .tui.json（launch + 按键/断言），之后 teshi tui <feature> 即可回放。
- WinApp 功能：为每步绑定 UIA 并连接原生窗口后，用 teshi winapp replay 回放。
- 同步 feature-runtime 中的预构建 runner，避免过期产物导致假失败。
- 对尚未实现/决策未定的功能（见上第 3 条），在源头完成实现或决策后再补回放。
