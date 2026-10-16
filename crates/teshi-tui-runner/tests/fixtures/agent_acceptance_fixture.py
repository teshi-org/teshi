"""Local-only controlled ACP and OpenAI streams for product feature acceptance."""
import http.server
import json
import os
from pathlib import Path
import sys
import time
import uuid

root = Path(sys.argv[2])
root.mkdir(parents=True, exist_ok=True)


def trace(value):
    with (root / "trace.jsonl").open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(dict(value, pid=os.getpid()), ensure_ascii=False) + "\n")


def commands(turn):
    path = root / (turn + ".jsonl")
    offset = 0
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        if path.exists():
            lines = path.read_text(encoding="utf-8").splitlines()
            for line in lines[offset:]:
                if line:
                    yield json.loads(line)
            offset = len(lines)
        time.sleep(0.01)
    raise TimeoutError("fixture command deadline exceeded")


def send(value):
    print(json.dumps(value, ensure_ascii=False), flush=True)


if sys.argv[1] == "acp":
    name = sys.argv[3]
    controlled = len(sys.argv) > 4 and sys.argv[4] == "controlled"
    session = str(uuid.uuid4())
    model = "a"

    def behavior():
        path = root / (name + "-behavior.json")
        return json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}

    def settings():
        options = [{"value": "a", "name": "Alpha"}]
        if not behavior().get("removed"):
            options.append({"value": "b", "name": "Beta"})
        return [{"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": model, "options": options}]
    for line in sys.stdin:
        message = json.loads(line)
        method = message.get("method")
        result = {}
        if method == "initialize":
            result = {"protocolVersion": 1, "agentInfo": {"name": name}}
        elif method == "session/new":
            trace({"kind": "session", "session": session, "agent": name})
            result = {"sessionId": session}
            if (root / "model-settings").exists():
                result["configOptions"] = settings()
        elif method == "session/set_config_option":
            value = message["params"]["value"]
            rules = behavior()
            trace({"kind": "configure", "session": session, "agent": name, "value": value})
            if rules.get("timeout"):
                continue
            if rules.get("reject") == value:
                send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32602, "message": "setting rejected"}})
                continue
            model = value
            result = {"configOptions": settings()}
        elif method == "session/prompt":
            turn = str(uuid.uuid4())
            trace({"kind": "prompt", "turn": turn, "session": session, "agent": name, "model": model, "params": message["params"]})

            def update(value):
                send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": session, "update": value}})

            if not controlled:
                update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": behavior().get("reply", name + " response")}})
                result = {"stopReason": "end_turn"}
            else:
                for event in commands(turn):
                    kind = event["kind"]
                    if kind in ("reply", "reasoning"):
                        update({"sessionUpdate": "agent_message_chunk" if kind == "reply" else "agent_thought_chunk", "content": {"type": "text", "text": event["text"]}})
                    elif kind == "tool":
                        value = {"sessionUpdate": "tool_call" if "title" in event else "tool_call_update", "toolCallId": "read-project", "status": event["status"]}
                        if "title" in event:
                            value.update(title=event["title"], kind="read")
                        update(value)
                    elif kind in ("done", "error"):
                        if kind == "error":
                            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -1, "message": "controlled failure"}})
                        else:
                            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
                        break
                continue
        elif method == "session/cancel":
            continue
        if "id" in message:
            send({"jsonrpc": "2.0", "id": message["id"], "result": result})
else:
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            turn = str(uuid.uuid4())
            trace({"kind": "native", "turn": turn, "body": body})
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Connection", "close")
            self.end_headers()

            def chunk(delta, finish=None):
                payload = {"id": turn, "model": "acceptance", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
                self.wfile.write(("data: " + json.dumps(payload, ensure_ascii=False) + "\n\n").encode())
                self.wfile.flush()

            try:
                for event in commands(turn):
                    kind = event["kind"]
                    if kind in ("reply", "reasoning"):
                        chunk({"content" if kind == "reply" else "reasoning_content": event["text"]})
                    elif kind == "tool":
                        chunk({"tool_calls": [{"index": 0, "id": "native-inspect", "type": "function", "function": {"name": "get_feature_content", "arguments": json.dumps({"file_path": event["path"]})}}]})
                        chunk({}, "tool_calls")
                        self.wfile.write(b"data: [DONE]\n\n")
                        self.wfile.flush()
                        return
                    elif kind == "error":
                        self.wfile.write(b'data: {"error":{"message":"controlled failure"}}\n\n')
                        self.wfile.flush()
                        return
                    elif kind == "done":
                        chunk({}, "stop")
                        self.wfile.write(b"data: [DONE]\n\n")
                        self.wfile.flush()
                        return
            except (BrokenPipeError, ConnectionResetError):
                pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    (root / "native-port").write_text(str(server.server_port), encoding="utf-8")
    server.serve_forever()
