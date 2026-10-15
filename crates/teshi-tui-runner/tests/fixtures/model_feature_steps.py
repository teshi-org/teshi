"""Source-bound ACP model acceptance using the Rust-owned real terminal host."""
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
        self.temp = tempfile.TemporaryDirectory(prefix="teshi-model-acceptance-")
        self.root = Path(self.temp.name)
        self.store = self.root / "acp"
        self.trace_root = self.root / "trace"
        self.project = self.root / "project"
        for path in (self.store, self.trace_root, self.project):
            path.mkdir()
        (self.trace_root / "model-settings").touch()
        fixture = Path(__file__).with_name("model_acp_agent.py")
        profiles = [dict(id=name.lower().replace(" ", "-"), name=name, program=sys.executable,
                         args=[str(fixture), "acp", str(self.trace_root), name], env={})
                    for name in ("Fake Alpha", "Fake Beta")]
        (self.store / "installed.json").write_text(json.dumps(profiles), encoding="utf-8")
        self.host = subprocess.Popen([request["host"], "--terminal-host"], stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, text=True, encoding="utf-8",
                                     env={k: v for k, v in os.environ.items() if k not in
                                          ("TESHI_AGENT_BACKEND", "TESHI_ACP_PROGRAM", "TESHI_ACP_ARGS_JSON")})
        self.binary = os.environ["TESHI_SELF_TEST_BIN"]
        self.selected = None
        self.input = False

    def op(self, op, text="", **values):
        self.host.stdin.write(json.dumps(dict(op=op, text=text, **values)) + "\n")
        self.host.stdin.flush()
        reply = json.loads(self.host.stdout.readline())
        if not reply["ok"]:
            raise AssertionError(reply["error"])
        return reply.get("value")

    def expect(self, text, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            screen = self.op("screen")
            normalized = re.sub(r"\s+", " ", screen.replace("│", " "))
            if text in screen or text in normalized:
                return
            time.sleep(.05)
        raise AssertionError(f"Missing {text!r}\n{screen}")

    def launch(self):
        env = [(key, str(self.root / key.lower())) for key in
               ("APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME")]
        env += [("TESHI_ACP_STORE_DIR", str(self.store)), ("TESHI_APP_DATA_DIR", str(self.root / "data"))]
        self.op("launch", config=dict(program=self.binary, args=[str(self.project), "--requirements-root", str(self.root / "requirements")],
                                      cwd=str(self.root), env=env, cols=140, rows=40))
        self.expect("Explore")
        self.op("key", "3")
        self.expect("Welcome to AI Chat!")
        self.op("key", "Enter")
        self.input = True
        self.discover()

    def discover(self):
        if self.selected is not None:
            deadline = time.monotonic() + 10
            while not any(label in self.op("screen") for label in ("[Connect", "Model:", "[Error")):
                assert time.monotonic() < deadline, "settings row did not appear"
                time.sleep(.05)
        if "[Connect" in self.op("screen"):
            self.op("click", "[Connect")
            # Opening settings discovers the restored session before a prompt.
            deadline = time.monotonic() + 50
            while "Model:" not in self.op("screen") and "[Error" not in self.op("screen"):
                assert time.monotonic() < deadline, "connection deadline exceeded"
                time.sleep(.05)
            self.op("key", "Escape")

    def focus(self):
        if not self.input:
            self.op("key", "Enter")
            self.input = True

    def select(self, agent):
        self.focus()
        self.op("write", "/agent")
        self.expect("/agent")
        self.op("key", "Enter")
        self.expect("Fake Beta")
        self.op("key", "Down")
        if agent == "Fake Beta":
            self.op("key", "Down")
        self.op("key", "Enter")
        self.selected = agent
        self.expect("Model:")

    def model(self, name):
        self.op("click", "Model:")
        self.expect("●")
        self.op("key", "Up")
        if name == "Beta":
            self.op("key", "Down")
        self.op("key", "Enter")
        deadline = time.monotonic() + 15
        while True:
            screen = self.op("screen")
            if ("Model: " + name in screen and "●" not in screen) or "[Error" in screen:
                break
            assert time.monotonic() < deadline, "model change was not confirmed\n" + screen
            time.sleep(.05)

    def records(self):
        path = self.trace_root / "trace.jsonl"
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()] if path.exists() else []

    def rules(self, **rules):
        (self.trace_root / "Fake Alpha-behavior.json").write_text(json.dumps(rules), encoding="utf-8")

    def restart(self):
        self.op("close")
        self.launch()

    def new(self):
        self.op("key", "Escape")
        self.op("key", "a")
        self.input = True
        time.sleep(.1)
        self.discover()
        self.expect("Model:")

    def step(self, text):
        quoted = re.findall(r'"([^"]*)"', text)
        if text.startswith("an isolated Teshi configuration") or text.startswith("both agents advertise") or text == "no agent backend environment override is set":
            return  # The isolated World establishes and owns these prerequisites.
        if text == "Teshi is open in AI chat":
            self.launch()
        elif text.startswith("the original conversation uses"):
            self.select(quoted[0]); self.model(quoted[1]); self.expect("Model: Beta")
        elif text.startswith("the current conversation uses"):
            if self.selected != quoted[0]:
                self.select(quoted[0])
            screen = self.op("screen")
            assert quoted[0] + " ▾" in screen or quoted[0].lower().replace(" ", "-") + " ▾" in screen, screen
        elif text.startswith("I select ") and "agent backend panel" in text:
            self.select(quoted[0])
        elif text.startswith("I select the ACP model"):
            self.model(quoted[0])
        elif text == "the model change is confirmed":
            self.expect("[More")
        elif text.startswith("the confirmed ACP model is"):
            self.discover()
            self.expect("Model: " + quoted[0])
        elif text == "I create a new agent":
            self.new()
        elif text == 'I execute the slash command "/new"':
            self.focus(); self.op("write", "/new"); self.expect("/new"); self.op("key", "Tab"); time.sleep(.1); self.discover(); self.expect("Model:")
        elif text == "I restart Teshi with the same isolated configuration":
            self.restart()
        elif text == "I return to the original conversation":
            self.op("key", "Escape"); self.op("key", "k"); self.input = False
        elif text.startswith("I send ") or text.startswith("I attempt to send "):
            self.focus(); self.op("write", quoted[0]); self.op("key", "Enter")
        elif text.startswith("a reply from"):
            self.expect(quoted[0] + " response")
        elif text.startswith("the ACP trace confirms model"):
            prompts = [r for r in self.records() if r["kind"] == "prompt"]
            assert prompts and prompts[-1]["model"] == {"Alpha": "a", "Beta": "b"}[quoted[0]], prompts
            if "before the first prompt" in text:
                records = self.records(); prompt = prompts[-1]
                assert any(r["kind"] == "configure" and r["session"] == prompt["session"] and r["value"] == "b" for r in records[:records.index(prompt)])
        elif "rejects changes" in text:
            self.rules(reject="a")
        elif text == "the ACP settings report the rejected change":
            self.expect("[Error"); self.op("click", "[Error"); self.expect("setting rejected"); self.op("key", "Escape")
        elif "no longer advertises" in text:
            self.rules(removed=True)
        elif text == "the ACP settings report that the saved model is unavailable":
            self.expect("[Error"); self.op("click", "[Error"); self.expect("no longer available"); self.op("key", "Escape")
        elif "has a saved confirmed model preference" in text:
            self.select("Fake Alpha"); self.model("Beta"); self.expect("Model: Beta")
        elif "times out restoring" in text:
            self.rules(timeout=True)
        elif text == "the ACP settings report a model restoration timeout":
            self.expect("[Error", timeout=50); self.op("click", "[Error"); self.expect("timed out"); self.op("key", "Escape")
        elif text == "the ACP trace contains no prompt for the uncertain session":
            time.sleep(.3)
            assert not [r for r in self.records() if r["kind"] == "prompt"]
        elif "successfully accepts model changes again" in text:
            self.rules()
        elif text == "I reconnect the current ACP conversation":
            self.select("Fake Alpha")
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
        variants = [{}]
        if case["examples"]:
            variants = [dict(zip(table["headers"], row)) for table in case["examples"] for row in table["rows"]]
        errors = []
        for variant_index, variant in enumerate(variants):
            world = World(request)
            try:
                for step in case["steps"]:
                    text = step["canonical"]
                    for key, value in variant.items():
                        text = text.replace("<" + key + ">", value)
                    print(f"binding line {step['line']}: {text}", file=sys.stderr, flush=True)
                    emit("start_step", case_id=case["id"], step_id=str(step["line"]), text=step["text"], example=variant)
                    world.step(text)
                    emit("end_step", case_id=case["id"], step_id=str(step["line"]), status="passed", example=variant)
            except Exception as error:
                artifacts = Path(request["meta"].get("artifacts_dir", ".teshi/tui-results/model"))
                artifacts.mkdir(parents=True, exist_ok=True)
                screen = artifacts / (re.sub(r"[^A-Za-z0-9_-]", "_", str(case["id"])) + "-" + str(len(errors)) + ".txt")
                screen.write_text(world.op("screen"), encoding="utf-8")
                errors.append(f"line {step['line']}: {text}: {error}; screen={screen}")
                emit("end_step", case_id=case["id"], step_id=str(step["line"]), status="failed", message=str(error))
            finally:
                evidence = Path(request["meta"].get("artifacts_dir", ".teshi/tui-results/model"))
                evidence.mkdir(parents=True, exist_ok=True)
                stem = Path(case["feature_path"]).stem + "-" + re.sub(r"[^A-Za-z0-9_-]", "_", str(case["id"])) + "-" + str(variant_index)
                (evidence / (stem + "-trace.json")).write_text(json.dumps(world.records(), indent=2), encoding="utf-8")
                (evidence / (stem + "-screen.txt")).write_text(world.op("screen"), encoding="utf-8")
                world.close()
        if errors:
            failed += 1
            emit("case_failed", case_id=case["id"], duration_ms=0, error={"message": "; ".join(errors)})
        else:
            emit("case_passed", case_id=case["id"], duration_ms=0)
    emit("end_run", passed=len(request["cases"])-failed, failed=failed, skipped=0)
    return bool(failed)


if __name__ == "__main__":
    sys.exit(main())
