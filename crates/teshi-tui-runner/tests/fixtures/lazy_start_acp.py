"""Deterministic ACP lifecycle trace for passive chat and explicit startup."""

import json
import os
from pathlib import Path
import sys
import time


trace = Path(sys.argv[1])
mode = sys.argv[2]
session_seq = 0
session_models = {}
session_turns = {}


def record(method):
    with trace.open("a", encoding="utf-8") as output:
        output.write(json.dumps({"method": method, "pid": os.getpid()}) + "\n")


def send(message):
    print(json.dumps(message), flush=True)


def options(model):
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
        session_seq += 1
        sid = f"lazy-{os.getpid()}-{session_seq}"
        session_models[sid] = "a"
        session_turns[sid] = 0
        result = {"sessionId": sid, "configOptions": options(session_models[sid])}
    elif method == "session/set_config_option":
        params = message["params"]
        sid = params.get("sessionId")
        value = params["value"]
        if sid is not None:
            session_models[sid] = value
        result = {"configOptions": options(session_models.get(sid, value))}
    elif method == "session/prompt":
        sid = message.get("params", {}).get("sessionId", f"lazy-{os.getpid()}")
        turn = session_turns.get(sid, 0) + 1
        session_turns[sid] = turn
        send({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": sid, "update": {
                "sessionUpdate": "agent_message_chunk", "content": {
                    "type": "text", "text": f"LAZY_REPLY_{turn}"}}}})
        result = {"stopReason": "end_turn"}
    if "id" in message:
        send({"jsonrpc": "2.0", "id": message["id"], "result": result})
