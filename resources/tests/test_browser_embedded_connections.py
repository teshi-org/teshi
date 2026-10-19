"""Real sidecar regression for short-lived CLI WebSocket connections."""

import asyncio
import json
from pathlib import Path
import socket
import subprocess
import sys
import unittest

import psutil
import websockets


class EmbeddedConnectionTests(unittest.IsolatedAsyncioTestCase):
    async def test_abrupt_disconnects_do_not_block_unread_stderr(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            cdp_port = listener.getsockname()[1]
        script = Path(__file__).resolve().parents[1] / "browser_service.py"
        process = subprocess.Popen(
            [sys.executable, str(script), "--port", "0", "--cdp-port",
             str(cdp_port), "--no-preview-stream"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        try:
            port = int(await asyncio.wait_for(
                asyncio.to_thread(process.stdout.readline), timeout=20
            ))
            # Match the engine's unread stderr pipe. Repeated peer disconnects
            # must not produce tracebacks that eventually block the event loop.
            for index in range(48):
                connection = await websockets.connect(
                    f"ws://127.0.0.1:{port}", open_timeout=3
                )
                try:
                    await connection.send(json.dumps({
                        "cmd": "get_page_snapshot", "request_id": str(index)
                    }))
                    reply = json.loads(await asyncio.wait_for(connection.recv(), 3))
                    self.assertEqual(reply["request_id"], str(index))
                finally:
                    connection.transport.abort()
                await asyncio.sleep(0.01)
        finally:
            children = psutil.Process(process.pid).children(recursive=True)
            process.kill()
            for child in reversed(children):
                try:
                    child.kill()
                except psutil.NoSuchProcess:
                    pass
            await asyncio.to_thread(process.wait, 10)
            process.stdout.close()
            process.stderr.close()


if __name__ == "__main__":
    unittest.main()
