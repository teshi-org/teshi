"""Controlled API transport for a real in-process dispatch lifetime test."""

import base64
import hashlib
import http.server
import json
from pathlib import Path
import sys
import struct
import time

def main():
    root = Path(sys.argv[1])
    project = root / "project-a"
    endpoint = project / ".teshi/api-endpoint.json"
    endpoint.parent.mkdir(parents=True, exist_ok=True)

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_):
            pass

        def do_GET(self):
            key = self.headers["Sec-WebSocket-Key"]
            accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
            self.send_response(101)
            self.send_header("Upgrade", "websocket")
            self.send_header("Connection", "Upgrade")
            self.send_header("Sec-WebSocket-Accept", accept)
            self.end_headers()
            try:
                while True:
                    header = self.rfile.read(2)
                    if len(header) != 2 or header[0] & 0x0F == 8:
                        break
                    size = header[1] & 0x7F
                    if size == 126:
                        size = struct.unpack("!H", self.rfile.read(2))[0]
                    elif size == 127:
                        size = struct.unpack("!Q", self.rfile.read(8))[0]
                    assert size < 65536, "fixture request exceeds its limit"
                    mask = self.rfile.read(4) if header[1] & 0x80 else b""
                    payload = self.rfile.read(size)
                    if mask:
                        payload = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
                    request = json.loads(payload)
                    if request.get("cmd") == "execute_step":
                        (root / "dispatch-active").write_text("active", encoding="utf-8")
                        deadline = time.monotonic() + 30
                        while not (root / "dispatch-release").exists() and time.monotonic() < deadline:
                            time.sleep(.01)
                        with (project / "dispatch-writes.txt").open("a", encoding="utf-8") as stream:
                            stream.write("A-only\n")
                        (root / "dispatch-finished").write_text("finished", encoding="utf-8")
                    response = json.dumps(dict(type="response", ok=True, request_id=request.get("request_id"))).encode()
                    prefix = bytes([0x81, len(response)]) if len(response) < 126 else b"\x81\x7e" + struct.pack("!H", len(response))
                    self.wfile.write(prefix + response)
                    self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                pass
            self.close_connection = True

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    endpoint.write_text(json.dumps(dict(ws_url=f"ws://127.0.0.1:{server.server_port}", project_root=str(project))), encoding="utf-8")
    server.serve_forever()


if __name__ == "__main__":
    main()
