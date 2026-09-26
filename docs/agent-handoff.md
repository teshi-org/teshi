Discovery未改；Chrome仍走Python。

修改：background.js为stream WebSocket加入epoch；旧回调、direct command、重连定时器不能修改当前连接、generation、队列、backoff。network-capture.test.mjs加入Fake WebSocket/定时器回归。

验证：node --test extension/teshi-bridge/tests/protocol.test.mjs extension/teshi-bridge/tests/network-capture.test.mjs；36/36通过，0失败，退出码0。

未执行：未跑resources/tests/test_browser_two_profile_rust_transport.py；已有A/B导航+Snapshot证据，本轮无新增结果。真实Chrome E2E、全量workspace回归未验证。
下一步：execute_browser_action→executeLocator Click/pointer_click。
