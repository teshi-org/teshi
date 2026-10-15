"""Real-PTY ACP authoring acceptance for the paired Feature files."""
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
    def __init__(self, request):
        self.temp = tempfile.TemporaryDirectory(prefix="teshi-acp-authoring-")
        self.root = Path(self.temp.name)
        self.store = self.root / "acp"
        self.project = self.root / "project"
        self.requirements = self.root / "requirements"
        self.store.mkdir()
        self.project.mkdir()
        self.requirements.mkdir()
        fixture = Path(__file__).resolve().parents[4] / "crates" / "teshi-agent-runtime" / "tests" / "fixtures" / "fake_acp_agent.py"
        (self.store / "installed.json").write_text(json.dumps([{
            "id": "fixture-authoring",
            "name": "Fixture Authoring",
            "program": sys.executable,
            "args": [str(fixture), "mcp_review"],
            "env": {},
        }]), encoding="utf-8")
        self.host = subprocess.Popen(
            [request["host"], "--terminal-host"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            encoding="utf-8",
        )
        self.binary = os.environ["TESHI_SELF_TEST_BIN"]
        self.input_focused = False

    def op(self, op, text="", **values):
        self.host.stdin.write(json.dumps(dict(op=op, text=text, **values)) + "\n")
        self.host.stdin.flush()
        reply = json.loads(self.host.stdout.readline())
        if not reply["ok"]:
            raise AssertionError(reply["error"])
        return reply.get("value")

    def screen(self):
        return self.op("screen")

    def expect(self, text, timeout=30):
        deadline = time.monotonic() + timeout
        last = ""
        while time.monotonic() < deadline:
            last = self.screen()
            normalized = re.sub(r"\s+", " ", last.replace("│", " "))
            if text in last or text in normalized:
                return last
            time.sleep(0.05)
        raise AssertionError(f"Missing {text!r}\n{last}")

    def expect_absent(self, text, timeout=30):
        deadline = time.monotonic() + timeout
        last = ""
        while time.monotonic() < deadline:
            last = self.screen()
            normalized = re.sub(r"\s+", " ", last.replace("│", " "))
            if text not in last and text not in normalized:
                return last
            time.sleep(0.05)
        raise AssertionError(f"Unexpected {text!r}\n{last}")

    def launch(self):
        env = [
            ("APPDATA", str(self.root / "appdata")),
            ("LOCALAPPDATA", str(self.root / "localappdata")),
            ("XDG_CONFIG_HOME", str(self.root / "config")),
            ("XDG_DATA_HOME", str(self.root / "data-home")),
            ("TESHI_ACP_STORE_DIR", str(self.store)),
            ("TESHI_APP_DATA_DIR", str(self.root / "app-data")),
        ]
        self.op("launch", config={
            "program": self.binary,
            "args": [str(self.project), "--requirements-root", str(self.requirements)],
            "cwd": str(self.root),
            "env": env,
            "cols": 140,
            "rows": 40,
        })
        self.expect("Explore")

    def open_ai_and_select_fixture(self):
        self.op("key", "3")
        self.expect("Welcome to AI Chat!")
        self.op("key", "Enter")
        self.input_focused = True
        self.op("write", "/agent")
        self.expect("/agent")
        self.op("key", "Enter")
        self.expect("Fixture Authoring")
        # The ACP selector always puts Native first.
        self.op("key", "Down")
        self.op("key", "Enter")
        time.sleep(0.2)

    def send_prompt(self):
        if not self.input_focused:
            self.op("key", "Enter")
            self.input_focused = True
        self.op("write", "Generate a login feature from the requirements")
        self.op("key", "Enter")
        self.input_focused = True

    def step(self, text):
        if text == "Teshi is open with a deterministic ACP authoring agent":
            self.launch()
        elif text == "I open AI chat for ACP authoring":
            self.open_ai_and_select_fixture()
        elif text == "I send ACP authoring requirements":
            self.send_prompt()
        elif text == "the ACP authoring review is visible":
            self.expect("Reviewing Test Points")
            self.expect("Proposed")
            self.op("key", "Escape")
            self.input_focused = False
            self.op("key", "5")
            self.expect("Test Points")
            self.expect("Valid login")
        elif text == "I approve the proposed test point":
            self.op("write", "a")
            self.expect("Approved")
        elif text == "I continue ACP generation":
            self.op("write", "c")
            self.op("key", "3")
            self.expect("Waiting for approval", timeout=45)
        elif text == "ACP requests the generated Feature file":
            self.expect("Waiting for approval")
        elif text == "I accept the generated Feature file":
            self.op("write", "y")
            self.expect("Scenario: Valid login", timeout=45)
        elif text == "ACP requests the scenario insertion":
            self.expect("Scenario: Valid login")
        elif text == "I accept the scenario insertion":
            self.op("write", "y")
            self.expect_absent("Pending Change", timeout=45)
        elif text == "the generated Feature is visible in the editor":
            self.op("key", "1")
            self.expect("Valid login")
            self.expect("Then access is granted")
        elif text == "I save the generated Feature":
            self.op("key", "s")
        elif text == "the generated Feature is saved to disk":
            path = self.project / "approved.feature"
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                if path.exists() and "Scenario: Valid login" in path.read_text(encoding="utf-8") and "Then access is granted" in path.read_text(encoding="utf-8"):
                    return
                time.sleep(0.05)
            raise AssertionError(f"saved Feature is missing or incomplete: {path}")
        elif text == "I leave ACP authoring input":
            self.op("key", "Escape")
            self.input_focused = False
        elif text == "I quit ACP authoring":
            self.op("key", "q")
        elif text == "ACP authoring exits successfully":
            self.op("exit", code=0, timeout_ms=15_000)
        else:
            raise AssertionError("Unbound step: " + text)

    def close(self):
        try:
            self.op("close")
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
        world = World(request)
        errors = []
        try:
            for step in case["steps"]:
                text = step["canonical"]
                emit("start_step", case_id=case["id"], step_id=str(step["line"]), text=step["text"])
                world.step(text)
                emit("end_step", case_id=case["id"], step_id=str(step["line"]), status="passed")
        except Exception as error:
            artifacts = Path(request["meta"].get("artifacts_dir", ".teshi/tui-results/acp-authoring"))
            artifacts.mkdir(parents=True, exist_ok=True)
            screen = artifacts / (re.sub(r"[^A-Za-z0-9_-]", "_", str(case["id"])) + ".txt")
            try:
                screen.write_text(world.screen(), encoding="utf-8")
            except Exception:
                pass
            errors.append(str(error) + f"; screen={screen}")
            emit("end_step", case_id=case["id"], step_id=str(step["line"]), status="failed", message=str(error))
        finally:
            world.close()
        if errors:
            failed += 1
            emit("case_failed", case_id=case["id"], duration_ms=0, error={"message": "; ".join(errors)})
        else:
            emit("case_passed", case_id=case["id"], duration_ms=0)
    emit("end_run", passed=len(request["cases"]) - failed, failed=failed, skipped=0)
    return bool(failed)


if __name__ == "__main__":
    sys.exit(main())
