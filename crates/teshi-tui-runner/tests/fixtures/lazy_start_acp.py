"""Deterministic ACP lifecycle trace for passive chat and explicit startup."""

import json
import os
from pathlib import Path
import sys
import time


trace = Path(sys.argv[1])
mode = sys.argv[2]
model = "a"
turn = 0


def record(method):
    with trace.open("a", encoding="utf-8") as output:
        output.write(json.dumps({"method": method, "pid": os.getpid()}) + "\n")


def send(message):
    print(json.dumps(message), flush=True)


def options():
    return [{"id": "model", "name": "Model", "category": "model",
             "type": "select", "currentValue": model,
             "options": [{"value": "a", "name": "Alpha"},
                         {"value": "b", "name": "Beta"}]}]


record("spawn")
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    record(method)
    result = {}
    if method == "initialize":
        if mode == "fail":
            send({"jsonrpc": "2.0", "id": message["id"], "error": {
                "code": -1, "message": "lazy-start fixture rejected initialization"}})
            continue
        result = {"protocolVersion": 1, "agentInfo": {"name": "lazy-start"}}
    elif method == "session/new":
        time.sleep(1)
        result = {"sessionId": f"lazy-{os.getpid()}", "configOptions": options()}
    elif method == "session/set_config_option":
        model = message["params"]["value"]
        result = {"configOptions": options()}
    elif method == "session/prompt":
        turn += 1
        send({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": f"lazy-{os.getpid()}", "update": {
                "sessionUpdate": "agent_message_chunk", "content": {
                    "type": "text", "text": f"LAZY_REPLY_{turn}"}}}})
        result = {"stopReason": "end_turn"}
    if "id" in message:
        send({"jsonrpc": "2.0", "id": message["id"], "result": result})
