## MODIFIED Requirements

### Requirement: TUI owns requirements gathering for feature generation

TUI SHALL 支持通过 AI Agent generation pipeline 从自由文本、对话或当前用户级需求库收集需求。当用户要求创建或生成 feature 时，agent SHALL 收集需求、提出非 Gherkin 测试点、等待显式人工审批、从已批准测试点规划场景，最后才写入 `.feature` 文件。持久化需求来源 SHALL 使用 `(store_id, document_id, document_revision, range)`，而不是项目相对路径。

#### Scenario: 用户从 chat 开始生成

- **WHEN** 用户要求 TUI agent 根据需求创建 feature
- **THEN** agent SHALL 进入 requirements gathering
- **AND** SHALL NOT 在未提交 requirements 时直接跳到测试点提议、场景规划或文件写入

#### Scenario: 需求包含粘贴文本

- **WHEN** 用户把多行需求文本粘贴到 TUI AI input
- **THEN** 系统 SHALL 接受文本并把它提供给 agent conversation
- **AND** 用户 SHALL 能把接受的文本持久化到当前用户级需求库，而不是项目目录

#### Scenario: 从全局需求库选择需求

- **WHEN** 用户从当前用户级需求库选择一个或多个持久化文档或范围开始生成
- **THEN** pipeline SHALL 保留 store、document、revision 和 range 身份作为 generation sources

## ADDED Requirements

### Requirement: AI 测试点生成支持迭代来源范围

TUI SHALL 允许用户在开始需求收集或测试点生成前选择当前需求库中的全部需求、一个命名迭代或未分配文档作为来源范围。该范围 SHALL 固定当前 `store_id`。选定单个范围后，系统 MUST 只向 agent 暴露该 store 和 iteration 范围内的需求文档，并 MUST 拒绝 `submit_requirements` 或 `propose_test_points` 对范围外文档的引用。未显式选择时 SHALL 使用当前需求库的全部需求。

#### Scenario: 用户选择一个迭代生成测试点

- **WHEN** 用户选择一个命名迭代并开始 AI 测试点生成
- **THEN** agent SHALL 只能列出和读取标注为该迭代的需求文档
- **AND** 提交的 requirement source refs SHALL 仅包含当前 `store_id`、该迭代内的稳定文档 ID 和当前 revision

#### Scenario: 用户选择未分配需求

- **WHEN** 用户选择未分配范围并开始 AI 测试点生成
- **THEN** agent SHALL 只能使用没有迭代标注的需求文档

#### Scenario: 用户不限制迭代

- **WHEN** 用户以全部需求范围开始生成
- **THEN** 系统 SHALL 允许 agent 使用当前用户级需求库中的所有有效文档
- **AND** SHALL 保持现有自由文本需求输入能力

#### Scenario: Agent 引用范围外文档

- **WHEN** `submit_requirements` 或 `propose_test_points` 引用不属于当前来源范围的 document ID
- **THEN** 系统 MUST 拒绝该工具调用并返回可操作的诊断信息
- **AND** SHALL NOT 持久化部分测试点或推进生成阶段

#### Scenario: Agent 引用其他需求库

- **WHEN** `submit_requirements` 或 `propose_test_points` 引用与 active scope 不同的 `store_id`
- **THEN** 系统 MUST 拒绝该工具调用
- **AND** SHALL NOT 在当前需求库中按相同 document ID 猜测匹配

#### Scenario: 选定范围在生成期间发生漂移

- **WHEN** 需求索引的迭代标注在来源范围选定后发生变化
- **THEN** 系统 SHALL 在读取需求、提交 requirements 和持久化测试点之前重新校验范围成员关系与 revision
- **AND** 范围不再匹配的来源 SHALL 使生成暂停并要求用户确认新的范围

### Requirement: 迭代生成范围可本地恢复

系统 SHALL 将包含 `store_id` 和 iteration filter 的当前 `RequirementSourceScope` 与可恢复生成状态一起保存在项目本地，并 SHALL 在 TUI 重启后恢复其语义。恢复操作 SHALL NOT 隐式批准测试点或跳过既有阶段门禁。旧版状态缺少 store identity 时 SHALL 要求用户确认当前全局需求库，而不是自动把旧项目文档引用绑定到它。

#### Scenario: TUI 在测试点评审期间重启

- **WHEN** 用户以一个命名迭代生成测试点，并在 Reviewing Test Points 阶段关闭 TUI
- **THEN** 重新打开项目后系统 SHALL 恢复相同的迭代来源范围
- **AND** SHALL 保持原有测试点的未批准状态

#### Scenario: 加载旧版生成状态

- **WHEN** `.teshi/generation-state.json` 不含 `RequirementSourceScope`
- **THEN** 系统 SHALL 暂停文档来源驱动的生成并要求用户确认当前需求库
- **AND** 自由文本-only 会话 MAY 在不绑定旧项目需求的情况下恢复
- **AND** SHALL NOT 自动读取旧 `<project>/requirements/`

#### Scenario: 恢复时迭代已不存在

- **WHEN** 已保存的命名迭代在当前需求索引中不再存在
- **THEN** 系统 SHALL 暂停继续生成并显示范围失效诊断
- **AND** SHALL 要求用户重新选择来源范围，而不是回退到全部需求

#### Scenario: 恢复时需求库不匹配

- **WHEN** 已保存的 `store_id` 与当前需求库 `_teshi.json` 中的身份不同
- **THEN** 系统 SHALL 暂停继续生成并显示需求库不匹配诊断
- **AND** SHALL NOT 把 source refs 解析到当前需求库
