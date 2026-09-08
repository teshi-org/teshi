## 1. 依赖与代理能力

- [x] 1.1 在 workspace reqwest features 中启用 `system-proxy`，保持 `default-features = false`，并更新 `Cargo.lock`
- [x] 1.2 为 `teshi-update` 增加 Windows 目标下的 `rustls-native-certs` 直接依赖，确认版本与 reqwest/rustls 依赖图兼容
- [x] 1.3 使用 Cargo feature tree 验证 `system-proxy` 已启用，且本地 browser bridge 的显式 `no_proxy()` 不受影响

## 2. Windows 更新 TLS 客户端

- [x] 2.1 抽取 `GithubHttp` client 构造逻辑，为 Windows 实现系统根证书加载、reqwest certificate 转换和空根证书集合检查
- [x] 2.2 在 Windows client builder 上应用 `tls_certs_only(...)`，在非 Windows 平台保留现有 platform verifier
- [x] 2.3 为根证书读取或转换失败增加可诊断的默认拒绝错误，确保不存在 `danger_accept_invalid_certs` 或 TLS 失败后的弱校验重试
- [x] 2.4 确认代理路由不改变 `allowed_url`、HTTPS/443、重定向白名单、超时和下载完整性校验

## 3. 自动化测试

- [x] 3.1 为证书加载成功、部分无效证书、无可用根证书和加载失败增加确定性单元测试
- [x] 3.2 使用隔离进程或串行环境保护增加 `HTTPS_PROXY`、`ALL_PROXY`、`NO_PROXY` 和环境变量优先级测试
- [x] 3.3 使用本地 HTTPS 与 CONNECT proxy fixture 验证代理请求、代理失败、不受信证书、域名不匹配和非允许重定向
- [x] 3.4 增加或保留 native desktop 本地 browser bridge 的 `no_proxy()` 回归覆盖
- [x] 3.5 验证代理返回损坏或截断载荷时，现有大小和 SHA256 检查仍在安装前拒绝载荷

## 4. Windows 内网验收与文档

- [x] 4.1 在 CRL/OCSP 不可达的 Windows 环境验证 `teshi update --check` 可通过 `HTTPS_PROXY` 访问 GitHub
- [x] 4.2 验证 Windows 静态系统代理和 `ProxyOverride`，并记录 PAC/WPAD、分协议复杂代理值和代理认证的实测边界
- [x] 4.3 在启用 TLS 解密的企业代理下分别验证已安装企业根证书时成功、未安装时严格拒绝
- [x] 4.4 更新用户文档，说明支持的代理配置、环境变量继承、Windows 在线吊销例外和禁止关闭全部 TLS 验证

## 5. 质量门禁

- [x] 5.1 运行 updater/helper 定向 check、test 和 clippy，修复所有新增警告
- [x] 5.2 运行 `cargo fmt --all --check` 以及 native workspace 的 check、test、clippy gates，并排除 wasm-only `teshi-web`
- [x] 5.3 对照 `proxied-self-update-transport` 的每个场景核验实现，并运行严格 OpenSpec validation
