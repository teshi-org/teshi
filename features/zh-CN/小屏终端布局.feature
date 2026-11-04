# language: zh-CN
# Requirement: doc-31; partially bound; see test results for scenario status.
@tui
功能: 小屏终端布局

  场景: 保证聚焦内容可访问
    假如 窄终端打开需求页
    当 我在各面板间移动焦点
    那么 每个聚焦面板变为可见且不丢失修改

  场景: 调整尺寸不残留点击区域
    假如 展示多个面板的终端
    当 我缩小后再放大终端
    那么 只有可见控件响应点击且保留选择
