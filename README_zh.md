# teshi

终端优先的 BDD 编辑器，支持 AI 辅助、思维导图导航和外部测试运行器集成。

## 快速开始

```bash
cargo run -- path/to/features/          # 打开 .feature 文件目录
cargo run -- path/to/file.feature       # 打开单个 feature 文件
cargo run -- run path/to/file.feature   # 运行 BDD 测试（基于 NDJSON 的运行器）
```

不带参数启动将打开一个空缓冲区。

## 标签页

| 按键 | 标签页 | 功能 |
|-----|-----|---------|
| `1` | Explore（浏览） | 三列浏览器：features → scenarios → steps。可导航、编辑、运行和 AI 建议。 |
| `2` | MindMap（思维导图） | 交互式树状视图，展示所有场景和步骤，支持高亮、过滤和跨文件步骤复用检测。 |
| `3` | AI | 基于函数调用 LLM 代理的聊天界面，可检查项目并将编辑排入审批队列。 |
| `4` | Help（帮助） | 应用内快捷键参考。 |

## Explore（浏览）标签页

以三列布局展示项目状态：

- **Features 列** — `.feature` 文件列表。`j`/`k` 或 `↑`/`↓` 移动；`e` 进入编辑器编辑所选文件；`a` 为所选场景打开 AI 建议提示。
- **Scenarios 列** — 所选 feature 中的场景列表。显示测试运行状态（pending / running / passed / failed / skipped）。`r` 运行所选场景。
- **Steps 列** — 所选场景的步骤列表，带测试用例状态。`Enter` 切换失败步骤的详情。

列间导航：`Tab` / `→` 向右移动，`BackTab` / `←` / `h` 向左移动。

## MindMap（思维导图）标签页

三阶段布局，以树状展示完整步骤层级：

- **树面板**（左）— 可折叠的 features → scenarios → steps 树。`Enter` 展开/折叠节点；适用第一阶段快捷键。
- **编辑预览面板**（右，第二阶段）— 所选节点源代码的只读预览。选择非根节点时可用。
- **步骤编辑面板**（右，第三阶段）— 可编辑所选步骤的文本内容。

可通过 AI 工具（`highlight_mindmap_nodes`、`apply_mindmap_filter`）实现高亮和过滤。按 `Tab` 循环切换 MindMap 位置选择。

## AI 标签页

基于 LLM 函数调用代理的聊天界面。代理可访问六个工具：

| 工具 | 描述 |
|------|-------------|
| `get_project_info` | 项目概览：feature 文件、场景/步骤计数、当前活动文件 |
| `get_feature_content` | 获取 `.feature` 文件的完整解析内容 |
| `highlight_mindmap_nodes` | 高亮匹配条件的 MindMap 节点 |
| `apply_mindmap_filter` | 按节点名称过滤 MindMap 树 |
| `insert_scenario` | 插入新场景（排入用户审批队列） |
| `update_step` | 更新步骤文本（排入用户审批队列） |

编辑类工具会将更改排入审批队列：`Y` 接受，`N`/`Esc` 拒绝，`D` 查看差异。`Esc` 在聊天输入框和消息列表之间切换。`Alt+↑`/`Alt+↓` 滚动聊天历史。

## 编辑器快捷键

### 导航（Explore / MindMap）

| 按键 | 操作 |
|-----|--------|
| `↑` / `↓` / `j` / `k` | 上一个/下一个可导航行或树节点 |
| `←` / `→` / `h` / `l` | 切换关键词/正文焦点；在列之间移动 |
| `Home` / `End` | 第一个/最后一个节点或行 |
| `PageUp` / `PageDown` | 滚动约 10 个节点或行 |

### 编辑（编辑器 / 步骤编辑模式）

| 按键 | 操作 |
|-----|--------|
| `e` | 进入所选文件的编辑器 |
| `Enter` | 打开步骤编辑或提交当前行编辑 |
| `Space` | 在关键词上：打开步骤关键词选择器；在正文上：开始编辑 |
| `Tab` | 插入新步骤行（拆分或下方插入） |
| `Backspace` / `Delete` | 删除字符或合并行 |
| `Esc` | 清除输入状态 / 关闭弹出层 |
| `d` `d` | 删除当前步骤或场景 |
| `y` `y` | 复制当前步骤 |
| `p` | 粘贴已复制的步骤 |

### 结构性编辑

| 按键 | 操作 |
|-----|--------|
| `Ctrl+/` | 撤销（完整缓冲区快照） |
| `Ctrl+Y` | 重做 |
| `s` | 保存当前文件 |
| `q` | 退出（缓冲区有未保存更改时需按两次） |

## 外部测试运行器

`teshi run` 子命令通过可配置的 NDJSON 运行器执行 BDD feature 文件。

```bash
teshi run tests/features/editor.feature
```

在 `teshi.toml` 中配置运行器命令：

```toml
[runner]
command = "cargo"
args = ["run", "--bin", "teshi-runner"]
```

测试结果以 NDJSON 格式流式返回，在 Explore 标签页中以内联方式显示，场景和步骤带有状态颜色。

## 自举

teshi 自身的功能矩阵以 BDD feature 文件形式描述在 `tests/features/` 目录下：

- `editor.feature` — BDD 导航、步骤编辑、关键词选择器、语法高亮
- `mindmap.feature` — 思维导图树视图、三阶段布局、步骤复用检测
- `project.feature` — 单/多文件加载、Gherkin 解析、编辑与树同步

更多演示文件位于 `tests/features_demo/` 目录下。

## 语法高亮

- Gherkin 头部（`Feature`、`Scenario`、`Scenario Outline`、`Examples`、`Background`）
- 步骤关键词（`Given`、`When`、`Then`、`And`、`But`）
- 标签（`@tag`）
- 注释（`# ...`）
- 字符串（`"..."`）
- 表格和文档字符串标记（`|`、`"""`）

## 环境变量

### LLM（AI 标签页）

| 变量 | 必填 | 默认值 | 描述 |
|---|---|---|---|
| `TESHI_LLM_API_KEY` | 是 | — | LLM 提供商的 API 密钥 |
| `TESHI_LLM_BASE_URL` | 否 | `https://api.openai.com/v1` | 兼容 OpenAI 的 API 基础 URL |
| `TESHI_LLM_MODEL` | 否 | `gpt-4o-mini` | 要使用的模型名称 |
| `TESHI_LLM_MAX_TOKENS` | 否 | `1024` | 每次完成的最大 token 数 |
| `TESHI_LLM_TEMPERATURE` | 否 | `0.7` | 采样温度 |

未设置 `TESHI_LLM_API_KEY` 时，AI 标签页会隐藏。

### Runner（测试运行器）

| 变量 | 必填 | 默认值 | 描述 |
|---|---|---|---|
| `TESHI_RUNNER_CMD` | 是（无 `teshi.toml` 时） | — | 测试运行器可执行文件 |
| `TESHI_RUNNER_ARGS` | 否 | — | 运行器的参数（空格分隔） |
| `TESHI_RUNNER_CWD` | 否 | 当前目录 | 运行器的工作目录 |

环境变量优先级高于 `teshi.toml` 中的值。

### 诊断

| 变量 | 用途 |
|---|---|
| `TESHI_DIAG_PATH` | 将诊断日志写入此文件路径 |
| `TESHI_NO_RAW` | 禁用原始终端模式 |
| `TESHI_NO_ALT` | 禁用备用屏幕 |

## 配置文件

工作目录中的 `teshi.toml` 配置文件支持：

```toml
[runner]
command = "cargo"
args = ["run", "--bin", "teshi-runner"]
cwd = "."          # 可选的工作目录

[llm]
# 暂不支持 toml 配置；请使用上述环境变量。
```
