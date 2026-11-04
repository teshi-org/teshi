# language: zh-CN
# 来源：doc-45，revision d64e954d0b66d9d6；用户授权直接生成，未关联已审批测试点。
@tui
功能: 项目目录加载
  默认加载项目的 features，显式 -R 才递归扫描整个目标目录。

  背景:
    假如 已准备隔离的启动目录和项目 xxx
    并且 隔离项目不使用真实用户的配置和会话数据

  场景大纲: 默认打开项目只加载 features
    假如 项目 xxx 的 features 中有“登录.feature”和“auth/退出.feature”
    并且 项目 xxx 的其他目录中有“外层功能.feature”
    并且 features 中没有语言子目录
    当 我在“<工作目录>”中启动“<命令>”
    那么 Feature 列表恰好包含“登录”和“退出”
    并且 活动项目根目录为项目 xxx

    例子:
      | 工作目录 | 命令      |
      | 项目 xxx | teshi     |
      | 项目 xxx | teshi .   |
      | 启动目录 | teshi xxx |

  场景大纲: 按系统界面语言选择 Feature
    假如 项目 xxx 的 features 中同时存在 zh-CN 和 en-US 语言目录
    并且 每个语言目录中都有直属和深层子目录中的 Feature
    并且 features 根层还有一个公共 Feature
    并且 系统首选界面语言为“<语言>”
    当 我在启动目录中启动“teshi xxx”
    那么 Feature 列表恰好包含“<语言>”目录及其子目录中的 Feature
    并且 活动项目根目录为项目 xxx

    例子:
      | 语言  |
      | zh-CN |
      | en-US |

  场景大纲: 显式递归加载整个目标目录
    假如 项目 xxx 的 features 中有“登录.feature”
    并且 项目 xxx 的其他深层目录中有“其他功能.feature”
    并且 项目 xxx 外有“目标之外.feature”
    当 我在“<工作目录>”中启动“<命令>”
    那么 Feature 列表恰好包含“登录”和“其他功能”
    并且 活动项目根目录为项目 xxx

    例子:
      | 工作目录 | 命令         |
      | 项目 xxx | teshi -R     |
      | 项目 xxx | teshi . -R   |
      | 启动目录 | teshi xxx -R |

  场景大纲: 缺少 features 不扩大默认扫描范围
    假如 项目 xxx 没有 features 目录
    并且 项目 xxx 的其他目录中有“其他功能.feature”
    当 我在“<工作目录>”中启动“<命令>”
    那么 Feature 列表为空
    并且 下一次外部文件检查完成后 Feature 列表仍为空

    例子:
      | 工作目录 | 命令      |
      | 项目 xxx | teshi     |
      | 项目 xxx | teshi .   |
      | 启动目录 | teshi xxx |

  场景大纲: 缺少 features 时加载根目录文件
    假如 项目 xxx 没有 features 目录
    并且 项目 xxx 根目录有“根层功能.feature”
    并且 项目 xxx 的其他目录中有“其他功能.feature”
    当 我在“<工作目录>”中启动“<命令>”
    那么 Feature 列表恰好包含“根层功能”
    并且 项目 xxx 没有 features 目录

    例子:
      | 工作目录 | 命令      |
      | 项目 xxx | teshi     |
      | 项目 xxx | teshi .   |
      | 启动目录 | teshi xxx |

  场景大纲: 缺少 features 时显式递归加载根目录和子目录
    假如 项目 xxx 没有 features 目录
    并且 项目 xxx 根目录有“根层功能.feature”
    并且 项目 xxx 的其他目录中有“其他功能.feature”
    当 我在“<工作目录>”中启动“<命令>”
    那么 Feature 列表恰好包含“根层功能”和“其他功能”
    并且 项目 xxx 没有 features 目录

    例子:
      | 工作目录 | 命令                    |
      | 项目 xxx | teshi -R                |
      | 项目 xxx | teshi . --recursive     |
      | 启动目录 | teshi xxx -R            |
      | 启动目录 | teshi xxx --recursive   |

  场景大纲: 缺少 features 且没有功能文件时打开空项目
    假如 项目 xxx 没有 features 目录
    当 我在“项目 xxx”中启动“<命令>”
    那么 Feature 列表为空
    并且 项目 xxx 没有 features 目录

    例子:
      | 命令                |
      | teshi               |
      | teshi -R            |
      | teshi --recursive   |

  场景: 语言扫描目录不改变项目上下文
    假如 项目 xxx 的 features 中存在与系统界面语言匹配的语言目录
    并且 项目 xxx 的配置指定隔离的测试运行工作目录
    并且 项目 xxx 已有保存的项目会话
    当 我在启动目录中启动“teshi xxx”
    那么 加载项目 xxx 的配置和会话
    并且 测试运行路径按项目 xxx 的配置解析
    并且 项目状态不会写入所选语言目录

  场景: 默认后台刷新保持所选语言范围
    假如 系统首选界面语言为“zh-CN”
    并且 项目 xxx 的 features 中同时存在 zh-CN 和 en-US 语言目录
    当 我在启动目录中启动“teshi xxx”
    并且 我在 zh-CN 的深层子目录中新增“新增中文.feature”
    并且 我在 en-US 中新增“新增英文.feature”
    并且 我在项目 xxx 的 features 之外新增“外层新增.feature”
    那么 下一次外部文件检查完成后可以看到“新增中文”
    并且 Feature 列表不包含“新增英文”和“外层新增”

  场景: 没有语言目录时后台刷新覆盖 features 子目录
    假如 项目 xxx 的 features 中没有语言子目录
    当 我在启动目录中启动“teshi xxx”
    并且 我在 features 的深层子目录中新增“新增功能.feature”
    并且 我在 features 之外新增“外层新增.feature”
    那么 下一次外部文件检查完成后可以看到“新增功能”
    并且 Feature 列表不包含“外层新增”

  场景: 显式递归的后台刷新覆盖目标目录
    假如 项目 xxx 的 features 中有“登录.feature”
    当 我在启动目录中启动“teshi xxx -R”
    并且 我在项目 xxx 的 features 之外的深层目录新增“深层新增.feature”
    并且 我在项目 xxx 之外新增“目标之外.feature”
    那么 下一次外部文件检查完成后可以看到“深层新增”
    并且 Feature 列表不包含“目标之外”

  # 待补充：性能数据规模和响应时限确认后增加可量化场景。
  # 待补充：缺少匹配语言目录的提示和回退策略确认后增加边界场景。
