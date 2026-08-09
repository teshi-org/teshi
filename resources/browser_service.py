"""Browser bridge for teshi-desktop: embedded Playwright or Chrome extension."""

from __future__ import annotations

import argparse
import asyncio
import base64
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import parse_qs, urlparse
from typing import Any

DEFAULT_DISCOVERY_PORT = 17373
# Extension is considered connected if heartbeat POST was received within this window.
HEARTBEAT_TTL_SEC = 8.0
EXTENSION_FRAME_WS_PATH = "/extension/frames"
FRAME_MAGIC = b"TSH1"


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


def paths_equal(got: str, expected: Path) -> bool:
    """Compare project roots (case-insensitive on Windows)."""
    if not got or not str(got).strip():
        return True
    try:
        a = Path(got).resolve()
        b = expected.resolve()
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
    ) -> dict[str, Any]:
        if self.page is None:
            return {"ok": False, "error": "browser not ready", "code": "browser_not_ready"}
        if not selector:
            return {"ok": False, "error": "selector is required", "code": "invalid_selector"}
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
            locator = self.page.locator(selector).first
            try:
                await locator.wait_for(state="visible", timeout=timeout_ms)
                if action == "click":
                    await locator.click(timeout=timeout_ms)
                elif action == "fill":
                    await locator.fill(value or "", timeout=timeout_ms)
                elif action == "type":
                    await locator.click(timeout=timeout_ms)
                    # Wait for shell to spawn before typing; the Terminal tab
                    # click triggers async shell spawn which may not complete
                    # before the action's keyboard input arrives.
                    await asyncio.sleep(3)
                    await self.page.keyboard.type(value or "")
                    await self.page.keyboard.press("Enter")
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

            return {"ok": True, "selector": selector, "action": action}

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

            # Also inject __teshiMakeShortSelector if not already present
            try:
                await self.page.evaluate(
                    "typeof window.__teshiMakeShortSelector === 'function'"
                )
            except Exception:
                await self.page.evaluate(MAKE_SHORT_SELECTOR_JS)

            return normalize_snapshot(url, title, tree, buttons)

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
    session: EmbeddedSession, data: dict[str, Any]
) -> dict[str, Any]:
    cmd = data.get("cmd")
    request_id = data.get("request_id")

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
                        reply = await handle_embedded_command(session, data)
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


class ChromeBridge:
    """Chrome mode: extension talks HTTP heartbeat; agents use WebSocket."""

    def __init__(
        self,
        project_root: Path,
        ws_url: str,
        discovery_port: int,
        extension_frame_ws_url: str,
        frame_callback: Any | None = None,
        event_callback: Any | None = None,
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
        self._deprecated_json_frame_warned = False

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
            "transport": "http-heartbeat+ws-screencast",
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


def _http_response(status: int, body: bytes, content_type: str = "application/json") -> bytes:
    reason = "OK" if status == 200 else "Not Found"
    header = (
        f"HTTP/1.1 {status} {reason}\r\n"
        f"Content-Type: {content_type}\r\n"
        "Access-Control-Allow-Origin: *\r\n"
        "Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n"
        "Access-Control-Allow-Headers: Content-Type, X-Project-Root, X-Tab-Id, X-Url\r\n"
        f"Content-Length: {len(body)}\r\n"
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
    body = await reader.read(length) if length > 0 else b""
    return request_line, headers, body


async def run_http_discovery(
    bridge: ChromeBridge, host: str, discovery_port: int
) -> None:
    async def handle_client(
        reader: asyncio.StreamReader, writer: asyncio.StreamWriter
    ) -> None:
        try:
            request_line, _headers, body = await _read_http_request(reader)
            parts = request_line.split()
            method = parts[0].upper() if parts else ""
            path = parts[1] if len(parts) > 1 else ""

            if method == "OPTIONS":
                writer.write(_http_response(200, b""))
            elif method == "GET" and path == "/v1/bridge":
                payload = json.dumps(bridge.bridge_info()).encode("utf-8")
                writer.write(_http_response(200, payload))
            elif method == "POST" and path == "/v1/bridge/heartbeat":
                data = json.loads(body.decode("utf-8") or "{}")
                result = await bridge.handle_heartbeat(data)
                writer.write(_http_response(200, json.dumps(result).encode("utf-8")))
            elif method == "POST" and path == "/v1/bridge/response":
                text = body.decode("utf-8") or "{}"
                if len(body) > 65536:
                    data = await asyncio.to_thread(json.loads, text)
                else:
                    data = json.loads(text)
                result = await bridge.handle_extension_response(data)
                writer.write(_http_response(200, json.dumps(result).encode("utf-8")))
            elif method == "POST" and path == "/v1/bridge/activate_tab":
                data = json.loads(body.decode("utf-8") or "{}")
                result = await bridge.handle_activate_tab_http(data)
                writer.write(_http_response(200, json.dumps(result).encode("utf-8")))
            elif method == "POST" and path == "/v1/bridge/capture_now":
                data = json.loads(body.decode("utf-8") or "{}")
                result = await bridge.handle_capture_now_http(data)
                writer.write(_http_response(200, json.dumps(result).encode("utf-8")))
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


async def run_chrome(
    host: str,
    port: int,
    discovery_port: int,
    project_root: Path,
) -> None:
    import websockets

    ws_url = f"ws://{host}:{port}"
    extension_frame_ws_url = f"ws://{host}:{port}{EXTENSION_FRAME_WS_PATH}"
    clients: set[Any] = set()

    async def broadcast_ws_message(message: dict[str, Any]) -> None:
        if not clients:
            return
        payload = json.dumps(message)
        dead: list[Any] = []

        async def send_one(ws: Any) -> None:
            try:
                await ws.send(payload)
            except Exception:
                dead.append(ws)

        await asyncio.gather(*(send_one(ws) for ws in list(clients)))
        for ws in dead:
            clients.discard(ws)

    async def broadcast_frame(frame_payload: dict[str, Any]) -> None:
        await broadcast_ws_message(frame_payload)

    bridge = ChromeBridge(
        project_root,
        ws_url,
        discovery_port,
        extension_frame_ws_url,
        frame_callback=broadcast_frame,
        event_callback=broadcast_ws_message,
    )
    bridge.write_endpoint()

    http_task = asyncio.create_task(run_http_discovery(bridge, host, discovery_port))

    async def handle_extension_websocket(websocket: Any) -> None:
        stream_authenticated = False
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
                        await websocket.send(json.dumps(ack))
                    elif data.get("type") == "frame_error":
                        bridge.last_frame_error = str(
                            data.get("error", "extension stream error")
                        )
                        bridge.write_endpoint()
                        bridge._schedule_stream_event(
                            {
                                "type": "frame_error",
                                "error": bridge.last_frame_error,
                            }
                        )
                elif isinstance(message, bytes) and stream_authenticated:
                    await bridge.handle_extension_binary(message)
        except Exception as exc:  # noqa: BLE001
            print(f"extension frame websocket closed: {exc}", file=sys.stderr)

    async def handle_desktop_websocket(websocket: Any) -> None:
        clients.add(websocket)
        if bridge._last_frame is not None:
            try:
                await websocket.send(json.dumps(bridge._last_frame))
            except Exception:
                clients.discard(websocket)
        try:
            async for message in websocket:
                try:
                    data = json.loads(message)
                except json.JSONDecodeError:
                    continue
                if data.get("type") == "frame" and data.get("data"):
                    frame_out: dict[str, Any] = {
                        "type": "frame",
                        "data": data.get("data", ""),
                        "url": str(data.get("url", bridge.page_url)),
                    }
                    raw_tab = data.get("tab_id")
                    if raw_tab is not None:
                        try:
                            frame_out["tab_id"] = int(raw_tab)
                        except (TypeError, ValueError):
                            pass
                    await bridge._emit_frame(frame_out)
                    continue
                if "cmd" in data:
                    reply = await bridge.forward_command(data)
                    await websocket.send(json.dumps(reply))
        finally:
            clients.discard(websocket)

    async def connection_handler(websocket: Any) -> None:
        path = _websocket_path(websocket)
        if path == EXTENSION_FRAME_WS_PATH:
            await handle_extension_websocket(websocket)
            return
        await handle_desktop_websocket(websocket)

    async with websockets.serve(connection_handler, host, port) as server:
        actual_port = server.sockets[0].getsockname()[1]
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
        "--no-preview-stream",
        action="store_true",
        help="Disable JPEG preview loop (CI/headless replay)",
    )
    args = parser.parse_args()
    project_root = Path(args.project_root) if args.project_root else None

    try:
        if args.mode == "chrome":
            if project_root is None:
                print("chrome mode requires --project-root", file=sys.stderr)
                sys.exit(1)
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
