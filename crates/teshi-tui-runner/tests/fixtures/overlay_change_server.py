"""Loopback-only tool proposal for rendered Change Summary acceptance."""

import http.server
import json
from pathlib import Path
import sys

root = Path(sys.argv[1])
root.mkdir(parents=True, exist_ok=True)


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        with (root / "requests.jsonl").open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(body, ensure_ascii=False) + "\n")
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        has_result = any(message.get("role") == "tool" for message in body["messages"])
        if has_result:
            delta, finish = {"content": "OVERLAY_PROPOSAL_COMPLETE"}, "stop"
        else:
            delta = {"tool_calls": [{"index": index, "id": f"overlay-update-{name}", "type": "function", "function": {
                "name": "update_step", "arguments": json.dumps({"file_path": "location.feature",
                    "scenario_name": name, "step_index": 1, "new_text": f"PROPOSED_{name}_CHANGE"})}}
                for index, name in enumerate(("Alpha", "Beta"))]}
            finish = "tool_calls"
        payload = {"id": "overlay", "model": "acceptance", "choices": [{"index": 0,
            "delta": delta, "finish_reason": finish}]}
        self.wfile.write(("data: " + json.dumps(payload) + "\n\ndata: [DONE]\n\n").encode())
        self.wfile.flush()


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
(root / "port").write_text(str(server.server_port), encoding="utf-8")
server.serve_forever()
