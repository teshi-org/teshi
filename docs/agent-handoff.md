基线 8fcaddb；生产 Chrome 仍走 Python。

本轮完成 4.1：Rust broker 增加 LocatorSnapshot/SnapshotElement normalization、LocatorIntent score、候选生成/排序、typed verification status/result 合并；session 用同一模型生成作用域 @e，state 继续复用 execute_locator，未新增通道、重试或 DOM 副作用。

验证：Rust broker 60/60、Python locator 8/8、真实双 Profile transport 1/1；clippy -D warnings、fmt、git diff --check、openspec strict validate 均通过。日志均为临时文件；pytest 仅有既存 .pytest_cache 权限 warning。

未完成：4.2–4.5、截图/Console/Network/授权、生产切换与最终验收；不要改用生产 Rust。
