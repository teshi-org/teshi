# language: en
#
# Regression: Remove-Module PSReadLine -Force 导致 ConPTY 将终端查询回复序列
# 误识别为 Enter 按键，使嵌入式终端提示符无限循环。
#
# 修复方案：改用 Set-PSReadLineOption -PredictionSource None，
# 保留 PSReadLine 模块（保持正常输入处理），仅关闭预测补全的 VT 重绘。
#
# Ref: crates/teshi-runtime/src/terminal.rs EMBEDDED_SHELL_INIT
#   commit 63f4832

@web-ui @embedded @regression @bug
Feature: 嵌入式终端不会自动重复 Enter
  卸载 PSReadLine 后，.NET ConsoleHost 回退到原始输入处理，
  会将 ConPTY 设备属性回复序列误读为 \r，导致提示符无限循环。
  保持 PSReadLine 加载（仅关闭预测功能）可避免此问题。

  Background:
    Given 已经进入项目页面
    And 已经打开项目
    And 已经切换到 Terminal 标签页

  Scenario: 切换 Terminal 标签页后提示符不会自动重复
    When 切换到 Terminal 标签页
    Then 提示符应在 3 秒内只出现一次
    And 不应看到自动执行的空命令

  Scenario: 手动按 Enter 仍正常工作
    Given 终端提示符已就绪
    When 我在空提示符上按 Enter
    Then 应出现一个全新的提示符，且只出现一次
