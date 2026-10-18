#!/usr/bin/env python3
"""Bind legacy TUI Features to the real PTY runner.

The repository's older TUI regressions were already driven by Rust-owned PTY
tests.  This adapter gives those same flows the standard ``teshi run``
protocol so the Feature matrix can execute them and persist case results.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parent.parent
STEP_RE = re.compile(
    r"^\s*(?:Given|When|Then|And|But|假设|假如|当|那么|并且|而且|同时|如果|但是)\s+(.+?)\s*$"
)
SCENARIO_RE = re.compile(r"^\s*(?:Scenario(?: Outline)?|场景(?:大纲)?):\s*(.+?)\s*$")

CANONICAL = {
    "tui_acp_long_conversation.feature": "tui_acp_long_conversation.feature",
    "ACP长对话.feature": "tui_acp_long_conversation.feature",
    "tui_acp_permission_allow.feature": "tui_acp_permission_allow.feature",
    "ACP权限允许对话.feature": "tui_acp_permission_allow.feature",
    "tui_acp_permission_reject.feature": "tui_acp_permission_reject.feature",
    "ACP权限拒绝对话.feature": "tui_acp_permission_reject.feature",
    "tui_acp_settings.feature": "tui_acp_settings.feature",
    "ACP会话设置.feature": "tui_acp_settings.feature",
    "tui_acp_wrapped_chat.feature": "tui_acp_wrapped_chat.feature",
    "ACP换行对话.feature": "tui_acp_wrapped_chat.feature",
    "tui_editor.feature": "tui_editor.feature",
    "终端功能编辑.feature": "tui_editor.feature",
    "tui_exit_performance.feature": "tui_exit_performance.feature",
    "TUI退出性能.feature": "tui_exit_performance.feature",
    "tui_codebuddy_live_conversation.feature": "tui_codebuddy_live_conversation.feature",
    "CodeBuddy真实ACP对话.feature": "tui_codebuddy_live_conversation.feature",
}


def parse_source(source: str) -> tuple[list[dict], list[dict]]:
    background: list[dict] = []
    scenarios: list[dict] = []
    current: dict | None = None
    in_examples = False
    for line_number, line in enumerate(source.splitlines(), 1):
        scenario = SCENARIO_RE.match(line)
        if scenario:
            current = {"name": scenario.group(1), "line": line_number, "steps": []}
            scenarios.append(current)
            in_examples = False
            continue
        if current and line.strip().startswith("Examples:"):
            in_examples = True
            continue
        if in_examples and line.strip().startswith("|"):
            continue
        step = STEP_RE.match(line)
        if step:
            item = {"line": line_number, "text": step.group(1)}
            if current is None:
                background.append(item)
            else:
                current["steps"].append(item)
    return background, scenarios


def action(kind: str, **values) -> dict:
    return {"action": kind, **values}


def expect(text: str, timeout_ms: int = 15000) -> dict:
    return action("expect_text", text=text, visible=True, timeout_ms=timeout_ms)


def idle(timeout_ms: int = 15000) -> dict:
    return action("expect_stable_text", text="│ ○ ", visible=True, timeout_ms=timeout_ms, stable_ms=150)


def key(value: str) -> dict:
    return action("key", key=value)


def write(value: str) -> dict:
    return action("write", text=value)


def exit_expect(timeout_ms: int = 10000) -> dict:
    return action("expect_exit", code=0, timeout_ms=timeout_ms)


def env_pair(root: Path, name: str, value: str) -> tuple[str, str]:
    return name, value


def launch(root: Path, teshi: str, *, mode: str, cols: int, rows: int) -> dict:
    project = root / "project"
    requirements = root / "requirements"
    project.mkdir(parents=True, exist_ok=True)
    requirements.mkdir(parents=True, exist_ok=True)
    (project / "sample.feature").write_text(
        "Feature: Terminal fixture\n"
        "  Scenario: Sample scenario\n"
        "    Given a sample step\n",
        encoding="utf-8",
    )
    values = [
        env_pair(root, "APPDATA", str(root / "appdata")),
        env_pair(root, "LOCALAPPDATA", str(root / "localdata")),
        env_pair(root, "XDG_CONFIG_HOME", str(root / "config")),
        env_pair(root, "XDG_DATA_HOME", str(root / "data")),
        env_pair(root, "TESHI_ACP_STORE_DIR", str(root / "acp")),
        env_pair(root, "TESHI_SESSION_DIR", str(root / "sessions")),
    ]
    if mode == "exit":
        fixture = ROOT / "crates/teshi-tui-runner/tests/fixtures/agent_acceptance_fixture.py"
        trace = root / "exit-trace"
        trace.mkdir(exist_ok=True)
        (trace / "Exit Agent-behavior.json").write_text(json.dumps({"reply":"Inspection finished"}), encoding="utf-8")
        values.extend([
            ("TESHI_AGENT_BACKEND", "acp"),
            ("TESHI_ACP_PROGRAM", sys.executable),
            ("TESHI_ACP_ARGS_JSON", json.dumps([str(fixture), "acp", str(trace), "Exit Agent"])),
        ])
    elif mode == "wrapped":
        fixture = ROOT / "crates/teshi-tui-runner/tests/fixtures/wrapped_chat_acp.py"
        values.extend(
            [
                env_pair(root, "TESHI_AGENT_BACKEND", "acp"),
                env_pair(root, "TESHI_ACP_PROGRAM", sys.executable),
                env_pair(root, "TESHI_ACP_ARGS_JSON", json.dumps([str(fixture)])),
            ]
        )
    elif mode == "permission":
        fixture = ROOT / "crates/teshi-tui-runner/tests/fixtures/permission_prompt_acp.py"
        values.extend(
            [
                env_pair(root, "TESHI_AGENT_BACKEND", "acp"),
                env_pair(root, "TESHI_ACP_PROGRAM", sys.executable),
                env_pair(root, "TESHI_ACP_ARGS_JSON", json.dumps([str(fixture)])),
            ]
        )
    elif mode == "settings":
        fixture = ROOT / "crates/teshi-agent-runtime/tests/fixtures/fake_acp_agent.py"
        values.extend(
            [
                env_pair(root, "TESHI_AGENT_BACKEND", "acp"),
                env_pair(root, "TESHI_ACP_PROGRAM", sys.executable),
                env_pair(
                    root,
                    "TESHI_ACP_ARGS_JSON",
                    json.dumps([str(fixture), "settings"]),
                ),
            ]
        )
    elif mode == "editor":
        generic = Path(teshi).with_name(
            "teshi-tui-runner" + (".exe" if os.name == "nt" else "")
        )
        child = (
            "import sys; print('self-runner-ready', flush=True); "
            "[None for line in sys.stdin if line.strip() == 'quit']"
        )
        (project / "sample.tui.json").write_text(
            json.dumps(
                {
                    "version": 1,
                    "mode": "tui",
                    "steps": [
                        {
                            "line": 3,
                            "text": "a sample step",
                            "confirmed": True,
                            "actions": [
                                action(
                                    "launch",
                                    config={
                                        "program": sys.executable,
                                        "args": ["-c", child],
                                        "cwd": str(root),
                                        "env": [],
                                        "cols": 80,
                                        "rows": 24,
                                    },
                                ),
                                expect("self-runner-ready", timeout_ms=10000),
                            ],
                        }
                    ],
                },
                ensure_ascii=False,
            ),
            encoding="utf-8",
        )
        values.append(env_pair(root, "TESHI_TUI_RUNNER", str(generic)))
    elif mode == "codebuddy":
        app_data = os.environ["APPDATA"]
        local_data = os.environ["LOCALAPPDATA"]
        manifest = Path(local_data) / "teshi/acp/installed.json"
        installed = json.loads(manifest.read_text(encoding="utf-8"))
        profile = next(agent for agent in installed if agent["id"] == "codebuddy-code")
        package_root = Path(profile["program"]).parent.parent
        entry = package_root / "@tencent-ai/codebuddy-code/bin/codebuddy"
        if not entry.is_file():
            raise ValueError("installed CodeBuddy entry is missing")
        proxy = ROOT / "crates/teshi-tui-runner/tests/fixtures/codebuddy_acp_proxy.mjs"
        values.extend([
            ("TESHI_AGENT_BACKEND", "acp"),
            ("TESHI_ACP_PROGRAM", "node"),
            ("TESHI_ACP_ARGS_JSON", json.dumps([str(proxy)])),
            ("TESHI_CODEBUDDY_ENTRY", str(entry)),
            ("TESHI_CODEBUDDY_ARGS_JSON", json.dumps(profile.get("args", []))),
            ("TESHI_CODEBUDDY_PROTOCOL_LOG", str(root / "codebuddy-protocol.ndjson")),
            ("TESHI_CODEBUDDY_APPDATA", app_data),
            ("TESHI_CODEBUDDY_LOCALAPPDATA", local_data),
        ])
    return action(
        "launch",
        config={
            "program": teshi,
            "args": [str(project), "--requirements-root", str(requirements)],
            "cwd": str(root),
            "env": values,
            "cols": cols,
            "rows": rows,
        },
    )


def background_actions(canonical_name: str, root: Path, teshi: str) -> list[list[dict]]:
    if canonical_name == "tui_codebuddy_live_conversation.feature":
        return [[launch(root, teshi, mode="codebuddy", cols=100, rows=30), expect("Explore")]]
    if canonical_name == "tui_editor.feature":
        return [[launch(root, teshi, mode="editor", cols=140, rows=40), expect("Sample scenario")]]
    if canonical_name == "tui_acp_settings.feature":
        return [[launch(root, teshi, mode="settings", cols=140, rows=32), expect("Explore")]]
    if canonical_name in {
        "tui_acp_long_conversation.feature",
        "tui_acp_wrapped_chat.feature",
    }:
        return [[launch(root, teshi, mode="wrapped", cols=100, rows=30), expect("Explore")]]
    if canonical_name in {
        "tui_acp_permission_allow.feature",
        "tui_acp_permission_reject.feature",
    }:
        return [[launch(root, teshi, mode="permission", cols=100, rows=30), expect("Explore")]]
    if canonical_name == "tui_exit_performance.feature":
        return [
            [launch(root, teshi, mode="exit", cols=100, rows=30), expect("Explore")],
            [expect("Explore")],
            [expect("Explore")],
        ]
    raise ValueError(f"no TUI binding recipe for {canonical_name}")


def scenario_actions(canonical_name: str, scenario_index: int, root: Path, teshi: str) -> list[list[dict]]:
    if canonical_name == "tui_codebuddy_live_conversation.feature":
        actions = [[key("3"), expect("Welcome to AI Chat!"), key("Enter")]]
        for turn in range(1, 11):
            actions.extend([
                [write(f"This is live conversation turn {turn}. Reply only with the concatenation of LIVE_REPLY_ and {turn:02}. Do not use tools or add explanation."), key("Enter")],
                [expect(f"LIVE_REPLY_{turn:02}", timeout_ms=120000), idle(timeout_ms=120000)],
            ])
        return actions + [[key("Escape")], [key("q")], [exit_expect()]]
    if canonical_name == "tui_editor.feature":
        if scenario_index == 0:
            return [
                [expect("Sample scenario")],
                [key("3")],
                [expect("Welcome to AI Chat!")],
                [key("1")],
                [expect("Sample scenario")],
                [key("q")],
                [exit_expect()],
            ]
        return [
            [expect("Sample scenario")],
            [key("r")],
            [key("e"), expect("Edit [Enter]")],
            [expect("Status: Passed"), expect("[passed] a sample step")],
            [key("Enter"), key("End"), write(" updated"), key("Enter")],
            [expect("a sample step updated")],
            [key("s")],
            [expect("a sample step updated")],
            [key("q")],
            [exit_expect()],
        ]

    if canonical_name == "tui_acp_wrapped_chat.feature":
        actions: list[list[dict]] = [[key("3"), expect("Welcome to AI Chat!"), key("Enter")]]
        for turn in range(1, 7):
            actions.extend(
                [
                    [write(f"Conversation turn {turn}: inspect the project context carefully. T{turn:02d}"), expect(f"T{turn:02d}"), key("Enter")],
                    [expect(f"WRAPPED_REPLY_{turn:02d}"), idle()],
                ]
            )
        actions.extend([[key("Escape")], [key("q")], [exit_expect()]])
        return actions

    if canonical_name == "tui_acp_long_conversation.feature":
        actions = [[key("3"), expect("Welcome to AI Chat!"), key("Enter")]]
        for first, last in ((1, 10), (11, 20), (21, 30)):
            batch: list[dict] = []
            for turn in range(first, last + 1):
                batch.extend(
                    [
                        write(
                            f"Conversation turn {turn}: inspect the project context carefully and return the deterministic response marker. T{turn:02d}"
                        ),
                        expect(f"T{turn:02d}"),
                        key("Enter"),
                        expect(f"WRAPPED_REPLY_{turn:02d}"),
                        idle(),
                    ]
                )
            actions.extend([[*batch], [expect(f"WRAPPED_REPLY_{last:02d}")]])
        actions.extend([[key("Escape")], [key("q")], [exit_expect()]])
        return actions

    if canonical_name in {
        "tui_acp_permission_allow.feature",
        "tui_acp_permission_reject.feature",
    }:
        actions = [
            [key("3"), expect("Welcome to AI Chat!"), key("Enter")],
            [write("Please inspect the project and report back."), key("Enter")],
        ]
        if canonical_name.endswith("allow.feature"):
            actions.extend(
                [
                    [expect("Allow once"), expect("Allow always")],
                    [key("Down")],
                    [key("Enter")],
                    [expect("PERMISSION_AFTER_allow_always")],
                    [expect("PERMISSION_BEFORE")],
                    [key("Escape")],
                    [key("q")],
                    [exit_expect()],
                ]
            )
        else:
            actions.extend(
                [
                    [expect("Allow once"), expect("Allow always")],
                    # First blur the focused input, then reject the request.
                    [key("Escape"), key("Escape"),
                     action("expect_text", text="Allow once", visible=False, timeout_ms=5000)],
                    [action("expect_text", text="PERMISSION_AFTER", visible=False, timeout_ms=5000)],
                    [key("Escape")],
                    [key("q")],
                    [exit_expect()],
                ]
            )
        return actions

    if canonical_name == "tui_acp_settings.feature":
        return [
            [key("3"), expect("Welcome to AI Chat!"), key("Enter")],
            [write("/settings"), key("Enter"), expect("Model: Alpha")],
            [expect("Thinking"), expect("[More")],
            [action("click_text", text="Model: Alpha", timeout_ms=15000), expect("Beta")],
            [key("Down"), key("Enter")],
            [expect("Model: Beta")],
            [expect("Thinking: Low")],
            [action("resize", cols=64, rows=24)],
            [expect("[More")],
            [
                write("/settings"),
                key("Enter"),
                expect("Fast: Off"),
                key("Down"),
                key("Down"),
                key("Enter"),
                key("Down"),
                key("Enter"),
            ],
            [write("/settings"), key("Enter"), expect("Fast: On"), key("Escape")],
            [key("Escape")],
            [key("q")],
            [exit_expect()],
        ]

    if canonical_name == "tui_exit_performance.feature":
        if scenario_index == 0:
            return [
                [expect("Explore")],
                [key("q")],
                [exit_expect(1000)],
                [exit_expect(1000)],
            ]
        if scenario_index == 1:
            return [
                [expect("Explore")],
                [expect("Explore")],
                [key("q")],
                [exit_expect(3000)],
                [exit_expect(1000)],
                [exit_expect(1000)],
            ]
        if scenario_index == 2:
            return [
                [expect("Explore")],
                [key("q")],
                [exit_expect(5000)],
                [exit_expect(1000)],
                [exit_expect(1000)],
            ]
        if scenario_index == 3:
            return [
                [expect("Explore")],
                [expect("Explore")],
                [key("q")],
                [exit_expect(1000)],
                [exit_expect(1000)],
            ]
        return [
            [key("3"), expect("Welcome to AI Chat!"), key("Enter")],
            [write("Inspect the project"), key("Enter"), expect("Inspection finished"), expect("│ ○ ")],
            [key("Escape"), key("q")],
            [exit_expect(1000)],
            [launch(root, teshi, mode="exit", cols=100, rows=30), expect("Explore")],
            [key("3"), key("Enter"), write("/resume"), key("Enter")],
            [expect("Inspection finished")],
        ]
    raise ValueError(f"no scenario binding recipe for {canonical_name} #{scenario_index}")


def make_sidecar(source: str, canonical_source: str, root: Path, teshi: str, key_name: str) -> dict:
    actual_background, actual_scenarios = parse_source(source)
    canonical_background, canonical_scenarios = parse_source(canonical_source)
    background = background_actions(key_name, root, teshi)
    if len(background) != len(actual_background) or len(background) != len(canonical_background):
        raise ValueError(f"background binding count mismatch for {key_name}")
    bindings = []
    for item, actions in zip(actual_background, background):
        bindings.append({"line": item["line"], "text": item["text"], "confirmed": True, "actions": actions})
    if len(actual_scenarios) != len(canonical_scenarios):
        raise ValueError(f"scenario count mismatch for {key_name}")
    for index, (actual, canonical) in enumerate(zip(actual_scenarios, canonical_scenarios)):
        actions = scenario_actions(key_name, index, root, teshi)
        if len(actions) != len(actual["steps"]) or len(actions) != len(canonical["steps"]):
            raise ValueError(
                f"step binding count mismatch for {key_name} scenario {index}: "
                f"actions={len(actions)} actual={len(actual['steps'])} canonical={len(canonical['steps'])}"
            )
        for item, action_group in zip(actual["steps"], actions):
            bindings.append({"line": item["line"], "text": item["text"], "confirmed": True, "actions": action_group})
    return {"version": 1, "mode": "tui", "steps": bindings}


def emit(value: dict) -> None:
    print(json.dumps(value, ensure_ascii=False), flush=True)


def main() -> int:
    request = json.loads(sys.stdin.readline())
    if os.environ.get("TESHI_FEATURE_ARTIFACTS"):
        request.setdefault("meta", {})["artifacts_dir"] = os.environ["TESHI_FEATURE_ARTIFACTS"]
    cases = request.get("cases", [])
    if not cases:
        return 2
    feature_path = cases[0]["feature_path"]
    original_root = Path(request["meta"]["project_root"])
    original_feature = original_root / feature_path
    key_name = CANONICAL.get(original_feature.name)
    if key_name is None:
        print(f"unsupported TUI Feature: {original_feature.name}", file=sys.stderr)
        return 2
    if key_name == "tui_codebuddy_live_conversation.feature" and os.environ.get("TESHI_CODEBUDDY_LIVE") != "1":
        emit({"type": "start_run", "total": len(cases)})
        for case in cases:
            emit({"type": "start_case", "case_id": case["id"], "name": case["scenario"]})
            emit({"type": "case_skipped", "case_id": case["id"], "reason": "blocked: live CodeBuddy credentials are not enabled"})
        emit({"type": "end_run", "passed": 0, "failed": 0, "skipped": len(cases)})
        return 0

    teshi = os.environ.get("TESHI_SELF_TEST_BIN")
    if not teshi:
        print("TESHI_SELF_TEST_BIN is required", file=sys.stderr)
        return 2
    generic = os.environ.get("TESHI_GENERIC_TUI_RUNNER")
    if not generic:
        candidate = Path(teshi).with_name("teshi-tui-runner" + (".exe" if os.name == "nt" else ""))
        generic = str(candidate)
    canonical_path = ROOT / "features/en-US" / key_name
    canonical_source = canonical_path.read_text(encoding="utf-8")

    emit({"type": "start_run", "total": len(cases)})
    failed = 0
    for case in cases:
        with tempfile.TemporaryDirectory(prefix="teshi-tui-feature-", dir=os.environ.get("TESHI_FEATURE_TEMP")) as name:
            temp_root = Path(name)
            target_feature = temp_root / feature_path
            target_feature.parent.mkdir(parents=True, exist_ok=True)
            source = original_feature.read_text(encoding="utf-8")
            target_feature.write_text(source, encoding="utf-8")
            sidecar = make_sidecar(source, canonical_source, temp_root, teshi, key_name)
            target_feature.with_suffix(".tui.json").write_text(
                json.dumps(sidecar, ensure_ascii=False, indent=2), encoding="utf-8"
            )
            child_request = dict(request)
            child_request["cases"] = [case]
            child_request["meta"] = dict(request.get("meta", {}))
            child_request["meta"]["project_root"] = str(temp_root)
            child_request["meta"]["artifacts_dir"] = str(temp_root / "artifacts")
            completed = subprocess.run(
                [generic],
                input=json.dumps(child_request, ensure_ascii=False) + "\n",
                text=True,
                encoding="utf-8",
                errors="replace",
                capture_output=True,
            )
            child_events = []
            for line in completed.stdout.splitlines():
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if event.get("type") not in {"start_run", "end_run"}:
                    if event.get("type") == "case_failed":
                        destination = Path(
                            request.get("meta", {}).get(
                                "artifacts_dir", str(ROOT / ".teshi" / "tui-results")
                            )
                        )
                        destination.mkdir(parents=True, exist_ok=True)
                        for attachment in event.get("error", {}).get("attachments", []):
                            source_attachment = Path(attachment.get("path", ""))
                            if source_attachment.is_file():
                                target_attachment = destination / (
                                    case["id"].replace(":", "_") + "-" + source_attachment.name
                                )
                                target_attachment.write_bytes(source_attachment.read_bytes())
                                attachment["path"] = str(target_attachment)
                    child_events.append(event)
                    emit(event)
            # Keep provider diagnostics before the isolated project is removed.
            protocol_log = temp_root / "codebuddy-protocol.ndjson"
            destination = Path(request.get("meta", {}).get("artifacts_dir", str(ROOT / ".teshi/tui-results")))
            case_destination = destination / re.sub(r"[^A-Za-z0-9_-]", "_", case["id"])
            if (temp_root / "artifacts").is_dir():
                shutil.copytree(temp_root / "artifacts", destination, dirs_exist_ok=True)
            if protocol_log.is_file():
                case_destination.mkdir(parents=True, exist_ok=True)
                target = case_destination / "codebuddy-protocol.ndjson"
                target.write_bytes(protocol_log.read_bytes())
            if not any(event.get("type") in {"case_passed", "case_failed", "case_skipped"} for event in child_events):
                failed += 1
                emit(
                    {
                        "type": "case_failed",
                        "case_id": case["id"],
                        "duration_ms": None,
                        "error": {"message": completed.stderr.strip() or "TUI runner produced no case result", "attachments": []},
                    }
                )
            elif any(event.get("type") == "case_failed" for event in child_events):
                failed += 1
    emit({"type": "end_run", "passed": len(cases) - failed, "failed": failed, "skipped": 0})
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
