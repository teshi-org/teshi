# language: zh-CN

@cli
@validation-e2e
功能: Gherkin 校验自举
  作为 Teshi 用户
  我希望 `teshi check` 在执行前校验 Feature 源文本
  从而格式错误可获得明确提示，合法文件仍然可以使用

  背景:
    假如 一个隔离的 Feature 校验项目

  场景: 诊断中文步骤缺少分隔符
    假如 项目包含一个格式错误的 zh-CN Feature
    当 测试人员以 JSON 运行 teshi check 校验格式错误的 Feature
    那么 命令以退出码 1 失败
    并且 JSON 报告包含诊断代码 "missing_step_separator"，位于第 4 行第 6 列
    并且 诊断建议为 "当 用户登录"

  场景: 接受有效源文本和步骤附件
    假如 项目包含带有描述文本和步骤附件的有效 Feature
    当 测试人员以 JSON 运行 teshi check 校验有效的 Feature
    那么 命令成功
    并且 JSON 报告包含零个错误
    并且 JSON 报告不包含无法识别的可执行区域行

  场景: 接受有效中文源文本
    假如 项目包含一个有效的 zh-CN Feature
    当 测试人员以 JSON 运行 teshi check 校验有效的 zh-CN Feature
    那么 命令成功
    并且 JSON 报告包含零个错误

  场景: 拒绝可执行区域中无法识别的文本
    假如 项目包含一个带有无法识别可执行区域文本的 Feature
    当 测试人员以 JSON 运行 teshi check 校验该 Feature
    那么 命令以退出码 1 失败
    并且 JSON 报告包含诊断代码 "unrecognized_executable_line"，位于第 4 行第 5 列

  场景: 明确范围只校验选定的 Feature
    假如 项目包含一个有效和一个无效的 Feature
    当 测试人员以 JSON 运行 teshi check 校验无效的 Feature
    那么 命令以退出码 1 失败
    并且 JSON 范围为 ["features/invalid.feature"]
    并且 JSON 报告包含诊断代码 "missing_step_separator"，位于第 3 行第 10 列

  场景: 全部范围报告有序诊断
    假如 项目包含一个有效和一个无效的 Feature
    当 测试人员以 JSON 运行 teshi check 校验所有 Feature
    那么 命令以退出码 1 失败
    并且 JSON 范围为 ["features/invalid.feature", "features/valid.feature"]
    并且 全部范围 JSON 报告包含有序诊断

  场景: 警告不会导致校验失败
    假如 项目包含一个只有警告的 Feature
    当 测试人员以 JSON 运行 teshi check 校验只有警告的 Feature
    那么 命令成功
    并且 警告代码 scenario_starts_without_given 可见

  场景: 合法的重复 Given 步骤不会阻断校验
    假如 项目包含一个重复 Given 的 Feature
    当 测试人员以 JSON 运行 teshi check 校验重复 Given 的 Feature
    那么 命令成功
    并且 JSON 报告包含警告代码 "missing_when" 和 "missing_then"

  场景: 目录运行忽略无效兄弟 Feature
    假如 项目包含一个有效的选定目录和一个无效的兄弟 Feature
    当 测试人员对选定目录启动 BDD 运行
    那么 命令成功
    并且 选定目录运行成功且不包含兄弟目录诊断

  场景: 拒绝冲突的校验范围
    假如 项目包含一个无效的 Feature
    当 测试人员同时使用冲突的范围选项运行 teshi check
    那么 命令以退出码 2 失败
    并且 输出说明范围选项互斥

  场景: unbound 拒绝无效 Feature
    假如 项目包含一个无效的 Feature
    当 测试人员列出无效 Feature 的未绑定步骤
    那么 命令以退出码 1 失败
    并且 输出包含 missing_step_separator 校验证据
    并且 绑定命令没有返回成功的空列表

  场景: 校验失败时 next-unbound 保留活动步骤
    假如 项目已有一个活动步骤并且包含一个无效 Feature
    当 测试人员为无效 Feature 选择下一个未绑定步骤
    那么 命令以退出码 1 失败
    并且 输出包含 missing_step_separator 校验证据
    并且 现有活动步骤保持不变

  场景: 无效 Feature 不会启动 BDD 运行
    假如 项目包含一个无效的 Feature
    当 测试人员为无效 Feature 启动 BDD 运行
    那么 命令以退出码 1 失败
    并且 输出包含 missing_step_separator 校验证据
    并且 嵌套 runner 没有启动

  场景: replay 在目标操作前失败
    假如 项目包含一个无效的 Feature
    当 测试人员为无效 Feature 启动 browser 和 WinApp replay
    那么 两个 replay 命令都在目标操作前失败

  场景: 重复校验报告具有确定性
    假如 项目包含一个格式错误的 zh-CN Feature
    当 测试人员以 JSON 重复运行两次格式错误的校验
    那么 命令以退出码 1 失败
    并且 两份 JSON 报告完全一致

  场景: daemon 警告报告结构完整且可见
    假如 项目包含一个只有警告的 Feature 和一个运行中的 daemon
    当 测试人员通过 daemon 列出只有警告 Feature 的未绑定步骤
    那么 命令成功
    并且 daemon 警告报告结构完整且可见

  场景: daemon 校验错误保留结构化报告
    假如 项目包含一个无效 Feature 和一个运行中的 daemon
    当 测试人员通过 daemon 列出无效 Feature 的未绑定步骤
    那么 命令以退出码 1 失败
    并且 daemon 错误包含结构化校验报告
