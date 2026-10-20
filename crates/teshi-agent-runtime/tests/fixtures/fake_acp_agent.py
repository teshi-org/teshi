import json
import sys
import os
import threading
import time
import socket
import re
from pathlib import Path

mode = sys.argv[1]
model = "a"
fast = False
prompt_count = 0


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
        if mode.startswith("mcp_"):
            mcp_server = message["params"]["mcpServers"][0]
            assert mcp_server["name"] == "teshi-authoring"
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
        prompt_count += 1
        if mode == "refusal_once":
            assert message["params"]["sessionId"] == "fake-session"
            if prompt_count == 1:
                reason = "refusal"
            else:
                assert "do not repeat completed tool actions" in message["params"]["prompt"][0]["text"]
                send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "fake-session", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Continued in original session"}}}})
                reason = "end_turn"
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": reason}})
            continue
        if mode.startswith("mcp_"):
            address = mcp_server["args"][-1]
            host, port = address.rsplit(":", 1)
            token = next(item["value"] for item in mcp_server["env"]
                         if item["name"] == "TESHI_AUTHORING_BRIDGE_TOKEN")

            def mcp_request(method, params=None):
                with socket.create_connection((host, int(port)), timeout=10) as connection:
                    connection.settimeout(20)
                    connection.sendall((json.dumps({"token": token, "request": {
                        "jsonrpc": "2.0", "id": 1, "method": method,
                        "params": params or {}}}) + "\n").encode())
                    return json.loads(connection.makefile("r").readline())["result"]

            discovery = mcp_request("tools/list")
            if mode == "mcp_catalog":
                prompt = message["params"]["prompt"][0]["text"]
                text = json.dumps({"tools": [tool["name"] for tool in discovery["tools"]],
                                   "restored": "[Restored conversation history]" in prompt})
                send({"jsonrpc": "2.0", "method": "session/update", "params": {
                    "sessionId": "fake-session", "update": {"sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": text}}}})
                send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
                continue
            assert any(tool["name"] == "submit_requirements" for tool in discovery["tools"])
            results = []
            if mode == "mcp_review" and prompt_count > 1:
                prompt = "\n".join(
                    item.get("text", "")
                    for item in message["params"].get("prompt", [])
                    if isinstance(item, dict)
                )
                approved = re.search(r"Human approved test points \[([^]]+)\]", prompt)
                assert approved, "approved test-point ids missing from continuation prompt"
                ids = [item.strip() for item in approved.group(1).split(",") if item.strip()]
                steps = ["Given a user", "When the user signs in", "Then access is granted"]
                calls = [
                    ("generate_plan", {"features": [{"file_name": "approved.feature", "feature_name": "Auth",
                        "scenarios": [{"name": "Valid login", "steps": steps, "test_point_ids": ids}]}]}),
                    ("create_feature_file", {"file_name": "approved.feature", "feature_name": "Auth"}),
                    ("insert_scenario", {"file_path": "approved.feature", "scenario_name": "Valid login",
                        "steps": steps, "test_point_ids": ids}),
                    ("validate_feature", {"file_path": "approved.feature"}),
                ]
            elif mode == "mcp_review":
                calls = [
                    ("submit_requirements", {"feature_name": "Auth", "scenario_descriptions": ["login"]}),
                    ("propose_test_points", {"test_points": [{"title": "Valid login", "objective": "User authenticates", "hierarchy_path": ["Auth", "Login"]}]}),
                    ("generate_plan", {"features": []}),
                    ("create_feature_file", {"file_name": "blocked.feature", "feature_name": "Blocked"}),
                ]
            else:
                calls = [("create_feature_file", {"file_name": "mcp-created.feature", "feature_name": "MCP created"})]
            for name, arguments in calls:
                results.append(mcp_request("tools/call", {"name": name, "arguments": arguments}))
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": "fake-session", "update": {"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": json.dumps(results)}}}})
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
            continue
        if mode.startswith("visual_"):
            gate = Path(os.environ["TESHI_ACTIVITY_GATE"])

            def update(value):
                send({"jsonrpc": "2.0", "method": "session/update", "params": {
                    "sessionId": "fake-session", "update": value}})

            def wait_stage(stage):
                deadline = time.monotonic() + 30
                while not gate.with_suffix("." + stage).exists():
                    if time.monotonic() > deadline:
                        raise TimeoutError(stage)
                    time.sleep(0.02)

            update({"sessionUpdate": "agent_message_chunk", "content": {
                "type": "text", "text": "I will inspect the project"}})
            wait_stage("tools")
            if mode == "visual_failed":
                send({"jsonrpc": "2.0", "id": message["id"], "error": {
                    "code": -1, "message": "controlled failure"}})
                continue
            if mode == "visual_cancelled":
                send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "cancelled"}})
                continue
            update({"sessionUpdate": "tool_call", "toolCallId": "read-project",
                    "title": "Read project", "status": "in_progress"})
            update({"sessionUpdate": "tool_call_update", "toolCallId": "read-project",
                    "status": "completed"})
            if mode == "visual_no_final":
                send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
                continue
            update({"sessionUpdate": "agent_message_chunk", "content": {
                "type": "text", "text": "Inspection finished"}})
            wait_stage("finish")
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
            continue
        if mode == "restored":
            prompt = message["params"]["prompt"][0]["text"]
            if prompt_count == 1:
                assert "previous question" in prompt and "previous answer" in prompt
                assert "current question" in prompt
            else:
                assert "[Restored conversation history]" not in prompt
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
            if mode == "activity":
                for update in [
                    {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "I will inspect the project"}},
                    {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "Inspecting "}},
                    {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "the project"}},
                    {"sessionUpdate": "tool_call", "toolCallId": "read-1", "title": "Read project", "status": "in_progress"},
                    {"sessionUpdate": "tool_call_update", "toolCallId": "read-1", "status": "completed"},
                ]:
                    send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "fake-session", "update": update}})
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": "fake-session", "update": {
                    "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hello"}}}})
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": "fake-session", "update": {
                    "sessionUpdate": "tool_call", "title": "reading project", "status": "in_progress"}}})
            if mode == "activity":
                send({"jsonrpc": "2.0", "method": "session/update", "params": {
                    "sessionId": "fake-session", "update": {
                        "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Inspection finished"}}}})
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
