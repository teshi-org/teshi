#!/usr/bin/env python3
"""Protocol v1 demo runner for the Explore Run Details pane.

Teshi sends one `run` request on stdin. This runner emits a deterministic set of
events and writes synthetic evidence into `meta.evidence_run_directory`, which
is always under `.teshi/logs/replay-screenshots/<run>/`.

The demo is intentionally self-contained: no network access, no third-party
runtime dependencies. Pillow is used for nicer screenshots when available; a
stdlib PNG fallback keeps the runner working without it.
"""

from __future__ import annotations

import json
import shutil
import struct
import sys
import time
import zlib
from pathlib import Path

try:  # Optional: nicer labels inside the synthetic screenshots.
    from PIL import Image, ImageDraw, ImageFont
except ImportError:  # pragma: no cover - the fallback is the portable path
    Image = None
    ImageDraw = None
    ImageFont = None


PASSED = "passed"
FAILED = "failed"
BLOCKED = "blocked"
SKIPPED = "skipped"


def emit(event: dict) -> None:
    print(json.dumps(event, ensure_ascii=False), flush=True)


def steps_for(feature_path: Path, scenario_name: str) -> list[tuple[int, str]]:
    """Return `(line, body)` pairs for one Scenario, in source order."""
    try:
        lines = feature_path.read_text(encoding="utf-8").splitlines()
    except OSError:
        return []
    found = False
    steps: list[tuple[int, str]] = []
    for number, line in enumerate(lines, 1):
        stripped = line.strip()
        if stripped.startswith(("Scenario:", "Scenario Outline:")):
            name = stripped.split(":", 1)[1].strip()
            if found:
                break
            found = name == scenario_name
            continue
        if not found:
            continue
        for keyword in ("Given ", "When ", "Then ", "And ", "But "):
            if stripped.startswith(keyword):
                steps.append((number, stripped[len(keyword) :]))
                break
    return steps


def _fill(buffer: bytearray, width: int, x: int, y: int, w: int, h: int, color: tuple[int, int, int]) -> None:
    r, g, b = color
    for row in range(max(0, y), min(540, y + h)):
        base = (row * width + max(0, x)) * 3
        for column in range(max(0, x), min(width, x + w)):
            offset = (row * width + column) * 3
            buffer[offset] = r
            buffer[offset + 1] = g
            buffer[offset + 2] = b


def _png_fallback(path: Path, status: str, step_text: str) -> None:
    width, height = 960, 540
    pixels = bytearray(width * height * 3)
    _fill(pixels, width, 0, 0, width, height, (0x1C, 0x1C, 0x1C))
    _fill(pixels, width, 0, 0, width, 56, (0x30, 0x30, 0x30))
    _fill(pixels, width, 20, 20, 16, 16, (0xAF, 0x87, 0xFF))
    _fill(pixels, width, 0, 56, 240, height - 56, (0x26, 0x26, 0x26))
    _fill(pixels, width, 280, 110, 620, 260, (0x30, 0x30, 0x30))
    _fill(pixels, width, 280, 110, 6, 260, (0xAF, 0x87, 0xFF))
    _fill(pixels, width, 320, 160, 460, 12, (0x5F, 0x5F, 0x5F))
    _fill(pixels, width, 320, 190, 340, 12, (0x5F, 0x5F, 0x5F))
    _fill(pixels, width, 320, 250, 420, 48, (0x1C, 0x1C, 0x1C))
    _fill(pixels, width, 320, 320, 180, 36, (0x41, 0x37, 0x55))
    color = (0x5F, 0xFF, 0x87) if status == PASSED else (0xFF, 0x5F, 0x5F)
    _fill(pixels, width, 760, 120, 110, 28, color)
    _fill(pixels, width, 0, 500, width, 1, (0x5F, 0x5F, 0x5F))
    # Mark the step number with tick marks so different steps look different.
    try:
        number = int(step_text.split("#", 1)[1].split()[0])
    except (IndexError, ValueError):
        number = 1
    for index in range(number):
        _fill(pixels, width, 20 + index * 24, 470, 14, 8, (0x87, 0x5F, 0xAF))

    raw = b"".join(b"\x00" + bytes(pixels[row * width * 3 : (row + 1) * width * 3]) for row in range(height))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + kind
            + data
            + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
        )

    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 6))
        + chunk(b"IEND", b"")
    )


def screenshot(path: Path, case_name: str, step_text: str, status: str) -> None:
    accent = (0x5F, 0xFF, 0x87) if status == PASSED else (0xFF, 0x5F, 0x5F)
    label = "PASSED" if status == PASSED else "FAILED"
    if Image is None:
        _png_fallback(path, status, f"#{step_text.count(' ')}")
        return
    image = Image.new("RGB", (960, 540), (0x1C, 0x1C, 0x1C))
    draw = ImageDraw.Draw(image)
    try:
        title_font = ImageFont.load_default(size=26)
        body_font = ImageFont.load_default(size=18)
    except TypeError:  # Pillow < 10 has no sized default font
        title_font = ImageFont.load_default()
        body_font = ImageFont.load_default()
    draw.rectangle((0, 0, 960, 56), fill=(0x30, 0x30, 0x30))
    draw.rectangle((20, 20, 34, 34), fill=(0xAF, 0x87, 0xFF))
    draw.text((50, 16), "teshi demo browser", font=body_font, fill=(0xEE, 0xEE, 0xEE))
    draw.text((180, 16), "http://127.0.0.1:3000/login", font=body_font, fill=(0x94, 0x94, 0x94))
    draw.rectangle((0, 56, 240, 540), fill=(0x26, 0x26, 0x26))
    draw.text((20, 80), "Demo shop", font=title_font, fill=(0xAF, 0x87, 0xFF))
    for index, row in enumerate(("Login", "Orders", "Checkout", "Settings")):
        draw.rectangle((0, 130 + index * 42, 240, 168 + index * 42), fill=(0x33, 0x2C, 0x41) if index == 0 else (0x26, 0x26, 0x26))
        draw.text((24, 140 + index * 42), row, font=body_font, fill=(0xEE, 0xEE, 0xEE))
    draw.rectangle((280, 110, 900, 400), fill=(0x30, 0x30, 0x30))
    draw.rectangle((280, 110, 286, 400), fill=(0xAF, 0x87, 0xFF))
    draw.text((320, 136), case_name, font=title_font, fill=(0xEE, 0xEE, 0xEE))
    draw.text((320, 178), step_text, font=body_font, fill=(0xD0, 0xD0, 0xD0))
    draw.rectangle((320, 230, 780, 278), fill=(0x1C, 0x1C, 0x1C))
    draw.text((336, 244), "demo@teshi.test", font=body_font, fill=(0x94, 0x94, 0x94))
    draw.rectangle((320, 298, 500, 336), fill=(0x41, 0x37, 0x55))
    draw.text((340, 310), "Sign in", font=body_font, fill=(0xEE, 0xEE, 0xEE))
    draw.rectangle((740, 120, 880, 152), fill=accent)
    draw.text((756, 129), label, font=body_font, fill=(0x1C, 0x1C, 0x1C))
    draw.rectangle((0, 500, 960, 501), fill=(0x5F, 0x5F, 0x5F))
    draw.text((24, 512), "teshi demo evidence · 960×540", font=body_font, fill=(0x94, 0x94, 0x94))
    image.save(path)


def write_unknown(directory: Path) -> Path:
    path = directory / "stdout.ndjson"
    path.write_text(
        "\n".join(
            [
                '{"type":"start_step","step_id":"demo:L1"}',
                '{"type":"log","message":"demo runner trace"}',
                '{"type":"end_step","step_id":"demo:L1","status":"passed"}',
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    return path


def copy_recording(project_root: Path, directory: Path) -> Path | None:
    source = project_root / "assets" / "run.mp4"
    if not source.is_file():
        return None
    target = directory / "run.mp4"
    shutil.copyfile(source, target)
    return target


def exchange(case_id: str, step_id: str, status: int, failed: bool, body: dict) -> dict:
    return {
        "type": "http_exchange",
        "exchange_id": f"{step_id}-exchange",
        "case_id": case_id,
        "step_id": step_id,
        "template": "login.json.j2",
        "method": "POST",
        "url": "https://demo.teshi.test/api/login",
        "request_headers": {"Accept": "application/json", "Authorization": "***"},
        "request_body": body,
        "status": status,
        "response_headers": {"content-type": "application/json"},
        "response_body": {"error": "bad credentials"} if failed else {"session": "8f2c…a91"},
        "duration_ms": 128 if failed else 212,
        "extract": {"session": "8f2c…a91"},
        "asserts": [{"name": "status_ok", "passed": not failed, "value": status}],
        "redacted": True,
    }


def plan_for(scenario: str) -> str:
    if scenario == "Rejected password":
        return FAILED
    if scenario in {"Unavailable target", "Delivery quote blocked"}:
        return BLOCKED
    return PASSED


def run_case(case: dict, directory: Path, project_root: Path) -> tuple[str, int]:
    case_id = case["id"]
    case_name = case["scenario"]
    outcome = plan_for(case_name)
    emit({"type": "start_case", "case_id": case_id, "name": case_name})

    if outcome == BLOCKED:
        reason = "browser session unavailable" if case_name == "Unavailable target" else "delivery service offline"
        emit(
            {
                "type": "case_blocked",
                "case_id": case_id,
                "reason": reason,
                "category": "target_unavailable",
            }
        )
        time.sleep(0.15)
        return outcome, 0

    feature_path = Path(case["feature_path"])
    steps = steps_for(feature_path, case_name)
    if not steps:
        steps = [(1, "the demo runner has no parsed step")]

    run_directory = directory / case_id.replace(":", "_")
    run_directory.mkdir(parents=True, exist_ok=True)
    overview = run_directory / "overview.png"
    screenshot(overview, case_name, "case overview", FAILED if outcome == FAILED else PASSED)
    emit(
        {
            "type": "artifact",
            "case_id": case_id,
            "step_id": None,
            "kind": "screenshot",
            "path": str(overview),
        }
    )
    screenshot_paths: list[Path] = []
    recording: Path | None = None
    unknown = write_unknown(run_directory)

    for index, (line, text) in enumerate(steps):
        step_id = f"{case_id}:L{line}"
        emit({"type": "start_step", "case_id": case_id, "step_id": step_id, "text": text})
        for log_index in range(3):
            emit(
                {
                    "type": "log",
                    "case_id": case_id,
                    "message": f"[{case_name}] step {index + 1}.{log_index + 1} {text[:48]}",
                }
            )
        time.sleep(0.08)

        failed_step = outcome == FAILED and "status is 200" in text
        if index == 2 and case_name == "Successful login":
            emit(exchange(case_id, step_id, 200, False, {"demo_user": "ada", "password": "***"}))
        if failed_step:
            emit(exchange(case_id, step_id, 401, True, {"demo_user": "ada", "password": "***"}))
        if index >= 1 and outcome != BLOCKED:
            image_path = run_directory / f"step-{index + 1}.png"
            screenshot(image_path, case_name, text, FAILED if failed_step else PASSED)
            screenshot_paths.append(image_path)
            emit(
                {
                    "type": "artifact",
                    "case_id": case_id,
                    "step_id": step_id,
                    "kind": "screenshot",
                    "path": str(image_path),
                }
            )
        if index == len(steps) - 1:
            if outcome == FAILED:
                emit({"type": "end_step", "case_id": case_id, "step_id": step_id, "status": SKIPPED})
                continue
            if case_name in {"Locked account", "Successful checkout", "Successful login"}:
                recording = copy_recording(project_root, run_directory)
                if recording is not None:
                    emit(
                        {
                            "type": "artifact",
                            "case_id": case_id,
                            "step_id": step_id,
                            "kind": "recording",
                            "path": str(recording),
                        }
                    )
            emit(
                {
                    "type": "artifact",
                    "case_id": case_id,
                    "step_id": None,
                    "kind": "raw_events",
                    "path": str(unknown),
                }
            )
        emit(
            {
                "type": "end_step",
                "case_id": case_id,
                "step_id": step_id,
                "status": FAILED if failed_step else PASSED,
                "message": "assertion failed: expected 200, got 401" if failed_step else None,
            }
        )
        if failed_step:
            for later_line, later_text in steps[index + 1 :]:
                later_id = f"{case_id}:L{later_line}"
                emit({"type": "start_step", "case_id": case_id, "step_id": later_id, "text": later_text})
                emit({"type": "end_step", "case_id": case_id, "step_id": later_id, "status": SKIPPED})
            break

    if outcome == FAILED:
        emit(
            {
                "type": "case_failed",
                "case_id": case_id,
                "duration_ms": 410,
                "error": {
                    "message": "expected 200, got 401",
                    "stack": "at demo login assertion",
                    "attachments": [
                        {
                            "kind": "screenshot",
                            "path": str(screenshot_paths[-1]) if screenshot_paths else str(unknown),
                            "step_id": f"{case_id}:L{steps[min(2, len(steps) - 1)][0]}",
                        }
                    ],
                },
            }
        )
    else:
        emit({"type": "case_passed", "case_id": case_id, "duration_ms": 1240 if case_name == "Successful login" else 860})
    time.sleep(0.15)
    return outcome, len(steps)


def main() -> int:
    line = sys.stdin.readline()
    if not line:
        return 1
    request = json.loads(line)
    meta = request.get("meta", {})
    project_root = Path(meta.get("project_root", ".")).resolve()
    directory = Path(meta.get("evidence_run_directory", project_root / ".teshi/logs/replay-screenshots/demo"))
    directory.mkdir(parents=True, exist_ok=True)
    cases = request.get("cases", [])

    emit(
        {
            "type": "start_run",
            "run_id": directory.name,
            "total": len(cases),
            "protocol_version": 1,
            "capabilities": ["steps", "artifacts", "blocked", "http_exchange"],
        }
    )
    passed = failed = blocked = skipped = 0
    for case in cases:
        outcome, _ = run_case(case, directory, project_root)
        if outcome == PASSED:
            passed += 1
        elif outcome == FAILED:
            failed += 1
        else:
            blocked += 1
    emit(
        {
            "type": "end_run",
            "passed": passed,
            "failed": failed,
            "skipped": skipped,
            "blocked": blocked,
        }
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
