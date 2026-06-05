import asyncio
import json
import time
import base64
import httpx
import websockets

API = "http://127.0.0.1:1421/api/v1"
WS = "ws://127.0.0.1:1421/api/v1/events"

async def test_command(cmd: str, label: str):
    async with websockets.connect(WS) as ws:
        print(f"\n{'='*60}")
        print(f"TEST: {label}")
        print(f"Command: {repr(cmd)}")
        print('='*60)
        
        # Drain
        await asyncio.sleep(1)
        try:
            while True:
                msg = await asyncio.wait_for(ws.recv(), timeout=0.1)
        except asyncio.TimeoutError:
            pass

        # Spawn fresh terminal
        async with httpx.AsyncClient() as http:
            await http.post(f"{API}/terminal/stop")
            await asyncio.sleep(0.3)
            await http.post(f"{API}/terminal/spawn", json={"cols": 80, "rows": 24})
        await asyncio.sleep(2)

        # Drain startup
        try:
            while True:
                msg = await asyncio.wait_for(ws.recv(), timeout=0.1)
        except asyncio.TimeoutError:
            pass

        # Write command
        async with httpx.AsyncClient() as http:
            await http.post(f"{API}/terminal/write", json={"data": cmd + "\r\n"})

        # Collect
        raw_payloads = []
        start = time.time()
        while time.time() - start < 6:
            try:
                msg = await asyncio.wait_for(ws.recv(), timeout=0.5)
                data = json.loads(msg)
                if data["event"] == "terminal-output":
                    raw_payloads.append((time.time(), data["payload"]))
            except asyncio.TimeoutError:
                pass

        # Analyze
        all_text = [base64.b64decode(p).decode("utf-8", errors="replace") for _, p in raw_payloads]
        full_output = "".join(all_text)
        print(f"\nFull reconstructed output:\n{repr(full_output[:500])}")
        
        # Check consecutive duplicates
        dups = []
        for i in range(1, len(raw_payloads)):
            if raw_payloads[i][1] == raw_payloads[i-1][1]:
                dups.append(i)
                decoded = base64.b64decode(raw_payloads[i][1]).decode("utf-8", errors="replace")
                print(f"\n  *** DUPLICATE [{i}] == [{i-1}] ({len(decoded)} bytes): {repr(decoded)}")

        print(f"\nResult: {len(raw_payloads)} events, {len(dups)} consecutive duplicates")
        return len(dups) > 0

async def main():
    tests = [
        ("echo LINE_ONE & echo LINE_TWO & echo LINE_THREE", "Echo 3 unique lines"),
        ("dir", "Directory listing"),
    ]
    has_dups = False
    for cmd, label in tests:
        dup = await test_command(cmd, label)
        has_dups = has_dups or dup

    print(f"\n{'='*60}")
    if has_dups:
        print("CONCLUSION: Duplicate terminal-output events CONFIRMED")
    else:
        print("CONCLUSION: No duplicate terminal-output events detected")

if __name__ == "__main__":
    asyncio.run(main())
