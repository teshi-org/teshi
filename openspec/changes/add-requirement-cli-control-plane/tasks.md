## 1. 建立 Requirement Store Service 门面

- [x] 1.1 在 `teshi-engine` authoring 模块增加 document-id 门面，导出 `resolve_requirement_ref`、`list_requirement_documents`、`read_requirement_document`、`update_requirement_document_body` 和 `set_requirement_documents_iteration`，内部复用现有 store lock、`save_requirement_markdown` 和 iteration 规范化。
- [x] 1.2 实现引用解析：精确 `document_id`，然后将 `\` 规范为 `/` 后的精确 path，然后唯一 title；零匹配与歧义返回可区分错误，不猜测。
- [x] 1.3 让 `set_requirement_documents_iteration` 在一次 lock 内更新多个 ID；任一 ID 缺失或 iteration 名称无效时整批失败且不部分写入。单文档 `set_requirement_document_iteration` 改为调用该批量 API。
- [x] 1.4 实现 `update_requirement_document_body` 的 expected revision 守卫、force 覆盖和“正文未变则不改索引”行为。
- [x] 1.5 添加 engine 单元测试，覆盖解析顺序、路径分隔符、歧义标题、list 过滤、未初始化 store fail closed、revision 冲突、force、批量 iteration 成功与整批回滚。

## 2. 扩展 `teshi requirements` 的 list 与 show

- [x] 2.1 在 `RequirementsCommand` 增加 `List` / `Show`，支持 `--iteration`、`--unassigned`、`--json`；`--iteration` 与 `--unassigned` 互斥。
- [x] 2.2 实现文本与 JSON 输出：`list --json` 使用含 `store_id`、`store_path`、`documents` 的信封；`show` 默认只打印 Markdown；`show --json` 另含元数据和 `body`。
- [x] 2.3 实现 JSON 失败合同与退出码：成功 0，歧义 2，其余错误 1；`--json` 失败时 stdout 含稳定 `code`（含 `requirement_not_found` 与 `ambiguous_requirement_ref`）。
- [x] 2.4 添加 CLI 测试，覆盖默认 TUI 不抢占、`path`/`import-project` 仍可用、list 过滤、空结果退出 0、show 按 ID/路径/唯一标题解析，以及歧义标题退出 2。

## 3. 实现 set-iteration 与 clear-iteration

- [x] 3.1 增加 `set-iteration <ref>... --iteration <name>` 和 `clear-iteration <ref>...`；迭代名必须走 `--iteration`，拒绝把位置参数当作 iteration。
- [x] 3.2 将每个 `<ref>` 解析为唯一 ID 后调用一次 `set_requirement_documents_iteration`；任一引用失败则不写入。
- [x] 3.3 添加 CLI/集成测试，覆盖批量设置、批量清除、无效 iteration 名称、未知 ID 整批失败，以及缺少 `--iteration` 时不误解析。

## 4. 实现 edit：系统编辑器与非交互写入

- [x] 4.1 实现 TTY `edit`：把当前正文写入 `.md` 临时文件，按 `$VISUAL` → `$EDITOR` → Windows `notepad` 启动编辑器；无 TTY 且无输入时不启动编辑器。
- [x] 4.2 实现 `--file` 与 stdin 正文写入，二者互斥；提交走 `update_requirement_document_body`，并支持 `--force`。
- [x] 4.3 实现 revision 冲突：拒绝覆盖并在错误中给出临时文件路径；编辑器未改文件则退出 0 且不写索引。
- [x] 4.4 添加测试，覆盖 `--file` 成功、stdin 写入、非 TTY 缺输入失败、revision 冲突、`--force`、未变化 no-op；编辑器启动用可注入 command 避免真实 GUI。

## 5. 文档与质量验证

- [x] 5.1 更新 `doc/cli-usage.md` 和 `doc/user-guide.md`，说明五个新命令、引用解析、JSON 信封、`$EDITOR`、Windows fallback、`--file`/stdin 和 `--force`。
- [x] 5.2 确认 rustdoc 覆盖新的 public 门面与 CLI 处理函数，用户可见错误信息为英文。
- [x] 5.3 运行 `cargo fmt --all --check`、受影响 crate 定向测试，以及 native workspace `cargo check --workspace --exclude teshi-web --locked`。
- [x] 5.4 运行 native workspace `cargo test --workspace --exclude teshi-web --locked`、`cargo clippy --workspace --exclude teshi-web --locked --all-targets --all-features -- -D warnings` 和 docs gate。

## 6. Requirement CLI E2E（Teshi Gherkin + NDJSON runner）

- [x] 6.1 新增 `features/en-US` 下的 `requirement_*.feature` 与 `features/zh-CN` 下对应的中文文件名（`@cli`），用业务语言覆盖 list/show、批量 iteration、`--file` 正文写入和 JSON 失败合同；不启动 `$EDITOR`。按语言目录单独执行 CLI 文件，避免与 `@web-ui` 场景混跑。
- [x] 6.2 实现 `teshi-requirement-cli-runner`：解析 Teshi NDJSON `run` 请求，按 feature 执行 Gherkin steps，并调用真实 `teshi` 二进制。
- [x] 6.3 `teshi run` 在配置了 NDJSON runner（`--runner-cmd` / `teshi.toml [runner]`）时优先于 daemon 与 Python engine 自动探测。
- [x] 6.4 通过 `teshi run --runner-cmd teshi-requirement-cli-runner` 跑通上述 feature。
