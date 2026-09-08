## ADDED Requirements

### Requirement: 自动发现更新代理
Teshi 自更新传输 SHALL 遵循 `HTTPS_PROXY`/`https_proxy`、`ALL_PROXY`/`all_proxy` 和 `NO_PROXY`/`no_proxy`。在受支持的平台上且没有对应环境变量时，传输 SHALL 使用 reqwest `system-proxy` 能力发现系统手动代理配置。

#### Scenario: 环境变量代理
- **WHEN** 进程设置有效的 `HTTPS_PROXY`，且更新客户端请求 GitHub HTTPS 资源
- **THEN** 更新检查和下载通过该代理连接，不尝试直连 GitHub

#### Scenario: 系统手动代理
- **WHEN** Windows 启用静态手动代理且进程没有设置对应代理环境变量
- **THEN** 更新客户端使用 Windows 系统代理完成 GitHub HTTPS 请求

#### Scenario: 环境变量优先
- **WHEN** 环境变量代理与系统手动代理同时存在
- **THEN** 更新客户端使用对应环境变量代理

#### Scenario: 代理绕过
- **WHEN** 目标主机匹配 `NO_PROXY` 或受支持的系统代理绕过规则
- **THEN** 更新客户端不为该目标使用代理

### Requirement: Windows 离线证书验证
Windows 自更新 GitHub client SHALL 使用从 Windows 信任存储加载的根证书快照执行 rustls WebPKI 验证，并 SHALL NOT 依赖在线 CRL/OCSP 查询。它 MUST 继续验证证书链、目标域名、有效期、用途和签名。

#### Scenario: CRL 服务不可达
- **WHEN** GitHub 经代理可达、服务器证书链受 Windows 信任，但外部 CRL/OCSP 服务不可达
- **THEN** 更新检查和下载完成 TLS 验证，不因在线吊销查询失败

#### Scenario: 企业 TLS 拦截
- **WHEN** 企业代理呈现由已安装 Windows 企业根证书签发且目标域名匹配的证书
- **THEN** 更新客户端接受该证书链并继续更新请求

#### Scenario: 不受信证书
- **WHEN** 代理或远端呈现无法链到已加载 Windows 根证书的证书
- **THEN** 更新客户端拒绝连接并报告网络或 TLS 错误

#### Scenario: 域名不匹配
- **WHEN** 代理或远端呈现受信任但不匹配 GitHub 目标域名的证书
- **THEN** 更新客户端拒绝连接

### Requirement: 证书初始化失败时默认拒绝
Windows 更新客户端 SHALL 在根证书加载失败、没有可用根证书或根证书转换失败时停止构造。它 MUST NOT 回退到关闭证书验证的 client，也 MUST NOT 在任意 TLS 错误后自动使用弱校验重试。

#### Scenario: 无可用系统根证书
- **WHEN** Windows 信任存储无法读取或没有产生可用根证书
- **THEN** 更新操作在发出 GitHub 请求前失败并返回可诊断错误

#### Scenario: TLS 握手失败
- **WHEN** 严格 TLS 握手因证书错误失败
- **THEN** 更新客户端不使用 `danger_accept_invalid_certs` 或其他弱校验路径重试

### Requirement: 保持 GitHub 更新传输边界
代理和 Windows 证书验证调整 MUST NOT 放宽现有 HTTPS、端口、目标来源或重定向策略。更新元数据和载荷 SHALL 继续接受现有大小、清单和 SHA256 校验。

#### Scenario: 代理访问允许的 GitHub 目标
- **WHEN** 允许的 GitHub URL 通过代理请求
- **THEN** 来源策略依据原始目标 URL 校验，而不是依据代理 URL 校验

#### Scenario: 非允许重定向
- **WHEN** GitHub 响应将更新请求重定向到来源白名单外的主机
- **THEN** 更新客户端拒绝该重定向，即使代理可以访问该主机

#### Scenario: 下载完整性失败
- **WHEN** 代理返回的更新载荷与声明的大小或 SHA256 不一致
- **THEN** 更新流程拒绝载荷且不进入安装

### Requirement: 限定行为范围并说明支持边界
离线吊销行为 SHALL 仅应用于 Windows 的 `teshi-update` GitHub client。其他网络 client SHALL 保持各自明确的代理和 TLS 策略。项目文档 SHALL 说明环境变量、静态系统代理、PAC/WPAD、代理认证和 Windows 吊销检查的支持边界。

#### Scenario: 本地 browser bridge
- **WHEN** native desktop 连接显式配置 `no_proxy()` 的本地 browser bridge
- **THEN** 启用 workspace `system-proxy` 不会把该本地请求发送到系统代理

#### Scenario: 非 Windows 更新客户端
- **WHEN** Teshi 在非 Windows native 平台创建更新客户端
- **THEN** 它保持现有平台 TLS verifier，不启用 Windows 专用根证书快照路径

#### Scenario: PAC 或集成认证代理
- **WHEN** 部署只提供 PAC/WPAD 或 reqwest 不支持的集成代理认证
- **THEN** Teshi 文档不将该配置声明为自动支持，并提供使用明确代理环境变量或报告不支持的指引
