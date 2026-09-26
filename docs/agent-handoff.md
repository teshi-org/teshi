基于78b50ada；Discovery未改，生产Chrome仍走Python。

修改：internal_broker.rs 增加专用 --enable-p0-control；state.rs 校验Navigation/Snapshot的Profile p0.control，修复顶层lease_token renew/release；test_browser_two_profile_rust_transport.py 增加双Profile真实导航+Snapshot。

验证：fmt；broker 51/51；CLI 2/2；Node 32/32。A/B generation=1/2，导航与Snapshot的URL、title、target、request_id、snapshot_id隔离且成功；单测覆盖断线/超时/取消/迟到响应；日志在artifact。

剩余：harness清理bug退出1，修复后未重复页面副作用请求；未跑全量。下一入口：execute_browser_action→Extension executeLocator 的Click/pointer_click。
