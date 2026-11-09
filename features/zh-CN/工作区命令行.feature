# language: zh-CN

@cli
功能: 工作区命令行
  通过 teshi workspace 命令创建、附加、查看和归档工作区成员。

  背景:
    假如 隔离的工作区注册表目录

  @teshi-tp:tp-6
  场景: 用两个已有目录创建工作区
    假如 两个已存在且未注册的目录 alpha 和 beta
    当 我用这两个目录创建名为 demo 的工作区并请求 JSON
    那么 创建成功且 alpha 为 slot 0、beta 为 slot 1
    并且 重新读取注册表仍能看到工作区 demo

  @teshi-tp:tp-6
  场景: 同名目录的成员名自动去重
    假如 两个位于不同父目录且同名为 alpha 的目录
    当 我用这两个目录创建工作区
    那么 成员名分别为 alpha 和 alpha-2

  @teshi-tp:tp-6
  场景大纲: 拒绝无效的创建请求
    假如 已存在名为 demo 的工作区
    当 我以<无效输入>创建工作区
    那么 创建失败并提示“<提示>”
    并且 注册表中的工作区数量不变

    例子:
      | 无效输入            | 提示                             |
      | 空白名称            | workspace name must not be empty |
      | 已被占用的名称 demo | already exists                   |
      | 不存在的目录        | resolve directory                |

  @teshi-tp:tp-7
  场景: 从仓库创建 worktree 并注册为工作区
    假如 一个有提交且未注册的 git 仓库
    当 我从该仓库以新分支 feature/x 和基点 main 创建工作区
    那么 新 worktree 目录已检出分支 feature/x
    并且 仓库为 slot 0 且 worktree 成员记录分支 feature/x

  @teshi-tp:tp-7 @teshi-tp:tp-11
  场景大纲: worktree 参数不完整时拒绝执行
    假如 一个有提交的 git 仓库
    当 我执行 <命令> 并只提供 <参数>
    那么 命令因参数错误失败并提示“<提示>”
    并且 没有创建任何 worktree 或分支

    例子:
      | 命令             | 参数               | 提示                                           |
      | workspace new    | --from 和 --branch | --from requires both --branch and --worktree   |
      | workspace new    | --branch           | --branch, --worktree, and --base require --from |
      | workspace new    | --base             | --base requires --from                         |
      | workspace attach | --from             | --from requires --branch                       |
      | workspace attach | --base             | --branch and --base require --from             |

  @teshi-tp:tp-8
  场景大纲: 请求被拒绝时不残留 worktree 和分支
    假如 <前置情形>
    当 我用 <命令> 从该仓库以新分支 feature/x 创建 worktree
    那么 命令失败并提示“<提示>”
    并且 目标 worktree 目录不存在
    并且 仓库中不存在分支 feature/x

    例子:
      | 前置情形                  | 命令                                 | 提示                                  |
      | 仓库已属于工作区 first    | workspace new                        | already belongs to workspace 'first'  |
      | 已存在名为 demo 的工作区  | workspace new --name demo            | already exists                        |
      | 仓库已属于工作区 demo     | workspace attach --workspace missing | unknown workspace 'missing'           |

  @teshi-tp:tp-9
  场景大纲: 一个目录只能属于一个工作区
    假如 目录 alpha 已属于工作区 demo
    当 我用<路径写法>把 alpha 附加到工作区
    那么 附加失败并提示该目录已属于工作区 demo

    例子:
      | 路径写法             |
      | 原始路径             |
      | 含 .. 冗余段的路径   |
      | 大小写不同的路径     |

  @teshi-tp:tp-10
  场景大纲: 附加已有目录时选择目标工作区
    假如 注册表中<已有工作区>
    并且 一个已存在且未注册的目录 gamma
    当 我附加 gamma 并<工作区参数>
    那么 <结果>

    例子:
      | 已有工作区          | 工作区参数     | 结果                             |
      | 只有工作区 one      | 不指定工作区   | gamma 成为工作区 one 的成员      |
      | 有工作区 one 和 two | 不指定工作区   | 附加失败并要求传入 --workspace   |
      | 有工作区 one 和 two | 指定工作区 two | gamma 成为工作区 two 的成员      |
      | 没有任何工作区      | 不指定工作区   | 附加失败并提示先创建工作区       |

  @teshi-tp:tp-11
  场景: 在已有工作区中新建 worktree 成员
    假如 工作区 demo 已包含一个有提交的 git 仓库
    当 我从该仓库以新分支 feature/x 和基点 main 把尚不存在的路径 wt 附加到工作区
    那么 wt 目录已检出分支 feature/x
    并且 工作区新成员 wt 记录分支 feature/x

  @teshi-tp:tp-12
  场景大纲: 归档成员
    假如 工作区 demo 包含仓库成员和一个<成员类型>成员 wt
    当 我归档成员 wt
    那么 <目录结果>
    并且 注册表中不再有成员 wt
    并且 输出说明“<动作说明>”

    例子:
      | 成员类型        | 目录结果                         | 动作说明                          |
      | linked worktree | wt 目录已删除且其分支仍存在      | worktree removed, branch kept     |
      | 普通目录        | wt 目录仍然存在                  | directory kept, unregistered only |

  @teshi-tp:tp-13
  场景大纲: 归档的安全保护
    假如 <前置情形>
    当 我不加 --force 归档<目标>
    那么 归档失败并提示“<提示>”
    并且 注册表与成员目录保持不变

    例子:
      | 前置情形                                         | 目标                         | 提示                |
      | 工作区 demo 中的 worktree 成员 wt 有未提交修改   | 成员 wt                      | uncommitted changes |
      | 工作区 demo 中 worktree 成员 wt 的 git 状态无法读取 | 成员 wt                   | state is unknown    |
      | 工作区 demo 只有一个成员 alpha                   | 成员 alpha                   | only one member     |
      | 工作区 demo 有两个成员                           | 不存在的成员 ghost           | unknown member      |
      | 只存在工作区 demo                                | 工作区 missing 中的成员 alpha | unknown workspace   |

  @teshi-tp:tp-13
  场景: 加 --force 归档有未提交修改的 worktree
    假如 工作区 demo 中的 worktree 成员 wt 有未提交修改
    当 我加 --force 归档成员 wt
    那么 归档成功且 wt 目录已删除
    并且 注册表中不再有成员 wt

  @teshi-tp:tp-14
  场景: 空注册表时列出工作区
    假如 注册表中没有任何工作区
    当 我列出工作区
    那么 输出提示尚无已注册的工作区

  @teshi-tp:tp-14
  场景大纲: 以 JSON 查看工作区
    假如 已存在包含两个成员的工作区 demo
    当 我以 JSON 执行 <查看命令>
    那么 输出是 ok 为 true 的 JSON 信封
    并且 信封中包含<内容>

    例子:
      | 查看命令            | 内容                       |
      | workspace list      | 工作区列表和注册表目录     |
      | workspace show demo | 工作区 demo 及其两个成员   |

  @teshi-tp:tp-14
  场景大纲: show 的目标工作区解析
    假如 注册表中<已有工作区>
    并且 当前目录是<当前目录>
    当 我执行 show 并<参数>
    那么 <结果>

    例子:
      | 已有工作区          | 当前目录           | 参数                | 结果                                   |
      | 有工作区 one 和 two | 工作区 two 的成员  | 不指定工作区        | 显示工作区 two                         |
      | 只有工作区 one      | 未注册目录         | 不指定工作区        | 显示工作区 one                         |
      | 有工作区 one 和 two | 未注册目录         | 不指定工作区        | 失败并提示当前目录不属于任何工作区     |
      | 只有工作区 one      | 未注册目录         | 指定工作区 missing  | 失败并提示 unknown workspace           |
