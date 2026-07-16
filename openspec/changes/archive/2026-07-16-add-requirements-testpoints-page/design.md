## Context

teshi 目前有三套界面（TUI、Web、Desktop），共享同一个 daemon 后端。Desktop/Web 前端是 React SPA，使用 React Context 管理状态，布局采用三面板可调整大小的工作区 + 底部 Dock。当前没有需求分析或测试点生成功能——用户只能手工编写 `.feature` 文件。

本次设计在一个月前的 BDD Feature Convention 和 AI Agent 基础设施之上，新增「需求→测试点」页面作为默认视图。

## Goals / Non-Goals

**Goals:**

- 在 desktop/web 前端新增「需求→测试点」页面，作为默认启动视图
- 后端新增 API 端点，调用 LLM 完成需求拆分、测试点生成、Mock HTML 生成
- 实现词语级双向映射交互（测试点 ↔ 需求原文）
- 左上角固定切换按钮，瞬间在需求页与原有工作区之间切换
- 生成结果持久化到 `.teshi/testpoints/` 目录

**Non-Goals:**

- TUI 端 (`teshi .`) 不包含此功能（本需求仅限 desktop/web）
- 不修改 Gherkin 面板或 Feature 文件
- 不生成 Gherkin Scenario（测试点仅作为分析产物）
- 不支持多人协作编辑

## Decisions

### 1. 前端组件拆解

```
App.tsx
├── ModeToggle.tsx          ← 左上角固定切换按钮（新增）
├── RequirementsPage.tsx    ← 需求→测试点页面容器（新增）
│   ├── RequirementsInput.tsx    ← 左侧：输入区域 + 需求原文渲染（新增）
│   ├── MindMapViewer.tsx        ← 中间：FreeMind 树图渲染（新增）
│   └── MockHtmlViewer.tsx       ← 右侧：sandboxed iframe（新增）
└── WorkspacePage.tsx       ← 原有工作区（重构：包装现有内容）
    ├── ResizableWorkspace.tsx   ← 现有三面板布局
    └── BottomDock.tsx           ← 现有底部 Dock
```

**理由**：将两种视图模式拆分为独立组件，`App.tsx` 根据 `viewMode` 状态条件渲染。`WorkspacePage.tsx` 仅负责包裹现有组件，不出现在需求分析页面中。

### 2. 视图切换策略

使用 React state (`viewMode: 'requirements' | 'workspace'`) 控制条件渲染，而非路由。两个视图完全互斥，不共享 DOM。

```
┌──────────────┐  viewMode='requirements'  ┌──────────────┐
│Requirements  │ ◀───────────────────────▶ │  Workspace   │
│    Page      │     点击 toggle 按钮       │    Page      │
└──────────────┘                           └──────────────┘
```

**理由**：不是真正的路由——两个视图不存在 URL 变化。条件渲染保证切换瞬时完成（无需卸载/重挂载，或用 `display:none` 保持 DOM）。如果用 React Router，会引入不必要的路由复杂度。另外需要考虑：切换回 workspace 时应保留原有的列宽、dock 展开状态、文件树状态等，所以使用 `display:none` 而非条件挂载可能更合适——实际实现时用 CSS visibility/display 控制，确保 WorkspacePage 的 DOM 不被销毁。

**修正**：两个页面始终挂载，通过 CSS `display` 切换可见性。避免条件渲染导致状态丢失。

### 3. API 设计

```
POST /api/v1/requirements/generate
Content-Type: application/json

Request:
{
  "requirements_text": "用户可以通过邮箱密码登录系统。密码错误时显示提示。"
}

Response (200):
{
  "slug": "20260706-191058",
  "segments": [
    { "id": "w1", "text": "用户",     "pos": [0, 2] },
    { "id": "w2", "text": "可以通过",  "pos": [2, 6] },
    { "id": "w3", "text": "邮箱密码",  "pos": [6, 10] },
    { "id": "w4", "text": "登录系统",  "pos": [10, 14] }
  ],
  "mindmap_xml": "<map version=\"1.0.1\">\n  <node TEXT=\"用户管理\">...",
  "mock_html": "<!DOCTYPE html>\n<html>..."
}

Response (4xx/5xx):
{
  "error": "<message>"
}
```

**理由**：单一端点完成全部生成，避免多轮 API 调用的复杂性。LLM 调用放在 daemon 后端而非前端，保护 API key 不暴露到浏览器。slug 由后端根据时间戳生成，前端可覆盖（暂不支持）。

### 4. LLM Prompt 策略

使用 OpenAI 兼容的 function calling，定义严格的 JSON Schema 约束输出格式：

```
Tool: generate_testpoints
Parameters:
  - segments: array of { id, text, pos }
  - mindmap_xml: string (FreeMind XML)
  - mock_html: string (complete HTML document)

System prompt 关键约束：
1. 分词粒度：以语义词语为单位（不是逐字）
2. FreeMind 规范：Link 属性用逗号分隔多个 segment ID
3. Mock HTML：高保真，包含 CSS 样式和表单元素
4. 只生成与需求描述直接相关的测试点，不编造
```

**理由**：使用 tool calling 而非普通聊天回复，让 LLM 返回结构化 JSON，避免正则解析的脆弱性。但 tool calling 也会带来问题：如果 LLM 不支持 tool calling，需要降级方案（纯文本 + 代码块解析）。

### 5. FreeMind 渲染方案

后端返回 FreeMind XML 字符串，前端需要解析并渲染为交互式树图。

**方案 A**：前端纯 JS 解析 XML + 自绘 SVG/Canvas
**方案 B**：使用现有 React 树组件（如 react-arborist），先解析 XML 为树数据

**选择方案 B**：XML → 递归解析为 tree nodes → 传入树组件渲染。

```typescript
interface MindMapNode {
  id: string;
  text: string;
  link?: string[];   // parsed from LINK attribute
  children: MindMapNode[];
}
```

解析逻辑写入 `desktop/src/lib/mindmap-parser.ts`。渲染使用已有的树形 UI 模式（若项目中有），或引入轻量树组件。

### 6. 数据流

```
  RequirementsInput                    MindMapViewer
       │                                    │
       │  1. 用户输入需求文本                │
       │  2. 点击"生成"                    │
       ▼                                    │
  POST /api/v1/requirements/generate ────────┤
       │                                    │
       │  3. Daemon 转发给 LLM              │
       │  4. LLM 返回结构化 JSON            │
       │  5. Daemon 保存至 .teshi/          │
       │  6. 返回 JSON 给前端               │
       ▼                                    ▼
  segments[]  ◀──── 双向映射 ────▶  mindmap_xml → tree nodes
       │                                    │
       ▼                                    ▼
  渲染词语级文本            ◀────▶    渲染树图 + 交互
       │                                    │
       └────────── 选择联动 ────────────────┘
                           │
                           ▼
                    MockHtmlViewer
                    (iframe sandbox)
```

### 7. 存储结构

```
项目根目录/
└── .teshi/
    └── testpoints/
        ├── 20260706-191058/
        │   ├── requirements.mm      ← FreeMind XML
        │   └── mock.html            ← 完整 HTML
        ├── 20260706-210530/
        │   ├── requirements.mm
        │   └── mock.html
        └── _index.json              ← 所有生成记录索引（可选）
```

**理由**：按 slug 分目录，方便未来支持多次生成和历史回溯。

## Risks / Trade-offs

| Risk | Mitigation |
|------|-----------|
| LLM 返回的 FreeMind XML 格式不规范（缺少 LINK 属性、XML 畸形） | 后端做 XML well-formedness 校验，前端做降级渲染（缺失 LINK 则不展示高亮） |
| LLM 返回的 segments 覆盖不全或有重叠 | 后端校验：所有 seg.pos 区间必须连续覆盖 [0, len(text)] 且不重叠 |
| 高保真 Mock HTML 可能包含恶意脚本 | iframe 使用 `sandbox="allow-scripts"` 禁用导航、弹窗、表单提交等危险行为 |
| 大段需求文本导致 LLM token 超限 | 前端限制输入文本长度（如 5000 字符），后端做 token 估算后截断 |
| 切换视图时 workspace 状态丢失 | 使用 CSS display 切换而非条件挂载，保持 DOM 状态 |
| `.mm` 解析性能（大型导图） | FreeMind XML 体积通常很小（<100KB），递归解析 O(n)，无性能风险 |
