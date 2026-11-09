# language: zh-CN

@tui
功能: Agent 工作区上下文
  从工作区成员启动的 agent 通过环境变量和 get_workspace_info 工具了解自己所在的成员和同组成员。

  背景:
    假如 隔离的工作区注册表目录

  @teshi-tp:tp-21
  场景: ACP agent 获得工作区环境变量
    假如 工作区 demo 的成员 beta 位于 slot 1 且检出分支 dev/b
    并且 Teshi 打开成员 beta
    当 操作者在 AI 标签页启动 ACP agent 并让它输出工作区环境变量
    那么 输出包含工作区 demo 的 id、名称、状态目录和 beta 的路径
    并且 输出中的成员名为 beta、分支为 dev/b
    并且 输出中的端口为工作区端口基址加 1
    并且 成员列表 JSON 包含工作区的全部成员

  @teshi-tp:tp-21
  场景: 未注册目录中 agent 正常启动且没有工作区变量
    假如 Teshi 打开一个未注册的项目目录
    当 操作者在 AI 标签页启动 ACP agent 并让它输出工作区环境变量
    那么 agent 正常回复
    并且 输出中没有任何以 TESHI_WORKSPACE 开头的变量

  @teshi-tp:tp-22
  场景: 内置 agent 查询工作区信息
    假如 工作区 demo 有成员 alpha 和 beta 且已设置共享构建目录
    并且 Teshi 打开成员 beta
    当 操作者让内置 agent 调用 get_workspace_info
    那么 该工具调用不需要参数
    并且 结果包含工作区名称与 id、当前成员 beta、状态目录、共享构建目录和 beta 的端口
    并且 成员列表列出 alpha 和 beta 的分支与槽位并标出 beta 为当前成员
    并且 聊天中该工具调用显示为读取操作

  @teshi-tp:tp-22
  场景: 未注册目录中查询工作区信息给出指引
    假如 Teshi 打开一个未注册的项目目录
    当 操作者让内置 agent 调用 get_workspace_info
    那么 结果说明该目录不属于任何 Teshi 工作区
    并且 结果提示使用 teshi workspace new 或 attach 注册
