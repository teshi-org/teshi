# language: zh-CN
# Requirement: doc-73; authored only; not bound, executed or accepted.
# Expectations describe the implemented policy; bind and replay before claiming acceptance.
@tui
功能: 终端标题规则

  场景: 运行中显示进度与项目
    假如 有一个 Agent 会话正在运行
    当 我查看终端标题
    那么 标题包含运行标记与项目名且不包含会话正文

  场景: 多个会话聚合运行状态
    假如 三个 Agent 会话中有一个正在运行
    当 我查看终端标题
    那么 标题显示三个会话中一个正在运行

  场景: 等待审批时改用等待标记
    假如 一个 Agent 会话正在等待用户审批
    当 我查看终端标题
    那么 标题显示等待标记而不是运行标记

  场景: 轮次结束后保留结果标记
    假如 一轮 Agent 会话已经结束
    当 我查看终端标题
    那么 标题显示成功或失败标记

  场景: 退出后恢复标题
    假如 终端标题已由 Teshi 更新
    当 我退出 Teshi
    那么 终端标题被复位为空

  场景: 关闭标题开关后不修改标题
    假如 用户设置了 TESHI_NO_TITLE
    当 我启动 Teshi 并运行 Agent 会话
    那么 Teshi 不修改终端标题
