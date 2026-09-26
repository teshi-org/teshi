Discovery 未改；生产 Chrome 仍走 Python。

修改：Rust Broker execute_locator DTO 支持 CSS/test_id/role/name/@e；执行前校验 Profile/target/project/caller/Lease/snapshot/revision/context，复用 click/pointer_click，不重试。

验证：Broker 57/57、Node 19/19、CLI build；真实双 Chrome 1/1、exit 0（临时日志）。两 Profile 四类定位副作用各1、pointerdown各1、Playwright mutations=0；过期/跨 Profile/错误 Lease/歧义(2)拒绝。

未完成：生产切换、智能排序、Fill/断言、截图/Network/Discovery、全 workspace。下一步：OpenSpec 4.1–4.5 ranking/verification/Feature replay。
