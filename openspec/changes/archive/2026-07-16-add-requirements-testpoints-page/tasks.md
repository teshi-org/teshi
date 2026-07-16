## 1. Backend: API 端点与 LLM 集成

- [x] 1.1 定义请求/响应 Rust 结构体（`Segment`, `GenerateRequest`, `GenerateResponse`）
- [x] 1.2 编写 LLM system prompt 模板（需求拆分 + FreeMind XML + 高保真 HTML，使用 tool calling 约束输出格式）
- [x] 1.3 实现 `POST /api/v1/requirements/generate` 端点，调用 LLM 并解析结构化响应
- [x] 1.4 添加响应校验：XML well-formedness 检查、segments 区间连续性校验
- [x] 1.5 实现结果持久化：保存 `requirements.mm` 和 `mock.html` 到 `.teshi/testpoints/<slug>/`
- [x] 1.6 添加错误处理和降级逻辑（LLM 超时、格式异常、空响应）

## 2. Frontend: 核心组件

- [x] 2.1 创建 `ModeToggle.tsx` — 左上角固定切换按钮，通过 CSS display 控制两个视图可见性
- [x] 2.2 创建 `RequirementsInput.tsx` — 文本输入区 + "生成"按钮 + loading 状态 + 错误提示
- [x] 2.3 创建 `RequirementsText.tsx` — 将 segments 渲染为可点击的词语级文本，支持高亮态
- [x] 2.4 创建 `lib/mindmap-parser.ts` — FreeMind XML → `MindMapNode[]` 解析器
- [x] 2.5 创建 `MindMapViewer.tsx` — 树图渲染组件，支持展开/折叠、节点选中、高亮
- [x] 2.6 创建 `MockHtmlViewer.tsx` — sandboxed iframe 渲染 mock HTML
- [x] 2.7 创建 `RequirementsPage.tsx` — 三栏容器布局，组合上述组件

## 3. Frontend: 集成与交互

- [x] 3.1 修改 `App.tsx`，引入 `viewMode` 状态，使用 CSS display 切换 `RequirementsPage` 和 `WorkspacePage`
- [x] 3.2 将现有内容包装为 `WorkspacePage.tsx`，确保切换后状态不丢失
- [x] 3.3 实现 API 调用：`POST /api/v1/requirements/generate`，解析返回的 JSON
- [x] 3.4 实现双向映射交互：点击测试点节点 → 高亮对应词语；点击词语 → 高亮对应测试点节点
- [x] 3.5 页面加载时检查 `.teshi/testpoints/` 下是否有已保存的结果，有则自动加载最新一份
- [x] 3.6 设置 `RequirementsPage` 为默认视图（应用启动时 `viewMode` 初始值为 `'requirements'`）

## 4. 收尾与验证

- [x] 4.1 端到端手动测试：粘贴需求 → 生成 → 验证三栏渲染正确
- [x] 4.2 验证切换按钮在两个视图间瞬间切换，workspace 状态保留
- [x] 4.3 验证双向高亮交互正确（节点 ↔ 词语）
- [x] 4.4 验证 mock HTML 在 sandboxed iframe 中安全渲染
- [x] 4.5 验证 `.teshi/testpoints/` 目录下文件正确保存
- [x] 4.6 验证错误场景：空输入提示、LLM 失败降级、畸形 XML 降级
