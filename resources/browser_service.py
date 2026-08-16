"""Browser bridge for teshi-desktop: embedded Playwright or Chrome extension."""

from __future__ import annotations

import argparse
import asyncio
import base64
import json
import os
import re
import secrets
import socket
import sys
import time
import urllib.error
import urllib.request
import uuid
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import parse_qs, urlparse
from typing import Any

from browser_agent_broker import (
    PROTOCOL_VERSION,
    SCHEMA_VERSION,
    BrokerError,
    BrowserSessionBroker,
    apply_verification_results,
    generate_playwright_candidates,
    operation_success,
)

DEFAULT_DISCOVERY_PORT = 17373
# Extension is considered connected if heartbeat POST was received within this window.
HEARTBEAT_TTL_SEC = 8.0
EXTENSION_FRAME_WS_PATH = "/extension/frames"
FRAME_MAGIC = b"TSH1"
MANAGED_BROWSER_ARTIFACT_SUBDIR = Path(".teshi") / "artifacts" / "browser"
MAX_BROWSER_ARTIFACT_BYTES = 50 * 1024 * 1024
MAX_BROWSER_SCREENSHOT_DIMENSION = 16_384
MAX_BROWSER_SCREENSHOT_PIXELS = 100_000_000
MAX_BROWSER_UPLOAD_FILES = 20
MAX_BROWSER_UPLOAD_FILE_BYTES = 100 * 1024 * 1024
MAX_BROWSER_UPLOAD_TOTAL_BYTES = 250 * 1024 * 1024
MAX_PRIVILEGED_SCRIPT_BYTES = 1024 * 1024
MAX_PRIVILEGED_RESULT_BYTES = 1024 * 1024
MAX_PRIVILEGED_CDP_PARAMS_BYTES = 256 * 1024
MAX_PRIVILEGED_COOKIE_ENTRIES = 500
MAX_BROWSER_WS_MESSAGE_BYTES = 72 * 1024 * 1024
CHROME_EXTENSION_ORIGIN_RE = re.compile(r"^chrome-extension://[a-p]{32}$")
ALLOWED_CONTENT_SETTINGS = {
    "notifications",
    "popups",
    "geolocation",
    "camera",
    "microphone",
    "automatic_downloads",
}
PRIVILEGED_POLICY_FILENAME = "browser-policy.json"


def load_browser_privileged_policy(project_root: Path) -> set[str]:
    """Load explicit user/project privileged capability allowlists; default deny."""
    candidates = [project_root / ".teshi" / PRIVILEGED_POLICY_FILENAME]
    local_app_data = os.environ.get("LOCALAPPDATA") or os.environ.get("APPDATA")
    if local_app_data:
        candidates.append(Path(local_app_data) / "teshi" / PRIVILEGED_POLICY_FILENAME)
    else:
        config_home = os.environ.get("XDG_CONFIG_HOME")
        candidates.append(
            (Path(config_home) if config_home else Path.home() / ".config")
            / "teshi"
            / PRIVILEGED_POLICY_FILENAME
        )
    allowed: set[str] = set()
    for path in candidates:
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError, TypeError):
            continue
        privileged = payload.get("privileged") if isinstance(payload, dict) else None
        raw = privileged.get("allow") if isinstance(privileged, dict) else None
        if isinstance(raw, list):
            allowed.update(
                str(item).strip().lower()
                for item in raw
                if str(item).strip()
            )
    return allowed


def load_browser_raw_cdp_allowlist(project_root: Path) -> set[str]:
    """Load explicitly allowlisted page-scoped CDP domain.method names."""
    path = project_root / ".teshi" / PRIVILEGED_POLICY_FILENAME
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError, TypeError):
        return set()
    privileged = payload.get("privileged") if isinstance(payload, dict) else None
    raw = privileged.get("raw_cdp_methods") if isinstance(privileged, dict) else None
    if not isinstance(raw, list):
        return set()
    return {str(item).strip() for item in raw if str(item).strip()}


def validate_raw_cdp_method(project_root: Path, method: Any) -> str:
    """Enforce explicit policy and reject browser/target/filesystem escape surfaces."""
    normalized = _browser_clean_text(method)
    if not re.fullmatch(r"[A-Za-z][A-Za-z0-9]*\.[A-Za-z][A-Za-z0-9]*", normalized):
        raise BrokerError("invalid_browser_operation", "CDP method must be Domain.method")
    blocked_domains = {
        "Browser",
        "Target",
        "SystemInfo",
        "FileSystem",
        "IO",
        # Runtime and Debugger methods can execute script and therefore require
        # the separately audited `javascript` capability.
        "Runtime",
        "Debugger",
    }
    blocked_methods = {
        "Page.setDownloadBehavior",
        "Browser.setDownloadBehavior",
        "Storage.clearDataForOrigin",
        "DOM.setFileInputFiles",
        "Page.addScriptToEvaluateOnNewDocument",
    }
    accesses_cookies = "cookie" in normalized.lower()
    if (
        normalized.split(".", 1)[0] in blocked_domains
        or normalized in blocked_methods
        or accesses_cookies
    ):
        raise BrokerError(
            "browser_capability_denied",
            "CDP method can escape the selected page or access a separately gated surface",
            {"method": normalized},
        )
    if normalized not in load_browser_raw_cdp_allowlist(project_root):
        raise BrokerError(
            "browser_capability_denied",
            "CDP method is not allowlisted by effective project policy",
            {"method": normalized},
        )
    return normalized


def _browser_clean_text(value: Any) -> str:
    return "" if value is None else str(value).strip()


def _artifact_component(value: Any, fallback: str = "artifact") -> str:
    """Return one bounded filename component without path semantics."""
    cleaned = re.sub(r"[^A-Za-z0-9._-]+", "-", str(value or "")).strip("-._")
    return (cleaned or fallback)[:80]


def validate_browser_upload_files(project_root: Path, raw_paths: Any) -> list[str]:
    """Resolve explicit upload files inside the authorized project without path leaks."""
    if not isinstance(raw_paths, list) or not raw_paths:
        raise BrokerError(
            "invalid_browser_operation", "upload requires at least one explicit file"
        )
    if len(raw_paths) > MAX_BROWSER_UPLOAD_FILES:
        raise BrokerError(
            "invalid_browser_operation",
            "upload exceeds the configured file-count limit",
            {"max_files": MAX_BROWSER_UPLOAD_FILES},
        )
    root = project_root.resolve()
    resolved: list[str] = []
    total_bytes = 0
    for index, raw_path in enumerate(raw_paths):
        candidate = Path(str(raw_path))
        if not candidate.is_absolute():
            candidate = root / candidate
        try:
            path = candidate.resolve(strict=True)
        except (OSError, RuntimeError) as exc:
            raise BrokerError(
                "invalid_browser_operation",
                "upload file is missing or inaccessible",
                {"file_index": index},
            ) from exc
        try:
            path.relative_to(root)
        except ValueError as exc:
            raise BrokerError(
                "browser_capability_denied",
                "upload file is outside the authorized project root",
                {"file_index": index, "policy": "project_root_only"},
            ) from exc
        if not path.is_file():
            raise BrokerError(
                "invalid_browser_operation",
                "upload target is not a regular file",
                {"file_index": index},
            )
        size = path.stat().st_size
        if size > MAX_BROWSER_UPLOAD_FILE_BYTES:
            raise BrokerError(
                "invalid_browser_operation",
                "upload file exceeds the configured byte limit",
                {"file_index": index, "max_file_bytes": MAX_BROWSER_UPLOAD_FILE_BYTES},
            )
        total_bytes += size
        if total_bytes > MAX_BROWSER_UPLOAD_TOTAL_BYTES:
            raise BrokerError(
                "invalid_browser_operation",
                "upload files exceed the configured total byte limit",
                {"max_total_bytes": MAX_BROWSER_UPLOAD_TOTAL_BYTES},
            )
        resolved.append(str(path))
    return resolved


def managed_browser_artifact_path(
    project_root: Path,
    target: dict[str, Any],
    request_id: str,
    extension: str,
) -> Path:
    """Build a project-scoped artifact path correlated to request and target."""
    suffix = _artifact_component(extension.lower().lstrip("."), "bin")[:8]
    profile = _artifact_component(target.get("extension_instance_id"), "profile")[:24]
    window_id = int(target.get("window_id") or 0)
    tab_id = int(target.get("tab_id") or 0)
    request = _artifact_component(request_id, "request")
    filename = f"{request}-{profile}-w{window_id}-t{tab_id}.{suffix}"
    artifact_root = (project_root / MANAGED_BROWSER_ARTIFACT_SUBDIR).resolve()
    path = (artifact_root / filename).resolve()
    if path.parent != artifact_root:
        raise BrokerError("browser_artifact_failure", "invalid managed artifact path")
    return path


def persist_managed_browser_artifact(
    project_root: Path,
    target: dict[str, Any],
    request_id: str,
    page_context_revision: str,
    artifact_format: str,
    payload: bytes,
    warnings: list[str] | None = None,
) -> dict[str, Any]:
    """Persist bounded binary output and return non-inline artifact metadata."""
    if not payload:
        raise BrokerError("browser_artifact_failure", "browser artifact is empty")
    if len(payload) > MAX_BROWSER_ARTIFACT_BYTES:
        raise BrokerError(
            "browser_artifact_failure",
            "browser artifact exceeds the configured byte limit",
            {"max_bytes": MAX_BROWSER_ARTIFACT_BYTES},
        )
    path = managed_browser_artifact_path(
        project_root, target, request_id, artifact_format
    )
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
    except OSError as exc:
        raise BrokerError(
            "browser_artifact_failure", "could not persist browser artifact"
        ) from exc
    return {
        "path": str(path),
        "size": len(payload),
        "format": artifact_format,
        "target": target,
        "request_id": request_id,
        "page_context_revision": page_context_revision,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "warnings": list(warnings or []),
        "managed": True,
    }


def public_browser_artifact_metadata(artifact: dict[str, Any]) -> dict[str, Any]:
    """Whitelist normal JSON output so binary/internal storage state never leaks."""
    return {
        key: artifact[key]
        for key in (
            "path",
            "size",
            "format",
            "target",
            "request_id",
            "page_context_revision",
            "warnings",
        )
    }


def cleanup_managed_browser_artifacts(
    project_root: Path, paths: list[Any]
) -> dict[str, list[str]]:
    """Remove only explicit files directly owned by managed artifact storage."""
    artifact_root = (project_root / MANAGED_BROWSER_ARTIFACT_SUBDIR).resolve()
    removed: list[str] = []
    missing: list[str] = []
    for raw in paths:
        path = Path(str(raw)).resolve()
        if path.parent != artifact_root:
            raise BrokerError(
                "browser_artifact_failure",
                "cleanup path is outside managed browser artifact storage",
            )
        if not path.exists():
            missing.append(str(path))
            continue
        if not path.is_file():
            raise BrokerError(
                "browser_artifact_failure", "cleanup target is not a managed artifact file"
            )
        try:
            path.unlink()
        except OSError as exc:
            raise BrokerError(
                "browser_artifact_failure", "could not remove managed browser artifact"
            ) from exc
        removed.append(str(path))
    return {"removed": removed, "missing": missing}


def debug_enabled() -> bool:
    """Return true when verbose browser bridge diagnostics should be persisted."""
    return bool(str(os.environ.get("TESHI_BROWSER_DEBUG", "")).strip())


def debug_log(project_root: Path | None, event: str, payload: dict[str, Any]) -> None:
    """Append one JSONL diagnostic record under `.teshi/logs` when enabled."""
    if project_root is None or not debug_enabled():
        return
    try:
        log_dir = project_root / ".teshi" / "logs"
        log_dir.mkdir(parents=True, exist_ok=True)
        record = {
            "ts": time.time(),
            "event": event,
            **payload,
        }
        with (log_dir / "browser-bridge.log").open("a", encoding="utf-8") as f:
            f.write(json.dumps(record, ensure_ascii=False) + "\n")
    except OSError:
        return


def _without_windows_extended_prefix(value: str) -> str:
    """Return the ordinary spelling of a Windows extended-length path."""
    if value.startswith("\\\\?\\UNC\\"):
        return "\\\\" + value[8:]
    if value.startswith("\\\\?\\"):
        return value[4:]
    return value


def paths_equal(got: str, expected: Path) -> bool:
    """Compare project roots, including equivalent Windows path spellings."""
    if not got or not str(got).strip():
        return True
    try:
        a = Path(_without_windows_extended_prefix(got)).resolve()
        b = Path(_without_windows_extended_prefix(str(expected))).resolve()
        if a == b:
            return True
        return str(a).casefold() == str(b).casefold()
    except OSError:
        return False

HIGHLIGHT_CONFIG = {
    "showInfo": True,
    "showStyles": True,
    "showRulers": False,
    "showExtensionLines": False,
    "contentColor": {"r": 37, "g": 99, "b": 235, "a": 0.35},
    "borderColor": {"r": 37, "g": 99, "b": 235, "a": 0.9},
}

INTERACTIVE_SELECTOR = (
    "button, [role='button'], input, input[type='submit'], select, "
    "a[href], [role='link'], textarea"
)

INTERACTIVE_EVAL = f"""() => {{
  const elements = Array.from(document.querySelectorAll({json.dumps(INTERACTIVE_SELECTOR)}));
  return elements.slice(0, 60).map(el => ({{
    tag: el.tagName.toLowerCase(),
    text: (el.innerText || el.value || el.getAttribute('aria-label') || '').trim().slice(0, 120),
    id: el.id || null,
    testId: el.getAttribute('data-testid'),
    role: el.getAttribute('role'),
    classes: el.className || null,
  }}));
}}"""

EXECUTE_ACTIONS = {
    "click",
    "pointer_click",
    "fill",
    "type",
    "assert_visible",
    "assert_text",
    "assert_text_count",
    "select",
    "press_key",
}

# ── Engine-inspired JS helpers injected into page context ──

# Short, robust CSS selector generator (port of engine's makeShortSelector)
MAKE_SHORT_SELECTOR_JS = """() => {
  window.__teshiMakeShortSelector = function(e) {
    if (!e || e === document.body || e === document.documentElement) return e ? e.tagName.toLowerCase() : '';
    if (e.id) return '#' + e.id;
    var tid = e.getAttribute('data-testid');
    if (tid) return '[data-testid="' + tid.replace(/"/g,'\\\\"') + '"]';
    var na = e.getAttribute('name');
    if (na) return '[name="' + na.replace(/"/g,'\\\\"') + '"]';
    var aa = e.getAttribute('aria-label');
    if (aa) return '[aria-label="' + aa.replace(/"/g,'\\\\"') + '"]';
    var pa = e.getAttribute('placeholder');
    if (pa) return '[placeholder="' + pa.replace(/"/g,'\\\\"') + '"]';
    var ta = e.getAttribute('title');
    if (ta) return '[title="' + ta.replace(/"/g,'\\\\"') + '"]';
    var href = e.getAttribute('href');
    if (href && e.tagName.toLowerCase() === 'a') return 'a[href*="' + href.replace(/"/g,'\\\\"') + '"]';
    var path = [], cur = e;
    while (cur && cur !== document.body && cur !== document.documentElement) {
      var tag = cur.tagName.toLowerCase(), seg = tag;
      var cls = cur.className;
      var parts = [];
      if (cls && typeof cls === 'string') {
        parts = cls.trim().split(/\\s+/).filter(function(c){
          return c && !/^[a-z]+-[a-z]+-\\d+$/.test(c) && c.indexOf('__')===-1
            && !/^sc-[A-Z]/.test(c) && !/^_[a-z]+_/.test(c);
        }).slice(0,2);
      }
      if (parts.length) { seg += '.' + parts.join('.'); }
      else {
        var p = cur.parentElement;
        if (p) {
          var ch = Array.from(p.children);
          var same = ch.filter(function(s){return s.tagName===cur.tagName;});
          if (same.length>1) seg += ':nth-of-type('+(same.indexOf(cur)+1)+')';
        }
      }
      path.unshift(seg);
      cur = cur.parentElement;
    }
    var vt = (e.innerText||e.textContent||'').trim().substring(0,60);
    if (vt.length>=3) return path.join(' > ')+':has-text("'+vt.replace(/"/g,'\\\\"')+'")';
    return path.join(' > ');
  };
  return true;
}"""

# Rich element snapshot (port of engine's getElementSnapshot)
GET_ELEMENT_SNAPSHOT_JS = """(selector) => {
  var el = document.querySelector(selector);
  if (!el) return null;
  function computeAccessibleName(el) {
    var lb = el.getAttribute('aria-labelledby');
    if (lb) { var ids=lb.split(/\\s+/), parts=[]; for(var i=0;i<ids.length;i++){var ref=document.getElementById(ids[i]);if(ref){var t=(ref.textContent||'').trim();if(t)parts.push(t);}} if(parts.length) return parts.join(' '); }
    var al = el.getAttribute('aria-label'); if(al) return al.trim();
    var tag=el.tagName.toLowerCase(); if((tag==='img'||tag==='area'||el.getAttribute('role')==='img')){var alt=el.getAttribute('alt');if(alt)return alt.trim();}
    if(el.labels&&el.labels.length){var lt=el.labels[0].textContent.trim();if(lt)return lt;}
    var ti=el.getAttribute('title'); if(ti) return ti.trim();
    if(tag==='button'||tag==='a'||el.getAttribute('role')==='button'||el.getAttribute('role')==='link'||el.getAttribute('role')==='menuitem'){var ct=(el.innerText||el.textContent||'').trim();if(ct)return ct.substring(0,120);}
    if(tag==='input'){var it=(el.getAttribute('type')||'text').toLowerCase();if(it==='button'||it==='submit'||it==='reset'){var v=el.getAttribute('value');if(v)return v.trim();}}
    return null;
  }
  var txt = (el.textContent||'').trim().substring(0,120);
  var attrs = {}; try{var names=el.getAttributeNames?el.getAttributeNames():[];for(var i=0;i<names.length;i++){var n=names[i],v=el.getAttribute(n);if(v!==null&&v!==undefined)attrs[n]=v;}}catch(e){}
  var rect = null; try{var r=el.getBoundingClientRect();rect={x:r.x,y:r.y,width:r.width,height:r.height};}catch(e){}
  var styles=null; try{var cs=window.getComputedStyle(el);styles={display:cs.display,visibility:cs.visibility,opacity:cs.opacity,position:cs.position,pointerEvents:cs.pointerEvents};}catch(e){}
  var parent=el.parentElement, parentRect=null;
  if(parent){try{var pr=parent.getBoundingClientRect();parentRect={x:pr.x,y:pr.y,width:pr.width,height:pr.height};}catch(e){}}
  var siblingIndex=-1,totalSiblings=0;
  if(parent){var children=Array.from(parent.children);totalSiblings=children.length;siblingIndex=children.indexOf(el);}
  var shortSel=window.__teshiMakeShortSelector?window.__teshiMakeShortSelector(el):null;
  var parentSel=parent&&window.__teshiMakeShortSelector?window.__teshiMakeShortSelector(parent):null;
  return {
    id: el.id||null,
    testid: el.getAttribute('data-testid')||null,
    ariaLabel: el.getAttribute('aria-label')||null,
    role: el.getAttribute('role')||null,
    placeholder: el.getAttribute('placeholder')||null,
    name: el.getAttribute('name')||null,
    tag: tag,
    text: txt,
    classes: (el.className&&typeof el.className==='string')?el.className.trim().split(/\\s+/).slice(0,3).join('.'):'',
    label: (el.labels&&el.labels.length)?el.labels[0].textContent.trim():null,
    alt: el.getAttribute('alt')||null,
    title: el.getAttribute('title')||null,
    computedAccessibleName: computeAccessibleName(el),
    allAttributes: attrs,
    rect: rect,
    computedStyles: styles,
    shortSelector: shortSel,
    domPath: shortSel,
    parentTag: parent?parent.tagName.toLowerCase():null,
    parentSelector: parentSel,
    siblingIndex: siblingIndex,
    totalSiblings: totalSiblings,
    parentBoundingRect: parentRect,
    inShadowDOM: el.getRootNode?el.getRootNode() instanceof ShadowRoot:false,
    inIframe: window!==window.top,
  };
}"""

# Probe element for the best locator by priority (port of SmartLocatorEnhancer._probe_element)
PROBE_LOCATOR_JS = """(selector) => {
  var el = document.querySelector(selector);
  if (!el) return null;
  var result = { testid: el.getAttribute('data-testid'), ariaLabel: el.getAttribute('aria-label'),
    role: el.getAttribute('role')||(el.tagName==='A'?'link':el.tagName==='BUTTON'?'button':el.tagName==='INPUT'?(el.getAttribute('type')||'textbox'):null),
    placeholder: el.getAttribute('placeholder'), name: el.getAttribute('name'),
    label: (el.labels&&el.labels.length)?el.labels[0].textContent.trim():null,
    text: (el.textContent||'').trim().substring(0,120), tag: el.tagName.toLowerCase(),
    alt: el.getAttribute('alt')||null, title: el.getAttribute('title')||null };
  return result;
}"""


def normalize_snapshot(
    url: str,
    title: str,
    accessibility_tree: Any,
    interactive_elements: list[Any],
) -> dict[str, Any]:
    """Shared response shape for embedded and chrome modes."""
    return {
        "ok": True,
        "url": url,
        "title": title,
        "accessibility_tree": accessibility_tree,
        "interactive_elements": interactive_elements,
    }


def write_cdp_endpoint_file(
    project_root: Path,
    *,
    mode: str,
    ws_url: str,
    page_url: str,
    discovery_port: int | None = None,
    cdp_http_url: str | None = None,
    extension_connected: bool = False,
    extension_frame_ws_url: str | None = None,
    broker_pid: int | None = None,
    broker_start_id: str | None = None,
) -> None:
    teshi_dir = project_root / ".teshi"
    teshi_dir.mkdir(parents=True, exist_ok=True)
    payload: dict[str, Any] = {
        "mode": mode,
        "ws_url": ws_url,
        "page_url": page_url,
        "bridge": "python",
        "extension_connected": extension_connected,
    }
    if mode == "embedded":
        payload["viewport"] = {"width": 1920, "height": 1080}
        if cdp_http_url:
            payload["http_url"] = cdp_http_url
    if mode == "chrome" and discovery_port is not None:
        payload["discovery_url"] = f"http://127.0.0.1:{discovery_port}/v1/bridge"
        payload["schema_version"] = SCHEMA_VERSION
        payload["protocol_version"] = PROTOCOL_VERSION
        payload["broker_pid"] = broker_pid or os.getpid()
        if broker_start_id:
            payload["broker_start_id"] = broker_start_id
    if extension_frame_ws_url:
        payload["extension_frame_ws_url"] = extension_frame_ws_url
    (teshi_dir / "cdp-endpoint.json").write_text(
        json.dumps(payload, indent=2),
        encoding="utf-8",
    )


def parse_tsh1_frame(data: bytes) -> tuple[dict[str, Any], bytes] | None:
    """Parse extension binary frame: magic TSH1 + meta_len + meta JSON + JPEG."""
    if len(data) < 8 or data[:4] != FRAME_MAGIC:
        return None
    meta_len = int.from_bytes(data[4:8], "little")
    end_meta = 8 + meta_len
    if len(data) < end_meta:
        return None
    try:
        meta = json.loads(data[8:end_meta].decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return None
    if not isinstance(meta, dict):
        return None
    return meta, data[end_meta:]


def build_frame_out_sync(meta: dict[str, Any], jpeg: bytes) -> dict[str, Any]:
    """Build desktop WebSocket frame payload (base64 JPEG) off the event loop."""
    frame_out: dict[str, Any] = {
        "type": "frame",
        "data": base64.b64encode(jpeg).decode("ascii"),
        "url": str(meta.get("url", "")),
    }
    raw_tab = meta.get("tab_id")
    if raw_tab is not None:
        try:
            frame_out["tab_id"] = int(raw_tab)
        except (TypeError, ValueError):
            pass
    instance_id = str(meta.get("extension_instance_id", "")).strip()
    raw_window = meta.get("window_id")
    if instance_id and raw_window is not None and raw_tab is not None:
        try:
            frame_out["extension_instance_id"] = instance_id
            frame_out["target"] = {
                "extension_instance_id": instance_id,
                "window_id": int(raw_window),
                "tab_id": int(raw_tab),
            }
        except (TypeError, ValueError):
            pass
    if meta.get("request_id"):
        frame_out["request_id"] = str(meta["request_id"])
    return frame_out


def fetch_playwright_cdp_endpoint(cdp_port: int) -> dict[str, Any]:
    url = f"http://127.0.0.1:{cdp_port}/json/version"
    with urllib.request.urlopen(url, timeout=5) as response:
        payload = json.loads(response.read().decode("utf-8"))
    return {
        "ws_url": payload.get("webSocketDebuggerUrl", ""),
        "http_url": f"http://127.0.0.1:{cdp_port}",
    }


# --- Embedded (Playwright) backend ---


class EmbeddedSession:
    def __init__(self) -> None:
        self.page = None
        self.browser = None
        self.context = None
        self.playwright = None
        self.cdp_session = None
        self._lock = asyncio.Lock()

    async def start(self, cdp_port: int) -> None:
        from playwright.async_api import async_playwright

        self.playwright = await async_playwright().start()
        self.browser = await self.playwright.chromium.launch(
            headless=True,
            args=[f"--remote-debugging-port={cdp_port}"],
        )
        self.context = await self.browser.new_context(
            viewport={"width": 1920, "height": 1080}
        )
        self.page = await self.context.new_page()
        await self.page.goto("about:blank")
        self.cdp_session = await self.context.new_cdp_session(self.page)

        # Inject engine-inspired JS helpers
        await self.page.evaluate(MAKE_SHORT_SELECTOR_JS)

    def current_url(self) -> str:
        if self.page is None:
            return "about:blank"
        return self.page.url

    async def navigate(self, url: str) -> None:
        async with self._lock:
            if self.page is not None:
                await self.page.goto(url, wait_until="domcontentloaded")

    async def screenshot_jpeg_b64(self) -> str:
        async with self._lock:
            if self.page is None:
                return ""
            png = await self.page.screenshot(type="jpeg", quality=70)
            return base64.b64encode(png).decode("ascii")

    async def clear_highlight(self) -> None:
        async with self._lock:
            if self.cdp_session is None:
                return
            await self.cdp_session.send("Overlay.hideHighlight", {})

    async def highlight_selector(self, selector: str) -> dict[str, Any]:
        async with self._lock:
            if self.page is None or self.cdp_session is None:
                return {"ok": False, "error": "browser not ready"}

            await self.cdp_session.send("Overlay.hideHighlight", {})
            locator = self.page.locator(selector)
            count = await locator.count()
            if count == 0:
                return {"ok": False, "error": f"selector matched no elements: {selector}"}
            if count > 1:
                return {
                    "ok": False,
                    "error": f"selector matched {count} elements; refine selector",
                }

            object_result = await self.cdp_session.send(
                "Runtime.evaluate",
                {
                    "expression": f"document.querySelector({json.dumps(selector)})",
                    "returnByValue": False,
                },
            )
            object_id = object_result.get("result", {}).get("objectId")
            if not object_id:
                return {"ok": False, "error": "could not evaluate selector in page context"}

            node_result = await self.cdp_session.send(
                "DOM.requestNode",
                {"objectId": object_id},
            )
            node_id = node_result.get("nodeId")
            if not node_id:
                return {"ok": False, "error": "could not resolve node id"}

            await self.cdp_session.send(
                "Overlay.highlightNode",
                {"highlightConfig": HIGHLIGHT_CONFIG, "nodeId": node_id},
            )
            box = await locator.bounding_box()
            return {
                "ok": True,
                "selector": selector,
                "node_id": node_id,
                "bounding_box": box,
            }

    async def execute_locator(
        self,
        selector: str,
        action: str,
        value: str | None = None,
        timeout_ms: int = 5000,
        candidate: dict[str, Any] | None = None,
        focus: bool = False,
    ) -> dict[str, Any]:
        if self.page is None:
            return {"ok": False, "error": "browser not ready", "code": "browser_not_ready"}
        if not selector and not candidate:
            return {"ok": False, "error": "selector or candidate is required", "code": "invalid_selector"}
        if action not in EXECUTE_ACTIONS:
            return {
                "ok": False,
                "error": f"unsupported action: {action}",
                "code": "unsupported_action",
            }
        if action in {"fill", "assert_text", "assert_text_count", "select", "press_key", "type"} and value is None:
            return {
                "ok": False,
                "error": f"value is required for {action}",
                "code": "missing_value",
            }

        async with self._lock:
            locator = self._action_locator(selector, candidate)
            try:
                await locator.wait_for(state="visible", timeout=timeout_ms)
                if action == "click":
                    await locator.click(timeout=timeout_ms)
                elif action == "pointer_click":
                    box = await locator.bounding_box()
                    if box is None:
                        return {"ok": False, "error": "element has no pointer hit point", "code": "pointer_hit_test_failed"}
                    x = box["x"] + box["width"] / 2
                    y = box["y"] + box["height"] / 2
                    await self.page.mouse.click(x, y)
                elif action == "fill":
                    await locator.fill(value or "", timeout=timeout_ms)
                elif action == "type":
                    await locator.click(timeout=timeout_ms)
                    await locator.press_sequentially(value or "", timeout=timeout_ms)
                elif action == "assert_visible":
                    pass
                elif action == "assert_text":
                    text = await locator.inner_text(timeout=timeout_ms)
                    if (value or "") not in text:
                        return {
                            "ok": False,
                            "error": "text assertion failed",
                            "code": "assert_text_failed",
                            "actual": text,
                        }
                elif action == "assert_text_count":
                    text = await locator.inner_text(timeout=timeout_ms)
                    actual_count = text.count(value or "")
                    if actual_count != 1:
                        return {
                            "ok": False,
                            "error": f"text '{value}' found {actual_count} times, expected 1",
                            "code": "assert_text_count_failed",
                            "actual_count": actual_count,
                        }
                elif action == "select":
                    await locator.select_option(value or "", timeout=timeout_ms)
                elif action == "press_key":
                    await locator.press(value or "", timeout=timeout_ms)
            except Exception as exc:  # noqa: BLE001
                message = str(exc)
                code = "timeout" if "Timeout" in message else "execute_failed"
                if action == "assert_visible" and "hidden" in message.lower():
                    message = (
                        f"{message}; element may be on a hidden panel — "
                        "switch tabs first (e.g. click FileTreeTab)"
                    )
                return {"ok": False, "error": message, "code": code}

            return {
                "ok": True,
                "selector": selector or None,
                "candidate": candidate,
                "action": action,
                "focus": {"requested": focus, "changed": action == "pointer_click"},
            }

    def _action_locator(
        self, selector: str, candidate: dict[str, Any] | None
    ) -> Any:
        if candidate is None:
            return self.page.locator(selector).first
        return self._playwright_locator(candidate).first

    async def wait_for_browser_condition(
        self,
        wait: dict[str, Any] | None,
        timeout_ms: int,
        selector: str,
        candidate: dict[str, Any] | None,
    ) -> dict[str, Any] | None:
        if not wait:
            return None
        try:
            kind = str(wait.get("kind") or "")
            if kind == "url":
                pattern = str(wait.get("pattern") or "")
                await self.page.wait_for_function(
                    "pattern => location.href.includes(pattern)",
                    pattern,
                    timeout=timeout_ms,
                )
            elif kind == "visible_text":
                await self.page.get_by_text(
                    str(wait.get("text") or ""), exact=False
                ).first.wait_for(state="visible", timeout=timeout_ms)
            elif kind == "page_revision_change":
                previous = str(wait.get("from") or "")
                deadline = time.monotonic() + timeout_ms / 1000
                while time.monotonic() < deadline:
                    current = await self.get_page_snapshot()
                    if str(current.get("page_context_revision") or "") != previous:
                        break
                    await asyncio.sleep(0.08)
                else:
                    raise TimeoutError("page revision did not change")
            elif kind == "load_complete":
                await self.page.wait_for_load_state("load", timeout=timeout_ms)
            elif kind == "element_state":
                state = str(wait.get("state") or "")
                locator = self._action_locator(selector, candidate)
                if state in {"visible", "hidden"}:
                    await locator.wait_for(state=state, timeout=timeout_ms)
                elif state == "enabled":
                    await locator.wait_for(state="visible", timeout=timeout_ms)
                    if not await locator.is_enabled():
                        raise TimeoutError("element is disabled")
                elif state == "disabled":
                    await locator.wait_for(state="visible", timeout=timeout_ms)
                    if await locator.is_enabled():
                        raise TimeoutError("element is enabled")
            return {"ok": True, "condition": wait}
        except Exception as exc:  # noqa: BLE001
            return {
                "ok": False,
                "code": "browser_wait_timeout",
                "error": str(exc),
                "condition": wait,
            }

    async def open_project_via_api(self, path: str) -> dict[str, Any]:
        async with self._lock:
            if self.page is None:
                return {"ok": False, "error": "browser not ready", "code": "browser_not_ready"}
            if not path.strip():
                return {"ok": False, "error": "path is required", "code": "missing_path"}
            try:
                result = await self.page.evaluate(
                    """async (projectPath) => {
                        if (typeof window.__teshiE2eOpenProject === 'function') {
                            await window.__teshiE2eOpenProject(projectPath);
                            return { ok: true };
                        }
                        const res = await fetch('/api/v1/projects/open', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({ path: projectPath }),
                        });
                        if (!res.ok) {
                            let message = res.statusText;
                            try {
                                const body = await res.json();
                                if (body && body.error) message = body.error;
                            } catch (_) {}
                            throw new Error(message);
                        }
                        return { ok: true };
                    }""",
                    path,
                )
                if isinstance(result, dict) and not result.get("ok", True):
                    return {"ok": False, "error": "open_project evaluate failed", "code": "open_project_failed"}
                await self.page.wait_for_selector(
                    '[data-testid="FileTreeTab"], [data-testid="TerminalTab"]',
                    timeout=15_000,
                )
                return {"ok": True, "path": path, "url": self.page.url}
            except Exception as exc:  # noqa: BLE001
                return {"ok": False, "error": str(exc), "code": "open_project_failed"}

    async def get_page_snapshot(self) -> dict[str, Any]:
        async with self._lock:
            if self.page is None:
                return {"ok": False, "error": "browser not ready"}

            title = await self.page.title()
            url = self.page.url
            try:
                tree = await self.page.accessibility.snapshot(interesting_only=False)
            except Exception as exc:  # noqa: BLE001
                tree = {"error": str(exc)}

            buttons = await self.page.evaluate(INTERACTIVE_EVAL)
            revision = await self.page.evaluate(
                """() => {
                    if (!globalThis.__teshiPageContextRevision) {
                        globalThis.__teshiPageContextRevision =
                            (globalThis.crypto?.randomUUID?.() || `${Date.now()}-${Math.random()}`);
                    }
                    return globalThis.__teshiPageContextRevision;
                }"""
            )

            # Also inject __teshiMakeShortSelector if not already present
            try:
                await self.page.evaluate(
                    "typeof window.__teshiMakeShortSelector === 'function'"
                )
            except Exception:
                await self.page.evaluate(MAKE_SHORT_SELECTOR_JS)

            snapshot = normalize_snapshot(url, title, tree, buttons)
            snapshot["page_context_revision"] = str(revision)
            return snapshot

    async def verify_playwright_candidates(
        self,
        candidates: list[dict[str, Any]],
        expected_revision: str,
    ) -> dict[str, Any]:
        """Evaluate structured Playwright candidates in the current document."""
        async with self._lock:
            if self.page is None:
                return {"ok": False, "code": "browser_unavailable", "error": "browser not ready"}
            current_revision = str(
                await self.page.evaluate(
                    """() => globalThis.__teshiPageContextRevision || ''"""
                )
            )
            if expected_revision and current_revision != expected_revision:
                return {
                    "ok": False,
                    "code": "stale_page_context",
                    "error": "page changed after the locator snapshot was acquired",
                    "page_context_revision": current_revision,
                }
            verification: list[dict[str, Any]] = []
            for candidate in candidates:
                expression = str(candidate.get("expression", ""))
                try:
                    locator = self._playwright_locator(candidate)
                    count = await locator.count()
                    visible = count > 0 and await locator.first.is_visible()
                    enabled = count > 0 and await locator.first.is_enabled()
                    verification.append(
                        {
                            "expression": expression,
                            "match_count": count,
                            "visible": visible,
                            "enabled": enabled,
                        }
                    )
                except Exception as exc:  # noqa: BLE001
                    verification.append(
                        {
                            "expression": expression,
                            "match_count": 0,
                            "visible": False,
                            "enabled": False,
                            "error": str(exc),
                        }
                    )
            return {
                "ok": True,
                "page_context_revision": current_revision,
                "verification": verification,
            }

    def _playwright_locator(self, candidate: dict[str, Any]) -> Any:
        """Build a Playwright locator from structured arguments without eval."""
        if self.page is None:
            raise RuntimeError("browser not ready")
        context: Any = self.page
        locator_context = candidate.get("context")
        frame_hint = (
            str(locator_context.get("frame", ""))
            if isinstance(locator_context, dict)
            else ""
        )
        if frame_hint:
            matched = next(
                (
                    frame
                    for frame in self.page.frames
                    if frame_hint in frame.url or frame.name == frame_hint
                ),
                None,
            )
            if matched is not None:
                context = matched
        kind = str(candidate.get("kind", ""))
        arguments = candidate.get("arguments")
        args = arguments if isinstance(arguments, dict) else {}
        if kind == "role":
            return context.get_by_role(
                str(args.get("role", "")),
                name=str(args.get("name", "")),
                exact=bool(args.get("exact", True)),
            )
        if kind == "label":
            return context.get_by_label(
                str(args.get("text", "")), exact=bool(args.get("exact", True))
            )
        if kind == "placeholder":
            return context.get_by_placeholder(
                str(args.get("text", "")), exact=bool(args.get("exact", True))
            )
        if kind in {"test_id", "attribute"}:
            attribute = str(args.get("attribute", "data-testid"))
            value = str(args.get("value", ""))
            return context.locator(f"[{attribute}={json.dumps(value)}]")
        if kind == "text":
            return context.get_by_text(
                str(args.get("text", "")), exact=bool(args.get("exact", True))
            )
        if kind == "css":
            return context.locator(str(args.get("selector", "")))
        raise ValueError(f"unsupported Playwright locator kind: {kind}")

    async def smart_enhance_locator(self, selector: str) -> dict[str, Any]:
        """Probe an element for the best-priority locator.
        Priority: testid > role+aria-label > role+text > label > placeholder > css
        """
        async with self._lock:
            if self.page is None:
                return {"ok": False, "error": "browser not ready"}

            # Ensure JS helper is injected
            try:
                await self.page.evaluate(
                    "typeof window.__teshiMakeShortSelector === 'function'"
                )
            except Exception:
                await self.page.evaluate(MAKE_SHORT_SELECTOR_JS)

            attrs = await self.page.evaluate(PROBE_LOCATOR_JS, selector)
            if not attrs:
                return {"ok": False, "error": "selector matched no elements"}

            # Also get rich snapshot
            snapshot = await self.page.evaluate(GET_ELEMENT_SNAPSHOT_JS, selector)

            candidates = []

            # Priority: testid > role+name > label > placeholder > text > css
            if attrs.get("testid"):
                candidates.append({
                    "strategy": "testid",
                    "value": f'[data-testid="{attrs["testid"]}"]',
                    "confidence": 0.95,
                    "rationale": "data-testid attribute",
                })

            role = attrs.get("role")
            aria_label = attrs.get("ariaLabel")
            if role and aria_label:
                candidates.append({
                    "strategy": "role",
                    "value": role,
                    "name": aria_label,
                    "confidence": 0.90,
                    "rationale": f"role={role} + aria-label",
                })

            if role and attrs.get("text"):
                candidates.append({
                    "strategy": "role",
                    "value": role,
                    "name": attrs["text"],
                    "confidence": 0.80,
                    "rationale": f"role={role} + visible text",
                })

            if attrs.get("label"):
                candidates.append({
                    "strategy": "label",
                    "value": f'[aria-label="{attrs["label"]}"]',
                    "confidence": 0.75,
                    "rationale": "label text",
                })

            if attrs.get("placeholder"):
                candidates.append({
                    "strategy": "placeholder",
                    "value": attrs["placeholder"],
                    "confidence": 0.65,
                    "rationale": "placeholder text",
                })

            if attrs.get("text") and len(attrs["text"]) > 1:
                candidates.append({
                    "strategy": "text",
                    "value": attrs["text"],
                    "confidence": 0.50,
                    "rationale": "visible text",
                })

            # Get short selector from makeShortSelector
            short_css = snapshot.get("shortSelector") if snapshot else None
            if short_css:
                candidates.append({
                    "strategy": "css",
                    "value": short_css,
                    "confidence": 0.70,
                    "rationale": "short CSS selector from DOM analysis",
                })

            return {
                "ok": True,
                "selector": selector,
                "candidates": candidates,
                "snapshot": snapshot,
            }

    async def heal_execute_locator(
        self,
        selector: str,
        action: str,
        value: str | None = None,
        timeout_ms: int = 5000,
    ) -> dict[str, Any]:
        """Execute a locator action with self-healing retry chain.

        On failure, attempts alternative strategies (attribute matching,
        text matching, DOM path shortening) before reporting failure.
        """
        result = await self.execute_locator(selector, action, value, timeout_ms)
        if result.get("ok"):
            return result

        # Self-healing: try alternative selectors
        from urllib.parse import urlparse

        healed_selector = None
        healed_strategy = None

        # 1. Try attribute-based matching (data-testid, aria-label, etc.)
        snapshot = await self.page.evaluate(GET_ELEMENT_SNAPSHOT_JS, selector)
        if snapshot:
            attrs = snapshot.get("allAttributes", {})
            priority_attrs = ["data-testid", "aria-label", "role", "name", "title", "alt"]
            for attr_name in priority_attrs:
                attr_value = attrs.get(attr_name)
                if attr_value and len(str(attr_value)) <= 150:
                    attempted = f'[{attr_name}="{attr_value}"]'
                    r = await self.execute_locator(attempted, action, value, timeout_ms)
                    if r.get("ok"):
                        healed_selector = attempted
                        healed_strategy = f"attr_{attr_name}"
                        break
            if not healed_selector:
                # Try all other non-style/class attributes
                for attr_name, attr_value in attrs.items():
                    if not attr_value or len(str(attr_value)) > 150:
                        continue
                    if attr_name in ("style", "class", "id", *priority_attrs):
                        continue
                    attempted = f'[{attr_name}="{attr_value}"]'
                    r = await self.execute_locator(attempted, action, value, timeout_ms)
                    if r.get("ok"):
                        healed_selector = attempted
                        healed_strategy = "attr_match"
                        break

        # 2. Try text matching
        if not healed_selector and snapshot and snapshot.get("text"):
            text_val = snapshot["text"]
            if len(text_val) >= 2:
                try:
                    text_loc = self.page.get_by_text(text_val, exact=False)
                    count = await text_loc.count()
                    if count > 0:
                        r = await self.execute_locator(
                            f':has-text("{text_val}")', action, value, timeout_ms
                        )
                        if r.get("ok"):
                            healed_selector = f':has-text("{text_val}")'
                            healed_strategy = "text_match"
                except Exception:
                    pass

        # 3. Try DOM path shortening
        if not healed_selector and snapshot and snapshot.get("domPath"):
            dom_path = snapshot["domPath"]
            parts = dom_path.split(" > ")
            for i in range(len(parts) - 1, 0, -1):
                shorter = " > ".join(parts[:i]) + " > *"
                r = await self.execute_locator(shorter, action, value, timeout_ms)
                if r.get("ok"):
                    healed_selector = shorter
                    healed_strategy = "dom_path"
                    break

        if healed_selector:
            return {
                "ok": True,
                "selector": healed_selector,
                "action": action,
                "healed": True,
                "original_selector": selector,
                "healed_strategy": healed_strategy,
            }

        return result


async def handle_embedded_command(
    session: EmbeddedSession,
    data: dict[str, Any],
    broker: BrowserSessionBroker | None = None,
    project_root: Path | None = None,
) -> dict[str, Any]:
    cmd = data.get("cmd")
    request_id = data.get("request_id")

    if broker is not None:
        record = broker.register_heartbeat(
            {
                "extension_instance_id": "embedded-session",
                "profile_label": "Embedded Chromium",
                "extension_version": "embedded",
                "protocol_version": PROTOCOL_VERSION,
                "browser": {"name": "Chromium", "version": "", "platform": sys.platform},
                "active_window_id": 0,
                "active_tab_id": 1,
                "url": session.current_url(),
                "title": "Embedded Chromium",
                "windows": [
                    {
                        "id": 0,
                        "focused": True,
                        "tabs": [
                            {
                                "id": 1,
                                "window_id": 0,
                                "url": session.current_url(),
                                "title": "Embedded Chromium",
                                "active": True,
                                "debuggable": True,
                            }
                        ],
                    }
                ],
            }
        )
        if cmd == "list_browser_sessions":
            return operation_success(
                str(cmd), str(request_id or ""), sessions=broker.list_sessions()
            )
        if cmd == "cleanup_browser_artifacts":
            if project_root is None:
                raise BrokerError(
                    "browser_artifact_failure", "project root is unavailable for cleanup"
                )
            cleaned = cleanup_managed_browser_artifacts(
                project_root,
                data.get("paths") if isinstance(data.get("paths"), list) else [],
            )
            return operation_success(str(cmd), str(request_id or ""), **cleaned)
        if cmd == "list_browser_tabs":
            try:
                tabs = broker.list_tabs(str(data.get("extension_instance_id") or ""))
                return operation_success(str(cmd), str(request_id or ""), **tabs)
            except BrokerError as exc:
                return exc.response(str(request_id or ""), str(cmd))
        if cmd == "acquire_browser_lease":
            try:
                lease = broker.acquire_lease(
                    str(data.get("extension_instance_id") or ""),
                    str(data.get("owner_label") or "external-agent"),
                    data.get("ttl_secs"),
                )
                return operation_success(str(cmd), str(request_id or ""), lease=lease)
            except BrokerError as exc:
                return exc.response(str(request_id or ""), str(cmd))
        if cmd == "renew_browser_lease":
            try:
                lease = broker.renew_lease(
                    str(data.get("extension_instance_id") or ""),
                    str(data.get("lease_token") or ""),
                    data.get("ttl_secs"),
                )
                return operation_success(str(cmd), str(request_id or ""), lease=lease)
            except BrokerError as exc:
                return exc.response(str(request_id or ""), str(cmd))
        if cmd == "release_browser_lease":
            try:
                released = broker.release_lease(
                    str(data.get("extension_instance_id") or ""),
                    str(data.get("lease_token") or ""),
                )
                return operation_success(str(cmd), str(request_id or ""), **released)
            except BrokerError as exc:
                return exc.response(str(request_id or ""), str(cmd))
        if cmd in {
            "resolve_playwright_locator",
            "verify_playwright_locator",
            "capture_browser_evidence",
            "capture_browser_screenshot",
            "generate_browser_pdf",
            "execute_browser_action",
            "navigate",
            "go_back",
        } or (cmd == "get_page_snapshot" and data.get("target") is not None):
            try:
                _record, target, _ephemeral = broker.authorize_command(
                    data, legacy_compatibility=False
                )
                if cmd == "get_page_snapshot":
                    snapshot = await session.get_page_snapshot()
                    snapshot_response = {
                        "request_id": str(request_id or ""),
                        **snapshot,
                    }
                    broker.cache_snapshot_references(
                        _record, target, snapshot_response
                    )
                    return operation_success(
                        str(cmd),
                        str(request_id or ""),
                        target=target,
                        **{
                            key: value
                            for key, value in snapshot_response.items()
                            if key not in {"ok", "request_id"}
                        },
                    )
                if cmd == "navigate":
                    await session.navigate(str(data.get("url") or "about:blank"))
                    action_result = {
                        "ok": True,
                        "url": session.current_url(),
                    }
                    wait_result = await session.wait_for_browser_condition(
                        data.get("wait")
                        if isinstance(data.get("wait"), dict)
                        else None,
                        int(data.get("timeout_ms") or 15000),
                        "html",
                        None,
                    )
                    return operation_success(
                        str(cmd),
                        str(request_id or ""),
                        target=target,
                        action_outcome=action_result,
                        wait_outcome=wait_result,
                        url=session.current_url(),
                    )
                if cmd == "go_back":
                    await session.page.go_back(
                        wait_until="load", timeout=int(data.get("timeout_ms") or 15000)
                    )
                    return operation_success(
                        str(cmd),
                        str(request_id or ""),
                        target=target,
                        url=session.current_url(),
                    )
                if cmd == "capture_browser_screenshot":
                    if project_root is None:
                        raise BrokerError(
                            "browser_artifact_failure",
                            "project root is unavailable for managed artifacts",
                        )
                    snapshot = await session.get_page_snapshot()
                    revision = str(snapshot.get("page_context_revision") or "")
                    expected = _browser_clean_text(data.get("page_context_revision"))
                    if expected and expected != revision:
                        raise BrokerError(
                            "stale_page_context",
                            "page changed before screenshot capture",
                            {"page_context_revision": revision},
                        )
                    artifact_format = _browser_clean_text(data.get("format")) or "png"
                    if artifact_format not in {"png", "jpeg"}:
                        raise BrokerError(
                            "invalid_browser_operation", "format must be png or jpeg"
                        )
                    options: dict[str, Any] = {"type": artifact_format}
                    element = data.get("element")
                    element_locator = None
                    if element is not None:
                        if not isinstance(element, dict) or data.get("full_page"):
                            raise BrokerError(
                                "invalid_browser_operation",
                                "element screenshot input is invalid",
                            )
                        choices = [
                            bool(_browser_clean_text(element.get("reference"))),
                            isinstance(element.get("candidate"), dict),
                            bool(_browser_clean_text(element.get("css"))),
                        ]
                        if sum(choices) != 1:
                            raise BrokerError(
                                "invalid_browser_operation",
                                "exactly one element input is required",
                            )
                        candidate = dict(element["candidate"]) if choices[1] else None
                        selector = _browser_clean_text(element.get("css"))
                        if choices[0]:
                            reference = broker.resolve_element_reference(
                                _record.extension_instance_id,
                                target,
                                _browser_clean_text(element.get("reference")),
                                page_context_revision=_browser_clean_text(
                                    element.get("page_context_revision")
                                ),
                                snapshot_id=_browser_clean_text(element.get("snapshot_id")),
                            )
                            selector = _browser_clean_text(
                                reference.element.get("shortSelector")
                            )
                            raw_candidate = reference.element.get("candidate")
                            if isinstance(raw_candidate, dict):
                                candidate = raw_candidate
                        element_locator = session._action_locator(selector, candidate)
                        if await element_locator.count() != 1:
                            raise BrokerError(
                                "stale_element_reference",
                                "element screenshot target is no longer unique",
                            )
                    if data.get("full_page"):
                        dimensions = await session.page.evaluate(
                            "() => ({width: Math.max(document.documentElement.scrollWidth, document.body?.scrollWidth || 0), height: Math.max(document.documentElement.scrollHeight, document.body?.scrollHeight || 0)})"
                        )
                        width = int(dimensions.get("width") or 0)
                        height = int(dimensions.get("height") or 0)
                        if (
                            width > MAX_BROWSER_SCREENSHOT_DIMENSION
                            or height > MAX_BROWSER_SCREENSHOT_DIMENSION
                            or width * height > MAX_BROWSER_SCREENSHOT_PIXELS
                        ):
                            raise BrokerError(
                                "browser_artifact_failure",
                                "full-page screenshot exceeds configured dimension limits",
                                {
                                    "width": width,
                                    "height": height,
                                    "max_dimension": MAX_BROWSER_SCREENSHOT_DIMENSION,
                                    "max_pixels": MAX_BROWSER_SCREENSHOT_PIXELS,
                                },
                            )
                        options["full_page"] = True
                    if artifact_format == "jpeg":
                        options["quality"] = int(data.get("quality") or 80)
                    payload = (
                        await element_locator.screenshot(**options)
                        if element_locator is not None
                        else await session.page.screenshot(**options)
                    )
                    artifact = persist_managed_browser_artifact(
                        project_root,
                        target,
                        str(request_id or ""),
                        revision,
                        artifact_format,
                        payload,
                    )
                    return operation_success(
                        str(cmd),
                        str(request_id or ""),
                        target=target,
                        artifact=public_browser_artifact_metadata(artifact),
                    )
                if cmd == "generate_browser_pdf":
                    if project_root is None:
                        raise BrokerError(
                            "browser_artifact_failure",
                            "project root is unavailable for managed artifacts",
                        )
                    snapshot = await session.get_page_snapshot()
                    revision = str(snapshot.get("page_context_revision") or "")
                    expected = _browser_clean_text(data.get("page_context_revision"))
                    if expected and expected != revision:
                        raise BrokerError(
                            "stale_page_context", "page changed before PDF generation"
                        )
                    scale = float(data.get("scale") or 1.0)
                    if not 0.1 <= scale <= 2.0:
                        raise BrokerError("invalid_browser_operation", "PDF scale is out of range")
                    payload = await session.page.pdf(
                        format=_browser_clean_text(data.get("paper_format")) or "A4",
                        landscape=bool(data.get("landscape")),
                        scale=scale,
                        print_background=bool(data.get("print_background")),
                    )
                    artifact = persist_managed_browser_artifact(
                        project_root,
                        target,
                        str(request_id or ""),
                        revision,
                        "pdf",
                        payload,
                    )
                    return operation_success(
                        str(cmd),
                        str(request_id or ""),
                        target=target,
                        artifact=public_browser_artifact_metadata(artifact),
                    )
                if cmd == "resolve_playwright_locator":
                    snapshot = await session.get_page_snapshot()
                    intent = data.get("intent") if isinstance(data.get("intent"), dict) else {}
                    configured_ids = data.get("test_id_attributes")
                    element, candidates = generate_playwright_candidates(
                        snapshot,
                        intent,
                        configured_ids if isinstance(configured_ids, list) else None,
                    )
                    revision = str(snapshot.get("page_context_revision") or "")
                    observed = await session.verify_playwright_candidates(
                        candidates, revision
                    )
                    if not observed.get("ok"):
                        return {
                            **observed,
                            "schema_version": SCHEMA_VERSION,
                            "operation": str(cmd),
                            "request_id": str(request_id or ""),
                            "target": target,
                        }
                    ranked = apply_verification_results(
                        candidates, observed.get("verification", [])
                    )
                    recommended = next(
                        (
                            candidate
                            for candidate in ranked
                            if candidate.get("verification") == "verified"
                        ),
                        None,
                    )
                    return operation_success(
                        str(cmd),
                        str(request_id or ""),
                        target=target,
                        page_context_revision=revision,
                        url=str(snapshot.get("url") or ""),
                        title=str(snapshot.get("title") or ""),
                        element=element,
                        recommended=recommended,
                        candidates=ranked,
                    )
                if cmd == "verify_playwright_locator":
                    candidate = data.get("candidate")
                    if not isinstance(candidate, dict):
                        raise BrokerError(
                            "invalid_browser_operation", "candidate is required"
                        )
                    revision = str(data.get("page_context_revision") or "")
                    observed = await session.verify_playwright_candidates(
                        [candidate], revision
                    )
                    if not observed.get("ok"):
                        return {
                            **observed,
                            "schema_version": SCHEMA_VERSION,
                            "operation": str(cmd),
                            "request_id": str(request_id or ""),
                            "target": target,
                        }
                    merged = apply_verification_results(
                        [candidate], observed.get("verification", [])
                    )
                    return operation_success(
                        str(cmd),
                        str(request_id or ""),
                        target=target,
                        page_context_revision=revision,
                        candidate=merged[0],
                    )
                if cmd == "execute_browser_action":
                    element = data.get("element")
                    if not isinstance(element, dict):
                        raise BrokerError(
                            "invalid_browser_operation", "element is required"
                        )
                    choices = [
                        bool(_browser_clean_text(element.get("reference"))),
                        isinstance(element.get("candidate"), dict),
                        bool(_browser_clean_text(element.get("css"))),
                    ]
                    if sum(choices) != 1:
                        raise BrokerError(
                            "invalid_browser_operation",
                            "exactly one element input is required",
                        )
                    candidate = (
                        dict(element["candidate"]) if choices[1] else None
                    )
                    selector = _browser_clean_text(element.get("css"))
                    if choices[0]:
                        reference = broker.resolve_element_reference(
                            _record.extension_instance_id,
                            target,
                            _browser_clean_text(element.get("reference")),
                            page_context_revision=_browser_clean_text(
                                element.get("page_context_revision")
                            ),
                            snapshot_id=_browser_clean_text(
                                element.get("snapshot_id")
                            ),
                        )
                        selector = _browser_clean_text(
                            reference.element.get("shortSelector")
                        )
                        raw_candidate = reference.element.get("candidate")
                        candidate = (
                            dict(raw_candidate)
                            if isinstance(raw_candidate, dict)
                            else None
                        )
                    if candidate is not None:
                        observed = await session.verify_playwright_candidates(
                            [candidate],
                            _browser_clean_text(
                                element.get("page_context_revision")
                            ),
                        )
                        first = (observed.get("verification") or [{}])[0]
                        if int(first.get("match_count") or 0) != 1:
                            raise BrokerError(
                                "stale_element_reference",
                                "structured locator is no longer unique",
                            )
                    action_result = await session.execute_locator(
                        selector,
                        str(data.get("action") or ""),
                        data.get("value"),
                        int(data.get("timeout_ms") or 5000),
                        candidate,
                        bool(data.get("focus")),
                    )
                    wait_result = None
                    if action_result.get("ok"):
                        wait_result = await session.wait_for_browser_condition(
                            data.get("wait")
                            if isinstance(data.get("wait"), dict)
                            else None,
                            int(data.get("timeout_ms") or 5000),
                            selector,
                            candidate,
                        )
                    return operation_success(
                        str(cmd),
                        str(request_id or ""),
                        target=target,
                        action_outcome=action_result,
                        wait_outcome=wait_result,
                    )
                revision = str(data.get("page_context_revision") or "")
                current = await session.get_page_snapshot()
                current_revision = str(current.get("page_context_revision") or "")
                if revision and revision != current_revision:
                    raise BrokerError(
                        "stale_page_context",
                        "page changed before screenshot evidence could be captured",
                    )
                screenshot = await session.screenshot_jpeg_b64()
                reference = f"inline:{request_id}"
                if project_root is not None:
                    safe_request = re.sub(
                        r"[^A-Za-z0-9._-]+", "-", str(request_id or "evidence")
                    )[:120]
                    evidence_dir = project_root / ".teshi" / "evidence"
                    evidence_dir.mkdir(parents=True, exist_ok=True)
                    path = evidence_dir / f"{safe_request}.jpg"
                    path.write_bytes(base64.b64decode(screenshot))
                    reference = str(path)
                return operation_success(
                    str(cmd),
                    str(request_id or ""),
                    target=target,
                    evidence={
                        "request_id": str(request_id or ""),
                        "target": target,
                        "media_type": "image/jpeg",
                        "reference": reference,
                        "page_context_revision": current_revision,
                    },
                )
            except BrokerError as exc:
                return exc.response(str(request_id or ""), str(cmd))

    if cmd == "navigate":
        await session.navigate(data.get("url", "about:blank"))
        return {
            "type": "response",
            "request_id": request_id,
            "cmd": "navigate",
            "ok": True,
            "url": session.current_url(),
        }

    if cmd == "highlight_selector":
        result = await session.highlight_selector(data.get("selector", ""))
        return {"type": "response", "request_id": request_id, **result}

    if cmd == "clear_highlight":
        await session.clear_highlight()
        return {"type": "response", "request_id": request_id, "ok": True}

    if cmd == "get_page_snapshot":
        snapshot = await session.get_page_snapshot()
        return {"type": "response", "request_id": request_id, **snapshot}

    if cmd == "open_project":
        result = await session.open_project_via_api(str(data.get("path", "")))
        return {"type": "response", "request_id": request_id, **result}

    if cmd == "execute_locator":
        result = await session.execute_locator(
            str(data.get("selector", "")),
            str(data.get("action", "")),
            data.get("value"),
            int(data.get("timeout_ms", 5000) or 5000),
        )
        return {"type": "response", "request_id": request_id, **result}

    if cmd == "heal_execute_locator":
        result = await session.heal_execute_locator(
            str(data.get("selector", "")),
            str(data.get("action", "")),
            data.get("value"),
            int(data.get("timeout_ms", 5000) or 5000),
        )
        return {"type": "response", "request_id": request_id, **result}

    if cmd == "enhance_locator":
        result = await session.smart_enhance_locator(
            str(data.get("selector", "")),
        )
        return {"type": "response", "request_id": request_id, **result}

    if cmd == "screenshot":
        b64 = await session.screenshot_jpeg_b64()
        return {
            "type": "response",
            "request_id": request_id,
            "ok": True,
            "screenshot": b64,
        }

    return {
        "type": "response",
        "request_id": request_id,
        "ok": False,
        "error": f"unknown cmd: {cmd}",
    }


async def run_embedded(
    host: str,
    port: int,
    cdp_port: int,
    project_root: Path | None,
    *,
    no_preview_stream: bool = False,
) -> None:
    import websockets

    session = EmbeddedSession()
    await session.start(cdp_port)
    agent_broker = BrowserSessionBroker(HEARTBEAT_TTL_SEC)
    cdp_meta: dict[str, Any] = {}
    if project_root is not None:
        try:
            cdp_meta = fetch_playwright_cdp_endpoint(cdp_port)
            write_cdp_endpoint_file(
                project_root,
                mode="embedded",
                ws_url=f"ws://{host}:{port}",
                page_url=session.current_url(),
                cdp_http_url=cdp_meta.get("http_url"),
                extension_connected=False,
            )
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
            print(f"warning: failed to write cdp-endpoint.json: {exc}", file=sys.stderr)

    clients: set[Any] = set()

    async def handler(websocket: Any) -> None:
        clients.add(websocket)
        try:
            async for message in websocket:
                try:
                    data = json.loads(message)
                except json.JSONDecodeError:
                    continue
                if "cmd" in data:
                    request_id = data.get("request_id")
                    debug_log(
                        project_root,
                        "embedded_command_start",
                        {
                            "cmd": data.get("cmd"),
                            "request_id": request_id,
                            "url": data.get("url"),
                        },
                    )
                    try:
                        reply = await handle_embedded_command(
                            session, data, agent_broker, project_root
                        )
                    except Exception as exc:  # noqa: BLE001
                        print(
                            f"embedded command failed: {data.get('cmd')}: {exc}",
                            file=sys.stderr,
                        )
                        reply = {
                            "type": "response",
                            "request_id": request_id,
                            "ok": False,
                            "error": str(exc),
                        }
                    debug_log(
                        project_root,
                        "embedded_command_end",
                        {
                            "cmd": data.get("cmd"),
                            "request_id": request_id,
                            "ok": reply.get("ok"),
                            "error": reply.get("error"),
                        },
                    )
                    await websocket.send(json.dumps(reply))
                    if (
                        project_root is not None
                        and data.get("cmd") == "navigate"
                        and reply.get("ok")
                    ):
                        write_cdp_endpoint_file(
                            project_root,
                            mode="embedded",
                            ws_url=f"ws://{host}:{actual_port}",
                            page_url=session.current_url(),
                            cdp_http_url=cdp_meta.get("http_url"),
                        )
        finally:
            clients.discard(websocket)

    async with websockets.serve(handler, host, port) as server:
        actual_port = server.sockets[0].getsockname()[1]
        print(actual_port, flush=True)
        # Re-write cdp-endpoint.json with the actual WebSocket port so CLI commands
        # (browser doctor / execute / snapshot) can find the sidecar. The initial
        # write before websockets.serve used --port 0, which yields ws://...:0.
        if project_root is not None:
            try:
                cdp_meta = fetch_playwright_cdp_endpoint(cdp_port)
                write_cdp_endpoint_file(
                    project_root,
                    mode="embedded",
                    ws_url=f"ws://{host}:{actual_port}",
                    page_url=session.current_url(),
                    cdp_http_url=cdp_meta.get("http_url"),
                    extension_connected=False,
                )
            except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
                print(f"warning: failed to rewrite cdp-endpoint.json: {exc}", file=sys.stderr)
        if no_preview_stream:
            # CI/CLI mode: avoid concurrent JPEG screencast with locator commands.
            await asyncio.Future()
        while True:
            if clients:
                frame = await session.screenshot_jpeg_b64()
                payload = json.dumps(
                    {
                        "type": "frame",
                        "data": frame,
                        "url": session.current_url(),
                    }
                )
                dead = []
                for ws in clients:
                    try:
                        await ws.send(payload)
                    except Exception:
                        dead.append(ws)
                for ws in dead:
                    clients.discard(ws)
            await asyncio.sleep(0.125)


# --- Chrome extension backend ---


class _LegacyChromeBridge:
    """Chrome mode: extension talks HTTP heartbeat; agents use WebSocket."""

    def __init__(
        self,
        project_root: Path,
        ws_url: str,
        discovery_port: int,
        extension_frame_ws_url: str,
        frame_callback: Any | None = None,
        event_callback: Any | None = None,
        direct_command_callback: Any | None = None,
    ) -> None:
        self.project_root = project_root.resolve()
        self.ws_url = ws_url
        self.discovery_port = discovery_port
        self.extension_frame_ws_url = extension_frame_ws_url
        self.page_url = ""
        self.page_title = ""
        self.active_tab_id: int | None = None
        self.tabs: list[dict[str, Any]] = []
        self.last_frame_error = ""
        self._last_frame: dict[str, Any] | None = None
        self.last_frame_at: float | None = None
        self.last_heartbeat = 0.0
        self._cmd_queue: list[dict[str, Any]] = []
        self._pending: dict[str, asyncio.Future[dict[str, Any]]] = {}
        self._pending_stream_restart = False
        self._frame_callback = frame_callback
        self._event_callback = event_callback
        self._direct_command_callback = direct_command_callback
        self._deprecated_json_frame_warned = False
        self.broker_pid = os.getpid()
        self.broker_start_id = uuid.uuid4().hex

    def extension_alive(self) -> bool:
        return (time.monotonic() - self.last_heartbeat) < HEARTBEAT_TTL_SEC

    def bridge_info(self) -> dict[str, Any]:
        last_frame_age_ms: int | None = None
        if self.last_frame_at is not None:
            last_frame_age_ms = int((time.monotonic() - self.last_frame_at) * 1000)
        return {
            "ws_url": self.ws_url,
            "extension_frame_ws_url": self.extension_frame_ws_url,
            "project_root": str(self.project_root),
            "mode": "chrome",
            "broker_scope": "user_session",
            "transport": "http-heartbeat+ws-screencast",
            "command_transport": "direct-ws+heartbeat-fallback",
            "extension_connected": self.extension_alive(),
            "page_url": self.page_url,
            "title": self.page_title,
            "active_tab_id": self.active_tab_id,
            "tabs": self.tabs,
            "last_frame_error": self.last_frame_error,
            "last_frame_age_ms": last_frame_age_ms,
        }

    def write_endpoint(self) -> None:
        write_cdp_endpoint_file(
            self.project_root,
            mode="chrome",
            ws_url=self.ws_url,
            page_url=self.page_url or "about:blank",
            discovery_port=self.discovery_port,
            extension_connected=self.extension_alive(),
            extension_frame_ws_url=self.extension_frame_ws_url,
        )

    def debug_log(self, event: str, payload: dict[str, Any]) -> None:
        debug_log(self.project_root, event, payload)

    async def handle_heartbeat(self, payload: dict[str, Any]) -> dict[str, Any]:
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return {
                "ok": False,
                "error": f"project_root mismatch: expected {self.project_root}",
            }
        self.last_heartbeat = time.monotonic()
        self.page_url = str(payload.get("url", self.page_url))
        self.page_title = str(payload.get("title", self.page_title))
        raw_active = payload.get("active_tab_id")
        if raw_active is not None:
            try:
                self.active_tab_id = int(raw_active)
            except (TypeError, ValueError):
                pass
        raw_tabs = payload.get("tabs")
        if isinstance(raw_tabs, list):
            self.tabs = raw_tabs
        frame_error = payload.get("frame_error")
        if isinstance(frame_error, str) and frame_error.strip():
            self.last_frame_error = frame_error.strip()
        self.write_endpoint()
        pending_cmd = self._cmd_queue.pop(0) if self._cmd_queue else None
        stream_restart = self._pending_stream_restart
        self._pending_stream_restart = False
        return {
            "ok": True,
            "cmd": pending_cmd,
            "stream_restart": stream_restart,
            # Legacy alias for older extension builds.
            "force_capture": stream_restart,
        }

    def _apply_frame_state(self, frame_out: dict[str, Any]) -> None:
        """Update bridge metadata and cache the latest frame (sync, HTTP-fast)."""
        if frame_out.get("url"):
            self.page_url = str(frame_out["url"])
        raw_tab = frame_out.get("tab_id")
        if raw_tab is not None:
            try:
                self.active_tab_id = int(raw_tab)
            except (TypeError, ValueError):
                pass
        self._last_frame = frame_out
        self.last_frame_at = time.monotonic()
        self.last_frame_error = ""
        self.write_endpoint()

    async def _emit_frame(self, frame_out: dict[str, Any]) -> None:
        self._apply_frame_state(frame_out)
        if self._frame_callback is not None:
            await self._frame_callback(frame_out)

    def _schedule_frame_broadcast(self, frame_out: dict[str, Any]) -> None:
        """Push frames to desktop WebSocket clients without blocking HTTP /response."""
        if self._frame_callback is None:
            return
        asyncio.create_task(self._frame_callback(frame_out))

    async def _emit_stream_event(self, event: dict[str, Any]) -> None:
        if self._event_callback is not None:
            await self._event_callback(event)

    def _schedule_stream_event(self, event: dict[str, Any]) -> None:
        if self._event_callback is None:
            return
        asyncio.create_task(self._emit_stream_event(event))

    def validate_stream_hello(self, payload: dict[str, Any]) -> dict[str, Any]:
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return {
                "type": "stream_hello_ack",
                "ok": False,
                "error": f"project_root mismatch: expected {self.project_root}",
            }
        self.last_heartbeat = time.monotonic()
        return {"type": "stream_hello_ack", "ok": True}

    async def handle_extension_binary(self, data: bytes) -> None:
        parsed = parse_tsh1_frame(data)
        if parsed is None:
            return
        meta, jpeg = parsed
        if not jpeg:
            return
        frame_out = await asyncio.to_thread(build_frame_out_sync, meta, jpeg)
        self.last_heartbeat = time.monotonic()
        self._apply_frame_state(frame_out)
        self._schedule_frame_broadcast(frame_out)

    async def handle_extension_response(self, payload: dict[str, Any]) -> dict[str, Any]:
        if payload.get("type") == "frame_error":
            self.last_frame_error = str(payload.get("error", "screenshot failed"))
            self.write_endpoint()
            self._schedule_stream_event(
                {
                    "type": "frame_error",
                    "error": self.last_frame_error,
                }
            )
            return {"ok": True}

        if payload.get("type") == "frame":
            if not self._deprecated_json_frame_warned:
                self._deprecated_json_frame_warned = True
                print(
                    "warning: JSON frame on POST /v1/bridge/response is deprecated; "
                    "use extension WebSocket screencast (/extension/frames)",
                    file=sys.stderr,
                )
            data_field = payload.get("data", "")
            if isinstance(data_field, str) and len(data_field) > 4096:
                return {"ok": True, "deprecated": True, "ignored": True}
            self.last_heartbeat = time.monotonic()
            self.page_title = str(payload.get("title", self.page_title))
            frame_out = {
                "type": "frame",
                "data": data_field,
                "url": str(payload.get("url", self.page_url)),
            }
            raw_tab = payload.get("tab_id")
            if raw_tab is not None:
                try:
                    frame_out["tab_id"] = int(raw_tab)
                except (TypeError, ValueError):
                    pass
            self._apply_frame_state(frame_out)
            self._schedule_frame_broadcast(frame_out)
            return {"ok": True}

        request_id = payload.get("request_id")
        if request_id:
            fut = self._pending.pop(str(request_id), None)
            if fut and not fut.done():
                fut.set_result(payload)
            if payload.get("cmd") in {
                "get_page_snapshot",
                "navigate",
                "activate_tab",
            } and payload.get("ok"):
                self.page_url = str(payload.get("url", self.page_url))
                self.page_title = str(payload.get("title", self.page_title))
                self.write_endpoint()
            self.debug_log(
                "extension_response",
                {
                    "cmd": payload.get("cmd"),
                    "request_id": request_id,
                    "ok": payload.get("ok"),
                    "url": payload.get("url"),
                    "error": payload.get("error"),
                },
            )
        return {"ok": True}

    def queue_command_front(self, cmd: str, **fields: Any) -> str:
        """Enqueue a command for the next extension heartbeat (front of queue)."""
        request_id = str(fields.pop("request_id", f"cmd-{time.monotonic()}"))
        entry: dict[str, Any] = {
            "type": "cmd",
            "request_id": request_id,
            "cmd": cmd,
        }
        entry.update(fields)
        self._cmd_queue.insert(0, entry)
        return request_id

    async def handle_activate_tab_http(self, payload: dict[str, Any]) -> dict[str, Any]:
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return {
                "ok": False,
                "error": f"project_root mismatch: expected {self.project_root}",
            }
        raw_tab = payload.get("tab_id")
        if raw_tab is None:
            return {"ok": False, "error": "tab_id is required"}
        try:
            tab_id = int(raw_tab)
        except (TypeError, ValueError):
            return {"ok": False, "error": "tab_id must be an integer"}
        self.active_tab_id = tab_id
        self.write_endpoint()
        self.queue_command_front("activate_tab", tab_id=tab_id)
        self._pending_stream_restart = True
        return {"ok": True}

    async def handle_capture_now_http(self, payload: dict[str, Any]) -> dict[str, Any]:
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return {
                "ok": False,
                "error": f"project_root mismatch: expected {self.project_root}",
            }
        self._pending_stream_restart = True
        return {"ok": True}

    async def forward_command(self, data: dict[str, Any]) -> dict[str, Any]:
        request_id = str(data.get("request_id") or "")
        started = time.monotonic()
        self.debug_log(
            "chrome_command_start",
            {
                "cmd": data.get("cmd"),
                "request_id": request_id,
                "selector": data.get("selector"),
                "url": data.get("url"),
            },
        )
        if not self.extension_alive():
            self.debug_log(
                "chrome_command_error",
                {
                    "cmd": data.get("cmd"),
                    "request_id": request_id,
                    "error": "extension_not_connected",
                },
            )
            return {
                "type": "response",
                "request_id": request_id,
                "ok": False,
                "error": (
                    "Chrome extension not connected (no heartbeat). Keep the "
                    "target application tab active in Chrome and ensure "
                    "teshi-bridge is loaded — it polls every second while the bridge runs."
                ),
            }

        loop = asyncio.get_running_loop()
        fut: asyncio.Future[dict[str, Any]] = loop.create_future()
        self._pending[request_id] = fut
        queued: dict[str, Any] = {
            "type": "cmd",
            "request_id": request_id,
            "cmd": data.get("cmd"),
            "selector": data.get("selector"),
            "url": data.get("url"),
        }
        for field in ("action", "value", "timeout_ms"):
            if data.get(field) is not None:
                queued[field] = data.get(field)
        if data.get("tab_id") is not None:
            queued["tab_id"] = data.get("tab_id")
        cmd_name = str(data.get("cmd") or "")
        if cmd_name in ("get_page_snapshot", "highlight_selector"):
            self._cmd_queue.insert(0, queued)
        else:
            self._cmd_queue.append(queued)
        try:
            result = await asyncio.wait_for(fut, timeout=45.0)
            self.debug_log(
                "chrome_command_end",
                {
                    "cmd": data.get("cmd"),
                    "request_id": request_id,
                    "elapsed_ms": int((time.monotonic() - started) * 1000),
                    "ok": result.get("ok"),
                    "error": result.get("error"),
                },
            )
            return result
        except asyncio.TimeoutError:
            self._pending.pop(request_id, None)
            self._cmd_queue = [c for c in self._cmd_queue if c.get("request_id") != request_id]
            self.debug_log(
                "chrome_command_timeout",
                {
                    "cmd": data.get("cmd"),
                    "request_id": request_id,
                    "elapsed_ms": int((time.monotonic() - started) * 1000),
                },
            )
            return {
                "type": "response",
                "request_id": request_id,
                "ok": False,
                "error": "extension did not respond in time (heartbeat may have stalled)",
            }


class ChromeBridge:
    """Chrome mode multi-session broker with a legacy single-session projection."""

    def __init__(
        self,
        project_root: Path,
        ws_url: str,
        discovery_port: int,
        extension_frame_ws_url: str,
        frame_callback: Any | None = None,
        event_callback: Any | None = None,
        direct_command_callback: Any | None = None,
    ) -> None:
        self.project_root = project_root.resolve()
        self.ws_url = ws_url
        self.discovery_port = discovery_port
        self.extension_frame_ws_url = extension_frame_ws_url
        self.broker = BrowserSessionBroker(HEARTBEAT_TTL_SEC)
        self._frame_callback = frame_callback
        self._event_callback = event_callback
        self._direct_command_callback = direct_command_callback
        self._last_frame: dict[str, Any] | None = None
        self._stream_restart_instances: set[str] = set()
        self._deprecated_json_frame_warned = False
        self.broker_pid = os.getpid()
        self.broker_start_id = uuid.uuid4().hex

    @property
    def last_frame_error(self) -> str:
        """Return the sole live session's frame error for legacy UI consumers."""
        info = self.broker.bridge_info()
        return str(info.get("last_frame_error", ""))

    @last_frame_error.setter
    def last_frame_error(self, value: str) -> None:
        """Apply a stream error only when one session can be selected safely."""
        try:
            record, _target, _explicit = self.broker.resolve_target(None)
        except BrokerError:
            return
        record.last_frame_error = str(value)[:1000]

    def extension_alive(self) -> bool:
        """Return whether at least one compatible extension session is live."""
        return bool(
            self.broker.bridge_info().get("extension_connected")
        )

    def bridge_info(self) -> dict[str, Any]:
        """Return versioned discovery plus the legacy single-session fields."""
        return {
            "ws_url": self.ws_url,
            "extension_frame_ws_url": self.extension_frame_ws_url,
            "project_root": str(self.project_root),
            "mode": "chrome",
            "transport": "http-heartbeat+ws-screencast",
            "command_transport": "direct-ws+heartbeat-fallback",
            "broker_pid": self.broker_pid,
            "broker_scope": "user_session",
            "broker_start_id": self.broker_start_id,
            "broker_features": ["p0.control"],
            **self.broker.bridge_info(),
        }

    def write_endpoint(self) -> None:
        """Refresh legacy `.teshi/cdp-endpoint.json` discovery metadata."""
        info = self.bridge_info()
        write_cdp_endpoint_file(
            self.project_root,
            mode="chrome",
            ws_url=self.ws_url,
            page_url=str(info.get("page_url") or "about:blank"),
            discovery_port=self.discovery_port,
            extension_connected=bool(info.get("extension_connected")),
            extension_frame_ws_url=self.extension_frame_ws_url,
            broker_pid=self.broker_pid,
            broker_start_id=self.broker_start_id,
        )

    def debug_log(self, event: str, payload: dict[str, Any]) -> None:
        """Persist opt-in broker diagnostics without page payloads or secrets."""
        debug_log(self.project_root, event, payload)

    def request_project_root(self, data: dict[str, Any]) -> Path:
        """Resolve the caller's project context instead of reusing broker startup state."""
        raw = _browser_clean_text(data.get("project_root"))
        candidate = Path(raw) if raw else self.project_root
        try:
            resolved = candidate.resolve(strict=True)
        except (OSError, RuntimeError) as exc:
            raise BrokerError(
                "invalid_browser_operation",
                "request project root is missing or inaccessible",
            ) from exc
        if not resolved.is_dir():
            raise BrokerError(
                "invalid_browser_operation", "request project root is not a directory"
            )
        return resolved

    async def handle_heartbeat(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Register one extension instance and return only its queued command."""
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return BrokerError(
                "invalid_browser_operation",
                f"project_root mismatch: expected {self.project_root}",
            ).response()
        record = self.broker.register_heartbeat(payload)
        response = self.broker.heartbeat_response(record)
        restart = record.extension_instance_id in self._stream_restart_instances
        self._stream_restart_instances.discard(record.extension_instance_id)
        response["stream_restart"] = restart
        response["force_capture"] = restart
        if not record.compatible():
            response.update(
                {
                    "ok": False,
                    "code": "incompatible_browser_session",
                    "error": (
                        f"extension protocol {record.protocol_version} is incompatible; "
                        f"protocol {PROTOCOL_VERSION} is required"
                    ),
                }
            )
        self.write_endpoint()
        return response

    def validate_stream_hello(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Authenticate a preview stream to one registered extension instance."""
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return {
                "type": "stream_hello_ack",
                "ok": False,
                "code": "invalid_browser_operation",
                "error": f"project_root mismatch: expected {self.project_root}",
            }
        instance_id = str(payload.get("extension_instance_id", "")).strip()
        if not instance_id:
            # Legacy preview is accepted only when its heartbeat is the sole session.
            try:
                record, _target, _explicit = self.broker.resolve_target(None)
                instance_id = record.extension_instance_id
            except BrokerError as exc:
                return {
                    "type": "stream_hello_ack",
                    "ok": False,
                    "code": exc.code,
                    "error": exc.message,
                }
        try:
            record = self.broker.require_session(instance_id)
        except BrokerError as exc:
            return {
                "type": "stream_hello_ack",
                "ok": False,
                "code": exc.code,
                "error": exc.message,
            }
        return {
            "type": "stream_hello_ack",
            "ok": True,
            "schema_version": SCHEMA_VERSION,
            "protocol_version": PROTOCOL_VERSION,
            "extension_instance_id": record.extension_instance_id,
        }

    async def handle_extension_binary(
        self,
        data: bytes,
        stream_instance_id: str,
    ) -> None:
        """Route one binary preview frame to its authenticated session."""
        parsed = parse_tsh1_frame(data)
        if parsed is None:
            return
        meta, jpeg = parsed
        if not jpeg:
            return
        meta_instance = str(meta.get("extension_instance_id", "")).strip()
        if meta_instance and meta_instance != stream_instance_id:
            self.debug_log(
                "frame_target_mismatch",
                {
                    "stream_instance_id": stream_instance_id,
                    "frame_instance_id": meta_instance,
                },
            )
            return
        meta["extension_instance_id"] = stream_instance_id
        frame_out = await asyncio.to_thread(build_frame_out_sync, meta, jpeg)
        target = frame_out.get("target")
        if not isinstance(target, dict):
            try:
                record = self.broker.require_session(stream_instance_id)
                target = record.active_target()
            except BrokerError:
                return
        if not isinstance(target, dict):
            return
        try:
            self.broker.update_frame(stream_instance_id, target, frame_out)
        except BrokerError as exc:
            self.debug_log("frame_rejected", {"code": exc.code})
            return
        self._last_frame = frame_out
        self.write_endpoint()
        if self._frame_callback is not None:
            await self._frame_callback(frame_out)

    async def handle_extension_response(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Correlate a JSON response without allowing cross-session delivery."""
        instance_id = str(payload.get("extension_instance_id", "")).strip()
        if payload.get("type") == "console_event":
            target = payload.get("target")
            accepted = self.broker.record_console_event(
                instance_id,
                target if isinstance(target, dict) else {},
                payload.get("event"),
            )
            return {
                "ok": True,
                "schema_version": SCHEMA_VERSION,
                "accepted": accepted,
            }
        if payload.get("type") == "network_event":
            target = payload.get("target")
            accepted = self.broker.record_network_event(
                instance_id,
                target if isinstance(target, dict) else {},
                payload.get("event"),
            )
            return {
                "ok": True,
                "schema_version": SCHEMA_VERSION,
                "accepted": accepted,
            }
        if payload.get("type") == "frame_error":
            if not instance_id:
                try:
                    record, _target, _explicit = self.broker.resolve_target(None)
                    instance_id = record.extension_instance_id
                except BrokerError as exc:
                    return exc.response()
            record = self.broker.sessions.get(instance_id)
            if record is None:
                return BrokerError(
                    "browser_target_not_found",
                    f"browser session not found: {instance_id}",
                ).response()
            record.last_frame_error = str(
                payload.get("error", "screenshot failed")
            )[:1000]
            self.write_endpoint()
            await self._emit_event(
                {
                    "type": "frame_error",
                    "extension_instance_id": instance_id,
                    "error": record.last_frame_error,
                }
            )
            return {"ok": True, "schema_version": SCHEMA_VERSION}

        if payload.get("type") == "frame":
            if not self._deprecated_json_frame_warned:
                self._deprecated_json_frame_warned = True
                print(
                    "warning: JSON frame on POST /v1/bridge/response is deprecated; "
                    "use extension WebSocket screencast (/extension/frames)",
                    file=sys.stderr,
                )
            return {"ok": True, "deprecated": True, "ignored": True}

        try:
            pending = self.broker.accept_response(payload)
        except BrokerError as exc:
            self.debug_log(
                "extension_response_rejected",
                {
                    "request_id": payload.get("request_id"),
                    "code": exc.code,
                },
            )
            return exc.response(str(payload.get("request_id", "")))
        if pending is not None:
            record = self.broker.sessions.get(pending.extension_instance_id)
            if record is not None and payload.get("ok"):
                if payload.get("url"):
                    record.page_url = str(payload["url"])
                if payload.get("title"):
                    record.page_title = str(payload["title"])
                target = pending.target
                record.active_window_id = int(target["window_id"])
                record.active_tab_id = int(target["tab_id"])
            self.debug_log(
                "extension_response",
                {
                    "cmd": pending.operation,
                    "request_id": pending.request_id,
                    "extension_instance_id": pending.extension_instance_id,
                    "ok": payload.get("ok"),
                    "code": payload.get("code"),
                },
            )
        self.write_endpoint()
        return {"ok": True, "schema_version": SCHEMA_VERSION}

    async def handle_activate_tab_http(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Queue legacy UI tab activation with explicit session disambiguation."""
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return BrokerError(
                "invalid_browser_operation",
                f"project_root mismatch: expected {self.project_root}",
            ).response()
        instance_id = str(
            payload.get("extension_instance_id") or payload.get("session") or ""
        ).strip()
        tab_id = payload.get("tab_id")
        window_id = payload.get("window_id")
        raw_target: dict[str, Any] | None = None
        if instance_id:
            record = self.broker.sessions.get(instance_id)
            if record is None:
                return BrokerError(
                    "browser_target_not_found",
                    f"browser session not found: {instance_id}",
                ).response()
            if window_id is None:
                matching = [
                    tab
                    for tab in record.iter_tabs()
                    if str(tab.get("id")) == str(tab_id)
                ]
                if len(matching) == 1:
                    window_id = matching[0].get("window_id")
            raw_target = {
                "extension_instance_id": instance_id,
                "window_id": window_id,
                "tab_id": tab_id,
            }
        data = {
            "cmd": "activate_tab",
            "request_id": f"ui-activate-{time.monotonic_ns()}",
            "target": raw_target,
            "tab_id": tab_id,
        }
        ui_ephemeral_token: str | None = None
        try:
            if raw_target is not None:
                # The local browser panel is a compatibility UI, not an external
                # lease owner. Give its explicit session selection a short lease
                # so it cannot race an agent that already owns the profile.
                record, normalized_target, _explicit = self.broker.resolve_target(
                    raw_target
                )
                lease = self.broker.acquire_lease(
                    record.extension_instance_id,
                    "teshi-browser-panel",
                    15,
                )
                ui_ephemeral_token = str(lease["lease_token"])
                data["target"] = normalized_target
                data["lease_token"] = ui_ephemeral_token
            record, target, ephemeral = self.broker.authorize_command(
                data, legacy_compatibility=True
            )
            data["tab_id"] = target["tab_id"]
            data["window_id"] = target["window_id"]
            loop = asyncio.get_running_loop()
            future: asyncio.Future[dict[str, Any]] = loop.create_future()
            self.broker.queue_command(
                record,
                target,
                data,
                future,
                ephemeral_lease_token=ephemeral or ui_ephemeral_token,
                front=True,
            )
            async def expire_ui_activation() -> None:
                try:
                    await asyncio.wait_for(asyncio.shield(future), timeout=15)
                except asyncio.TimeoutError:
                    self.broker.cancel_request(
                        str(data["request_id"]),
                        BrokerError(
                            "browser_operation_timeout",
                            "browser tab activation timed out; retry after checking extension health",
                        ),
                    )

            asyncio.create_task(expire_ui_activation())
            self._stream_restart_instances.add(record.extension_instance_id)
            return operation_success(
                "activate_tab",
                str(data["request_id"]),
                target=target,
                queued=True,
            )
        except BrokerError as exc:
            if ui_ephemeral_token and instance_id:
                try:
                    self.broker.release_lease(instance_id, ui_ephemeral_token)
                except BrokerError:
                    pass
            return exc.response(str(data["request_id"]), "activate_tab")

    async def handle_capture_now_http(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Request preview restart for one explicit or unambiguous session."""
        got = str(payload.get("project_root", ""))
        if not paths_equal(got, self.project_root):
            return BrokerError(
                "invalid_browser_operation",
                f"project_root mismatch: expected {self.project_root}",
            ).response()
        instance_id = str(
            payload.get("extension_instance_id") or payload.get("session") or ""
        ).strip()
        try:
            if instance_id:
                record = self.broker.require_session(instance_id)
            else:
                record, _target, _explicit = self.broker.resolve_target(None)
            self._stream_restart_instances.add(record.extension_instance_id)
            return operation_success(
                "capture_now",
                f"capture-{time.monotonic_ns()}",
                extension_instance_id=record.extension_instance_id,
                queued=True,
            )
        except BrokerError as exc:
            return exc.response(operation="capture_now")

    async def forward_command(self, data: dict[str, Any]) -> dict[str, Any]:
        """Execute one local broker or targeted extension operation."""
        operation = str(data.get("cmd") or "")
        request_id = str(data.get("request_id") or f"browser-{time.monotonic_ns()}")
        data = dict(data)
        data["request_id"] = request_id
        started = time.monotonic()
        self.debug_log(
            "chrome_command_start",
            {"cmd": operation, "request_id": request_id},
        )
        try:
            if operation == "list_browser_sessions":
                return operation_success(
                    operation,
                    request_id,
                    sessions=self.broker.list_sessions(),
                )
            if operation == "cleanup_browser_artifacts":
                cleaned = cleanup_managed_browser_artifacts(
                    self.request_project_root(data),
                    data.get("paths") if isinstance(data.get("paths"), list) else [],
                )
                return operation_success(operation, request_id, **cleaned)
            if operation == "list_browser_tabs":
                result = self.broker.list_tabs(
                    str(data.get("extension_instance_id") or "")
                )
                return operation_success(operation, request_id, **result)
            if operation == "lookup_browser_sessions":
                matches = self.broker.lookup_sessions(
                    extension_instance_id=_browser_clean_text(
                        data.get("extension_instance_id")
                    ),
                    profile_label=_browser_clean_text(data.get("profile_label")),
                    browser_name=_browser_clean_text(data.get("browser_name")),
                    tab_id=(
                        int(data["tab_id"])
                        if data.get("tab_id") is not None
                        else None
                    ),
                )
                if data.get("tab_id") is not None and len(matches) > 1:
                    raise BrokerError(
                        "ambiguous_browser_target",
                        "tab_id exists in more than one browser Profile; add session or label",
                        {"match_count": len(matches)},
                    )
                return operation_success(operation, request_id, sessions=matches)
            if operation == "set_browser_profile_label":
                label = self.broker.set_profile_label(
                    _browser_clean_text(data.get("extension_instance_id")),
                    _browser_clean_text(data.get("profile_label")),
                )
                return operation_success(operation, request_id, profile_label=label)
            if operation == "clear_browser_profile_label":
                self.broker.clear_profile_label(
                    _browser_clean_text(data.get("extension_instance_id"))
                )
                return operation_success(operation, request_id, profile_label=None)
            if operation == "acquire_browser_lease":
                lease = self.broker.acquire_lease(
                    str(data.get("extension_instance_id") or ""),
                    str(data.get("owner_label") or "external-agent"),
                    data.get("ttl_secs"),
                )
                return operation_success(operation, request_id, lease=lease)
            if operation == "renew_browser_lease":
                lease = self.broker.renew_lease(
                    str(data.get("extension_instance_id") or ""),
                    str(data.get("lease_token") or ""),
                    data.get("ttl_secs"),
                )
                return operation_success(operation, request_id, lease=lease)
            if operation == "release_browser_lease":
                result = self.broker.release_lease(
                    str(data.get("extension_instance_id") or ""),
                    str(data.get("lease_token") or ""),
                )
                return operation_success(operation, request_id, **result)
            if operation == "create_browser_capability_grant":
                record, target, _explicit = self.broker.resolve_target(data.get("target"))
                project_root = self.request_project_root(data)
                grant = self.broker.create_capability_grant(
                    extension_instance_id=record.extension_instance_id,
                    lease_token=str(data.get("lease_token") or ""),
                    capability=data.get("capability"),
                    project_root=project_root,
                    caller_label=data.get("caller_label"),
                    ttl_secs=data.get("ttl_secs"),
                    interactive_confirmed=bool(data.get("interactive_confirmed")),
                    non_interactive=bool(data.get("non_interactive")),
                    acknowledged_capability=data.get("acknowledged_capability"),
                    policy_capabilities=load_browser_privileged_policy(project_root),
                )
                self.broker.append_privileged_audit(
                    capability=grant["capability"],
                    caller_label=data.get("caller_label"),
                    target=target,
                    request_id=request_id,
                    outcome="granted",
                    arguments={"ttl_secs": data.get("ttl_secs"), "non_interactive": bool(data.get("non_interactive"))},
                )
                return operation_success(operation, request_id, grant=grant)
            if operation == "list_browser_capability_grants":
                grants = self.broker.list_capability_grants(
                    project_root=self.request_project_root(data),
                    extension_instance_id=data.get("extension_instance_id"),
                )
                return operation_success(operation, request_id, grants=grants)
            if operation == "revoke_browser_capability_grant":
                revoked = self.broker.revoke_capability_grant(
                    data.get("grant_id"),
                    project_root=self.request_project_root(data),
                )
                self.broker.append_privileged_audit(
                    capability="grant-revocation",
                    caller_label=data.get("caller_label"),
                    target=None,
                    request_id=request_id,
                    outcome="revoked",
                    arguments={"grant_id": revoked["grant_id"]},
                )
                return operation_success(operation, request_id, **revoked)
            if operation == "expire_browser_capability_grants":
                before = len(self.broker.capability_grants)
                self.broker.expire_capability_grants()
                return operation_success(
                    operation,
                    request_id,
                    expired=before - len(self.broker.capability_grants),
                )
            if operation == "list_browser_privileged_audit":
                return operation_success(
                    operation,
                    request_id,
                    records=self.broker.list_privileged_audit(data.get("limit")),
                )
            if operation == "start_console_capture":
                authorized = self.broker.authorize_command(
                    data, legacy_compatibility=False
                )
                record, target, _ephemeral = authorized
                capture = self.broker.start_console_capture(
                    record,
                    target,
                    levels=data.get("levels"),
                    max_age_ms=data.get("max_age_ms"),
                    max_entries=data.get("max_entries"),
                    max_bytes=data.get("max_bytes"),
                    sensitive_fields=data.get("sensitive_fields"),
                )
                result = await self._forward_extension_command(
                    data, authorized=authorized
                )
                if not result.get("ok"):
                    self.broker.stop_console_capture(record, target)
                    return result
                return operation_success(operation, request_id, capture=capture)
            if operation == "list_console_events":
                record, target, _ephemeral = self.broker.authorize_command(
                    data, legacy_compatibility=False
                )
                result = self.broker.list_console_events(
                    record,
                    target,
                    levels=data.get("levels"),
                    max_age_ms=data.get("max_age_ms"),
                    max_entries=data.get("max_entries"),
                    max_bytes=data.get("max_bytes"),
                )
                return operation_success(operation, request_id, **result)
            if operation == "clear_console_capture":
                record, target, _ephemeral = self.broker.authorize_command(
                    data, legacy_compatibility=False
                )
                result = self.broker.clear_console_capture(record, target)
                return operation_success(operation, request_id, **result)
            if operation == "stop_console_capture":
                authorized = self.broker.authorize_command(
                    data, legacy_compatibility=False
                )
                record, target, _ephemeral = authorized
                result = await self._forward_extension_command(
                    data, authorized=authorized
                )
                if not result.get("ok"):
                    return result
                stopped = self.broker.stop_console_capture(record, target)
                return operation_success(operation, request_id, **stopped)
            if operation == "start_network_capture":
                authorized = self.broker.authorize_command(
                    data, legacy_compatibility=False
                )
                record, target, _ephemeral = authorized
                capture = self.broker.start_network_capture(
                    record,
                    target,
                    max_age_ms=data.get("max_age_ms"),
                    max_entries=data.get("max_entries"),
                    max_bytes=data.get("max_bytes"),
                    max_body_bytes=data.get("max_body_bytes"),
                    sensitive_fields=data.get("sensitive_fields"),
                )
                result = await self._forward_extension_command(
                    data, authorized=authorized
                )
                if not result.get("ok"):
                    self.broker.stop_network_capture(record, target)
                    return result
                return operation_success(operation, request_id, capture=capture)
            if operation == "list_network_requests":
                record, target, _ephemeral = self.broker.authorize_command(
                    data, legacy_compatibility=False
                )
                result = self.broker.list_network_requests(
                    record,
                    target,
                    max_age_ms=data.get("max_age_ms"),
                    max_entries=data.get("max_entries"),
                    max_bytes=data.get("max_bytes"),
                )
                return operation_success(operation, request_id, **result)
            if operation == "get_network_request_detail":
                authorized = self.broker.authorize_command(
                    data, legacy_compatibility=False
                )
                record, target, _ephemeral = authorized
                if not data.get("include_body"):
                    result = self.broker.get_network_request_detail(
                        record, target, data.get("network_request_id")
                    )
                    return operation_success(operation, request_id, **result)
                extension_request = dict(data)
                extension_request["cmd"] = "get_network_response_body"
                result = await self._forward_extension_command(
                    extension_request, authorized=authorized
                )
                if not result.get("ok"):
                    return {
                        **result,
                        "request_id": request_id,
                        "operation": operation,
                    }
                bounded = self.broker.bound_network_body(
                    record,
                    target,
                    data.get("network_request_id"),
                    result.get("body"),
                    result.get("base64_encoded"),
                    data.get("max_body_bytes"),
                )
                return operation_success(operation, request_id, **bounded)
            if operation == "clear_network_capture":
                record, target, _ephemeral = self.broker.authorize_command(
                    data, legacy_compatibility=False
                )
                result = self.broker.clear_network_capture(record, target)
                return operation_success(operation, request_id, **result)
            if operation == "stop_network_capture":
                authorized = self.broker.authorize_command(
                    data, legacy_compatibility=False
                )
                record, target, _ephemeral = authorized
                result = await self._forward_extension_command(
                    data, authorized=authorized
                )
                if not result.get("ok"):
                    return result
                stopped = self.broker.stop_network_capture(record, target)
                return operation_success(operation, request_id, **stopped)
            if operation == "execute_privileged_javascript":
                return await self._execute_privileged_javascript(data)
            if operation == "execute_privileged_cdp":
                return await self._execute_privileged_cdp(data)
            if operation == "list_browser_cookies":
                return await self._list_browser_cookies(data)
            if operation == "access_browser_content_setting":
                return await self._access_browser_content_setting(data)
            if operation == "list_browser_extensions":
                return await self._list_browser_extensions(data)
            if operation == "resolve_playwright_locator":
                return await self._resolve_playwright_locator(data)
            if operation == "verify_playwright_locator":
                return await self._verify_playwright_locator(data)
            if operation == "execute_browser_action":
                return await self._execute_browser_action(data)
            if operation == "capture_browser_evidence":
                return await self._capture_browser_evidence(data)
            if operation == "capture_browser_screenshot":
                return await self._capture_browser_screenshot(data)
            if operation == "generate_browser_pdf":
                return await self._generate_browser_pdf(data)
            return await self._forward_extension_command(data)
        except BrokerError as exc:
            response = exc.response(request_id, operation)
            self.debug_log(
                "chrome_command_error",
                {"cmd": operation, "request_id": request_id, "code": exc.code},
            )
            return response
        finally:
            self.debug_log(
                "chrome_command_end",
                {
                    "cmd": operation,
                    "request_id": request_id,
                    "elapsed_ms": int((time.monotonic() - started) * 1000),
                },
            )

    async def _forward_extension_command(
        self,
        data: dict[str, Any],
        *,
        authorized: tuple[Any, dict[str, Any], str | None] | None = None,
        timeout: float = 45.0,
    ) -> dict[str, Any]:
        if authorized is None:
            authorized = self.broker.authorize_command(data)
        record, target, ephemeral = authorized
        if not record.compatible():
            raise BrokerError(
                "incompatible_browser_session",
                "selected extension protocol is incompatible",
            )
        loop = asyncio.get_running_loop()
        future: asyncio.Future[dict[str, Any]] = loop.create_future()
        command = dict(data)
        command["target"] = target
        self.broker.queue_command(
            record,
            target,
            command,
            future,
            ephemeral_lease_token=ephemeral,
            front=str(data.get("cmd")) in {"get_page_snapshot", "highlight_selector"},
        )
        request_id = str(data["request_id"])
        if self._direct_command_callback is not None:
            direct_command = self.broker.take_queued_command(
                record.extension_instance_id, request_id
            )
            if direct_command is not None:
                sent = False
                try:
                    sent = bool(
                        await self._direct_command_callback(
                            record.extension_instance_id, direct_command
                        )
                    )
                except Exception:  # noqa: BLE001
                    sent = False
                if not sent:
                    self.broker.restore_queued_command(
                        record.extension_instance_id, direct_command
                    )
        try:
            result = await asyncio.wait_for(future, timeout=timeout)
            return result
        except asyncio.TimeoutError as exc:
            error = BrokerError(
                "browser_operation_timeout",
                "extension did not respond before the operation timeout",
                {"extension_instance_id": record.extension_instance_id},
            )
            self.broker.cancel_request(request_id, error)
            raise error from exc

    def _authorize_privileged(
        self, data: dict[str, Any], capability: str
    ) -> tuple[tuple[Any, dict[str, Any], str | None], Path]:
        authorized = self.broker.authorize_command(data, legacy_compatibility=False)
        record, target, _ephemeral = authorized
        project_root = self.request_project_root(data)
        self.broker.validate_capability_grant(
            token=data.get("capability_grant_token"),
            capability=capability,
            extension_instance_id=record.extension_instance_id,
            project_root=project_root,
            caller_label=data.get("caller_label"),
        )
        return authorized, project_root

    async def _execute_privileged_javascript(
        self, data: dict[str, Any]
    ) -> dict[str, Any]:
        request_id = str(data["request_id"])
        target = data.get("target") if isinstance(data.get("target"), dict) else None
        expression = str(data.get("expression") or "")
        source_bytes = len(expression.encode("utf-8"))
        arguments = {
            "source": str(data.get("source_kind") or "inline")[:20],
            "source_bytes": source_bytes,
        }
        try:
            if not expression or source_bytes > MAX_PRIVILEGED_SCRIPT_BYTES:
                raise BrokerError(
                    "invalid_browser_operation",
                    "JavaScript source is empty or exceeds the configured byte limit",
                    {"max_source_bytes": MAX_PRIVILEGED_SCRIPT_BYTES},
                )
            max_result_bytes = max(
                1,
                min(
                    int(data.get("max_result_bytes") or 65_536),
                    MAX_PRIVILEGED_RESULT_BYTES,
                ),
            )
            authorized, _project_root = self._authorize_privileged(data, "javascript")
            _record, target, _ephemeral = authorized
            command = {
                key: value
                for key, value in data.items()
                if key not in {"capability_grant_token", "source_kind"}
            }
            command["max_result_bytes"] = max_result_bytes
            result = await self._forward_extension_command(
                command, authorized=authorized, timeout=max(1.0, int(data.get("timeout_ms") or 5000) / 1000 + 2)
            )
            self.broker.append_privileged_audit(
                capability="javascript",
                caller_label=data.get("caller_label"),
                target=target,
                request_id=request_id,
                outcome="succeeded" if result.get("ok") else str(result.get("code") or "failed"),
                arguments=arguments,
            )
            return result
        except BrokerError as exc:
            self.broker.append_privileged_audit(
                capability="javascript",
                caller_label=data.get("caller_label"),
                target=target,
                request_id=request_id,
                outcome=exc.code,
                arguments=arguments,
            )
            raise

    async def _execute_privileged_cdp(self, data: dict[str, Any]) -> dict[str, Any]:
        request_id = str(data["request_id"])
        target = data.get("target") if isinstance(data.get("target"), dict) else None
        method = _browser_clean_text(data.get("method"))
        params = data.get("params") if isinstance(data.get("params"), dict) else {}
        arguments = {"method": method, "parameter_keys": sorted(str(key)[:120] for key in params)[:128]}
        try:
            authorized, project_root = self._authorize_privileged(data, "raw-cdp")
            method = validate_raw_cdp_method(project_root, method)
            if len(json.dumps(params).encode("utf-8")) > MAX_PRIVILEGED_CDP_PARAMS_BYTES:
                raise BrokerError(
                    "invalid_browser_operation",
                    "CDP parameters exceed the configured byte limit",
                    {"max_parameter_bytes": MAX_PRIVILEGED_CDP_PARAMS_BYTES},
                )
            _record, target, _ephemeral = authorized
            command = {
                key: value
                for key, value in data.items()
                if key != "capability_grant_token"
            }
            command["method"] = method
            command["params"] = params
            command["max_result_bytes"] = max(
                1,
                min(int(data.get("max_result_bytes") or 65_536), MAX_PRIVILEGED_RESULT_BYTES),
            )
            result = await self._forward_extension_command(command, authorized=authorized)
            self.broker.append_privileged_audit(
                capability="raw-cdp",
                caller_label=data.get("caller_label"),
                target=target,
                request_id=request_id,
                outcome="succeeded" if result.get("ok") else str(result.get("code") or "failed"),
                arguments=arguments,
            )
            return result
        except BrokerError as exc:
            self.broker.append_privileged_audit(
                capability="raw-cdp",
                caller_label=data.get("caller_label"),
                target=target,
                request_id=request_id,
                outcome=exc.code,
                arguments=arguments,
            )
            raise

    async def _list_browser_cookies(self, data: dict[str, Any]) -> dict[str, Any]:
        request_id = str(data["request_id"])
        target = data.get("target") if isinstance(data.get("target"), dict) else None
        include_values = bool(data.get("include_values"))
        arguments = {"include_values": include_values, "scope": "selected-tab-url"}
        try:
            authorized, project_root = self._authorize_privileged(data, "cookies")
            record, target, _ephemeral = authorized
            self.broker.require_optional_permission(record, "cookies")
            if include_values:
                self.broker.validate_capability_grant(
                    token=data.get("value_capability_grant_token"),
                    capability="cookie-values",
                    extension_instance_id=record.extension_instance_id,
                    project_root=project_root,
                    caller_label=data.get("caller_label"),
                )
            command = {
                key: value
                for key, value in data.items()
                if key not in {"capability_grant_token", "value_capability_grant_token"}
            }
            command["max_entries"] = max(
                1, min(int(data.get("max_entries") or 200), MAX_PRIVILEGED_COOKIE_ENTRIES)
            )
            command["max_result_bytes"] = max(
                1, min(int(data.get("max_result_bytes") or 262_144), MAX_PRIVILEGED_RESULT_BYTES)
            )
            result = await self._forward_extension_command(command, authorized=authorized)
            if not include_values and isinstance(result.get("cookies"), list):
                for cookie in result["cookies"]:
                    if isinstance(cookie, dict):
                        cookie.pop("value", None)
                        cookie["value_redacted"] = True
            self.broker.append_privileged_audit(
                capability="cookie-values" if include_values else "cookies",
                caller_label=data.get("caller_label"), target=target,
                request_id=request_id,
                outcome="succeeded" if result.get("ok") else str(result.get("code") or "failed"),
                arguments=arguments,
            )
            return result
        except BrokerError as exc:
            self.broker.append_privileged_audit(
                capability="cookie-values" if include_values else "cookies",
                caller_label=data.get("caller_label"), target=target,
                request_id=request_id, outcome=exc.code, arguments=arguments,
            )
            raise

    async def _access_browser_content_setting(self, data: dict[str, Any]) -> dict[str, Any]:
        request_id = str(data["request_id"])
        target = data.get("target") if isinstance(data.get("target"), dict) else None
        setting = _browser_clean_text(data.get("setting")).lower().replace("-", "_")
        value = data.get("value")
        arguments = {"setting": setting, "operation": "set" if value is not None else "get"}
        try:
            if setting not in ALLOWED_CONTENT_SETTINGS:
                raise BrokerError(
                    "browser_capability_denied",
                    "content setting is not in the supported allowlist",
                    {"allowed_settings": sorted(ALLOWED_CONTENT_SETTINGS)},
                )
            if value is not None and _browser_clean_text(value) not in {"allow", "block", "ask"}:
                raise BrokerError("invalid_browser_operation", "content setting value must be allow, block, or ask")
            authorized, _project_root = self._authorize_privileged(data, "content-settings")
            record, target, _ephemeral = authorized
            self.broker.require_optional_permission(record, "content_settings")
            command = {key: item for key, item in data.items() if key != "capability_grant_token"}
            command["setting"] = setting
            result = await self._forward_extension_command(command, authorized=authorized)
            self.broker.append_privileged_audit(
                capability="content-settings", caller_label=data.get("caller_label"),
                target=target, request_id=request_id,
                outcome="succeeded" if result.get("ok") else str(result.get("code") or "failed"),
                arguments=arguments,
            )
            return result
        except BrokerError as exc:
            self.broker.append_privileged_audit(
                capability="content-settings", caller_label=data.get("caller_label"),
                target=target, request_id=request_id, outcome=exc.code, arguments=arguments,
            )
            raise

    async def _list_browser_extensions(self, data: dict[str, Any]) -> dict[str, Any]:
        request_id = str(data["request_id"])
        target = data.get("target") if isinstance(data.get("target"), dict) else None
        arguments = {"operation": "list-metadata"}
        try:
            authorized, _project_root = self._authorize_privileged(
                data, "extension-management"
            )
            record, target, _ephemeral = authorized
            self.broker.require_optional_permission(record, "extension_management")
            command = {key: item for key, item in data.items() if key != "capability_grant_token"}
            command["max_entries"] = max(1, min(int(data.get("max_entries") or 200), 500))
            result = await self._forward_extension_command(command, authorized=authorized)
            self.broker.append_privileged_audit(
                capability="extension-management", caller_label=data.get("caller_label"),
                target=target, request_id=request_id,
                outcome="succeeded" if result.get("ok") else str(result.get("code") or "failed"),
                arguments=arguments,
            )
            return result
        except BrokerError as exc:
            self.broker.append_privileged_audit(
                capability="extension-management", caller_label=data.get("caller_label"),
                target=target, request_id=request_id, outcome=exc.code, arguments=arguments,
            )
            raise

    async def _resolve_playwright_locator(
        self,
        data: dict[str, Any],
    ) -> dict[str, Any]:
        record, target, ephemeral = self.broker.authorize_command(
            data, legacy_compatibility=False
        )
        if record.is_legacy():
            raise BrokerError(
                "incompatible_browser_session",
                "Playwright locator acquisition requires a protocol-v1 extension",
                {"required_protocol_version": PROTOCOL_VERSION},
            )
        request_id = str(data["request_id"])
        snapshot_request = {
            "cmd": "get_page_snapshot",
            "request_id": f"{request_id}:snapshot",
            "target": target,
        }
        snapshot = await self._forward_extension_command(
            snapshot_request,
            authorized=(record, target, None),
        )
        if not snapshot.get("ok"):
            return {
                **snapshot,
                "request_id": request_id,
                "operation": "resolve_playwright_locator",
            }
        intent = data.get("intent") if isinstance(data.get("intent"), dict) else {}
        test_ids = data.get("test_id_attributes")
        element, candidates = generate_playwright_candidates(
            snapshot,
            intent,
            test_ids if isinstance(test_ids, list) else None,
        )
        revision = str(snapshot.get("page_context_revision") or "")
        verify_request = {
            "cmd": "verify_playwright_locators",
            "request_id": f"{request_id}:verify",
            "target": target,
            "page_context_revision": revision,
            "candidates": candidates,
        }
        verified = await self._forward_extension_command(
            verify_request,
            authorized=(record, target, ephemeral),
        )
        if not verified.get("ok"):
            return {
                **verified,
                "request_id": request_id,
                "operation": "resolve_playwright_locator",
                "target": target,
            }
        verification = (
            verified.get("verification")
            if isinstance(verified.get("verification"), list)
            else []
        )
        ranked = apply_verification_results(candidates, verification)
        recommended = next(
            (
                candidate
                for candidate in ranked
                if candidate.get("verification") == "verified"
            ),
            None,
        )
        return operation_success(
            "resolve_playwright_locator",
            request_id,
            target=target,
            page_context_revision=revision,
            url=str(snapshot.get("url") or ""),
            title=str(snapshot.get("title") or ""),
            element=element,
            recommended=recommended,
            candidates=ranked,
        )

    async def _verify_playwright_locator(
        self,
        data: dict[str, Any],
    ) -> dict[str, Any]:
        record, target, ephemeral = self.broker.authorize_command(
            data, legacy_compatibility=False
        )
        candidate = data.get("candidate")
        if not isinstance(candidate, dict):
            raise BrokerError(
                "invalid_browser_operation", "candidate is required"
            )
        request_id = str(data["request_id"])
        command = {
            "cmd": "verify_playwright_locators",
            "request_id": f"{request_id}:verify",
            "target": target,
            "page_context_revision": data.get("page_context_revision"),
            "candidates": [candidate],
        }
        result = await self._forward_extension_command(
            command,
            authorized=(record, target, ephemeral),
        )
        if not result.get("ok"):
            return {
                **result,
                "request_id": request_id,
                "operation": "verify_playwright_locator",
                "target": target,
            }
        verification = (
            result.get("verification")
            if isinstance(result.get("verification"), list)
            else []
        )
        merged = apply_verification_results([candidate], verification)
        return operation_success(
            "verify_playwright_locator",
            request_id,
            target=target,
            page_context_revision=data.get("page_context_revision"),
            candidate=merged[0],
        )

    async def _capture_browser_evidence(
        self,
        data: dict[str, Any],
    ) -> dict[str, Any]:
        record, target, ephemeral = self.broker.authorize_command(
            data, legacy_compatibility=False
        )
        project_root = self.request_project_root(data)
        request_id = str(data["request_id"])
        command = {
            "cmd": "capture_browser_evidence",
            "request_id": f"{request_id}:capture",
            "target": target,
            "page_context_revision": data.get("page_context_revision"),
        }
        result = await self._forward_extension_command(
            command,
            authorized=(record, target, ephemeral),
        )
        if not result.get("ok"):
            return {
                **result,
                "request_id": request_id,
                "operation": "capture_browser_evidence",
                "target": target,
            }
        screenshot = result.get("screenshot")
        if not isinstance(screenshot, str) or not screenshot:
            raise BrokerError(
                "browser_operation_failed",
                "browser returned no screenshot evidence",
            )
        try:
            artifact = persist_managed_browser_artifact(
                project_root,
                target,
                request_id,
                str(
                    result.get("page_context_revision")
                    or data.get("page_context_revision")
                    or ""
                ),
                "jpeg",
                base64.b64decode(screenshot, validate=True),
            )
        except ValueError as exc:
            raise BrokerError(
                "browser_artifact_failure",
                "browser returned invalid screenshot data",
            ) from exc
        return operation_success(
            "capture_browser_evidence",
            request_id,
            target=target,
            evidence={
                "request_id": request_id,
                "target": target,
                "media_type": "image/jpeg",
                "reference": artifact["path"],
                "page_context_revision": artifact["page_context_revision"],
                "artifact": public_browser_artifact_metadata(artifact),
            },
        )

    async def _capture_browser_screenshot(
        self,
        data: dict[str, Any],
    ) -> dict[str, Any]:
        record, target, ephemeral = self.broker.authorize_command(
            data, legacy_compatibility=False
        )
        project_root = self.request_project_root(data)
        request_id = str(data["request_id"])
        artifact_format = _browser_clean_text(data.get("format")) or "png"
        if artifact_format not in {"png", "jpeg"}:
            raise BrokerError("invalid_browser_operation", "format must be png or jpeg")
        element = data.get("element")
        selector = ""
        candidate: dict[str, Any] | None = None
        locator_context: dict[str, Any] = {}
        if element is not None:
            if not isinstance(element, dict):
                raise BrokerError("invalid_browser_operation", "element must be an object")
            choices = [
                bool(_browser_clean_text(element.get("reference"))),
                isinstance(element.get("candidate"), dict),
                bool(_browser_clean_text(element.get("css"))),
            ]
            if sum(choices) != 1 or data.get("full_page"):
                raise BrokerError(
                    "invalid_browser_operation",
                    "element screenshot requires exactly one element input and cannot be full-page",
                )
            revision = _browser_clean_text(element.get("page_context_revision"))
            if choices[0]:
                reference = self.broker.resolve_element_reference(
                    record.extension_instance_id,
                    target,
                    _browser_clean_text(element.get("reference")),
                    page_context_revision=revision,
                    snapshot_id=_browser_clean_text(element.get("snapshot_id")),
                )
                revision = reference.page_context_revision
                locator_context = reference.context
                selector = _browser_clean_text(reference.element.get("shortSelector"))
                raw_candidate = reference.element.get("candidate")
                if isinstance(raw_candidate, dict):
                    candidate = raw_candidate
            elif choices[1]:
                candidate = dict(element["candidate"])
                context = candidate.get("context")
                if isinstance(context, dict):
                    locator_context = dict(context)
            else:
                selector = _browser_clean_text(element.get("css"))
            if candidate is not None:
                verified = await self._forward_extension_command(
                    {
                        "cmd": "verify_playwright_locators",
                        "request_id": f"{request_id}:preflight",
                        "target": target,
                        "page_context_revision": revision,
                        "candidates": [candidate],
                    },
                    authorized=(record, target, None),
                )
                first = (verified.get("verification") or [{}])[0]
                if not verified.get("ok") or int(first.get("match_count") or 0) != 1:
                    raise BrokerError(
                        "stale_element_reference",
                        "structured locator is no longer unique for element screenshot",
                    )
        command = {
            "cmd": "capture_browser_screenshot",
            "request_id": f"{request_id}:viewport",
            "target": target,
            "page_context_revision": data.get("page_context_revision"),
            "format": artifact_format,
            "quality": data.get("quality"),
            "full_page": bool(data.get("full_page")),
            "selector": selector or None,
            "candidate": candidate,
            "locator_context": locator_context,
        }
        result = await self._forward_extension_command(
            command, authorized=(record, target, ephemeral)
        )
        if not result.get("ok"):
            return {
                **result,
                "request_id": request_id,
                "operation": "capture_browser_screenshot",
                "target": target,
            }
        encoded = result.get("artifact_data")
        if not isinstance(encoded, str) or not encoded:
            raise BrokerError(
                "browser_artifact_failure", "browser returned no screenshot data"
            )
        try:
            payload = base64.b64decode(encoded, validate=True)
        except ValueError as exc:
            raise BrokerError(
                "browser_artifact_failure", "browser returned invalid screenshot data"
            ) from exc
        artifact = persist_managed_browser_artifact(
            project_root,
            target,
            request_id,
            str(result.get("page_context_revision") or ""),
            artifact_format,
            payload,
        )
        return operation_success(
            "capture_browser_screenshot",
            request_id,
            target=target,
            artifact=public_browser_artifact_metadata(artifact),
        )

    async def _generate_browser_pdf(self, data: dict[str, Any]) -> dict[str, Any]:
        record, target, ephemeral = self.broker.authorize_command(
            data, legacy_compatibility=False
        )
        project_root = self.request_project_root(data)
        request_id = str(data["request_id"])
        scale = float(data.get("scale") or 1.0)
        if not 0.1 <= scale <= 2.0:
            raise BrokerError("invalid_browser_operation", "PDF scale is out of range")
        result = await self._forward_extension_command(
            {
                "cmd": "generate_browser_pdf",
                "request_id": f"{request_id}:pdf",
                "target": target,
                "page_context_revision": data.get("page_context_revision"),
                "paper_format": data.get("paper_format") or "A4",
                "landscape": bool(data.get("landscape")),
                "scale": scale,
                "print_background": bool(data.get("print_background")),
            },
            authorized=(record, target, ephemeral),
        )
        if not result.get("ok"):
            return {
                **result,
                "request_id": request_id,
                "operation": "generate_browser_pdf",
                "target": target,
            }
        encoded = result.get("artifact_data")
        if not isinstance(encoded, str) or not encoded:
            raise BrokerError("browser_artifact_failure", "browser returned no PDF data")
        try:
            payload = base64.b64decode(encoded, validate=True)
        except ValueError as exc:
            raise BrokerError("browser_artifact_failure", "browser returned invalid PDF data") from exc
        artifact = persist_managed_browser_artifact(
            project_root,
            target,
            request_id,
            str(result.get("page_context_revision") or ""),
            "pdf",
            payload,
        )
        return operation_success(
            "generate_browser_pdf",
            request_id,
            target=target,
            artifact=public_browser_artifact_metadata(artifact),
        )

    async def _execute_browser_action(
        self,
        data: dict[str, Any],
    ) -> dict[str, Any]:
        """Re-verify exactly one structured input and dispatch one action."""
        record, target, ephemeral = self.broker.authorize_command(
            data, legacy_compatibility=False
        )
        element = data.get("element")
        if not isinstance(element, dict):
            raise BrokerError(
                "invalid_browser_operation", "element input is required"
            )
        choices = [
            bool(_browser_clean_text(element.get("reference"))),
            isinstance(element.get("candidate"), dict),
            bool(_browser_clean_text(element.get("css"))),
        ]
        if sum(choices) != 1:
            raise BrokerError(
                "invalid_browser_operation",
                "exactly one of element.reference, element.candidate, or element.css is required",
            )
        action = _browser_clean_text(data.get("action"))
        upload_files: list[str] = []
        if action == "upload":
            upload_files = validate_browser_upload_files(
                self.request_project_root(data), data.get("files")
            )
        elif data.get("files"):
            raise BrokerError(
                "invalid_browser_operation",
                "files are accepted only for the upload browser action",
            )
        value_required = {"fill", "type", "select", "press_key", "assert_text"}
        if action in value_required and data.get("value") is None:
            raise BrokerError(
                "invalid_browser_operation",
                f"value is required for browser action {action}",
            )
        if record.supported_actions and action not in record.supported_actions:
            raise BrokerError(
                "unsupported_browser_action",
                f"browser action is not advertised by this Profile: {action}",
                {"supported_actions": record.supported_actions},
            )
        revision = _browser_clean_text(element.get("page_context_revision"))
        snapshot_id = _browser_clean_text(element.get("snapshot_id"))
        candidate: dict[str, Any] | None = None
        selector = _browser_clean_text(element.get("css"))
        locator_context: dict[str, Any] = {}
        if choices[0]:
            reference = self.broker.resolve_element_reference(
                record.extension_instance_id,
                target,
                _browser_clean_text(element.get("reference")),
                page_context_revision=revision,
                snapshot_id=snapshot_id,
            )
            revision = reference.page_context_revision
            snapshot_id = reference.snapshot_id
            locator_context = reference.context
            raw_candidate = reference.element.get("candidate")
            if isinstance(raw_candidate, dict):
                candidate = raw_candidate
            selector = _browser_clean_text(reference.element.get("shortSelector"))
        elif choices[1]:
            candidate = dict(element["candidate"])
            context = candidate.get("context")
            if isinstance(context, dict):
                locator_context = dict(context)

        request_id = str(data["request_id"])
        if candidate is not None:
            verify_command = {
                "cmd": "verify_playwright_locators",
                "request_id": f"{request_id}:preflight",
                "target": target,
                "page_context_revision": revision,
                "candidates": [candidate],
            }
            verified = await self._forward_extension_command(
                verify_command,
                authorized=(record, target, None),
            )
            if not verified.get("ok"):
                return {
                    **verified,
                    "request_id": request_id,
                    "operation": "execute_browser_action",
                }
            verification = verified.get("verification")
            first = verification[0] if isinstance(verification, list) and verification else {}
            if not isinstance(first, dict) or int(first.get("match_count") or 0) != 1:
                raise BrokerError(
                    "stale_element_reference",
                    "structured locator is no longer unique; no action was executed",
                    {"retry": "request a new snapshot or locator candidate"},
                )

        command = {
            "cmd": "execute_locator",
            "request_id": f"{request_id}:action",
            "target": target,
            "action": action,
            "value": data.get("value"),
            "selector": selector or None,
            "candidate": candidate,
            "locator_context": locator_context,
            "page_context_revision": revision or None,
            "snapshot_id": snapshot_id or None,
            "focus": bool(data.get("focus")),
            "wait": data.get("wait"),
            "monitor": bool(data.get("monitor")),
            "files": upload_files,
        }
        wait = command.get("wait")
        if isinstance(wait, dict) and wait.get("kind") == "element_state":
            normalized_wait = dict(wait)
            normalized_wait["element"] = {
                "candidate": candidate,
                "css": selector or None,
                "page_context_revision": revision or None,
            }
            command["wait"] = normalized_wait
        result = await self._forward_extension_command(
            command,
            authorized=(record, target, ephemeral),
        )
        return {
            **result,
            "request_id": request_id,
            "operation": "execute_browser_action",
            "target": target,
            "page_context_revision": result.get("page_context_revision")
            or revision,
            "snapshot_id": snapshot_id or None,
        }

    async def _emit_event(self, event: dict[str, Any]) -> None:
        if self._event_callback is not None:
            await self._event_callback(event)


def _http_response(
    status: int,
    body: bytes,
    content_type: str = "application/json",
    cors_origin: str | None = None,
) -> bytes:
    reason = {200: "OK", 403: "Forbidden", 404: "Not Found"}.get(status, "Error")
    cors_headers = ""
    if cors_origin and CHROME_EXTENSION_ORIGIN_RE.fullmatch(cors_origin):
        cors_headers = (
            f"Access-Control-Allow-Origin: {cors_origin}\r\n"
            "Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n"
            "Access-Control-Allow-Headers: Content-Type, X-Teshi-Broker-Token\r\n"
            "Vary: Origin\r\n"
        )
    header = (
        f"HTTP/1.1 {status} {reason}\r\n"
        f"Content-Type: {content_type}\r\n"
        f"Content-Length: {len(body)}\r\n"
        f"{cors_headers}"
        "Connection: close\r\n"
        "\r\n"
    ).encode("ascii")
    return header + body


async def _read_http_request(
    reader: asyncio.StreamReader,
) -> tuple[str, dict[str, str], bytes]:
    request_line = (await reader.readline()).decode("utf-8", errors="ignore").strip()
    headers: dict[str, str] = {}
    while True:
        line = await reader.readline()
        if line in (b"\r\n", b"\n", b""):
            break
        decoded = line.decode("utf-8", errors="ignore").strip()
        if ":" in decoded:
            key, value = decoded.split(":", 1)
            headers[key.strip().lower()] = value.strip()
    length = int(headers.get("content-length", "0") or "0")
    if length < 0 or length > MAX_BROWSER_WS_MESSAGE_BYTES:
        raise ValueError("HTTP request body exceeds the bounded bridge limit")
    # StreamReader.read(n) may legally return fewer than n bytes. Extension
    # snapshot/response payloads are often split across TCP packets, so wait
    # for the complete Content-Length body before decoding JSON.
    body = await reader.readexactly(length) if length > 0 else b""
    return request_line, headers, body


def authenticated_http_post(
    headers: dict[str, str], expected_token: str, raw_path: str = ""
) -> bool:
    """Authorize a loopback bridge mutation without exposing CORS access."""
    header_token = headers.get("x-teshi-broker-token", "")
    query_tokens = parse_qs(urlparse(raw_path).query).get("token", [])
    query_token = query_tokens[0] if len(query_tokens) == 1 else ""
    return secrets.compare_digest(header_token, expected_token) or secrets.compare_digest(
        query_token, expected_token
    )


async def run_http_discovery(
    bridge: ChromeBridge, host: str, discovery_port: int, command_token: str
) -> None:
    async def handle_client(
        reader: asyncio.StreamReader, writer: asyncio.StreamWriter
    ) -> None:
        try:
            request_line, headers, body = await _read_http_request(reader)
            parts = request_line.split()
            method = parts[0].upper() if parts else ""
            raw_path = parts[1] if len(parts) > 1 else ""
            path = urlparse(raw_path).path
            raw_origin = headers.get("origin", "")
            extension_origin = (
                raw_origin if CHROME_EXTENSION_ORIGIN_RE.fullmatch(raw_origin) else None
            )

            if method == "POST" and not authenticated_http_post(
                headers, command_token, raw_path
            ):
                writer.write(
                    _http_response(
                        403, b'{"error":"forbidden"}', cors_origin=extension_origin
                    )
                )
            elif method == "OPTIONS":
                status = 200 if extension_origin else 403
                writer.write(_http_response(status, b"", cors_origin=extension_origin))
            elif method == "GET" and path == "/v1/bridge":
                payload = json.dumps(bridge.bridge_info()).encode("utf-8")
                writer.write(_http_response(200, payload, cors_origin=extension_origin))
            elif method == "POST" and path == "/v1/bridge/heartbeat":
                data = json.loads(body.decode("utf-8") or "{}")
                result = await bridge.handle_heartbeat(data)
                writer.write(
                    _http_response(
                        200,
                        json.dumps(result).encode("utf-8"),
                        cors_origin=extension_origin,
                    )
                )
            elif method == "POST" and path == "/v1/bridge/response":
                text = body.decode("utf-8") or "{}"
                if len(body) > 65536:
                    data = await asyncio.to_thread(json.loads, text)
                else:
                    data = json.loads(text)
                result = await bridge.handle_extension_response(data)
                writer.write(
                    _http_response(
                        200,
                        json.dumps(result).encode("utf-8"),
                        cors_origin=extension_origin,
                    )
                )
            elif method == "POST" and path == "/v1/bridge/activate_tab":
                data = json.loads(body.decode("utf-8") or "{}")
                result = await bridge.handle_activate_tab_http(data)
                writer.write(
                    _http_response(
                        200,
                        json.dumps(result).encode("utf-8"),
                        cors_origin=extension_origin,
                    )
                )
            elif method == "POST" and path == "/v1/bridge/capture_now":
                data = json.loads(body.decode("utf-8") or "{}")
                result = await bridge.handle_capture_now_http(data)
                writer.write(
                    _http_response(
                        200,
                        json.dumps(result).encode("utf-8"),
                        cors_origin=extension_origin,
                    )
                )
            else:
                writer.write(_http_response(404, b"{}"))
            await writer.drain()
        finally:
            writer.close()
            await writer.wait_closed()

    server = await asyncio.start_server(handle_client, host, discovery_port)
    async with server:
        await server.serve_forever()


def _websocket_path(websocket: Any) -> str:
    request = getattr(websocket, "request", None)
    if request is not None:
        return str(getattr(request, "path", "/") or "/")
    return str(getattr(websocket, "path", "/") or "/")


def authenticated_websocket_path(raw_path: str, expected_token: str) -> str | None:
    """Return the route only when the URL carries exactly one matching token."""
    request_url = urlparse(raw_path)
    supplied_tokens = parse_qs(request_url.query).get("token", [])
    supplied_token = supplied_tokens[0] if len(supplied_tokens) == 1 else ""
    if not secrets.compare_digest(supplied_token, expected_token):
        return None
    return request_url.path


def _bind_websocket_listener(host: str, port: int) -> tuple[socket.socket, int]:
    """Bind once so an ephemeral port is known before discovery is published."""
    family = socket.AF_INET6 if ":" in host else socket.AF_INET
    listener = socket.socket(family, socket.SOCK_STREAM)
    try:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        listener.bind((host, port))
        listener.listen()
        listener.setblocking(False)
        return listener, int(listener.getsockname()[1])
    except Exception:
        listener.close()
        raise


async def run_chrome(
    host: str,
    port: int,
    discovery_port: int,
    project_root: Path,
) -> None:
    import websockets

    listener, actual_port = _bind_websocket_listener(host, port)
    command_token = secrets.token_urlsafe(32)
    ws_url = f"ws://{host}:{actual_port}/?token={command_token}"
    extension_frame_ws_url = (
        f"ws://{host}:{actual_port}{EXTENSION_FRAME_WS_PATH}?token={command_token}"
    )
    clients: dict[Any, str | None] = {}
    extension_streams: dict[str, Any] = {}

    async def broadcast_ws_message(message: dict[str, Any]) -> None:
        if not clients:
            return
        payload = json.dumps(message)
        dead: list[Any] = []
        message_instance = str(
            message.get("extension_instance_id")
            or (message.get("target") or {}).get("extension_instance_id")
            or ""
        )
        default_instance = str(
            bridge.bridge_info().get("selected_session_id") or ""
        )

        async def send_one(ws: Any, selected_instance: str | None) -> None:
            if message_instance:
                effective = selected_instance or default_instance
                if not effective or effective != message_instance:
                    return
            try:
                await ws.send(payload)
            except Exception:
                dead.append(ws)

        await asyncio.gather(
            *(send_one(ws, selected) for ws, selected in list(clients.items()))
        )
        for ws in dead:
            clients.pop(ws, None)

    async def broadcast_frame(frame_payload: dict[str, Any]) -> None:
        await broadcast_ws_message(frame_payload)

    async def send_direct_command(
        extension_instance_id: str, command: dict[str, Any]
    ) -> bool:
        websocket = extension_streams.get(extension_instance_id)
        if websocket is None:
            return False
        try:
            await websocket.send(
                json.dumps({"type": "direct_command", "command": command})
            )
            return True
        except Exception:  # noqa: BLE001
            extension_streams.pop(extension_instance_id, None)
            return False

    bridge = ChromeBridge(
        project_root,
        ws_url,
        discovery_port,
        extension_frame_ws_url,
        frame_callback=broadcast_frame,
        event_callback=broadcast_ws_message,
        direct_command_callback=send_direct_command,
    )
    bridge.write_endpoint()

    http_task = asyncio.create_task(
        run_http_discovery(bridge, host, discovery_port, command_token)
    )

    async def handle_extension_websocket(websocket: Any) -> None:
        stream_authenticated = False
        stream_instance_id = ""
        try:
            async for message in websocket:
                if isinstance(message, str):
                    try:
                        data = json.loads(message)
                    except json.JSONDecodeError:
                        continue
                    if data.get("type") == "stream_hello":
                        ack = bridge.validate_stream_hello(data)
                        stream_authenticated = bool(ack.get("ok"))
                        stream_instance_id = str(
                            ack.get("extension_instance_id") or ""
                        )
                        if stream_authenticated and stream_instance_id:
                            extension_streams[stream_instance_id] = websocket
                        await websocket.send(json.dumps(ack))
                    elif data.get("type") == "response" and stream_authenticated:
                        data["extension_instance_id"] = stream_instance_id
                        await bridge.handle_extension_response(data)
                    elif data.get("type") == "frame_error":
                        await bridge.handle_extension_response(
                            {
                                "type": "frame_error",
                                "extension_instance_id": stream_instance_id,
                                "error": data.get(
                                    "error", "extension stream error"
                                ),
                            }
                        )
                elif (
                    isinstance(message, bytes)
                    and stream_authenticated
                    and stream_instance_id
                ):
                    await bridge.handle_extension_binary(message, stream_instance_id)
        except Exception as exc:  # noqa: BLE001
            print(f"extension frame websocket closed: {exc}", file=sys.stderr)
        finally:
            if (
                stream_instance_id
                and extension_streams.get(stream_instance_id) is websocket
            ):
                extension_streams.pop(stream_instance_id, None)

    async def handle_desktop_websocket(websocket: Any) -> None:
        clients[websocket] = None
        default_instance = str(
            bridge.bridge_info().get("selected_session_id") or ""
        )
        initial_frame = bridge._last_frame
        initial_instance = str(
            (initial_frame or {}).get("extension_instance_id")
            or ((initial_frame or {}).get("target") or {}).get(
                "extension_instance_id"
            )
            or ""
        )
        if initial_frame is not None and initial_instance == default_instance:
            try:
                await websocket.send(json.dumps(initial_frame))
            except Exception:
                clients.pop(websocket, None)
        try:
            async for message in websocket:
                try:
                    data = json.loads(message)
                except json.JSONDecodeError:
                    continue
                if data.get("cmd") == "subscribe_browser_session":
                    instance_id = str(
                        data.get("extension_instance_id") or ""
                    ).strip()
                    try:
                        record = bridge.broker.require_session(instance_id)
                    except BrokerError as exc:
                        await websocket.send(
                            json.dumps(
                                exc.response(
                                    str(data.get("request_id") or ""),
                                    "subscribe_browser_session",
                                )
                            )
                        )
                        continue
                    clients[websocket] = instance_id
                    await websocket.send(
                        json.dumps(
                            operation_success(
                                "subscribe_browser_session",
                                str(data.get("request_id") or ""),
                                extension_instance_id=instance_id,
                            )
                        )
                    )
                    if record.latest_frame is not None:
                        await websocket.send(json.dumps(record.latest_frame))
                    continue
                if data.get("type") == "frame" and data.get("data"):
                    frame_out: dict[str, Any] = {
                        "type": "frame",
                        "data": data.get("data", ""),
                        "url": str(
                            data.get("url", bridge.bridge_info().get("page_url", ""))
                        ),
                    }
                    raw_tab = data.get("tab_id")
                    if raw_tab is not None:
                        try:
                            frame_out["tab_id"] = int(raw_tab)
                        except (TypeError, ValueError):
                            pass
                    # Desktop-originated frames are legacy-only and cannot be
                    # routed safely when several extension sessions are live.
                    if bridge.bridge_info().get("ambiguous_browser_target"):
                        continue
                    default_instance = str(
                        bridge.bridge_info().get("selected_session_id") or ""
                    )
                    if default_instance:
                        try:
                            record, target, _explicit = bridge.broker.resolve_target(
                                None
                            )
                            frame_out["extension_instance_id"] = default_instance
                            frame_out["target"] = target
                            bridge.broker.update_frame(
                                record.extension_instance_id, target, frame_out
                            )
                            bridge._last_frame = frame_out
                            await broadcast_frame(frame_out)
                        except BrokerError:
                            pass
                    continue
                if "cmd" in data:
                    reply = await bridge.forward_command(data)
                    await websocket.send(json.dumps(reply))
        finally:
            clients.pop(websocket, None)

    async def connection_handler(websocket: Any) -> None:
        path = authenticated_websocket_path(_websocket_path(websocket), command_token)
        if path is None:
            await websocket.close(1008, "invalid browser broker token")
            return
        if path == EXTENSION_FRAME_WS_PATH:
            await handle_extension_websocket(websocket)
            return
        await handle_desktop_websocket(websocket)

    async with websockets.serve(
        connection_handler,
        sock=listener,
        origins=[None, CHROME_EXTENSION_ORIGIN_RE],
        max_size=MAX_BROWSER_WS_MESSAGE_BYTES,
        max_queue=4,
    ):
        print(actual_port, flush=True)
        await asyncio.gather(http_task, asyncio.Future())


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--mode", choices=("embedded", "chrome"), default="embedded")
    parser.add_argument("--cdp-port", type=int, default=0)
    parser.add_argument("--discovery-port", type=int, default=DEFAULT_DISCOVERY_PORT)
    parser.add_argument("--project-root", default="")
    parser.add_argument(
        "--user-session",
        action="store_true",
        help="Run Chrome mode as a reusable per-user broker; project root is compatibility context only",
    )
    parser.add_argument(
        "--no-preview-stream",
        action="store_true",
        help="Disable JPEG preview loop (CI/headless replay)",
    )
    args = parser.parse_args()
    project_root = Path(args.project_root) if args.project_root else None

    try:
        if args.mode == "chrome":
            if project_root is None and not args.user_session:
                print("chrome mode requires --project-root", file=sys.stderr)
                sys.exit(1)
            if project_root is None:
                project_root = Path.cwd()
            asyncio.run(
                run_chrome(args.host, args.port, args.discovery_port, project_root)
            )
        else:
            if args.cdp_port <= 0:
                print("embedded mode requires --cdp-port", file=sys.stderr)
                sys.exit(1)
            asyncio.run(
                run_embedded(
                    args.host,
                    args.port,
                    args.cdp_port,
                    project_root,
                    no_preview_stream=args.no_preview_stream,
                )
            )
    except KeyboardInterrupt:
        sys.exit(0)


if __name__ == "__main__":
    main()
