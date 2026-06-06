"""Win32/WinUI app bridge for teshi-desktop.

The service mirrors the browser bridge shape: WebSocket clients send one JSON
command and receive a typed response, while preview clients receive JPEG frames
as JSON `frame` messages. UI Automation provides element discovery/actions and
Pillow ImageGrab provides the fallback screenshot stream.
"""

from __future__ import annotations

import argparse
import asyncio
import base64
import csv
import ctypes
import io
import json
import os
import subprocess
import sys
import time
from ctypes import wintypes
from pathlib import Path
from typing import Any

try:
    import websockets
except ImportError as exc:  # pragma: no cover - startup preflight catches this
    print(f"websockets import failed: {exc}", file=sys.stderr)
    raise

try:
    import uiautomation as auto
except ImportError:
    auto = None

try:
    from PIL import ImageGrab
except ImportError:
    ImageGrab = None


FRAME_INTERVAL_SEC = 1.0 / 8.0
MAX_SNAPSHOT_DEPTH = 8
MAX_SNAPSHOT_NODES = 300
INTERACTIVE_TYPES = {
    "ButtonControl",
    "CheckBoxControl",
    "ComboBoxControl",
    "EditControl",
    "HyperlinkControl",
    "ListItemControl",
    "MenuItemControl",
    "RadioButtonControl",
    "TabItemControl",
}


def debug_enabled() -> bool:
    """Return true when verbose diagnostics should be persisted."""
    return bool(str(os.environ.get("TESHI_WINAPP_DEBUG", "")).strip())


def debug_log(project_root: Path | None, event: str, payload: dict[str, Any]) -> None:
    """Append one JSONL diagnostic record under `.teshi/logs` when enabled."""
    if project_root is None or not debug_enabled():
        return
    try:
        log_dir = project_root / ".teshi" / "logs"
        log_dir.mkdir(parents=True, exist_ok=True)
        record = {"ts": time.time(), "event": event, **payload}
        with (log_dir / "winapp-bridge.log").open("a", encoding="utf-8") as f:
            f.write(json.dumps(record, ensure_ascii=False) + "\n")
    except OSError:
        return


def write_endpoint_file(project_root: Path, *, mode: str, ws_url: str) -> None:
    """Write the local endpoint used by terminal agents."""
    teshi_dir = project_root / ".teshi"
    teshi_dir.mkdir(parents=True, exist_ok=True)
    payload = {
        "mode": mode,
        "ws_url": ws_url,
        "page_url": "winapp://detached",
        "updated_at_ms": int(time.time() * 1000),
    }
    (teshi_dir / "cdp-endpoint.json").write_text(
        json.dumps(payload, indent=2),
        encoding="utf-8",
    )


def update_endpoint_target(project_root: Path | None, url: str, title: str | None = None) -> None:
    """Update target metadata without changing the agent endpoint."""
    if project_root is None:
        return
    path = project_root / ".teshi" / "cdp-endpoint.json"
    if not path.exists():
        return
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
        payload["page_url"] = url
        if title:
            payload["title"] = title
        payload["updated_at_ms"] = int(time.time() * 1000)
        path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    except (OSError, json.JSONDecodeError):
        return


class Rect(ctypes.Structure):
    """Win32 RECT wrapper."""

    _fields_ = [
        ("left", ctypes.c_long),
        ("top", ctypes.c_long),
        ("right", ctypes.c_long),
        ("bottom", ctypes.c_long),
    ]


if os.name == "nt":
    user32 = ctypes.windll.user32
    user32.EnumWindows.argtypes = [
        ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM),
        wintypes.LPARAM,
    ]
    user32.EnumWindows.restype = ctypes.c_bool
    user32.IsWindowVisible.argtypes = [wintypes.HWND]
    user32.IsWindowVisible.restype = ctypes.c_bool
    user32.GetWindowTextLengthW.argtypes = [wintypes.HWND]
    user32.GetWindowTextLengthW.restype = ctypes.c_int
    user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    user32.GetWindowTextW.restype = ctypes.c_int
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.GetWindowThreadProcessId.restype = wintypes.DWORD
    user32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(Rect)]
    user32.GetWindowRect.restype = ctypes.c_bool
    user32.SetForegroundWindow.argtypes = [wintypes.HWND]
    user32.SetForegroundWindow.restype = ctypes.c_bool
else:  # pragma: no cover - this sidecar is Windows-only
    user32 = None


def get_window_title(hwnd: int) -> str:
    """Return the current window title for `hwnd`."""
    if user32 is None:
        return ""
    length = user32.GetWindowTextLengthW(hwnd)
    if length <= 0:
        return ""
    buffer = ctypes.create_unicode_buffer(length + 1)
    user32.GetWindowTextW(hwnd, buffer, length + 1)
    return buffer.value


def get_window_pid(hwnd: int) -> int:
    """Return owning process id for `hwnd`."""
    if user32 is None:
        return 0
    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return int(pid.value)


def get_window_rect(hwnd: int) -> tuple[int, int, int, int] | None:
    """Return the screen rectangle for `hwnd`, if available."""
    if user32 is None:
        return None
    rect = Rect()
    if not user32.GetWindowRect(hwnd, ctypes.byref(rect)):
        return None
    if rect.right <= rect.left or rect.bottom <= rect.top:
        return None
    return (rect.left, rect.top, rect.right, rect.bottom)


def enum_windows() -> list[dict[str, Any]]:
    """List visible top-level windows."""
    if user32 is None:
        return []
    windows: list[dict[str, Any]] = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)
    def callback(hwnd: int, _lparam: int) -> bool:
        if not user32.IsWindowVisible(hwnd):
            return True
        title = get_window_title(hwnd).strip()
        if not title:
            return True
        rect = get_window_rect(hwnd)
        windows.append(
            {
                "hwnd": hwnd,
                "title": title,
                "pid": get_window_pid(hwnd),
                "rect": rect,
            }
        )
        return True

    user32.EnumWindows(callback, 0)
    return windows


def process_name_for_pid(pid: int) -> str | None:
    """Best-effort process executable name for display and matching."""
    if pid <= 0:
        return None
    try:
        output = subprocess.check_output(
            ["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except (OSError, subprocess.SubprocessError):
        return None
    if not output or output.startswith("INFO:"):
        return None
    try:
        return next(csv.reader([output]))[0]  # type: ignore[name-defined]
    except Exception:
        return output.split(",", 1)[0].strip('"') or None


def find_window(
    *,
    hwnd: int | None = None,
    title: str | None = None,
    pid: int | None = None,
    process_name: str | None = None,
) -> dict[str, Any] | None:
    """Find a top-level window by hwnd, title fragment, pid, or process name."""
    windows = enum_windows()
    if hwnd is not None:
        return next((w for w in windows if int(w["hwnd"]) == hwnd), None)
    if pid is not None:
        return next((w for w in windows if int(w["pid"]) == pid), None)
    if title:
        needle = title.casefold()
        return next((w for w in windows if needle in str(w["title"]).casefold()), None)
    if process_name:
        needle = process_name.casefold()
        for window in windows:
            name = process_name_for_pid(int(window["pid"])) or ""
            if needle in name.casefold():
                window["process_name"] = name
                return window
    return None


def rect_to_dict(rect: Any) -> dict[str, int] | None:
    """Normalize UIA rectangle-like values."""
    if rect is None:
        return None
    names = ("left", "top", "right", "bottom")
    if all(hasattr(rect, name) for name in names):
        left, top, right, bottom = (int(getattr(rect, name)) for name in names)
    elif isinstance(rect, (tuple, list)) and len(rect) >= 4:
        left, top, right, bottom = (int(v) for v in rect[:4])
    else:
        return None
    return {
        "left": left,
        "top": top,
        "right": right,
        "bottom": bottom,
        "width": max(0, right - left),
        "height": max(0, bottom - top),
    }


def escape_selector_value(value: str) -> str:
    """Escape separator characters used in compact UIA selectors."""
    return value.replace("\\", "\\\\").replace(";", "\\;").replace("=", "\\=")


def split_selector(selector: str) -> dict[str, str]:
    """Parse `key=value;key=value` selectors with basic escaping."""
    raw = selector.strip()
    if raw.startswith("uia:"):
        raw = raw[4:]
    parts: list[str] = []
    buf: list[str] = []
    escaped = False
    for ch in raw:
        if escaped:
            buf.append(ch)
            escaped = False
        elif ch == "\\":
            escaped = True
        elif ch == ";":
            parts.append("".join(buf))
            buf = []
        else:
            buf.append(ch)
    parts.append("".join(buf))
    out: dict[str, str] = {}
    for part in parts:
        if "=" in part:
            key, value = part.split("=", 1)
            out[key.strip().lower()] = value.strip()
    if not out and raw:
        out["automation_id"] = raw
    return out


def selector_for_control(node: dict[str, Any]) -> str:
    """Build the most stable compact selector available for a UIA node."""
    automation_id = node.get("automation_id")
    if automation_id:
        return f"uia:automation_id={escape_selector_value(str(automation_id))}"
    control_type = node.get("control_type")
    name = node.get("name")
    if control_type and name:
        return (
            "uia:"
            f"control_type={escape_selector_value(str(control_type))};"
            f"name={escape_selector_value(str(name))}"
        )
    if name:
        return f"uia:name={escape_selector_value(str(name))}"
    path = node.get("path")
    return f"uia:path={escape_selector_value(str(path or '0'))}"


def get_control_property(control: Any, name: str) -> Any:
    """Return a UIA control property, swallowing provider-specific failures."""
    try:
        return getattr(control, name)
    except Exception:
        return None


class HighlightOverlay:
    """Tiny topmost rectangle overlay used to show selected native elements."""

    def __init__(self) -> None:
        self._proc: subprocess.Popen[str] | None = None

    def show(self, rect: dict[str, int]) -> None:
        """Show the highlight rectangle for a few seconds."""
        self.clear()
        script = (
            "import tkinter as tk, sys\n"
            "x,y,w,h = map(int, sys.argv[1:5])\n"
            "root = tk.Tk(); root.overrideredirect(True); root.attributes('-topmost', True)\n"
            "root.attributes('-alpha', 0.35); root.geometry(f'{w}x{h}+{x}+{y}')\n"
            "c = tk.Canvas(root, width=w, height=h, highlightthickness=0, bg='white')\n"
            "c.pack(fill='both', expand=True)\n"
            "c.create_rectangle(2, 2, max(2, w-3), max(2, h-3), outline='#2563eb', width=4)\n"
            "root.after(3500, root.destroy); root.mainloop()\n"
        )
        self._proc = subprocess.Popen(
            [
                sys.executable,
                "-c",
                script,
                str(rect["left"]),
                str(rect["top"]),
                str(max(1, rect["width"])),
                str(max(1, rect["height"])),
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            text=True,
        )

    def clear(self) -> None:
        """Close any active overlay process."""
        if self._proc and self._proc.poll() is None:
            self._proc.terminate()
        self._proc = None


class WinAppSession:
    """State for one attached native application window."""

    def __init__(self, project_root: Path | None) -> None:
        self.project_root = project_root
        self.hwnd: int | None = None
        self.title = ""
        self.process: subprocess.Popen[str] | None = None
        self.clients: set[Any] = set()
        self.overlay = HighlightOverlay()
        self._frame_seq = 0
        self._last_error = ""

    def target_url(self) -> str:
        """Return a stable display URL for the selected native target."""
        if self.hwnd is None:
            return "winapp://detached"
        return f"winapp://hwnd/{self.hwnd}"

    def attach(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Attach to a visible top-level window."""
        hwnd = payload.get("hwnd")
        pid = payload.get("pid")
        try:
            hwnd_i = int(hwnd) if hwnd is not None else None
            pid_i = int(pid) if pid is not None else None
        except (TypeError, ValueError):
            return {"ok": False, "error": "hwnd and pid must be integers"}
        window = find_window(
            hwnd=hwnd_i,
            title=payload.get("title"),
            pid=pid_i,
            process_name=payload.get("process_name"),
        )
        if not window:
            return {"ok": False, "error": "matching top-level window not found"}
        self.hwnd = int(window["hwnd"])
        self.title = str(window["title"])
        update_endpoint_target(self.project_root, self.target_url(), self.title)
        debug_log(self.project_root, "attached", {"hwnd": self.hwnd, "title": self.title})
        return {"ok": True, "target": self.target_info()}

    async def launch(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Launch an app process and attach to its first visible window."""
        path = str(payload.get("path") or "").strip()
        if not path:
            return {"ok": False, "error": "path is required"}
        args = payload.get("args") or []
        if not isinstance(args, list):
            return {"ok": False, "error": "args must be a list"}
        try:
            self.process = subprocess.Popen([path, *[str(a) for a in args]], text=True)
        except OSError as exc:
            return {"ok": False, "error": f"launch failed: {exc}"}
        deadline = time.monotonic() + float(payload.get("timeout_ms") or 15000) / 1000.0
        title = payload.get("title")
        while time.monotonic() < deadline:
            window = find_window(pid=self.process.pid)
            if not window and title:
                window = find_window(title=str(title))
            if window:
                self.hwnd = int(window["hwnd"])
                self.title = str(window["title"])
                update_endpoint_target(self.project_root, self.target_url(), self.title)
                return {"ok": True, "target": self.target_info()}
            await asyncio.sleep(0.25)
        return {"ok": False, "error": "launched process did not expose a visible window"}

    def target_info(self) -> dict[str, Any]:
        """Return current target metadata."""
        if self.hwnd is None:
            return {"attached": False}
        return {
            "attached": True,
            "hwnd": self.hwnd,
            "title": get_window_title(self.hwnd) or self.title,
            "pid": get_window_pid(self.hwnd),
            "rect": get_window_rect(self.hwnd),
            "url": self.target_url(),
        }

    def root_control(self) -> Any:
        """Return the UIA root control for the attached window."""
        if auto is None:
            raise RuntimeError(
                "uiautomation is not installed; run `pip install -r python/requirements.txt`"
            )
        if self.hwnd is None:
            raise RuntimeError("no WinUI3 window attached; run `teshi winapp attach` first")
        return auto.ControlFromHandle(self.hwnd)

    def snapshot(self) -> dict[str, Any]:
        """Return a UIA tree and flattened interactive element list."""
        root = self.root_control()
        nodes: list[dict[str, Any]] = []

        def walk(control: Any, depth: int, path: str) -> dict[str, Any] | None:
            if len(nodes) >= MAX_SNAPSHOT_NODES or depth > MAX_SNAPSHOT_DEPTH:
                return None
            node = {
                "path": path,
                "name": get_control_property(control, "Name") or "",
                "automation_id": get_control_property(control, "AutomationId") or "",
                "control_type": get_control_property(control, "ControlTypeName") or "",
                "class_name": get_control_property(control, "ClassName") or "",
                "bounding_rectangle": rect_to_dict(
                    get_control_property(control, "BoundingRectangle")
                ),
                "is_enabled": bool(get_control_property(control, "IsEnabled")),
                "is_offscreen": bool(get_control_property(control, "IsOffscreen")),
                "children": [],
            }
            node["selector"] = selector_for_control(node)
            nodes.append(node)
            try:
                children = control.GetChildren()
            except Exception:
                children = []
            for idx, child in enumerate(children):
                child_node = walk(child, depth + 1, f"{path}/{idx}")
                if child_node is not None:
                    node["children"].append(child_node)
            return node

        tree = walk(root, 0, "0") or {}
        interactive = [
            {
                "name": node.get("name"),
                "automation_id": node.get("automation_id"),
                "control_type": node.get("control_type"),
                "class_name": node.get("class_name"),
                "bounding_rectangle": node.get("bounding_rectangle"),
                "selector": node.get("selector"),
            }
            for node in nodes
            if node.get("control_type") in INTERACTIVE_TYPES
            and not node.get("is_offscreen")
        ][:80]
        return {
            "ok": True,
            "url": self.target_url(),
            "title": get_window_title(self.hwnd or 0) or self.title,
            "accessibility_tree": tree,
            "interactive_elements": interactive,
            "target": self.target_info(),
        }

    def find_control(self, selector: str) -> Any:
        """Find a UIA control from a compact selector."""
        criteria = split_selector(selector)
        root = self.root_control()
        if "path" in criteria:
            control = root
            for part in criteria["path"].split("/")[1:]:
                try:
                    idx = int(part)
                    control = control.GetChildren()[idx]
                except Exception as exc:
                    raise RuntimeError(f"path selector did not resolve: {selector}") from exc
            return control

        def matches(control: Any) -> bool:
            values = {
                "automation_id": get_control_property(control, "AutomationId") or "",
                "name": get_control_property(control, "Name") or "",
                "control_type": get_control_property(control, "ControlTypeName") or "",
                "class_name": get_control_property(control, "ClassName") or "",
            }
            for key, expected in criteria.items():
                if key not in values:
                    continue
                if str(values[key]).casefold() != expected.casefold():
                    return False
            return True

        stack = [root]
        while stack:
            control = stack.pop(0)
            if matches(control):
                return control
            try:
                stack.extend(control.GetChildren())
            except Exception:
                continue
        raise RuntimeError(f"selector did not resolve: {selector}")

    def highlight(self, selector: str) -> dict[str, Any]:
        """Highlight the selected native element."""
        control = self.find_control(selector)
        rect = rect_to_dict(get_control_property(control, "BoundingRectangle"))
        if not rect or rect["width"] <= 0 or rect["height"] <= 0:
            return {"ok": False, "error": "selector resolved but has no visible bounds"}
        self.overlay.show(rect)
        return {"ok": True, "selector": selector, "bounds": rect}

    def clear_highlight(self) -> dict[str, Any]:
        """Clear the active highlight overlay."""
        self.overlay.clear()
        return {"ok": True}

    def execute(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Run one action against a selected UIA control."""
        selector = str(payload.get("selector") or "")
        action = str(payload.get("action") or "click")
        value = payload.get("value")
        control = self.find_control(selector)
        try:
            if self.hwnd is not None and user32 is not None:
                user32.SetForegroundWindow(self.hwnd)
            if action == "click":
                self._click(control)
            elif action == "fill":
                if value is None:
                    return {"ok": False, "error": "fill requires value"}
                self._fill(control, str(value))
            elif action == "assert_visible":
                rect = rect_to_dict(get_control_property(control, "BoundingRectangle"))
                if not rect or rect["width"] <= 0 or rect["height"] <= 0:
                    return {"ok": False, "error": "element is not visible"}
            elif action == "assert_text":
                expected = str(value or "")
                actual = self._control_text(control)
                if expected not in actual:
                    return {
                        "ok": False,
                        "error": f"expected text {expected!r}, got {actual!r}",
                    }
            elif action == "select":
                self._select(control)
            elif action == "press_key":
                if value is None:
                    return {"ok": False, "error": "press_key requires value"}
                if auto is None:
                    return {"ok": False, "error": "uiautomation is not installed"}
                control.SetFocus()
                auto.SendKeys(str(value), waitTime=0.05)
            else:
                return {"ok": False, "error": f"unsupported_action: {action}"}
        except Exception as exc:
            return {"ok": False, "error": str(exc)}
        return {"ok": True, "selector": selector, "action": action}

    def _click(self, control: Any) -> None:
        try:
            control.GetInvokePattern().Invoke()
            return
        except Exception:
            pass
        try:
            control.Click(simulateMove=False)
            return
        except Exception:
            pass
        rect = rect_to_dict(get_control_property(control, "BoundingRectangle"))
        if auto is None or not rect:
            raise RuntimeError("element cannot be clicked")
        auto.Click(rect["left"] + rect["width"] // 2, rect["top"] + rect["height"] // 2)

    def _fill(self, control: Any, value: str) -> None:
        try:
            control.GetValuePattern().SetValue(value)
            return
        except Exception:
            pass
        if auto is None:
            raise RuntimeError("uiautomation is not installed")
        control.SetFocus()
        auto.SendKeys("{Ctrl}a", waitTime=0.05)
        auto.SendKeys(value, waitTime=0.05)

    def _select(self, control: Any) -> None:
        try:
            control.GetSelectionItemPattern().Select()
            return
        except Exception:
            self._click(control)

    def _control_text(self, control: Any) -> str:
        try:
            value = control.GetValuePattern().Value
            if value:
                return str(value)
        except Exception:
            pass
        return str(get_control_property(control, "Name") or "")

    def capture_jpeg(self) -> bytes:
        """Capture the attached window as JPEG bytes."""
        if self.hwnd is None:
            raise RuntimeError("no WinUI3 window attached")
        if ImageGrab is None:
            raise RuntimeError(
                "Pillow is not installed; run `pip install -r python/requirements.txt`"
            )
        bbox = get_window_rect(self.hwnd)
        if bbox is None:
            raise RuntimeError("target window has no valid bounds")
        # all_screens=True is required when the target HWND is on a non-primary monitor.
        image = ImageGrab.grab(bbox=bbox, all_screens=True)
        buffer = io.BytesIO()
        image.convert("RGB").save(buffer, format="JPEG", quality=70)
        return buffer.getvalue()

    async def broadcast_frame_loop(self) -> None:
        """Broadcast preview frames while clients are connected."""
        while True:
            await asyncio.sleep(FRAME_INTERVAL_SEC)
            if not self.clients:
                continue
            if self.hwnd is None:
                continue
            try:
                jpg = self.capture_jpeg()
                self._frame_seq += 1
                payload = json.dumps(
                    {
                        "type": "frame",
                        "data": base64.b64encode(jpg).decode("ascii"),
                        "url": self.target_url(),
                        "title": get_window_title(self.hwnd) or self.title,
                        "seq": self._frame_seq,
                    }
                )
                self._last_error = ""
            except Exception as exc:
                error = str(exc)
                if error == self._last_error:
                    continue
                self._last_error = error
                payload = json.dumps({"type": "frame_error", "error": error})
            await self.broadcast(payload)

    async def broadcast(self, message: str) -> None:
        """Send a message to all connected clients."""
        dead = []
        for client in list(self.clients):
            try:
                await client.send(message)
            except Exception:
                dead.append(client)
        for client in dead:
            self.clients.discard(client)


async def handle_command(session: WinAppSession, payload: dict[str, Any]) -> dict[str, Any]:
    """Dispatch one sidecar command."""
    cmd = payload.get("cmd")
    try:
        if cmd == "list_windows":
            return {"ok": True, "windows": enum_windows()}
        if cmd == "attach_window":
            return session.attach(payload)
        if cmd == "launch_app":
            return await session.launch(payload)
        if cmd == "get_ui_snapshot":
            return session.snapshot()
        if cmd == "highlight_selector":
            return session.highlight(str(payload.get("selector") or ""))
        if cmd == "clear_highlight":
            return session.clear_highlight()
        if cmd == "execute_locator":
            return session.execute(payload)
        if cmd == "screenshot":
            jpg = session.capture_jpeg()
            b64 = base64.b64encode(jpg).decode("ascii")
            return {"type": "response", "request_id": payload.get("request_id"), "ok": True, "screenshot": b64}
        if cmd == "get_target":
            return {"ok": True, "target": session.target_info()}
        return {"ok": False, "error": f"unknown command: {cmd}"}
    except Exception as exc:
        return {"ok": False, "error": str(exc)}


async def run_server(host: str, port: int, project_root: Path | None) -> None:
    """Run the WinApp sidecar WebSocket server."""
    session = WinAppSession(project_root)
    ws_url = f"ws://{host}:{port}"
    if project_root is not None:
        write_endpoint_file(project_root, mode="winapp", ws_url=ws_url)

    async def handler(websocket: Any) -> None:
        session.clients.add(websocket)
        try:
            async for message in websocket:
                if isinstance(message, bytes):
                    continue
                try:
                    payload = json.loads(message)
                except json.JSONDecodeError:
                    await websocket.send(
                        json.dumps({"type": "response", "ok": False, "error": "invalid JSON"})
                    )
                    continue
                request_id = payload.get("request_id")
                response = await handle_command(session, payload)
                response["type"] = "response"
                if request_id is not None:
                    response["request_id"] = request_id
                await websocket.send(json.dumps(response))
        finally:
            session.clients.discard(websocket)

    frame_task = asyncio.create_task(session.broadcast_frame_loop())
    async with websockets.serve(handler, host, port):
        print(json.dumps({"ready": True, "mode": "winapp", "ws_url": ws_url}), flush=True)
        try:
            await asyncio.Future()
        finally:
            frame_task.cancel()


def main() -> None:
    """Parse arguments and start the sidecar."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--mode", default="winapp")
    parser.add_argument("--project-root")
    args = parser.parse_args()
    project_root = Path(args.project_root).resolve() if args.project_root else None
    if os.name != "nt":
        print("winapp mode is only supported on Windows", file=sys.stderr)
        sys.exit(2)
    asyncio.run(run_server(args.host, args.port, project_root))


if __name__ == "__main__":
    main()
