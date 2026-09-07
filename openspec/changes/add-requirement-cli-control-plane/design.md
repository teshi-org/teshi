## Context

Teshi 的需求文档已从项目目录迁到用户级全局库。稳定身份是 `(store_id, document_id)`，正文是普通 Markdown，iteration 存在 `_teshi.json`。Engine 已提供带 store lock 的写入入口：

- `load_authoring_artifacts`
- `save_requirement_markdown`
- `set_requirement_document_iteration`

`teshi requirements` 命名空间已经存在，但只有 `path` 和 `import-project`。日常浏览、正文提交和 iteration 标注仍只能走 TUI。`apps/teshi-cli` 仍是薄启动器：`web` 走 daemon，其余进入 `teshi_tui::run`，真正的 CLI 分发在 `crates/teshi-tui/src/cli/`。

TUI Agent 的 `list_requirement_documents` / `read_requirement_document` 绑定 generation source scope，不能当作库级自动化边界。本变更要补上这个边界，并让 CLI 成为同一 Service 的第一个适配器。

## Goals / Non-Goals

**Goals:**

- 在 Engine 增加面向 `document_id` 的 Requirement Store Service，作为 TUI、CLI 和后续 Agent/MCP 的共享写入/读取门面。
- 扩展 `teshi requirements`，提供 `list` / `show` / `edit` / `set-iteration` / `clear-iteration`。
- 保持 `teshi` 无子命令时进入 TUI。
- 用稳定 JSON 合同服务 Agent；用 `$VISUAL`/`$EDITOR` 服务人类正文编辑。
- 引用解析统一到 `document_id`；突变命令在歧义时失败。
- 正文提交带 `document_revision` 乐观并发守卫。

**Non-Goals:**

- 不在 CLI 内实现文本编辑器，也不提供 `--replace "整篇 Markdown"`。
- 不新增 `create` / `delete` / `rename` / `move` / `search` / `validate` / `watch`。
- 不按 iteration filter 做盲目批量（例如把全部 Unassigned 划进某个迭代）。
- 不把 CLI 套上 AI generation source scope。
- 不在本变更重写 TUI Requirements 编辑器，也不新增 MCP requirement tools 或 GPUI authoring UI。
- 不引入 `RequirementStoreService` trait 或多后端抽象；当前只有本地文件 store。

## Decisions

### 1. CLI 继续留在 `teshi-tui`，只扩展现有 `teshi requirements`

`teshi requirements path|import-project` 已经由 `crates/teshi-tui/src/cli/requirements.rs` 处理。新命令加入同一 `RequirementsCommand` 枚举。`apps/teshi-cli` 不增加第二套 clap 树。

无子命令的 `teshi` 仍进入 TUI。全局 `--requirements-root` 继续作用于所有 `requirements` 子命令。

备选方案是把 requirement CLI 搬到 `teshi-cli`。这会拆开现有分发，且与 `teshi steps` / `teshi browser` 不一致，因此不采用。

### 2. Engine 增加函数式门面，而不是 trait

在 `teshi-engine` authoring 模块新增 document-id 门面（例如 `authoring/service.rs`），内部调用现有 lock/path/revision 原语：

```text
resolve_requirement_ref(index, query) -> document_id
list_requirement_documents(root, filter)
read_requirement_document(root, document_id)
update_requirement_document_body(root, document_id, body, expected_revision)
set_requirement_documents_iteration(root, document_ids, iteration)
```

`set_requirement_documents_iteration` 在**一次** store lock 内更新多个文档，避免批量 CLI 对每个 ID 反复加锁。现有单文档 `set_requirement_document_iteration` 可保留为薄封装，或改为调用批量 API。

`update_requirement_document_body` 先按 ID 查路径，再调用 `save_requirement_markdown`。调用方不得直接写 Markdown 或改 `_teshi.json`。

不抽取 trait：没有第二存储后端。TUI 本变更不强制迁完；CLI 是第一消费者。后续 TUI/Agent 应改调同一组函数，而不是复制文件逻辑。

### 3. 引用解析：精确 ID，然后路径，然后唯一标题

`query` 去首尾空白后按以下顺序解析，命中即停：

1. 与某个 `document_id` 完全相等；
2. 将 `\` 规范为 `/` 后与某个索引 `path` 完全相等；
3. 与某个 `title` 完全相等，且该标题在库中唯一。

零匹配 → `requirement_not_found`。ID 唯一命中以外的步骤若得到多个文档 → `ambiguous_requirement_ref`，并返回全部候选的 `id` 与 `path`。子串、basename、大小写折叠都不用于突变命令。

`list` 的 `--iteration` / `--unassigned` 是过滤，不是引用解析。`edit` / `show` / `set-iteration` / `clear-iteration` 的每个 `<ref>` 都必须 resolve 到唯一 ID。

### 4. `set-iteration` 的值始终走 `--iteration`

位置参数无法同时表达多个 ID 和迭代名。采用：

```bash
teshi requirements set-iteration doc-12 doc-13 --iteration "Sprint 12"
teshi requirements clear-iteration doc-12 doc-13
```

至少需要一个 `<ref>`。`--iteration` 复用 `normalize_iteration_name()`。清空迭代只用 `clear-iteration`，不用空字符串。

### 5. `edit` 编辑临时副本，提交走 Service

不让编辑器直接写库内 Markdown，避免未提交的半成品或 revision 冲突后磁盘与索引分叉。

流程：

1. `read_requirement_document` 得到 body 与 revision；
2. 写入带 `.md` 后缀的临时文件；
3. TTY 下启动 `$VISUAL`，否则 `$EDITOR`；都未设置时，Windows 回退 `notepad`，其他平台失败并提示设置 `EDITOR`；
4. 编辑器退出后重读临时文件；
5. 正文未变则成功退出且不写库；
6. 否则以打开时的 revision 调用 `update_requirement_document_body`；
7. 磁盘 revision 已变则拒绝覆盖，错误中给出临时文件路径，用户可用 `--file` 重试或加 `--force`。

非交互写入：

```bash
teshi requirements edit doc-12 --file body.md
teshi requirements edit doc-12 < body.md
```

`--file` 与 stdin 管道互斥。非 TTY 且无 `--file`、无 stdin 数据时失败，不启动编辑器。`--force` 跳过 revision 检查，仍走 Service。

Agent 的主路径是 `list --json`、`show`、`set-iteration`；正文写入用 `--file`/stdin，而不是 `$EDITOR`。

### 6. JSON 是稳定 Agent 合同，默认文本只给人看

`--json` 出现时，成功和失败都往 stdout 写 JSON；文本模式下成功写 stdout、诊断写 stderr。

`list --json`：

```json
{
  "store_id": "reqstore-...",
  "store_path": "C:\\Users\\...\\requirements",
  "documents": [
    {
      "id": "doc-12",
      "title": "登录需求",
      "path": "auth/login.md",
      "iteration": "Sprint 12",
      "revision": "..."
    }
  ]
}
```

未分配时 `iteration` 为 `null`。用户最初的“顶层数组”可读性更好，但 Agent 需要 `store_id`，因此使用信封对象。字段名从第一版起视为稳定。

`show` 默认只打印 Markdown 正文，便于重定向。`show --json` 在同一信封字段上增加 `body`。

失败对象包含稳定 `code`，例如 `requirement_not_found`、`ambiguous_requirement_ref`、`revision_conflict`、`requirement_store_uninitialized`、`invalid_iteration_name`、`editor_unavailable`。歧义时附带 `matches: [{id, path, title}]`。

退出码：成功 `0`；歧义 `2`；其余错误 `1`。

`list --iteration NAME` 与 `--unassigned` 互斥。空库或过滤为空仍退出 `0`，JSON 中 `documents` 为空数组。未初始化或缺少 `store_id` 的非空目录 fail closed。

### 7. CLI 是库级操作，不套 generation scope

`teshi requirements list/show` 看到当前全局库中符合 filter 的全部文档。TUI Agent 工具继续受 `RequirementSourceScope` 约束。两个读取面，一个存储实现。

### 8. 第一版不迁 TUI 交互，但禁止新的旁路写入

`authoring_tab.rs` 的 modal editor、dirty-buffer 守卫和 `create_document` 内存草稿保持原样。本变更新增的写入必须走 Service。文档写明后续 TUI save/iteration 应改调同一门面，作为跟进而不是本变更阻塞项。

## Risks / Trade-offs

- [TTY `edit` 对 Agent 无用] → 第一版同时提供 `--file`/stdin；Agent 文档强调 JSON 读接口和 iteration 写入才是自动化主路径。
- [编辑器直接改库文件会导致冲突后磁盘与索引分叉] → 编辑临时副本，仅在 Service 提交时写库。
- [TUI 内存 dirty buffer 与 CLI 提交互相覆盖] → revision 守卫只对磁盘索引生效；文档说明不要 CLI 编辑当前 TUI 未保存文档。文件监视留到以后。
- [模糊标题匹配误改文档] → 突变命令只接受唯一精确匹配；歧义退出 `2` 并列出候选。
- [批量 iteration 多次加锁拉长竞态窗口] → 一次 lock 更新全部 ID；任一 ID 不存在则整批失败、不部分写入。
- [Windows 未设置 `EDITOR`] → 回退 `notepad`；无 TTY 时不回退，直接失败。
- [JSON 顶层数组与信封对象的选择] → 选信封以携带 `store_id`；在文档和 `--help` 中写明，避免 Agent 猜测。

## Migration Plan

1. 落地 Service 门面和单元测试（resolve、list/filter、read、body update、revision 冲突、批量 iteration）。
2. 扩展 clap 与 `handle_requirements_command`，先做 `list`/`show`，再做 iteration，最后做 `edit`。
3. 更新 `doc/cli-usage.md` 和 `doc/user-guide.md`。
4. 保留 `path` / `import-project` 行为不变。
5. 用 Teshi Gherkin + NDJSON runner 覆盖 list/show/edit/iteration 控制面。

回滚：删除新子命令即可；不改 `_teshi.json` schema，旧版本仍能读库。新 CLI 写入的 iteration/正文与现有 TUI 格式兼容。

## Open Questions

无。实现基线为：现有 `teshi requirements` 命名空间、函数式 Engine 门面、精确引用解析、`--iteration` 标志、临时文件 `$EDITOR`、stdin/`--file` 非交互写入、JSON 信封对象。
