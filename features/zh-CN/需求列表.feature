# language: zh-CN

@cli
功能: 需求列表
  作为使用需求库的测试人员或代理
  我想按迭代或身份列出并查看文档
  以便无需打开 TUI 即可检查需求库

  样例文档:
  - doc-12 auth/login.md 标题为 Login，属于 Sprint 12
  - doc-37 mobile/login.md 标题为 Login，未分配
  - doc-9 shop/checkout.md 标题为 Checkout，未分配

  背景:
    假如 已有一份包含登录和结账样例文档的隔离需求库

  场景: 按命名迭代列出 JSON 时只包含已分配文档
    当 测试人员以 JSON 列出迭代 "Sprint 12" 中的需求
    那么 命令成功
    并且 JSON 列表仅包含文档 "doc-12"

  场景: 未分配 JSON 列表排除已分配文档
    当 测试人员以 JSON 列出未分配的需求
    那么 命令成功
    并且 JSON 列表仅包含文档 "doc-37" 和 "doc-9"

  场景: 空的命名迭代仍然成功
    当 测试人员以 JSON 列出迭代 "Missing" 中的需求
    那么 命令成功
    并且 JSON 列表为空

  场景: 迭代与未分配过滤器不能同时使用
    当 测试人员同时按迭代 "Sprint 12" 和未分配列出需求
    那么 命令失败
    并且 未列出任何需求文档

  场景: 唯一标题显示结账文档
    当 测试人员显示标题为 "Checkout" 的文档
    那么 命令成功
    并且 输出是结账文档的正文

  场景: 文档 id 显示登录文档
    当 测试人员显示文档 "doc-12"
    那么 命令成功
    并且 输出以 "# Login" 开头

  场景: 相对路径显示结账文档
    当 测试人员显示路径为 "shop/checkout.md" 的文档
    那么 命令成功
    并且 输出是结账文档的正文

  场景: JSON 显示包含文档元数据和正文
    当 测试人员以 JSON 显示文档 "doc-12"
    那么 命令成功
    并且 JSON 显示信封的 id 为 "doc-12"、标题为 "Login"、路径为 "auth/login.md"、迭代为 "Sprint 12"
    并且 JSON 显示正文以 "# Login" 开头
    并且 JSON 显示信封包含 store_id 和 revision

  场景: 重复标题在 JSON 下被拒绝
    当 测试人员以 JSON 显示标题为 "Login" 的文档
    那么 命令以退出码 2 失败
    并且 JSON 错误码为 "ambiguous_requirement_ref"
