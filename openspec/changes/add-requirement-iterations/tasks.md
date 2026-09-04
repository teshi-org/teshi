## 1. 建立用户级全局需求库

- [x] 1.1 在 `teshi-engine` 增加 `requirements_data_dir()`，实现 `--requirements-root`、`TESHI_REQUIREMENTS_DIR`、`TESHI_APP_DATA_DIR` 和默认 `<app_data>/requirements` 的路径优先级，并补齐跨平台单元测试。
- [x] 1.2 将 requirements-root 参数从 CLI/desktop 启动边界传入 authoring loader，增加 `teshi requirements path` 或等价状态输出，使用户可确认实际需求库位置。
- [x] 1.3 在 `teshi-core` 定义带 rustdoc 的 `RequirementStoreId` 和 v2 `RequirementDocumentIndex.store_id`，实现首次空库初始化、稳定 serialization 与必填身份校验。
- [x] 1.4 添加全局库测试，覆盖默认目录、两类环境变量、CLI override、库整体移动、非空未初始化目录和 store identity 不匹配。

## 2. 实现旧项目需求显式导入

- [x] 2.1 增加 `teshi requirements import-project [PROJECT] --dry-run`，读取旧 v1 index、Markdown 和项目 test points，并输出目标 store、ID/path 冲突及拟执行映射。
- [x] 2.2 实现无冲突导入，保留 document IDs 和相对路径，为项目 requirement links 补充目标 `store_id`，且不删除旧项目文件。
- [x] 2.3 实现 ID/path 冲突的确定性重映射与显式确认，确保目标文档和项目 test-point links 使用同一映射。
- [x] 2.4 使用 staging、文件锁、原子 index/testpoints replacement 和失败清理 journal，保证中断后目标需求库、项目测试点与源目录仍可恢复。
- [x] 2.5 添加 migration 集成测试，覆盖 dry-run、无冲突、同内容复用、不同内容冲突、用户拒绝、写入失败回滚和重复执行。

## 3. 将 Authoring 切换到全局库

- [x] 3.1 重构 `load_authoring_artifacts` 及调用链，分别接收 `project_root` 和解析后的 `requirements_root`：需求从全局库加载，测试点继续从 `<project>/testpoints/testpoints.json` 加载。
- [x] 3.2 删除项目 `requirements/` runtime fallback；检测到旧目录时只产生带 import 命令的 migration diagnostic。
- [x] 3.3 为 `RequirementLink`、anchor resolver、reverse traceability 和 scenario generation 增加 `store_id`，store 不匹配时 fail closed。
- [x] 3.4 更新项目 test-point serialization 与兼容读取：已迁移链接使用 `(store_id, document_id)`，缺少 store identity 的旧链接显示待迁移诊断而不猜测。
- [x] 3.5 添加 core/engine/TUI 测试，覆盖跨项目共享同一需求库、项目测试点隔离、错误 store、缺失文档以及不存在项目需求 fallback。

## 4. 扩展全局需求文档的迭代元数据

- [x] 4.1 在 `RequirementDocumentMeta` 增加向后兼容的 `iteration: Option<String>`，实现 trim、控制字符校验和确定性发现迭代名称的共享 helper，并补齐 public API rustdoc。
- [x] 4.2 增加原子更新全局文档 iteration 的 engine API，保持 store/document IDs、路径、Markdown 正文和 revision 不变。
- [x] 4.3 添加模型与持久化测试，覆盖命名迭代、`Unassigned`、缺字段 JSON、无效名称、大小写区分、重载和 metadata-only 更新不影响审批。

## 5. 在 Requirements tab 浏览全局需求和迭代

- [x] 5.1 在 `AuthoringUiState` 增加 `RequirementIterationFilter`、`RequirementGroupMode`、动态迭代列表和重建 selection 的逻辑。
- [x] 5.2 重构 requirement tree，使 `Path` 保持路径层级，`Iteration` 生成“迭代 → 路径 → 文档”层级，并支持 `All`、`Named`、`Unassigned`。
- [x] 5.3 增加 iteration 选择 overlay 和文档 iteration 编辑交互；切换会隐藏当前文档时安全处理 selection，存在 `buffer_dirty` 时阻止数据丢失。
- [x] 5.4 在用户级 `AppSettings` 增加按 `store_id` 保存的 requirements view 偏好，并在命名 iteration 不存在时把 filter 恢复为 `All`。
- [x] 5.5 更新 `keymap.rs`、footer/help 和 tree 标题，使用 English 用户界面字符串并持续显示 current requirement store/path 与 filter。
- [x] 5.6 添加 TUI 测试，覆盖跨项目共享视图、filter/group 恢复、空结果、未分配、外部新迭代、无效已保存 filter 和 dirty-buffer 门禁。

## 6. 约束 AI 生成的全局需求来源

- [x] 6.1 在 `teshi-agent::pipeline` 增加包含 `store_id` 和 iteration filter 的 `RequirementSourceScope`，并为 `RequirementSourceRef` 增加 `store_id` 与系统填充的 `document_revision`。
- [x] 6.2 更新项目 `.teshi/generation-state.json` 保存与恢复，覆盖 store/iteration 恢复、store 不匹配、iteration 消失、revision 漂移和旧 document-backed session 暂停。
- [x] 6.3 在 AI generation 开始流程增加显式 source-scope 确认，显示需求库路径、store ID 和 iteration；允许使用 Requirements filter 作为建议但不隐式绑定。
- [x] 6.4 在 `teshi-agent` 增加 `list_requirement_documents` 和 `read_requirement_document` 工具 schema，在 TUI handler 中只返回 active scope 内的本地全局库数据。
- [x] 6.5 更新 system prompt 和阶段 guidance，要求按需读取 current store；避免一次性注入全部 Markdown，并明确自由文本不等同于需求库来源。
- [x] 6.6 扩展 `submit_requirements`，原子校验 store、document、iteration、Unicode range 和 revision，再填充 refs 并推进阶段。
- [x] 6.7 扩展 `propose_test_points` 和 review-to-planning 边界，在写项目 testpoints 或继续生成前重新校验所有组合引用与 active scope。
- [x] 6.8 添加工具和 pipeline 集成测试，覆盖范围内成功、错误 store、范围外文档、索引重分类、正文 revision 变化、自由文本兼容和无部分写入。

## 7. 文档与质量验证

- [x] 7.1 更新 `doc/user-guide.md`、`doc/cli-usage.md` 和 keybinding/development 文档，说明各平台默认需求目录、override、全局共享、独立 Git、iteration 和迁移流程。
- [x] 7.2 更新示例与测试 fixture，移除“项目内 `requirements/` 是运行时来源”的过时说明，并保留 migration fixture。
- [x] 7.3 运行 `cargo fmt --all --check`、受影响 crate 定向测试与 native workspace `cargo check`，修复格式和编译问题。
- [x] 7.4 运行 native workspace test、clippy `-D warnings` 和 doc gates（排除 wasm-only `teshi-web`），确认没有回归。
