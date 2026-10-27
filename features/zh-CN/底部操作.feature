# language: zh-CN
# Requirement: doc-34; partially bound; see test results for scenario status.
@tui
功能: 底部操作

  场景: 点击可见操作
    假如 需求页显示保存操作
    当 我点击保存
    那么 发生与键盘快捷键相同的保存行为

  场景: 截断操作不可点击
    假如 终端过窄以致无法显示全部底部操作
    当 我点击底部空白区域
    那么 不触发隐藏操作
