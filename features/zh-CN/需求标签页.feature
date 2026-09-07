# language: zh-CN

@cli
功能: 需求标签页
  作为在无项目路径时打开 teshi 的测试人员
  我想让需求标签页加载用户级需求库
  以便阅读已经保存的文档

  样例文档:
  - doc-12 auth/login.md 标题为 Login，属于 Sprint 12
  - doc-37 mobile/login.md 标题为 Login，未分配
  - doc-9 shop/checkout.md 标题为 Checkout，未分配

  背景:
    假如 已有一份包含登录和结账样例文档的隔离需求库

  场景: 无项目路径启动 teshi 仍能加载需求库
    当 测试人员在不提供项目路径的情况下打开 teshi
    那么 需求标签页列出文档 "doc-12"、"doc-37" 和 "doc-9"
    并且 需求标签页显示文档 "doc-12" 的正文
