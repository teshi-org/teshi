# 回放推进记录（续）— 2026-10-27

在 doc/replay-status-2026-10-27.md 的基础上，本次实际动手补齐回放录制（runner wiring）。

## 已验证可回放（已落盘 .tui.json 并通过）

- features/en-US/chat_profile_title.feature — 场景 default title：通过。
  - 关键：步骤文本需去掉 Gherkin 关键字（Given/When/Then），binding.text 必须与解析后的 step.text 完全一致，否则报 stale。
- features/en-US/footer_actions.feature — 场景 Click a visible action：通过。
  - 验证路径：launch → 按 4 进 Requirements → 页脚出现 Save [Ctrl+s] → click_text Save → 仍在 Requirements。

## 已证明的回放管线

1. 建立隔离 fixture 工程 .teshi/feature-replays/fixture/（project/requirements/appdata/...），launch 指向 target/debug/teshi.exe。
2. 按 feature 的步骤行号/文本编写 <name>.tui.json（actions: launch/key/write/click_text/expect_text）。
3. 用 `teshi tui <feature> --scenario <名>` 回放；失败时 runner 会把实际屏幕 dump 到 .teshi/tui-results/example-*/screen.txt，据此修正 expect_text。

## 关键发现：多数新功能无法用当前 text-only tui-runner 忠实回放

逐文件细读后发现，32 个缺回放的功能里，能靠“默认启动 + 键盘/点击 + 文本断言”忠实回放的只是少数；其余需要特殊前置或当前 runner 不支持的断言类型：

- external_feature_sync（doc-30）：场景要求在“运行期间外部增删文件/保持脏缓冲”等，runner 的 action 集合没有“外部改文件”原语，无法回放。
- combined_detail_layout / compact_terminal_layout / explore_horizontal_navigation（doc-31/33/36）：断言两栏布局、横向滚动位置、焦点切换后的可见性——属于视觉/布局断言，文本断言只能间接覆盖，且需要特定前置（窄终端 cols、超长 Step）。
- chat_runtime_context / chat_acp_status_row / chat_focus_hint（doc-25/26/27/29）：需要“进行中的 ACP 对话/工具调用/焦点切换”等运行时状态，默认启动无法进入。
- requirement_timestamp_names（doc-24）：需要创建并保存需求库文档、再校验文件名——可用 ExpectFile，但需驱动 Requirements 创建/保存流程。
- pointer_feedback（doc-35）：依赖鼠标悬停样式，PTY 无真实指针，click_text 只能“点击”不能“悬停”。
- terminal_title_policy（doc-28）：产品决策未定，行为未实现，回放必然失败。
- winapp_*（doc-20/21/22）：需原生窗口 + UIA 绑定，命令行只能在窗口已连接时回放。

结论：盲目把 32 个功能都“写个 .tui.json 让它过”会产出与产品行为不符的假通过。应当只对能忠实回放的少数功能落盘真实录制；其余按“缺前置/runner 不支持/行为未实现”分类记录为 blocker，不伪造通过。

## 建议的后续优先级（待确认）

A. 仅对“可忠实回放”的子集继续补齐录制（如 chat_profile_title 其余场景需注入自定义 profile fixture；footer_actions 场景2需窄终端 fixture；requirement_timestamp_names 需驱动创建/保存）。
B. 对 external_feature_sync / 布局/滚动/焦点 类功能，先扩展 runner 断言能力或提供专用 harness，再补录制。
C. winapp_* 需连接原生窗口后由 winapp replay 补绑定。

本记录未改动任何 feature 文件。
