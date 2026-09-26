根因：Profile 可能只有 Worker URL，无新 Playwright `serviceworker` 事件/可评估句柄；原测试误判，Broker 已恢复。Rust 握手缺 `stream_hello_ack`，扩展未记 generation；重启后旧 token 401/403 未重发现。

文件：broker state/session、extension background+两测试、two-profile 测试、OpenSpec tasks、本交接。测试以 Broker ready+generation 判定并保留观测缺口；扩展认证失败重发现。

验证：Node 32/32、Rust 50/50、差分 1/1、CLI/fmt/py_compile 通过；真实 1/1：A 1→3、B 隔离，重启 A/B=1/2，旧 token 拒绝；进程/端口清理正常。

剩余/入口：Worker 观测缺口仍是 Playwright 边界；未跑全量，未改 Discovery/P0/生产路径。下一入口 4.1，生产 Chrome 仍用 Python Broker。
