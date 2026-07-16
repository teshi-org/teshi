## Why

当前的测试点罗列方式缺乏结构化和可追溯性：用户无法清楚地看到每个测试点覆盖了哪些需求，也无法从需求原文快速定位到对应的测试点。需要一个「需求→测试点」的可视化分析页面，利用 AI Agent 自动拆分需求并生成结构化的测试点思维导图和高保真界面 Mock。

## What Changes

- **新增页面**：在 teshi desktop/web 中新增「需求→测试点」页面，作为默认展示视图
- **需求输入**：用户粘贴自由文本需求，一键提交给 AI Agent 分析
- **AI 生成**：Agent 自动拆解需求为词语级段落，生成 FreeMind (.mm) 格式的测试点思维导图，同时生成高保真 Mock HTML 展示被测界面逻辑
- **双向映射**：点击测试点节点 → 高亮需求原文对应词语；点击需求原文词语 → 高亮对应测试点节点
- **视图切换**：左上角固定切换按钮，瞬间在「需求→测试点」页面与原有工作区（Feature 编辑+截图流+Terminal）之间切换
- **持久化**：生成结果保存到 `.teshi/testpoints/<slug>/` 目录（`requirements.mm` + `mock.html`）

## Capabilities

### New Capabilities

- `requirements-testpoints-page`: 需求分析与测试点生成页面，包含需求输入、AI 驱动的需求拆分与测试点生成、双向映射思维导图、高保真 Mock HTML 预览，以及与原工作区的无缝切换。

### Modified Capabilities

<!-- No existing capabilities are modified at spec level -->

## Impact

- **前端 (desktop/src/)**：新增 `RequirementsPage.tsx`、`RequirementsInput.tsx`、`MindMapViewer.tsx`、`MockHtmlViewer.tsx`；修改 `App.tsx` 添加页面路由/切换逻辑；新增切换按钮组件
- **后端 (crates/teshi-daemon/)**：新增 `POST /api/v1/requirements/generate` 端点，调用 LLM 并返回结构化 JSON（segments + mindmap_xml + mock_html）
- **Agent (src/agent/)**：新增专用的 system prompt 模板用于需求拆分 + FreeMind XML 生成 + 高保真 HTML 生成
- **文件系统**：新增 `.teshi/testpoints/` 目录约定，用于持久化生成结果
- **依赖**：前端需要引入 FreeMind XML 解析/渲染库（或将解析逻辑放到后端）
