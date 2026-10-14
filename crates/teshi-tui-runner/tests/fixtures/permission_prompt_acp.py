import json
import sys
import threading


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def chunk(text):
    send({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": "permission-session",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": text},
            },
        },
    })


for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {"protocolVersion": 1, "agentInfo": {"name": "permission-fixture"}},
        })
    elif method == "session/new":
        send({
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {"sessionId": "permission-session"},
        })
    elif method == "session/prompt":
        chunk("PERMISSION_BEFORE")
        send({
            "jsonrpc": "2.0",
            "id": 91,
            "method": "session/request_permission",
            "params": {
                "sessionId": "permission-session",
                "options": [
                    {"optionId": "allow_once", "name": "Allow once", "kind": "allow_once"},
                    {"optionId": "allow_always", "name": "Allow always", "kind": "allow_always"},
                    {"optionId": "reject", "name": "Reject", "kind": "reject_once"},
                ],
            },
        })
        # Wait for Teshi's decision; allowed turns continue, rejected turns stop.
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
        reader.join(timeout=10.0)
        response = response_container["value"]
        outcome = (response or {}).get("result", {}).get("outcome", {})
        if response is None or response.get("error") or outcome.get("outcome") != "selected":
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {"stopReason": "refusal"},
            })
        else:
            chunk(f"PERMISSION_AFTER_{outcome.get('optionId')}")
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {"stopReason": "end_turn"},
            })
