## Context

Teshi 当前把 authoring 数据拆在项目内：`requirements/_teshi.json` 建立需求文档稳定 ID、路径、标题和 revision，`requirements/**/*.md` 保存正文，`testpoints/testpoints.json` 保存该项目的测试点。这个模型默认“一份需求只服务一个项目”，不适合产品需求横跨多个代码仓库和任意工作目录的场景。

Teshi 已有统一用户级 app data 根目录：优先使用非空 `TESHI_APP_DATA_DIR`，否则使用 `dirs::data_dir()/teshi`；Windows 默认是 `%APPDATA%\teshi`。本变更在该根下划分 `requirements/` 作为默认全局需求库，同时允许用户把整个库放到其他磁盘或独立 Git 仓库。

该变更横跨 `teshi-engine` 路径解析和 app settings、`teshi-core` authoring identity、`teshi-tui` Requirements/AI 状态、`teshi-agent` generation session，以及旧项目 authoring 数据迁移。需求必须保持 local-first、普通文件可编辑、路径可移动，并且不能同时读取全局和项目目录形成两个事实来源。

## Goals / Non-Goals

**Goals:**

- 完全取消项目内 `requirements/` 作为运行时来源，所有项目默认共享用户级需求库。
- 默认把需求库放在 `<app_data>/requirements`，并提供清晰的 CLI 与环境变量覆盖。
- 使用稳定 `store_id` 和 `document_id`，使需求库移动后引用仍然可靠。
- 让测试点继续归属具体项目，但通过 `(store_id, document_id)` 链接全局需求。
- 在全局需求元数据中保存 iteration，并在 TUI 中过滤、分组和恢复视图偏好。
- 在 AI generation session 中固定需求库、迭代和 revision，防止范围静默漂移。
- 为现有项目需求提供显式、安全、可检查的导入流程。

**Non-Goals:**

- 不保留 `<project>/requirements/` fallback 或双写兼容模式。
- 不把所有项目的测试点合并到用户级需求库。
- 不支持同时挂载多个需求库；一次 Teshi 进程只有一个 current requirement store。
- 不实现远程同步、账户级云存储、多人并发编辑或项目管理平台集成。
- 不在 GPUI/web/daemon 中增加需求 authoring UI。
- 不让一份需求文档同时属于多个迭代。

## Decisions

### 1. 默认需求库位于 Teshi app data 的 `requirements/` 子目录

新增共享路径解析 API `requirements_data_dir()`，按以下优先级返回绝对路径：

1. 当前命令显式传入的 `--requirements-root <PATH>`；
2. 非空 `TESHI_REQUIREMENTS_DIR`；
3. `app_data_dir().join("requirements")`。

`app_data_dir()` 继续遵循 `TESHI_APP_DATA_DIR`。因此 Windows 默认位置为：

```text
%APPDATA%\teshi\requirements\
```

Linux 通常为 `$XDG_DATA_HOME/teshi/requirements`，macOS 由 `dirs::data_dir()` 解析。CLI override 只作用于本次进程，环境变量适合固定自定义库；默认路径不写入项目配置。

备选方案是使用 `dirs::document_dir()/Teshi`。它更容易被用户发现，但会引入新的根目录约定，与 Teshi 已有 backup/override 机制分离。采用 app data 子目录，并通过 TUI、`teshi requirements path` 和文档明确展示实际位置。

### 2. 全局需求库是带稳定身份的普通文件目录

默认布局：

```text
<requirements_root>/
├── _teshi.json
└── **/*.md
```

新建全局索引使用 schema version 2，并要求顶层 `store_id`：

```json
{
  "version": 2,
  "store_id": "reqstore-...",
  "documents": []
}
```

`store_id` 在首次初始化时生成，此后移动、复制路径或通过 override 打开都不改变。文档完整身份是 `(store_id, document_id)`；绝对路径从不写入测试点或 generation state。

空目录可在首次 authoring 时初始化。非空目录缺少 `_teshi.json`，或索引缺少有效 `store_id` 时 fail closed，并提示 initialize/import；不得按当前路径临时生成身份。

备选方案是把路径当作 store identity。路径跨机器和移动不稳定，也可能让同名 document ID 解析到错误库，因此不采用。

### 3. 项目内需求只允许显式导入，不参与运行时读取

所有 authoring 加载入口改为接收解析后的 `requirements_root`，不再调用 `project_root.join("requirements")`。若检测到 `<project>/requirements/`，TUI 只显示 migration diagnostic；读取、筛选和 AI 工具仍只使用 current global store。

提供 `teshi requirements import-project [PROJECT]`：

1. 读取旧 v1 index、Markdown 和项目 `testpoints/testpoints.json`；
2. 计算 ID/path 冲突和目标写入计划；
3. 无冲突时保留 document IDs；冲突时产生确定性的新 ID/path 映射；
4. 经用户确认后，将文档、目标 v2 index 和项目 test-point links 写入 staging；
5. 验证完整结果后原子替换目标 index 与项目 testpoints；
6. 保留旧项目文件，由用户在验证后删除。

目标文档文件无法跨目录整体原子提交，因此导入 journal 记录已复制文件；失败时删除本次新建且未被目标 index 引用的文件，并保留旧源数据。导入命令支持 dry-run，默认不覆盖不同内容。

不采用自动导入，因为多个项目可能包含相同路径或 document ID，静默合并会错误改变 traceability。

### 4. Iteration 保存在全局文档元数据中

`RequirementDocumentMeta` 境加：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub iteration: Option<String>
```

`None` 是唯一未分配表示，UI 显示 English `Unassigned`。保存前去除首尾空白，拒绝空值和控制字符；其他 Unicode 文本保持原样并区分大小写。迭代集合从全局 index 动态推导，不建立独立 registry。

改变 iteration 只原子写入全局 `_teshi.json`，不会移动 Markdown、更换 IDs、改变正文 revision 或重置测试点审批状态。

### 5. Requirement links 使用 store 与 document 组合身份

`RequirementSourceRef` 和 `RequirementLink` 增加 `store_id`。AI source ref 另外记录系统解析的 `document_revision`：

```rust
RequirementSourceRef {
    store_id,
    document_id,
    document_revision,
    range,
}
```

`testpoints/testpoints.json` 继续位于具体项目，scenario refs 也保持项目相对路径。这样同一全局需求可被多个项目分别实现和测试，而每个项目保留自己的测试点审批与 Gherkin traceability。

resolver 必须先比较 `store_id`，再查找 `document_id`；store 不匹配时不得在当前库猜测同 ID 文档。旧项目测试点在 import 时补充目标 store identity；未迁移的旧 links 保持 invalid 并给出操作指引。

### 6. Requirements view 使用全局库并把偏好保存到用户级 settings

`AuthoringUiState` 增加：

```rust
enum RequirementIterationFilter {
    All,
    Named(String),
    Unassigned,
}

enum RequirementGroupMode {
    Path,
    Iteration,
}
```

树先按 filter 筛选，再按 `Path` 或“iteration → path”构建。浏览和编辑均作用于 current global store；从不同项目打开 Requirements tab 会看到同一份文档。

filter/group 是用户对需求库的视图偏好，不属于某个项目。`AppSettings` 增加按 `store_id` 索引的 `requirements_views`，保存在 `<app_data>/settings.json`。命名 iteration 已不存在时恢复为 `All`；group mode 可继续恢复。

备选方案是保存在项目 `.teshi/settings.json`。这会导致同一全局需求库在不同项目中产生互相矛盾的视图状态，因此不采用。

### 7. Generation scope 固定 store 与 iteration，仍保存在项目 session

在 `teshi-agent::pipeline` 定义：

```rust
struct RequirementSourceScope {
    store_id: RequirementStoreId,
    iteration: IterationFilter,
}
```

该 scope 属于一次项目 generation，因此保存在 `<project>/.teshi/generation-state.json`。TUI 在进入 Gathering 前显式显示并确认 store path、store ID 和 iteration；Requirements view filter 可以作为默认建议，但不会隐式控制 AI。

session restore 时：

- store ID 不匹配：暂停，绝不把 refs 绑定到当前库；
- iteration 不存在：暂停并要求重选；
- 文档离开 iteration 或 revision 改变：保留 session，要求重新确认 sources；
- 旧 session 缺少 store ID：只有 free-text-only 路径可兼容恢复，文档 refs 必须通过 import/重选重建。

### 8. Agent 通过受 scope 约束的本地只读工具访问需求

新增：

- `list_requirement_documents`：返回 active scope 内 ID、标题、相对路径、iteration、revision；
- `read_requirement_document`：按组合身份返回 active scope 内 Markdown 和 revision。

handler 每次从 current global store 重新检查 store ID、iteration membership 和 revision，不依赖 prompt 自律。`submit_requirements` 验证全部 refs 后由系统填入 revision，再一次性推进阶段；`propose_test_points` 在写项目 testpoints 前再次验证 requirement links。

正文按需读取，不一次性注入 system prompt，以限制 context 大小。所有读取来自本地文件，不引入网络服务。

## Risks / Trade-offs

- [把用户文档放在隐藏 app data 下不易发现] → 在 TUI、CLI status 和文档中持续显示实际路径，并支持 `teshi requirements open/path`。
- [Windows Roaming AppData 可能不适合大型需求库] → 支持 `TESHI_REQUIREMENTS_DIR` 和 `--requirements-root`，允许迁移到 Documents、独立磁盘或 Git checkout。
- [用户切换到错误需求库导致项目引用失效] → 用 required `store_id` fail closed，不按路径或 document ID 猜测。
- [项目内旧需求不再自动出现] → 启动时检测并给出明确 import 命令；提供 dry-run、冲突计划和源文件保留。
- [多个项目导入时发生 ID/path 冲突] → 预计算确定性映射，并在同一操作中重写该项目 test-point links。
- [全局 `_teshi.json` 成为并发写热点] → 使用进程级/文件锁、稳定排序和原子替换；检测磁盘 revision 变化后拒绝覆盖。
- [需求库与项目 Git 生命周期分离] → 需求库仍为普通 Markdown/JSON，可独立初始化 Git；Teshi 不隐式执行同步。
- [同一全局需求修改会影响多个项目] → 每个项目 link 保存 revision，变化后各项目按现有规则独立进入 `NeedsReview`。

## Migration Plan

1. 增加 `requirements_data_dir()`、CLI/env override、v2 global index 和 `store_id`，先以只读路径诊断验证。
2. 扩展 source/link identity、resolver 和 serialization；保持旧 JSON 可解析为“待迁移”，但不让其绑定到未知 store。
3. 实现 `teshi requirements import-project --dry-run` 与确认写入，覆盖无冲突、冲突、失败恢复和 test-point link rewrite。
4. 将 TUI authoring loader 切换到全局库，并删除项目 `requirements/` runtime fallback。
5. 增加 iteration metadata、全局 filter/group、用户级 view settings。
6. 增加 generation scope、只读工具以及所有读取/写入边界校验。
7. 更新用户文档，明确默认目录、override、备份、独立 Git 和迁移流程。

回滚到旧版本后，旧程序不会理解 v2 global store，也不会自动读取它；导入过程中保留的项目旧需求可供旧版本使用。新版本不会双写两个位置。

## Open Questions

无。实现基线为：一个 current 用户级需求库、默认 `<app_data>/requirements`、无项目 fallback、稳定 `store_id`、项目级 test points、显式 import 和用户级视图偏好。
