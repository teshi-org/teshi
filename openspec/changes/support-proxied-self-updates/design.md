## 背景

workspace 将 `reqwest` 配置为 `default-features = false`，并显式启用 `rustls`、`blocking`、`json` 和 `stream`。reqwest 0.13 在当前配置下已经读取 `HTTP_PROXY`、`HTTPS_PROXY`、`ALL_PROXY` 和 `NO_PROXY`；`system-proxy` 的额外价值是读取 Windows `Internet Settings` 和 macOS 系统手动代理配置，而不是开启环境变量支持。

`crates/teshi-update/src/github.rs` 中的 `GithubHttp` 创建独立的 blocking client，用于 GitHub API、更新清单、校验和与安装包下载。它没有调用 `no_proxy()`，因此启用代理发现后无需在每次请求中显式选择代理。

reqwest 0.13 的 rustls 后端默认使用 `rustls-platform-verifier`。Windows 实现会调用 `CertGetCertificateChain` 并为终端证书启用在线吊销检查。CryptoAPI 发起的 CRL/OCSP 请求不经过 reqwest 的 HTTP 代理，因此“GitHub 经代理可达”与“Windows 吊销服务可达”是两个独立条件。

## 目标与非目标

**目标：**

- 更新检查和下载自动遵循代理环境变量及受支持的系统手动代理设置。
- Windows 内网无法访问 CRL/OCSP 时，合法且受信任的 GitHub 或企业 TLS 拦截证书仍可完成更新。
- 只跳过在线吊销查询，保留证书链、域名、有效期、用途、签名和 HTTPS 校验。
- 将 TLS 策略变化限定在 `teshi-update` 的 GitHub 传输，保持其他 reqwest 客户端现有行为。
- 对代理和证书初始化失败提供可诊断、可测试且默认拒绝的错误。

**非目标：**

- 不支持通过 `danger_accept_invalid_certs(true)` 或等价方式关闭 TLS 验证。
- 不为 PAC/WPAD、NTLM、Kerberos 或任意企业代理认证实现新的协议栈。
- 不代理本地 browser bridge；现有显式 `no_proxy()` 行为保持不变。
- 不改变更新来源白名单、GitHub 发布解析、下载完整性校验或安装事务。
- 不把 SHA256 校验描述为可替代 TLS 或独立签名。

## 技术决策

### 1. 在 workspace 级启用 `system-proxy`

在根 `Cargo.toml` 的 reqwest features 中加入 `system-proxy`，使所有未显式调用 `no_proxy()` 的 reqwest client 获得一致的系统代理发现能力。环境变量优先于系统配置，`NO_PROXY`/`no_proxy` 继续控制绕过规则。

选择 workspace 级配置是因为 Cargo features 是累加的，局部依赖声明最终仍会统一启用该能力。显式保留 `default-features = false`，避免无意启用无关 reqwest 默认功能。

备选方案是在 `GithubHttp` 中解析环境变量并构造 `reqwest::Proxy`。该方案会重复 reqwest 的代理匹配、凭据脱敏和 `NO_PROXY` 语义，也无法自然复用 Windows 手动代理设置，因此不采用。

### 2. Windows 更新客户端使用系统根证书快照和 WebPKI

仅在 Windows 构建 `GithubHttp` 时，通过直接依赖 `rustls-native-certs` 加载 Windows 根证书，并转换为 reqwest certificates 后传给 `ClientBuilder::tls_certs_only(...)`。该路径继续使用 rustls 完成 WebPKI 校验，但不调用 Windows 平台 verifier，因此不会发起在线 CRL/OCSP 查询。

使用系统根证书而不是固定 `webpki-roots`，以兼容安装到 Windows 信任存储中的企业 TLS 拦截根证书。非 Windows 平台继续使用 reqwest 当前平台 verifier，避免扩大行为变化。

若根证书加载失败、没有获得可用根证书或证书转换失败，client 构造 SHALL 失败并返回明确错误；不得退回到无验证 TLS。根证书只在创建 `GithubHttp` 时加载一次，检查和下载各自新建 client 时会获得新的系统证书快照。

备选方案包括：

- `danger_accept_invalid_certs(true)`：同时关闭身份和链验证，风险不可接受。
- 遇到 TLS 错误后自动重试弱校验：无法可靠区分 CRL 不可达与中间人攻击，且会让安全策略依赖错误字符串，因此不采用。
- 固定 Mozilla roots：无法信任合法的企业拦截根证书。
- 自定义 Windows verifier 或维护 `rustls-platform-verifier` fork：可保留更多 Windows 信任语义，但实现和维护成本显著高于本次范围；上游尚无稳定的“仅关闭在线吊销”API。

### 3. 保持传输边界与失败语义

代理只改变连接路由，不改变原始目标 URL。`allowed_url`、HTTPS/443 限制和重定向策略继续依据目标 URL 校验，代理地址不加入 GitHub来源白名单。

显式代理配置无效、代理拒绝 CONNECT、代理认证失败、系统根加载失败和 TLS 校验失败继续归入可诊断的更新网络错误。错误消息不得输出代理密码或完整凭据。配置了代理但连接失败时，不自动回退直连，以免绕过企业网络策略。

Windows 系统代理发现限于 reqwest/hyper-util 当前支持的 `ProxyEnable`、`ProxyServer` 和 `ProxyOverride` 静态设置。PAC/WPAD 和集成认证不在能力承诺中；文档引导此类环境使用明确的 `HTTPS_PROXY`/`ALL_PROXY`，或报告为不支持。

### 4. 分层验证代理与 TLS 行为

纯逻辑测试覆盖 client 构造失败、根证书转换、URL 白名单和既有重定向策略。传输测试使用本地受控 HTTPS 目标与 CONNECT 代理，验证环境变量代理、绕过、代理失败和不受信证书拒绝；测试必须串行隔离进程环境，避免全局代理变量污染并行测试。

Windows 验证增加手动或 CI 可执行矩阵：静态系统代理、`HTTPS_PROXY`、企业根证书、外部 CRL 被阻断，以及无企业根证书时拒绝 TLS。PAC/WPAD 与 NTLM/Kerberos 标记为未承诺能力，而不是误报通过。

## 风险与权衡

- [系统根证书快照不保留 Windows 全部动态不信任和证书约束语义] → 仅限定于更新 GitHub client，继续执行 WebPKI 校验，并在文档中明确差异。
- [关闭在线吊销检查降低对已吊销终端证书的即时检测能力] → 保留严格 TLS 身份校验、GitHub 来源限制和现有下载校验；未来优先采用上游可配置的离线平台 verifier。
- [workspace feature 影响其他 reqwest client] → 依赖默认代理行为符合用户系统配置；本地服务 client 已显式 `no_proxy()`，回归检查其行为。
- [环境变量是进程启动时继承的] → 每次 client 构造重新读取当前进程环境；桌面从 Explorer 启动时优先依赖持久环境或 Windows 系统手动代理设置。
- [Windows 静态代理格式或认证方式超出 hyper-util 支持范围] → 提供环境变量替代路径，并在验证矩阵中覆盖项目实际使用的代理类型。
- [修改全局环境变量的测试相互干扰] → 将相关测试放入独立进程或使用串行锁与完整环境恢复。

## 迁移与回滚

1. 增加 reqwest `system-proxy` feature 和 Windows 目标下的直接证书加载依赖，更新 `Cargo.lock`。
2. 抽取并测试 `GithubHttp` client 构造，使 Windows 使用系统根证书快照，其他平台保持现状。
3. 运行 updater 定向测试和 native workspace 的 format、check、test、clippy gates。
4. 在 CRL 不可达的 Windows 内网代理环境执行更新检查和下载验证，再发布下一版本。
5. 若出现代理回归，可先回滚 `system-proxy` feature；若出现 Windows TLS 回归，可独立回滚 updater 的 `tls_certs_only` 路径。两部分不改变更新状态或磁盘格式。

## 待确认事项

- 项目实际内网代理是否为 Windows 静态手动代理、环境变量代理或 PAC/WPAD；后两者中的 PAC/WPAD 不由当前 `system-proxy` 实现覆盖。
- 企业代理是否进行 TLS 解密，以及企业根证书安装在 Current User、Local Machine 或两者的哪个 Windows 信任存储。
- 企业代理是否要求 Basic、NTLM 或 Kerberos 认证；只有 reqwest 当前支持且能由代理 URL 表达的认证方式纳入本次实现验证。
