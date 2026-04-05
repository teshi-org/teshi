@editor
Feature: BDD 编辑器
  作为一名 BDD 从业者
  我希望使用结构感知的快捷键来导航和编辑 Gherkin feature 文件
  这样我能高效编写和维护测试场景

  # --- 导航 ---

  Scenario: 在 BDD 节点间向下导航
    Given 我已打开一个 feature 文件
    And 光标位于一个 BDD 节点上
    When 我按下方向键
    Then 光标移动到下一个 BDD 节点
    And 跳过非结构行

  Scenario: 在 BDD 节点间向上导航
    Given 我已打开一个 feature 文件
    And 光标位于一个 BDD 节点上
    When 我按上方向键
    Then 光标移动到上一个 BDD 节点

  Scenario: 在关键字焦点与正文焦点间切换
    Given 光标位于步骤行
    And 焦点在关键字上
    When 我按右方向键
    Then 焦点切换到正文

  Scenario: 在正文焦点与关键字焦点间切换
    Given 光标位于步骤行
    And 焦点在正文上
    When 我按左方向键
    Then 焦点切换到关键字

  Scenario: 正文链垂直导航
    Given 焦点在正文上
    And 光标位于步骤行或可编辑标题行
    When 我按下方向键
    Then 光标移动到下一条步骤行或可编辑标题行
    And 链中包含 Scenario 和 Feature 标题行

  Scenario: 跳转到第一个 BDD 节点
    Given 我已打开一个 feature 文件
    When 我按 Home 键
    Then 光标移动到第一个 BDD 节点

  Scenario: 跳转到最后一个 BDD 节点
    Given 我已打开一个 feature 文件
    When 我按 End 键
    Then 光标移动到最后一个 BDD 节点

  Scenario: Page 导航
    Given 我打开了包含很多节点的 feature 文件
    When 我按 PageDown
    Then 光标前进约 10 个 BDD 节点
    When 我按 PageUp
    Then 光标后退约 10 个 BDD 节点

  # --- 步骤正文编辑 ---

  Scenario: 激活步骤正文编辑
    Given 光标位于步骤行
    And 焦点在正文上
    When 我按空格键
    Then 步骤输入模式激活
    And 光标移动到行尾

  Scenario: 在步骤输入模式中输入字符
    Given 步骤输入模式已激活
    When 我输入可打印字符
    Then 字符插入到光标位置

  Scenario: 提交步骤正文编辑
    Given 步骤输入模式已激活
    When 我按 Enter
    Then 步骤输入模式关闭
    And 编辑内容保留在缓冲区中

  Scenario: Backspace 遵守关键字边界
    Given 步骤输入模式已激活
    And 光标在正文起始位置
    When 我按 Backspace
    Then 不删除字符
    And 光标不移动

  Scenario: 步骤输入模式下的 Delete 键
    Given 步骤输入模式已激活
    And 光标不在行尾
    When 我按 Delete
    Then 删除光标后的字符

  Scenario: 使用 Escape 取消编辑
    Given 步骤输入模式已激活
    When 我按 Escape
    Then 步骤输入模式关闭

  # --- 步骤关键字选择器 ---

  Scenario: 打开步骤关键字选择器
    Given 光标位于步骤行
    And 焦点在关键字上
    When 我按空格键
    Then 步骤关键字选择器打开
    And 当前关键字被预选

  Scenario: 导航关键字选择器
    Given 步骤关键字选择器已打开
    When 我按下方向键
    Then 选中项移动到下一个关键字选项

  Scenario: 确认关键字选择
    Given 步骤关键字选择器已打开
    And 已高亮不同的关键字
    When 我按 Enter
    Then 缓冲区中的步骤关键字被替换
    And 选择器关闭

  Scenario: 取消关键字选择
    Given 步骤关键字选择器已打开
    When 我按 Escape
    Then 选择器关闭
    And 原关键字被保留

  Scenario: 头部行不可用关键字选择器
    Given 光标位于 Feature 头部行
    And 焦点在关键字上
    When 我按空格键
    Then 不会打开选择器
    And 状态信息提示仅限步骤行

  # --- 标题与描述编辑 ---

  Scenario: 编辑 Feature 标题
    Given 光标位于 Feature 头部行
    And 焦点在正文上
    When 我按空格键
    Then 步骤输入模式在冒号后的标题文本处激活

  Scenario: 编辑 Scenario 标题
    Given 光标位于 Scenario 头部行
    And 焦点在正文上
    When 我按空格键
    Then 步骤输入模式在冒号后的标题文本处激活

  Scenario: 编辑 feature 描述行
    Given 光标位于 Feature 头部下方的描述行
    When 我按空格键
    Then 步骤输入模式激活
    And 编辑从该行第 0 列开始

  # --- 语法高亮 ---

  Scenario: Gherkin 关键字高亮
    Given 已加载一个 feature 文件
    Then 结构关键字以不同颜色渲染
    And 步骤关键字以不同颜色渲染

  Scenario: 标签高亮
    Given 行以类似 "@smoke" 的标签开头
    Then 标签以标签颜色渲染

  Scenario: 字符串高亮
    Given 步骤包含类似 "alice" 的带引号字符串
    Then 引号内字符串以字符串颜色渲染

  Scenario: 表格与文档字符串高亮
    Given 一个 feature 文件包含以 "|" 开头的表格行
    Then 表格分隔符被高亮
    And 文档字符串标记被高亮

  # --- 文件操作 ---

  Scenario: 保存文件
    Given 缓冲区已被修改
    When 我按 s
    Then 文件写入磁盘
    And 状态栏显示保存路径
    And 脏标记被清除

  Scenario: 未保存更改退出需确认
    Given 缓冲区已被修改
    When 我按 q
    Then 状态栏显示确认信息
    When 我再次按 q
    Then 应用退出

  Scenario: 无未保存更改直接退出
    Given 缓冲区未被修改
    When 我按 q
    Then 应用立即退出

  # --- 标签切换 ---

  Scenario: 在标签间切换
    Given MindMap 标签页处于活动状态
    When 我按 2
    Then Help 标签页变为活动状态
    When 我按 1
    Then MindMap 标签页再次变为活动状态

  Scenario: 标签切换清除编辑状态
    Given 编辑器面板的步骤输入模式已激活
    When 我切换到 Help 标签页
    Then 步骤输入模式关闭
    And 所有打开的关键字选择器关闭
