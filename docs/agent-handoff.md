基线 3af242e；未 push，生产 Chrome 走 Python。

4.2 完成并勾选。改动 protocol/session/state、background.js、fixture及三套测试。@e candidate 携带 frame/Shadow context，复用 execute_locator；排序、同源 iframe、Shadow host、唯一匹配和跨域错误已验证。

验证：broker 69/69、Python 9/9、Node 38/38；双 Profile 测 iframe/Shadow/CSS、缺失/多匹配、过期 @e、revision 变化；fmt、clippy、strict validate 通过。

未实现：跨域/嵌套 iframe、closed Shadow、iframe pointer_click 坐标、P0 fill/assert。4.3 入口：crates/teshi-browser-broker/src/coordinator.rs prepare/finalize_response、state.rs dispatch。
