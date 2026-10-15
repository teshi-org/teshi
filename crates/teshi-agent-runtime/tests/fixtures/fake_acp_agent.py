import json
import sys
import os
import threading
from pathlib import Path

mode = sys.argv[1]
model = "a"
fast = False


def config_options():
    return [
        {"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": model,
         "options": [{"value": "a", "name": "Alpha"}, {"value": "b", "name": "Beta"}, {"value": "reject", "name": "Rejected"}]},
        {"id": "effort", "name": "Thinking", "category": "thought_level", "type": "select", "currentValue": "high" if model == "a" else "low",
         "options": [{"value": "high" if model == "a" else "low", "name": "High" if model == "a" else "Low"}]},
        {"id": "fast", "name": "Fast", "type": "boolean", "currentValue": fast},
    ]

if len(sys.argv) > 2:
    marker = Path(sys.argv[2])
    marker.write_text(str(os.getpid()), encoding="utf-8")


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        if mode == "initialize_failed":
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -1, "message": "init rejected"}})
        else:
            send({"jsonrpc": "2.0", "id": message["id"], "result": {
                "protocolVersion": 1, "agentInfo": {"name": "fake-agent"}}})
    elif method == "session/new":
        if mode == "session_failed":
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -1, "message": "session rejected"}})
        else:
            result = {"sessionId": "fake-session"}
            if mode.startswith("settings"):
                result["configOptions"] = config_options()
            send({"jsonrpc": "2.0", "id": message["id"], "result": result})
            if mode == "settings_idle":
                model = "b"
                send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "fake-session", "update": {"sessionUpdate": "config_option_update", "configOptions": config_options()}}})
    elif method == "session/set_config_option":
        params = message["params"]
        assert params["sessionId"] == "fake-session"
        if params["value"] == "reject":
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32602, "message": "setting rejected"}})
            continue
        if mode == "settings_timeout":
            continue
        if params["configId"] == "model":
            assert isinstance(params["value"], str)
            model = params["value"]
        elif params["configId"] == "fast":
            assert params["type"] == "boolean" and isinstance(params["value"], bool)
            fast = params["value"]
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"configOptions": config_options()}})
    elif method == "session/prompt":
        if mode == "crash":
            sys.exit(13)
        if mode == "prompt_failed":
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -1, "message": "prompt rejected"}})
        elif mode == "permission_then_exit":
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": "fake-session", "update": {
                    "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hello"}}}})
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": "fake-session", "update": {
                    "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": " world"}}}})
            send({"jsonrpc": "2.0", "id": 91, "method": "session/request_permission", "params": {
                "sessionId": "fake-session",
                "options": [{"optionId": "allow_once", "name": "Allow once", "kind": "allow_once"}]}})
            # Wait for Teshi's permission response. If it errors or cancels, mimic an agent that gives up.
            response_container = {"value": None}
            def read_response():
                for line in sys.stdin:
                    try:
                        response = json.loads(line)
                    except Exception:
                        continue
                    if response.get("id") == 91:
                        response_container["value"] = response
                        break
            reader = threading.Thread(target=read_response, daemon=True)
            reader.start()
            reader.join(timeout=2.0)
            response = response_container["value"]
            if response is None or response.get("error") or response.get("result", {}).get("outcome", {}).get("outcome") == "cancelled":
                sys.exit(7)
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
        else:
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": "fake-session", "update": {
                    "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hello"}}}})
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": "fake-session", "update": {
                    "sessionUpdate": "tool_call", "title": "reading project", "status": "in_progress"}}})
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
