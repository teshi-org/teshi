"""Measure broker-to-extension command delivery overhead for P0 transport choice."""

from __future__ import annotations

import asyncio
import statistics
import sys
import time
from pathlib import Path

RESOURCES = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RESOURCES))

from browser_agent_broker import BrowserSessionBroker  # noqa: E402
from test_browser_agent_broker import heartbeat, target  # noqa: E402

HEARTBEAT_SECONDS = 1.5
SAMPLES = 8


async def measure_heartbeat() -> list[float]:
    broker = BrowserSessionBroker()
    record = broker.register_heartbeat(heartbeat("benchmark"))
    latencies: list[float] = []
    for index in range(SAMPLES):
        future = asyncio.get_running_loop().create_future()
        request_id = f"heartbeat-{index}"
        broker.queue_command(
            record,
            target("benchmark"),
            {"cmd": "get_page_snapshot", "request_id": request_id},
            future,
        )
        started = time.perf_counter()
        # Alternate offsets across one real 1.5 s extension polling interval.
        offset = HEARTBEAT_SECONDS * ((index + 1) / (SAMPLES + 1))
        await asyncio.sleep(HEARTBEAT_SECONDS - offset)
        command = broker.heartbeat_response(record)["cmd"]
        assert command and command["request_id"] == request_id
        latencies.append((time.perf_counter() - started) * 1000)
        broker.cancel_request(request_id, RuntimeErrorAdapter())
    return latencies


async def measure_direct() -> list[float]:
    broker = BrowserSessionBroker()
    record = broker.register_heartbeat(heartbeat("benchmark"))
    latencies: list[float] = []
    for index in range(SAMPLES):
        future = asyncio.get_running_loop().create_future()
        request_id = f"direct-{index}"
        broker.queue_command(
            record,
            target("benchmark"),
            {"cmd": "get_page_snapshot", "request_id": request_id},
            future,
        )
        started = time.perf_counter()
        command = broker.take_queued_command("benchmark", request_id)
        assert command and command["request_id"] == request_id
        await asyncio.sleep(0)  # one event-loop turn, matching a ready WebSocket send
        latencies.append((time.perf_counter() - started) * 1000)
        broker.cancel_request(request_id, RuntimeErrorAdapter())
    return latencies


class RuntimeErrorAdapter:
    """Minimal response adapter accepted by cancel_request for benchmark cleanup."""

    def response(self, request_id: str, operation: str) -> dict[str, object]:
        return {"ok": False, "request_id": request_id, "operation": operation}


def summarize(values: list[float]) -> dict[str, float]:
    ordered = sorted(values)
    p95_index = min(len(ordered) - 1, round(0.95 * (len(ordered) - 1)))
    return {
        "median_ms": statistics.median(values),
        "p95_ms": ordered[p95_index],
        "min_ms": ordered[0],
        "max_ms": ordered[-1],
    }


async def main() -> None:
    heartbeat_values = await measure_heartbeat()
    direct_values = await measure_direct()
    print({"samples": SAMPLES, "heartbeat": summarize(heartbeat_values), "direct": summarize(direct_values)})


if __name__ == "__main__":
    asyncio.run(main())
