#!/usr/bin/env python3
"""Bind legacy TUI Features to the real PTY runner.

The repository's older TUI regressions were already driven by Rust-owned PTY
tests.  This adapter gives those same flows the standard ``teshi run``
protocol so the Feature matrix can execute them and persist case results.
"""

from __future__ import annotations

import json
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time


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
    "tui_project_switch.feature": "tui_project_switch.feature",
    "项目切换.feature": "tui_project_switch.feature",
    "tui_codebuddy_live_conversation.feature": "tui_codebuddy_live_conversation.feature",
    "CodeBuddy真实ACP对话.feature": "tui_codebuddy_live_conversation.feature",
}

RUNNER_SHUTDOWN_SCENARIOS = {
    "Stopping a running task switches only after its runner tree exits",
    "停止运行中的任务后等待运行器进程树退出再切换",
}

AGENT_SWITCH_SCENARIOS = {
    "Canceling a switch while ACP stops keeps project A usable": "acp",
    "ACP 停止期间取消切换仍可使用项目 A": "acp",
    "Switching projects closes a stalled native model stream": "native",
    "切换项目会关闭阻塞的原生模型流": "native",
}

SESSION_SWITCH_SCENARIOS = {
    "Project session lists and resume stay isolated after switching": "history",
    "切换项目后会话列表和恢复保持隔离": "history",
    "Restarting in project B restores only B's history": "restart",
    "在项目 B 重启后只恢复 B 的历史": "restart",
    "Explicitly loaded legacy history remains unassigned": "legacy",
    "显式载入的旧会话仍保持未归属": "legacy",
    "A shared global requirement reloads each project's own links": "links",
    "同一全局需求重新载入各项目自己的关联": "links",
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
    if mode == "project-switch":
        project_a = root / "project-a"
        project_b = root / "project-b"
        for name, project_path, runner_name in (
            ("project-a", project_a, "runner-a"),
            ("project-b", project_b, "runner-b"),
        ):
            project_path.mkdir(parents=True, exist_ok=True)
            (project_path / "sample.feature").write_text(
                f"Feature: {name}\n"
                "  Scenario: Ready\n"
                f"    Given the {name} step\n",
                encoding="utf-8",
            )
            runner_fixture = ROOT / "crates/teshi-tui-runner/tests/fixtures/project_switch_runner.py"
            (project_path / "teshi.toml").write_text(
                "[runner]\n"
                f"cmd = {json.dumps(sys.executable)}\n"
                f"args = {json.dumps([str(runner_fixture), 'runner', name, str(project_path / 'runner-evidence')])}\n"
                'cwd = "."\n',
                encoding="utf-8",
            )
        app_data = root / "app-data"
        app_data.mkdir(parents=True, exist_ok=True)
        (app_data / "recent.json").write_text(
            json.dumps(
                {
                    "paths": [],
                    "projects": [
                        {
                            "root": str(project_b),
                            "display_name": "project-b",
                            "last_opened_ms": 1,
                        }
                    ],
                },
                ensure_ascii=False,
            ),
            encoding="utf-8",
        )
        project = project_a
        invalid_project = root / "project-invalid"
        invalid_project.mkdir()
        (invalid_project / "teshi.toml").write_text("[runner\n", encoding="utf-8")
    else:
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
    elif mode == "project-switch":
        values.append(env_pair(root, "TESHI_APP_DATA_DIR", str(root / "app-data")))
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


def project_switch_sidecar(source: str, template_path: Path, root: Path, teshi: str) -> dict:
    """Prepare isolated projects and expand only the fixture's declared paths.

    Source-bound step identities stay in the template, so the Rust runner can
    reject changed prose or line numbers before launching a terminal.
    """
    template = template_path.read_text(encoding="utf-8")
    values = {"${TESHI_SELF_TEST_BIN}": teshi, "${FIXTURE_ROOT}": str(root)}

    def expand(value):
        if isinstance(value, str):
            for token, replacement in values.items():
                value = value.replace(token, replacement)
            if "${" in value:
                raise ValueError(f"unknown project-switch template token: {value}")
            return value
        if isinstance(value, list):
            return [expand(item) for item in value]
        if isinstance(value, dict):
            return {key: expand(item) for key, item in value.items()}
        return value

    sidecar = expand(json.loads(template))
    background, scenarios = parse_source(source)
    source_steps = background + [step for scenario in scenarios for step in scenario["steps"]]
    bound_steps = [{"line": step["line"], "text": step["text"]} for step in sidecar["steps"]]
    if bound_steps != source_steps:
        raise ValueError("stale project-switch template: source steps or lines changed")
    launch(root, teshi, mode="project-switch", cols=120, rows=35)
    return sidecar


def make_sidecar(source: str, canonical_source: str, root: Path, teshi: str, key_name: str, template_path: Path | None = None) -> dict:
    if key_name == "tui_project_switch.feature":
        if template_path is None:
            raise ValueError("project-switch requires a source-bound template")
        return project_switch_sidecar(source, template_path, root, teshi)
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


def project_fixture_files(root: Path) -> dict[str, str]:
    """Fingerprint source files whose in-memory edits must stay unsaved."""
    paths = [root / name / filename for name in ("project-a", "project-b")
             for filename in ("sample.feature", "teshi.toml")]
    return {path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in paths}


def process_identity(pid: int) -> dict | None:
    """Read an OS creation identity without signaling or mutating a process.

    Missing processes return None. Access failures remain errors: an unknown
    process must never satisfy a shutdown assertion.
    """
    pid = int(pid)
    if pid <= 0:
        raise ValueError("fixture process ID must be positive")
    if os.name == "nt":
        import ctypes
        from ctypes import wintypes

        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
        kernel.GetProcessTimes.restype = wintypes.BOOL
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel.CloseHandle.restype = wintypes.BOOL
        handle = kernel.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        if not handle:
            error = ctypes.get_last_error()
            if error == 87:  # ERROR_INVALID_PARAMETER: the PID no longer exists.
                return None
            raise ValueError(f"OpenProcess({pid}) failed with Windows error {error}")
        try:
            created, exited, kernel_time, user_time = (wintypes.FILETIME() for _ in range(4))
            if not kernel.GetProcessTimes(handle, ctypes.byref(created), ctypes.byref(exited),
                                          ctypes.byref(kernel_time), ctypes.byref(user_time)):
                raise ValueError(f"GetProcessTimes({pid}) failed with Windows error {ctypes.get_last_error()}")
            ticks = lambda value: (value.dwHighDateTime << 32) | value.dwLowDateTime
            return {"pid": pid, "platform": "windows", "creation_time": str(ticks(created)),
                    "running": ticks(exited) == 0, "exit_time": str(ticks(exited)) if ticks(exited) else None}
        finally:
            kernel.CloseHandle(handle)
    if sys.platform.startswith("linux"):
        try:
            stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8")
        except FileNotFoundError:
            return None
        fields = stat[stat.rfind(")") + 2:].split()
        if len(fields) < 20:
            raise ValueError(f"invalid /proc/{pid}/stat process identity")
        boot_id = Path("/proc/sys/kernel/random/boot_id").read_text(encoding="ascii").strip()
        return {"pid": pid, "platform": "linux", "creation_time": f"{boot_id}:{fields[19]}",
                "running": fields[0] not in {"Z", "X"}}
    raise ValueError(f"process creation identity is unavailable on {sys.platform}")


def process_running(pid: int, expected_identity: dict | None = None, evidence: dict | None = None) -> bool:
    """Check an owned creation identity; a reused PID is a different process."""
    probe = evidence if evidence is not None else {}
    probe.update(pid=int(pid), expected_identity=expected_identity)
    try:
        if expected_identity is not None:
            created = expected_identity.get("creation_time")
            platform = expected_identity.get("platform")
            valid_created = isinstance(created, str) and (
                (platform == "windows" and created.isascii() and created.isdecimal() and int(created) > 0)
                or (platform == "linux" and re.fullmatch(
                    r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}:[1-9][0-9]*", created
                ) is not None)
            )
            if expected_identity.get("pid") != int(pid) or not valid_created:
                raise ValueError("missing or inconsistent recorded process creation identity")
        observed = process_identity(pid)
        probe["observed_identity"] = observed
        if observed is None:
            probe["state"] = "pid_absent"
            return False
        if expected_identity is not None and observed["platform"] != expected_identity["platform"]:
            raise ValueError("recorded process platform differs from the current identity probe")
        if expected_identity is not None and any(
            observed[key] != expected_identity[key] for key in ("pid", "platform", "creation_time")
        ):
            probe["state"] = "pid_reused"
            return False
        probe["state"] = "running" if observed["running"] else "exited"
        return observed["running"]
    except (OSError, ValueError, KeyError) as error:
        probe.update(state="unknown", error=str(error))
        raise ValueError(f"could not verify fixture process identity for PID {pid}: {error}") from error


def configure_agent_fixture(sidecar: dict, root: Path, mode: str):
    """Configure only this case's launch and retain ownership of its HTTP provider."""
    fixture = ROOT / "crates/teshi-agent-runtime/tests/fixtures/project_switch_agent.py"
    config = sidecar["steps"][0]["actions"][0]["config"]
    config["env"].append(("TESHI_AGENT_BACKEND", mode))
    if mode == "acp":
        config["env"].extend([
            ("TESHI_ACP_PROGRAM", sys.executable),
            ("TESHI_ACP_ARGS_JSON", json.dumps([str(fixture), "acp", str(root)])),
        ])
        return None
    provider = subprocess.Popen([sys.executable, str(fixture), "native", str(root)],
                                cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.monotonic() + 10
    port_file = root / "agent-evidence/native-port"
    while not port_file.is_file():
        if provider.poll() is not None or time.monotonic() >= deadline:
            stop_fixture_process(provider)
            raise ValueError("native project fixture did not become ready")
        time.sleep(0.01)
    config["env"].extend([
        ("NO_PROXY", "127.0.0.1,localhost,::1"),
        ("TESHI_LLM_API_KEY", "isolated-fixture-key"),
        ("TESHI_LLM_BASE_URL", f"http://127.0.0.1:{port_file.read_text()}/v1"),
        ("TESHI_LLM_MODEL", "project-fixture"),
    ])
    return provider


def stop_fixture_process(process):
    """Stop only an explicitly owned fixture handle."""
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def verify_agent_shutdown(root: Path, mode: str) -> dict:
    trace_path = root / "agent-evidence/trace.jsonl"
    events = [json.loads(line) for line in trace_path.read_text(encoding="utf-8").splitlines()]
    prompts = [event for event in events if event["kind"] == f"{mode}_prompt"]
    a = next(event for event in prompts if event["project"] == "project-a")
    b = next(event for event in prompts if event["project"] == "project-b")
    if "B_PROJECT_ONLY_REQUEST" not in b["text"] or "A_PROJECT_ONLY_REQUEST" in b["text"]:
        raise ValueError("B received A's conversation context or lost B's own request")
    if mode == "native":
        if not b["a_stream_closed"]:
            raise ValueError("the stalled A transport was still open when B replied")
        return {"mode": mode, "old_stream_closed": True, "b_context_isolated": True}
    stopped = next(event for event in events if event["kind"] == "acp_exit" and event["pid"] == a["pid"])
    session = next(event for event in events if event["kind"] == "acp_session" and event["pid"] == b["pid"])
    if stopped["at"] >= session["at"] or process_running(a["pid"]):
        raise ValueError("B activated before A's ACP process exited")
    for prompt in (a, b):
        start = next(event for event in events if event["kind"] == "acp_start" and event["pid"] == prompt["pid"])
        if Path(start["cwd"]).resolve() != (root / prompt["project"]).resolve():
            raise ValueError("ACP process cwd belongs to another project")
    writes = root / "project-a/agent-writes.txt"
    before = writes.read_bytes()
    time.sleep(0.2)
    if not before or writes.read_bytes() != before:
        raise ValueError("A's ACP task continued writing after B activation")
    if not any(event["kind"] == "acp_late_attempt" for event in events):
        raise ValueError("fixture did not exercise a late ACP update")
    return {"mode": mode, "a_pid": a["pid"], "b_pid": b["pid"], "a_process_exited": True,
            "writes_stopped": True, "b_context_isolated": True, "cwd_isolated": True,
            "shutdown_before_b_session": True, "late_update_attempted": True}


def verify_runner_shutdown(root: Path, evidence_path: Path | None = None) -> dict:
    """Check stable A writes and B's actual process cwd before fixture cleanup."""
    evidence = {"schema_version": 2, "process_probes": {}}
    try:
        a = root / "project-a/runner-evidence"
        b = root / "project-b/runner-evidence"
        records = {name: json.loads(path.read_text(encoding="utf-8")) for name, path in (
            ("a_runner", a / "project-a-runner.json"),
            ("a_child", a / "project-a-child.json"),
            ("b_runner", b / "project-b-runner.json"),
        )}
        evidence["records"] = records
        writes = [a / "project-a-runner.writes", a / "project-a-child.writes"]
        before = [path.read_bytes() for path in writes]
        time.sleep(0.3)
        after = [path.read_bytes() for path in writes]
        evidence.update(a_write_counts=[len(data.splitlines()) for data in before],
                        a_write_counts_after=[len(data.splitlines()) for data in after],
                        a_write_sha256=[hashlib.sha256(data).hexdigest() for data in before],
                        a_write_sha256_after=[hashlib.sha256(data).hexdigest() for data in after])
        if before != after or not all(before):
            raise ValueError("project A runner tree was not quiescent after project B activation")
        evidence["writes_stopped"] = True
        for name, project in (("a_runner", "project-a"), ("a_child", "project-a"), ("b_runner", "project-b")):
            if Path(records[name]["cwd"]).resolve() != (root / project).resolve():
                raise ValueError(f"{name} ran in another project's working directory")
        if list(b.glob("project-a*")):
            raise ValueError("project A runner wrote into project B")
        evidence["cwd_isolated"] = True
        alive = []
        for name in ("a_runner", "a_child"):
            probe = evidence["process_probes"].setdefault(name, {})
            identity = records[name].get("process_identity")
            if not isinstance(identity, dict):
                probe.update(state="unknown", error="missing recorded process creation identity")
                raise ValueError(f"{name} has no recorded process creation identity")
            alive.append(process_running(records[name]["pid"], identity, probe))
        if any(alive):
            raise ValueError("project A runner process is still alive after project B activation")
        evidence["a_processes_exited"] = True
        return evidence
    except (OSError, ValueError, KeyError) as error:
        evidence["error"] = str(error)
        raise
    finally:
        if evidence_path is not None:
            evidence_path.parent.mkdir(parents=True, exist_ok=True)
            evidence_path.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")


def fixture_feature_paths(project_root: Path, requested_path: str) -> tuple[Path, Path]:
    """Keep an absolute CLI source path from overriding the fixture root."""
    original = (project_root / requested_path).resolve()
    relative = original.relative_to(project_root.resolve())
    return original, relative


def configure_session_fixture(root: Path, mode: str) -> dict:
    """Seed only the temporary session store and shared requirement library."""
    sessions = root / "sessions"
    sessions.mkdir()
    for name, label, owner in (
        ("session-a", "SESSION_A", root / "project-a"),
        ("session-b", "SESSION_B", root / "project-b"),
        ("legacy", "LEGACY_HISTORY", None),
    ):
        message = {"role": "Assistant", "content": label + "_REPLY"}
        session = {
            "id": name, "created_at": "1800000000000", "updated_at": "1800000000000",
            "model_label": label, "message_count": 1, "messages": [message],
            "backend_kind": "native", "project_root": str(owner) if owner else None,
        }
        (sessions / f"{name}.json").write_text(json.dumps(session), encoding="utf-8")
    if mode != "links":
        return {}
    store = root / "requirements"
    store_id = "reqstore-project-switch-fixture"
    body = "# Shared requirement\n\nGlobal verification intent.\n"
    (store / "shared.md").write_text(body, encoding="utf-8")
    (store / "_teshi.json").write_text(json.dumps({
        "version": 2, "store_id": store_id, "documents": [{
            "id": "shared", "path": "shared.md", "title": "Shared requirement",
            "revision": "fixture-revision",
        }],
    }), encoding="utf-8")
    files = [store / "shared.md", store / "_teshi.json"]
    for project, title in (("project-a", "POINT_A_ONLY"), ("project-b", "POINT_B_ONLY")):
        path = root / project / "testpoints/testpoints.json"
        path.parent.mkdir()
        path.write_text(json.dumps({"version": 1, "test_points": [{
            "id": title, "title": title, "objective": "Verify the shared intent",
            "hierarchy_path": ["Shared"], "review_state": "proposed",
            "requirement_links": [{
                "store_id": store_id, "document_id": "shared",
                "document_revision": "fixture-revision",
                "position": {"start": 2, "end": 20},
                "quote": {"quote": "Shared requirement"}, "resolution": "resolved",
            }],
        }]}), encoding="utf-8")
        files.append(path)
    return {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest() for path in files}


def verify_session_fixture(root: Path, mode: str, before: dict) -> dict:
    sessions = {}
    for path in sorted((root / "sessions").glob("*.json")):
        session = json.loads(path.read_text(encoding="utf-8"))
        session_id = session["id"]
        if session_id not in {"session-a", "session-b", "legacy"}:
            raise ValueError(f"unexpected conversation snapshot: {session_id}")
        expected_owner = {"session-a": root / "project-a", "session-b": root / "project-b", "legacy": None}[session_id]
        owner = Path(session["project_root"]) if session.get("project_root") else None
        if owner != expected_owner:
            raise ValueError(f"session ownership changed: {session_id}")
        expected_reply = {"session-a": "SESSION_A_REPLY", "session-b": "SESSION_B_REPLY", "legacy": "LEGACY_HISTORY_REPLY"}[session_id]
        if [message["content"] for message in session["messages"]] != [expected_reply]:
            raise ValueError(f"conversation context leaked: {session_id}")
        sessions[session_id] = session
    if len(sessions) != 3:
        raise ValueError("session fixture history was lost")
    after = {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in before}
    if before != after:
        raise ValueError("project switching mutated the global requirement or local links")
    return {"mode": mode, "sessions": sessions, "source_files_before": before,
            "source_files_after": after, "ownership_preserved": True}


def main() -> int:
    request = json.loads(sys.stdin.readline())
    if os.environ.get("TESHI_FEATURE_ARTIFACTS"):
        request.setdefault("meta", {})["artifacts_dir"] = os.environ["TESHI_FEATURE_ARTIFACTS"]
    cases = request.get("cases", [])
    if not cases:
        return 2
    original_root = Path(request["meta"]["project_root"])
    original_feature, feature_path = fixture_feature_paths(original_root, cases[0]["feature_path"])
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
            sidecar = make_sidecar(
                source, canonical_source, temp_root, teshi, key_name,
                template_path=original_feature.with_suffix(".tui.template.json"),
            )
            agent_mode = AGENT_SWITCH_SCENARIOS.get(case["scenario"])
            provider = configure_agent_fixture(sidecar, temp_root, agent_mode) if agent_mode else None
            session_mode = SESSION_SWITCH_SCENARIOS.get(case["scenario"])
            session_sources = configure_session_fixture(temp_root, session_mode) if session_mode else None
            target_feature.with_suffix(".tui.json").write_text(
                json.dumps(sidecar, ensure_ascii=False, indent=2), encoding="utf-8"
            )
            child_request = dict(request)
            child_request["cases"] = [{**case, "feature_path": feature_path.as_posix()}]
            child_request["meta"] = dict(request.get("meta", {}))
            child_request["meta"]["project_root"] = str(temp_root)
            child_request["meta"]["artifacts_dir"] = str(temp_root / "artifacts")
            original_files = project_fixture_files(temp_root) if key_name == "tui_project_switch.feature" else None
            unrelated = None
            if case["scenario"] in RUNNER_SHUTDOWN_SCENARIOS:
                unrelated = subprocess.Popen([
                    sys.executable, str(ROOT / "crates/teshi-tui-runner/tests/fixtures/project_switch_runner.py"),
                    "child", "unrelated", str(temp_root / "unrelated-evidence"),
                ], cwd=temp_root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                completed = subprocess.run(
                    [generic],
                    input=json.dumps(child_request, ensure_ascii=False) + "\n",
                    text=True,
                    encoding="utf-8",
                    errors="replace",
                    capture_output=True,
                )
            except BaseException:
                stop_fixture_process(provider)
                if unrelated is not None:
                    unrelated.kill()
                    unrelated.wait()
                raise
            file_error = None
            if original_files is not None:
                try:
                    current_files = project_fixture_files(temp_root)
                    if current_files != original_files:
                        file_error = "project switching changed fixture source files without Save"
                    if (temp_root / "missing-project").exists():
                        file_error = "opening a missing project unexpectedly created its directory"
                except OSError as error:
                    current_files = {}
                    file_error = f"project switching lost a fixture source file: {error}"
                runner_evidence = None
                if session_mode:
                    try:
                        session_evidence = verify_session_fixture(temp_root, session_mode, session_sources)
                    except (OSError, ValueError, KeyError) as error:
                        file_error = f"session/association verification failed: {error}"
                        session_evidence = {"mode": session_mode, "error": file_error}
                    session_path = temp_root / "artifacts" / re.sub(r"[^A-Za-z0-9_-]", "_", case["id"]) / "session-isolation.json"
                    session_path.parent.mkdir(parents=True, exist_ok=True)
                    session_path.write_text(json.dumps(session_evidence, indent=2) + "\n", encoding="utf-8")
                if agent_mode:
                    try:
                        agent_evidence = verify_agent_shutdown(temp_root, agent_mode)
                    except (OSError, ValueError, StopIteration) as error:
                        file_error = f"Agent shutdown verification failed: {error}"
                        agent_evidence = {"mode": agent_mode, "error": file_error}
                    agent_path = temp_root / "artifacts" / re.sub(r"[^A-Za-z0-9_-]", "_", case["id"]) / "agent-shutdown.json"
                    agent_path.parent.mkdir(parents=True, exist_ok=True)
                    agent_path.write_text(json.dumps(agent_evidence, indent=2) + "\n", encoding="utf-8")
                    shutil.copyfile(temp_root / "agent-evidence/trace.jsonl", agent_path.with_name("agent-trace.jsonl"))
                stop_fixture_process(provider)
                if case["scenario"] in RUNNER_SHUTDOWN_SCENARIOS:
                    runner_path = temp_root / "artifacts" / re.sub(r"[^A-Za-z0-9_-]", "_", case["id"]) / "runner-shutdown.json"
                    try:
                        runner_evidence = verify_runner_shutdown(temp_root, runner_path)
                        if unrelated is None or unrelated.poll() is not None:
                            raise ValueError("stopping the runner also stopped an unrelated process")
                        runner_evidence["unrelated_process_survived"] = True
                    except (OSError, ValueError, KeyError) as error:
                        file_error = f"runner shutdown verification failed: {error}"
                    # Retain raw ownership and write evidence even if verification
                    # failed. A PID-only error cannot be diagnosed after cleanup.
                    raw_evidence = runner_path.parent / "runner-fixture"
                    for project_name, roles in (("project-a", ("runner", "child")), ("project-b", ("runner",))):
                        for role in roles:
                            for extension in ("json", "writes"):
                                source = temp_root / project_name / "runner-evidence" / f"{project_name}-{role}.{extension}"
                                if source.is_file():
                                    raw_evidence.mkdir(parents=True, exist_ok=True)
                                    shutil.copyfile(source, raw_evidence / source.name)
                # Fixture-only cooperative cleanup also works after a failed
                # baseline replay; never target unrelated Python processes.
                for project_name in ("project-a", "project-b"):
                    evidence_root = temp_root / project_name / "runner-evidence"
                    if evidence_root.is_dir():
                        (evidence_root / "cleanup").touch()
                if unrelated is not None:
                    (temp_root / "unrelated-evidence/cleanup").touch()
                    try:
                        unrelated.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        unrelated.kill()
                        unrelated.wait()
                if runner_evidence is not None:
                    runner_path = temp_root / "artifacts" / re.sub(r"[^A-Za-z0-9_-]", "_", case["id"]) / "runner-shutdown.json"
                    runner_path.parent.mkdir(parents=True, exist_ok=True)
                    runner_path.write_text(json.dumps(runner_evidence, indent=2) + "\n", encoding="utf-8")
                file_evidence = temp_root / "artifacts" / re.sub(r"[^A-Za-z0-9_-]", "_", case["id"]) / "fixture-files.json"
                file_evidence.parent.mkdir(parents=True, exist_ok=True)
                file_evidence.write_text(json.dumps({
                    "before": original_files, "after": current_files,
                    "unchanged": file_error is None, "error": file_error,
                }, indent=2) + "\n", encoding="utf-8")
            child_events = []
            for line in completed.stdout.splitlines():
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if event.get("type") == "case_passed" and file_error:
                    event = {
                        "type": "case_failed", "case_id": case["id"],
                        "duration_ms": event.get("duration_ms"),
                        "error": {"message": file_error, "attachments": []},
                    }
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
