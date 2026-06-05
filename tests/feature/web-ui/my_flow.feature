# language: en

@web-ui @embedded
Feature: 文件同步验证
  验证在终端中创建文件后，文件标签页能同步显示新增的文件。

  Background:
    Given teshi web is running at http://127.0.0.1:1420/?e2e=1

  Scenario: 终端新建文件后文件标签页同步刷新
    Given 用户已打开一个项目文件夹
    When 用户在终端中进入项目目录
    And 用户在终端中执行"touch .e2e-sync-test"
    Then 文件标签页中应出现".e2e-sync-test"
    And 文件列表应保持与文件系统一致
