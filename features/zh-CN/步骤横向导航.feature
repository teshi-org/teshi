# language: zh-CN
# Requirement: doc-33; partially bound; see test results for scenario status.
@tui
功能: 步骤横向导航

  场景: 访问长步骤文本
    假如 步骤文本宽于细节面板
    当 我在面板聚焦时按 Shift 和右方向键
    那么 文本横向滚动且面板焦点不变

  场景: 停在内容边界
    假如 步骤细节已滚动至最右边
    当 我继续向右滚动再加宽面板
    那么 滚动位置保持在内容范围内且滚动条反映该位置
