import json
import sys
import os
import threading
from pathlib import Path

mode = sys.argv[1]
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
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"sessionId": "fake-session"}})
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
