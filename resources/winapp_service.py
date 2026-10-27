"""Win32/WinUI app bridge for teshi-desktop.

The service mirrors the browser bridge shape: WebSocket clients send one JSON
command and receive a typed response, while preview clients receive JPEG frames
as JSON `frame` messages. UI Automation provides element discovery/actions and
Pillow ImageGrab provides the fallback screenshot stream.
"""

from __future__ import annotations

from update_participant import register as _register_update_participant
_update_participant = _register_update_participant(__file__)

import argparse
import asyncio
import base64
import csv
import ctypes
import io
import json
import os
import secrets
import subprocess
import sys
import threading
import time
from contextlib import contextmanager, suppress
from ctypes import wintypes
from enum import Enum
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlparse

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
    from PIL import Image, ImageGrab, PngImagePlugin
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

UIA_ACTIONS = frozenset(
    {
        "click",
        "pointer_click",
        "fill",
        "select",
        "press_key",
        "assert_visible",
        "assert_text",
        "assert_not_exists",
    }
)


class IntegrityLevel(str, Enum):
    """Windows token integrity levels used by the WinApp execution boundary."""

    UNKNOWN = "Unknown"
    UNTRUSTED = "Untrusted"
    LOW = "Low"
    MEDIUM = "Medium"
    HIGH = "High"
    SYSTEM = "System"
    PROTECTED = "Protected"

    @property
    def is_elevated(self) -> bool:
        """Return whether this level is high enough to control elevated UI."""
        return self in {
            IntegrityLevel.HIGH,
            IntegrityLevel.SYSTEM,
            IntegrityLevel.PROTECTED,
        }


def parse_integrity_level(rid: int) -> IntegrityLevel:
    """Map a Windows mandatory-label RID to a stable Teshi integrity level."""
    rid = int(rid)
    if rid < 0x1000:
        return IntegrityLevel.UNTRUSTED
    if rid < 0x2000:
        return IntegrityLevel.LOW
    if rid < 0x3000:
        return IntegrityLevel.MEDIUM
    if rid < 0x4000:
        return IntegrityLevel.HIGH
    if rid < 0x5000:
        return IntegrityLevel.SYSTEM
    return IntegrityLevel.PROTECTED


def integrity_mismatch_message(
    executor: IntegrityLevel,
    target: IntegrityLevel,
) -> str | None:
    """Return an error unless executor and target integrity are exactly known and equal."""
    if executor is IntegrityLevel.UNKNOWN or target is IntegrityLevel.UNKNOWN:
        return (
            "Unable to verify the Teshi WinApp executor and target integrity levels; "
            "refusing the action."
        )
    if target.is_elevated and not executor.is_elevated:
        return (
            "Target process is elevated, but the Teshi WinApp executor is not elevated. "
            "Restart or create an elevated WinApp session."
        )
    if executor is not target:
        return (
            f"WinApp integrity mismatch: executor is {executor.value}, but target is "
            f"{target.value}; the levels must match exactly."
        )
    return None


def _websocket_path(websocket: Any) -> str:
    """Return the request path across supported websockets package versions."""
    request = getattr(websocket, "request", None)
    if request is not None:
        return str(getattr(request, "path", "/") or "/")
    return str(getattr(websocket, "path", "/") or "/")


def authenticated_websocket_path(raw_path: str, expected_token: str) -> str | None:
    """Return the route only when the WebSocket URL has exactly one valid token."""
    request_url = urlparse(raw_path)
    supplied_tokens = parse_qs(request_url.query, keep_blank_values=True).get("token", [])
    supplied_token = supplied_tokens[0] if len(supplied_tokens) == 1 else ""
    if not secrets.compare_digest(supplied_token, expected_token):
        return None
    return request_url.path


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


class MouseInput(ctypes.Structure):
    """Win32 MOUSEINPUT payload used by SendInput."""

    _fields_ = [
        ("dx", ctypes.c_long),
        ("dy", ctypes.c_long),
        ("mouseData", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.c_size_t),
    ]


class InputUnion(ctypes.Union):
    """Union payload for a Win32 INPUT structure."""

    _fields_ = [("mi", MouseInput)]


class Input(ctypes.Structure):
    """Win32 INPUT structure limited to mouse input."""

    _anonymous_ = ("union",)
    _fields_ = [
        ("type", wintypes.DWORD),
        ("union", InputUnion),
    ]


MOUSE_INPUT = 0
MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
MOUSEEVENTF_RIGHTDOWN = 0x0008
MOUSEEVENTF_RIGHTUP = 0x0010
MOUSEEVENTF_ABSOLUTE = 0x8000
MOUSEEVENTF_VIRTUALDESK = 0x4000
SM_XVIRTUALSCREEN = 76
SM_YVIRTUALSCREEN = 77
SM_CXVIRTUALSCREEN = 78
SM_CYVIRTUALSCREEN = 79

# Win32 message, mapping, and virtual-key constants are numeric data rather
# than platform handles. Keep them available when the Windows-only sidecar is
# imported on another platform for parser and behavior tests.
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

# SMTO_* flags for SendMessageTimeout.
SMTO_NORMAL = 0x0000
SMTO_ABORTIFHUNG = 0x0002

# MapVirtualKey mapping types.
MAPVK_VK_TO_VSC = 0
MAPVK_VSC_TO_VK = 1
MAPVK_VK_TO_CHAR = 2

# Virtual-key codes for special keys.
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


if os.name == "nt":
    user32 = ctypes.windll.user32
    kernel32 = ctypes.windll.kernel32
    advapi32 = ctypes.windll.advapi32
    user32.SetThreadDpiAwarenessContext.argtypes = [ctypes.c_void_p]
    user32.SetThreadDpiAwarenessContext.restype = ctypes.c_void_p
    user32.GetDpiForWindow.argtypes = [wintypes.HWND]
    user32.GetDpiForWindow.restype = ctypes.c_uint
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
    user32.GetForegroundWindow.argtypes = []
    user32.GetForegroundWindow.restype = wintypes.HWND
    user32.GetSystemMetrics.argtypes = [ctypes.c_int]
    user32.GetSystemMetrics.restype = ctypes.c_int
    user32.SendInput.argtypes = [wintypes.UINT, ctypes.POINTER(Input), ctypes.c_int]
    user32.SendInput.restype = wintypes.UINT
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
    user32.ClientToScreen.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.POINT)]
    user32.ClientToScreen.restype = ctypes.c_bool
    user32.ShowWindow.argtypes = [wintypes.HWND, ctypes.c_int]
    user32.ShowWindow.restype = ctypes.c_bool
    user32.SetWindowPos.argtypes = [wintypes.HWND, wintypes.HWND, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int, wintypes.UINT]
    user32.SetWindowPos.restype = ctypes.c_bool
    user32.ScreenToClient.argtypes = [wintypes.HWND, ctypes.POINTER(ctypes.c_long * 2)]
    user32.ScreenToClient.restype = ctypes.c_bool

    kernel32.GetCurrentProcess.argtypes = []
    kernel32.GetCurrentProcess.restype = wintypes.HANDLE
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL
    advapi32.OpenProcessToken.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.HANDLE),
    ]
    advapi32.OpenProcessToken.restype = wintypes.BOOL
    advapi32.GetTokenInformation.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        ctypes.c_void_p,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
    ]
    advapi32.GetTokenInformation.restype = wintypes.BOOL
    advapi32.GetSidSubAuthorityCount.argtypes = [ctypes.c_void_p]
    advapi32.GetSidSubAuthorityCount.restype = ctypes.POINTER(ctypes.c_ubyte)
    advapi32.GetSidSubAuthority.argtypes = [ctypes.c_void_p, wintypes.DWORD]
    advapi32.GetSidSubAuthority.restype = ctypes.POINTER(wintypes.DWORD)

else:  # pragma: no cover - this sidecar is Windows-only
    user32 = None
    kernel32 = None
    advapi32 = None


PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
TOKEN_QUERY = 0x0008
TOKEN_INTEGRITY_LEVEL = 25


def _token_integrity_rid(token: Any) -> int | None:
    """Read the final sub-authority from a Windows token mandatory label."""
    if advapi32 is None:
        return None
    required = wintypes.DWORD()
    advapi32.GetTokenInformation(
        token,
        TOKEN_INTEGRITY_LEVEL,
        None,
        0,
        ctypes.byref(required),
    )
    if required.value <= 0:
        return None
    buffer = ctypes.create_string_buffer(required.value)
    if not advapi32.GetTokenInformation(
        token,
        TOKEN_INTEGRITY_LEVEL,
        buffer,
        required.value,
        ctypes.byref(required),
    ):
        return None
    sid = ctypes.cast(buffer, ctypes.POINTER(ctypes.c_void_p)).contents.value
    if not sid:
        return None
    count = advapi32.GetSidSubAuthorityCount(ctypes.c_void_p(sid))
    if not count or count.contents.value == 0:
        return None
    rid = advapi32.GetSidSubAuthority(
        ctypes.c_void_p(sid), count.contents.value - 1
    )
    return int(rid.contents.value) if rid else None


def process_integrity_level(pid: int) -> IntegrityLevel:
    """Return a process token's integrity level, or ``Unknown`` on API failure."""
    if os.name != "nt" or kernel32 is None or advapi32 is None:
        return IntegrityLevel.UNKNOWN
    try:
        pid = int(pid)
    except (TypeError, ValueError):
        return IntegrityLevel.UNKNOWN
    if pid <= 0:
        return IntegrityLevel.UNKNOWN

    is_current = pid == os.getpid()
    process = (
        kernel32.GetCurrentProcess()
        if is_current
        else kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    )
    if not process:
        return IntegrityLevel.UNKNOWN
    token = wintypes.HANDLE()
    try:
        if not advapi32.OpenProcessToken(process, TOKEN_QUERY, ctypes.byref(token)):
            return IntegrityLevel.UNKNOWN
        rid = _token_integrity_rid(token)
        return parse_integrity_level(rid) if rid is not None else IntegrityLevel.UNKNOWN
    finally:
        if token:
            kernel32.CloseHandle(token)
        if not is_current:
            kernel32.CloseHandle(process)


def current_process_integrity() -> IntegrityLevel:
    """Return the integrity level of the WinApp executor process itself."""
    return process_integrity_level(os.getpid())

# Map of SendKeys-style key names to virtual-key codes.
# Supports the same {Name} syntax as uiautomation.SendKeys.
KEY_NAME_TO_VK: dict[str, int] = {
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
    get_window_thread_process_id = getattr(user32, "GetWindowThreadProcessId", None)
    if get_window_thread_process_id is None:
        return 0
    pid = wintypes.DWORD()
    get_window_thread_process_id(hwnd, ctypes.byref(pid))
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


@contextmanager
def physical_coordinates():
    """Scope DPI awareness to visual commands without changing legacy UIA actions."""
    if user32 is None:
        yield
        return
    previous = user32.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4))
    if not previous:
        raise RuntimeError("cannot establish physical screen coordinate context")
    try:
        yield
    finally:
        if not user32.SetThreadDpiAwarenessContext(previous):
            raise RuntimeError("cannot restore thread DPI awareness context")


def normalized_absolute_coordinate(value: int, origin: int, span: int) -> int:
    """Map a physical virtual-desktop coordinate to SendInput's absolute range."""
    if span <= 1:
        raise RuntimeError("virtual desktop has invalid dimensions")
    normalized = round((value - origin) * 65535 / (span - 1))
    return max(0, min(65535, normalized))


def virtual_desktop_coordinate(x: int, y: int) -> tuple[int, int]:
    """Convert physical screen coordinates, including negative origins, for SendInput."""
    if user32 is None:
        raise RuntimeError("SendInput requires Windows user32")
    left = int(user32.GetSystemMetrics(SM_XVIRTUALSCREEN))
    top = int(user32.GetSystemMetrics(SM_YVIRTUALSCREEN))
    width = int(user32.GetSystemMetrics(SM_CXVIRTUALSCREEN))
    height = int(user32.GetSystemMetrics(SM_CYVIRTUALSCREEN))
    return (
        normalized_absolute_coordinate(x, left, width),
        normalized_absolute_coordinate(y, top, height),
    )


def mouse_input(dx: int, dy: int, flags: int) -> Input:
    """Build one mouse INPUT event for a pointer move or button transition."""
    event = Input()
    event.type = MOUSE_INPUT
    event.mi = MouseInput(dx, dy, 0, flags, 0, 0)
    return event


def send_pointer_click(x: int, y: int, count: int = 1, button: str = "left") -> None:
    """Move the system pointer and inject complete button clicks.

    ``count`` > 1 produces a multi-click sequence (2 = double click), which
    some shell surfaces such as notification-area icons require.
    """
    if user32 is None:
        raise RuntimeError("pointer_click requires Windows SendInput")
    if button not in ("left", "right"):
        raise RuntimeError(f"unsupported pointer button: {button!r}")
    if count < 1:
        raise RuntimeError("pointer click count must be >= 1")
    down_flag = MOUSEEVENTF_LEFTDOWN if button == "left" else MOUSEEVENTF_RIGHTDOWN
    up_flag = MOUSEEVENTF_LEFTUP if button == "left" else MOUSEEVENTF_RIGHTUP
    absolute_x, absolute_y = virtual_desktop_coordinate(x, y)
    events = [
        mouse_input(
            absolute_x,
            absolute_y,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        )
    ]
    for _ in range(count):
        events.append(mouse_input(0, 0, down_flag))
        events.append(mouse_input(0, 0, up_flag))
    inputs = (Input * len(events))(*events)
    injected = int(user32.SendInput(len(events), inputs, ctypes.sizeof(Input)))
    if injected != len(events):
        raise RuntimeError(
            f"SendInput failed: injected {injected} of {len(events)} pointer events"
        )


def get_capture_bounds(hwnd: int) -> tuple[int, int, int, int] | None:
    """WGC window frames cover DWM extended frame bounds, not client bounds."""
    if os.name != "nt":
        return None
    rect = Rect()
    result = ctypes.windll.dwmapi.DwmGetWindowAttribute(
        wintypes.HWND(hwnd), 9, ctypes.byref(rect), ctypes.sizeof(rect)
    )
    if result != 0:
        return None
    return rect.left, rect.top, rect.right, rect.bottom


def crop_element(image: Any, frame_bounds: tuple[int, int, int, int], bounds: dict) -> Any:
    """Translate physical UIA screen pixels into an exact, unscaled frame crop."""
    left, top, right, bottom = frame_bounds
    if image.size != (right - left, bottom - top):
        raise RuntimeError("capture dimensions do not match physical screen bounds")
    x, y, width, height = (bounds[k] for k in ("x", "y", "width", "height"))
    if any(int(v) != v for v in (x, y, width, height)):
        raise RuntimeError("non-integral UIA bounds cannot be mapped reliably")
    x, y, width, height = map(int, (x, y, width, height))
    if width <= 0 or height <= 0 or x < left or y < top or x + width > right or y + height > bottom:
        raise RuntimeError("element bounds are outside the captured window")
    return image.crop((x - left, y - top, x - left + width, y - top + height)).convert("RGB")


def resolve_screenshot_element(snapshot: dict, selector: str) -> dict:
    """Resolve uniquely among visible interactive nodes without changing action lookup."""
    criteria = split_selector(selector)
    allowed = {"path", "name", "automation_id", "control_type", "class_name"}
    if not criteria or not set(criteria) <= allowed:
        raise RuntimeError("invalid screenshot selector")
    if snapshot.get("truncated"):
        raise RuntimeError("UIA snapshot is incomplete; uniqueness cannot be established")
    matches = [node for node in snapshot["interactive_elements"]
               if node.get("is_offscreen") is False
               and all(str(node.get(k, "")).casefold() == v.casefold() for k, v in criteria.items())]
    if len(matches) != 1:
        raise RuntimeError(f"screenshot selector matched {len(matches)} visible interactive elements")
    return matches[0]


def compare_pixels(baseline: Any, actual: Any, tolerance: int) -> tuple[int, Any]:
    """Count each pixel once when any RGB channel exceeds the inclusive tolerance."""
    if type(tolerance) is not int or not 0 <= tolerance <= 255:
        raise ValueError("pixel_tolerance must be an integer from 0 to 255")
    width, height = max(baseline.width, actual.width), max(baseline.height, actual.height)
    diff = Image.new("RGB", (width, height))
    base, current, output = baseline.convert("RGB").load(), actual.convert("RGB").load(), diff.load()
    changed = 0
    for y in range(height):
        for x in range(width):
            if (x >= baseline.width or y >= baseline.height) and (x >= actual.width or y >= actual.height):
                continue
            missing = x >= baseline.width or y >= baseline.height or x >= actual.width or y >= actual.height
            mismatch = missing or any(abs(a - b) > tolerance for a, b in zip(base[x, y], current[x, y]))
            if mismatch:
                changed += 1
                output[x, y] = (255, 0, 255)
    return changed, diff


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

    def capture_rgb(self) -> tuple[Any, tuple[int, int, int, int]]:
        """Capture physical screen pixels; Pillow's GDI path does not draw cursors."""
        if ImageGrab is None:
            raise RuntimeError("Pillow is unavailable for lossless screen capture")
        bbox = get_window_rect(self.hwnd)
        if bbox is None:
            raise RuntimeError("target window has no valid bounds")
        image = ImageGrab.grab(bbox=bbox, all_screens=True).convert("RGB")
        if get_window_rect(self.hwnd) != bbox:
            raise RuntimeError("window moved during capture")
        return image, bbox


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
        self._capture_factory = factory
        self._lock = threading.Lock()
        self._first_frame = threading.Event()
        self._latest_jpeg: bytes | None = None
        self._latest_rgb: Any = None
        self._latest_bounds: Any = None
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
                    self._latest_rgb = image
                    self._latest_bounds = get_capture_bounds(hwnd)
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

    def capture_rgb(self) -> tuple[Any, tuple[int, int, int, int]]:
        """Restart the same backend to obtain a fresh frame even for static windows."""
        # WGC emits only on damage. Waiting for another preview frame can time
        # out on a static window, while reusing it can compare stale pixels.
        bounds_before = get_capture_bounds(self.hwnd)
        self.stop()
        self.__init__(self.hwnd, capture_factory=self._capture_factory)
        self.capture_jpeg()
        with self._lock:
            if self._latest_bounds is None:
                raise RuntimeError("cannot establish WGC screen bounds")
            if bounds_before != self._latest_bounds or get_capture_bounds(self.hwnd) != self._latest_bounds:
                raise RuntimeError("window moved during WGC capture")
            return self._latest_rgb.copy(), self._latest_bounds

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


# --- Windows shell (taskbar notification area) automation ---
#
# WHY: notification-area icons belong to Explorer's shell surface, not to any
# attached application window, so they are discovered through the shell's own
# UIA tree instead of the attached session's root_control(). Shell operations
# never mutate the attach session.

TASKBAR_WINDOW_CLASS = "Shell_TrayWnd"
TRAY_BRIDGE_CLASS_FRAGMENT = "DesktopWindowContentBridge"
TRAY_ICON_AUTOMATION_ID = "NotifyItemIcon"
OVERFLOW_WINDOW_CLASS = "TopLevelWindowForOverflowXamlIsland"
NOTIFY_ICON_UID_PROBE_LIMIT = 8
MAX_SHELL_TREE_DEPTH = 16
WAIT_WINDOW_POLL_INTERVAL_SEC = 0.25


class NotifyIconIdentifier(ctypes.Structure):
    """NOTIFYICONIDENTIFIER for Shell_NotifyIconGetRect."""

    _fields_ = [
        ("cbSize", wintypes.DWORD),
        ("hWnd", wintypes.HWND),
        ("uID", wintypes.UINT),
        ("guidItem", ctypes.c_byte * 16),
    ]


def shell_notify_icon_rect(hwnd: int, uid: int) -> tuple[int, int, int, int] | None:
    """Return the physical screen rectangle of one notification icon.

    WHY: Shell_NotifyIconGetRect is the supported shell API that maps a
    (window, icon id) pair to its on-screen tray position. It reports failure
    as E_FAIL, which ctypes surfaces as OSError, when the pair owns no icon.
    """
    if os.name != "nt":
        return None
    try:
        get_rect = ctypes.windll.shell32.Shell_NotifyIconGetRect
    except AttributeError:
        return None
    get_rect.restype = ctypes.HRESULT
    get_rect.argtypes = [ctypes.POINTER(NotifyIconIdentifier), ctypes.POINTER(Rect)]
    identifier = NotifyIconIdentifier(
        cbSize=ctypes.sizeof(NotifyIconIdentifier), hWnd=hwnd, uID=uid
    )
    rect = Rect()
    try:
        hr = get_rect(ctypes.byref(identifier), ctypes.byref(rect))
    except OSError:
        return None
    if hr != 0 or rect.right <= rect.left or rect.bottom <= rect.top:
        return None
    return rect.left, rect.top, rect.right, rect.bottom


def notify_icon_rect_for_pid(pid: int) -> tuple[int, int, int, int] | None:
    """Find the tray icon rectangle owned by any window of ``pid``.

    WHY: apps register tray icons against a hidden message window, so every
    top-level window of the process is probed with the small uID range apps
    conventionally use. A wrong pair fails cheaply with E_FAIL.
    """
    for hwnd in windows_of_pid(pid):
        for uid in range(NOTIFY_ICON_UID_PROBE_LIMIT):
            rect = shell_notify_icon_rect(hwnd, uid)
            if rect is not None:
                return rect
    return None


def windows_of_pid(pid: int) -> list[int]:
    """All top-level window handles owned by ``pid``, visible or not."""
    if user32 is None:
        return []
    found: list[int] = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)
    def callback(hwnd: int, _lparam: int) -> bool:
        window_pid = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(window_pid))
        if window_pid.value == pid:
            found.append(hwnd)
        return True

    user32.EnumWindows(callback, 0)
    return found


def find_taskbar_bridge_hwnd() -> int | None:
    """Locate the taskbar XAML content bridge window.

    WHY: Windows 11 renders the taskbar as a XAML island and exposes
    notification-area icons as UIA ButtonControls (AutomationId
    ``NotifyItemIcon``) under a ``DesktopWindowContentBridge`` child of
    ``Shell_TrayWnd``. The legacy TrayNotifyWnd/ToolbarWindow32 chain no
    longer carries UIA children, so the bridge is the discovery path.
    """
    if user32 is None:
        return None
    taskbar = user32.FindWindowW(TASKBAR_WINDOW_CLASS, None)
    if not taskbar:
        return None
    found: list[int] = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)
    def callback(hwnd: int, _lparam: int) -> bool:
        class_name = ctypes.create_unicode_buffer(512)
        user32.GetClassNameW(hwnd, class_name, 512)
        if TRAY_BRIDGE_CLASS_FRAGMENT in class_name.value:
            found.append(hwnd)
            return False
        return True

    user32.EnumChildWindows(taskbar, callback, 0)
    return found[0] if found else None


def find_overflow_window() -> int | None:
    """Return the visible hidden-icons overflow flyout window, if open."""
    if user32 is None:
        return None
    found: list[int] = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)
    def callback(hwnd: int, _lparam: int) -> bool:
        if not user32.IsWindowVisible(hwnd):
            return True
        class_name = ctypes.create_unicode_buffer(512)
        user32.GetClassNameW(hwnd, class_name, 512)
        if class_name.value == OVERFLOW_WINDOW_CLASS:
            found.append(hwnd)
        return True

    user32.EnumWindows(callback, 0)
    return found[0] if found else None


def _tray_icon_buttons(root: Any) -> list[dict[str, Any]]:
    """Collect NotifyItemIcon buttons from one UIA subtree."""
    icons: list[dict[str, Any]] = []

    def walk(control: Any, depth: int) -> None:
        if depth > MAX_SHELL_TREE_DEPTH:
            return
        try:
            if (
                get_control_property(control, "ControlTypeName") == "ButtonControl"
                and get_control_property(control, "AutomationId")
                == TRAY_ICON_AUTOMATION_ID
            ):
                rect = rect_to_dict(get_control_property(control, "BoundingRectangle"))
                icons.append(
                    {
                        "name": get_control_property(control, "Name") or "",
                        "class_name": get_control_property(control, "ClassName") or "",
                        "rect": (
                            (rect["left"], rect["top"], rect["right"], rect["bottom"])
                            if rect
                            else None
                        ),
                    }
                )
        except Exception:
            return
        try:
            children = control.GetChildren()
        except Exception:
            return
        for child in children:
            walk(child, depth + 1)

    walk(root, 0)
    return icons


@physical_coordinates()
def shell_tray_icons() -> dict[str, Any]:
    """Enumerate notification-area icons from the taskbar and open overflow.

    The response lists every icon with its tooltip name, UIA class, physical
    rectangle, and area (``notification`` or ``overflow``). Icons inside an
    already-open overflow flyout are included; the flyout is never opened
    here because that requires physical pointer input.
    """
    if auto is None:
        return {
            "ok": False,
            "error": "uiautomation is not installed; run `pip install -r python/requirements.txt`",
        }
    bridge = find_taskbar_bridge_hwnd()
    if bridge is None:
        return {
            "ok": False,
            "error": (
                "taskbar XAML bridge window not found; the Windows shell taskbar "
                "is unavailable or its UIA surface changed"
            ),
        }
    icons: list[dict[str, Any]] = []
    try:
        root = auto.ControlFromHandle(bridge)
        for icon in _tray_icon_buttons(root):
            icons.append({**icon, "area": "notification"})
    except Exception as exc:
        return {"ok": False, "error": f"UIA traversal of the taskbar failed: {exc}"}
    overflow = find_overflow_window()
    overflow_open = overflow is not None
    if overflow_open:
        try:
            overflow_root = auto.ControlFromHandle(overflow)
            for icon in _tray_icon_buttons(overflow_root):
                icons.append({**icon, "area": "overflow"})
        except Exception as exc:
            return {
                "ok": False,
                "error": f"UIA traversal of the overflow flyout failed: {exc}",
                "icons": icons,
            }
    return {"ok": True, "icons": icons, "overflow_open": overflow_open}


@physical_coordinates()
def reveal_autohidden_taskbar() -> None:
    """Move to the taskbar's exposed edge before resolving its icon bounds."""
    taskbar = user32.FindWindowW(TASKBAR_WINDOW_CLASS, None)
    rect = Rect()
    if not taskbar or not user32.GetWindowRect(taskbar, ctypes.byref(rect)):
        return
    left = user32.GetSystemMetrics(SM_XVIRTUALSCREEN)
    top = user32.GetSystemMetrics(SM_YVIRTUALSCREEN)
    right = left + user32.GetSystemMetrics(SM_CXVIRTUALSCREEN)
    bottom = top + user32.GetSystemMetrics(SM_CYVIRTUALSCREEN)
    if rect.left >= left and rect.top >= top and rect.right <= right and rect.bottom <= bottom:
        return
    x = max(left, min(right - 1, (rect.left + rect.right) // 2))
    y = max(top, min(bottom - 1, (rect.top + rect.bottom) // 2))
    absolute_x, absolute_y = virtual_desktop_coordinate(x, y)
    event = mouse_input(absolute_x, absolute_y,
                        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK)
    if user32.SendInput(1, ctypes.byref(event), ctypes.sizeof(Input)) != 1:
        raise RuntimeError("SendInput failed while revealing the taskbar")
    time.sleep(0.5)


@physical_coordinates()
def _open_tray_overflow(timeout_s: float) -> int | None:
    """Best-effort: click the show-hidden-icons chevron and wait for the flyout.

    WHY: the chevron only exists in the UIA tree while icons are actually
    hidden, and its label is localized, so it is matched by the shell's
    language-independent class pattern instead of a display name. Returns the
    overflow window handle once it opens, or None when the chevron cannot be
    identified or the flyout does not appear.
    """
    if auto is None:
        return None
    bridge = find_taskbar_bridge_hwnd()
    if bridge is None:
        return None
    try:
        root = auto.ControlFromHandle(bridge)
    except Exception:
        return None
    chevron: Any = None

    def walk(control: Any, depth: int) -> None:
        nonlocal chevron
        if depth > MAX_SHELL_TREE_DEPTH or chevron is not None:
            return
        try:
            if (
                get_control_property(control, "ControlTypeName") == "ButtonControl"
                and (
                    "chevron" in (get_control_property(control, "ClassName") or "").casefold()
                    or (
                        get_control_property(control, "AutomationId") == "SystemTrayIcon"
                        and any(get_control_property(child, "Name") == "\ue971"
                                for child in control.GetChildren())
                    )
                )
            ):
                chevron = control
                return
        except Exception:
            return
        try:
            children = control.GetChildren()
        except Exception:
            return
        for child in children:
            walk(child, depth + 1)

    walk(root, 0)
    if chevron is None:
        return None
    rect = rect_to_dict(get_control_property(chevron, "BoundingRectangle"))
    if not rect or rect["width"] <= 0 or rect["height"] <= 0:
        return None
    try:
        with physical_coordinates():
            send_pointer_click(
                rect["left"] + rect["width"] // 2,
                rect["top"] + rect["height"] // 2,
            )
    except Exception:
        return None
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        overflow = find_overflow_window()
        if overflow is not None:
            return overflow
        time.sleep(WAIT_WINDOW_POLL_INTERVAL_SEC)
    return None


def process_window_state(pid: int) -> dict[str, Any]:
    """Summarize the visibility state of a process's top-level windows.

    A window counts as visible only when Win32 reports it visible and not
    minimized; hidden-to-tray windows report visible=False. The first
    visible window handle is returned so callers can detect HWND changes
    after a window is destroyed and recreated.
    """
    if user32 is None:
        return {"visible": False, "hwnd": None, "title": None, "windows": []}
    windows: list[dict[str, Any]] = []
    visible_hwnd: int | None = None
    for hwnd in windows_of_pid(pid):
        entry = {
            "hwnd": hwnd,
            "title": get_window_title(hwnd),
            "visible": bool(user32.IsWindowVisible(hwnd)),
            "minimized": bool(user32.IsIconic(hwnd)),
        }
        windows.append(entry)
        if entry["visible"] and not entry["minimized"] and visible_hwnd is None:
            visible_hwnd = hwnd
    return {
        "visible": visible_hwnd is not None,
        "hwnd": visible_hwnd,
        "title": get_window_title(visible_hwnd) if visible_hwnd else None,
        "windows": windows,
    }


def element_visibility_error(control: Any, owner_hwnd: int | None = None) -> str | None:
    """Return why a UIA element is not genuinely visible, or None when visible.

    WHY: a hidden or minimized window can still expose non-zero UIA
    BoundingRectangle values, so rectangle checks alone produce false
    positives. IsOffscreen is the UIA visibility contract; the owning window
    is additionally verified against Win32 IsWindowVisible/IsIconic. When the
    UIA provider cannot answer, the check fails instead of guessing.
    """
    offscreen = get_control_property(control, "IsOffscreen")
    if offscreen is None:
        return "UIA provider cannot determine IsOffscreen for this element"
    if offscreen is True:
        return "element is offscreen per UIA IsOffscreen"
    rect = rect_to_dict(get_control_property(control, "BoundingRectangle"))
    if not rect or rect["width"] <= 0 or rect["height"] <= 0:
        return "element has no visible UIA bounds"
    hwnd = owner_hwnd or get_control_property(control, "NativeWindowHandle")
    if hwnd and user32 is not None:
        hwnd = int(hwnd)
        if not user32.IsWindowVisible(hwnd):
            return "owning window is hidden (IsWindowVisible is false)"
        if user32.IsIconic(hwnd):
            return "owning window is minimized"
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


class ControlNotFoundError(RuntimeError):
    """The UIA tree was fully traversed without finding a matching control."""


class ControlTraversalError(RuntimeError):
    """The UIA provider failed while the tree was being traversed."""


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


class RequiresInteractiveInput(RuntimeError):
    """The requested action needs explicit permission to disturb the desktop."""


class ActionResultUncertain(Exception):
    """An action was dispatched; retrying could duplicate its effects."""


def interactive_input_error(action: str) -> dict[str, Any]:
    """Describe the request-scoped permission required for physical input."""
    return {"ok": False, "code": "requires_interactive_input", "action": action,
            "error": f"{action} requires --mode foreground --allow-interactive-input; "
                     "physical input is forbidden in background mode and auto mode",
            "required_mode": "foreground", "required_permission": "allow_interactive_input"}


def interactive_input_allowed(payload: dict[str, Any]) -> bool:
    """Never coerce strings or replay confirmation into input permission."""
    return (str(payload.get("mode") or "auto").casefold() == "foreground"
            and payload.get("allow_interactive_input") is True)


def configure_nonintrusive_uia_client(backend: Any, create_object: Any) -> Any:
    """Disable UIA's implicit focus on the client that resolves our elements."""
    shared = backend._AutomationClient.instance()
    interface = shared.UIAutomationCore.IUIAutomation2
    try:
        client = shared.IUIAutomation.QueryInterface(interface)
    except Exception:
        # The legacy CUIAutomation class cannot expose the Windows 8 focus policy.
        client = create_object("{e22ad333-b25f-460c-83d0-0581107395c9}", interface=interface)
    client.AutoSetFocus = False
    if bool(client.AutoSetFocus):
        raise RuntimeError("UIA AutoSetFocus remained enabled")
    walker = client.RawViewWalker
    shared.IUIAutomation = client
    shared.ViewWalker = walker
    return client


def ensure_nonintrusive_uia() -> None:
    """Fail before mutation when implicit UIA focus cannot be disabled."""
    try:
        import comtypes.client
        from uiautomation import uiautomation as backend

        configure_nonintrusive_uia_client(backend, comtypes.client.CreateObject)
    except Exception as exc:
        raise RequiresInteractiveInput(
            f"cannot disable implicit UIA focus before mutation: {exc}"
        ) from exc


class WinAppSession:
    """State for one attached native application window."""

    def __init__(self, project_root: Path | None) -> None:
        self.project_root = project_root
        self.hwnd: int | None = None
        self.target_pid: int | None = None
        self.title = ""
        self.process: subprocess.Popen[str] | None = None
        self.clients: set[Any] = set()
        self.overlay = HighlightOverlay()
        self._frame_seq = 0
        self._last_error = ""
        self._capture_switch_lock = threading.RLock()
        self._capture_backend: WgcCaptureBackend | ImageGrabCaptureBackend | None = None
        self._capture_fallback_reason = ""
        self.instances: dict[str, subprocess.Popen] = {}
        self.observations: dict[str, dict] = {}

    def instance_command(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Operate only on process handles launched and owned by this executor."""
        action = payload.get("action", "status")
        token = str(payload.get("instance") or "")
        if action == "launch":
            argv = payload.get("argv")
            if not isinstance(argv, list) or not argv or not all(isinstance(v, str) for v in argv):
                return {"ok": False, "code": "invalid_argv"}
            process = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                       stderr=subprocess.DEVNULL, shell=False)
            token = secrets.token_hex(16)
            self.instances[token] = process
        process = self.instances.get(token)
        if process is None:
            return {"ok": False, "code": "instance_unknown", "error": "Instance ownership lost or unknown; no PID-based fallback"}
        if action == "stop" and process.poll() is None:
            process.terminate()
        elif action not in ("launch", "status", "wait", "stop"):
            return {"ok": False, "code": "unsupported_instance_action"}
        if action in ("wait", "stop"):
            try:
                process.wait(timeout=min(max(float(payload.get("wait_ms", 1000)), 1), 10000) / 1000)
            except subprocess.TimeoutExpired:
                return {"ok": False, "code": "instance_wait_timeout", "instance": token,
                        "pid": process.pid, "business_result": "unverified", "retry_safe": action == "wait"}
        exit_code = process.poll()
        # Popen retains the original process handle even after the PID is reused.
        windows = windows_of_pid(process.pid) if exit_code is None else []
        exit_code = process.poll()
        if exit_code is not None:
            windows = []
        passed = ("expected_exit_code" not in payload or exit_code == payload["expected_exit_code"])
        passed &= ("expected_windows" not in payload or len(windows) == payload["expected_windows"])
        passed &= ("expected_alive" not in payload or (exit_code is None) == payload["expected_alive"])
        return {"ok": passed, "instance": token, "pid": process.pid, "alive": exit_code is None,
                "exit_code": exit_code, "window_count": len(windows), "windows": windows,
                "business_result": "passed" if passed and any(k.startswith("expected_") for k in payload) else "failed" if not passed else "unverified"}

    def external_check(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Run an explicitly supplied checker with JSON stdin and a strict result contract."""
        argv = payload.get("argv")
        if not isinstance(argv, list) or not argv or not all(isinstance(v, str) for v in argv):
            return {"ok": False, "code": "invalid_checker_argv"}
        timeout = min(max(float(payload.get("check_timeout_ms", 5000)), 1), 30000) / 1000
        process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True, encoding="utf-8", shell=False,
                                   cwd=self.project_root)
        try:
            stdout, stderr = process.communicate(json.dumps({"version": 1, "input": payload.get("input", {})}), timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            try:
                stdout, stderr = process.communicate(timeout=1)
            except subprocess.TimeoutExpired:
                stdout, stderr = "", "Checker descendants retained output pipes"
                if process.stdout: process.stdout.close()
                if process.stderr: process.stderr.close()
            root = (self.project_root or Path.cwd()) / ".teshi/artifacts/checks"
            root.mkdir(parents=True, exist_ok=True)
            evidence = root / f"{secrets.token_hex(16)}.json"
            response = {"ok": False, "code": "checker_timeout", "business_result": "unverified", "retry_safe": False}
            evidence.write_text(json.dumps({**response, "stdout": stdout[:1024*1024], "stderr": stderr[:65536]}), encoding="utf-8")
            return {**response, "evidence": str(evidence)}
        root = (self.project_root or Path.cwd()) / ".teshi/artifacts/checks"
        root.mkdir(parents=True, exist_ok=True)
        evidence = root / f"{secrets.token_hex(16)}.json"
        try:
            result = json.loads(stdout) if len(stdout) <= 1024 * 1024 else None
            if not isinstance(result, dict) or result.get("business_result") not in ("passed", "failed", "blocked", "unverified"):
                raise ValueError("checker must return a structured business_result")
            if result["business_result"] == "passed" and result.get("thresholds_verified") is not True:
                raise ValueError("uncalibrated thresholds cannot report passed")
            ok = process.returncode == 0 and result["business_result"] == "passed"
            response = {"ok": ok, "exit_code": process.returncode, "result": result,
                        "business_result": result["business_result"] if process.returncode == 0 else "failed"}
        except (ValueError, TypeError) as exc:
            response = {"ok": False, "code": "invalid_checker_result", "error": str(exc),
                        "exit_code": process.returncode, "business_result": "unverified"}
        evidence.write_text(json.dumps({**response, "stdout": stdout[:1024*1024], "stderr": stderr[:65536]}), encoding="utf-8")
        return {**response, "evidence": str(evidence)}

    def window_geometry(self) -> dict[str, Any]:
        """Physical client coordinates bound to a process incarnation and window."""
        identity = native_target_identity(self.hwnd)
        if identity is None:
            raise RuntimeError("target identity is unavailable")
        rect, origin = Rect(), wintypes.POINT()
        if not user32.GetClientRect(self.hwnd, ctypes.byref(rect)) or not user32.ClientToScreen(self.hwnd, ctypes.byref(origin)):
            raise RuntimeError("cannot read client geometry")
        return {"target": identity, "window_bounds": get_window_rect(self.hwnd),
                "origin": [origin.x, origin.y], "width": rect.right, "height": rect.bottom,
                "dpi": int(user32.GetDpiForWindow(self.hwnd)), "units": "physical_client_pixels"}

    def observe_window(self, scope: str = "client", region: Any = None) -> dict[str, Any]:
        """Capture without requiring a UIA element; reject geometry changes during capture."""
        with physical_coordinates():
            before = self.window_geometry()
            with self._capture_switch_lock:
                if self._capture_backend is None:
                    raise RuntimeError("capture backend unavailable")
                image, bounds = self._capture_backend.capture_rgb()
            if before != self.window_geometry():
                raise RuntimeError("window changed during capture; observe again")
            if scope == "window":
                crop = list(bounds)
            elif scope in ("client", "region"):
                x, y = before["origin"]
                left, top, width, height = (0, 0, before["width"], before["height"])
                if scope == "region":
                    if not isinstance(region, list) or len(region) != 4 or not all(type(v) is int for v in region):
                        raise ValueError("region must be [x,y,width,height] in client pixels")
                    left, top, width, height = region
                if min(left, top) < 0 or min(width, height) <= 0 or left + width > before["width"] or top + height > before["height"]:
                    raise ValueError("capture region is outside the client")
                crop = [x + left, y + top, x + left + width, y + top + height]
            else:
                raise ValueError("scope must be window, client or region")
            if crop[0] < bounds[0] or crop[1] < bounds[1] or crop[2] > bounds[2] or crop[3] > bounds[3]:
                raise RuntimeError("capture does not contain the requested region")
            image = image.crop((crop[0]-bounds[0], crop[1]-bounds[1], crop[2]-bounds[0], crop[3]-bounds[1]))
            token = secrets.token_hex(16)
            root = (self.project_root or Path.cwd()) / ".teshi/artifacts/visual"
            root.mkdir(parents=True, exist_ok=True)
            path = root / f"{token}.png"
            image.save(path, format="PNG")
            self.observations = {token: before}
            return {"ok": True, "observation": token, **before, "screenshot": str(path),
                    "capture_origin": crop[:2], "dimensions": list(image.size), "time": time.time(),
                    **self.capture_metadata(), "business_result": "unverified"}

    def visual_plan(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Bounded sequential exploration; never retry input or create bindings."""
        steps = payload.get("steps")
        if not isinstance(steps, list) or not 1 <= len(steps) <= 32:
            return {"ok": False, "code": "invalid_plan", "error": "Provide 1..32 sequential steps"}
        supported = {"observe", "wait_stable", "click", "double_click", "scroll", "drag", "keys", "text", "maximize", "restore", "resize"}
        if any(not isinstance(s, dict) or s.get("action") not in supported for s in steps):
            return {"ok": False, "code": "unsupported_visual_action"}
        if any(s["action"] not in ("observe", "wait_stable") for s in steps) and not interactive_input_allowed(payload):
            return interactive_input_error("visual_plan")
        if error := self.integrity_guard("visual_plan"):
            return error
        results = []
        root = (self.project_root or Path.cwd()) / ".teshi/artifacts/visual"
        root.mkdir(parents=True, exist_ok=True)
        journal = root / f"plan-{secrets.token_hex(16)}.jsonl"
        observation = payload.get("observation")
        for index, step in enumerate(steps):
            started, dispatched = time.monotonic(), False
            # Persist intent before native dispatch. A supervisor timeout can kill
            # this executor; an unmatched start is an uncertain step, never a retry cue.
            with journal.open("a", encoding="utf-8") as out:
                out.write(json.dumps({"phase": "started", "index": index,
                    "action": step["action"], "observation": observation,
                    "target": native_target_identity(self.hwnd), "business_result": "unverified"}) + "\n")
                out.flush()
                os.fsync(out.fileno())
            try:
                with physical_coordinates():
                    action = step["action"]
                    if action == "wait_stable":
                        deadline = time.monotonic() + min(max(float(step.get("wait_ms", 2000)), 100), 5000) / 1000
                        previous, matches = None, 0
                        while time.monotonic() < deadline:
                            result = self.observe_window()
                            pixels = Path(result["screenshot"]).read_bytes()
                            matches = matches + 1 if pixels == previous else 0
                            if matches >= 2:
                                break
                            previous = pixels
                            time.sleep(0.1)
                        else:
                            raise RuntimeError("visual stability deadline exceeded")
                        result["stable"] = True
                        observation = result["observation"]
                    elif action == "observe":
                        result = self.observe_window(step.get("scope", "client"), step.get("region"))
                        observation = result["observation"]
                    else:
                        geometry = self.observations.get(observation)
                        if geometry is None or geometry != self.window_geometry():
                            raise ValueError("stale observation; capture again before input")
                        self._activate_foreground(action)
                        if geometry != self.window_geometry():
                            raise ValueError("activation changed window geometry; capture again")
                        if action in ("click", "double_click", "scroll", "drag"):
                            x, y = step.get("x"), step.get("y")
                            if type(x) is not int or type(y) is not int or not 0 <= x < geometry["width"] or not 0 <= y < geometry["height"]:
                                raise ValueError("coordinates must be inside the observed client")
                            end_x, end_y = step.get("to_x"), step.get("to_y")
                            if action == "drag" and (type(end_x) is not int or type(end_y) is not int or not 0 <= end_x < geometry["width"] or not 0 <= end_y < geometry["height"]):
                                raise ValueError("drag destination must be inside the observed client")
                            delta = step.get("delta", 120)
                            if action == "scroll" and (type(delta) is not int or not -12000 <= delta <= 12000):
                                raise ValueError("scroll delta must be an integer in -12000..12000")
                            dispatched = True
                            sx, sy = geometry["origin"][0]+x, geometry["origin"][1]+y
                            if action in ("click", "double_click"):
                                send_pointer_click(sx, sy, 2 if action == "double_click" else 1)
                            else:
                                def inject(event):
                                    inputs = (Input * 1)(event)
                                    if user32.SendInput(1, inputs, ctypes.sizeof(Input)) != 1:
                                        raise ActionResultUncertain("partial pointer dispatch")
                                def move(px, py):
                                    ax, ay = virtual_desktop_coordinate(px, py)
                                    inject(mouse_input(ax, ay, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK))
                                move(sx, sy)
                                if action == "scroll":
                                    event = mouse_input(0, 0, 0x0800)
                                    event.mi.mouseData = delta & 0xffffffff
                                    inject(event)
                                else:
                                    try:
                                        inject(mouse_input(0, 0, MOUSEEVENTF_LEFTDOWN))
                                        for part in range(1, 11):
                                            if user32.GetForegroundWindow() != self.hwnd or geometry != self.window_geometry():
                                                raise ActionResultUncertain("target changed during drag")
                                            move(sx + (end_x-x)*part//10, sy + (end_y-y)*part//10)
                                            time.sleep(0.01)
                                    finally:
                                        inject(mouse_input(0, 0, MOUSEEVENTF_LEFTUP))
                        elif action in ("keys", "text"):
                            if auto is None:
                                raise RuntimeError("keyboard provider unavailable")
                            value = step.get("value")
                            if not isinstance(value, str) or len(value) > 4096:
                                raise ValueError("input value must be a string of at most 4096 characters")
                            dispatched = True
                            if action == "text":
                                value = "".join("{{}" if c == "{" else "{}}" if c == "}" else c for c in value)
                            auto.SendKeys(value, waitTime=0.05)
                        elif action == "resize":
                            width, height = step.get("width"), step.get("height")
                            if type(width) is not int or type(height) is not int or not 1 <= width <= 16384 or not 1 <= height <= 16384:
                                raise ValueError("window size must be in 1..16384 physical pixels")
                            dispatched = True
                            if not user32.SetWindowPos(self.hwnd, None, 0, 0, width, height, 0x0002 | 0x0004):
                                raise ActionResultUncertain("SetWindowPos failed")
                        else:
                            dispatched = True
                            user32.ShowWindow(self.hwnd, 3 if action == "maximize" else 9)
                        result = {"ok": True, "action_outcome": "provider_completed", "business_result": "unverified",
                                  "input_method": "physical", "coordinate_reference": observation, "target": geometry["target"]}
                        try:
                            result["after"] = self.observe_window()
                            observation = result["after"]["observation"]
                        except Exception as exc:
                            result["screenshot_error"] = str(exc)
                            observation = None
            except Exception as exc:
                result = {"ok": False, "error": str(exc), "code": "action_result_uncertain" if dispatched else "visual_precondition_failed",
                          "business_result": "unverified", "retry_safe": not dispatched}
            result.update(phase="finished", index=index, elapsed_ms=round((time.monotonic()-started)*1000))
            results.append(result)
            with journal.open("a", encoding="utf-8") as out:
                out.write(json.dumps(result) + "\n")
                out.flush()
            if not result["ok"] or result.get("screenshot_error"):
                break
        return {"ok": len(results) == len(steps) and all(r["ok"] and not r.get("screenshot_error") for r in results),
                "results": results, "journal": str(journal), "completed_steps": sum(r["ok"] for r in results),
                "business_result": "unverified", "retry_safe": False, "observation": observation}

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
            self.target_pid = int(window.get("pid") or 0) or None
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

    def integrity_status(self) -> dict[str, Any]:
        """Return executor and attached-target integrity diagnostics."""
        executor = current_process_integrity()
        target_pid = get_window_pid(self.hwnd) if self.hwnd is not None else None
        target = (
            process_integrity_level(target_pid)
            if target_pid is not None and target_pid > 0
            else None
        )
        mismatch = (
            integrity_mismatch_message(executor, target)
            if target is not None
            else None
        )
        return {
            "executor_pid": os.getpid(),
            "executor_integrity": executor.value,
            "target_pid": target_pid,
            "target_integrity": target.value if target is not None else None,
            "target_elevated": target.is_elevated if target is not None else False,
            "integrity_match": target is not None and mismatch is None,
        }

    def integrity_guard(self, action: str) -> dict[str, Any] | None:
        """Return a deterministic error before an action crosses UIPI."""
        integrity = self.integrity_status()
        try:
            executor = IntegrityLevel(integrity["executor_integrity"])
            target = IntegrityLevel(integrity["target_integrity"] or "Unknown")
        except ValueError:
            executor = IntegrityLevel.UNKNOWN
            target = IntegrityLevel.UNKNOWN
        if integrity["target_pid"] is None:
            # Launching before attachment has no target process to compare. The
            # subsequent operation still has to establish a target before it can
            # perform UIA/input/capture work.
            mismatch = None
        else:
            mismatch = integrity_mismatch_message(executor, target)
        if not mismatch:
            return None
        return {
            "ok": False,
            "action": action,
            "code": "integrity_mismatch",
            "executor_integrity": executor.value,
            "target_integrity": target.value,
            "target_pid": integrity["target_pid"],
            "error": mismatch,
        }

    def status(self) -> dict[str, Any]:
        """Return executor and target status without requiring an attached target."""
        return {
            "ok": True,
            **self.integrity_status(),
            "target": self.target_info(),
        }

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
            self.process = subprocess.Popen([path, *[str(a) for a in args]], text=True,
                                            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                            stderr=subprocess.DEVNULL)
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
            **self.integrity_status(),
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

    def snapshot(self, *, for_screenshot: bool = False) -> dict[str, Any]:
        """Return a UIA tree and flattened interactive element list."""
        root = self.root_control()
        nodes: list[dict[str, Any]] = []
        truncated = False

        def walk(control: Any, depth: int, path: str) -> dict[str, Any] | None:
            nonlocal truncated
            if len(nodes) >= MAX_SNAPSHOT_NODES or depth > MAX_SNAPSHOT_DEPTH:
                truncated = True
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
                "is_offscreen": get_control_property(control, "IsOffscreen") if for_screenshot else bool(get_control_property(control, "IsOffscreen")),
                "children": [],
            }
            if for_screenshot:
                raw_rect = get_control_property(control, "BoundingRectangle")
                coordinates = None
                if raw_rect is not None and all(hasattr(raw_rect, key) for key in ("left", "top", "right", "bottom")):
                    coordinates = tuple(getattr(raw_rect, key) for key in ("left", "top", "right", "bottom"))
                elif isinstance(raw_rect, (tuple, list)) and len(raw_rect) >= 4:
                    coordinates = raw_rect[:4]
                if coordinates is not None:
                    left, top, right, bottom = coordinates
                    node["bounding_rectangle"] = dict(left=left, top=top, right=right, bottom=bottom,
                                                       width=right-left, height=bottom-top)
                if node["is_offscreen"] is None:
                    truncated = True
            node["selector"] = selector_for_control(node)
            nodes.append(node)
            try:
                children = control.GetChildren()
            except Exception:
                truncated = True
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
                "path": node.get("path"),
                "is_offscreen": node.get("is_offscreen"),
            }
            for node in nodes
            if node.get("control_type") in INTERACTIVE_TYPES
            and not node.get("is_offscreen")
        ]
        interactive_count = len(interactive)
        if not for_screenshot:
            interactive = interactive[:80]
        return {
            "ok": True,
            "truncated": truncated,
            "url": self.target_url(),
            "title": get_window_title(self.hwnd or 0) or self.title,
            "accessibility_tree": tree,
            "interactive_elements": interactive,
            "capabilities": {
                "interactive_count": interactive_count,
                "returned_count": len(interactive),
                "traversal_complete": not truncated,
                "uia": "available" if interactive_count else "unknown" if truncated else "containers_only",
                "visual_actions": ["observe", "wait_stable", "click", "double_click", "scroll", "drag", "keys", "text", "maximize", "restore", "resize"],
                "unsupported_visual_actions": [],
                "next": "Use semantic UIA selectors" if interactive_count else "Traversal is incomplete; absence is not established" if truncated else "Use visual-plan observation; physical input requires foreground permission",
            },
            "target": self.target_info(),
        }

    def visual_command(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Shared PNG capture and visual assertion boundary for CLI and replay."""
        if error := self.integrity_guard("visual"):
            return {**error, "selector": str(payload.get("selector") or "")}
        try:
            with physical_coordinates():
                return self._visual_command(payload)
        except Exception as exc:
            return {"ok": False, "selector": str(payload.get("selector") or ""), "error": str(exc)}

    def _visual_command(self, payload: dict[str, Any]) -> dict[str, Any]:
        selector = str(payload.get("selector") or "")
        result = {"ok": False, "selector": selector}
        try:
            node = resolve_screenshot_element(self.snapshot(for_screenshot=True), selector)
            rectangle = node["bounding_rectangle"]
            if rectangle is None:
                raise RuntimeError("element has no UIA screen bounds")
            bounds = {"x": rectangle["left"], "y": rectangle["top"],
                      "width": rectangle["width"], "height": rectangle["height"]}
            result["bounds"] = bounds
            dpi = int(user32.GetDpiForWindow(self.hwnd))
            if not dpi:
                raise RuntimeError("GetDpiForWindow failed")
            result["dpi"] = dpi
            with self._capture_switch_lock:
                if self._capture_backend is None:
                    raise RuntimeError("no active capture backend")
                image, frame_bounds = self._capture_backend.capture_rgb()
            after = resolve_screenshot_element(self.snapshot(for_screenshot=True), selector)
            if after["bounding_rectangle"] != rectangle:
                raise RuntimeError("element moved during capture")
            actual = crop_element(image, frame_bounds, bounds)
            root = self.project_root or Path.cwd()
            def artifact(value: Any) -> Path:
                if not value:
                    raise ValueError("artifact path is required")
                path = Path(str(value))
                return path if path.is_absolute() else root / path
            if payload.get("cmd") == "element_screenshot":
                out = artifact(payload.get("out"))
                out.parent.mkdir(parents=True, exist_ok=True)
                metadata = PngImagePlugin.PngInfo()
                metadata.add_text("teshi_bounds", json.dumps(bounds))
                metadata.add_text("teshi_dpi", str(dpi))
                actual.save(out, format="PNG", pnginfo=metadata)
                return {**result, "ok": True, "out": str(payload["out"])}
            baseline_path = artifact(payload.get("baseline"))
            out = artifact(payload.get("diff_out"))
            if out.resolve() == baseline_path.resolve() or (out.exists() and baseline_path.exists() and os.path.samefile(out, baseline_path)):
                raise ValueError("diff output must not overwrite baseline")
            with Image.open(baseline_path) as source:
                if source.format != "PNG":
                    raise ValueError("baseline must be lossless PNG")
                recorded = json.loads(source.info.get("teshi_bounds", "null"))
                baseline = source.convert("RGB")
            tolerance = payload.get("pixel_tolerance", 8)
            changed, diff = compare_pixels(baseline, actual, tolerance)
            size_changed = baseline.size != actual.size or (recorded is not None and any(
                recorded[k] != bounds[k] for k in ("width", "height")))
            result.update(changed_pixels=changed, baseline_bounds=recorded, baseline_dimensions={"width": baseline.width, "height": baseline.height},
                          actual_dimensions={"width": actual.width, "height": actual.height}, pixel_tolerance=tolerance)
            if changed or size_changed:
                out.parent.mkdir(parents=True, exist_ok=True)
                diff.save(out, format="PNG")
                return {**result, "diff_out": str(out), "error": "element dimensions changed" if size_changed else "screenshot pixels differ"}
            # A previous failed run must not leave a misleading diff on success.
            out.unlink(missing_ok=True)
            return {**result, "ok": True}
        except Exception as exc:
            return {**result, "error": str(exc)}

    def find_control(self, selector: str) -> Any:
        """Find a UIA control from a compact selector."""
        criteria = split_selector(selector)
        root = self.root_control()
        if "path" in criteria:
            control = root
            for part in criteria["path"].split("/")[1:]:
                try:
                    idx = int(part)
                except (TypeError, ValueError) as exc:
                    raise ControlNotFoundError(
                        f"path selector did not resolve: {selector}"
                    ) from exc
                try:
                    children = control.GetChildren()
                except Exception as exc:  # noqa: BLE001
                    raise ControlTraversalError(
                        f"UIA provider unavailable while resolving path selector: {selector}: {exc}"
                    ) from exc
                try:
                    control = children[idx]
                except IndexError as exc:
                    raise ControlNotFoundError(
                        f"path selector did not resolve: {selector}"
                    ) from exc
                except Exception as exc:  # noqa: BLE001
                    raise ControlTraversalError(
                        f"UIA traversal failed while resolving path selector: {selector}: {exc}"
                    ) from exc
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
                children = control.GetChildren()
                stack.extend(children)
            except Exception as exc:  # noqa: BLE001
                raise ControlTraversalError(
                    f"UIA provider unavailable while resolving selector: {selector}: {exc}"
                ) from exc
        raise ControlNotFoundError(f"selector did not resolve: {selector}")

    def control_exists(self, selector: str) -> bool:
        """Return whether the legacy UIA selector resolves to any control."""
        try:
            self.find_control(selector)
        except ControlNotFoundError:
            return False
        except RuntimeError as exc:
            # Keep compatibility with callers/tests that provide a legacy
            # resolver returning the historical plain RuntimeError.
            message = str(exc)
            if message.startswith((
                "selector did not resolve:",
                "path selector did not resolve:",
            )):
                return False
            raise
        return True

    def _control_at_path(self, path: str, selector: str) -> Any:
        """Resolve a snapshot path without changing the legacy selector resolver."""
        parts = path.split("/")
        if not parts or parts[0] != "0":
            raise RuntimeError(f"snapshot path did not resolve: {selector}")
        control = self.root_control()
        for part in parts[1:]:
            try:
                control = control.GetChildren()[int(part)]
            except Exception as exc:
                raise RuntimeError(f"snapshot path did not resolve: {selector}") from exc
        return control

    def find_visible_interactive_control(self, selector: str) -> tuple[Any, dict[str, int]]:
        """Resolve one visible interactive UIA control and its physical bounds."""
        node = resolve_screenshot_element(
            self.snapshot(for_screenshot=True),
            selector,
        )
        rectangle = node.get("bounding_rectangle")
        if not rectangle or rectangle["width"] <= 0 or rectangle["height"] <= 0:
            raise RuntimeError("element has no visible UIA screen bounds")
        path = node.get("path")
        if not path:
            raise RuntimeError("visible UIA element has no snapshot path")
        control = self._control_at_path(str(path), selector)
        if get_control_property(control, "IsOffscreen") is True:
            raise RuntimeError("element became offscreen before pointer_click")
        current_rectangle = rect_to_dict(
            get_control_property(control, "BoundingRectangle")
        )
        if (
            not current_rectangle
            or current_rectangle["width"] <= 0
            or current_rectangle["height"] <= 0
        ):
            raise RuntimeError("element has no visible UIA screen bounds")
        return control, current_rectangle

    def highlight(self, selector: str) -> dict[str, Any]:
        """Highlight the selected native element."""
        if error := self.integrity_guard("highlight"):
            return {**error, "selector": selector}
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

    def shell_tray_list(self) -> dict[str, Any]:
        """List notification-area icons without touching the attach session."""
        return shell_tray_icons()

    @physical_coordinates()
    def shell_tray_click(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Click one notification-area icon identified by its tooltip name.

        The icon must resolve to exactly one UIA NotifyItemIcon button. When
        several icons share the name, ``pid`` disambiguates through
        Shell_NotifyIconGetRect; without it the command fails with the
        candidate list instead of clicking an arbitrary icon. Physical
        pointer input requires foreground mode and explicit request permission.
        """
        name = str(payload.get("name") or "").strip()
        if not name:
            return {"ok": False, "error": "tray click requires a non-empty name"}
        button = str(payload.get("button") or "left").casefold()
        if button not in ("left", "right"):
            return {"ok": False, "error": f"unsupported button: {button!r}; use left or right"}
        try:
            count = int(payload.get("count") or 1)
        except (TypeError, ValueError):
            return {"ok": False, "error": "count must be an integer"}
        if count < 1:
            return {"ok": False, "error": "count must be >= 1"}
        mode = str(payload.get("mode") or "auto").casefold()
        if mode not in ("auto", "foreground", "background"):
            return {
                "ok": False,
                "error": f"invalid mode: {mode!r}; use auto, foreground, or background",
            }
        if not interactive_input_allowed(payload):
            return interactive_input_error("shell_tray_click")
        timeout_s = float(payload.get("timeout_ms") or 10000) / 1000.0

        reveal_autohidden_taskbar()
        listing = shell_tray_icons()
        if not listing.get("ok"):
            return listing
        icons = listing["icons"]
        matches = [icon for icon in icons if icon["name"].casefold() == name.casefold()]

        if not matches:
            # The icon may live in the hidden-icons overflow flyout. Opening it
            # needs one physical click on the chevron, which the mode policy
            # above already authorized.
            overflow = _open_tray_overflow(min(timeout_s, 5.0))
            if overflow is not None:
                listing = shell_tray_icons()
                icons = listing.get("icons", [])
                matches = [icon for icon in icons if icon["name"].casefold() == name.casefold()]

        if not matches:
            return {
                "ok": False,
                "error": f"tray icon not found: {name!r}",
                "available_icons": sorted({icon["name"] for icon in icons}),
            }

        def candidate(icon: dict[str, Any]) -> dict[str, Any]:
            rect = icon.get("rect")
            return {
                "name": icon["name"],
                "area": icon.get("area"),
                "rect": (
                    {"left": rect[0], "top": rect[1], "right": rect[2], "bottom": rect[3]}
                    if rect
                    else None
                ),
            }

        target: dict[str, Any] | None = None
        pid = payload.get("pid")
        if pid is not None:
            try:
                pid_i = int(pid)
            except (TypeError, ValueError):
                return {"ok": False, "error": "pid must be an integer"}
            want = notify_icon_rect_for_pid(pid_i)
            if want is None:
                return {
                    "ok": False,
                    "error": f"no notification icon is owned by pid {pid_i}",
                }
            # Shell and UIA round scaled overflow bounds independently. A tiny
            # edge tolerance preserves unique ownership without rejecting the
            # same icon for a one-pixel provider difference.
            exact = [icon for icon in matches if icon.get("rect") is not None
                     and all(abs(a - b) <= 2 for a, b in zip(icon["rect"], want))]
            if len(exact) != 1:
                return {
                    "ok": False,
                    "error": (
                        f"tray icon {name!r} matches {len(matches)} button(s) but "
                        f"{len(exact)} belong to pid {pid_i}"
                    ),
                    "candidates": [candidate(icon) for icon in matches],
                }
            target = exact[0]
        elif len(matches) > 1:
            return {
                "ok": False,
                "error": (
                    f"multiple tray icons named {name!r}; pass --pid to disambiguate"
                ),
                "candidates": [candidate(icon) for icon in matches],
            }
        else:
            target = matches[0]

        rect = target.get("rect")
        if rect is None:
            return {"ok": False, "error": "tray icon has no UIA bounds"}
        center_x = (rect[0] + rect[2]) // 2
        center_y = (rect[1] + rect[3]) // 2
        try:
            with physical_coordinates():
                send_pointer_click(center_x, center_y, count=count, button=button)
        except Exception as exc:
            return {"ok": False, "error": f"tray click failed: {exc}"}
        return {
            "ok": True,
            "action_outcome": "input_injected",
            "business_result": "unverified",
            "name": target["name"],
            "area": target.get("area"),
            "button": button,
            "count": count,
            "rect": {"left": rect[0], "top": rect[1], "right": rect[2], "bottom": rect[3]},
        }

    def wait_window(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Wait for a process window to reach a visibility state.

        Polls by pid so a destroyed and recreated window (new HWND) is still
        found, and distinguishes hidden, minimized, and absent windows. When
        the session is attached to the same pid and the live window carries a
        different HWND, the session is refreshed to it so later UIA and
        screenshot commands target the restored window.
        """
        try:
            pid = int(payload.get("pid"))
        except (TypeError, ValueError):
            return {"ok": False, "error": "pid must be an integer"}
        want_visible = str(payload.get("state") or "visible").casefold() != "hidden"
        timeout_s = float(payload.get("timeout_ms") or 10000) / 1000.0
        deadline = time.monotonic() + timeout_s
        while True:
            expected_created = payload.get("_expected_process_created")
            if expected_created is not None and native_process_created(pid) != expected_created:
                return {"ok": False, "code": "target_invalid", "error": "Target process exited or changed identity"}
            state = process_window_state(pid)
            if state["visible"] == want_visible:
                if (
                    want_visible
                    and self.target_pid == pid
                    and state["hwnd"] is not None
                    and state["hwnd"] != self.hwnd
                ):
                    window = find_window(hwnd=state["hwnd"])
                    if window:
                        self._attach_target(window)
                return {
                    "ok": True,
                    "pid": pid,
                    "state": "visible" if want_visible else "hidden",
                    "hwnd": state["hwnd"],
                    "title": state["title"],
                }
            if time.monotonic() >= deadline:
                return {
                    "ok": False,
                    "error": (
                        f"pid {pid} did not reach state "
                        f"{'visible' if want_visible else 'hidden'} "
                        f"within {timeout_s:.1f}s"
                    ),
                    "pid": pid,
                    "last_state": state,
                }
            time.sleep(WAIT_WINDOW_POLL_INTERVAL_SEC)

    def execute(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Run one action against a selected UIA control, or exec a system command.

        mode: "auto" (default; choose the least intrusive implementation)
              "background" (strictly forbid focus and physical input)
              "foreground" (allow focused keyboard fallbacks when required)

        ``pointer_click`` requires foreground mode and request-scoped permission.
        Ordinary
        ``click`` never injects physical pointer input.

        exec actions (no selector needed):
          exec: launch <path>           spawn a process
          exec: close                   send WM_CLOSE to attached window
          exec: assert_process <name>   fail if no process matches
          exec: assert_no_process <name>  fail if any process matches

        selector assertions:
          assert_not_exists              pass only when the UIA selector matches no control
        """
        selector = str(payload.get("selector") or "")
        action = str(payload.get("action") or "click")
        value = payload.get("value")
        mode = str(payload.get("mode") or "auto").casefold()
        try:
            if mode not in ("auto", "foreground", "background"):
                return {
                    "ok": False,
                    "error": (
                        f"invalid mode: {mode!r}; use auto, foreground, or background"
                    ),
                }
            if (action == "pointer_click" or
                    (mode == "foreground" and action in ("click", "fill", "select", "press_key"))):
                if not interactive_input_allowed(payload):
                    return interactive_input_error(action)
            # --- exec actions (no UIA control needed) ---
            if action == "assert_screenshot":
                return self.visual_command({**payload, "baseline": value})
            if action == "exec":
                if error := self.integrity_guard(action):
                    return {**error, "selector": selector}
                return self._handle_exec(selector, str(value or ""))
            if action in UIA_ACTIONS:
                if error := self.integrity_guard(action):
                    return {**error, "selector": selector}
            if action in ("click", "fill", "select"):
                ensure_nonintrusive_uia()
            if action == "assert_not_exists":
                try:
                    exists = self.control_exists(selector)
                except Exception as exc:
                    return {
                        "ok": False,
                        "selector": selector,
                        "action": action,
                        "error": str(exc),
                    }
                if exists:
                    return {
                        "ok": False,
                        "selector": selector,
                        "action": action,
                        "exists": True,
                        "error": f"element exists: {selector}",
                    }
                return {
                    "ok": True,
                    "selector": selector,
                    "action": action,
                    "exists": False,
                }

            if action == "pointer_click":
                try:
                    self._activate_foreground("pointer_click")
                    with physical_coordinates():
                        _, rectangle = self.find_visible_interactive_control(selector)
                        center_x = rectangle["left"] + rectangle["width"] // 2
                        center_y = rectangle["top"] + rectangle["height"] // 2
                        send_pointer_click(center_x, center_y)
                except Exception as exc:
                    return {"ok": False, "error": str(exc)}
                return {"ok": True, "selector": selector, "action": action,
                        "action_outcome": "input_injected", "business_result": "unverified"}

            control = self.find_control(selector)
            uia_method = None
            if action == "click":
                uia_method = self._click(control, mode)
            elif action == "fill":
                if value is None:
                    return {"ok": False, "error": "fill requires value"}
                self._fill(control, str(value), mode)
                if self._control_text(control) != str(value):
                    raise ActionResultUncertain("fill was dispatched but value readback did not match")
            elif action == "assert_visible":
                error = element_visibility_error(control, self.hwnd)
                if error is not None:
                    return {
                        "ok": False,
                        "selector": selector,
                        "action": action,
                        "error": error,
                    }
            elif action == "assert_text":
                expected = str(value or "")
                actual = self._control_text(control)
                if expected not in actual:
                    return {
                        "ok": False,
                        "error": f"expected text {expected!r}, got {actual!r}",
                    }
            elif action == "select":
                uia_method = self._select(control, mode)
            elif action == "press_key":
                if value is None:
                    return {"ok": False, "error": "press_key requires value"}
                self._press_key(control, str(value), mode)
            else:
                return {"ok": False, "error": f"unsupported_action: {action}"}
        except RequiresInteractiveInput as exc:
            return {**interactive_input_error(action), "detail": str(exc)}
        except ActionResultUncertain as exc:
            return {"ok": False, "code": "action_result_uncertain", "error": str(exc),
                    "action": action, "action_outcome": "dispatched", "business_result": "unverified"}
        except Exception as exc:
            return {"ok": False, "error": str(exc)}
        result = {"ok": True, "selector": selector, "action": action}
        if action in ("click", "select", "press_key"):
            result.update(action_outcome="provider_completed", business_result="unverified")
        if uia_method is not None:
            result["input_method"] = uia_method
        return result

    def _handle_exec(self, sub_action: str, value_arg: str) -> dict[str, Any]:
        """Handle exec commands: launch, close, assert_process, assert_no_process."""
        if sub_action == "launch":
            if not value_arg:
                return {"ok": False, "error": "exec launch requires value_arg (exe path)"}
            exe_name = os.path.basename(value_arg)
            if _process_is_running(exe_name):
                return {"ok": True, "exec": "launch", "skipped": True, "reason": f"{exe_name} is already running"}
            try:
                proc = subprocess.Popen([value_arg], text=True, stdin=subprocess.DEVNULL,
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                return {"ok": True, "exec": "launch", "pid": proc.pid}
            except OSError as exc:
                return {"ok": False, "error": f"launch failed: {exc}"}

        if sub_action == "close":
            if self.hwnd is None or user32 is None:
                return {"ok": False, "error": "no attached window to close"}
            if not user32.PostMessageW(self.hwnd, 0x0010, 0, 0):  # WM_CLOSE
                get_last_error = getattr(ctypes, "get_last_error", lambda: 0)
                error_code = int(get_last_error())
                detail = f" (Win32 error {error_code})" if error_code else ""
                return {"ok": False, "error": f"close failed{detail}"}
            closing_hwnd = self.hwnd
            deadline = time.monotonic() + 1.0
            while time.monotonic() < deadline:
                if not is_window(closing_hwnd):
                    return {"ok": True, "exec": "close", "hwnd": closing_hwnd,
                            "action_outcome": "window_closed", "business_result": "verified"}
                time.sleep(0.05)
            return {"ok": False, "code": "action_result_uncertain", "exec": "close",
                    "hwnd": self.hwnd, "action_outcome": "dispatched",
                    "business_result": "unverified", "error": "WM_CLOSE queued; window closure is unverified"}

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

    def _activate_foreground(self, action: str) -> None:
        """Activate the attached window only for an explicit intrusive path."""
        if self.hwnd is None or user32 is None:
            raise RuntimeError(f"{action} requires an attached Windows window")
        if user32.GetForegroundWindow() == self.hwnd:
            return
        user32.SetForegroundWindow(self.hwnd)
        # Windows can reject activation or report failure for an already-active
        # target. Never inject keys/pointer input based only on its return value.
        if user32.GetForegroundWindow() != self.hwnd:
            raise RuntimeError(f"SetForegroundWindow failed for {action}; target is not foreground")

    @staticmethod
    def _uia_click(control: Any) -> str:
        """Activate a control through UIA without focus or pointer injection."""
        patterns = (
            ("GetInvokePattern", "Invoke"),
            ("GetSelectionItemPattern", "Select"),
            ("GetTogglePattern", "Toggle"),
            ("GetLegacyIAccessiblePattern", "DoDefaultAction"),
        )
        for getter, method in patterns:
            try:
                pattern = getattr(control, getter)()
            except Exception:
                continue
            if pattern is None:
                continue
            try:
                previous = pattern.ToggleState if method == "Toggle" else None
                completed = getattr(pattern, method)()
                if method == "Select" and not bool(pattern.IsSelected):
                    raise ActionResultUncertain("UIA Select completed but selection was not confirmed")
                if method == "Toggle" and pattern.ToggleState == previous:
                    raise ActionResultUncertain("UIA Toggle completed but state did not change")
            except ActionResultUncertain:
                raise
            except Exception as exc:
                raise ActionResultUncertain(f"UIA {method} may have executed: {exc}") from exc
            if completed is not True:
                raise ActionResultUncertain(f"UIA {method} did not confirm completion")
            return f"{getter.removeprefix('Get')}.{method}"
        raise RuntimeError("no supported non-intrusive UIA activation pattern")

    def _click(self, control: Any, mode: str) -> str:
        try:
            return self._uia_click(control)
        except RuntimeError as uia_error:
            try:
                self._background_click(control)
                return
            except RuntimeError as background_error:
                if mode != "foreground":
                    restriction = (
                        "background mode forbids foreground fallback"
                        if mode == "background"
                        else "auto mode does not implicitly grant foreground input"
                    )
                    raise RequiresInteractiveInput(
                        f"click could not be completed non-intrusively: {uia_error}; "
                        f"{background_error}; {restriction}"
                    ) from background_error

        self._focused_keys(control, "click", "{Enter}")
        return "focused_keyboard"

    def _fill(self, control: Any, value: str, mode: str) -> None:
        try:
            pattern = control.GetValuePattern()
        except Exception:
            pattern = None
        if pattern is not None:
            try:
                completed = pattern.SetValue(value)
            except Exception as exc:
                raise ActionResultUncertain(f"ValuePattern.SetValue may have executed: {exc}") from exc
            if completed is not True:
                raise ActionResultUncertain("ValuePattern.SetValue did not confirm completion")
            return
        try:
            self._background_fill(control, value)
            return
        except RuntimeError as background_error:
            if mode != "foreground":
                raise RequiresInteractiveInput(
                    f"fill could not complete non-intrusively: {background_error}"
                ) from background_error

        self._focused_keys(control, "fill", "{Ctrl}a", value)

    def _select(self, control: Any, mode: str) -> str:
        try:
            pattern = control.GetSelectionItemPattern()
        except Exception:
            pattern = None
        if pattern is None:
            return self._click(control, mode)
        try:
            completed = pattern.Select()
            if not bool(pattern.IsSelected):
                raise ActionResultUncertain("SelectionItemPattern.Select did not select the item")
        except ActionResultUncertain:
            raise
        except Exception as exc:
            raise ActionResultUncertain(f"SelectionItemPattern.Select may have executed: {exc}") from exc
        if completed is not True:
            raise ActionResultUncertain("SelectionItemPattern.Select did not confirm completion")
        return "SelectionItemPattern.Select"

    def _press_key(self, control: Any, value: str, mode: str) -> None:
        try:
            self._background_press_key(control, value)
            return
        except RuntimeError as background_error:
            if mode != "foreground":
                restriction = (
                    "background mode forbids focus and keyboard injection"
                    if mode == "background"
                    else "auto mode does not implicitly grant foreground input"
                )
                raise RequiresInteractiveInput(
                    f"press_key could not use background messages: {background_error}; "
                    f"{restriction}"
                ) from background_error

        self._focused_keys(control, "press_key", value)

    def _focused_keys(self, control: Any, action: str, *keys: str) -> None:
        """Send authorized keyboard input only after the provider confirms focus."""
        self._activate_foreground(f"{action} keyboard fallback")
        if auto is None:
            raise RuntimeError("uiautomation is not installed")
        try:
            if control.SetFocus() is not True:
                raise ActionResultUncertain("SetFocus did not confirm focus; keyboard input was not sent")
            for key in keys:
                auto.SendKeys(key, waitTime=0.05)
        except ActionResultUncertain:
            raise
        except Exception as exc:
            raise ActionResultUncertain(f"focused input completion is unknown: {exc}") from exc

    def _control_text(self, control: Any) -> str:
        try:
            value = control.GetValuePattern().Value
            if value is not None:
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

    def _get_native_control_hwnd(self, control: Any) -> int | None:
        """Return only a control-owned HWND suitable for direct text input."""
        try:
            hwnd = get_control_property(control, "NativeWindowHandle")
            if not hwnd:
                return None
            hwnd = int(hwnd)
            if hwnd == 0 or (self.hwnd is not None and hwnd == self.hwnd):
                return None
            return hwnd
        except Exception:
            return None

    def _background_click(self, control: Any) -> None:
        """Post WM_LBUTTONDOWN/UP without foreground or physical input."""
        # Get screen coordinates from UIA, convert to client coords.
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

        # Post the click. PostMessage targets the window queue and never moves
        # the real system pointer.
        if user32 is None:
            raise RuntimeError("background click requires win32 messaging")
        down = user32.PostMessageW(hwnd, WM_LBUTTONDOWN, 0x0001, lparam)  # MK_LBUTTON
        up = user32.PostMessageW(hwnd, WM_LBUTTONUP, 0, lparam)
        if not down or not up:
            raise ActionResultUncertain("background click messages may have been partially queued")
        raise ActionResultUncertain("background click queued; business result is unverified")

    def _background_fill(self, control: Any, value: str) -> None:
        """Set text through messages, without foreground activation."""
        if user32 is None:
            raise RuntimeError("background fill requires win32 API (not available)")
        # A missing control HWND must never fall back to the attached window:
        # WM_SETTEXT there changes the top-level title and falsely reports success.
        target = self._get_native_control_hwnd(control)
        if target is None:
            raise RuntimeError("no control HWND for background fill")

        result = user32.SendMessageTimeoutW(
            target, WM_SETTEXT, 0, ctypes.c_wchar_p(value),
            SMTO_ABORTIFHUNG, 2000, None,
        )
        if result != 0:
            return

        # A timeout may occur after WM_SETTEXT was processed. Never type again.
        raise ActionResultUncertain("WM_SETTEXT completion is unknown; verify the value before retrying")

    def _background_select(self, control: Any) -> None:
        """Select a list item via UIA SelectionItemPattern or background click."""
        try:
            if control.GetSelectionItemPattern().Select() is not False:
                return
            raise RuntimeError("SelectionItemPattern.Select reported failure")
        except Exception:
            try:
                self._uia_click(control)
            except RuntimeError:
                self._background_click(control)

    def _background_press_key(self, control: Any, key_str: str) -> None:
        """Post keystrokes to the target without foreground activation."""
        hwnd = self._get_control_hwnd(control)
        if hwnd is None:
            raise RuntimeError("no target HWND for background press_key")
        if user32 is None:
            raise RuntimeError("background press_key requires win32 messaging")
        self._post_key_string(hwnd, key_str)
        raise ActionResultUncertain("background keys queued; business result is unverified")

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
        pending_modifiers: list[int] = []
        modifier_names = {
            "ctrl": VK_CONTROL,
            "control": VK_CONTROL,
            "shift": VK_SHIFT,
            "alt": VK_MENU,
        }

        def emit_key(token: dict[str, Any]) -> None:
            """Keep brace modifiers held across the key they modify."""
            for vk in pending_modifiers:
                tokens.append({"type": "modifier", "vk": vk, "down": True})
            tokens.append(token)
            for vk in reversed(pending_modifiers):
                tokens.append({"type": "modifier", "vk": vk, "down": False})
            pending_modifiers.clear()

        def flush_modifiers() -> None:
            """Release a trailing bare modifier as an ordinary key press."""
            for vk in pending_modifiers:
                tokens.append({"type": "modifier", "vk": vk, "down": True})
            for vk in reversed(pending_modifiers):
                tokens.append({"type": "modifier", "vk": vk, "down": False})
            pending_modifiers.clear()

        i = 0
        n = len(key_str)
        while i < n:
            ch = key_str[i]
            if ch == "{":
                if i + 1 < n and key_str[i + 1] == "{":
                    emit_key({"type": "char", "char": "{"})
                    i += 2
                    continue
                # Find matching closing brace.
                end = key_str.find("}", i + 1)
                if end == -1:
                    emit_key({"type": "char", "char": ch})
                    i += 1
                    continue
                if end == i + 1:
                    # Empty braces: literal {
                    emit_key({"type": "char", "char": "{"})
                    i = end + 1
                    continue
                inner = key_str[i + 1:end]
                i = end + 1

                low = inner.casefold()
                modifier_vk = modifier_names.get(low)
                if modifier_vk is not None:
                    pending_modifiers.append(modifier_vk)
                    continue

                # Single token: maybe special key, maybe literal text.
                vk = KEY_NAME_TO_VK.get(low)
                if vk is not None:
                    emit_key({"type": "special", "vk": vk, "name": inner})
                elif len(inner) == 1:
                    emit_key({"type": "char", "char": inner})
                else:
                    # Unknown — treat as literal text.
                    for literal in inner:
                        emit_key({"type": "char", "char": literal})
            elif ch == "}":
                if i + 1 < n and key_str[i + 1] == "}":
                    emit_key({"type": "char", "char": "}"})
                    i += 2
                    continue
                # Unescaped closing brace: literal }
                emit_key({"type": "char", "char": "}"})
                i += 1
            else:
                emit_key({"type": "char", "char": ch})
                i += 1
        flush_modifiers()
        return tokens

    def _post_message(self, hwnd: int, message: int, wparam: int, lparam: int) -> None:
        """Post one input message and fail closed when Windows rejects it."""
        if user32 is None:
            raise RuntimeError("background input requires win32 messaging")
        if not user32.PostMessageW(hwnd, message, wparam, lparam):
            raise ActionResultUncertain(f"PostMessageW failed for message 0x{message:04X}; partial dispatch cannot be excluded")

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
                    self._post_message(
                        hwnd, WM_KEYDOWN, VK_SHIFT,
                        self._make_lparam(scan_code=shift_scan),
                    )
                if ctrl_down:
                    ctrl_scan = user32.MapVirtualKeyW(VK_CONTROL, MAPVK_VK_TO_VSC)
                    self._post_message(
                        hwnd, WM_KEYDOWN, VK_CONTROL,
                        self._make_lparam(scan_code=ctrl_scan),
                    )
                if alt_down:
                    alt_scan = user32.MapVirtualKeyW(VK_MENU, MAPVK_VK_TO_VSC)
                    self._post_message(
                        hwnd, WM_SYSKEYDOWN, VK_MENU,
                        self._make_lparam(scan_code=alt_scan),
                    )
                # Key-down
                self._post_message(
                    hwnd, WM_KEYDOWN, vk, self._make_lparam(scan_code=scan)
                )
                # WM_CHAR for printable characters
                self._post_message(
                    hwnd, WM_CHAR, ord(ch), self._make_lparam(scan_code=scan)
                )
                # Key-up
                self._post_message(
                    hwnd,
                    WM_KEYUP,
                    vk,
                    self._make_lparam(scan_code=scan, previous=True, transition=True),
                )
                # Release modifiers
                if shift_down:
                    self._post_message(
                        hwnd,
                        WM_KEYUP,
                        VK_SHIFT,
                        self._make_lparam(
                            scan_code=shift_scan, previous=True, transition=True
                        ),
                    )
                if ctrl_down:
                    self._post_message(
                        hwnd,
                        WM_KEYUP,
                        VK_CONTROL,
                        self._make_lparam(
                            scan_code=ctrl_scan, previous=True, transition=True
                        ),
                    )
                if alt_down:
                    self._post_message(
                        hwnd,
                        WM_SYSKEYUP,
                        VK_MENU,
                        self._make_lparam(
                            scan_code=alt_scan, previous=True, transition=True
                        ),
                    )
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
                self._post_message(
                    hwnd,
                    WM_KEYDOWN,
                    vk,
                    self._make_lparam(scan_code=scan, extended=ext_flag),
                )
                self._post_message(
                    hwnd,
                    WM_KEYUP,
                    vk,
                    self._make_lparam(
                        scan_code=scan,
                        extended=ext_flag,
                        previous=True,
                        transition=True,
                    ),
                )
            elif t == "modifier":
                vk = token["vk"]
                down = token["down"]
                scan = user32.MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)
                is_extended = vk in (VK_RCONTROL, VK_RMENU, VK_RSHIFT)
                msg = WM_KEYDOWN if down else WM_KEYUP
                lparam = self._make_lparam(scan_code=scan, extended=is_extended)
                if not down:
                    lparam |= (1 << 30) | (1 << 31)  # previous=1, transition=1
                self._post_message(hwnd, msg, vk, lparam)
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
                self._post_message(
                    hwnd, WM_KEYDOWN, VK_SHIFT,
                    self._make_lparam(scan_code=shift_scan),
                )
            if ctrl_down:
                ctrl_scan = user32.MapVirtualKeyW(VK_CONTROL, MAPVK_VK_TO_VSC)
                self._post_message(
                    hwnd, WM_KEYDOWN, VK_CONTROL,
                    self._make_lparam(scan_code=ctrl_scan),
                )
            if alt_down:
                alt_scan = user32.MapVirtualKeyW(VK_MENU, MAPVK_VK_TO_VSC)
                self._post_message(
                    hwnd, WM_SYSKEYDOWN, VK_MENU,
                    self._make_lparam(scan_code=alt_scan),
                )
            self._post_message(
                hwnd, WM_KEYDOWN, vk, self._make_lparam(scan_code=scan)
            )
            self._post_message(
                hwnd, WM_CHAR, ord(ch), self._make_lparam(scan_code=scan)
            )
            self._post_message(
                hwnd,
                WM_KEYUP,
                vk,
                self._make_lparam(scan_code=scan, previous=True, transition=True),
            )
            if shift_down:
                self._post_message(
                    hwnd,
                    WM_KEYUP,
                    VK_SHIFT,
                    self._make_lparam(
                        scan_code=shift_scan, previous=True, transition=True
                    ),
                )
            if ctrl_down:
                self._post_message(
                    hwnd,
                    WM_KEYUP,
                    VK_CONTROL,
                    self._make_lparam(
                        scan_code=ctrl_scan, previous=True, transition=True
                    ),
                )
            if alt_down:
                self._post_message(
                    hwnd,
                    WM_SYSKEYUP,
                    VK_MENU,
                    self._make_lparam(
                        scan_code=alt_scan, previous=True, transition=True
                    ),
                )

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
                if error := self.integrity_guard("preview"):
                    raise RuntimeError(error["error"])
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
        if cmd in ("element_screenshot", "assert_screenshot"):
            return session.visual_command(payload)
        if cmd == "list_windows":
            return {"ok": True, "windows": enum_windows()}
        if cmd == "get_status":
            return session.status()
        if cmd == "instance":
            return session.instance_command(payload)
        if cmd == "external_check":
            return session.external_check(payload)
        if cmd == "visual_plan":
            return session.visual_plan(payload)
        if cmd == "attach_window":
            return session.attach(payload)
        if cmd == "launch_app":
            if error := session.integrity_guard("launch_app"):
                return error
            return await session.launch(payload)
        if cmd == "get_ui_snapshot":
            if error := session.integrity_guard("snapshot"):
                return error
            return session.snapshot()
        if cmd == "highlight_selector":
            return session.highlight(str(payload.get("selector") or ""))
        if cmd == "clear_highlight":
            return session.clear_highlight()
        if cmd == "execute_locator":
            return session.execute(payload)
        if cmd == "shell_tray_list":
            return session.shell_tray_list()
        if cmd == "shell_tray_click":
            return session.shell_tray_click(payload)
        if cmd == "wait_window":
            return session.wait_window(payload)
        if cmd == "screenshot":
            if error := session.integrity_guard("screenshot"):
                return error
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


def native_target_identity(hwnd: int | None) -> dict[str, Any] | None:
    """Identify a live window and its process incarnation without UIA."""
    if not hwnd or not is_window(hwnd):
        return None
    pid = get_window_pid(hwnd)
    if not pid:
        return None
    created = native_process_created(pid)
    return {"hwnd": hwnd, "pid": pid, "created": created} if created is not None else None


def native_process_created(pid: int) -> int | None:
    """Read process incarnation independently of window lifetime."""
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel.OpenProcess(0x1000, False, pid)
    if not handle:
        return None
    try:
        times = [wintypes.FILETIME() for _ in range(4)]
        if not kernel.GetProcessTimes(handle, *[ctypes.byref(value) for value in times]):
            return None
        return (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime
    finally:
        kernel.CloseHandle(handle)


def watch_supervisor(pid: int) -> None:
    """Exit only this executor if its owning supervisor dies, even during UIA."""
    if os.name == "nt":
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        handle = kernel.OpenProcess(0x00100000, False, pid)
        if handle:
            kernel.WaitForSingleObject(handle, 0xFFFFFFFF)
            kernel.CloseHandle(handle)
    else:
        while os.getppid() == pid:
            time.sleep(0.1)
    os._exit(2)


def run_executor(project_root: Path | None, supervisor_pid: int) -> None:
    """Own COM/UIA and capture in an expendable, private protocol process."""
    threading.Thread(target=watch_supervisor, args=(supervisor_pid,), daemon=True).start()
    session = WinAppSession(project_root)
    loop = asyncio.new_event_loop()
    try:
        for line in sys.stdin:
            payload = json.loads(line)
            identity = payload.pop("_restore_target", None)
            if identity is not None:
                if (payload.get("_lifecycle")
                        and native_target_identity(identity["hwnd"]) != identity
                        and native_process_created(identity["pid"]) == identity["created"]):
                    session.hwnd = identity["hwnd"]
                    session.target_pid = identity["pid"]
                    result = {"ok": True}
                elif native_target_identity(identity["hwnd"]) != identity:
                    result = {"ok": False, "code": "target_invalid", "error": "Saved target identity changed"}
                else:
                    result = session.attach({"hwnd": identity["hwnd"], "pid": identity["pid"]})
            else:
                result = loop.run_until_complete(handle_command(session, payload))
            current_identity = native_target_identity(session.hwnd)
            if current_identity is None and session.target_pid and identity is not None:
                current_identity = identity
            result["_target_identity"] = current_identity
            print(json.dumps(result), flush=True)
    finally:
        session.close()
        loop.close()


class WinAppSupervisor:
    """Keep protocol health responsive while a single owned executor is blocked."""

    def __init__(self, project_root: Path | None, executor_command: list[str] | None = None):
        self.project_root = project_root
        self.worker = None
        self.active_request = None
        self.target_identity = None
        self.generation = 0
        self.preview_active = False
        self.business_pending = 0
        self.lock = asyncio.Lock()
        self.executor_command = executor_command
        self.identity_file = project_root / ".teshi/winapp-target.json" if project_root else None
        if self.identity_file:
            with suppress(OSError, ValueError):
                identity = json.loads(self.identity_file.read_text(encoding="utf-8"))
                if isinstance(identity, dict) and set(identity) == {"hwnd", "pid", "created"}:
                    self.target_identity = identity

    async def stop_worker(self) -> None:
        worker, self.worker = self.worker, None
        self.generation += 1
        if worker is not None and worker.returncode is None:
            with suppress(ProcessLookupError):
                worker.kill()
            await asyncio.wait_for(worker.wait(), 1.0)

    async def exchange(self, payload: dict[str, Any]) -> dict[str, Any]:
        worker = self.worker
        worker.stdin.write((json.dumps(payload) + "\n").encode("utf-8"))
        await worker.stdin.drain()
        line = await worker.stdout.readline()
        if not line:
            raise RuntimeError("executor exited before replying")
        result = json.loads(line)
        if not isinstance(result, dict) or type(result.get("ok")) is not bool:
            raise RuntimeError("executor response must be an object with a boolean ok field")
        identity = result.pop("_target_identity", None)
        if identity is not None and (not isinstance(identity, dict) or set(identity) != {"hwnd", "pid", "created"}):
            raise RuntimeError("executor returned an invalid target identity")
        if identity is not None or (payload.get("cmd") not in ("wait_window", "instance", "external_check") and "_restore_target" not in payload):
            self.target_identity = identity
        self.save_identity()
        return result

    def save_identity(self) -> None:
        """Persist only a verified identity, never a title-based recovery hint."""
        if self.identity_file:
            self.identity_file.parent.mkdir(parents=True, exist_ok=True)
            temporary = self.identity_file.with_suffix(".tmp")
            temporary.write_text(json.dumps(self.target_identity), encoding="utf-8")
            temporary.replace(self.identity_file)

    async def execute(self, payload: dict[str, Any], *, background: bool = False) -> dict[str, Any]:
        """Skip competing previews and let business requests wait for a preview."""
        if background:
            if self.lock.locked() or self.business_pending:
                return {"ok": False, "code": "preview_skipped"}
            self.preview_active = True
        else:
            self.business_pending += 1
        try:
            return await self._execute(payload)
        finally:
            if background:
                self.preview_active = False
            else:
                self.business_pending -= 1

    async def _execute(self, payload: dict[str, Any]) -> dict[str, Any]:
        cmd = payload.get("cmd")
        if cmd == "health":
            valid = self.target_identity is not None and native_target_identity(
                self.target_identity["hwnd"]) == self.target_identity
            return {"ok": True, "protocol": "teshi-winapp", "version": 1,
                    "generation": self.generation, "busy": self.lock.locked(),
                    "active_request": self.active_request,
                    "target": self.target_identity, "target_valid": valid,
                    "project": str(self.project_root) if self.project_root else None}
        if cmd == "cancel":
            if not payload.get("cancel_request_id") or payload["cancel_request_id"] != self.active_request:
                return {"ok": False, "code": "request_not_active", "error": "Cancellation identity does not match"}
            await self.stop_worker()
            return {"ok": True, "cancelled_request": payload["cancel_request_id"],
                    "business_result": "unverified", "retry_safe": False}
        if self.lock.locked() and not self.preview_active:
            return {"ok": False, "code": "executor_busy", "stage": "queue",
                    "error": "Executor is busy; query health or cancel its request", "retry_safe": True}
        timeout_ms = payload.get("timeout_ms", 10000)
        if not isinstance(timeout_ms, (int, float)) or not 0 < timeout_ms <= 120000:
            return {"ok": False, "code": "invalid_timeout", "error": "timeout_ms must be in 1..120000"}
        deadline = asyncio.get_running_loop().time() + timeout_ms / 1000
        try:
            await asyncio.wait_for(self.lock.acquire(), timeout_ms / 1000)
        except TimeoutError:
            return {"ok": False, "code": "command_timeout", "stage": "queue",
                    "timeout_ms": timeout_ms, "error": "Executor queue deadline exceeded",
                    "action_outcome": "not_dispatched", "business_result": "unverified", "retry_safe": True}
        try:
            self.active_request = f"{payload.get('request_id') or 'request'}:{secrets.token_hex(12)}"
            stage = "executor_start"
            dispatched = False
            generation = self.generation
            try:
                async with asyncio.timeout_at(deadline):
                    if self.worker is None or self.worker.returncode is not None:
                        args = self.executor_command or [sys.executable, str(Path(__file__).resolve()),
                                                         "--executor", "--supervisor-pid", str(os.getpid())]
                        if self.project_root and not self.executor_command:
                            args.extend(["--project-root", str(self.project_root)])
                        options = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
                        self.worker = await asyncio.create_subprocess_exec(
                            *args, stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
                            stderr=asyncio.subprocess.DEVNULL, limit=32 * 1024 * 1024, **options)
                        if self.target_identity and cmd not in ("attach_window", "launch_app", "instance", "external_check"):
                            stage = "restore_target"
                            identity = dict(self.target_identity)
                            result = await self.exchange({"_restore_target": identity, "_lifecycle": cmd == "wait_window"})
                            if not result.get("ok"):
                                # Keep the saved process incarnation for lifecycle queries,
                                # but discard the executor whose session was never restored.
                                await self.stop_worker()
                                return result
                    stage = "native_execution"
                    selector = str(payload.get("selector") or "")
                    needs_target = cmd in ("visual_plan", "get_ui_snapshot", "element_screenshot", "assert_screenshot", "highlight_selector", "screenshot") or (
                        cmd == "execute_locator" and (not selector.startswith("exec:") or selector.startswith("exec:close")))
                    if needs_target and self.target_identity is None:
                        return {"ok": False, "code": "target_invalid", "error": "No verified target; attach explicitly"}
                    if self.target_identity and cmd not in ("attach_window", "launch_app", "list_windows", "get_status", "instance", "external_check"):
                        if cmd == "wait_window":
                            valid = native_process_created(self.target_identity["pid"]) == self.target_identity["created"]
                        else:
                            valid = native_target_identity(self.target_identity["hwnd"]) == self.target_identity
                        if not valid:
                            return {"ok": False, "code": "target_invalid", "error": "Saved target exited or changed identity"}
                    if cmd in ("attach_window", "launch_app"):
                        # An interrupted target switch must never silently fall
                        # back to the previous window on the next command.
                        self.target_identity = None
                        self.save_identity()
                    if cmd == "wait_window" and self.target_identity and str(payload.get("pid")) == str(self.target_identity["pid"]):
                        payload = {**payload, "_expected_process_created": self.target_identity["created"]}
                    dispatched = True
                    return await self.exchange(payload)
            except (TimeoutError, asyncio.CancelledError) as exc:
                await self.stop_worker()
                if isinstance(exc, asyncio.CancelledError):
                    raise
                return {"ok": False, "code": "command_timeout", "stage": stage,
                        "timeout_ms": timeout_ms, "request_id": self.active_request,
                        "error": "Executor deadline exceeded; owned executor isolated",
                        "action_outcome": "uncertain" if dispatched else "not_dispatched",
                        "business_result": "unverified", "retry_safe": not dispatched}
            except (OSError, RuntimeError, ValueError) as exc:
                cancelled = self.generation != generation
                await self.stop_worker()
                return {"ok": False, "code": "command_cancelled" if cancelled else "executor_failed",
                        "stage": stage, "error": str(exc), "retry_safe": not dispatched,
                        "business_result": "unverified"}
            finally:
                self.active_request = None
        finally:
            self.lock.release()


async def run_server(
    host: str,
    port: int,
    project_root: Path | None,
    auth_token: str,
) -> None:
    """Run the WinApp sidecar WebSocket server."""
    if not auth_token:
        raise ValueError("WinApp sidecar authentication token is required")
    session = WinAppSupervisor(project_root)
    clients: set[Any] = set()

    async def handler(websocket: Any) -> None:
        clients.add(websocket)
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
                response = await session.execute(payload)
                response["type"] = "response"
                if request_id is not None:
                    response["request_id"] = request_id
                await websocket.send(json.dumps(response))
        finally:
            clients.discard(websocket)

    async def connection_handler(websocket: Any) -> None:
        if authenticated_websocket_path(_websocket_path(websocket), auth_token) is None:
            await websocket.send(json.dumps({"type": "response", "ok": False,
                                            "code": "authentication_failed", "error": "Invalid sidecar credential"}))
            await websocket.close(code=1008, reason="invalid WinApp sidecar token")
            return
        await handler(websocket)

    async def preview_loop() -> None:
        sequence = 0
        while True:
            await asyncio.sleep(FRAME_INTERVAL_SEC)
            if not clients or not session.target_identity or session.lock.locked():
                continue
            response = await session.execute({"cmd": "screenshot", "timeout_ms": 3000}, background=True)
            sequence += 1
            if response.get("ok"):
                frame = {"type": "frame", "data": response.get("screenshot"), "seq": sequence,
                         "url": f"winapp://hwnd/{session.target_identity['hwnd']}" if session.target_identity else "winapp://detached"}
            else:
                frame = {"type": "frame_error", "error": response.get("error"), "code": response.get("code")}
            for client in list(clients):
                with suppress(Exception):
                    await asyncio.wait_for(client.send(json.dumps(frame)), 0.5)

    frame_task = asyncio.create_task(preview_loop())
    try:
        async with websockets.serve(
            connection_handler,
            host,
            port,
            origins=[None],
        ) as server:
            actual_port = server.sockets[0].getsockname()[1]
            ws_url = f"ws://{host}:{actual_port}/?token={auth_token}"
            if project_root is not None:
                write_endpoint_file(project_root, mode="winapp", ws_url=ws_url)
            print(json.dumps({"ready": True, "mode": "winapp", "ws_url": ws_url}), flush=True)
            await asyncio.Future()
    finally:
        frame_task.cancel()
        with suppress(asyncio.CancelledError):
            await frame_task
        await session.stop_worker()


def main() -> None:
    """Parse arguments and start the sidecar."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--executor", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--supervisor-pid", type=int, help=argparse.SUPPRESS)
    parser.add_argument("--mode", default="winapp")
    parser.add_argument("--project-root")
    parser.add_argument("--auth-token")
    args = parser.parse_args()
    project_root = Path(args.project_root).resolve() if args.project_root else None
    if os.name != "nt":
        print("winapp mode is only supported on Windows", file=sys.stderr)
        sys.exit(2)
    if args.executor:
        if not args.supervisor_pid:
            parser.error("executor requires an owning supervisor")
        run_executor(project_root, args.supervisor_pid)
    else:
        asyncio.run(run_server(args.host, args.port, project_root, args.auth_token))


if __name__ == "__main__":
    main()
