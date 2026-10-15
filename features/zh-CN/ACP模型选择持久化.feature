# language: zh-CN

@tui
功能: ACP模型选择持久化
  每个已安装的 ACP 代理记住最近一次确认成功的模型选择。
  新会话在发送第一条消息之前恢复该模型。

  背景:
    假如 已有隔离的 Teshi 配置并安装了确定性 ACP 代理"Fake Alpha"和"Fake Beta"
    并且 两个代理均公布模型"Alpha"和"Beta"且默认模型为"Alpha"
    并且 未设置代理后端环境覆盖
    并且 Teshi 已打开 AI 聊天

  # test_point_ids: ACP-MODEL-TP1, ACP-MODEL-TP2
  场景大纲: 新会话恢复已确认的模型
    假如 当前对话使用"Fake Alpha"
    当 我选择 ACP 模型"Beta"
    那么 模型变更已确认
    当 我<会话操作>
    那么 当前对话使用"Fake Alpha"
    并且 已确认的 ACP 模型为"Beta"
    当 我发送"验证恢复后的模型"
    那么 可以看到来自"Fake Alpha"的回复
    并且 ACP 记录确认新会话在第一条消息之前已使用模型"Beta"

    例子:
      | 会话操作                         |
      | 创建新代理                       |
      | 执行斜杠命令"/new"               |
      | 使用同一份隔离配置重启Teshi       |

  # test_point_ids: ACP-MODEL-TP3
  场景: 代理偏好与已有对话互不影响
    假如 原来的对话使用"Fake Alpha"且已确认模型"Beta"
    当 我创建新代理
    并且 我选择 ACP 模型"Alpha"
    那么 模型变更已确认
    当 我返回原来的对话
    那么 已确认的 ACP 模型为"Beta"
    当 我从代理后端面板选择"Fake Beta"
    那么 已确认的 ACP 模型为"Alpha"
    当 我选择 ACP 模型"Beta"
    那么 模型变更已确认
    当 我创建新代理
    并且 我从代理后端面板选择"Fake Alpha"
    那么 已确认的 ACP 模型为"Alpha"

  # test_point_ids: ACP-MODEL-TP4
  场景: 被拒绝的模型选择不覆盖已保存偏好
    假如 当前对话使用"Fake Alpha"
    当 我选择 ACP 模型"Beta"
    那么 模型变更已确认
    假如 "Fake Alpha"拒绝切换到模型"Alpha"
    当 我选择 ACP 模型"Alpha"
    那么 ACP 设置提示变更被拒绝
    并且 已确认的 ACP 模型为"Beta"
    当 我使用同一份隔离配置重启Teshi
    那么 已确认的 ACP 模型为"Beta"

  # test_point_ids: ACP-MODEL-TP5
  场景: 已保存模型不可用时使用代理默认模型
    假如 当前对话使用"Fake Alpha"
    当 我选择 ACP 模型"Beta"
    那么 模型变更已确认
    假如 "Fake Alpha"不再为新会话公布模型"Beta"
    当 我使用同一份隔离配置重启Teshi
    那么 ACP 设置提示已保存的模型不可用
    并且 已确认的 ACP 模型为"Alpha"
    当 我发送"验证默认模型"
    那么 可以看到来自"Fake Alpha"的回复
    并且 ACP 记录确认该消息使用模型"Alpha"

  # test_point_ids: ACP-MODEL-TP6
  场景: 恢复超时时阻止消息直至模型状态重新确认
    假如 "Fake Alpha"已有确认成功的模型偏好"Beta"
    并且 "Fake Alpha"在新会话恢复模型"Beta"时超时
    当 我使用同一份隔离配置重启Teshi
    那么 ACP 设置提示模型恢复超时
    当 我尝试发送"不要在模型状态不确定时发送"
    那么 ACP 记录中没有发往状态不确定会话的消息
    假如 "Fake Alpha"重新成功接受模型变更
    当 我重新连接当前 ACP 对话
    那么 已确认的 ACP 模型为"Beta"
    当 我发送"验证重新确认的模型"
    那么 可以看到来自"Fake Alpha"的回复
