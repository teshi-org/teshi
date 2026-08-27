# P0 command transport benchmark

Date: 2026-08-11
Host: Windows development workspace
Command: `python resources/tests/benchmark_browser_command_transport.py`

The benchmark measures broker-to-extension delivery overhead with the extension's configured 1,500 ms heartbeat cadence and with an already-negotiated direct WebSocket. It uses eight samples per transport. It excludes browser action execution time, so the result isolates the transport choice.

| Transport | Median | p95 | Observed range |
|---|---:|---:|---:|
| Heartbeat queue | 750.99 ms | 1,348.21 ms | 171.23–1,348.21 ms |
| Direct WebSocket event-loop delivery | 0.004 ms | 0.019 ms | 0.004–0.019 ms |

Decision: implement the negotiated direct command channel for P0. The existing authenticated extension frame WebSocket now carries `direct_command` messages and correlated responses. Heartbeats remain the liveness mechanism and bounded fallback. Before direct delivery, the broker atomically removes the request from the heartbeat queue; if the WebSocket send fails, it restores that same request exactly once. This preserves at-most-once mutation dispatch across transports.

Limitations: the direct figure is event-loop delivery overhead on a ready local WebSocket path and does not include extension service-worker wake-up or CDP action time. Real Chromium acceptance in task 4.8 remains the end-to-end latency gate.

## CLI bootstrap smoke

After building `teshi-cli`, the first `teshi browser sessions` invocation started a broker while port 17373 was initially closed. Discovery reported protocol 1, `broker_scope=user_session`, and `command_transport=direct-ws+heartbeat-fallback`. A second invocation returned an empty successful session list because no extension Profile was connected, and reused the same broker PID and start identity. The project compatibility endpoint contained the same PID, start identity, protocol, discovery URL, command WebSocket, and extension frame WebSocket.
