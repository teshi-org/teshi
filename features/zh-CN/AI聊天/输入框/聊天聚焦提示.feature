# language: zh-CN
# Requirement: doc-29; source-pinned PTY case replay; see results.
@tui
功能: 聊天聚焦提示

  场景: 聚焦不增加边框提示
    假如 带 token 统计的聊天输入框
    当 我反复聚焦和移出输入框
    那么 仅边框高亮变化且 token 统计仍然可见

  场景: 保留发送和换行
    假如 已聚焦的聊天输入框
    当 我插入换行后发送消息
    那么 消息保留换行且只发送一次
