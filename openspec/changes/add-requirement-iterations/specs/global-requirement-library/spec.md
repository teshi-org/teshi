## ADDED Requirements

### Requirement: Teshi 使用用户级全局需求库

系统 SHALL 从一个独立于当前项目的用户级需求库读取和写入需求文档。默认需求库根目录 SHALL 为 `<app_data>/requirements`；路径解析优先级 SHALL 依次为 CLI `--requirements-root`、非空 `TESHI_REQUIREMENTS_DIR`、`app_data_dir()/requirements`，其中 `app_data_dir()` 继续遵循 `TESHI_APP_DATA_DIR` 覆盖。系统 SHALL NOT 把 `<project>/requirements/` 作为运行时需求来源或 fallback。

#### Scenario: 使用默认需求库

- **WHEN** 用户未提供 CLI 或环境变量覆盖
- **THEN** 系统 SHALL 使用 `app_data_dir()/requirements`
- **AND** Windows 默认 SHALL 解析为 `%APPDATA%\teshi\requirements`

#### Scenario: 使用独立需求目录覆盖

- **WHEN** 用户设置非空 `TESHI_REQUIREMENTS_DIR`
- **THEN** 系统 SHALL 使用该目录而不追加 `requirements`
- **AND** SHALL 在所有项目中使用同一解析结果

#### Scenario: CLI 覆盖环境变量

- **WHEN** 同时提供 `--requirements-root` 和 `TESHI_REQUIREMENTS_DIR`
- **THEN** 系统 SHALL 使用 CLI 指定目录

#### Scenario: 项目仍包含旧 requirements 目录

- **WHEN** 当前项目存在 `<project>/requirements/`
- **THEN** 系统 SHALL NOT 自动加载或合并其中的文档
- **AND** SHALL 提示用户运行显式导入流程

### Requirement: 全局需求库拥有稳定身份和普通文件布局

需求库 SHALL 在根目录 `_teshi.json` 中保存稳定 `store_id`、schema version 和文档索引，并 SHALL 将 Markdown 文档保存为相对于该根目录的普通文件。需求的完整身份 SHALL 是 `(store_id, document_id)`；移动整个需求库或通过不同本机路径打开它 SHALL NOT 改变该身份。

#### Scenario: 首次初始化默认需求库

- **WHEN** 用户首次创建需求且需求库尚未初始化
- **THEN** 系统 SHALL 创建需求库根目录和包含唯一稳定 `store_id` 的 `_teshi.json`
- **AND** SHALL 原子写入索引

#### Scenario: 需求库移动到其他目录

- **WHEN** 用户移动完整需求库并通过路径覆盖重新打开
- **THEN** 系统 SHALL 从 `_teshi.json` 恢复相同 `store_id` 和 document IDs
- **AND** 项目测试点中的有效组合引用 SHALL 继续指向相同需求

#### Scenario: 需求库索引缺少 store ID

- **WHEN** 非空全局需求库的 `_teshi.json` 缺少有效 `store_id`
- **THEN** 系统 SHALL 拒绝把它当作已初始化的全局需求库
- **AND** SHALL 提供导入或修复诊断，而不是临时按路径生成身份

### Requirement: 旧项目需求通过显式事务导入

系统 SHALL 提供显式 import，将 `<project>/requirements/` 的索引和 Markdown 复制到当前全局需求库，并 SHALL 保持或确定性重映射文档 ID 与路径。若该项目存在 test points，系统 SHALL 在同一导入事务中把 requirement links 更新为目标 `(store_id, document_id)`。导入成功 SHALL NOT 自动删除源文件。

#### Scenario: 无冲突导入旧项目

- **WHEN** 用户导入一个文档 ID 和目标路径均不冲突的旧项目需求目录
- **THEN** 系统 SHALL 保留原 document IDs 和相对路径
- **AND** SHALL 为该项目的 test-point links 补充目标 `store_id`

#### Scenario: 导入遇到 ID 或路径冲突

- **WHEN** 目标需求库已包含相同 document ID 或目标路径但内容身份不同
- **THEN** 系统 SHALL 在写入前展示冲突和确定性重映射计划
- **AND** 未经用户确认 SHALL NOT 修改需求库或项目测试点

#### Scenario: 导入期间写入失败

- **WHEN** 复制文档、写入目标索引或重写 test points 任一步骤失败
- **THEN** 系统 SHALL 保留此前完整可读的目标需求库和项目测试点
- **AND** SHALL NOT 删除旧项目需求文件

#### Scenario: 导入成功后旧目录仍存在

- **WHEN** 已导入项目再次启动且旧 `<project>/requirements/` 尚未删除
- **THEN** 系统 SHALL 只加载全局需求库
- **AND** SHALL NOT 形成两个事实来源
