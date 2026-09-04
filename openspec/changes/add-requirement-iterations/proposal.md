## Why

当前需求文档被锁定在单个项目的 `requirements/` 下，无法自然服务于跨多个项目和目录的产品需求；同时文档缺少所属迭代这一业务维度，用户难以聚焦当前迭代，也无法约束 AI 只基于该范围生成测试点。需要将需求提升为 Teshi 用户级、local-first 的共享数据，并让迭代成为浏览和生成范围的一等元数据。

## What Changes

- **BREAKING**: 不再把 `<project>/requirements/` 作为需求文档运行时来源；需求统一由用户级需求库提供。
- 在 Teshi app data 下增加默认需求库 `<app_data>/requirements/`，并支持 `--requirements-root`、`TESHI_REQUIREMENTS_DIR` 和 `TESHI_APP_DATA_DIR` 覆盖。
- 为需求库增加稳定 `store_id`，使项目测试点和 AI 会话通过 `(store_id, document_id)` 引用需求，而不依赖绝对路径。
- 为每个需求文档增加可选的迭代标注，并将其持久化在全局需求库的 `_teshi.json` 中；未标注文档归入 `Unassigned`。
- 在 TUI Requirements tab 中增加按迭代过滤和按迭代分组浏览，选择结果只影响视图，不改变文档路径、稳定 ID 或内容 revision。
- 在用户级 `settings.json` 中按 `store_id` 保存 Requirements tab 的最近 filter/group 偏好。
- 在 AI 测试点生成入口增加迭代范围选择；选定迭代后，需求收集和 `propose_test_points` 只能使用该需求库及迭代内的文档，并在项目的可恢复生成状态中保存该选择。
- 保持 local-first：需求库可直接用普通文件和 Git 管理，不引入远程服务、账户状态或网络同步依赖。
- 提供显式旧项目需求导入流程；运行时不会回退读取项目内 `requirements/`，避免两个事实来源。

## Capabilities

### New Capabilities

- `global-requirement-library`: 定义用户级需求库的默认位置、覆盖规则、稳定身份、文件布局以及项目内旧需求的显式导入行为。

### Modified Capabilities

- `tui-requirements-testpoints-authoring`: 将需求 authoring 从项目目录切换到当前用户级需求库，扩展 `(store_id, document_id)` traceability，并支持迭代标注、过滤、分组和用户级视图偏好。
- `tui-requirements-generation`: 将需求来源切换到当前用户级需求库，支持选择单个迭代作为唯一来源，并在项目会话中固定 store 和 revision。

## Impact

- **Core model and validation**: 增加 `RequirementStoreId`、全局需求库索引身份和 `RequirementDocumentMeta.iteration`；requirement links 使用组合身份。
- **Persistence**: `<app_data>/requirements/_teshi.json` 和同根 Markdown 保存共享需求；用户级 `settings.json` 保存视图偏好；项目 `.teshi/generation-state.json` 保存当前生成的 store/iteration 范围。
- **TUI**: Requirements tab 从用户级需求库加载，增加路径配置诊断、迭代标注、过滤、分组和对应键位/状态提示；AI tab 增加生成范围选择与可见反馈。
- **Agent pipeline**: `submit_requirements`、source refs、requirement links 和 prompt/context 组装受选定 store、iteration 和 revision 约束。
- **Migration**: 增加显式 import，将旧项目需求复制到用户级库，处理 ID/path 冲突并原子重写该项目测试点引用；不保留项目目录 fallback。
- **Compatibility**: 测试点和 Gherkin 继续归属具体项目，测试点审批门禁不变；旧 generation state 需要安全恢复或重新选择全局来源。
- **Out of scope**: 不增加 GPUI/web/daemon authoring UI，不实现远程迭代管理、多人同步或项目管理平台集成。
