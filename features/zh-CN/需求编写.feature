# language: zh-CN

@cli
功能: 需求编写
  作为使用需求库的测试人员或代理
  我想分配迭代并替换文档正文
  以便无需打开 TUI 即可更新需求库

  样例文档:
  - doc-12 auth/login.md 标题为 Login，属于 Sprint 12
  - doc-37 mobile/login.md 标题为 Login，未分配
  - doc-9 shop/checkout.md 标题为 Checkout，未分配

  背景:
    假如 已有一份包含登录和结账样例文档的隔离需求库

  场景: 批量分配把两份文档划入同一迭代
    当 测试人员将文档 "doc-12" 和 "doc-9" 分配到迭代 "Sprint 13"
    那么 命令成功
    并且 文档 "doc-12" 和 "doc-9" 属于迭代 "Sprint 13"

  场景: 批次中的未知文档不会改动已有分配
    当 测试人员将文档 "doc-12" 和 "missing" 分配到迭代 "Sprint 13"
    那么 命令失败
    并且 文档 "doc-12" 属于迭代 "Sprint 12"

  场景: 清除迭代后文档回到未分配
    假如 文档 "doc-12" 和 "doc-9" 属于迭代 "Sprint 13"
    当 测试人员清除文档 "doc-12" 和 "doc-9" 的迭代
    那么 命令成功
    并且 文档 "doc-12" 和 "doc-9" 未分配迭代

  场景: 文件替换会更新登录文档正文
    当 测试人员将文档 "doc-12" 的正文替换为:
      """
      # Login

      Updated authentication flow.
      """
    那么 命令成功
    并且 文档 "doc-12" 的正文为:
      """
      # Login

      Updated authentication flow.
      """

  场景: 未变化的替换不会改写修订
    当 测试人员重新提交文档 "doc-12" 的当前正文
    那么 命令成功
    并且 文档 "doc-12" 的修订未改变

  场景: 非交互编辑且未提供正文时被拒绝
    当 测试人员在未提供正文的情况下以 JSON 编辑文档 "doc-12"
    那么 命令失败
    并且 JSON 错误码为 "missing_edit_input"

  场景: 空白迭代名被拒绝
    当 测试人员以 JSON 将文档 "doc-12" 分配到迭代 "   "
    那么 命令失败
    并且 JSON 错误码为 "invalid_iteration_name"
