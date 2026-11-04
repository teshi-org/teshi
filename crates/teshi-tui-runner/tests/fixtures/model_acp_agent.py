"""Deterministic ACP model choices, errors, and session traces for PTY acceptance."""
import json
import os
from pathlib import Path
import sys
import uuid

root = Path(sys.argv[2])
name = sys.argv[3]
session = str(uuid.uuid4())
model = "a"


def trace(value):
    with (root / "trace.jsonl").open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(dict(value, pid=os.getpid())) + "\n")


def send(value):
    print(json.dumps(value), flush=True)


def behavior():
    path = root / (name + "-behavior.json")
    return json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}


def settings():
    choices = [{"value": "a", "name": "Alpha"}]
    if not behavior().get("removed"):
        choices.append({"value": "b", "name": "Beta"})
    return [{"id": "model", "name": "Model", "category": "model", "type": "select",
             "currentValue": model, "options": choices}]


for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    result = {}
    if method == "initialize":
        result = {"protocolVersion": 1, "agentInfo": {"name": name}}
    elif method == "session/new":
        trace({"kind": "session", "session": session, "agent": name})
        result = {"sessionId": session, "configOptions": settings()}
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
        trace({"kind": "prompt", "session": session, "agent": name, "model": model, "params": message["params"]})
        send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": session, "update": {
            "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": name + " response"}}}})
        result = {"stopReason": "end_turn"}
    elif method == "session/cancel":
        continue
    if "id" in message:
        send({"jsonrpc": "2.0", "id": message["id"], "result": result})
