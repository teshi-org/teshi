# language: zh-CN
# Requirement: doc-36; partially bound; see test results for scenario status.
@tui
功能: 合并细节布局

  场景: 两列组合辅助细节
    假如 正常宽度的需求、测试点或浏览页
    当 我查看导航和细节
    那么 导航位于左侧且两个细节面板在右列上下排列

  场景: 小屏所有面板仍可访问
    假如 窄终端显示紧凑页面
    当 我通过键盘循环聚焦每个面板
    那么 每个面板聚焦时显示并保持已有选择
