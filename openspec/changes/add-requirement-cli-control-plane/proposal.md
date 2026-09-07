## Why

全局需求库已经有稳定 `(store_id, document_id)`、store lock 和 Engine 写入入口，但 Agent、脚本和外部工具仍无法在不理解 `_teshi.json`、路径和 revision 的情况下操作它。TUI 适合人编辑正文；现在需要把 CLI 做成同一 Requirement Service 的命令式控制面，而不是再实现一套文本编辑器。

## What Changes

- 在 `teshi-engine` 增加面向 `document_id` 的 Requirement Store Service 门面：`resolve`、`list`、`read`、`update body`、`set/clear iteration`。TUI、CLI 和后续 Agent/MCP 都通过该门面访问需求库，而不是各自改文件。
- 扩展现有 `teshi requirements` 命名空间，在保留 `path` 与 `import-project` 的同时增加：
  - `list`（`--iteration`、`--unassigned`、`--json`）
  - `show <ref>`
  - `edit <ref>`（TTY 调用 `$VISUAL`/`$EDITOR`；非交互走 stdin 或 `--file`）
  - `set-iteration <ref...>` / `clear-iteration <ref...>`
- `teshi` 无子命令时继续进入 TUI；只有显式 `teshi requirements ...` 才走非交互 CLI。
- 文档引用优先解析稳定 `document_id`，并允许唯一的相对路径或标题；匹配不唯一时失败并列出候选，突变命令不得猜测。
- `edit` 与非交互正文写入在提交前校验 `document_revision`；磁盘 revision 变化时拒绝覆盖，除非 `--force`。
- **不做** CLI 内置 Markdown 编辑器，也 **不做** `--replace "整篇正文"` 这种参数化整文接口。
- 第一版 **不做** `create`、`delete`、`rename`、`move`、`search`、`validate`、按 filter 的盲目批量、MCP requirement tools，以及 GPUI/web authoring UI。

## Capabilities

### New Capabilities

- `requirement-store-service`: 以 `(store_id, document_id)` 为身份的共享需求库门面，覆盖解析、列表、读取、正文更新和 iteration 元数据更新，并包含 store lock 与 revision 守卫。
- `requirement-cli-control-plane`: 把上述门面暴露为 `teshi requirements` 命令，提供人类可读输出、稳定 JSON 合同、系统编辑器和工作流安全失败。

### Modified Capabilities

- （无）现有 TUI authoring 与 generation-scope 行为不变；CLI 是库级控制面，不套用 AI generation source scope。`global-requirement-library` 仍在 `add-requirement-iterations` 中，本变更扩展其已落地的 `teshi requirements` 命名空间，但不改该 capability 的库布局或导入合同。

## Impact

- **Engine**: 在现有 `load_authoring_artifacts`、`save_requirement_markdown`、`set_requirement_document_iteration` 之上增加 document-id 门面和 `resolve_document`；不引入第二套文件写入路径。
- **CLI**: `crates/teshi-tui/src/cli/requirements.rs` 与 `RequirementsCommand` 增加日常命令；`apps/teshi-cli` 保持薄启动器。
- **TUI / Agent**: Requirements tab 和 generation 工具不改交互合同。后续应逐步改调同一 Service；本变更不重写 `authoring_tab.rs` 编辑器。
- **Docs**: 更新 `doc/cli-usage.md` 与 `doc/user-guide.md`，说明 JSON 字段、引用解析、`$EDITOR` 以及 Windows fallback。
- **E2E**: 用 Teshi Gherkin + NDJSON runner 覆盖 `teshi requirements` 控制面（`features/en-US/requirement_*.feature` 与 `features/zh-CN/` 下的中文 `@cli` 文件名）。
- **Compatibility**: 无 breaking CLI 变更；现有 `path` / `import-project` 保留。JSON 字段从第一版起视为稳定 Agent 合同。
- **Out of scope**: CLI 文本编辑器、delete/rename/move/search/history/tag、create/validate 命令、MCP requirement tools、远程同步。
