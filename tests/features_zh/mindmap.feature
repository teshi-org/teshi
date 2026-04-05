@mindmap
Feature: MindMap 树视图
  作为管理多个 feature 文件的 BDD 从业者
  我希望在可折叠的树中看到所有 feature，且步骤节点不包含关键字
  这样我就能发现跨 feature 的步骤复用并导航项目结构

  # --- 树结构 ---

  Scenario: 显示项目树层级
    Given 我已打开包含 feature 文件的目录
    Then MindMap 标签页显示可折叠的树
    And 根节点代表项目目录
    And 每个场景的步骤序列在根节点下显示为路径
    And 共享的步骤前缀合并为一条路径
    And 树中不显示 feature 文件节点

  Scenario: 步骤节点仅显示正文
    Given 一个场景包含步骤 "Given 我在登录页面"
    Then 树将步骤节点显示为 "我在登录页面"
    And 不显示 Given、When、Then、And 或 But 关键字

  Scenario: 树中不显示标签
    Given 一个 feature 文件在 Feature 行上方有标签 "@auth @smoke"
    Then 树中不出现标签节点
    And 树中不出现标签文字

  Scenario: 树中不显示 Examples 表
    Given 一个 Scenario Outline 有 Examples 表
    Then 树中不出现 Examples 节点
    And 树中不出现表格行节点

  Scenario: Background 步骤作为共享前缀
    Given 一个 feature 包含带步骤的 Background
    And 该 Background 之后有多个场景
    Then 树路径以 Background 步骤开头
    And Background 步骤在这些场景中作为共享前缀

  # --- 步骤复用检测 ---

  Scenario: 跨文件相同步骤前缀被合并
    Given Feature A 包含 "When 我在登录页面"
    And Feature B 包含 "Given 我在登录页面"
    Then 树显示共享步骤节点 "我在登录页面"
    And 当前缀匹配时到该节点的路径被共享

  Scenario: 共享步骤不显示复用后缀
    Given 步骤正文 "I am on the login page" 出现在 3 个场景中
    Then 树不显示类似 "[x3]" 的复用后缀

  Scenario: 唯一步骤不显示复用标记
    Given 某步骤正文只出现在一个场景中
    Then 树节点没有复用后缀

  Scenario: 共享路径提供多个预览位置
    Given 一个共享步骤路径存在于多个场景中
    Then 预览上方显示位置条 "Location 1/N"
    When 我按 ]
    Then 预览切换到另一个 Feature 和 Scenario 位置
    When 我按 [
    Then 预览切换到上一个位置

  # --- 树导航 ---

  Scenario: 在树中向下移动选中项
    Given MindMap 树已显示
    When 我按下方向键
    Then 选中项移动到下一个可见树节点

  Scenario: 在树中向上移动选中项
    Given MindMap 树已显示
    When 我按上方向键
    Then 选中项移动到上一个可见树节点

  Scenario: 折叠树节点
    Given 一个树节点已展开且有子节点
    When 我按左方向键
    Then 子节点被隐藏
    And 节点显示折叠指示

  Scenario: 展开树节点
    Given 一个树节点已折叠且有子节点
    When 我按右方向键
    Then 子节点变为可见
    And 节点显示展开指示

  Scenario: 树为只读
    Given 选中项位于树中的步骤节点
    When 我按空格键
    Then 不进入编辑模式
    And 树内容保持不变

  # --- 三阶段视图切换 ---

  Scenario: Stage 1 - 树占满全宽
    Given 我已打开一个 feature 文件目录
    Then 树面板占满终端宽度
    And 编辑器或保留面板均不可见

  Scenario: Stage 1 到 Stage 2 - 打开编辑器预览
    Given 视图处于 Stage 1
    When 我在树节点上按 Enter
    Then 视图切换到 Stage 2
    And 树面板缩至约 45% 宽度
    And 编辑器预览面板在右侧显示约 55% 宽度
    And 编辑器预览中显示对应的 feature 文件内容

  Scenario: Stage 2 - 编辑器预览跟随树选择
    Given 视图处于 Stage 2
    When 我将选中项移动到另一个树节点
    Then 编辑器预览滚动到选中节点对应的行
    And 该行在编辑器预览中高亮

  Scenario: Stage 2 - 跨文件导航自动切换缓冲区
    Given 视图处于 Stage 2
    And 编辑器预览显示 editor.feature
    When 我导航到属于 mindmap.feature 的节点
    Then 编辑器预览自动切换到 mindmap.feature
    And 视图滚动到对应行

  Scenario: Stage 2 - 编辑器预览显示完整 Gherkin（含关键字）
    Given 视图处于 Stage 2
    And 树中的一步显示为 "我在登录页面"
    Then 编辑器预览显示完整行 "Given 我在登录页面"
    And 应用 Gherkin 语法高亮

  Scenario: Stage 2 到 Stage 3 - 进入编辑器并显示保留面板
    Given 视图处于 Stage 2
    When 我在没有子节点的叶子节点上按右方向键
    Then 视图切换到 Stage 3
    And 树面板完全隐藏
    And 编辑器面板在左侧显示约 65% 宽度
    And 保留面板在右侧显示约 35% 宽度

  Scenario: Stage 2 到 Stage 3 - 光标落在选中节点行
    Given 视图处于 Stage 2
    And 选中的树节点对应 feature 文件第 10 行
    When 我按右方向键进入 Stage 3
    Then 编辑器光标位于第 10 行
    And 焦点在关键字上

  Scenario: Stage 3 - 完整编辑器功能
    Given 视图处于 Stage 3
    Then 编辑器面板中的所有 BDD 导航功能可用
    And 可以通过空格键编辑步骤正文
    And 在关键字焦点上按空格可打开步骤关键字选择器
    And 保存与退出快捷键正常工作

  Scenario: Stage 3 - 保留面板显示占位信息
    Given 视图处于 Stage 3
    Then 保留面板显示占位信息
    And 占位信息指明计划功能包括步骤实现代码
    And 占位信息指明计划功能包括 BDD 执行器
    And 占位信息指明计划功能包括测试结果

  Scenario: Stage 3 到 Stage 2 - 返回树视图
    Given 视图处于 Stage 3
    And 没有任何编辑模式激活
    And 焦点在关键字上
    When 我按左方向键
    Then 视图切换到 Stage 2
    And 树面板重新出现在左侧
    And 保留面板隐藏

  Scenario: Stage 3 到 Stage 2 - 树选择与编辑器位置同步
    Given 视图处于 Stage 3
    And 我在编辑器中从 "Scenario: 登录" 导航到 "Scenario: 搜索"
    When 我按左方向键返回 Stage 2
    Then 树选择更新为最接近编辑器光标位置的节点

  Scenario: Stage 2 到 Stage 1 - 关闭编辑器预览
    Given 视图处于 Stage 2
    When 我按 Escape
    Then 视图切换到 Stage 1
    And 编辑器预览面板隐藏
    And 树占满终端宽度
