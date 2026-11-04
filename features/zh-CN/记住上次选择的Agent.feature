# language: zh-CN

@tui
功能: 记住上次选择的Agent
  新对话沿用最近一次主动选择的后端，重启后仍然有效。
  已有对话保留各自的后端和聊天记录。

  背景:
    假如 已有隔离的Teshi配置并安装了确定性ACP代理"Fake Alpha"和"Fake Beta"
    并且 未设置Agent后端环境变量覆盖
    并且 Teshi已打开AI聊天

  场景: 首次启动没有已保存选择时使用Native
    那么 当前对话使用Teshi Native

  # test_point_ids: TP1
  场景: 新建Agent沿用最近主动选择的ACP代理
    当 我在Agent后端面板选择"Fake Alpha"
    并且 我发送"原始对话"
    那么 可以看到来自"Fake Alpha"的回复
    当 我新建Agent
    那么 当前对话使用"Fake Alpha"
    并且 当前对话没有消息
    当 我发送"新Agent对话"
    那么 可以看到来自"Fake Alpha"的回复
    并且 fake ACP轨迹记录了每个对话各自独立的会话

  # test_point_ids: TP2
  场景: 新对话命令保留所选代理并创建独立会话
    当 我在Agent后端面板选择"Fake Alpha"
    并且 我发送"新建之前"
    那么 可以看到来自"Fake Alpha"的回复
    当 我执行斜杠命令"/new"
    那么 当前对话使用"Fake Alpha"
    并且 当前对话没有消息
    当 我发送"新建之后"
    那么 可以看到来自"Fake Alpha"的回复
    并且 fake ACP轨迹记录了每个对话各自独立的会话
    并且 新ACP会话的提示中不包含"新建之前"

  # test_point_ids: TP3
  场景: 重启记住最近一次主动选择的ACP代理
    当 我在Agent后端面板选择"Fake Alpha"
    并且 我在Agent后端面板选择"Fake Beta"
    并且 我使用同一份隔离配置重启Teshi
    那么 当前对话使用"Fake Beta"
    并且 当前对话没有消息
    当 我发送"重启之后"
    那么 可以看到来自"Fake Beta"的回复

  # test_point_ids: TP4
  场景: 主动选择Native替换已保存的ACP偏好
    当 我在Agent后端面板选择"Fake Alpha"
    并且 我在Agent后端面板选择Teshi Native
    并且 我新建Agent
    那么 当前对话使用Teshi Native
    当 我执行斜杠命令"/new"
    那么 当前对话使用Teshi Native
    当 我使用同一份隔离配置重启Teshi
    那么 当前对话使用Teshi Native

  # test_point_ids: TP5
  场景: 切换对话保留各自后端和主动选择偏好
    当 我在Agent后端面板选择"Fake Alpha"
    并且 我发送"Alpha历史"
    那么 可以看到来自"Fake Alpha"的回复
    当 我新建Agent
    并且 我在Agent后端面板选择"Fake Beta"
    并且 我切换到原始Agent
    那么 当前对话使用"Fake Alpha"
    并且 当前对话中可以看到"Alpha历史"
    当 我新建Agent
    那么 当前对话使用"Fake Beta"

  # test_point_ids: TP5
  场景: 恢复历史不会替换最近主动选择
    当 我在Agent后端面板选择"Fake Alpha"
    并且 我发送"已保存的Alpha历史"
    那么 可以看到来自"Fake Alpha"的回复
    当 我执行斜杠命令"/new"
    并且 我在Agent后端面板选择"Fake Beta"
    并且 我恢复包含"已保存的Alpha历史"的已保存对话
    那么 当前对话使用"Fake Alpha"
    并且 当前对话中可以看到"已保存的Alpha历史"
    当 我新建Agent
    那么 当前对话使用"Fake Beta"
    当 我使用同一份隔离配置重启Teshi
    那么 当前对话使用"Fake Beta"

  # test_point_ids: TP6
  场景: 已删除的首选ACP代理触发可见的Native回退
    当 我在Agent后端面板选择"Fake Alpha"
    并且 我退出Teshi
    并且 从隔离的已安装代理存储中移除"Fake Alpha"
    并且 我使用同一份隔离配置重新打开Teshi
    那么 可以看到指出"Fake Alpha"不可用并回退Native的警告
    并且 当前对话使用Teshi Native

  # test_point_ids: TP6
  场景: 连接失败不会静默切换首选代理
    当 我在Agent后端面板选择"Fake Alpha"
    并且 我退出Teshi
    并且 将已安装的"Fake Alpha"配置为连接失败
    并且 我使用同一份隔离配置重新打开Teshi
    并且 我发送"连接失败探测"
    那么 可以看到ACP连接错误
    并且 当前对话使用"Fake Alpha"
    并且 没有记录到Native模型请求
