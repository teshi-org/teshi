# language: zh-CN
# Requirement: doc-32; authored only; not bound, executed or accepted.
@tui
功能: 逐条步骤差异

  场景: 浏览新增和删除步骤
    假如 场景包含一个新增步骤和一个删除步骤
    当 我聚焦步骤细节并依次按 j 和 k
    那么 选择在独立标记的变化之间移动且包括已删除步骤

  场景: 首条变化的导航边界
    假如 首条步骤变化已选中
    当 我按 k
    那么 选择保持在首条变化
