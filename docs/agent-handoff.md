Discovery 未改；生产 Chrome 仍走 Python。

修改：Rust Broker 仅将明确 CSS 的 click/pointer_click 映射为 execute_locator；执行前校验 p0、Profile、Lease、revision；回包严格关联 request/operation/Profile/target/generation。保留 DOM/CDP pointer 语义；双 Profile 测试加入计数器、pointerdown、失败边界。

验证：Node 37/37；Broker 55/55；Python 编译通过。真实双 Chrome 1/1、退出码 0（日志 final6）：A click、B pointer_click 各一次，无串扰，B pointerdown=1；Playwright 仅观察；四类负例正确拒绝，无重试。

未完成：生产 Python、智能定位、截图/Network/Discovery、全 workspace。下一步继续 execute_locator DTO，保持 P0 fail-closed。
