#!/usr/bin/env python3
"""Terminal重复输出回归测试。

测试场景：逐字符输入命令时，PSReadLine 行内重绘不应累积为多行重复。
使用步骤：
  1. 通过 WebSocket 监听 terminal-output events
  2. 逐字符输入 echo DUP_MARKER_<TIMESTAMP>
  3. 收集输出
  4. 计数 marker 出现次数
  5. 如果出现 >2 次则判定为重复（bug）

Usage:
    python tests/regression_terminal_dup.py          # 默认 http://127.0.0.1:20253
    python tests/regression_terminal_dup.py --api http://127.0.0.1:20253
"""

import argparse
import asyncio
import json
import base64
import time
import sys
from datetime import datetime

import httpx
import websockets


async def drain(ws, timeout=0.3):
    try:
        while True:
            await asyncio.wait_for(ws.recv(), timeout=timeout)
    except (asyncio.TimeoutError, websockets.ConnectionClosed):
        pass


async def run(api_base: str) -> bool:
    marker = f"DUP_{datetime.now().strftime('%H%M%S%f')}"
    cmd = f"echo {marker}"

    print(f"[setup] API: {api_base}")
    print(f"[setup] marker: {marker}")
    print(f"[setup] command: {cmd!r}")
    print()

    # --- stop any existing terminal ---
    async with httpx.AsyncClient() as http:
        try:
            await http.post(f"{api_base}/api/v1/terminal/stop")
        except Exception:
            pass
        await asyncio.sleep(0.3)

    ws_url = api_base.replace("http://", "ws://").replace("https://", "wss://")

    async with websockets.connect(f"{ws_url}/api/v1/events") as ws:
        await drain(ws)

        # --- spawn fresh terminal ---
        async with httpx.AsyncClient() as http:
            r = await http.post(
                f"{api_base}/api/v1/terminal/spawn",
                json={"cols": 120, "rows": 40},
            )
            if r.status_code not in (200, 204):
                print(f"[FAIL] spawn returned {r.status_code}")
                return False
        print("[ ok ] terminal spawned")

        await asyncio.sleep(2)
        await drain(ws)

        # --- type command character by character (simulates Playwright type) ---
        async with httpx.AsyncClient() as http:
            for ch in cmd + "\r\n":
                await http.post(
                    f"{api_base}/api/v1/terminal/write",
                    json={"data": ch},
                )
                await asyncio.sleep(0.01)

        print(f"[ ok ] typed character by character: {cmd!r}")

        # --- collect output ---
        raw_payloads: list[str] = []
        deadline = time.time() + 8
        while time.time() < deadline:
            try:
                msg = await asyncio.wait_for(ws.recv(), timeout=0.5)
                data = json.loads(msg)
                if data.get("event") == "terminal-output":
                    raw_payloads.append(data["payload"])
            except asyncio.TimeoutError:
                pass

        print(f"[info] {len(raw_payloads)} terminal-output events")

        # --- reconstruct full text ---
        chunks = []
        for p in raw_payloads:
            try:
                chunks.append(base64.b64decode(p).decode("utf-8", errors="replace"))
            except Exception:
                chunks.append("")

        full = "".join(chunks)

        # --- count marker occurrences ---
        count = full.count(marker)
        expected_max = 2  # echo line + output line

        print()
        print(f"[result] {marker!r} appears {count} times")
        print(f"[result] expected max: {expected_max}")

        if count <= expected_max:
            print(f"[PASS] No duplication ({count} <= {expected_max})")
            return True
        else:
            print(f"[FAIL] Duplication detected ({count} > {expected_max})")
            # show context
            idx = 0
            for i in range(count):
                pos = full.find(marker, idx)
                if pos < 0:
                    break
                ctx = full[max(0, pos - 25):pos + len(marker) + 25]
                print(f"       #{i + 1}: ...{ctx!r}...")
                idx = pos + 1
            return False


def main():
    parser = argparse.ArgumentParser(
        description="Regression test: terminal character-by-character duplication"
    )
    parser.add_argument(
        "--api",
        default="http://127.0.0.1:20253",
        help="teshi web base URL (default: http://127.0.0.1:20253)",
    )
    args = parser.parse_args()

    ok = asyncio.run(run(args.api.rstrip("/")))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
