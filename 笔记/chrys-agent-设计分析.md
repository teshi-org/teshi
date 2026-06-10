# Chrys Agent 设计分析

> 记录时间：2026-06-11
> 分析对象：chrys v0.9.8（源码位于 `D:\Dev\Python\chrys\src\chrys/`）

---

## 1. 核心结论：Chrys 本身不是一个 skill

- **Skill ≠ Agent。** Skill 是 agent **可以使用的**领域知识包（`SKILL.md` 目录），agent 自身通过 **Agent Profile YAML** 定义。
- Chrys 的自我定义在 `profiles/agents/builtins/Code.yaml` 的 `instructions` 字段中，是一个普通 YAML 字符串，不是 skill。
- Skill 系统（`tools/skills/`）是渐进式知识暴露机制：
  1. 目录列出 skill 名称+描述（在 `<system-reminder>` 中）
  2. Agent 调用 `load_skill()` → 获取完整指令
  3. Agent 调用 `read_skill_resource()` / `run_skill_script()` → 使用 skill
- Skill 来源（三层自动扫描 + profile 配置路径）：
  - `~/.chrys/skills/`（Always-on 平台 skill）
  - `~/.agents/skills/`（共享用户 skill，opt-in）
  - `<cwd>/.agents/skills/`（工作区 skill，opt-in）
  - `skills.paths`（Profile YAML 中配置）

---

## 2. 为什么 Chrys 效果这么好

### 2.1 System Prompt 是精心打造的（核心）

文件：`Code.yaml` 的 `instructions` 字段。

每条指令针对特定的 LLM 失败模式：

| 指令 | 解决什么问题 |
|------|-------------|
| "Think about the bigger picture" — 先理解全局问题，再看具体任务 | 防止 LLM 只见树木不见森林，做出局部最优但全局错误的决策 |
| "Do exactly what was asked" — 只做被要求的事 | **防止过度交付**：修复 bug 时不重构周围代码，加功能时不加额外配置 |
| "Understand first, then act" — 先读代码，再改代码 | 减少"猜-失败-重试"循环，避免 mid-way 发现方案错误 |
| "Read before writing" — 理解现有模式再改动 | 保证风格一致，避免引入不匹配的约定 |
| "Prefer simplicity" — 最简单的方案优先 | 防止过早抽象，三行重复代码好过一个通用抽象 |
| "Review before finishing" — 完成后自审查 | 在用户发现之前抓住自己的错误 |
| "Due diligence" — 改一个函数检查所有调用点 | 阻止破坏性变更传播 |
| "Be direct. Lead with the action." — 直接说答案 | **消除废话**，不重述用户的话、不叙述搜索过程 |
| "Only add error handling at system boundaries" | 防止在内部代码加无意义的防御检查 |

### 2.2 分层上下文管理

**三层架构：**

1. **Memory 系统**（`schema.py:244-260`）
   - 自动加载 `AGENTS.md` 到 system prompt
   - 支持文件列表 + 目录递归
   - 结合 token 预算做裁切

2. **SystemReminderMiddleware**（`system_reminder.py:107`）
   - 每次 LLM 调用前附加 `<system-reminder>` 到最后一条用户消息
   - 包含：运行时环境（cwd/OS/shell/时间）、token 用量、profile 切换、skill 目录
   - 操作的是 `ChatContext.messages` 的**浅拷贝**，不污染 session 状态
   - 用户自己写的 `<system-reminder>` 会被转义为 `&lt;system-reminder&gt;`

3. **三阶段上下文压缩**（`compaction.py` + `Code.yaml:190-231`）
   - **触发阈值**：85%（`reserved_context_pct: 0.15`）
   - **Phase 1**：汇总最旧已完成轮次的工具调用结果（保留调用参数）
   - **Phase 2**：完全删除最旧已完成轮次的工具调用组
   - **Phase 3**：删除**当前轮次**全部工具调用，替换为 LLM 生成的 `[LAST_WORDS]` 进度笔记
   - LAST_WORDS 模板极为详细：用户请求、已读文件、已做决策、剩余任务、已发现的陷阱、已用的 skill

### 2.3 Sub-agent 架构（并发 + 专业化）

文件：`Code.yaml:160-188` + `tools/sub_agent.py`

| Sub-agent | 角色 | 核心约束 |
|-----------|------|----------|
| `explore_agent` | 只读代码探索 | 无写权限，shell 只读 |
| `plan_agent` | 只读架构规划 | 同上 + 返回结构化计划 |
| `general_agent` | 全读写子任务执行 | 同主 agent 工具，但不能问用户问题 |

**关键设计：**
- 每个 sub-agent 在自己的 LLM 会话中运行，有**自己的上下文窗口**
- `sub_agent_only: true` 标记防止用户直接调用
- `max_total_concurrency: 2` 并发控制
- 每个 sub-agent 有独立的 SystemReminderMiddleware 和压缩策略

### 2.4 审批策略（安全 + 速度平衡）

文件：`Code.yaml:237-241` + `agent/approval.py`

```yaml
approval:
  default: auto
  overrides:
    shell: require
    filesystem.write: require
```

- 读操作自动执行
- 写操作和 shell 需要人工确认（可以看 diff）
- 三种模式：`auto` / `require` / `skip`
- `user_can_override: false` — 防止 agent 绕过安全策略
- 可选 ApprovalJudge LLM（`CHRYS_MODEL_PROFILE_APPROVAL_JUDGE`）做自动审批

### 2.5 声明式配置（全部可定制）

**三层配置：**

| 层次 | 位置 | 覆盖什么 |
|------|------|----------|
| 内置默认 | `profiles/agents/builtins/*.yaml` | 5 个内置 agent profile（Code/Plan/Explore/General/QA） |
| 用户级 | `~/.chrys/agents/*.yaml` | 完全覆盖 system prompt、工具集、skill 路径、sub-agent、审批、压缩、memory |
| 项目级 | `.teshi/agents/*/` | 按 id 覆盖用户级 agent |

**Model 配置**：`~/.chrys/models/*.yaml`（每个文件定义 provider、model_id、base_url、api_key、chat_options）

**运行时覆盖**：`~/.chrys/.env`（`CHRYS_*` 环境变量）

---

## 3. 架构全景

```
chrys.exe (C:\Users\lilin\.local\bin\chrys.exe)
│
├─ cli/app.py :: main()          # 入口调度器
├─ runtime.py :: bootstrap()     # dotenv / patches / telemetry
│
├─ core/agent_builder.py :: build_agent()
│   ├─ AgentProfile (从 YAML 加载)
│   │   ├─ instructions → System Prompt
│   │   ├─ tools → ToolRegistry (builtins/MCP)
│   │   ├─ skills → ChrysSkillsProvider
│   │   ├─ sub_agents → SubAgentTools
│   │   ├─ approval → ApprovalPolicy
│   │   ├─ compaction → ContextStrategy
│   │   ├─ memory → MemoryLoader
│   │   └─ model → ModelProfile
│   └─ SystemReminderMiddleware (ChatMiddleware)
│       └─ 每轮追加 <system-reminder> 标签
│
├─ Agent Framework (Microsoft agent-framework-core==1.7.0)
│   Agent + Executor + ChatMiddleware chain:
│   ├─ SystemReminderMiddleware
│   ├─ ApprovalMiddleware
│   ├─ InjectionMiddleware
│   ├─ AskUserMiddleware
│   └─ ResponseValidationMiddleware
│
└─ 配置目录: %APPDATA%/chrys/
    ├─ agents/*.yaml     ← User agent profiles
    ├─ models/*.yaml     ← API keys, endpoints
    ├─ skills/           ← Platform skills
    ├─ sessions/         ← 持久化会话
    └─ hooks/            ← 生命周期钩子
```

内置 agent 的依赖关系：

```
Code (主 agent, 默认)
├─ Explore (sub-agent)    — 只读探索
├─ Plan   (sub-agent)     — 只读规划
└─ General (sub-agent)    — 全读写子任务

QA (独立 agent)           — 只读问答
└─ Explore (sub-agent)
```

---

## 4. 值得借鉴的设计模式

1. **System prompt 的指令不是"建议"而是"约束"** — 每条都解决一个已知的 LLM 失败模式
2. **上下文分层**：不把动态信息塞进 system prompt，而是用 middleware 在每次调用前注入
3. **Sub-agent 卸载**：复杂任务分割到独立 LLM 会话，不争抢主上下文
4. **压缩模板**：LAST_WORDS 模板有 20+ 个具体项目，不是模糊的"总结一下"
5. **配置文件即代码**：YAML 定义 agent 全部行为，不需要改源码
6. **渐进式 skill 暴露**：只展示 skill 名和描述，详情按需加载 — 省 token
