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
import threading
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
    from PIL import Image, ImageGrab
except ImportError:
    Image = None
    ImageGrab = None

try:
    from windows_capture import WindowsCapture
except Exception as exc:
    WindowsCapture = None
    WGC_IMPORT_ERROR = str(exc)
else:
    WGC_IMPORT_ERROR = ""


FRAME_INTERVAL_SEC = 1.0 / 8.0
FIRST_FRAME_TIMEOUT_SEC = 2.0
JPEG_QUALITY = 70
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
    user32.IsWindow.argtypes = [wintypes.HWND]
    user32.IsWindow.restype = ctypes.c_bool
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
    # Background (non-intrusive) input: PostMessage / SendMessage
    user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    user32.PostMessageW.restype = ctypes.c_bool
    user32.SendMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    user32.SendMessageW.restype = ctypes.c_longlong
    user32.SendMessageTimeoutW.argtypes = [
        wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM,
        wintypes.UINT, wintypes.UINT, ctypes.POINTER(ctypes.c_ulonglong),
    ]
    user32.SendMessageTimeoutW.restype = ctypes.c_longlong
    user32.MapVirtualKeyW.argtypes = [wintypes.UINT, wintypes.UINT]
    user32.MapVirtualKeyW.restype = wintypes.UINT
    user32.VkKeyScanW.argtypes = [ctypes.c_wchar]
    user32.VkKeyScanW.restype = ctypes.c_short
    user32.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(Rect)]
    user32.GetClientRect.restype = ctypes.c_bool
    user32.ScreenToClient.argtypes = [wintypes.HWND, ctypes.POINTER(ctypes.c_long * 2)]
    user32.ScreenToClient.restype = ctypes.c_bool

    # Window messages
    WM_KEYDOWN = 0x0100
    WM_KEYUP = 0x0101
    WM_CHAR = 0x0102
    WM_SYSKEYDOWN = 0x0104
    WM_SYSKEYUP = 0x0105
    WM_LBUTTONDOWN = 0x0201
    WM_LBUTTONUP = 0x0202
    WM_MOUSEMOVE = 0x0200
    WM_RBUTTONDOWN = 0x0204
    WM_RBUTTONUP = 0x0205
    WM_SETTEXT = 0x000C
    WM_GETTEXT = 0x000D
    WM_GETTEXTLENGTH = 0x000E
    WM_KILLFOCUS = 0x0008
    WM_SETFOCUS = 0x0007
    # SMTO_* flags for SendMessageTimeout
    SMTO_NORMAL = 0x0000
    SMTO_ABORTIFHUNG = 0x0002
    # MapVirtualKey mapping types
    MAPVK_VK_TO_VSC = 0
    MAPVK_VSC_TO_VK = 1
    MAPVK_VK_TO_CHAR = 2
    # Virtual-key codes for special keys
    VK_BACK = 0x08
    VK_TAB = 0x09
    VK_RETURN = 0x0D
    VK_SHIFT = 0x10
    VK_CONTROL = 0x11
    VK_MENU = 0x12  # Alt
    VK_PAUSE = 0x13
    VK_CAPITAL = 0x14
    VK_ESCAPE = 0x1B
    VK_SPACE = 0x20
    VK_PRIOR = 0x21  # Page Up
    VK_NEXT = 0x22  # Page Down
    VK_END = 0x23
    VK_HOME = 0x24
    VK_LEFT = 0x25
    VK_UP = 0x26
    VK_RIGHT = 0x27
    VK_DOWN = 0x28
    VK_SELECT = 0x29
    VK_PRINT = 0x2A
    VK_EXECUTE = 0x2B
    VK_SNAPSHOT = 0x2C
    VK_INSERT = 0x2D
    VK_DELETE = 0x2E
    VK_HELP = 0x2F
    VK_LWIN = 0x5B
    VK_RWIN = 0x5C
    VK_APPS = 0x5D
    VK_SLEEP = 0x5F
    VK_NUMPAD0 = 0x60
    VK_NUMPAD9 = 0x69
    VK_MULTIPLY = 0x6A
    VK_ADD = 0x6B
    VK_SEPARATOR = 0x6C
    VK_SUBTRACT = 0x6D
    VK_DECIMAL = 0x6E
    VK_DIVIDE = 0x6F
    VK_F1 = 0x70
    VK_F12 = 0x7B
    VK_F24 = 0x87
    VK_NUMLOCK = 0x90
    VK_SCROLL = 0x91
    VK_LSHIFT = 0xA0
    VK_RSHIFT = 0xA1
    VK_LCONTROL = 0xA2
    VK_RCONTROL = 0xA3
    VK_LMENU = 0xA4
    VK_RMENU = 0xA5
else:  # pragma: no cover - this sidecar is Windows-only
    user32 = None

# Map of SendKeys-style key names to virtual-key codes.
# Supports the same {Name} syntax as uiautomation.SendKeys.
KEY_NAME_TO_VK: dict[str, int] = {}
if os.name == "nt":
    KEY_NAME_TO_VK = {
        "backspace": VK_BACK, "bs": VK_BACK, "bksp": VK_BACK,
        "break": VK_PAUSE,
        "capslock": VK_CAPITAL,
        "delete": VK_DELETE, "del": VK_DELETE,
        "down": VK_DOWN,
        "end": VK_END,
        "enter": VK_RETURN, "return": VK_RETURN, "~": VK_RETURN,
        "esc": VK_ESCAPE, "escape": VK_ESCAPE,
        "help": VK_HELP,
        "home": VK_HOME,
        "ins": VK_INSERT, "insert": VK_INSERT,
        "left": VK_LEFT,
        "numlock": VK_NUMLOCK,
        "pgdn": VK_NEXT, "pagedown": VK_NEXT,
        "pgup": VK_PRIOR, "pageup": VK_PRIOR,
        "prtsc": VK_SNAPSHOT, "printscreen": VK_SNAPSHOT,
        "right": VK_RIGHT,
        "scrolllock": VK_SCROLL,
        "space": VK_SPACE, " ": VK_SPACE,
        "tab": VK_TAB,
        "up": VK_UP,
        "f1": VK_F1, "f2": VK_F1 + 1, "f3": VK_F1 + 2, "f4": VK_F1 + 3,
        "f5": VK_F1 + 4, "f6": VK_F1 + 5, "f7": VK_F1 + 6, "f8": VK_F1 + 7,
        "f9": VK_F1 + 8, "f10": VK_F1 + 9, "f11": VK_F1 + 10, "f12": VK_F1 + 11,
        "add": VK_ADD, "subtract": VK_SUBTRACT, "multiply": VK_MULTIPLY,
        "divide": VK_DIVIDE, "decimal": VK_DECIMAL, "separator": VK_SEPARATOR,
    }


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


def is_window(hwnd: int) -> bool:
    """Return whether `hwnd` still identifies a live window."""
    return bool(user32 is not None and user32.IsWindow(hwnd))


class ImageGrabCaptureBackend:
    """Visibility-sensitive screen-rectangle capture fallback."""

    name = "imagegrab"

    def __init__(self, hwnd: int, fallback_reason: str = "") -> None:
        self.hwnd = hwnd
        self.fallback_reason = fallback_reason

    def capture_jpeg(self, _timeout: float = FIRST_FRAME_TIMEOUT_SEC) -> bytes:
        if ImageGrab is None:
            raise RuntimeError(
                "Pillow is not installed; run `pip install -r python/requirements.txt`"
            )
        bbox = get_window_rect(self.hwnd)
        if bbox is None:
            raise RuntimeError("target window has no valid bounds")
        image = ImageGrab.grab(bbox=bbox, all_screens=True)
        buffer = io.BytesIO()
        image.convert("RGB").save(buffer, format="JPEG", quality=JPEG_QUALITY)
        return buffer.getvalue()

    def stop(self) -> None:
        """ImageGrab has no persistent capture session."""


class WgcCaptureBackend:
    """Latest-frame Windows Graphics Capture session for one HWND."""

    name = "wgc"
    fallback_reason = ""

    def __init__(self, hwnd: int, capture_factory: Any = None) -> None:
        factory = capture_factory or WindowsCapture
        if factory is None:
            detail = f": {WGC_IMPORT_ERROR}" if WGC_IMPORT_ERROR else ""
            raise RuntimeError(f"windows-capture is unavailable{detail}")
        if Image is None:
            raise RuntimeError("Pillow is unavailable for WGC JPEG encoding")

        self.hwnd = hwnd
        self._lock = threading.Lock()
        self._first_frame = threading.Event()
        self._latest_jpeg: bytes | None = None
        self._terminal_error = ""
        self._control: Any = None
        self._capture = factory(
            cursor_capture=False,
            draw_border=False,
            secondary_window=False,
            minimum_update_interval=round(FRAME_INTERVAL_SEC * 1000),
            monitor_index=None,
            window_name=None,
            window_hwnd=hwnd,
        )

        @self._capture.event
        def on_frame_arrived(frame: Any, capture_control: Any) -> None:
            try:
                # windows-capture exposes BGRA8 data borrowed for the callback.
                # Encode immediately so no native frame memory crosses threads.
                rgb = frame.frame_buffer[:, :, [2, 1, 0]]
                image = Image.fromarray(rgb, mode="RGB")
                buffer = io.BytesIO()
                image.save(buffer, format="JPEG", quality=JPEG_QUALITY)
                with self._lock:
                    self._latest_jpeg = buffer.getvalue()
                self._first_frame.set()
            except Exception as exc:
                with self._lock:
                    self._terminal_error = f"WGC frame encoding failed: {exc}"
                self._first_frame.set()
                capture_control.stop()

        @self._capture.event
        def on_closed() -> None:
            with self._lock:
                if not self._terminal_error:
                    self._terminal_error = "WGC capture session closed"
            self._first_frame.set()

        self._control = self._capture.start_free_threaded()

    def capture_jpeg(self, timeout: float = FIRST_FRAME_TIMEOUT_SEC) -> bytes:
        if not self._first_frame.wait(timeout):
            raise RuntimeError(f"WGC did not produce a frame within {timeout:g} seconds")
        with self._lock:
            error = self._terminal_error
            latest = self._latest_jpeg
        if error:
            raise RuntimeError(error)
        if self._control is not None and self._control.is_finished():
            raise RuntimeError("WGC capture thread stopped unexpectedly")
        if latest is None:
            raise RuntimeError("WGC capture returned no frame")
        return latest

    def stop(self) -> None:
        control = self._control
        self._control = None
        if control is None:
            return
        try:
            control.stop()
        except Exception:
            return
        try:
            control.wait()
        except Exception:
            return


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


def _process_is_running(process_name: str) -> bool:
    """Check whether any process whose image name contains `process_name` is running."""
    try:
        output = subprocess.check_output(
            ["tasklist", "/FI", f"IMAGENAME eq {process_name}", "/FO", "CSV", "/NH"],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except (OSError, subprocess.SubprocessError):
        return False
    if not output or output.startswith("INFO:"):
        return False
    try:
        row = next(csv.reader([output]))
        return row[0].strip('"').casefold() == process_name.casefold()
    except Exception:
        return process_name.casefold() in output.casefold()


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
        self._capture_switch_lock = threading.RLock()
        self._capture_backend: WgcCaptureBackend | ImageGrabCaptureBackend | None = None
        self._capture_fallback_reason = ""

    def _stop_capture_backend(self) -> None:
        backend = self._capture_backend
        self._capture_backend = None
        if backend is not None:
            backend.stop()

    def _use_imagegrab(self, reason: str) -> None:
        """Replace the active WGC session with the compatibility backend."""
        hwnd = self.hwnd
        if hwnd is None:
            raise RuntimeError("no WinUI3 window attached")
        self._stop_capture_backend()
        self._capture_fallback_reason = reason
        self._capture_backend = ImageGrabCaptureBackend(hwnd, reason)
        debug_log(
            self.project_root,
            "capture_fallback",
            {"hwnd": hwnd, "backend": "imagegrab", "reason": reason},
        )

    def _attach_target(self, window: dict[str, Any]) -> None:
        """Replace the current target and start its preferred capture backend."""
        with self._capture_switch_lock:
            self._stop_capture_backend()
            self.hwnd = int(window["hwnd"])
            self.title = str(window["title"])
            self._capture_fallback_reason = ""
            try:
                self._capture_backend = WgcCaptureBackend(self.hwnd)
            except Exception as exc:
                self._use_imagegrab(str(exc))
        update_endpoint_target(self.project_root, self.target_url(), self.title)
        debug_log(
            self.project_root,
            "attached",
            {
                "hwnd": self.hwnd,
                "title": self.title,
                "capture_backend": self.capture_backend_name,
                "capture_fallback_reason": self._capture_fallback_reason,
            },
        )

    @property
    def capture_backend_name(self) -> str:
        backend = self._capture_backend
        return backend.name if backend is not None else ""

    def capture_metadata(self) -> dict[str, str]:
        metadata = {"capture_backend": self.capture_backend_name}
        if self._capture_fallback_reason:
            metadata["capture_fallback_reason"] = self._capture_fallback_reason
        return metadata

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
        self._attach_target(window)
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
                self._attach_target(window)
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
            **self.capture_metadata(),
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
        """Run one action against a selected UIA control, or exec a system command.

        mode: "foreground" (default, SetForegroundWindow + SendInput)
              "background" (PostMessage/SendMessage, no focus change)

        exec actions (no selector needed):
          exec: launch <path>           spawn a process
          exec: close                   send WM_CLOSE to attached window
          exec: assert_process <name>   fail if no process matches
          exec: assert_no_process <name>  fail if any process matches
        """
        selector = str(payload.get("selector") or "")
        action = str(payload.get("action") or "click")
        value = payload.get("value")
        mode = str(payload.get("mode") or "foreground").casefold()
        try:
            # --- exec actions (no UIA control needed) ---
            if action == "exec":
                return self._handle_exec(selector, str(value or ""))

            control = self.find_control(selector)
            if mode not in ("foreground", "background"):
                return {"ok": False, "error": f"invalid mode: {mode!r}; use foreground or background"}
            if mode == "foreground" and self.hwnd is not None and user32 is not None:
                user32.SetForegroundWindow(self.hwnd)
            if action == "click":
                if mode == "background":
                    self._background_click(control)
                else:
                    self._click(control)
            elif action == "fill":
                if value is None:
                    return {"ok": False, "error": "fill requires value"}
                if mode == "background":
                    self._background_fill(control, str(value))
                else:
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
                if mode == "background":
                    self._background_select(control)
                else:
                    self._select(control)
            elif action == "press_key":
                if value is None:
                    return {"ok": False, "error": "press_key requires value"}
                if mode == "background":
                    self._background_press_key(control, str(value))
                elif auto is None:
                    return {"ok": False, "error": "uiautomation is not installed"}
                else:
                    control.SetFocus()
                    auto.SendKeys(str(value), waitTime=0.05)
            else:
                return {"ok": False, "error": f"unsupported_action: {action}"}
        except Exception as exc:
            return {"ok": False, "error": str(exc)}
        return {"ok": True, "selector": selector, "action": action}

    def _handle_exec(self, sub_action: str, value_arg: str) -> dict[str, Any]:
        """Handle exec commands: launch, close, assert_process, assert_no_process."""
        if sub_action == "launch":
            if not value_arg:
                return {"ok": False, "error": "exec launch requires value_arg (exe path)"}
            exe_name = os.path.basename(value_arg)
            if _process_is_running(exe_name):
                return {"ok": True, "exec": "launch", "skipped": True, "reason": f"{exe_name} is already running"}
            try:
                proc = subprocess.Popen([value_arg], text=True)
                return {"ok": True, "exec": "launch", "pid": proc.pid}
            except OSError as exc:
                return {"ok": False, "error": f"launch failed: {exc}"}

        if sub_action == "close":
            if self.hwnd is None or user32 is None:
                return {"ok": False, "error": "no attached window to close"}
            user32.PostMessageW(self.hwnd, 0x0010, 0, 0)  # WM_CLOSE
            return {"ok": True, "exec": "close", "hwnd": self.hwnd}

        if sub_action == "kill":
            if not value_arg:
                return {"ok": False, "error": "exec kill requires value_arg (process name)"}
            try:
                subprocess.run(["taskkill", "/F", "/IM", value_arg], capture_output=True, text=True)
            except OSError as exc:
                return {"ok": False, "error": f"kill failed: {exc}"}
            return {"ok": True, "exec": "kill", "process": value_arg}

        if sub_action in ("assert_process", "assert_no_process"):
            if not value_arg:
                return {"ok": False, "error": f"exec {sub_action} requires value_arg (process name)"}
            found = _process_is_running(value_arg)
            if sub_action == "assert_process" and not found:
                return {"ok": False, "error": f"process {value_arg!r} is not running"}
            if sub_action == "assert_no_process" and found:
                return {"ok": False, "error": f"process {value_arg!r} is still running"}
            return {"ok": True, "exec": sub_action, "process": value_arg, "running": found}

        return {"ok": False, "error": f"unsupported exec action: {sub_action!r}"}

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

    # ── background (non-intrusive) action implementations ──────────────

    @staticmethod
    def _make_lparam(scan_code: int = 0, repeat_count: int = 0,
                     extended: bool = False, previous: bool = False,
                     transition: bool = False) -> int:
        """Pack scan code and flags into an lParam for WM_KEY* messages."""
        return (
            (repeat_count & 0xFFFF)
            | ((scan_code & 0xFF) << 16)
            | ((1 if extended else 0) << 24)
            | ((0 << 26) | (0 << 27))  # reserved
            | ((1 if previous else 0) << 30)
            | ((1 if transition else 0) << 31)
        )

    @staticmethod
    def _mousemove_lparam(x: int, y: int) -> int:
        """Pack client-relative x, y into an lParam for mouse messages."""
        return ((y & 0xFFFF) << 16) | (x & 0xFFFF)

    def _get_control_hwnd(self, control: Any) -> int | None:
        """Return the native HWND for a UIA control, or the top-level window."""
        try:
            hwnd = get_control_property(control, "NativeWindowHandle")
            if hwnd and int(hwnd) != 0:
                return int(hwnd)
        except Exception:
            pass
        return self.hwnd

    def _background_click(self, control: Any) -> None:
        """Post WM_LBUTTONDOWN/UP to the control without foreground activation."""
        # 1) Try InvokePattern — non-intrusive UIA call, no PostMessage needed.
        try:
            control.GetInvokePattern().Invoke()
            return
        except Exception:
            pass

        # 2) Get screen coordinates from UIA, convert to client coords.
        rect = rect_to_dict(get_control_property(control, "BoundingRectangle"))
        if not rect or rect["width"] <= 0 or rect["height"] <= 0:
            raise RuntimeError("element has no visible bounds for background click")

        hwnd = self._get_control_hwnd(control)
        if hwnd is None:
            raise RuntimeError("no target HWND for background click")

        screen_x = rect["left"] + rect["width"] // 2
        screen_y = rect["top"] + rect["height"] // 2

        # Convert screen coords to client coords.
        pt = (ctypes.c_long * 2)(screen_x, screen_y)
        if user32 is not None:
            user32.ScreenToClient(hwnd, pt)
        client_x, client_y = int(pt[0]), int(pt[1])
        lparam = self._mousemove_lparam(client_x, client_y)

        # 3) Post the click.
        if user32 is not None:
            user32.PostMessageW(hwnd, WM_LBUTTONDOWN, 0x0001, lparam)  # MK_LBUTTON
            user32.PostMessageW(hwnd, WM_LBUTTONUP, 0, lparam)

    def _background_fill(self, control: Any, value: str) -> None:
        """Set text via UIA ValuePattern or WM_SETTEXT, without foreground activation."""
        # 1) Try ValuePattern first — pure UIA, no input sim.
        try:
            control.GetValuePattern().SetValue(value)
            return
        except Exception:
            pass

        # 2) Try WM_SETTEXT on the control or top-level window.
        hwnd = self._get_control_hwnd(control)
        if hwnd is not None and user32 is not None:
            result = user32.SendMessageTimeoutW(
                hwnd, WM_SETTEXT, 0, ctypes.c_wchar_p(value),
                SMTO_ABORTIFHUNG, 2000, None,
            )
            if result != 0:
                return

        # 3) Fall back to PostMessage key sequence: Ctrl+A, then type the value.
        if user32 is None:
            raise RuntimeError("background fill requires win32 API (not available)")
        target = self._get_control_hwnd(control) or self.hwnd
        if target is None:
            raise RuntimeError("no target HWND for background fill")
        # Select-all via Ctrl+A
        ctrl_lparam = self._make_lparam(
            scan_code=user32.MapVirtualKeyW(VK_CONTROL, MAPVK_VK_TO_VSC))
        a_vk = ord("A")
        a_lparam = self._make_lparam(
            scan_code=user32.MapVirtualKeyW(a_vk, MAPVK_VK_TO_VSC))
        user32.PostMessageW(target, WM_KEYDOWN, VK_CONTROL, ctrl_lparam)
        user32.PostMessageW(target, WM_KEYDOWN, a_vk, a_lparam)
        user32.PostMessageW(target, WM_KEYUP, a_vk, a_lparam | (1 << 31) | (1 << 30))
        user32.PostMessageW(target, WM_KEYUP, VK_CONTROL, ctrl_lparam | (1 << 31) | (1 << 30))
        # Type the value character by character.
        self._post_text(target, value)

    def _background_select(self, control: Any) -> None:
        """Select a list item via UIA SelectionItemPattern or background click."""
        try:
            control.GetSelectionItemPattern().Select()
            return
        except Exception:
            self._background_click(control)

    def _background_press_key(self, control: Any, key_str: str) -> None:
        """Post keystrokes to the target without foreground activation."""
        hwnd = self._get_control_hwnd(control)
        if hwnd is None:
            raise RuntimeError("no target HWND for background press_key")
        self._post_key_string(hwnd, key_str)

    # ── key string parsing and PostMessage helpers ──────────────────

    @staticmethod
    def _parse_sendkeys_tokens(key_str: str) -> list[dict[str, Any]]:
        """Parse a SendKeys-style string into a list of action tokens.

        Each token is a dict:
          {"type": "char",     "char": str}
          {"type": "special",  "vk": int, "name": str}
          {"type": "modifier", "vk": int, "down": bool}    # modifier press/release
          {"type": "literal_brace"}                         # escaped { or }

        Supports:
          - Plain text: "hello" → char tokens
          - Special keys: {Enter}, {Tab}, {F5}, {Down}, etc.
          - Modifier combos: {Ctrl}a, {Shift}{Tab}, {Ctrl}{Shift}A
          - Escaped braces: {{ → {, }} → }
        """
        tokens: list[dict[str, Any]] = []
        i = 0
        n = len(key_str)
        while i < n:
            ch = key_str[i]
            if ch == "{":
                # Find matching closing brace.
                end = key_str.find("}", i + 1)
                if end == -1:
                    tokens.append({"type": "char", "char": ch})
                    i += 1
                    continue
                if end == i + 1:
                    # Empty braces: literal {
                    tokens.append({"type": "char", "char": "{"})
                    i = end + 1
                    continue
                inner = key_str[i + 1:end]
                i = end + 1

                # Modifier + key: {Ctrl}X, {Shift}A, {Alt}Tab, {Ctrl}{Shift}A
                parts = inner.split("}{")
                if len(parts) == 1:
                    modkey = inner
                    low = modkey.casefold()
                    if low in ("ctrl", "control", "shift", "alt"):
                        # Bare modifier: press and release
                        vk = {VK_CONTROL: VK_CONTROL, "ctrl": VK_CONTROL,
                              "control": VK_CONTROL, "shift": VK_SHIFT,
                              "alt": VK_MENU}.get(low, 0)
                        if vk:
                            tokens.append({"type": "modifier", "vk": vk, "down": True})
                            tokens.append({"type": "modifier", "vk": vk, "down": False})
                        continue
                    # Single token: maybe special key, maybe char
                    vk = KEY_NAME_TO_VK.get(low)
                    if vk is not None:
                        tokens.append({"type": "special", "vk": vk, "name": modkey})
                    elif len(modkey) == 1:
                        tokens.append({"type": "char", "char": modkey})
                    else:
                        # Unknown — treat as literal text
                        for c in modkey:
                            tokens.append({"type": "char", "char": c})
                else:
                    # Multiple parts: modifiers + final key
                    mod_vks: list[int] = []
                    for part in parts[:-1]:
                        low = part.casefold()
                        vk = {VK_CONTROL: VK_CONTROL, "ctrl": VK_CONTROL,
                              "control": VK_CONTROL, "shift": VK_SHIFT,
                              "alt": VK_MENU}.get(low, 0)
                        if vk:
                            mod_vks.append(vk)
                    final = parts[-1]
                    low_final = final.casefold()
                    for vk in mod_vks:
                        tokens.append({"type": "modifier", "vk": vk, "down": True})
                    vk_final = KEY_NAME_TO_VK.get(low_final)
                    if vk_final is not None:
                        tokens.append({"type": "special", "vk": vk_final, "name": final})
                    elif len(final) == 1:
                        tokens.append({"type": "char", "char": final})
                    for vk in reversed(mod_vks):
                        tokens.append({"type": "modifier", "vk": vk, "down": False})
            elif ch == "}":
                # Unescaped closing brace: literal }
                tokens.append({"type": "char", "char": "}"})
                i += 1
            else:
                tokens.append({"type": "char", "char": ch})
                i += 1
        return tokens

    def _post_key_string(self, hwnd: int, key_str: str) -> None:
        """Parse key_str as SendKeys syntax and post to hwnd."""
        tokens = self._parse_sendkeys_tokens(key_str)
        for token in tokens:
            t = token["type"]
            if t == "char":
                ch = token["char"]
                vk_result = user32.VkKeyScanW(ch)
                vk = vk_result & 0xFF
                modifiers = (vk_result >> 8) & 0xFF  # 1=Shift, 2=Ctrl, 4=Alt
                if vk == 0xFFFF:
                    continue  # unmappable character
                scan = user32.MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)
                # Press modifiers if needed
                shift_down = bool(modifiers & 1)
                ctrl_down = bool(modifiers & 2)
                alt_down = bool(modifiers & 4)
                if shift_down:
                    shift_scan = user32.MapVirtualKeyW(VK_SHIFT, MAPVK_VK_TO_VSC)
                    user32.PostMessageW(hwnd, WM_KEYDOWN, VK_SHIFT,
                                        self._make_lparam(scan_code=shift_scan))
                if ctrl_down:
                    ctrl_scan = user32.MapVirtualKeyW(VK_CONTROL, MAPVK_VK_TO_VSC)
                    user32.PostMessageW(hwnd, WM_KEYDOWN, VK_CONTROL,
                                        self._make_lparam(scan_code=ctrl_scan))
                if alt_down:
                    alt_scan = user32.MapVirtualKeyW(VK_MENU, MAPVK_VK_TO_VSC)
                    user32.PostMessageW(hwnd, WM_SYSKEYDOWN, VK_MENU,
                                        self._make_lparam(scan_code=alt_scan))
                # Key-down
                user32.PostMessageW(hwnd, WM_KEYDOWN, vk,
                                    self._make_lparam(scan_code=scan))
                # WM_CHAR for printable characters
                user32.PostMessageW(hwnd, WM_CHAR, ord(ch),
                                    self._make_lparam(scan_code=scan))
                # Key-up
                user32.PostMessageW(hwnd, WM_KEYUP, vk,
                                    self._make_lparam(scan_code=scan, previous=True, transition=True))
                # Release modifiers
                if shift_down:
                    user32.PostMessageW(hwnd, WM_KEYUP, VK_SHIFT,
                                        self._make_lparam(scan_code=shift_scan, previous=True, transition=True))
                if ctrl_down:
                    user32.PostMessageW(hwnd, WM_KEYUP, VK_CONTROL,
                                        self._make_lparam(scan_code=ctrl_scan, previous=True, transition=True))
                if alt_down:
                    user32.PostMessageW(hwnd, WM_SYSKEYUP, VK_MENU,
                                        self._make_lparam(scan_code=alt_scan, previous=True, transition=True))
            elif t == "special":
                vk = token["vk"]
                scan = user32.MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)
                is_extended = vk in (
                    VK_INSERT, VK_DELETE, VK_HOME, VK_END, VK_PRIOR, VK_NEXT,
                    VK_LEFT, VK_UP, VK_RIGHT, VK_DOWN,
                    VK_DIVIDE, VK_NUMLOCK,
                    VK_RCONTROL, VK_RMENU, VK_RSHIFT,
                    VK_LWIN, VK_RWIN, VK_APPS,
                )
                ext_flag = is_extended
                user32.PostMessageW(hwnd, WM_KEYDOWN, vk,
                                    self._make_lparam(scan_code=scan, extended=ext_flag))
                user32.PostMessageW(hwnd, WM_KEYUP, vk,
                                    self._make_lparam(scan_code=scan, extended=ext_flag,
                                                      previous=True, transition=True))
            elif t == "modifier":
                vk = token["vk"]
                down = token["down"]
                scan = user32.MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)
                is_extended = vk in (VK_RCONTROL, VK_RMENU, VK_RSHIFT)
                msg = WM_KEYDOWN if down else WM_KEYUP
                lparam = self._make_lparam(scan_code=scan, extended=is_extended)
                if not down:
                    lparam |= (1 << 30) | (1 << 31)  # previous=1, transition=1
                user32.PostMessageW(hwnd, msg, vk, lparam)
            # "literal_brace" tokens are emitted as {"type": "char", "char": "{"} already

    def _post_text(self, hwnd: int, text: str) -> None:
        """Post WM_CHAR messages for a plain text string."""
        for ch in text:
            vk_result = user32.VkKeyScanW(ch)
            vk = vk_result & 0xFF
            modifiers = (vk_result >> 8) & 0xFF
            if vk == 0xFFFF:
                continue
            scan = user32.MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)
            shift_down = bool(modifiers & 1)
            ctrl_down = bool(modifiers & 2)
            alt_down = bool(modifiers & 4)
            if shift_down:
                shift_scan = user32.MapVirtualKeyW(VK_SHIFT, MAPVK_VK_TO_VSC)
                user32.PostMessageW(hwnd, WM_KEYDOWN, VK_SHIFT,
                                    self._make_lparam(scan_code=shift_scan))
            if ctrl_down:
                ctrl_scan = user32.MapVirtualKeyW(VK_CONTROL, MAPVK_VK_TO_VSC)
                user32.PostMessageW(hwnd, WM_KEYDOWN, VK_CONTROL,
                                    self._make_lparam(scan_code=ctrl_scan))
            if alt_down:
                alt_scan = user32.MapVirtualKeyW(VK_MENU, MAPVK_VK_TO_VSC)
                user32.PostMessageW(hwnd, WM_SYSKEYDOWN, VK_MENU,
                                    self._make_lparam(scan_code=alt_scan))
            user32.PostMessageW(hwnd, WM_KEYDOWN, vk,
                                self._make_lparam(scan_code=scan))
            user32.PostMessageW(hwnd, WM_CHAR, ord(ch),
                                self._make_lparam(scan_code=scan))
            user32.PostMessageW(hwnd, WM_KEYUP, vk,
                                self._make_lparam(scan_code=scan, previous=True, transition=True))
            if shift_down:
                user32.PostMessageW(hwnd, WM_KEYUP, VK_SHIFT,
                                    self._make_lparam(scan_code=shift_scan, previous=True, transition=True))
            if ctrl_down:
                user32.PostMessageW(hwnd, WM_KEYUP, VK_CONTROL,
                                    self._make_lparam(scan_code=ctrl_scan, previous=True, transition=True))
            if alt_down:
                user32.PostMessageW(hwnd, WM_SYSKEYUP, VK_MENU,
                                    self._make_lparam(scan_code=alt_scan, previous=True, transition=True))

    def capture_jpeg(self) -> bytes:
        """Capture the attached window as JPEG bytes."""
        with self._capture_switch_lock:
            if self.hwnd is None:
                raise RuntimeError("no WinUI3 window attached")
            if self._capture_backend is None:
                raise RuntimeError("capture backend is not initialized")
            try:
                return self._capture_backend.capture_jpeg()
            except Exception as exc:
                if self.capture_backend_name != "wgc":
                    raise
                if not is_window(self.hwnd):
                    raise RuntimeError("target window closed during WGC capture") from exc
                self._use_imagegrab(str(exc))
                return self._capture_backend.capture_jpeg()

    async def broadcast_frame_loop(self) -> None:
        """Broadcast preview frames while clients are connected."""
        while True:
            await asyncio.sleep(FRAME_INTERVAL_SEC)
            if not self.clients:
                continue
            if self.hwnd is None:
                continue
            try:
                jpg = await asyncio.to_thread(self.capture_jpeg)
                self._frame_seq += 1
                payload = json.dumps(
                    {
                        "type": "frame",
                        "data": base64.b64encode(jpg).decode("ascii"),
                        "url": self.target_url(),
                        "title": get_window_title(self.hwnd) or self.title,
                        "seq": self._frame_seq,
                        **self.capture_metadata(),
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

    def close(self) -> None:
        """Release persistent capture resources owned by this session."""
        with self._capture_switch_lock:
            self._stop_capture_backend()
        self.overlay.clear()

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
            jpg = await asyncio.to_thread(session.capture_jpeg)
            b64 = base64.b64encode(jpg).decode("ascii")
            return {
                "type": "response",
                "request_id": payload.get("request_id"),
                "ok": True,
                "screenshot": b64,
                **session.capture_metadata(),
            }
        if cmd == "get_target":
            return {"ok": True, "target": session.target_info()}
        return {"ok": False, "error": f"unknown command: {cmd}"}
    except Exception as exc:
        return {"ok": False, "error": str(exc)}


async def run_server(host: str, port: int, project_root: Path | None) -> None:
    """Run the WinApp sidecar WebSocket server."""
    session = WinAppSession(project_root)

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
    async with websockets.serve(handler, host, port) as server:
        actual_port = server.sockets[0].getsockname()[1]
        ws_url = f"ws://{host}:{actual_port}"
        if project_root is not None:
            write_endpoint_file(project_root, mode="winapp", ws_url=ws_url)
        print(json.dumps({"ready": True, "mode": "winapp", "ws_url": ws_url}), flush=True)
        try:
            await asyncio.Future()
        finally:
            frame_task.cancel()
            session.close()


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
