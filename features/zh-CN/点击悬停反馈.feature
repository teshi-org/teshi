# language: zh-CN
# Requirement: doc-35; partially bound; see test results for scenario status.
@tui
功能: 点击悬停反馈

  场景: 高亮可点击目标
    假如 可见操作按钮
    当 我将指针移入再移出按钮
    那么 目标获得并移除一致的可见悬停样式

  场景: 弹层阻止底层反馈
    假如 弹层遮住页面操作
    当 我将指针移到被遮住的操作位置
    那么 被遮住的操作不显示为可用
