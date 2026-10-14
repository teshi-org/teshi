import json
import sys


turn = 0


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {"protocolVersion": 1, "agentInfo": {"name": "wrapped-chat-fixture"}},
        })
    elif method == "session/new":
        send({
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {"sessionId": "wrapped-chat-session"},
        })
    elif method == "session/prompt":
        turn += 1
        marker = f"WRAPPED_REPLY_{turn:02d}"
        send({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "wrapped-chat-session",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": marker[:9]},
                },
            },
        })
        send({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "wrapped-chat-session",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": marker[9:]},
                },
            },
        })
        send({
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {"stopReason": "end_turn"},
        })
