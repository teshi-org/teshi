# language: zh-CN

@integration
功能: 思维导图中跨文件的步骤复用检测
  # 中等粒度：关注 StepIndex 构建以及思维导图树中跨文件步骤复用的可视化。
  # 稳定契约：`StepIndex::build` 跨功能和场景规范化步骤文本，
  #   `MindMapIndex::build_index` 构建字典树并将相同步骤文本合并为单个节点，
  #   `selected_node_context` 返回 `MindMapContext`（包含 path_labels 和 location_count）

  背景:
    假如 项目目录包含两个功能文件:
      | 文件               | 内容                                                    |
      | login.feature      | 场景: 登录 — 假如 我在登录页面             |
      |                    | 场景: 登出 — 假如 我在登录页面            |
      | dashboard.feature  | 场景: 查看仪表盘 — 假如 我在登录页面    |
      并且 TUI 已运行并加载了项目

  场景: 在思维导图树中识别跨文件复用的步骤
    当 按下 `2` 切换到思维导图标签页
      并且 展开树节点直到看到步骤级叶节点
    那么 步骤"我在登录页面"应显示为单个节点
      并且 该节点应指示 location_count >= 3（或显示它在多个位置被复用）
      并且 选择该节点应显示所有出现路径的列表
          （login.feature 和 dashboard.feature）

  场景: 使用 Tab 键循环查看复用步骤的位置
    假如 思维导图标签页已激活且选中了一个跨文件复用的步骤节点
    当 按下 `Tab`
    那么 右侧预览面板应跳转到该步骤的下一个出现位置
      并且 预览标题应更新以显示对应的功能文件名和场景名
    当 多次按下 `Tab`
    那么 它应循环显示该步骤的所有出现位置并最终回到第一个
