# language: zh-CN

@cli
功能: 工作区级测试点
  同一工作区的成员共用一份测试点文件；未注册目录保持项目内路径。

  背景:
    假如 隔离的工作区注册表目录

  @teshi-tp:tp-19
  场景: 成员之间共享测试点
    假如 工作区 demo 包含成员 alpha 和 beta
    当 我在 alpha 中保存一个测试点
    那么 在 beta 中读取测试点时能看到该测试点
    并且 测试点文件位于工作区状态目录下的 testpoints/testpoints.json
    并且 alpha 和 beta 目录下都没有新建 testpoints 目录

  @teshi-tp:tp-19
  场景: 未注册目录保持项目内的测试点路径
    假如 一个未注册的项目目录 outside
    当 我在 outside 中保存一个测试点
    那么 测试点写入 outside 下的 testpoints/testpoints.json

  @teshi-tp:tp-19
  场景: 导入需求时改写的是共享测试点文件
    假如 工作区 demo 的共享测试点文件中有关联旧需求文档的测试点
    并且 成员 alpha 目录下另有一份内容不同的旧版测试点文件
    当 我在 alpha 中把项目需求导入用户级需求库
    那么 共享测试点文件中的需求链接被改写为新文档
    并且 alpha 目录下的旧版测试点文件保持不变

  @teshi-tp:tp-20
  场景大纲: 注册表损坏时测试点操作明确失败
    假如 工作区注册表文件内容已损坏
    并且 一个项目目录 alpha
    当 我在 alpha 中<操作>
    那么 操作失败并提示注册表解析错误
    并且 alpha 目录下没有新建或改写测试点文件

    例子:
      | 操作       |
      | 读取测试点 |
      | 保存测试点 |
