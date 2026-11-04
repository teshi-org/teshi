# language: zh-CN
# Requirement: doc-28; authored only; not bound, executed or accepted.
# Proposed behavior; product decision pending.
@tui
功能: 终端标题规则

  场景: 建议的项目与聚合状态
    假如 三个 Agent 会话中有一个运行
    当 我查看建议的终端标题
    那么 标题标识项目以及三个会话中一个运行且不泄露会话正文

  场景: 决策前保留Shell标题
    假如 终端标题规则尚未定案
    当 我启动或切换 Agent 会话
    那么 Teshi 保持终端标题不变
