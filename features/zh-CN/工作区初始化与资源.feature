@cli @workspace
Feature: 工作区初始化与资源分配
  workspace.toml 声明新成员的初始化步骤；每个工作区拥有独立的端口段和可共享的构建目录。

  Background:
    Given 隔离的工作区注册表目录

  @teshi-tp:tp-15
  Scenario: include 把文件和目录复制到新成员
    Given 工作区 demo 的首成员包含文件 teshi.toml 和目录 config
    And workspace.toml 声明 include 为 teshi.toml 和 config
    When 我把一个空目录 beta 附加到工作区 demo
    Then beta 中出现与首成员内容相同的 teshi.toml 和 config
    And 初始化报告复制了 2 项

  @teshi-tp:tp-15
  Scenario Outline: include 的特殊条目
    Given 工作区 demo 的 workspace.toml 声明 include 为<条目>
    And <前置情形>
    When 我把目录 beta 附加到工作区 demo
    Then <结果>

    Examples:
      | 条目         | 前置情形                                       | 结果                                                       |
      | teshi.toml   | beta 中已有内容不同的 teshi.toml               | 初始化报告跳过该项且 beta 中的 teshi.toml 内容不变         |
      | absent.toml  | 首成员中没有 absent.toml                       | 附加成功且初始化报告列出缺失项 absent.toml                 |
      | ../secret    | 首成员目录保持默认                             | 初始化失败并提示条目必须留在成员目录内                     |
      | 一个绝对路径 | 首成员目录保持默认                             | 初始化失败并提示条目必须留在成员目录内                     |
      | config       | 首成员的 config 中含有指向自身的目录符号链接   | 初始化失败并提示包含目录符号链接                           |

  @teshi-tp:tp-15
  Scenario: 跳过初始化时不复制文件
    Given 工作区 demo 的 workspace.toml 声明 include 为 teshi.toml
    When 我以跳过初始化的方式把目录 beta 附加到工作区 demo
    Then beta 中没有 teshi.toml

  @teshi-tp:tp-16
  Scenario: setup 命令在成员目录内带声明的环境变量执行
    Given 工作区 demo 的 workspace.toml 声明一条把环境变量 MARKER 写入 marker.txt 的 setup 命令
    And workspace.toml 的 env 中声明 MARKER 为 ok
    When 我把目录 beta 附加到工作区 demo
    Then beta 中的 marker.txt 内容为 ok
    And 初始化报告执行了 1 条 setup 命令

  @teshi-tp:tp-16
  Scenario: setup 命令失败时给出可诊断的错误
    Given 工作区 demo 的 workspace.toml 声明一条向 stderr 输出 boom 并以状态 3 退出的 setup 命令
    When 我把目录 beta 附加到工作区 demo
    Then 命令失败且错误中包含该 setup 命令、beta 的路径、退出状态 3 和 boom

  @teshi-tp:tp-16
  Scenario: init 对所有非首成员重新执行
    Given 工作区 demo 有首成员 alpha 和成员 beta、gamma
    And workspace.toml 声明一条在成员目录写入 marker.txt 的 setup 命令
    When 我对工作区 demo 执行 init
    Then beta 和 gamma 中都出现 marker.txt
    And alpha 中没有 marker.txt

  @teshi-tp:tp-17
  Scenario Outline: 每个工作区拥有独立的端口段
    Given 注册表中已有 <已有数量> 个工作区
    When 我新建一个包含两个成员的工作区
    Then 新工作区的端口基址为 <端口基址>
    And 两个成员的端口分别为 <成员端口>
    And 没有成员被分配到端口 20253

    Examples:
      | 已有数量 | 端口基址 | 成员端口       |
      | 0        | 20300    | 20300 和 20301 |
      | 1        | 20400    | 20400 和 20401 |
      | 2        | 20500    | 20500 和 20501 |

  @teshi-tp:tp-17 @teshi-tp:tp-10
  Scenario: 归档后新成员复用最小空闲槽位
    Given 工作区 demo 有 slot 0、1、2 三个成员
    And 我已归档 slot 1 的成员
    When 我附加一个新目录到工作区 demo
    Then 新成员的槽位为 1
    And 新成员的端口为工作区端口基址加 1

  @teshi-tp:tp-18
  Scenario Outline: 共享构建目录的取值
    Given 首成员<首成员类型>
    When 我<目标目录参数>创建工作区
    Then 工作区的共享构建目录为<结果>
    And 从成员启动的 agent <CARGO 变量>

    Examples:
      | 首成员类型      | 目标目录参数                    | 结果                    | CARGO 变量                              |
      | 是 Cargo 项目   | 不指定 --target-dir             | 首成员下的 target 目录  | 获得指向该目录的 CARGO_TARGET_DIR       |
      | 不是 Cargo 项目 | 不指定 --target-dir             | 未设置                  | 没有 CARGO_TARGET_DIR                   |
      | 不是 Cargo 项目 | 指定 --target-dir 为 shared-target | shared-target        | 获得指向 shared-target 的 CARGO_TARGET_DIR |

  @teshi-tp:tp-18
  Scenario: 用户已有的 CARGO_TARGET_DIR 优先
    Given 首成员为 Cargo 项目的工作区 demo
    And Teshi 自身环境中 CARGO_TARGET_DIR 为 my-target
    When 我在工作区成员中启动 agent
    Then agent 环境中的 CARGO_TARGET_DIR 为 my-target
