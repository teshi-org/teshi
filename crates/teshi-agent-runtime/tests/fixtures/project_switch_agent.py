"""Local ACP and stalled HTTP transports for project teardown acceptance."""

import http.server
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import threading
import time


mode, root_name = sys.argv[1:]
root = Path(root_name)
evidence = root / "agent-evidence"
evidence.mkdir(parents=True, exist_ok=True)
lock = threading.Lock()


def trace(kind, **values):
    with lock, (evidence / "trace.jsonl").open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(dict(kind=kind, pid=os.getpid(), at=time.monotonic(), **values)) + "\n")


def send(value):
    print(json.dumps(value), flush=True)


def run_acp():
    trace("acp_start", cwd=os.getcwd())
    project = None
    stop = threading.Event()
    writer = None
    session = f"session-{os.getpid()}"

    def write_a():
        while not stop.wait(0.02):
            with (root / "project-a/agent-writes.txt").open("a", encoding="utf-8") as stream:
                stream.write("A\n")

    for line in sys.stdin:
        message = json.loads(line)
        method = message.get("method")
        result = {}
        if method == "initialize":
            result = {"protocolVersion": 1, "agentInfo": {"name": "Project fixture"}}
        elif method == "session/new":
            cwd = message["params"]["cwd"]
            project = Path(cwd).name
            # Never persist MCP descriptors or their authentication environment.
            trace("acp_session", cwd=cwd, project=project)
            result = {"sessionId": session}
        elif method == "session/prompt":
            text = "\n".join(item.get("text", "") for item in message["params"]["prompt"])
            trace("acp_prompt", project=project, text=text)
            reply = "A_AGENT_ACTIVE" if project == "project-a" else "B_AGENT_REPLY"
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": session, "update": {"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": reply}}}})
            if project == "project-a":
                if os.environ.get("TESHI_TEST_AGENT_CHILD") == "1":
                    subprocess.Popen([sys.executable, __file__, "child", str(root)])
                    deadline = time.monotonic() + 5
                    while not (evidence / "child-writes.txt").exists():
                        if time.monotonic() >= deadline:
                            raise RuntimeError("child did not publish readiness")
                        time.sleep(0.01)
                writer = threading.Thread(target=write_a)
                writer.start()
                continue
            result = {"stopReason": "end_turn"}
        elif method == "session/cancel":
            trace("acp_cancel", project=project)
            continue
        if "id" in message:
            send({"jsonrpc": "2.0", "id": message["id"], "result": result})
    if project == "project-a":
        trace("acp_stopping", project=project)
        # Delayed shutdown makes a synchronous frontend join observable. A late
        # update must not become a reply or tool result in the next project.
        time.sleep(1.25)
        try:
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": session, "update": {"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "A_LATE_REPLY"}}}})
        except (BrokenPipeError, OSError):
            pass
        trace("acp_late_attempt", project=project)
    stop.set()
    if writer:
        writer.join()
    trace("acp_exit", project=project)


def run_native():
    a_closed = threading.Event()

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            text = json.dumps(body.get("messages", []))
            project = "project-b" if "B_PROJECT_ONLY_REQUEST" in text else "project-a"
            if project == "project-b":
                closed = a_closed.wait(timeout=0.5)
                trace("native_prompt", project=project, text=text, a_stream_closed=closed)
            else:
                trace("native_prompt", project=project, text=text)
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Connection", "close")
            self.end_headers()
            marker = "A_AGENT_ACTIVE" if project == "project-a" else "B_AGENT_REPLY"
            payload = {"model": "project-fixture", "choices": [{"index": 0,
                       "delta": {"content": marker}, "finish_reason": None}]}
            self.wfile.write(("data: " + json.dumps(payload) + "\n\n").encode())
            self.wfile.flush()
            if project == "project-b":
                self.wfile.write(b'data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n')
                self.wfile.flush()
                return
            # Send no more chunks. Cancellation must interrupt the client's
            # stalled read rather than waiting for its normal chunk deadline.
            self.connection.settimeout(10)
            trace("native_a_waiting")
            try:
                incoming = self.connection.recv(1)
                trace("native_a_read", size=len(incoming))
                if incoming == b"":
                    trace("native_a_closed")
                    a_closed.set()
            except (ConnectionResetError, ConnectionAbortedError):
                trace("native_a_closed")
                a_closed.set()
            except socket.timeout:
                trace("native_a_timeout")

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    (evidence / "native-port").write_text(str(server.server_port), encoding="utf-8")
    server.serve_forever()


if mode == "acp":
    run_acp()
elif mode == "native":
    run_native()
elif mode == "child":
    trace("acp_child_start", cwd=os.getcwd())
    deadline = time.monotonic() + 30
    while not (evidence / "cleanup-child").exists() and time.monotonic() < deadline:
        with (evidence / "child-writes.txt").open("a", encoding="utf-8") as stream:
            stream.write("child\n")
        time.sleep(0.02)
else:
    raise ValueError(mode)
