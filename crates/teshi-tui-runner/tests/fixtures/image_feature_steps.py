"""Source-bound screenshot acceptance through a Rust-owned PTY.

These checks verify rendered controls, cancellation, limits, and source-region
metadata. The PTY's text/SVG output does not constitute terminal pixel QA.
"""

import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile
import time
import zlib


EVENT_LOG = None


def emit(kind, **values):
    event = json.dumps(dict(type=kind, **values), ensure_ascii=False)
    print(event, flush=True)
    if EVENT_LOG:
        with EVENT_LOG.open("a", encoding="utf-8") as stream:
            stream.write(event + "\n")


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def png(path, width, height, color, header_only=False):
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    compressor = zlib.compressobj(1)
    row = b"\0" + bytes(color) * width if not header_only else b""
    pieces = [compressor.compress(row) for _ in range(height if not header_only else 0)]
    data = b"".join(pieces) + compressor.flush()
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
                     + chunk(b"IDAT", data) + chunk(b"IEND", b""))


PAIRS = [
    ("an isolated project with recorded large and small screenshots", "已有包含大图和小图回放截图的隔离项目", "project"),
    ("Teshi is running with image graphics enabled", "Teshi 已在支持图片的终端中运行", "launch"),
    ("Teshi is running with image graphics disabled", "Teshi 已在禁用图片的终端中运行", "launch_disabled"),
    ("I open the screenshot viewer", "我打开截图查看器", "open"),
    ("the viewer controls are available within 1000 milliseconds", "查看器控件在 1000 毫秒内可用", "opened_promptly"),
    ("I immediately close the screenshot viewer", "我立即关闭截图查看器", "close"),
    ("I close the screenshot viewer", "我关闭截图查看器", "close"),
    ("MindMap is interactive within 1000 milliseconds", "思维导图在 1000 毫秒内可交互", "interactive"),
    ("I quit Teshi", "我退出 Teshi", "quit"),
    ("the process exits successfully within 1000 milliseconds", "进程在 1000 毫秒内成功退出", "exited"),
    ("the large screenshot is ready in fit view", "大图已按适应窗口模式准备完成", "fit_ready"),
    ("I choose original-size viewing", "我选择原始尺寸查看", "actual"),
    ("the magnification is 100 percent", "放大比例为百分之百", "actual_ready"),
    ("I pan right", "我向右平移", "pan"),
    ("the visible source region moves right without changing the capture", "可见原图区域向右移动且截图未切换", "panned"),
    ("I increase magnification", "我提高放大比例", "zoom"),
    ("the magnification is 125 percent", "放大比例为百分之一百二十五", "zoom_ready"),
    ("I resize the terminal", "我调整终端尺寸", "resize"),
    ("the magnification remains 100 percent", "放大比例仍为百分之百", "actual_ready"),
    ("I reset the screenshot view", "我重置截图视图", "fit"),
    ("I immediately select the next capture", "我立即选择下一张截图", "next"),
    ("only the small capture remains selected after pending work settles", "待处理任务结束后仍只选中小图", "small_ready"),
    ("the first capture exceeds the decoded-image limit", "第一张截图超过已解码图片的限制", "oversize"),
    ("the image limit error and external-viewer action are visible", "图片限制错误和外部查看入口可见", "limit_error"),
    ("the unavailable graphics reason and external-viewer action are visible", "图片不可用的原因和外部查看入口可见", "unavailable"),
    ("I open the Explore steps pane", "我打开探索页的步骤面板", "explore_pane"),
    ("the Explore panes mark captured scenarios and steps", "探索页标出已有截图的场景和步骤", "explore_markers"),
    ("I open the screenshot viewer from the Explore steps", "我从探索页的步骤打开截图查看器", "open_explore"),
    ("the screenshot viewer shows the capture of the selected step", "截图查看器显示所选步骤的截图", "explore_capture_ready"),
    ("I close the screenshot viewer from Explore", "我从探索页关闭截图查看器", "close_explore"),
    ("Explore is interactive within 1000 milliseconds", "探索页在 1000 毫秒内可交互", "explore_interactive"),
]
BINDINGS = {text: action for en, zh, action in PAIRS for text in (en, zh)}


class World:
    def __init__(self, request, evidence):
        self.temp = tempfile.TemporaryDirectory(prefix="teshi-image-")
        self.root = Path(self.temp.name)
        self.project = self.root / "project"
        self.project.mkdir()
        self.evidence = evidence
        host_env = {key: value for key, value in os.environ.items()
                    if not key.startswith("ZELLIJ") and key not in
                    ("TESHI_RUNNER_CMD", "TESHI_RUNNER_ARGS", "TESHI_RUNNER_CWD")}
        self.host = subprocess.Popen([request["host"], "--terminal-host"],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     text=True, encoding="utf-8", env=host_env)
        self.running = False
        self.last_screen = ""
        self.metrics = {}
        self.pan_from = None
        self.graphics = None

    def op(self, op, text="", **values):
        self.host.stdin.write(json.dumps(dict(op=op, text=text, **values)) + "\n")
        self.host.stdin.flush()
        reply = json.loads(self.host.stdout.readline())
        if not reply["ok"]:
            raise AssertionError(reply["error"])
        if op == "screen":
            self.last_screen = reply.get("value", "")
        return reply.get("value")

    def wait(self, predicate, timeout=20):
        deadline = time.perf_counter() + timeout
        while time.perf_counter() < deadline:
            screen = self.op("screen")
            if predicate(screen):
                return screen
            time.sleep(.03)
        raise AssertionError("Viewport condition timed out\n" + self.last_screen)

    @staticmethod
    def header(screen):
        return next((line for line in screen.splitlines() if "Screenshot " in line), "")

    def ready(self, marker, label):
        return self.wait(lambda screen: marker in self.header(screen)
                         and label in self.header(screen)
                         and "Loading" not in self.header(screen)
                         and "Drawing" not in self.header(screen))

    def key(self, text):
        self.op("key", text)

    def create_project(self):
        (self.project / "images.feature").write_text(
            "Feature: Image fixture\n  Scenario: Detail\n    Given a saved screenshot\n", encoding="utf-8")
        self.run = self.project / ".teshi/logs/replay-screenshots/11111111-1111-4111-8111-111111111111"
        self.run.mkdir(parents=True)
        png(self.run / "large.png", 8192, 4096, (180, 40, 20))
        png(self.run / "small.png", 1024, 512, (20, 180, 40))
        events = [dict(type="step_screenshot", capture_status="available", screenshot_file=file,
                       feature="images.feature", scenario="Detail", step_text="a saved screenshot",
                       step_line=3, status="passed", captured_at="2026-10-31T00:00:00Z")
                  for file in ("large.png", "small.png")]
        (self.run / "run.json").write_text(json.dumps(dict(
            started_at="2026-10-31T00:00:00Z", events=events)), encoding="utf-8")

    def launch(self, graphics):
        if self.running and self.graphics == graphics:
            return
        binary = str(Path(os.environ["TESHI_SELF_TEST_BIN"]).resolve())
        overrides = [(key, str(self.root / key.lower())) for key in (
            "TESHI_APP_DATA_DIR", "TESHI_SESSION_DIR", "TESHI_ACP_STORE_DIR", "TESHI_REQUIREMENTS_DIR")]
        overrides += [("TESHI_AGENT_BACKEND", "native"), ("TESHI_IMAGE_PROTOCOL", graphics)]
        config = dict(program=binary, args=[str(self.project)], cwd=str(self.root),
                      env=overrides, cols=132, rows=38)
        self.op("launch", config=config)
        self.running = True
        self.graphics = graphics
        with open(binary, "rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        (self.evidence / "launch.json").write_text(json.dumps(dict(config=config, sha256=digest), indent=2), encoding="utf-8")
        self.wait(lambda screen: "Image fixture" in screen or "images" in screen)
        self.key("2")
        self.wait(lambda screen: "MindMap [current" in screen)
        self.key("l")
        self.key("l")
        self.wait(lambda screen: "a saved screenshot" in screen)
        self.key("j")
        self.wait(lambda screen: "Scenario: Detail" in screen)

    def step(self, text):
        action = BINDINGS.get(text)
        if action is None:
            raise AssertionError("Unbound step: " + text)
        if action == "project":
            self.create_project()
            # Establish the tested product's viewport during fixture setup so
            # every source step has fresh evidence, including Background.
            self.launch("sixel")
        elif action in ("launch", "launch_disabled"):
            self.launch("sixel" if action == "launch" else "none")
        elif action == "open":
            self.view_started = time.perf_counter()
            self.key("i")
            self.wait(lambda screen: "Screenshot " in self.header(screen))
            self.metrics["open_ms"] = (time.perf_counter() - self.view_started) * 1000
        elif action == "opened_promptly":
            assert self.metrics["open_ms"] < 1000, self.metrics
        elif action == "close":
            self.close_started = time.perf_counter()
            self.key("Escape")
            self.wait(lambda screen: "Screenshot " not in screen and "MindMap" in screen, timeout=1)
            self.metrics["close_ms"] = (time.perf_counter() - self.close_started) * 1000
        elif action == "interactive":
            self.wait(lambda screen: "Screenshot " not in screen and "MindMap" in screen, timeout=1)
            # Measure the input/observation boundary, excluding the runner's
            # PNG evidence rendering between source steps.
            assert self.metrics["close_ms"] < 1000, self.metrics
        elif action == "quit":
            self.op("screenshot", str(self.evidence / "before-quit.svg"))
            self.quit_started = time.perf_counter()
            self.key("q")
            self.op("exit")
            self.metrics["quit_ms"] = (time.perf_counter() - self.quit_started) * 1000
            self.running = False
        elif action == "exited":
            assert self.metrics["quit_ms"] < 1000, self.metrics
        elif action == "fit_ready":
            self.ready("8192×4096", "Fit")
            self.metrics.setdefault("fit_ready_ms", []).append((time.perf_counter() - self.view_started) * 1000)
        elif action == "actual":
            self.key("1")
        elif action == "actual_ready":
            self.ready("8192×4096", "100%")
        elif action == "pan":
            screen = self.ready("8192×4096", "100%")
            self.pan_from = int(re.search(r"@ (\d+),", self.header(screen))[1])
            self.key("Right")
        elif action == "panned":
            def moved(screen):
                header = self.header(screen)
                match = re.search(r"@ (\d+),", header)
                return ("Screenshot 1/2" in header and "8192×4096" in header
                        and "Loading" not in header and match and int(match[1]) > self.pan_from)
            self.wait(moved)
        elif action == "zoom":
            self.op("write", "+")
        elif action == "zoom_ready":
            self.ready("8192×4096", "125%")
        elif action == "resize":
            self.op("resize", cols=116, rows=32)
        elif action == "fit":
            self.view_started = time.perf_counter()
            self.key("0")
        elif action == "next":
            self.key("j")
        elif action == "small_ready":
            self.ready("1024×512", "Fit")
            deadline = time.perf_counter() + .75
            while time.perf_counter() < deadline:
                screen = self.op("screen")
                assert "Screenshot 2/2" in self.header(screen), screen
                assert "8192×4096" not in self.header(screen), screen
                time.sleep(.05)
        elif action == "oversize":
            png(self.run / "large.png", 100_000, 100_000, (), header_only=True)
        elif action == "limit_error":
            self.wait(lambda screen: "Cannot show capture" in screen
                      and "limit" in screen.lower() and "Press o" in screen)
        elif action == "unavailable":
            self.wait(lambda screen: "no image protocol" in screen
                      and "TESHI_IMAGE_PROTOCOL=none" in screen and "Press o" in screen)
        elif action == "explore_pane":
            self.key("1")
            self.wait(lambda screen: "Steps" in screen and "Features" in screen)
            # Features -> Scenarios -> Steps owns the `i` entry point.
            self.key("l")
            self.key("l")
            self.wait(lambda screen: "a saved screenshot" in screen)
        elif action == "explore_markers":
            self.wait(lambda screen: "Detail ●2" in screen
                      and "a saved screenshot ●2" in screen)
        elif action == "open_explore":
            self.view_started = time.perf_counter()
            self.key("i")
            self.wait(lambda screen: "Screenshot " in self.header(screen))
        elif action == "explore_capture_ready":
            screen = self.ready("1/2", "a saved screenshot")
            # Explore keeps its Features column outside the viewer.
            assert "Features" in screen, screen
        elif action == "close_explore":
            self.key("Escape")
            self.wait(lambda screen: "Screenshot " not in screen and "Steps" in screen,
                      timeout=1)
        elif action == "explore_interactive":
            self.wait(lambda screen: "Screenshot " not in screen
                      and "Steps" in screen, timeout=1)

    def capture(self, case, step, directory, timing):
        event = dict(case_id=case["id"], step_id=f"{case['id']}:L{step['line']}",
                     feature=case["feature_path"], step_line=step["line"],
                     step_text=step["text"], source="pty", capture_timing=timing)
        try:
            stem = re.sub(r"[^A-Za-z0-9-]", "-", f"image-{case['id']}-L{step['line']}")
            event.update(self.op("capture", directory=str(directory), stem=stem),
                         capture_status="available")
        except Exception as error:
            event.update(capture_status="failed", message=str(error))
        return event

    def close(self):
        try:
            if self.running:
                self.op("screenshot", str(self.evidence / "viewport.svg"))
            (self.evidence / "viewport.txt").write_text(self.last_screen, encoding="utf-8")
            (self.evidence / "metrics.json").write_text(json.dumps(self.metrics, indent=2), encoding="utf-8")
        finally:
            self.op("close")
            self.host.stdin.close()
            self.host.wait(timeout=15)
            self.temp.cleanup()


def main():
    global EVENT_LOG
    request = json.load(sys.stdin)
    root = Path(os.environ.get("TESHI_FEATURE_ARTIFACTS", request["meta"].get("artifacts_dir", ".teshi/tui-results/images")))
    root.mkdir(parents=True, exist_ok=True)
    EVENT_LOG = root / "binding-events.ndjson"
    captures = Path(request["meta"].get("evidence_run_directory", str(root / "captures")))
    (root / "request.json").write_text(json.dumps(request, ensure_ascii=False, indent=2), encoding="utf-8")
    emit("start_run", total=len(request["cases"]))
    failed = 0
    for case in request["cases"]:
        emit("start_case", case_id=case["id"], name=case["scenario"])
        evidence = root / re.sub(r"[^A-Za-z0-9_-]", "_", str(case["id"]))
        evidence.mkdir(parents=True, exist_ok=True)
        world = World(request, evidence)
        records, errors = [], []
        step = dict(line=0, text="setup")
        before = None
        try:
            for step in case["steps"]:
                before = None
                step_id = f"{case['id']}:L{step['line']}"
                emit("start_step", case_id=case["id"], step_id=step_id, text=step["text"])
                if BINDINGS.get(step["text"]) == "quit":
                    before = world.capture(case, step, captures, "before_close")
                started = time.perf_counter()
                world.step(step["text"])
                records.append(dict(step, status="passed", elapsed_ms=(time.perf_counter() - started) * 1000))
                emit("end_step", case_id=case["id"], step_id=step_id, status="passed")
                capture = before or world.capture(case, step, captures, "after_step")
                emit("step_screenshot", **capture, status="passed")
                if world.running:
                    screen = world.op("screen")
                    (evidence / f"step-{step['line']}.txt").write_text(screen, encoding="utf-8")
        except Exception as error:
            errors.append(str(error))
            records.append(dict(step, status="failed", message=str(error)))
            emit("end_step", case_id=case["id"], step_id=f"{case['id']}:L{step['line']}", status="failed", message=str(error))
            capture = before or world.capture(case, step, captures, "after_step")
            emit("step_screenshot", **capture, status="failed")
        finally:
            try:
                world.close()
            except Exception as error:
                errors.append("cleanup: " + str(error))
            (evidence / "steps.json").write_text(json.dumps(records, ensure_ascii=False, indent=2), encoding="utf-8")
        if errors:
            failed += 1
            emit("case_failed", case_id=case["id"], error=dict(message="; ".join(errors)))
        else:
            emit("case_passed", case_id=case["id"])
    emit("end_run", passed=len(request["cases"]) - failed, failed=failed, skipped=0)
    return bool(failed)


if __name__ == "__main__":
    sys.exit(main())
