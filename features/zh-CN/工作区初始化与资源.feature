# language: zh-CN
# Authored only; not bound, executed or accepted: `teshi workspace` is not implemented yet.
@cli
功能: 工作区初始化与资源分配
  workspace.toml 声明新成员的初始化步骤；每个工作区拥有独立的端口段和可共享的构建目录。

  背景:
    假如 隔离的工作区注册表目录

  @teshi-tp:tp-15
  场景: include 把文件和目录复制到新成员
    假如 工作区 demo 的首成员包含文件 teshi.toml 和目录 config
    并且 workspace.toml 声明 include 为 teshi.toml 和 config
    当 我把一个空目录 beta 附加到工作区 demo
    那么 beta 中出现与首成员内容相同的 teshi.toml 和 config
    并且 初始化报告复制了 2 项

  @teshi-tp:tp-15
  场景大纲: include 的特殊条目
    假如 工作区 demo 的 workspace.toml 声明 include 为<条目>
    并且 <前置情形>
    当 我把目录 beta 附加到工作区 demo
    那么 <结果>

    例子:
      | 条目         | 前置情形                                       | 结果                                                       |
      | teshi.toml   | beta 中已有内容不同的 teshi.toml               | 初始化报告跳过该项且 beta 中的 teshi.toml 内容不变         |
      | absent.toml  | 首成员中没有 absent.toml                       | 附加成功且初始化报告列出缺失项 absent.toml                 |
      | ../secret    | 首成员目录保持默认                             | 初始化失败并提示条目必须留在成员目录内                     |
      | 一个绝对路径 | 首成员目录保持默认                             | 初始化失败并提示条目必须留在成员目录内                     |
      | config       | 首成员的 config 中含有指向自身的目录符号链接   | 初始化失败并提示包含目录符号链接                           |

  @teshi-tp:tp-15
  场景: 跳过初始化时不复制文件
    假如 工作区 demo 的 workspace.toml 声明 include 为 teshi.toml
    当 我以跳过初始化的方式把目录 beta 附加到工作区 demo
    那么 beta 中没有 teshi.toml

  @teshi-tp:tp-16
  场景: setup 命令在成员目录内带声明的环境变量执行
    假如 工作区 demo 的 workspace.toml 声明一条把环境变量 MARKER 写入 marker.txt 的 setup 命令
    并且 workspace.toml 的 env 中声明 MARKER 为 ok
    当 我把目录 beta 附加到工作区 demo
    那么 beta 中的 marker.txt 内容为 ok
    并且 初始化报告执行了 1 条 setup 命令

  @teshi-tp:tp-16
  场景: setup 命令失败时给出可诊断的错误
    假如 工作区 demo 的 workspace.toml 声明一条向 stderr 输出 boom 并以状态 3 退出的 setup 命令
    当 我把目录 beta 附加到工作区 demo
    那么 命令失败且错误中包含该 setup 命令、beta 的路径、退出状态 3 和 boom

  @teshi-tp:tp-16
  场景: init 对所有非首成员重新执行
    假如 工作区 demo 有首成员 alpha 和成员 beta、gamma
    并且 workspace.toml 声明一条在成员目录写入 marker.txt 的 setup 命令
    当 我对工作区 demo 执行 init
    那么 beta 和 gamma 中都出现 marker.txt
    并且 alpha 中没有 marker.txt

  @teshi-tp:tp-17
  场景大纲: 每个工作区拥有独立的端口段
    假如 注册表中已有 <已有数量> 个工作区
    当 我新建一个包含两个成员的工作区
    那么 新工作区的端口基址为 <端口基址>
    并且 两个成员的端口分别为 <成员端口>
    并且 没有成员被分配到端口 20253

    例子:
      | 已有数量 | 端口基址 | 成员端口       |
      | 0        | 20300    | 20300 和 20301 |
      | 1        | 20400    | 20400 和 20401 |
      | 2        | 20500    | 20500 和 20501 |

  @teshi-tp:tp-17 @teshi-tp:tp-10
  场景: 归档后新成员复用最小空闲槽位
    假如 工作区 demo 有 slot 0、1、2 三个成员
    并且 我已归档 slot 1 的成员
    当 我附加一个新目录到工作区 demo
    那么 新成员的槽位为 1
    并且 新成员的端口为工作区端口基址加 1

  @teshi-tp:tp-18
  场景大纲: 共享构建目录的取值
    假如 首成员<首成员类型>
    当 我<目标目录参数>创建工作区
    那么 工作区的共享构建目录为<结果>
    并且 从成员启动的 agent <CARGO 变量>

    例子:
      | 首成员类型      | 目标目录参数                    | 结果                    | CARGO 变量                              |
      | 是 Cargo 项目   | 不指定 --target-dir             | 首成员下的 target 目录  | 获得指向该目录的 CARGO_TARGET_DIR       |
      | 不是 Cargo 项目 | 不指定 --target-dir             | 未设置                  | 没有 CARGO_TARGET_DIR                   |
      | 不是 Cargo 项目 | 指定 --target-dir 为 shared-target | shared-target        | 获得指向 shared-target 的 CARGO_TARGET_DIR |

  @teshi-tp:tp-18
  场景: 用户已有的 CARGO_TARGET_DIR 优先
    假如 首成员为 Cargo 项目的工作区 demo
    并且 Teshi 自身环境中 CARGO_TARGET_DIR 为 my-target
    当 我在工作区成员中启动 agent
    那么 agent 环境中的 CARGO_TARGET_DIR 为 my-target
