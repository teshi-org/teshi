@project
Feature: 项目文件管理
  作为一名 BDD 从业者
  我希望打开单个文件或整个 feature 文件目录
  这样我就能处理任意规模的项目

  # --- 单文件模式 ---

  Scenario: 打开单个 feature 文件
    Given 我用 .feature 文件路径运行 teshi
    Then 该文件被加载到编辑器缓冲区
    And 光标从第一个 BDD 节点开始

  Scenario: 以空缓冲区启动
    Given 我不带任何文件参数运行 teshi
    Then 创建一个空缓冲区
    And 状态栏提示为新缓冲区

  Scenario: 单文件也使用相同的三阶段布局
    Given 我使用单个 .feature 文件运行 teshi
    Then MindMap 树显示该文件的项目根节点和步骤路径
    And 三阶段视图切换正常工作

  # --- 多文件模式 ---

  Scenario: 打开 feature 文件目录
    Given 我用目录路径运行 teshi
    Then 递归发现所有 .feature 文件
    And MindMap 树显示所有发现文件的步骤路径

  Scenario: 按字典序发现 feature 文件
    Given 目录包含 "checkout.feature"、"auth.feature" 和 "search.feature"
    When 我在 teshi 中打开该目录
    Then 项目的 feature 列表顺序为: auth, checkout, search

  Scenario: 扫描嵌套目录
    Given 目录包含带有 .feature 文件的子目录
    When 我在 teshi 中打开顶层目录
    Then 发现所有子目录中的 feature 文件

  # --- Gherkin 解析 ---

  Scenario: 解析带标签的 Feature
    Given 一个 feature 文件以 "@auth @smoke" 开头
    And 下一行是 "Feature: 用户登录"
    Then 解析得到的 BddFeature 拥有标签 "auth" 和 "smoke"
    And feature 名称是 "用户登录"

  Scenario: 标签保存在 AST 中但不显示在树中
    Given 一个 feature 文件包含标签 "@auth @smoke"
    Then 解析后的 BddFeature.tags 包含 "auth" 和 "smoke"
    And MindMap 树中不出现任何标签节点

  Scenario: 解析 feature 描述
    Given Feature 头部后面跟着自由文本描述行
    Then 解析后的 BddFeature 捕获这些描述行
    And 描述在下一个结构块之前结束

  Scenario: 解析带步骤的 Background
    Given 一个 feature 文件包含带步骤的 Background 块
    Then 解析后的 BddFeature 包含正确步骤的 background
    And 每个步骤保留其源行号

  Scenario: 解析带步骤的 Scenario
    Given 一个 feature 文件包含 Scenario 块
    Then 解析后的 BddScenario 拥有场景名称
    And 其下所有步骤按顺序被捕获
    And 每个步骤的关键字与正文被分离

  Scenario: 解析带 Examples 的 Scenario Outline
    Given 一个 feature 文件包含带 Examples 表的 Scenario Outline
    Then 解析后的 BddScenario 的 kind 为 ScenarioOutline
    And Examples 表的表头和行被捕获

  Scenario: Examples 存在 AST 中但不显示在树中
    Given 一个 Scenario Outline 的 Examples 表有 3 行数据
    Then 解析后的 ExamplesTable 包含 3 行
    And MindMap 树中不出现 Examples 节点或表格行节点

  Scenario: 步骤行号被保留
    Given 步骤 "When 我搜索商品" 位于源文件第 15 行
    Then 解析后的 BddStep 的 line_number 为 15

  # --- Step 索引 ---

  Scenario: 在所有文件上构建 step 索引
    Given 加载了包含多个 feature 文件的项目
    Then 从所有文件的所有步骤构建 StepIndex
    And 索引键为规范化后的步骤正文
    And Background 步骤也包含在 StepIndex 中

  Scenario: 步骤规范化忽略关键字
    Given Feature A 含有 "Given 我登录"
    And Feature B 含有 "When 我登录"
    And Feature C 含有 "And 我登录"
    Then StepIndex 对 "我登录" 只有一个条目，并有 3 处使用

  Scenario: 步骤规范化会去除空白并转小写
    Given 一个步骤带有前导缩进 "    Given I Log In"
    Then 规范化后的正文是 "i log in"
    And "Given  I   log in" 的规范化正文是 "i log in"

  Scenario: 复用计数反映真实使用
    Given 步骤正文 "I log in" 出现在 5 个不同场景中
    Then StepIndex 中 "I log in" 条目有 5 个使用位置
    And 每个位置引用正确的 feature、scenario 和 step 索引

  # --- 编辑与树同步 ---

  Scenario: 从 Stage 3 返回会触发重新解析
    Given 我在 Stage 3 编辑器中编辑了步骤正文
    When 我按左方向键返回 Stage 2
    Then 当前 feature 文件从缓冲区重新解析
    And 树反映更新后的结构

  Scenario: 在 Stage 3 保存会触发同步
    Given 我在 Stage 3 编辑器中编辑了步骤正文
    When 我按 s 保存
    Then 文件写入磁盘
    And 树结构被刷新

  Scenario: 编辑后 Step 索引与树同步更新
    Given 步骤正文 "I log in" 的复用计数为 3
    And 我将其中一次改为 "I sign in"
    When 树刷新
    Then 该场景的树路径使用 "I sign in"
    And StepIndex 中 "I log in" 条目有 2 个使用位置
    And StepIndex 中 "I sign in" 条目有 1 个使用位置

  Scenario: 添加新步骤会更新树
    Given 我在 Stage 3 编辑器中新增一行步骤
    When 树刷新
    Then 新步骤出现在该场景下的树中
    And StepIndex 包含新的步骤正文
