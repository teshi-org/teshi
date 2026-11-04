# Teshi 需求整理清单

整理日期：2026-11-02。来源：已安装 Teshi CLI 的 requirements list --all-projects --json。保留原始 ID、正文、优先级、项目关联与交付状态。本清单仅做分类和后续工作建议，未重新验证实现或运行验收。

## 需要先处理的问题

- 项目归属分散：doc-38～43、doc-51～53 关联其他项目 ID，doc-44 未关联；应核实对应逻辑项目后再调整，避免跨项目误改。
- doc-1 是 WinApp 审计索引，doc-16～22 是拆分需求；排期时不要把索引重复计为开发任务。
- doc-3 与 doc-5 的标题修改诉求重叠，建议保留 doc-3 为主需求、doc-5 为来源反馈；暂不删除。
- doc-4/11/52 都涉及滚动条，但分别关注可用性、外观和滑块尺寸，应关联验收而非直接合并。
- doc-13/14/53 分别涉及通用复制、聊天文本选择、代码块复制，保持独立验收。
- doc-30 与 doc-46 分别针对 Feature 和需求库外部变更同步，不合并。
- doc-47/48/49 形成测试点提交、审核策略、批准状态同步链路，建议优先闭环 doc-49，再协同验收整条流程。
- doc-28 为待讨论设计；doc-40 为宽泛方向，需要先收敛目标与可观察验收条件。
- implemented 与 verified 分开维护；本次未运行测试或回放，不升级任何状态。

## 分类目录

### 安装与更新

| ID | 需求 | 优先级 | 原记录状态 | 归属 |
|---|---|---|---|---|
| doc-51 | teshi update 失败后必须整包重下，更新过程不可靠 | P1 | planned | 其他项目 ID，待核实 |

### 测试点与生成流程

| ID | 需求 | 优先级 | 原记录状态 | 归属 |
|---|---|---|---|---|
| doc-40 | 如何快速自我迭代 | 未设置 | planned | 其他项目 ID，待核实 |
| doc-47 | Agent 测试点提交工具可用性与 CLI 补充入口 | 未设置 | planned | dev 项目 |
| doc-48 | Feature 生成流程重构：可选人工审核与用户授权优先 | 未设置 | planned | dev 项目 |
| doc-49 | Bug：测试点已批准但 Agent 仍收到等待审核状态 | 未设置 | planned | dev 项目 |

### 通用交互与布局

| ID | 需求 | 优先级 | 原记录状态 | 归属 |
|---|---|---|---|---|
| doc-11 | 横向滚动条实现很丑 | 未设置 | implemented | dev 项目 |
| doc-12 | 操作方式不统一 | 未设置 | implemented | dev 项目 |
| doc-13 | 让copy命令生效 | 未设置 | implemented | dev 项目 |
| doc-31 | 小屏幕下 teshi 显示差：内容展示不全、冗余、排版混乱 | 未设置 | planned | dev 项目 |
| doc-34 | 底部按钮应可点击并需好好规划 | 未设置 | planned | dev 项目 |
| doc-35 | 可点击元素需具备 hover 反馈效果 | 未设置 | planned | dev 项目 |
| doc-36 | Requirement / Testpoints / Explore 等 tab 默认三栏过于占位置，应改为两栏或合并后两栏 | 未设置 | implemented | dev 项目 |
| doc-44 | teshi不要使用ctrl+c退出程序，应该使用ctrl+q退出程序 | 未设置 | 未记录 | 未关联 |
| doc-52 | 竖向滚动条滑块太短，需要拉长一点 | P2 | planned | 其他项目 ID，待核实 |

### 项目与加载

| ID | 需求 | 优先级 | 原记录状态 | 归属 |
|---|---|---|---|---|
| doc-2 | 项目切换与项目上下文隔离 | 未设置 | implemented | dev 项目 |
| doc-38 | requirement面板很难区分是哪个项目的 | 未设置 | planned | 其他项目 ID，待核实 |
| doc-45 | CLI 项目目录加载规则统一与递归扫描性能 | 未设置 | planned | dev 项目 |

### 需求库与编辑

| ID | 需求 | 优先级 | 原记录状态 | 归属 |
|---|---|---|---|---|
| doc-3 | Requirement 标题与左侧文档树同步 | 未设置 | implemented | dev 项目 |
| doc-4 | Requirements 面板需要支持滚动条 | 未设置 | implemented | dev 项目 |
| doc-5 | TUI修改requirements面板中的document的名字 | 未设置 | implemented | dev 项目 |
| doc-7 | Requirements的中间面板对vim的支持不够齐全 | 未设置 | implemented | dev 项目 |
| doc-8 | 需求文档不支持删除 | 未设置 | implemented | dev 项目 |
| doc-9 | 需求文档界面，展示优先级 | 未设置 | implemented | dev 项目 |
| doc-10 | Bug: 新增文档后未切换左侧树 | 未设置 | implemented | dev 项目 |
| doc-23 | Requirements CLI 支持直接新增需求条目 | P1 | implemented | dev 项目 |
| doc-24 | Requirement 文件默认使用时间戳命名 | 未设置 | implemented | dev 项目 |
| doc-39 | 创建新的requirement应该自动聚焦到右侧编辑窗口，且是Insert状态 | 未设置 | planned | 其他项目 ID，待核实 |
| doc-42 | 切换priority不支持在编辑面板中切换 | 未设置 | planned | 其他项目 ID，待核实 |
| doc-43 | requirement界面为什么要使用ctrl+n新增requirement | 未设置 | planned | 其他项目 ID，待核实 |
| doc-46 | TUI 需求库外部变更同步与可见性诊断 | 未设置 | planned | dev 项目 |

### AI 与 ACP

| ID | 需求 | 优先级 | 原记录状态 | 归属 |
|---|---|---|---|---|
| doc-6 | Agent、ACP运行过程没有展示 | 未设置 | implemented | dev 项目 |
| doc-14 | AI聊天tab中的内容现在无法文本选中复制 | 未设置 | implemented | dev 项目 |
| doc-25 | AI Chat 面板标题旁的 · Default 字样令人困惑 | 未设置 | implemented | dev 项目 |
| doc-26 | ACP 运行时底部等待动画应与 agent/mode/model 芯片同一行 | 未设置 | implemented | dev 项目 |
| doc-27 | AI Chat 底部展示运行时间与上下文窗口占用 | 未设置 | implemented | dev 项目 |
| doc-28 | 待探讨：多 agent 聊天时终端（PowerShell）标题应展示什么 | 未设置 | planned | dev 项目 |
| doc-29 | AI Chat 输入框聚焦时底部浮现的快捷键提示应移除 | 未设置 | implemented | dev 项目 |
| doc-41 | 使用acp模式时，希望可以通过斜杠命令切换approval | P1 | planned | 其他项目 ID，待核实 |
| doc-50 | 当前 ACP 会话 sessionId 的可见性与查询 | 未设置 | planned | dev 项目 |
| doc-53 | AI 聊天 tab 的代码块需要一键复制 | P1 | planned | 其他项目 ID，待核实 |
| doc-54 | 打开 AI Chat 不应默认启动 agent 会话 | P1 | planned | dev 项目 |

### Explore 与 Feature

| ID | 需求 | 优先级 | 原记录状态 | 归属 |
|---|---|---|---|---|
| doc-30 | 外部 feature 的增删改应在 TUI 界面实时反映 | 未设置 | implemented | dev 项目 |
| doc-32 | Explore 第三面板（Step 细节）的增删应以细粒度展示并支持 j/k 移动 | 未设置 | implemented | dev 项目 |
| doc-33 | Explore 第三面板（Step 细节）滚动与键盘交互不直观（横向内容展示不全） | 未设置 | implemented | dev 项目 |
| doc-37 | Explore 中实时变更但用户尚未查看的步骤需要显式标记 | 未设置 | planned | dev 项目 |
| doc-55 | Explore feature 搜索输入防抖 | P1 | implemented | dev 项目 |

### Runner 与 WinApp

| ID | 需求 | 优先级 | 原记录状态 | 归属 |
|---|---|---|---|---|
| doc-1 | WinApp 自动化能力审计索引（Enhook 三 Agent 会话及源码核查） | P0 | planned | dev 项目 |
| doc-15 | TUI runner 支持 Scenario Outline | 未设置 | implemented | dev 项目 |
| doc-16 | 端到端硬超时、取消与执行隔离 | P0 | implemented | dev 项目 |
| doc-17 | 协议级健康检查与安全恢复 | P0 | implemented | dev 项目 |
| doc-18 | WinApp replay 完整覆盖与严格结果语义 | P0 | implemented | dev 项目 |
| doc-19 | 执行结果独立于截图，保留失败证据 | P0 | implemented | dev 项目 |
| doc-20 | 实例级进程与窗口生命周期验收 | P1 | planned | dev 项目 |
| doc-21 | 可扩展业务视觉与外部判定 | P1 | planned | dev 项目 |
| doc-22 | 能力发现与 Agent 文档同步 | P1 | planned | dev 项目 |

## 范围外条目

teshi-winapp-tray-steps 关联另一项目，标题为“winapp 原生支持托盘点击与窗口状态步骤”；暂列为跨项目候选，不自动并入 Teshi。

## 建议推进顺序

1. 核实项目归属，补齐缺失的 Teshi 需求可见性。
2. 处理生成链路阻塞（doc-49/47/48）、更新可靠性（doc-51）、Agent 延迟启动（doc-54）；这是建议顺序，不修改原优先级。
3. 将已有 implemented 条目安排验收，记录实际通过、失败和阻塞证据。
4. 再安排布局、交互、可见性与待讨论设计。
