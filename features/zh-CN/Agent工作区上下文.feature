@tui @workspace
Feature: Agent 工作区上下文
  从工作区成员启动的 agent 通过环境变量和 get_workspace_info 工具了解自己所在的成员和同组成员。

  Background:
    Given 隔离的工作区注册表目录

  @teshi-tp:tp-21
  Scenario: ACP agent 获得工作区环境变量
    Given 工作区 demo 的成员 beta 位于 slot 1 且检出分支 dev/b
    And Teshi 打开成员 beta
    When 操作者在 AI 标签页启动 ACP agent 并让它输出工作区环境变量
    Then 输出包含工作区 demo 的 id、名称、状态目录和 beta 的路径
    And 输出中的成员名为 beta、分支为 dev/b
    And 输出中的端口为工作区端口基址加 1
    And 成员列表 JSON 包含工作区的全部成员

  @teshi-tp:tp-21
  Scenario: 未注册目录中 agent 正常启动且没有工作区变量
    Given Teshi 打开一个未注册的项目目录
    When 操作者在 AI 标签页启动 ACP agent 并让它输出工作区环境变量
    Then agent 正常回复
    And 输出中没有任何以 TESHI_WORKSPACE 开头的变量

  @teshi-tp:tp-22
  Scenario: 内置 agent 查询工作区信息
    Given 工作区 demo 有成员 alpha 和 beta 且已设置共享构建目录
    And Teshi 打开成员 beta
    When 操作者让内置 agent 调用 get_workspace_info
    Then 该工具调用不需要参数
    And 结果包含工作区名称与 id、当前成员 beta、状态目录、共享构建目录和 beta 的端口
    And 成员列表列出 alpha 和 beta 的分支与槽位并标出 beta 为当前成员
    And 聊天中该工具调用显示为读取操作

  @teshi-tp:tp-22
  Scenario: 未注册目录中查询工作区信息给出指引
    Given Teshi 打开一个未注册的项目目录
    When 操作者让内置 agent 调用 get_workspace_info
    Then 结果说明该目录不属于任何 Teshi 工作区
    And 结果提示使用 teshi workspace new 或 attach 注册
