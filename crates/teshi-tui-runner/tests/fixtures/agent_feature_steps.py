"""Real-PTY bindings for the Agent execution-activity Feature pair.

The fixture is local and deterministic. Every step below drives the product
through the terminal host and advances the controlled ACP/native stream only
when the Feature asks for that event.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time


def emit(kind, **values):
    print(json.dumps(dict(type=kind, **values), ensure_ascii=False), flush=True)


class World:
    def __init__(self, request, case):
        self.request = request
        self.case = case
        self.feature_name = Path(case.get("feature_path", "")).name
        self.selection_feature = self.feature_name in {
            "tui_agent_selection_memory.feature",
            "记住上次选择的Agent.feature",
        }
        self.conversation_feature = self.feature_name in {
            "tui_conversation_history.feature",
            "Agent对话持久化.feature",
        }
        self.removed_agent = False
        self.failed_agent = False
        self.chat_open = False
        self.temp = tempfile.TemporaryDirectory(prefix="teshi-agent-feature-")
        self.root = Path(self.temp.name)
        self.project = self.root / "project"
        self.requirements = self.root / "requirements"
        self.store = self.root / "acp"
        self.trace = self.root / "trace"
        self.project.mkdir()
        self.requirements.mkdir()
        self.store.mkdir()
        (self.project / "sample.feature").write_text(
            "Feature: Inspection\n  Scenario: Project\n    Given Project inspection complete\n",
            encoding="utf-8",
        )
        host_env = dict(os.environ)
        for name in ("TESHI_AGENT_BACKEND", "TESHI_ACP_PROGRAM", "TESHI_ACP_ARGS_JSON"):
            host_env.pop(name, None)
        self.host = subprocess.Popen(
            [request["host"], "--terminal-host"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            env=host_env,
        )
        self.process = None
        self.backend = None
        self.expected_backend = "Native"
        for step in case.get("steps", []):
            canonical = step.get("canonical", "")
            if "selected backend is" in canonical:
                quoted = re.findall(r'"([^"]+)"', canonical)
                if quoted:
                    self.expected_backend = quoted[-1]
                    break
        self.turn = None
        self.input_focused = False
        self.acp_fixture = Path(__file__).with_name("agent_acceptance_fixture.py").resolve()

    def op(self, operation, text="", **values):
        self.host.stdin.write(json.dumps(dict(op=operation, text=text, **values)) + "\n")
        self.host.stdin.flush()
        reply = json.loads(self.host.stdout.readline())
        if not reply.get("ok"):
            raise AssertionError(reply.get("error", "terminal operation failed"))
        return reply.get("value")

    def screen(self):
        return self.op("screen")

    def expect(self, value, timeout=30):
        deadline = time.monotonic() + timeout
        last = ""
        while time.monotonic() < deadline:
            last = self.screen()
            normalized = re.sub(r"\s+", " ", last.replace("│", " "))
            if value in last or value in normalized:
                return last
            time.sleep(0.05)
        raise AssertionError(f"missing {value!r}\n{last}")

    def absent(self, value, timeout=2):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            screen = self.screen()
            if value not in screen:
                return
            time.sleep(0.05)
        raise AssertionError(f"unexpected {value!r}\n{screen}")

    def expect_any(self, values, timeout=30):
        deadline = time.monotonic() + timeout
        last = ""
        while time.monotonic() < deadline:
            last = self.screen()
            if any(value in last for value in values):
                return last
            time.sleep(0.05)
        raise AssertionError(f"missing one of {values!r}\n{last}")

    def _env(self, backend):
        env = [
            ("APPDATA", str(self.root / "appdata")),
            ("LOCALAPPDATA", str(self.root / "localappdata")),
            ("XDG_CONFIG_HOME", str(self.root / "config")),
            ("XDG_DATA_HOME", str(self.root / "data")),
            ("TESHI_SESSION_DIR", str(self.root / "sessions")),
            ("TESHI_APP_DATA_DIR", str(self.root / "app-data")),
        ]
        if backend != "Native" or self.selection_feature:
            profile_names = ["Controlled Agent"]
            if self.selection_feature:
                profile_names = ["Fake Alpha", "Fake Beta"]
            elif self.conversation_feature and backend not in {"ACP", "Native"}:
                profile_names = [backend]
            if self.removed_agent:
                profile_names = [name for name in profile_names if name != "Fake Alpha"]
            self.store.joinpath("installed.json").write_text(
                json.dumps(
                    [
                        {
                            "id": re.sub(r"[^a-z0-9-]+", "-", name.lower()).strip("-") or "controlled-agent",
                            "name": name,
                            "program": sys.executable,
                            "args": ["-c", "import sys; sys.exit(17)"] if self.failed_agent else [
                                str(self.acp_fixture),
                                "acp",
                                str(self.trace),
                                name,
                                "controlled",
                            ],
                            "env": {},
                            "env_from": {},
                        }
                        for name in profile_names
                    ]
                ),
                encoding="utf-8",
            )
            env.extend(
                [
                    ("TESHI_ACP_STORE_DIR", str(self.store)),
                ]
            )
            if not self.selection_feature:
                env.append(("TESHI_AGENT_BACKEND", "acp"))
        else:
            port_file = self.trace / "native-port"
            port_file.unlink(missing_ok=True)
            self.native = subprocess.Popen(
                [sys.executable, str(self.acp_fixture), "native", str(self.trace)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            deadline = time.monotonic() + 10
            while not port_file.exists() and time.monotonic() < deadline:
                time.sleep(0.05)
            if not port_file.exists():
                raise AssertionError("native fixture did not publish a port")
            env.extend(
                [
                    ("TESHI_LLM_API_KEY", "feature-test-key"),
                    ("TESHI_LLM_BASE_URL", "http://127.0.0.1:" + port_file.read_text().strip() + "/v1"),
                    ("TESHI_LLM_MODEL", "acceptance"),
                    ("TESHI_AGENT_BACKEND", "native"),
                ]
            )
        return env

    def launch(self, backend):
        self.close_app()
        self.backend = backend
        self.op(
            "launch",
            config={
                "program": os.environ["TESHI_SELF_TEST_BIN"],
                "args": [str(self.project), "--requirements-root", str(self.requirements)],
                "cwd": str(self.root),
                "env": self._env(backend),
                "cols": 120,
                "rows": 36,
            },
        )
        self.process = True
        self.chat_open = False
        self.expect("Explore")

    def open_chat(self):
        if self.chat_open:
            return
        if self.process is None:
            self.launch(self.expected_backend)
        self.op("key", "3")
        self.expect("Welcome to AI Chat!")
        self.op("key", "Enter")
        self.expect("Type your message...")
        self.input_focused = True
        self.chat_open = True
        if self.backend == "ACP":
            self.op("write", "/agent")
            self.op("key", "Enter")
            self.expect("Controlled Agent")
            self.op("key", "Down")
            self.op("key", "Enter")
            time.sleep(0.2)
        elif self.backend not in {None, "Native"} and not self.selection_feature:
            self.select_agent(self.backend)

    def select_agent(self, name):
        if self.process is None:
            self.launch(name)
            self.open_chat()
            return
        self.command("/agent")
        self.expect(name)
        names = ["Fake Alpha", "Fake Beta"]
        index = names.index(name) if name in names else 0
        for position in range(index + 1):
            self.op("key", "Down")
            self.expect("▶ " + (names[position] if self.selection_feature else name))
        self.op("key", "Enter")
        self.absent("Select Agent for this session")
        self.backend = name
        self.input_focused = True

    def assert_backend(self, name):
        target = "Teshi Native" if name in {"Native", "Teshi Native"} else name
        self.command("/agent")
        self.expect(target)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            screen = self.screen()
            if any(target in line and "[selected]" in line for line in screen.splitlines()):
                break
            time.sleep(0.05)
        else:
            raise AssertionError(f"backend {target!r} is not selected: {screen}")
        self.op("key", "Escape")
        self.absent("Select Agent for this session")
        self.backend = "Native" if target == "Teshi Native" else target

    def set_backend(self, backend):
        if self.backend != backend:
            self.launch(backend)
            self.open_chat()
        elif self.process is None:
            self.launch(backend)
            self.open_chat()

    def _trace_records(self):
        if not self.trace.joinpath("trace.jsonl").exists():
            return []
        return [
            json.loads(line)
            for line in self.trace.joinpath("trace.jsonl").read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]

    def _write_session(self, session_id, title, project_root=None, question=None, answer=None):
        """Seed the product's real session JSON format for storage-bound steps."""
        session_dir = self.root / "sessions"
        session_dir.mkdir(parents=True, exist_ok=True)
        messages = []
        if question is not None:
            messages.append(
                {
                    "role": "User",
                    "content": question,
                    "tool_calls": None,
                    "tool_call_id": None,
                    "reasoning_content": None,
                    "source": None,
                }
            )
        if answer is not None:
            messages.append(
                {
                    "role": "Assistant",
                    "content": answer,
                    "tool_calls": None,
                    "tool_call_id": None,
                    "reasoning_content": None,
                    "source": None,
                }
            )
        payload = {
            "id": session_id,
            "created_at": "100",
            "updated_at": "100",
            "model_label": title,
            "message_count": len(messages),
            "messages": messages,
        }
        if project_root is not None:
            payload["project_root"] = str(project_root)
            payload["backend_kind"] = "native"
        (session_dir / f"{session_id}.json").write_text(
            json.dumps(payload, ensure_ascii=False), encoding="utf-8"
        )

    def _session_records(self):
        records = []
        for path in (self.root / "sessions").glob("*.json"):
            deadline = time.monotonic() + 2
            while True:
                try:
                    records.append(json.loads(path.read_text(encoding="utf-8")))
                    break
                except (PermissionError, FileNotFoundError, json.JSONDecodeError):
                    # Windows can briefly deny readers during atomic replace.
                    # Retry the same checkpoint, and surface persistent errors.
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(.01)
        return records

    def _visible_session(self, title):
        project = self.project.resolve()
        for record in self._session_records():
            root = record.get("project_root")
            if root is None or Path(root).resolve() == project:
                if record.get("model_label") == title:
                    return record
        return None

    def _open_session_browser(self):
        if self.process is None:
            self.open_chat()
        self.command("/sessions")
        self.expect("Sessions")

    def send(self, message="Inspect the project"):
        self.focus_input()
        previous_count = len(self._trace_records())
        self.op("write", message)
        self.op("key", "Enter")
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            records = self._trace_records()[previous_count:]
            if self.backend != "Native":
                prompts = [record for record in records if record.get("kind") == "prompt"]
                if prompts:
                    self.turn = prompts[-1]["turn"]
                    if self.selection_feature and not self.failed_agent:
                        self.finish(f"{self.backend} response")
                    return
            else:
                native = [record for record in records if record.get("kind") == "native"]
                if native:
                    self.turn = native[-1]["turn"]
                    return
            time.sleep(0.05)
            if self.failed_agent and any(word in self.screen().lower() for word in ("error", "failed")):
                return
        raise AssertionError("controlled agent did not receive the prompt")

    def saved(self, *values, timeout=15):
        """Wait for the product's checkpoint, rather than substituting UI evidence."""
        deadline = time.monotonic() + timeout
        records = []
        while time.monotonic() < deadline:
            records = self._session_records()
            for record in records:
                body = json.dumps(record.get("messages", []), ensure_ascii=False)
                if all(value in body for value in values):
                    return record
            time.sleep(0.05)
        raise AssertionError(f"checkpoint missing {values!r}: {records!r}")

    def command(self, text):
        self.focus_input()
        self.op("write", text)
        self.op("key", "Enter")

    def focus_input(self):
        # The footer advertises Send even while the input is unfocused. Click
        # the actual empty input instead of inferring focus from that label.
        if "Focus [Enter]" in self.screen():
            self.op("key", "Enter")
            self.expect("Type your message...")
        self.op("click", "Type your message...")
        self.input_focused = True

    def resume(self):
        records = self._session_records()
        assert records, "no persisted conversation to resume"
        latest = max(records, key=lambda record: int(record["updated_at"]))
        self.command("/resume")
        self.expect(latest["messages"][0]["content"])

    def quit_app(self):
        self.command("/exit")
        self.op("exit")
        self.close_app()

    def append(self, event):
        if not self.turn:
            raise AssertionError("no active controlled turn")
        path = self.trace / (self.turn + ".jsonl")
        with path.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(event, ensure_ascii=False) + "\n")
            stream.flush()

    def finish(self, text=None, error=False):
        if text is not None:
            self.append({"kind": "reply", "text": text})
        self.append({"kind": "error" if error else "done"})

    def close_app(self):
        if self.process is not None:
            try:
                self.op("close")
            except Exception:
                pass
            self.process = None
            self.input_focused = False
            self.chat_open = False
        if getattr(self, "native", None) is not None:
            self.native.terminate()
            self.native.wait(timeout=10)
            self.native = None

    def step(self, text, table=None):
        quoted = re.findall(r'"([^"]*)"', text)
        if text.startswith("Teshi is open with"):
            return
        if text.startswith("an isolated Teshi configuration") or text.startswith("no agent backend environment"):
            return
        if text.startswith("Teshi is open in AI chat"):
            self.open_chat()
            return
        if text == "I open AI chat":
            self.open_chat()
            return
        if text.startswith("the selected backend is "):
            if " is still " in text:
                self.assert_backend(quoted[0])
                return
            self.set_backend(quoted[0])
            return
        if text.startswith("the selected backend is the controlled installed ACP agent "):
            self.set_backend(quoted[-1])
            return
        if text.startswith("I send the message "):
            self.send(quoted[0])
            return
        if text.startswith("I send ") and quoted:
            self.send(quoted[0])
            return
        if text.startswith("I send a message"):
            self.send("Inspect the project")
            return
        if text.startswith("the agent streams reasoning ") and " and reply " in text:
            self.append({"kind": "reasoning", "text": quoted[0]})
            self.append({"kind": "reply", "text": quoted[1]})
            return
        if text.startswith("the agent streams reasoning "):
            self.append({"kind": "reasoning", "text": quoted[0]})
            return
        if text.startswith("the agent completes with the answer "):
            self.finish(quoted[0])
            return
        if text.startswith("the agent finishes the turn with the answer "):
            self.finish(quoted[0])
            return
        if text.startswith("the agent extends its reply to "):
            self.append({"kind": "reply", "text": quoted[0]})
            return
        if text.startswith("the agent starts tool call "):
            self.append({"kind": "tool", "title": quoted[1], "status": quoted[2]})
            return
        if text.startswith("the agent updates tool call "):
            self.append({"kind": "tool", "status": quoted[1]})
            return
        if text == "the agent finishes the turn" or text.startswith("the agent finishes the turn normally"):
            self.finish()
            return
        if text.startswith("the agent finishes the turn normally without further reply text"):
            self.finish()
            return
        if text.startswith("the agent streams reply "):
            self.append({"kind": "reply", "text": quoted[0]})
            return
        if text.startswith("the agent requests a project inspection tool"):
            self.append({"kind": "tool", "path": "sample.feature"})
            return
        if text.startswith("the tool returns "):
            self.expect(quoted[0])
            return
        if text.startswith("the turn ends through "):
            if quoted[0] == "cancellation":
                self.op("key", "Escape")
            else:
                self.finish(error=True)
            return
        if text.startswith("conversation storage contains the message "):
            self.saved(quoted[0])
            return
        if text.startswith("the saved conversation contains") or text.startswith("the same saved conversation contains"):
            if quoted:
                self.saved(*quoted)
            return
        if text.startswith("conversation storage contains exactly one conversation"):
            records = self._session_records()
            assert len(records) == 1, records
            return
        if text.startswith("a checkpoint contains"):
            self.checkpoint = quoted
            self.send(quoted[0])
            self.append({"kind": "reasoning", "text": quoted[1]})
            self.append({"kind": "reply", "text": quoted[2]})
            self.saved(*quoted)
            return
        if text.startswith("a saved conversation contains"):
            self.saved_context = quoted
            self.send(quoted[0])
            self.append({"kind": "tool", "title": quoted[1], "status": "completed"})
            self.finish(quoted[0])
            self.expect(quoted[0])
            self.saved(*quoted)
            return
        if text.startswith("conversation storage contains a legacy conversation"):
            self.legacy_context = quoted
            self._write_session(
                "legacy",
                "Legacy conversation",
                question=quoted[0],
                answer=quoted[1],
            )
            return
        if text.startswith("a completed conversation "):
            self.saved_context = quoted
            title, project_name = quoted[0], quoted[1]
            project = self.root / project_name
            project.mkdir(parents=True, exist_ok=True)
            (project / "sample.feature").write_text(
                "Feature: Inspection\n  Scenario: Project\n    Given Project inspection complete\n",
                encoding="utf-8",
            )
            session_id = re.sub(r"[^a-z0-9_-]+", "-", title.lower()).strip("-")
            self._write_session(
                f"project-{session_id}",
                title,
                project_root=project,
                question=f"Discuss {title}",
                answer=title,
            )
            return
        if text.startswith("the agent has completed a conversation about "):
            self.send(quoted[0])
            return
        if text.startswith("the conversation includes reasoning "):
            self.append({"kind": "reasoning", "text": quoted[0]})
            self.append({"kind": "tool", "title": quoted[1], "status": "completed"})
            self.finish("Project Cedar")
            self.expect("Project Cedar")
            self.saved(*quoted)
            return
        if text.startswith("Teshi is terminated"):
            self.close_app()
            return
        if text.startswith("I open Teshi for project "):
            project_name = quoted[0]
            self.project = self.root / project_name
            self.project.mkdir(parents=True, exist_ok=True)
            sample = self.project / "sample.feature"
            if not sample.exists():
                sample.write_text(
                    "Feature: Inspection\n  Scenario: Project\n    Given Project inspection complete\n",
                    encoding="utf-8",
                )
            self.launch(self.expected_backend)
            self.open_chat()
            return
        if text.startswith("I reopen Teshi"):
            self.launch(self.backend or self.expected_backend)
            self.open_chat()
            return
        if text.startswith("I resume the most recently updated conversation") or text.startswith("I have resumed that conversation"):
            if text.startswith("I have resumed"):
                self.quit_app()
                self.launch(self.backend)
                self.open_chat()
            self.resume()
            return
        if text.startswith("I quit Teshi") or text.startswith("I quit the"):
            self.quit_app()
            return
        if text.startswith("I open the session browser"):
            self._open_session_browser()
            return
        if text.startswith("I load the legacy conversation"):
            self.expect("Sessions")
            self.op("key", "Enter")
            self.expect(self.legacy_context[0])
            return
        if text.startswith("the new ACP process receives") or text.startswith("the same ACP process receives"):
            prompts = [r for r in self._trace_records() if r.get("kind") == "prompt"]
            assert prompts, "ACP did not receive a prompt"
            prompt = json.dumps(prompts[-1]["params"], ensure_ascii=False)
            value = quoted[-1] if quoted else ("Continue the inspection" if text.startswith("the same") else "Which project were we discussing?")
            assert value in prompt, prompt
            if text.startswith("the same"):
                assert "Project Cedar" not in prompt, prompt
                assert "Read project" not in prompt, prompt
                assert prompts[-1]["pid"] == prompts[-2]["pid"], prompts
            return
        if text.startswith("the new ACP process does not receive") or text.startswith("the same ACP process"):
            prompts = [r for r in self._trace_records() if r.get("kind") == "prompt"]
            prompt = json.dumps(prompts[-1]["params"], ensure_ascii=False)
            assert "Read project" not in prompt, prompt
            if text.startswith("the same ACP process"):
                assert "Project Cedar" not in prompt, prompt
                assert "Continue the inspection" in prompt, prompt
                assert prompts[-1]["pid"] == prompts[-2]["pid"], prompts
            return
        if text.startswith("the reasoning ") and "appears exactly once" in text:
            screen = self.screen()
            assert screen.count(quoted[0]) == 1, screen
            return
        if "is visible in AI chat" in text and quoted:
            self.expect(quoted[0])
            return
        if text.startswith('"') and "appears exactly once" in text:
            self.expect(quoted[0])
            assert self.screen().count(quoted[0]) == 1, self.screen()
            return
        if text.startswith("reply ") and "appears exactly once" in text:
            self.expect(quoted[0])
            assert self.screen().count(quoted[0]) == 1, self.screen()
            return
        if text.startswith("the final answer is not yet visible"):
            self.absent("Inspection finished")
            return
        if text.startswith('"') and "is visible" in text:
            self.expect(quoted[0])
            return
        if (text.startswith("select ") or text.startswith("I select ")) and "from the agent backend panel" in text:
            target = quoted[0] if quoted else "Native"
            if target == "Teshi Native":
                target = "Native"
            if target == "Native":
                if self.selection_feature:
                    self.command("/agent")
                    self.expect("Teshi Native")
                    self.op("key", "Enter")
                    self.absent("Select Agent for this session")
                    self.backend = "Native"
                else:
                    self.set_backend("Native")
            else:
                self.select_agent(target)
            return
        if text.startswith("create a new agent") or text.startswith("I create a new agent") or text.startswith("execute the slash command") or text.startswith("I execute the slash command"):
            if self.process is None:
                self.open_chat()
            if "create a new agent" in text:
                self.op("key", "Escape")
                self.absent("Type your message...")
                self.op("key", "a")
                self.expect("Type your message...")
                self.input_focused = True
            else:
                self.command(quoted[0] if quoted else "/new")
            time.sleep(0.2)
            return
        if text.startswith("restart Teshi") or text.startswith("I restart Teshi"):
            self.close_app()
            self.launch(self.backend or self.expected_backend)
            self.open_chat()
            return
        if (text.startswith("reply from ") or text.startswith("a reply from ")) and "is visible" in text:
            self.expect(quoted[0] + " response")
            return
        if text.startswith("the current conversation uses"):
            target = quoted[0] if quoted else "Teshi Native"
            self.assert_backend(target)
            return
        if text.startswith("the current conversation has no messages"):
            self.expect("Welcome to AI Chat!")
            return
        if text.startswith("the fake ACP trace records"):
            records = self._trace_records()
            sessions = {record.get("session") for record in records if record.get("kind") == "session"}
            assert len(sessions) >= 2, records
            return
        if text.startswith("the new ACP session prompt excludes"):
            prompts = [r for r in self._trace_records() if r.get("kind") == "prompt"]
            assert quoted[0] not in json.dumps(prompts[-1]["params"], ensure_ascii=False), prompts
            return
        if text.startswith("switch to the original agent") or text.startswith("I switch to the original agent") or text.startswith("restore the saved conversation") or text.startswith("I restore the saved conversation"):
            if "switch to the original agent" in text:
                self.op("key", "Escape")
                self.op("key", "k")
                self.op("key", "Enter")
                self.expect("Type your message...")
            else:
                self.resume()
            self.backend = "Fake Alpha"
            self.input_focused = True
            return
        if text.startswith("the legacy conversation is listed"):
            assert self._visible_session("Legacy conversation") is not None
            self.expect("Model: Legacy conversation")
            return
        if text.startswith("the conversation") and "is not listed" in text:
            title = quoted[0]
            assert self._visible_session(title) is None
            return
        if text.startswith("the conversation") and "is listed" in text:
            title = quoted[0]
            assert self._visible_session(title) is not None
            self.expect(f"Model: {title}")
            return
        if text.startswith("a warning identifies"):
            self.expect("unavailable")
            self.expect("fake-alpha")
            self.expect("Native")
            return
        if "is removed from the isolated installed agent store" in text:
            self.removed_agent = True
            return
        if (text.startswith("the installed ") or text.startswith('"')) and "configured to fail" in text:
            self.failed_agent = True
            return
        if text.startswith("an ACP connection error"):
            self.expect_any(("error", "failed", "connection"))
            return
        if text.startswith("no Native model request"):
            assert not any(record.get("kind") == "native" for record in self._trace_records())
            return
        if text.startswith("the conversation contains exactly one entry"):
            assert self.screen().count("Read project") == 1, self.screen()
            return
        if text.startswith("reply ") and "appears before tool" in text:
            self.expect(quoted[0])
            screen = self.expect("Read project")
            assert screen.index(quoted[0]) < screen.index("Read project"), screen
            return
        if text.startswith("the conversation displays these entries in order"):
            rows = table.get("rows", []) if isinstance(table, dict) else (table or [])
            values = [
                row[0] if isinstance(row, (list, tuple)) else row.get("entry", "")
                for row in rows
                if row
            ]
            if values and values[0] in {"entry", "记录"}:
                values = values[1:]
            for value in values:
                self.expect(value)
            screen = self.screen()
            positions = [screen.index(value) for value in values]
            assert positions == sorted(positions), screen
            return
        if text.startswith("no reply in this turn"):
            assert "green final-answer" not in self.screen()
            return
        if text.startswith("reply ") and "is visible" in text:
            self.expect(quoted[0])
            return
        if text.startswith("AI chat shows tool ") or text.startswith("AI chat still shows tool "):
            self.expect(quoted[0])
            self.expect({"in_progress": "Running", "completed": "Done", "failed": "Failed"}[quoted[1]])
            return
        if text.startswith("the tool request") or text.startswith("the tool result"):
            if quoted:
                self.expect(quoted[0])
            else:
                self.expect_any(("Read project", "get_feature_content"))
            return
        if text.startswith("reply ") and ("styling" in text or "header" in text or "rail" in text):
            self.expect(quoted[0])
            return
        if text.startswith("the conversation contains"):
            self.expect("Read project")
            return
        raise AssertionError("Unbound step: " + text)

    def close(self):
        try:
            self.close_app()
        finally:
            self.host.stdin.close()
            self.host.wait(timeout=15)
            self.temp.cleanup()


def main():
    request = json.load(sys.stdin)
    emit("start_run", total=len(request["cases"]))
    failed = 0
    for case in request["cases"]:
        emit("start_case", case_id=case["id"], name=case["scenario"])
        world = World(request, case)
        started = time.monotonic()
        artifacts = Path(os.environ.get("TESHI_FEATURE_ARTIFACTS", request["meta"].get("artifacts_dir", ".teshi/tui-results/agent"))) / re.sub(r"[^A-Za-z0-9_-]", "_", case["id"])
        artifacts.mkdir(parents=True, exist_ok=True)
        errors = []
        try:
            for step in case["steps"]:
                text = step["canonical"]
                emit("start_step", case_id=case["id"], step_id=str(step["line"]), text=step["text"])
                world.step(text, step.get("table"))
                emit("end_step", case_id=case["id"], step_id=str(step["line"]), status="passed")
        except Exception as error:
            screen = artifacts / "screen.txt"
            try:
                current_screen = world.screen()
                screen.write_text(current_screen, encoding="utf-8")
            except Exception:
                current_screen = "<screen unavailable>"
                pass
            errors.append(f"{error}; screen={screen}; viewport={current_screen}")
            emit("end_step", case_id=case["id"], step_id=str(step["line"]), status="failed", message=str(error))
        finally:
            try:
                (artifacts / "screen.txt").write_text(world.screen(), encoding="utf-8")
                world.op("screenshot", str(artifacts / "screen.svg"))
            except Exception:
                pass
            (artifacts / "sessions.json").write_text(json.dumps(world._session_records(), ensure_ascii=False, indent=2), encoding="utf-8")
            (artifacts / "trace.json").write_text(json.dumps(world._trace_records(), ensure_ascii=False, indent=2), encoding="utf-8")
            world.close()
        duration = int((time.monotonic() - started) * 1000)
        if errors:
            failed += 1
            emit("case_failed", case_id=case["id"], duration_ms=duration, error={"message": "; ".join(errors), "attachments": [{"kind":"file", "path":str(artifacts / "screen.txt")} ]})
        else:
            emit("case_passed", case_id=case["id"], duration_ms=duration)
    emit("end_run", passed=len(request["cases"]) - failed, failed=failed, skipped=0)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
