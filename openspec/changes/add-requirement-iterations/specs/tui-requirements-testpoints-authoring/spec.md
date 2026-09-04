## MODIFIED Requirements

### Requirement: Requirement documents are durable project artifacts

系统 SHALL 将需求文档作为 Markdown 文件保存在当前用户级需求库中，并 SHALL 维护独立于显示标题和本地文件系统路径的稳定 `(store_id, document_id)` 身份。TUI 重启后 SHALL 从该需求库恢复需求层级和当前文档内容，并 SHALL NOT 使用项目内 `requirements/` 作为运行时来源。

#### Scenario: 用户创建需求文档

- **WHEN** 用户在 Requirements tab 创建需求文档
- **THEN** 系统 SHALL 在当前用户级需求库中持久化 Markdown 和稳定身份
- **AND** 文档 SHALL 出现在所有使用该需求库的项目 Requirements tree 中

#### Scenario: TUI 重启

- **WHEN** 用户在当前需求库已初始化的情况下重新打开项目
- **THEN** TUI SHALL 从用户级需求库重建相同的需求层级和文档身份

#### Scenario: 索引中的文档缺失

- **WHEN** 全局需求索引引用的 Markdown 已不存在
- **THEN** TUI SHALL 报告缺失文档
- **AND** SHALL NOT 静默把其身份分配给其他文件

### Requirement: Requirement links support resilient arbitrary text ranges

每个 test-point-to-requirement link SHALL 使用 `store_id` 和 `document_id` 标识来源，SHALL 支持任意非空文本范围，并 SHALL 同时保存字符位置以及包含原文和上下文的 quote selector。只有 `store_id` 与当前需求库匹配时系统才 SHALL 解析链接。文档编辑后系统 SHALL 使用 selector 重新解析，并 SHALL 将歧义、缺失或错误 store 的链接标记为 stale，而不是选择无关文本。

#### Scenario: 未变化文档按位置解析

- **WHEN** 保存的需求库 ID 和文档 revision 匹配，且保存位置仍选择相同 quote
- **THEN** 系统 SHALL 把链接解析到该范围

#### Scenario: 需求库不匹配

- **WHEN** 测试点引用的 `store_id` 与当前需求库不同
- **THEN** 系统 SHALL 报告来源不匹配
- **AND** SHALL NOT 在错误需求库中搜索相同 `document_id`

#### Scenario: 文本移动但内容未变

- **WHEN** 保存位置不再匹配，但 exact quote 和 context 在引用文档中唯一确定一个范围
- **THEN** 系统 SHALL 把链接重新锚定到该唯一范围

#### Scenario: Quote 变得有歧义

- **WHEN** 多个范围匹配且保存的 context 无法唯一确定一个范围
- **THEN** 系统 SHALL 将链接标记为 stale
- **AND** SHALL NOT 静默选择任一匹配范围

#### Scenario: 选择多字节文本

- **WHEN** 链接覆盖包含多字节 Unicode 字符的文本
- **THEN** 持久化字符 offset SHALL 在重载后保持相同的用户可见范围

## ADDED Requirements

### Requirement: 需求文档携带本地迭代元数据

系统 SHALL 允许每个需求文档声明至多一个用户定义的迭代名称，并 SHALL 将该值与稳定文档 ID、路径、标题和 revision 一同保存在全局需求库索引中。迭代名称 SHALL 是去除首尾空白后的非空文本；字段缺失 SHALL 表示文档未分配迭代。改变迭代 SHALL NOT 改变需求库或文档稳定 ID、Markdown 路径或内容 revision。

#### Scenario: 用户给需求文档分配迭代

- **WHEN** 用户在 Requirements tab 中为文档设置一个有效迭代名称并保存
- **THEN** 系统 SHALL 将迭代名称持久化到当前 `<requirements_root>/_teshi.json`
- **AND** 重新打开项目后 SHALL 恢复相同的迭代标注

#### Scenario: 用户清除需求文档的迭代

- **WHEN** 用户清除已分配的迭代并保存
- **THEN** 系统 SHALL 将该文档视为未分配
- **AND** SHALL NOT 删除或移动需求 Markdown 文件

#### Scenario: 旧版索引不含迭代字段

- **WHEN** 系统加载一个文档元数据均不含迭代字段的全局需求库索引
- **THEN** 系统 SHALL 成功加载这些文档
- **AND** SHALL 将它们显示在未分配集合中
- **AND** SHALL NOT 仅因加载而改写索引

#### Scenario: 迭代标注发生变化

- **WHEN** 用户只修改需求文档的迭代标注而不修改 Markdown 正文
- **THEN** 系统 SHALL 保持文档 revision 不变
- **AND** SHALL 保持已有 requirement links 与测试点审批状态不变

#### Scenario: 迭代名称无效

- **WHEN** 用户尝试保存空白迭代名称或包含控制字符的迭代名称
- **THEN** 系统 SHALL 拒绝该值并显示可操作的诊断信息
- **AND** SHALL 保留此前完整可读的需求索引

### Requirement: Requirements tab 支持按迭代过滤和分组

Requirements tab SHALL 提供全部迭代、单个命名迭代和未分配三种过滤范围，并 SHALL 提供按路径或按迭代分组的浏览方式。按迭代分组时，系统 SHALL 以迭代为顶层节点，并在每个迭代内保留既有路径层级。系统 SHALL 按当前 `store_id` 将最近的 filter/group 偏好保存在用户级 app settings；这些偏好 SHALL 只改变可见树，不改变需求库数据。

#### Scenario: 用户过滤到一个命名迭代

- **WHEN** 用户选择一个当前存在的迭代作为过滤范围
- **THEN** Requirements tree SHALL 只显示标注为该迭代的文档
- **AND** 状态栏 SHALL 显示当前迭代范围

#### Scenario: 用户过滤未分配文档

- **WHEN** 用户选择未分配过滤范围
- **THEN** Requirements tree SHALL 只显示没有迭代标注的文档

#### Scenario: 用户按迭代分组全部文档

- **WHEN** 用户在全部迭代范围中启用按迭代分组
- **THEN** Requirements tree SHALL 为每个命名迭代和未分配集合显示顶层节点
- **AND** 每个顶层节点内 SHALL 按文档相对路径重建原有层级

#### Scenario: 当前文档被过滤隐藏

- **WHEN** 用户切换过滤范围且当前选中文档不属于新范围
- **THEN** 系统 SHALL 选择新范围内第一个可见文档，或在范围为空时清除选择
- **AND** 编辑缓冲区存在未保存修改时 SHALL 阻止会丢失修改的切换并提示先保存或取消

#### Scenario: 外部编辑引入新的迭代名称

- **WHEN** 项目重新加载后需求索引包含此前未见过的有效迭代名称
- **THEN** 过滤器和分组树 SHALL 从本地索引自动发现该迭代
- **AND** SHALL NOT 要求访问远程 registry 或项目管理服务

#### Scenario: 用户重新打开不同项目

- **WHEN** 用户在同一需求库中设置 filter/group 偏好后重新打开任意项目
- **THEN** TUI SHALL 从用户级 app settings 恢复该 `store_id` 对应的偏好
- **AND** 无效或已删除的命名迭代过滤值 SHALL 安全回退为 `All`
