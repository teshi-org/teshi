# language: zh-CN

@e2e
功能: 运行 BDD 场景并查看测试结果
  # 粗粒度：涵盖配置运行器 → 触发运行 → 接收 NDJSON 事件 → 更新 UI 状态的端到端链路。
  # 稳定契约：`teshi run` CLI 子命令（runner::run_cli）、`runner::spawn_runner` NDJSON
  #   协议、`RunEvent::CasePassed/CaseFailed/EndRun` 事件流、`apply_run_event` 更新
  #   explore_case_status、UI 中的 RunStatus 颜色映射（status_color）

  背景:
    假如 项目目录包含一个有效的 teshi.toml 配置文件，其中包含运行器命令
      并且 项目目录包含一个功能文件，其中包含一个通过场景和一个失败场景

  场景: 在 Explore 标签页中运行单个场景并观察状态变化
    假如 TUI 正在 Explore 标签页运行
      并且 我已选中一个尚未运行的场景
      并且 该场景的状态显示为"待定"（灰色）
    当 我按下 `r`
    那么 该场景的状态应变为"运行中"（黄色）
      并且 其步骤的状态也应变为"运行中"（黄色）
    当 运行器返回结果后
    那么 对于通过的场景，场景和步骤状态应变为"通过"（绿色）
      并且 对于失败的场景，场景和步骤状态应变为"失败"（红色）
      并且 在失败的步骤上按下 `Enter` 应展开显示错误详情
          （包括错误信息和堆栈跟踪）

  场景: 从 CLI 子命令运行测试并检查 NDJSON 输出
    当 我在终端中运行 `teshi run --feature login.feature --scenario "成功登录"`
    那么 标准输出应包含 NDJSON 格式的测试事件行
      并且 输出应包含一个 `"type":"start_run"` 事件
      并且 输出应包含一个 `"type":"case_passed"` 或 `"type":"case_failed"` 事件
      并且 输出应包含一个 `"type":"end_run"` 事件，包含 passed/failed/skipped 计数
