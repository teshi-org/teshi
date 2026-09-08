## 背景与动机

Teshi 的自动更新目前无法可靠适配需要代理访问 GitHub、同时又无法访问外部 CRL/OCSP 服务的 Windows 内网。更新传输需要遵循用户代理配置，并在不关闭证书链、域名、有效期和签名验证的前提下，避免 Windows 在线吊销检查阻断更新。

## 变更内容

- 为 workspace 的 `reqwest` 启用 `system-proxy`，使更新客户端除代理环境变量外还可读取 Windows 和 macOS 的系统手动代理设置。
- 为 Windows 上的 GitHub 更新传输使用系统根证书快照和 rustls/webpki 校验，避免 `rustls-platform-verifier` 触发不可达的在线 CRL/OCSP 查询。
- 将放宽吊销检查的行为严格限定在 `teshi-update` 的 GitHub 客户端，不关闭证书链、域名、有效期或签名验证，也不改变其他网络客户端的 TLS 策略。
- 保留更新 URL 与重定向来源白名单、HTTPS 限制、超时、大小和 SHA256 校验。
- 增加代理选择、系统根证书加载失败、代理绕过和 TLS 拦截代理等场景的自动化与 Windows 验证覆盖。
- 记录静态系统代理、环境变量代理、PAC/WPAD 和代理认证的支持边界。

## 能力范围

### 新增能力

- `proxied-self-update-transport`：定义 Teshi 自更新访问 GitHub 时的代理发现、Windows 离线证书校验、安全边界和失败行为。

### 修改能力

无。`application-self-update` 仍位于尚未归档的 `add-application-self-update` 变更中，本变更以独立传输能力补充该工作，不声明修改尚未进入主规格的能力。

## 影响范围

- 主要代码：workspace `Cargo.toml`、`crates/teshi-update/src/github.rs` 及其测试。
- 依赖：启用 reqwest `system-proxy`；可能直接使用已在依赖图中的 `rustls-native-certs` 来加载平台根证书。
- 平台：Windows 更新传输行为发生变化；其他平台继续使用现有 TLS 校验，仅获得已支持平台的系统代理发现。
- 安全：Windows 更新连接不再依赖在线吊销状态，但仍执行标准 WebPKI 校验；系统根证书快照不能保留 Windows 证书存储中的全部动态不信任和约束语义，此差异需明确记录。
- 关联变更：依赖 `add-application-self-update` 已实现的 `GithubHttp`、GitHub 来源白名单、下载校验和更新测试基础。
