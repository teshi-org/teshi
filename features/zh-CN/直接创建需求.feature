# language: zh-CN
# Requirement: doc-23; executable via teshi-requirement-cli-runner.
# 2026-10-27: 5/5 scenarios passed; see doc/requirements-testing-2026-10-27.md.
@cli
功能: 直接创建需求

  场景: 从文件创建并显式关联项目
    假如 隔离需求库和标题为登录的 Markdown 文档
    当 我从该文档创建标题为登录且优先级为 P1 的需求并显式指定项目和请求 JSON
    那么 创建成功且 JSON 包含稳定 ID、路径、版本和 planned 项目关联

  场景: 从标准输入创建且不隐式关联
    假如 隔离需求库
    当 我从标准输入的 Markdown 创建需求并请求 JSON 且不指定项目
    那么 创建成功且需求没有项目关联

  场景: 拒绝标题不匹配
    假如 包含已有文档的隔离需求库
    当 我创建标题与 Markdown 一级标题不同的需求并请求 JSON
    那么 创建失败并返回 JSON 诊断且已有文档保持不变

  场景: 并发创建保留所有文档
    假如 包含已删除保留 ID 的隔离需求库
    当 两个客户端并发创建独立需求并请求 JSON
    那么 两次均成功且 ID 和路径不同并且不复用保留 ID

  场景: 保存失败不残留半成品
    假如 索引无法替换的隔离需求库
    当 我创建需求并请求 JSON
    那么 创建失败且不残留新增 Markdown 文档
