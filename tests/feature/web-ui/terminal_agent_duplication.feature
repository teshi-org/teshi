# language: en

@web-ui @embedded @bug
Feature: Agent 逐字符输入终端时输出重复
  当 Agent 通过 `type` action 或 `writeTerminal` 逐字符输入命令时，
  PowerShell PSReadLine 的行内预测重绘会在 xterm.js 中累积为可见的重复行，
  而不是覆盖同一行。

  Background:
    Given 已经进入项目页面
    And 已经打开项目
    And 已经切换到 Terminal 标签页

  Scenario: 逐字符输入命令不产生重复输出
    When 我在终端中逐字符输入 "echo DUP_MARKER_A"
    Then 终端的 DUP_MARKER_A 应只出现一次
