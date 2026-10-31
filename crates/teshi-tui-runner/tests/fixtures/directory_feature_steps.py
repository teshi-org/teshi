"""Executable bindings for project-directory loading; every variant owns a real PTY."""
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
import time


EVENT_LOG = None


def emit(kind, **values):
    event = json.dumps(dict(type=kind, **values), ensure_ascii=False)
    print(event, flush=True)
    if EVENT_LOG:
        with EVENT_LOG.open("a", encoding="utf-8") as stream:
            stream.write(event + "\n")


class World:
    def __init__(self, request, evidence):
        self.temp = tempfile.TemporaryDirectory(prefix="teshi-directory-")
        self.root = Path(self.temp.name)
        self.project = self.root / "xxx"
        self.project.mkdir()
        self.features = self.project / "features"
        self.evidence = evidence
        self.locale = None
        self.files = {}
        self.host = subprocess.Popen([request["host"], "--terminal-host"], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, text=True, encoding="utf-8",
            env={key: value for key, value in os.environ.items() if key not in
                ("TESHI_RUNNER_CMD", "TESHI_RUNNER_ARGS", "TESHI_RUNNER_CWD")})
        self.running = False
        self.command = None
        self.session_seed = None

    def op(self, op, text="", **values):
        self.host.stdin.write(json.dumps(dict(op=op, text=text, **values)) + "\n")
        self.host.stdin.flush()
        reply = json.loads(self.host.stdout.readline())
        if not reply["ok"]:
            raise AssertionError(reply["error"])
        return reply.get("value")

    def screen(self):
        return self.op("screen")

    def wait(self, predicate, timeout=15):
        deadline = time.monotonic() + timeout
        screen = ""
        while time.monotonic() < deadline:
            screen = self.screen()
            if predicate(screen):
                return screen
            time.sleep(.05)
        raise AssertionError("Viewport condition timed out\n" + screen)

    def expect(self, text):
        return self.wait(lambda screen: text in screen)

    def write_feature(self, path):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"Feature: {path.stem}\n  Scenario: Ready\n    Given fixture step\n", encoding="utf-8")
        self.files[str(path)] = path.stem

    def launch(self, cwd, command):
        assert command.startswith("teshi"), command
        args = shlex.split(command)[1:]
        binary = os.environ["TESHI_SELF_TEST_BIN"]
        env = [(key, str(self.root / key.lower())) for key in
            ("APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME",
             "TESHI_APP_DATA_DIR", "TESHI_SESSION_DIR", "TESHI_ACP_STORE_DIR")]
        env += [("TESHI_REQUIREMENTS_DIR", str(self.root / "requirements")),
                ("TESHI_AGENT_BACKEND", "native"),
                ("TESHI_FEATURE_LANGUAGE", self.locale or "")]
        args += ["--requirements-root", str(self.root / "requirements")]
        self.op("launch", config=dict(program=binary, args=args, cwd=str(cwd), env=env, cols=170, rows=44))
        self.running = True
        self.command = dict(program=binary, args=args, cwd=str(cwd), locale_override=self.locale,
            binary_sha256=hashlib.sha256(Path(binary).read_bytes()).hexdigest())
        self.expect("Explore")
        self.expect("Project: xxx")

    def capture(self, run_directory, case, step, attempt, timing="after_step"):
        event = dict(case_id=case["id"], step_id=f"{case['id']}:L{step['line']}",
                     attempt=attempt, feature=case["feature_path"], step_line=step["line"],
                     step_text=step["text"], source="pty", capture_timing=timing)
        try:
            stem = re.sub(r"[^A-Za-z0-9-]", "-", f"directory-{case['id']}-L{step['line']}-{attempt}")
            event.update(self.op("capture", directory=str(run_directory), stem=stem),
                         capture_status="available")
        except Exception as error:
            event.update(capture_status="failed", message=str(error))
        return event

    def exact(self, expected):
        expected = set(expected)
        candidates = set(self.files.values())
        # Directory rows start collapsed. Search all Feature paths to compare the
        # discovered scope without depending on expansion state.
        self.search_paths()
        self.wait(lambda screen: all(name in screen for name in expected))
        # Hold the negative assertion across multiple external refresh opportunities.
        deadline = time.monotonic() + 3.2
        while time.monotonic() < deadline:
            screen = self.screen()
            actual = {name for name in candidates if name in screen}
            assert actual == expected, (expected, actual, screen)
            time.sleep(.08)

    def search_paths(self):
        self.op("key", "Escape")
        self.op("key", "/")
        self.op("write", ".feature")
        self.op("key", "Enter")

    def open_sessions(self):
        self.op("key", "3")
        self.expect("Welcome to AI Chat!")
        self.op("key", "Enter")
        self.op("write", "/sessions")
        self.op("key", "Enter")
        self.expect("Project sessions")
        self.expect("DIRECTORY-SAVED-SESSION")
        self.op("key", "Escape")
        self.op("click", "Explore [1]")
        self.search_paths()
        self.expect("登录")
        self.op("click", "登录")
        self.expect("Ready")

    def step(self, text):
        translations = {
            "an isolated launch directory and project xxx": "已准备隔离的启动目录和项目 xxx",
            "the project uses isolated configuration and session data": "隔离项目不使用真实用户的配置和会话数据",
            "the Feature list is empty": "Feature 列表为空",
            "project xxx has no features directory": "项目 xxx 没有 features 目录",
            "project xxx has “根层功能.feature” in its root": "项目 xxx 根目录有“根层功能.feature”",
            "project xxx has “其他功能.feature” in another directory": "项目 xxx 的其他目录中有“其他功能.feature”",
            "the Feature list contains exactly “根层功能”": "Feature 列表恰好包含“根层功能”",
            "the Feature list contains exactly “根层功能” and “其他功能”": "Feature 列表恰好包含“根层功能”和“其他功能”",
        }
        text = translations.get(text, text)
        match = re.fullmatch(r"I launch “([^”]+)” in “([^”]+)”", text)
        if match:
            text = f"我在“{match[2]}”中启动“{match[1]}”"
        quoted = re.findall("“([^”]+)”", text)
        if text == "已准备隔离的启动目录和项目 xxx":
            assert self.project.is_dir() and self.root != self.project
            # Establish the tested product's viewport during fixture setup so
            # every source step has fresh evidence, including Background.
            self.launch(self.project, "teshi")
        elif text == "隔离项目不使用真实用户的配置和会话数据":
            assert self.project.is_relative_to(Path(tempfile.gettempdir()))
        elif text == "项目 xxx 的 features 中有“登录.feature”和“auth/退出.feature”":
            for name in quoted:
                self.write_feature(self.features / name)
        elif text == "项目 xxx 的 features 中有“登录.feature”":
            self.write_feature(self.features / quoted[0])
        elif text in ("项目 xxx 的其他目录中有“外层功能.feature”", "项目 xxx 的其他目录中有“其他功能.feature”", "项目 xxx 的其他深层目录中有“其他功能.feature”"):
            self.write_feature(self.project / "other/deep" / quoted[0])
        elif text == "features 中没有语言子目录" or text == "项目 xxx 的 features 中没有语言子目录":
            self.features.mkdir(exist_ok=True)
            assert not any((self.features / name).exists() for name in ("zh-CN", "en-US"))
        elif text == "项目 xxx 根目录有“根层功能.feature”":
            self.write_feature(self.project / quoted[0])
        elif text == "项目 xxx 没有 features 目录":
            assert not self.features.exists()
        elif text == "项目 xxx 的 features 中同时存在 zh-CN 和 en-US 语言目录":
            for locale in ("zh-CN", "en-US"):
                (self.features / locale).mkdir(parents=True, exist_ok=True)
        elif text == "每个语言目录中都有直属和深层子目录中的 Feature":
            for locale in ("zh-CN", "en-US"):
                for name in (f"{locale}-direct.feature", f"nested/{locale}-deep.feature"):
                    self.write_feature(self.features / locale / name)
        elif text == "features 根层还有一个公共 Feature":
            self.write_feature(self.features / "公共.feature")
        elif text.startswith("系统首选界面语言为"):
            self.locale = quoted[0]
        elif text == "项目 xxx 的 features 中存在与系统界面语言匹配的语言目录":
            self.locale = "zh-CN"
            self.write_feature(self.features / "zh-CN/登录.feature")
        elif text.startswith("项目 xxx 外有"):
            self.write_feature(self.root / quoted[0])
        elif text.startswith("我在启动目录中启动"):
            self.launch(self.root, quoted[0])
        elif text.startswith("我在“") and "中启动“" in text:
            self.launch(self.project if quoted[0] == "项目 xxx" else self.root, quoted[1])
        elif text == "Feature 列表恰好包含“根层功能”":
            self.exact(["根层功能"])
        elif text == "Feature 列表恰好包含“根层功能”和“其他功能”":
            self.exact(["根层功能", "其他功能"])
        elif text == "Feature 列表恰好包含“登录”和“退出”":
            self.exact(["登录", "退出"])
        elif text == "Feature 列表恰好包含“登录”和“其他功能”":
            self.exact(["登录", "其他功能"])
        elif text.startswith("Feature 列表恰好包含") and "目录及其子目录中的 Feature" in text:
            self.exact([f"{quoted[0]}-direct", f"{quoted[0]}-deep"])
        elif text == "活动项目根目录为项目 xxx":
            self.expect("Project: xxx")
            screen = self.screen()
            assert "Project: features" not in screen and "Project: zh-CN" not in screen, screen
        elif text in ("Feature 列表为空", "下一次外部文件检查完成后 Feature 列表仍为空"):
            self.expect("(no features)")
            self.exact([])
        elif text.startswith("我在 zh-CN 的深层子目录中新增"):
            self.write_feature(self.features / "zh-CN/nested" / quoted[0])
        elif text.startswith("我在 en-US 中新增"):
            self.write_feature(self.features / "en-US" / quoted[0])
        elif text.startswith("我在项目 xxx 的 features 之外新增"):
            self.write_feature(self.project / "other" / quoted[0])
        elif text.startswith("我在 features 的深层子目录中新增"):
            self.write_feature(self.features / "deep/nested" / quoted[0])
        elif text.startswith("我在 features 之外新增"):
            self.write_feature(self.project / "other" / quoted[0])
        elif text.startswith("我在项目 xxx 的 features 之外的深层目录新增"):
            self.write_feature(self.project / "other/deep" / quoted[0])
        elif text.startswith("我在项目 xxx 之外新增"):
            self.write_feature(self.root / quoted[0])
        elif text.startswith("下一次外部文件检查完成后可以看到"):
            self.search_paths()
            self.expect(quoted[0])
        elif text.startswith("Feature 列表不包含"):
            deadline = time.monotonic() + 3.2
            while time.monotonic() < deadline:
                screen = self.screen()
                assert all(name not in screen for name in quoted), screen
                time.sleep(.08)
        elif text == "项目 xxx 的配置指定隔离的测试运行工作目录":
            (self.project / "work").mkdir()
            script = str(Path(__file__).resolve())
            config = "[runner]\ncmd = " + json.dumps(sys.executable) + "\nargs = " + json.dumps([script, "--probe-runner", str(self.root / "probe.json")]) + '\ncwd = "work"\n'
            (self.project / "teshi.toml").write_text(config, encoding="utf-8")
        elif text == "项目 xxx 已有保存的项目会话":
            folder = self.root / "teshi_session_dir"
            folder.mkdir()
            payload = dict(id="directory-saved", created_at="100", updated_at="100", model_label="DIRECTORY-SAVED-SESSION", message_count=1,
                project_root=str(self.project), backend_kind="native", messages=[dict(role="User",content="saved directory question",tool_calls=None,tool_call_id=None,reasoning_content=None,source=None)])
            self.session_seed = folder / "directory-saved.json"
            self.session_seed.write_text(json.dumps(payload), encoding="utf-8")
        elif text == "加载项目 xxx 的配置和会话":
            self.open_sessions()
            self.op("key", "r")
            self.wait(lambda screen: (self.root / "probe.json").exists())
        elif text == "测试运行路径按项目 xxx 的配置解析":
            probe = json.loads((self.root / "probe.json").read_text(encoding="utf-8"))
            assert Path(probe["cwd"]).resolve() == (self.project / "work").resolve(), probe
            assert Path(probe["request"]["meta"]["project_root"]).resolve() == self.project.resolve(), probe
            (self.evidence / "runner-probe.json").write_text(json.dumps(probe, indent=2), encoding="utf-8")
        elif text == "项目状态不会写入所选语言目录":
            assert not (self.features / "zh-CN/.teshi").exists()
            assert self.session_seed.exists()
        else:
            raise AssertionError("Unbound step: " + text)

    def close(self):
        try:
            if self.running:
                (self.evidence / "screen.txt").write_text(self.screen(), encoding="utf-8")
                self.op("screenshot", str(self.evidence / "screen.svg"))
                self.op("key", "Escape")
                self.op("click", "Explore [1]")
                self.op("key", "q")
                self.op("exit")
            (self.evidence / "launch.json").write_text(json.dumps(self.command, ensure_ascii=False, indent=2), encoding="utf-8")
        finally:
            self.op("close")
            self.host.stdin.close()
            self.host.wait(timeout=15)
            self.temp.cleanup()


def probe_runner(path):
    request = json.loads(sys.stdin.readline())
    Path(path).write_text(json.dumps(dict(cwd=os.getcwd(), request=request)), encoding="utf-8")
    emit("start_run", total=len(request["cases"]))
    for case in request["cases"]:
        emit("start_case", case_id=case["id"], name=case["scenario"])
        emit("case_passed", case_id=case["id"])
    emit("end_run", passed=len(request["cases"]), failed=0, skipped=0)


def main():
    global EVENT_LOG
    request = json.load(sys.stdin)
    artifact_root = Path(os.environ.get("TESHI_FEATURE_ARTIFACTS", request["meta"].get("artifacts_dir", ".teshi/tui-results/directory")))
    artifact_root.mkdir(parents=True, exist_ok=True)
    EVENT_LOG = artifact_root / "binding-events.ndjson"
    captures = Path(request["meta"].get("evidence_run_directory", str(artifact_root / "captures")))
    (artifact_root / "request.json").write_text(json.dumps(request, ensure_ascii=False, indent=2), encoding="utf-8")
    emit("start_run", total=len(request["cases"]))
    failed = examples_failed = examples_passed = 0
    for case in request["cases"]:
        emit("start_case", case_id=case["id"], name=case["scenario"])
        variants = [dict(zip(table["headers"], row)) for table in case["examples"] for row in table["rows"]] or [{}]
        errors = []
        for index, variant in enumerate(variants):
            # Every Examples row is its own business execution.
            attempt = index + 1
            evidence = Path(os.environ.get("TESHI_FEATURE_ARTIFACTS", request["meta"].get("artifacts_dir", ".teshi/tui-results/directory"))) / re.sub(r"[^A-Za-z0-9_-]", "_", str(case["id"])) / str(index)
            evidence.mkdir(parents=True, exist_ok=True)
            world = World(request, evidence)
            events = []
            text = "setup"
            step = dict(line=0, text=text)
            try:
                for step in case["steps"]:
                    text = step["text"]
                    for key, value in variant.items():
                        text = text.replace("<" + key + ">", value)
                    event = dict(line=step["line"], text=text, example=variant)
                    emit("start_step", case_id=case["id"], step_id=str(step["line"]), attempt=attempt, text=text, example=variant)
                    world.step(text)
                    events.append(dict(event, status="passed"))
                    emit("end_step", case_id=case["id"], step_id=str(step["line"]), attempt=attempt, status="passed", example=variant)
                    emit("step_screenshot", **world.capture(captures, case, step, attempt), status="passed")
            except Exception as error:
                errors.append(f"{text}: {error}")
                events.append(dict(text=text, example=variant, status="failed", message=str(error)))
                emit("end_step", case_id=case["id"], step_id=str(step["line"]), attempt=attempt, status="failed", message=str(error))
                emit("step_screenshot", **world.capture(captures, case, step, attempt), status="failed")
            finally:
                try:
                    world.close()
                except Exception as error:
                    errors.append("Terminal cleanup: " + str(error))
                    events.append(dict(status="failed", message=str(error)))
                (evidence / "steps.json").write_text(json.dumps(events, ensure_ascii=False, indent=2), encoding="utf-8")
                ok = all(event["status"] == "passed" for event in events)
                examples_passed += int(ok)
                examples_failed += int(not ok)
        if errors:
            failed += 1
            emit("case_failed", case_id=case["id"], error=dict(message="; ".join(errors)))
        else:
            emit("case_passed", case_id=case["id"])
    emit("end_run", passed=len(request["cases"])-failed, failed=failed, skipped=0, examples_passed=examples_passed, examples_failed=examples_failed)
    return bool(failed)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--probe-runner":
        probe_runner(sys.argv[2])
    else:
        sys.exit(main())
