"""Embedded Playwright browser sidecar. Chrome automation uses the Rust broker."""

from __future__ import annotations

from update_participant import register as _register_update_participant
_update_participant = _register_update_participant(__file__)

import argparse
import asyncio
import base64
import json
import os
import re
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlparse
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
ALLOWED_CONTENT_SETTINGS = {
    "notifications",
    "popups",
    "geolocation",
    "camera",
    "microphone",
    "automatic_downloads",
}
PRIVILEGED_POLICY_FILENAME = "browser-policy.json"


def embedded_browser_headless() -> bool:
    """Return whether embedded Chromium should run headless by default.

    GPUI WebGPU requires a browser adapter, which is unavailable in some
    headless Chromium environments. Keep the existing CI-friendly default,
    while allowing local Web E2E runs to opt into a headed browser explicitly.
    """
    value = os.environ.get("TESHI_EMBEDDED_HEADLESS")
    if value is None:
        return True
    return value.strip().lower() not in {"0", "false", "no", "off"}


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
    "assert_not_exists",
    "assert_text",
    "assert_text_count",
    "select",
    "press_key",
}


def structured_locator_preflight_passes(action: str, match_count: int) -> bool:
    """Return whether a structured locator may reach the requested action.

    Positive actions still require one unique match.  A negative existence
    assertion is also valid when the structured locator has zero matches; the
    action implementation must then perform the actual negative assertion.
    """
    return match_count == 1 or (action == "assert_not_exists" and match_count == 0)


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
    broker_features: list[str] | None = None,
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
        if broker_features is not None:
            payload["broker_features"] = list(broker_features)
    if extension_frame_ws_url:
        payload["extension_frame_ws_url"] = extension_frame_ws_url
    endpoint_path = teshi_dir / "cdp-endpoint.json"
    temporary_path: str | None = None
    try:
        # CLI commands read this file concurrently with heartbeat/response
        # handlers.  Replacing a complete same-directory file prevents a
        # reader from observing the empty/truncated state of write_text().
        with tempfile.NamedTemporaryFile(
            "w",
            encoding="utf-8",
            dir=teshi_dir,
            prefix=".cdp-endpoint-",
            suffix=".tmp",
            delete=False,
        ) as stream:
            temporary_path = stream.name
            json.dump(payload, stream, indent=2)
            stream.flush()
            os.fsync(stream.fileno())
        for attempt in range(50):
            try:
                os.replace(temporary_path, endpoint_path)
                break
            except PermissionError:
                if attempt == 49:
                    raise
                # Windows readers may briefly hold the previous endpoint open
                # without sharing delete. Keep the old complete file visible
                # and retry the atomic replacement instead of truncating it.
                time.sleep(0.005)
        temporary_path = None
    finally:
        if temporary_path is not None:
            try:
                Path(temporary_path).unlink()
            except OSError:
                pass


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
        self.gpu_errors: list[str] = []

    async def start(self, cdp_port: int) -> None:
        from playwright.async_api import async_playwright

        self.playwright = await async_playwright().start()
        args = [f"--remote-debugging-port={cdp_port}"]
        if os.environ.get("TESHI_EMBEDDED_SOFTWARE_WEBGPU") == "1":
            args.extend(["--enable-unsafe-webgpu", "--enable-unsafe-swiftshader", "--use-gl=angle", "--use-angle=swiftshader", "--use-webgpu-adapter=swiftshader"])
        self.browser = await self.playwright.chromium.launch(
            headless=embedded_browser_headless(),
            args=args,
        )
        self.context = await self.browser.new_context(
            viewport={"width": 1920, "height": 1080}
        )
        self.page = await self.context.new_page()
        self.page.on("console", self._observe_gpu_error)
        await self.page.goto("about:blank")
        self.cdp_session = await self.context.new_cdp_session(self.page)

        # Inject engine-inspired JS helpers
        await self.page.evaluate(MAKE_SHORT_SELECTOR_JS)

    def _observe_gpu_error(self, message: Any) -> None:
        if message.type == "error" and "gpu" in message.text.lower():
            self.gpu_errors = (self.gpu_errors + [message.text[:2000]])[-5:]

    def current_url(self) -> str:
        if self.page is None:
            return "about:blank"
        return self.page.url

    async def navigate(self, url: str) -> None:
        async with self._lock:
            if self.page is not None:
                self.gpu_errors.clear()
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

            # DOM.requestNode requires the DOM domain to be enabled for a
            # Playwright-created CDP session. Without this, the selector can
            # execute successfully but locator verification cannot record the
            # required visual highlight.
            await self.cdp_session.send("DOM.enable")
            await self.cdp_session.send("Overlay.enable")
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

            document_result = await self.cdp_session.send(
                "DOM.getDocument", {"depth": -1, "pierce": True}
            )
            root_node_id = document_result.get("root", {}).get("nodeId")
            if not root_node_id:
                return {"ok": False, "error": "could not resolve document node"}
            node_result = await self.cdp_session.send(
                "DOM.querySelector",
                {"nodeId": root_node_id, "selector": selector},
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
                if action == "assert_not_exists":
                    match_count = await locator.count()
                    if match_count:
                        return {
                            "ok": False,
                            "selector": selector or None,
                            "candidate": candidate,
                            "action": action,
                            "match_count": match_count,
                            "error": "element exists",
                            "code": "assert_not_exists_failed",
                        }
                    return {
                        "ok": True,
                        "selector": selector or None,
                        "candidate": candidate,
                        "action": action,
                        "match_count": 0,
                    }
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
                if hasattr(self.page, "accessibility"):
                    tree = await self.page.accessibility.snapshot(interesting_only=False)
                else:
                    # Recent Playwright releases expose ARIA snapshots through
                    # locators instead of Page.accessibility.
                    tree = await self.page.locator("body").aria_snapshot()
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
            if self.gpu_errors:
                snapshot["gpu_errors"] = list(self.gpu_errors)
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
                    project_root=data.get("project_root"),
                    caller_label=data.get("caller_label"),
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
                    project_root=data.get("project_root"),
                    caller_label=data.get("caller_label"),
                )
                return operation_success(str(cmd), str(request_id or ""), lease=lease)
            except BrokerError as exc:
                return exc.response(str(request_id or ""), str(cmd))
        if cmd == "release_browser_lease":
            try:
                released = broker.release_lease(
                    str(data.get("extension_instance_id") or ""),
                    str(data.get("lease_token") or ""),
                    project_root=data.get("project_root"),
                    caller_label=data.get("caller_label"),
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
        } or (
            cmd in {"navigate", "go_back"} and data.get("target") is not None
        ) or (cmd == "get_page_snapshot" and data.get("target") is not None):
            # Embedded Playwright commands without a target are the local sidecar
            # transport used by the self-test workflow. Targeted commands still
            # go through the broker authorization path above.
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
                        verification = observed.get("verification")
                        first = (
                            verification[0]
                            if isinstance(verification, list) and verification
                            else {}
                        )
                        if (
                            not observed.get("ok")
                            or not isinstance(first, dict)
                            or not structured_locator_preflight_passes(
                                str(data.get("action") or ""),
                                int(first.get("match_count") or 0),
                            )
                        ):
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
    agent_broker = BrowserSessionBroker(
        HEARTBEAT_TTL_SEC,
        default_project_root=project_root,
        enforce_scoped_leases=True,
    )
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


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--mode", choices=("embedded",), default="embedded")
    parser.add_argument("--cdp-port", type=int, default=0)
    parser.add_argument("--discovery-port", type=int, default=DEFAULT_DISCOVERY_PORT)
    parser.add_argument("--project-root", default="")
    parser.add_argument(
        "--no-preview-stream",
        action="store_true",
        help="Disable JPEG preview loop (CI/headless replay)",
    )
    args = parser.parse_args()
    project_root = Path(args.project_root) if args.project_root else None

    try:
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
