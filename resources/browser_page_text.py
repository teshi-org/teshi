"""Rendered text collector shared in behavior with the Chrome extension.

Keep PAGE_TEXT_JS synchronized with extension/teshi-bridge/page-text.js.
The browser regression suite verifies equality and rendered behavior.
"""

PAGE_TEXT_JS = r'''
/** Read rendered DOM content without changing the page or exposing password values. */
function collectPageText({ selector = null, offset = 0, limit = 35000 } = {}) {
  if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(limit) || limit < 1 || limit > 100000) {
    return { ok: false, code: "invalid_arguments", error: "Invalid text offset or limit" };
  }
  let root;
  try { root = selector ? document.querySelector(selector) : document.body; }
  catch { return { ok: false, code: "invalid_selector", error: "Invalid text scope selector" }; }
  if (!root) return { ok: false, code: "element_not_found", error: "Text scope did not resolve" };
  for (let ancestor = root.parentElement; ancestor; ancestor = ancestor.parentElement) {
    const style = ancestor.ownerDocument.defaultView.getComputedStyle(ancestor);
    if (ancestor.hidden || style.display === "none" || style.contentVisibility === "hidden" || Number(style.opacity) === 0) {
      return { ok: true, text: "", offset, limit, total_chars: 0, returned_chars: 0,
        truncated: false, next_offset: null, warnings: [] };
    }
  }
  const parts = [];
  const warnings = new Set();
  const blocks = new Set(["DIV", "P", "H1", "H2", "H3", "H4", "H5", "H6", "LI", "TR", "SECTION", "ARTICLE", "HEADER", "FOOTER", "NAV", "BLOCKQUOTE", "PRE", "BR", "HR", "DT", "DD", "FIGCAPTION", "DETAILS", "SUMMARY"]);
  let visited = 0;
  function visit(node) {
    if (++visited > 100000) { warnings.add("dom_node_limit"); return; }
    if (node.nodeType === 3) { parts.push(node.textContent || ""); return; }
    if (node.nodeType !== 1) return;
    const tag = node.tagName;
    if (["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "META", "LINK"].includes(tag)) return;
    const style = node.ownerDocument.defaultView.getComputedStyle(node);
    if (node.hidden || style.display === "none" || style.contentVisibility === "hidden" || Number(style.opacity) === 0) return;
    if (blocks.has(tag)) parts.push("\n");
    const visible = style.visibility !== "hidden" && style.visibility !== "collapse";
    if (visible && ["INPUT", "TEXTAREA", "SELECT"].includes(tag)) {
      if (tag === "INPUT" && node.type === "hidden") return;
      const label = node.getAttribute("aria-label") || node.labels?.[0]?.textContent || node.getAttribute("placeholder") || node.getAttribute("name") || "";
      parts.push(`\n[${tag}${tag === "INPUT" ? ` type=${node.type}` : ""}${label ? ` ${label}` : ""}${node.disabled ? " disabled" : ""}${node.checked ? " checked" : ""}]`);
      if (node.type !== "password") {
        parts.push(tag === "SELECT" ? Array.from(node.selectedOptions || [], option => option.textContent).join(" ") : node.value || "");
      }
      parts.push("\n");
      return;
    }
    if (visible && tag === "BUTTON" && node.disabled) parts.push("[DISABLED] ");
    if (tag === "IFRAME" && visible) {
      try {
        if (node.contentDocument?.body) visit(node.contentDocument.body);
        else warnings.add("inaccessible_frame");
      } catch { warnings.add("inaccessible_frame"); }
    } else if (tag === "SLOT" && node.assignedNodes?.().length) {
      for (const child of node.assignedNodes()) visit(child);
    } else {
      for (const child of (node.shadowRoot || node).childNodes) {
        // Hidden text is omitted, while descendants may explicitly restore visibility.
        if (visible || child.nodeType !== 3) visit(child);
        if (visited > 100000) break;
      }
    }
    if (blocks.has(tag)) parts.push("\n");
  }
  visit(root);
  const characters = Array.from(parts.join("").split(/\r?\n/).map(line => line.replace(/\s+/g, " ").trim()).filter(Boolean).join("\n"));
  const end = Math.min(characters.length, offset + limit);
  return { ok: true, text: characters.slice(offset, end).join(""), offset, limit,
    total_chars: characters.length, returned_chars: Math.max(0, end - offset),
    truncated: end < characters.length || warnings.has("dom_node_limit"),
    next_offset: end < characters.length ? end : null, warnings: [...warnings] };
}

'''
