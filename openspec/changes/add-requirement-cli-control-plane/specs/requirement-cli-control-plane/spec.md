## ADDED Requirements

### Requirement: teshi requirements 提供库级控制面命令

系统 SHALL 在现有 `teshi requirements` 命名空间下提供 `list`、`show`、`edit`、`set-iteration` 和 `clear-iteration`，并 SHALL 保留 `path` 与 `import-project`。无子命令的 `teshi` SHALL 继续启动 TUI。这些命令 SHALL 操作当前用户级需求库（尊重 `--requirements-root`），并 SHALL NOT 应用 AI generation source scope。

#### Scenario: 无子命令仍进入 TUI

- **WHEN** 用户运行 `teshi` 或 `teshi .` 且没有 `requirements` 子命令
- **THEN** 系统 SHALL 启动 TUI
- **AND** SHALL NOT 打印需求列表

#### Scenario: 显式进入需求 CLI

- **WHEN** 用户运行 `teshi requirements list`
- **THEN** 系统 SHALL 执行非交互 list 命令
- **AND** SHALL NOT 打开 TUI

#### Scenario: 现有迁移命令仍然可用

- **WHEN** 用户运行 `teshi requirements path` 或 `teshi requirements import-project --dry-run`
- **THEN** 系统 SHALL 保持现有行为

### Requirement: list 支持迭代过滤和稳定 JSON

`teshi requirements list` SHALL 列出当前需求库中的文档元数据。它 SHALL 接受互斥的 `--iteration <name>` 与 `--unassigned`，并 SHALL 接受 `--json`。JSON 成功载荷 SHALL 为包含 `store_id`、`store_path` 和 `documents` 的信封对象；每个文档 SHALL 包含 `id`、`title`、`path`、`iteration` 和 `revision`。未分配文档的 `iteration` SHALL 为 JSON `null`。空结果 SHALL 退出 0。

#### Scenario: JSON 列出某个迭代

- **WHEN** 用户运行 `teshi requirements list --iteration "Sprint 12" --json`
- **THEN** stdout SHALL 包含该迭代中每个文档的 id、title、path、iteration 和 revision
- **AND** 载荷 SHALL 包含当前 `store_id`

#### Scenario: 列出未分配文档

- **WHEN** 用户运行 `teshi requirements list --unassigned`
- **THEN** 系统 SHALL 只显示没有 iteration 的文档

#### Scenario: 同时传入冲突过滤器

- **WHEN** 用户同时提供 `--iteration` 和 `--unassigned`
- **THEN** 系统 SHALL 拒绝该命令
- **AND** SHALL NOT 列出任何文档

#### Scenario: 过滤结果为空

- **WHEN** 命名 iteration 不包含任何文档
- **THEN** 系统 SHALL 退出 0
- **AND** `--json` 输出中的 `documents` SHALL 为空数组

### Requirement: show 默认打印正文并按引用解析文档

`teshi requirements show <ref>` SHALL 把 `<ref>` 解析为唯一 `document_id`，并 SHALL 把 Markdown 正文写到 stdout。`--json` SHALL 另外包含 id、title、path、iteration、revision、`store_id` 和 `body`。歧义引用 SHALL 退出 2，并列出候选；未找到 SHALL 退出非 0。

#### Scenario: 按文档 ID 显示

- **WHEN** 用户运行 `teshi requirements show doc-12`
- **THEN** stdout SHALL 仅包含该文档的当前 Markdown 正文

#### Scenario: JSON 显示包含元数据

- **WHEN** 用户运行 `teshi requirements show doc-12 --json`
- **THEN** 输出 SHALL 包含 `body` 以及 id、path、iteration、revision 和 `store_id`

#### Scenario: 标题匹配不唯一

- **WHEN** 用户运行 `teshi requirements show "登录需求"` 且多个文档使用该标题
- **THEN** 系统 SHALL 退出 2
- **AND** SHALL 列出每个匹配的 document ID 和 path
- **AND** SHALL NOT 打印任一文档正文

### Requirement: edit 通过系统编辑器或非交互输入提交正文

`teshi requirements edit <ref>` SHALL 解析到唯一文档，并 SHALL 通过共享服务提交正文。在 TTY 且未提供 `--file` 或 stdin 正文时，系统 SHALL 把当前正文放到 `.md` 临时文件中，用 `$VISUAL` 或 `$EDITOR` 打开；两者皆空时 Windows SHALL 回退到 `notepad`，其他平台 SHALL 失败。编辑器返回后系统 SHALL 重读临时文件并更新需求库。非 TTY 写入 SHALL 要求 `--file` 或 stdin，并 SHALL NOT 启动编辑器。系统 SHALL NOT 实现内置文本编辑器，也 SHALL NOT 提供把整篇正文作为 CLI 参数传入的 `--replace`。

#### Scenario: 在 TTY 中用系统编辑器编辑

- **WHEN** 用户在 TTY 中运行 `teshi requirements edit doc-12` 且设置了 `EDITOR`
- **THEN** 系统 SHALL 打开包含当前正文的临时 `.md` 文件
- **AND** 编辑器成功退出且正文已改时 SHALL 通过服务更新需求库索引 revision

#### Scenario: 通过文件非交互写入

- **WHEN** 用户运行 `teshi requirements edit doc-12 --file body.md`
- **THEN** 系统 SHALL 读取该文件
- **AND** SHALL 通过服务保存为该文档正文
- **AND** SHALL NOT 启动编辑器

#### Scenario: 非 TTY 缺少输入

- **WHEN** stdin 不是 TTY，且没有 `--file`，也没有 stdin 正文
- **THEN** 系统 SHALL 失败
- **AND** SHALL NOT 启动编辑器

#### Scenario: 编辑期间磁盘 revision 变化

- **WHEN** 打开编辑器或读取 `--file` 之后、提交之前，磁盘 revision 已变化，且用户未传 `--force`
- **THEN** 系统 SHALL 拒绝覆盖
- **AND** 若使用了临时文件，错误 SHALL 包含该路径

#### Scenario: 编辑器中未改正文

- **WHEN** 用户退出编辑器且临时文件与打开时正文相同
- **THEN** 系统 SHALL 退出 0
- **AND** SHALL NOT 改写需求库索引

### Requirement: set-iteration 和 clear-iteration 接受多个文档引用

`teshi requirements set-iteration <ref>... --iteration <name>` SHALL 把每个引用解析为唯一 `document_id`，并 SHALL 在一次服务调用中设置 iteration。`teshi requirements clear-iteration <ref>...` SHALL 清除这些文档的 iteration。迭代名称 SHALL 通过 `--iteration` 提供，而不是位置参数。任一引用歧义、未找到或名称无效时，系统 SHALL 失败并且 SHALL NOT 部分更新。

#### Scenario: 批量为文档设置迭代

- **WHEN** 用户运行 `teshi requirements set-iteration doc-12 doc-13 --iteration "Sprint 12"`
- **THEN** 系统 SHALL 把两个文档的 iteration 持久化为 `Sprint 12`
- **AND** SHALL NOT 改变它们的 Markdown 正文或 revision

#### Scenario: 缺少 iteration 标志

- **WHEN** 用户运行 `teshi requirements set-iteration doc-12 "Sprint 12"` 且没有 `--iteration`
- **THEN** 系统 SHALL 拒绝该命令
- **AND** SHALL NOT 把 `"Sprint 12"` 当作文档引用写入

#### Scenario: 批量中有歧义引用

- **WHEN** 其中一个引用匹配多个文档
- **THEN** 系统 SHALL 退出 2
- **AND** SHALL NOT 更改任何文档的 iteration

#### Scenario: 清除多个文档的迭代

- **WHEN** 用户运行 `teshi requirements clear-iteration doc-12 doc-13`
- **THEN** 系统 SHALL 把两个文档变为 Unassigned
- **AND** SHALL 把它们留在需求库中

### Requirement: CLI 失败对 Agent 是机器可读的

当命令带 `--json` 且失败时，系统 SHALL 向 stdout 写包含稳定 `code` 的 JSON 对象，而不是只写自由文本。歧义失败 SHALL 使用 code `ambiguous_requirement_ref`，并 SHALL 包含候选 `id` 与 `path`。成功 JSON 字段名 SHALL 保持稳定。

#### Scenario: JSON 模式下文档未找到

- **WHEN** 用户运行 `teshi requirements show missing-id --json`
- **THEN** stdout SHALL 包含 `"code": "requirement_not_found"`
- **AND** 进程 SHALL 退出非 0

#### Scenario: JSON 模式下引用歧义

- **WHEN** `show` 或突变命令在 `--json` 下遇到多个标题匹配
- **THEN** stdout SHALL 包含 `"code": "ambiguous_requirement_ref"` 和 `matches`
- **AND** 进程 SHALL 退出 2

### Requirement: 控制面有 Teshi Gherkin E2E

系统 SHALL 用 Teshi 自身的 Gherkin 场景描述 requirement CLI 控制面，并 SHALL 通过 `teshi run` 的 NDJSON runner 执行这些场景。E2E SHALL 调用真实 `teshi` 二进制，隔离 `--requirements-root`，并 SHALL NOT 启动系统编辑器。

#### Scenario: teshi run 执行 requirement CLI features

- **WHEN** 用户对 `features/en-US/requirement_*.feature` 或 `features/zh-CN` 中对应的中文 `@cli` 文件运行配置了 requirement CLI NDJSON runner 的 `teshi run`
- **THEN** 系统 SHALL 执行 list/show/iteration/edit 场景
- **AND** SHALL 在全部通过时报告 `failed=0`
