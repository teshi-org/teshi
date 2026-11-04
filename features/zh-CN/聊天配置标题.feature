# language: zh-CN
# Requirement: doc-25; source-pinned PTY case replay; see results.
# Product decision approved.
@tui
功能: 聊天配置标题

  场景: 默认标题
    假如 使用内置 Default 配置的聊天
    当 我打开聊天
    那么 标题省略 Default 后缀

  场景: 自定义配置标题
    假如 多个自定义 Agent 配置
    当 我切换当前配置
    那么 标题明确显示当前 Agent 配置

  场景: 名为Default的自定义配置仍可识别
    假如 有一个名称为 Default 的自定义 Agent 配置
    当 我使用该配置打开聊天
    那么 标题显示 Agent: Default
