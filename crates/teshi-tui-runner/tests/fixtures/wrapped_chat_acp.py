import json
import sys


session_seq = 0
session_turns = {}


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
        session_seq += 1
        sid = f"wrapped-chat-session-{session_seq}"
        session_turns[sid] = 0
        send({
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {"sessionId": sid},
        })
    elif method == "session/prompt":
        sid = message.get("params", {}).get("sessionId", "wrapped-chat-session")
        turn = session_turns.get(sid, 0) + 1
        session_turns[sid] = turn
        marker = f"WRAPPED_REPLY_{turn:02d}"
        send({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": sid,
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
                "sessionId": sid,
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
