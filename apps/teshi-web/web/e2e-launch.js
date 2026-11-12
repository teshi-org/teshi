/**
 * Local Web E2E session bootstrap.
 *
 * Loaded by `main.js` only for `?e2e=1` on a loopback host. The hosted Pages
 * deployment copies `index.html` and `main.js` alone, so this module (and its
 * legacy REST session call) never ships to production.
 *
 * @param {URL} url Current page URL; must already be gated to the local harness.
 */
export async function ensureLocalE2eLaunchFragment(url) {
  const fragment = new URLSearchParams(url.hash.replace(/^#/, ""));
  const existingToken = fragment.get("token") || "";
  const port = fragment.get("port") || url.port;
  if (port && /^[1-9][0-9]{0,4}$/.test(port) && /^[A-Za-z0-9_-]{16,256}$/.test(existingToken)) {
    return;
  }

  const response = await fetch("/api/v1/sessions", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ role: "hosted_web_ui" }),
  });
  if (!response.ok) {
    throw new Error(`Failed to create local E2E session (${response.status})`);
  }
  const session = await response.json();
  const token = typeof session.token === "string" ? session.token : "";
  if (!port || !/^[1-9][0-9]{0,4}$/.test(port) || !/^[A-Za-z0-9_-]{16,256}$/.test(token)) {
    throw new Error("Local E2E session response is missing a valid port or token");
  }
  url.hash = `port=${encodeURIComponent(port)}&token=${encodeURIComponent(token)}`;
  window.history.replaceState(null, "", url.toString());
}
