## ADDED Requirements

### Requirement: 需求库通过共享 document-id 门面访问

系统 SHALL 提供面向当前用户级需求库的共享服务 API，以稳定 `document_id` 标识文档，并 SHALL 通过现有 store lock、路径安全和 revision 更新原语读取或写入。调用方 SHALL NOT 直接改写 `_teshi.json` 或需求 Markdown 来完成 list、read、body update 或 iteration 更新。

#### Scenario: 按文档 ID 读取正文

- **WHEN** 调用方使用已存在的 `document_id` 读取文档
- **THEN** 系统 SHALL 返回该文档的 title、相对 path、iteration、revision 和 Markdown body
- **AND** SHALL 从当前需求库索引解析路径，而不是要求调用方提供文件系统路径

#### Scenario: 按文档 ID 更新正文

- **WHEN** 调用方提交某个 `document_id` 的新 Markdown body 且可选 expected revision 与磁盘一致
- **THEN** 系统 SHALL 在 store lock 下写入该文档文件
- **AND** SHALL 用 `compute_document_revision` 更新索引中的 revision
- **AND** SHALL NOT 改变该文档的 id、path 或 iteration

#### Scenario: 旁路文件写入不是正式更新

- **WHEN** 某个进程直接覆盖需求 Markdown 而未调用共享服务
- **THEN** 该写入 SHALL NOT 被视为一次正式的需求库更新
- **AND** 索引 revision SHALL 保持不变，直到下一次通过服务提交正文

### Requirement: 文档引用解析到唯一 document_id

系统 SHALL 把调用方提供的引用解析为当前需求库中的唯一 `document_id`。解析顺序 SHALL 为：与 `document_id` 完全相等，然后将路径分隔符规范为 `/` 后与索引 `path` 完全相等，然后与唯一 `title` 完全相等。零匹配或非唯一匹配 SHALL 失败；系统 SHALL NOT 选择第一个候选。

#### Scenario: 使用稳定文档 ID

- **WHEN** 引用等于某个文档的 `document_id`
- **THEN** 系统 SHALL 解析到该文档
- **AND** SHALL 忽略标题或路径碰巧相同的其他文档

#### Scenario: 使用相对路径

- **WHEN** 引用在规范化分隔符后等于某个索引路径，例如 `auth/login.md` 或 `auth\login.md`
- **THEN** 系统 SHALL 解析到该文档

#### Scenario: 标题不唯一

- **WHEN** 引用不是精确 ID 或路径，且多个文档标题完全相同
- **THEN** 系统 SHALL 拒绝解析
- **AND** SHALL 返回全部候选的 `document_id` 和 path

#### Scenario: 引用不存在

- **WHEN** 引用不能精确匹配任何 ID、路径或唯一标题
- **THEN** 系统 SHALL 报告未找到
- **AND** SHALL NOT 用子串或 basename 猜测

### Requirement: 列表和过滤不改变文档身份

系统 SHALL 能列出当前需求库中的文档，并 SHALL 支持按命名 iteration 或 Unassigned 过滤。过滤 SHALL NOT 改变 `store_id`、`document_id`、path 或 revision。缺少有效 `store_id` 的需求库 SHALL fail closed。

#### Scenario: 按迭代列出

- **WHEN** 调用方请求 iteration 为 `Sprint 12` 的文档
- **THEN** 系统 SHALL 只返回该 iteration 的文档元数据
- **AND** SHALL 包含 id、title、path、iteration 和 revision

#### Scenario: 列出未分配文档

- **WHEN** 调用方请求未分配 iteration 的文档
- **THEN** 系统 SHALL 只返回 `iteration` 为空的文档

#### Scenario: 需求库未初始化

- **WHEN** 目标目录没有带有效 `store_id` 的 `_teshi.json`
- **THEN** 系统 SHALL 拒绝 list/read/update
- **AND** SHALL NOT 按路径临时生成 store 身份

### Requirement: 正文更新使用 revision 守卫

在提供 expected revision 时，系统 SHALL 仅当磁盘索引中的 revision 仍匹配才写入正文。冲突时系统 SHALL 拒绝覆盖，并 SHALL 保留磁盘上的现有正文和索引。省略 expected revision 或显式 force SHALL 允许覆盖当前磁盘正文。

#### Scenario: 并发保存发生冲突

- **WHEN** 调用方带着打开时的 revision 提交正文，但磁盘 revision 已变化
- **THEN** 系统 SHALL 拒绝更新
- **AND** SHALL 使磁盘 Markdown 和索引保持冲突前的内容

#### Scenario: 强制覆盖

- **WHEN** 调用方在 revision 已变化的情况下请求 force 更新
- **THEN** 系统 SHALL 写入新正文并更新 revision
- **AND** SHALL 仍保持文档 id、path 和 iteration 不变

#### Scenario: 正文未变化

- **WHEN** 提交的正文与当前磁盘正文相同
- **THEN** 系统 SHALL 将操作视为成功
- **AND** SHALL NOT 无故改写索引字段

### Requirement: 批量 iteration 更新是原子的

系统 SHALL 允许一次调用为多个 `document_id` 设置或清除 iteration。该更新 SHALL 在同一 store lock 下完成，SHALL NOT 改变文档 id、path、正文或 revision，并且任一 ID 缺失或 iteration 名称无效时 SHALL NOT 部分写入。

#### Scenario: 批量为文档设置迭代

- **WHEN** 调用方为三个存在的文档设置同一有效 iteration 名称
- **THEN** 系统 SHALL 把三个文档的 iteration 持久化到 `_teshi.json`
- **AND** SHALL NOT 改写它们的 Markdown 或 revision

#### Scenario: 批量中包含未知文档

- **WHEN** 任一 `document_id` 在当前索引中不存在
- **THEN** 系统 SHALL 失败
- **AND** SHALL 使全部目标文档的 iteration 保持调用前状态

#### Scenario: 清除迭代

- **WHEN** 调用方清除一个或多个文档的 iteration
- **THEN** 系统 SHALL 将它们视为 Unassigned
- **AND** SHALL NOT 删除对应 Markdown 文件
