import json
import sys
import os
import queue
import threading
import time
import socket
import re
from pathlib import Path

mode = sys.argv[1]
model = "a"
fast = False
prompt_count = 0
approval = "ask"


def config_options():
    options = [
        {"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": model,
         "options": [{"value": "a", "name": "Alpha"}, {"value": "b", "name": "Beta"}, {"value": "reject", "name": "Rejected"}]},
        {"id": "effort", "name": "Thinking", "category": "thought_level", "type": "select", "currentValue": "high" if model == "a" else "low",
         "options": [{"value": "high" if model == "a" else "low", "name": "High" if model == "a" else "Low"}]},
        {"id": "fast", "name": "Fast", "type": "boolean", "currentValue": fast},
    ]
    if mode.startswith("settings_approval"):
        options.append({"id": "approval_policy", "name": "Approval Policy", "type": "select", "currentValue": approval,
                        "options": [{"value": "ask", "name": "Ask before changes"},
                                    {"value": "auto", "name": "Auto approve changes"},
                                    {"value": "reject", "name": "Rejected policy"}]})
    if mode == "settings_approval_multiple":
        options.append({"id": "permissions", "name": "Permissions", "type": "boolean", "currentValue": fast})
    return options

if len(sys.argv) > 2:
    marker = Path(sys.argv[2])
    marker.write_text(str(os.getpid()), encoding="utf-8")


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def send_update(session_id, value):
    send({"jsonrpc": "2.0", "method": "session/update",
          "params": {"sessionId": session_id, "update": value}})


def run_cancel_mode():
    """One long turn that only ends when Teshi sends session/cancel.

    The session id embeds the pid so a reconnecting backend is visible: the
    follow-up prompt must arrive in the same process and the same session.
    """
    session_id = "fake-session-{}".format(os.getpid())
    inbound = queue.Queue()

    def read_lines():
        for line in sys.stdin:
            try:
                inbound.put(json.loads(line))
            except Exception:
                pass

    threading.Thread(target=read_lines, daemon=True).start()
    prompts = 0
    while True:
        message = inbound.get()
        method = message.get("method")
        if method == "initialize":
            send({"jsonrpc": "2.0", "id": message["id"], "result": {
                "protocolVersion": 1, "agentInfo": {"name": "fake-agent"}}})
        elif method == "session/new":
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"sessionId": session_id}})
        elif method == "session/prompt":
            prompts += 1
            params = message["params"]
            assert params["sessionId"] == session_id, "prompt created a new ACP session"
            prompt = params["prompt"][0]["text"]
            if prompts > 1:
                assert "[Restored conversation history]" not in prompt, (
                    "cancel reconnected instead of reusing the session")
                send_update(session_id, {"sessionUpdate": "agent_message_chunk",
                                         "content": {"type": "text", "text": "second-turn-ok"}})
                send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
                continue
            send_update(session_id, {"sessionUpdate": "agent_message_chunk",
                                     "content": {"type": "text", "text": "working"}})
            if mode == "cancel_then_permission":
                send({"jsonrpc": "2.0", "id": 91, "method": "session/request_permission", "params": {
                    "sessionId": session_id,
                    "options": [{"optionId": "allow_once", "name": "Allow once", "kind": "allow_once"}]}})
            cancelled = False
            permission_answered = False
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                try:
                    nxt = inbound.get(timeout=0.05)
                except queue.Empty:
                    continue
                if nxt.get("id") == 91:
                    outcome = nxt.get("result", {}).get("outcome", {}).get("outcome")
                    assert outcome == "cancelled", (
                        "cancelled turn answered permission with {}".format(outcome))
                    permission_answered = True
                elif nxt.get("method") == "session/cancel":
                    assert nxt["params"]["sessionId"] == session_id
                    cancelled = True
                if cancelled and (mode != "cancel_then_permission" or permission_answered):
                    break
            assert cancelled, "agent never received session/cancel"
            assert mode != "cancel_then_permission" or permission_answered, (
                "pending permission was not answered as cancelled")
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "cancelled"}})


if mode in ("cancel_keeps_session", "cancel_then_permission"):
    run_cancel_mode()
    sys.exit(0)


def log_pool_event(entry):
    log_path = os.environ.get("TESHI_POOL_LOG")
    if log_path:
        with open(log_path, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(entry) + "\n")


def run_pooled_reuse_mode():
    pid = os.getpid()
    next_session = 0
    active_sessions = {}
    for line in sys.stdin:
        message = json.loads(line)
        method = message.get("method")
        if method == "initialize":
            log_pool_event({"method": "initialize", "pid": pid})
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {
                    "protocolVersion": 1,
                    "agentInfo": {"name": "fake-pooled-agent"},
                    "agentCapabilities": {
                        "sessionCapabilities": {
                            "close": {},
                            "list": {},
                            "delete": {},
                        }
                    },
                },
            })
        elif method == "session/new":
            next_session += 1
            sid = f"pool-{pid}-{next_session}"
            active_sessions[sid] = message["params"].get("cwd", "")
            log_pool_event({"method": "session/new", "pid": pid, "sessionId": sid})
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"sessionId": sid}})
        elif method == "session/close":
            sid = message["params"]["sessionId"]
            active_sessions.pop(sid, None)
            log_pool_event({"method": "session/close", "pid": pid, "sessionId": sid})
            send({"jsonrpc": "2.0", "id": message["id"], "result": {}})
        elif method == "session/delete":
            sid = message["params"]["sessionId"]
            active_sessions.pop(sid, None)
            log_pool_event({"method": "session/delete", "pid": pid, "sessionId": sid})
            send({"jsonrpc": "2.0", "id": message["id"], "result": {}})
        elif method == "session/list":
            cwd = message.get("params", {}).get("cwd")
            sessions = [
                {"sessionId": sid, "cwd": scwd, "title": f"Remote {sid}"}
                for sid, scwd in active_sessions.items()
                if cwd is None or scwd == cwd
            ]
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"sessions": sessions}})
        elif method == "session/prompt":
            params = message["params"]
            sid = params["sessionId"]
            prompt = params["prompt"][0]["text"]
            if "crash-now" in prompt:
                sys.exit(13)
            if sid not in active_sessions:
                send({
                    "jsonrpc": "2.0",
                    "id": message["id"],
                    "error": {"code": -32000, "message": f"session {sid} is closed"},
                })
                continue
            send_update(sid, {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": f"pid={pid};sid={sid};prompt={prompt}"},
            })
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})


if mode == "pooled_reuse":
    run_pooled_reuse_mode()
    sys.exit(0)


authenticated = mode not in ("auth_required", "auth_required_exit")

for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        if mode == "initialize_failed":
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -1, "message": "init rejected"}})
        else:
            result = {"protocolVersion": 1, "agentInfo": {"name": "fake-agent"}}
            capabilities = {}
            if mode == "images":
                capabilities["promptCapabilities"] = {"image": True}
            if mode in ("resume_session", "resume_then_prompt"):
                capabilities["sessionCapabilities"] = {"resume": {}}
            if mode in ("load_session", "load_then_prompt", "load_missing"):
                capabilities["loadSession"] = True
            if mode in ("auth_required", "auth_required_exit"):
                result["authMethods"] = [
                    {"id": "oauth", "name": "OAuth", "description": "Browser login"},
                    {"id": "token", "name": "API Token"},
                ]
            if capabilities:
                result["agentCapabilities"] = capabilities
            send({"jsonrpc": "2.0", "id": message["id"], "result": result})
    elif method == "authenticate":
        method_id = message.get("params", {}).get("methodId")
        if mode == "auth_required" and method_id == "oauth":
            authenticated = True
            send({"jsonrpc": "2.0", "id": message["id"], "result": {}})
        else:
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32000, "message": f"unsupported auth method: {method_id}"}})
    elif method == "session/resume":
        assert mode in ("resume_session", "resume_then_prompt"), mode
        params = message["params"]
        assert params["sessionId"] == "saved-session"
        assert params["cwd"]
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"sessionId": "saved-session"}})
    elif method == "session/load":
        assert mode in ("load_session", "load_then_prompt", "load_missing"), mode
        params = message["params"]
        if mode == "load_missing":
            send({"jsonrpc": "2.0", "id": message["id"], "error": {
                "code": -32000, "message": "session not found"}})
            continue
        assert params["sessionId"] == "saved-session"
        # Replay one historical turn before answering, as the protocol allows.
        send_update("saved-session", {
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": "replayed user"}})
        send_update("saved-session", {
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "replayed agent"}})
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"sessionId": "saved-session"}})
    elif method == "session/new":
        if mode in ("auth_required", "auth_required_exit") and not authenticated:
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32000, "message": "Authentication required"}})
            if mode == "auth_required_exit":
                time.sleep(0.1)
                sys.exit(0)
            continue
        if mode.startswith("mcp_"):
            mcp_server = message["params"]["mcpServers"][0]
            assert mcp_server["name"] == "teshi-authoring"
        if mode in ("resume_session", "resume_then_prompt", "load_session", "load_then_prompt"):
            raise AssertionError("restore-capable modes must not fall back to session/new")
        if mode == "session_failed":
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -1, "message": "session rejected"}})
        else:
            result = {"sessionId": "fake-session"}
            if mode == "settings_approval_delayed":
                time.sleep(0.3)
            if mode.startswith("settings"):
                result["configOptions"] = config_options()
            if mode == "approval_legacy":
                result["modes"] = {"currentModeId": approval, "availableModes": [
                    {"id": "ask", "name": "Ask before changes"},
                    {"id": "auto", "name": "Auto approve changes"}]}
            send({"jsonrpc": "2.0", "id": message["id"], "result": result})
            if mode == "commands_and_plan":
                send_update("fake-session", {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": [
                        {"name": "review", "description": "Review changes", "input": {"hint": "path"}},
                        {"name": "compact", "description": "Compact context"},
                    ],
                })
            if mode == "stalled_stdin":
                # A wedged agent: alive, but never reads Teshi's next write.
                time.sleep(600)
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
        elif params["configId"] == "approval_policy":
            approval = params["value"]
        elif params["configId"] in ("fast", "permissions"):
            assert params["type"] == "boolean" and isinstance(params["value"], bool)
            fast = params["value"]
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"configOptions": config_options()}})
    elif method == "session/set_mode":
        assert mode == "approval_legacy" and message["params"]["sessionId"] == "fake-session"
        approval = message["params"]["modeId"]
        send({"jsonrpc": "2.0", "id": message["id"], "result": {}})
        send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "fake-session", "update": {
            "sessionUpdate": "current_mode_update", "currentModeId": approval}}})
    elif method == "session/prompt":
        prompt_count += 1
        if mode in ("resume_then_prompt", "load_then_prompt", "resume_session", "load_session"):
            assert message["params"]["sessionId"] == "saved-session"
            prompt = message["params"]["prompt"][0]["text"]
            assert "[Restored conversation history]" not in prompt, prompt
            send_update("saved-session", {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "restored-ok"}})
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
            continue
        if mode == "load_missing":
            assert message["params"]["sessionId"] == "fake-session"
            prompt = message["params"]["prompt"][0]["text"]
            assert "[Restored conversation history]" in prompt, prompt
            send_update("fake-session", {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "seeded-ok"}})
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
            continue
        if mode == "images":
            summary = [
                block["type"] if block["type"] == "text"
                else "{}:{}".format(block["mimeType"], len(block["data"]))
                for block in message["params"]["prompt"]
            ]
            send({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": "fake-session", "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": json.dumps(summary)}}}})
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
            continue
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
        if mode.startswith("transient_"):
            # Mimics Cursor's adapter: a dropped connection becomes message
            # text and the turn still ends with end_turn.
            prompt = message["params"]["prompt"][0]["text"]

            def chunk(text):
                send_update("fake-session", {"sessionUpdate": "agent_message_chunk",
                                             "content": {"type": "text", "text": text}})

            notice = "\n\nError: RetriableError: [canceled] http/2 stream closed with error code CANCEL (0x8)"
            failing = mode == "transient_always" or prompt_count == 1
            if failing and mode == "transient_prompt_error_once":
                send({"jsonrpc": "2.0", "id": message["id"], "error": {
                    "code": -1, "message": "Error: RetriableError: ECONNRESET"}})
                continue
            if failing:
                if mode == "transient_after_progress":
                    chunk("working")
                    send_update("fake-session", {"sessionUpdate": "tool_call", "toolCallId": "edit-1",
                                                 "title": "Edit file", "status": "completed"})
                chunk(notice)
            elif "do not repeat completed tool actions" in prompt:
                chunk("continued")
            else:
                chunk("recovered:" + prompt)
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
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
                steps = ["Given a user", "When the user signs in", "Then access is granted"]
                calls = [
                    ("submit_requirements", {"feature_name": "Auth", "scenario_descriptions": ["login"]}),
                    ("propose_test_points", {"test_points": [{"title": "Valid login", "hierarchy_path": ["Auth", "Login"]}]}),
                    ("generate_plan", {"features": [{"file_name": "approved.feature", "feature_name": "Auth",
                        "scenarios": [{"name": "Valid login", "steps": steps, "test_point_ids": ["tp-1"]}]}]}),
                    ("create_feature_file", {"file_name": "approved.feature", "feature_name": "Auth"}),
                    ("insert_scenario", {"file_path": "approved.feature", "scenario_name": "Valid login",
                        "steps": steps, "test_point_ids": ["tp-1"]}),
                    ("validate_feature", {"file_path": "approved.feature"}),
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
        if mode in ("crash_once", "crash_once_then_permission"):
            state = Path(os.environ["TESHI_CRASH_STATE"])
            if not state.exists():
                state.write_text("crashed", encoding="utf-8")
                sys.exit(13)
            if mode == "crash_once":
                prompt = message["params"]["prompt"][0]["text"]
                assert "[Restored conversation history]" in prompt, (
                    "a reconnected process lost the saved transcript")
                send({"jsonrpc": "2.0", "method": "session/update", "params": {
                    "sessionId": "fake-session", "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": "restored-ok"}}}})
                send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
                continue
        if mode == "prompt_failed":
            send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -1, "message": "prompt rejected"}})
        elif mode in ("permission_then_exit", "crash_once_then_permission"):
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
        elif mode == "commands_and_plan":
            prompt = message["params"]["prompt"][0]["text"]
            if prompt_count == 1:
                send_update("fake-session", {
                    "sessionUpdate": "plan",
                    "entries": [
                        {"content": "Read feature files", "priority": "high", "status": "completed"},
                        {"content": "Verify scenario steps", "priority": "medium", "status": "in_progress"},
                    ],
                })
                send_update("fake-session", {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "planned"},
                })
            else:
                send_update("fake-session", {"sessionUpdate": "plan_removed"})
                send_update("fake-session", {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": f"echo:{prompt}"},
                })
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
        elif mode == "auth_required":
            assert authenticated, "session/prompt before authentication"
            prompt = message["params"]["prompt"][0]["text"]
            if "fail-prompt" in prompt:
                send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -1, "message": "prompt rejected"}})
            else:
                send_update("fake-session", {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "authed-ok"},
                })
                send({"jsonrpc": "2.0", "id": message["id"], "result": {"stopReason": "end_turn"}})
        else:
            if mode == "activity":
                for update in [
                    {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "I will inspect the project"}},
                    {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "Inspecting "}},
                    {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "the project"}},
                    {"sessionUpdate": "tool_call", "toolCallId": "read-1", "title": "Read project", "status": "in_progress", "kind": "read", "rawInput": {"path": "src/sample.rs"}, "locations": [{"path": "src/sample.rs"}]},
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
