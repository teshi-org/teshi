"""Local streaming provider for native queued-turn acceptance; no external service."""

import json
import sys
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

root = Path(sys.argv[1])
requests = 0


class Provider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        global requests
        requests += 1
        number = requests
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        with (root / "native-trace.jsonl").open("a", encoding="utf-8") as trace:
            trace.write(json.dumps(body) + "\n")
        first = ""
        if number == 1:
            first = 'data: {"model":"fixture","choices":[{"delta":{"reasoning_content":"NATIVE_REASONING streaming now"},"finish_reason":null}]}\n\n'
        final = "data: " + json.dumps({"model": "fixture", "choices": [{"delta": {
            "content": f"NATIVE_DONE_{number}"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 12, "completion_tokens": 8}}) + "\n\ndata: [DONE]\n\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len((first + final).encode())))
        self.send_header("Connection", "close")
        self.end_headers()
        if first:
            self.wfile.write(first.encode())
            self.wfile.flush()
            deadline = time.monotonic() + 20
            while not (root / "native.finish").exists() and time.monotonic() < deadline:
                time.sleep(0.02)
        self.wfile.write(final.encode())
        self.wfile.flush()


server = HTTPServer(("127.0.0.1", 0), Provider)
(root / "native-port").write_text(str(server.server_port), encoding="ascii")
server.serve_forever()
