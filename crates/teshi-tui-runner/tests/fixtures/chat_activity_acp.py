"""Deterministic chat UX fixture; gate files expose real PTY intermediate states."""

import json
import os
import sys
import threading
import time
from pathlib import Path

gate = os.environ.get("TESHI_CHAT_ACTIVITY_GATE")
trace = os.environ.get("TESHI_CHAT_ACTIVITY_TRACE")
lock = threading.Lock()
cancelled = threading.Event()
count = len(Path(trace).read_text(encoding="utf-8").splitlines()) if trace and Path(trace).exists() else 0


def send(value):
    with lock:
        print(json.dumps(value, ensure_ascii=False), flush=True)


def update(value):
    send({"jsonrpc": "2.0", "method": "session/update", "params": {
        "sessionId": "activity", "update": value}})


def wait_stage(name, token, delay=0.7):
    deadline = time.monotonic() + (20 if gate else delay)
    while time.monotonic() < deadline:
        if token.is_set():
            return False
        if gate and Path(gate + "." + name).exists():
            return True
        time.sleep(0.02)
    return not token.is_set()


def reply(request, number, token):
    text = "\n".join(item.get("text", "") for item in request["params"]["prompt"])
    if trace:
        with open(trace, "a", encoding="utf-8") as output:
            output.write(json.dumps({"number": number, "text": text}) + "\n")
    current_turn = text.rsplit("[Current turn]", 1)[-1]
    if "FAIL_TURN" in current_turn:
        if wait_stage("failure", token):
            send({"jsonrpc": "2.0", "id": request["id"], "error": {
                "code": -32603, "message": "ACTIVITY_FIXTURE_FAILURE"}})
        return
    if number == 1:
        if not wait_stage("thinking", token):
            return
        for part in ["THINKING_LINE inspect the chat; ", "中文推理 remains on one row. ",
                     "Compare the input and saved history. ", "Keep the output responsive. "]:
            if token.is_set():
                return
            update({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": part}})
            time.sleep(0.22)
        if not wait_stage("tools", token):
            return
        update({"sessionUpdate": "tool_call", "toolCallId": "read-chat", "title": "Inspect chat source",
                "kind": "read", "rawInput": {"path": "src/chat_fixture.rs"},
                "locations": [{"path": "src/chat_fixture.rs"}], "status": "in_progress"})
        time.sleep(0.1)
        update({"sessionUpdate": "tool_call_update", "toolCallId": "read-chat", "status": "completed"})
        update({"sessionUpdate": "tool_call", "toolCallId": "run-tests", "title": "Check fixture",
                "kind": "execute", "rawInput": {"command": "cargo test chat_fixture"}, "status": "in_progress"})
        time.sleep(0.1)
        update({"sessionUpdate": "tool_call_update", "toolCallId": "run-tests", "status": "failed"})
        if not wait_stage("finish", token):
            return
    update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": f"ACTIVITY_DONE_{number}"}})
    send({"jsonrpc": "2.0", "id": request["id"], "result": {"stopReason": "end_turn"}})


for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": request["id"], "result": {
            "protocolVersion": 1, "agentInfo": {"name": "chat-activity-fixture"}}})
    elif method == "session/new":
        send({"jsonrpc": "2.0", "id": request["id"], "result": {"sessionId": "activity"}})
    elif method == "session/prompt":
        count += 1
        cancelled = threading.Event()
        threading.Thread(target=reply, args=(request, count, cancelled), daemon=True).start()
    elif method == "session/cancel":
        cancelled.set()
