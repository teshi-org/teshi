import init, { run } from "./pkg/teshi_web.js";

const STARTUP_TIMEOUT_MS = 45_000;

const loading = document.getElementById("loading");
const loadingLabel = document.getElementById("loading-label");
const loadingDetail = document.getElementById("loading-detail");
const loadingBarTrack = document.getElementById("loading-bar-track");
const loadingBarFill = document.getElementById("loading-bar-fill");
const loadingErrorMessage = document.getElementById("loading-error-message");
const loadingErrorHint = document.getElementById("loading-error-hint");

let settled = false;
/** @type {ReturnType<typeof setTimeout> | null} */
let startupTimer = null;

/**
 * @param {{ label: string, ratio?: number | null, detail?: string, indeterminate?: boolean }} opts
 */
function setProgress({ label, ratio = null, detail = "", indeterminate = false }) {
  if (loadingLabel) {
    loadingLabel.textContent = label;
  }
  if (loadingDetail) {
    loadingDetail.textContent = detail;
  }
  if (!loadingBarTrack || !loadingBarFill) {
    return;
  }

  if (indeterminate || ratio == null || !Number.isFinite(ratio)) {
    loadingBarTrack.classList.add("indeterminate");
    loadingBarTrack.removeAttribute("aria-valuenow");
    loadingBarFill.style.width = "";
    return;
  }

  const clamped = Math.max(0, Math.min(1, ratio));
  const percent = Math.round(clamped * 100);
  loadingBarTrack.classList.remove("indeterminate");
  loadingBarTrack.setAttribute("aria-valuenow", String(percent));
  loadingBarFill.style.width = `${percent}%`;
}

/**
 * @returns {string}
 */
function insecureContextHint() {
  if (window.isSecureContext) {
    return "";
  }
  return (
    "This page is not a secure context (plain HTTP on a LAN IP is not treated like localhost). " +
    "WebGPU may be unavailable. Serve over HTTPS, open via http://127.0.0.1 on this device, " +
    "or add this origin to chrome://flags/#unsafely-treat-insecure-origin-as-secure."
  );
}

/**
 * @param {unknown} err
 * @returns {string}
 */
function formatError(err) {
  if (err instanceof Error) {
    return err.message || String(err);
  }
  if (typeof err === "string") {
    return err;
  }
  try {
    return String(err);
  } catch {
    return "Unknown error";
  }
}

/**
 * Show a persistent error on the loading overlay (keeps panel structure).
 *
 * @param {string} message
 * @param {string} [hint]
 */
function showError(message, hint = "") {
  if (settled) {
    return;
  }
  settled = true;
  if (startupTimer != null) {
    clearTimeout(startupTimer);
    startupTimer = null;
  }

  console.error(message, hint || undefined);
  if (!loading) {
    return;
  }

  loading.classList.add("error");
  loading.removeAttribute("hidden");
  if (loadingErrorMessage) {
    loadingErrorMessage.textContent = message;
  }
  if (loadingErrorHint) {
    loadingErrorHint.textContent = hint;
  }
}

function hideLoading() {
  if (settled) {
    return;
  }
  settled = true;
  if (startupTimer != null) {
    clearTimeout(startupTimer);
    startupTimer = null;
  }
  loading?.setAttribute("hidden", "");
}

/**
 * @param {number} bytes
 * @returns {string}
 */
function formatBytes(bytes) {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Fetch a WASM binary while reporting download progress.
 *
 * @param {string | URL} url
 * @param {(received: number, total: number | null) => void} onProgress
 * @returns {Promise<Uint8Array>}
 */
async function fetchWasmWithProgress(url, onProgress) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Failed to download WASM (${response.status} ${response.statusText})`);
  }

  const contentLength = response.headers.get("Content-Length");
  const total = contentLength ? Number(contentLength) : null;
  if (!response.body) {
    const buffer = await response.arrayBuffer();
    onProgress(buffer.byteLength, total ?? buffer.byteLength);
    return new Uint8Array(buffer);
  }

  const reader = response.body.getReader();
  const chunks = [];
  let received = 0;

  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    chunks.push(value);
    received += value.byteLength;
    onProgress(received, Number.isFinite(total) && total > 0 ? total : null);
  }

  const bytes = new Uint8Array(received);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

try {
  setProgress({
    label: "Downloading…",
    ratio: 0,
    detail: "0%",
  });

  const wasmUrl = new URL("./pkg/teshi_web_bg.wasm", import.meta.url);
  const bytes = await fetchWasmWithProgress(wasmUrl, (received, total) => {
    if (total != null && total > 0) {
      const ratio = received / total;
      setProgress({
        label: "Downloading…",
        ratio,
        detail: `${Math.round(ratio * 100)}% · ${formatBytes(received)} / ${formatBytes(total)}`,
      });
      return;
    }
    setProgress({
      label: "Downloading…",
      indeterminate: true,
      detail: formatBytes(received),
    });
  });

  setProgress({
    label: "Compiling…",
    ratio: 1,
    detail: "100%",
  });
  await init({ module_or_path: bytes });

  setProgress({
    label: "Starting…",
    indeterminate: true,
    detail: "Initializing GPU…",
  });

  startupTimer = setTimeout(() => {
    showError(
      "Timed out while starting the UI. The GPU may be unavailable or still initializing.",
      insecureContextHint(),
    );
  }, STARTUP_TIMEOUT_MS);

  run(
    () => {
      hideLoading();
    },
    /** @param {string} message */
    (message) => {
      const hint = insecureContextHint();
      showError(formatError(message), hint);
    },
  );
} catch (err) {
  showError(`Failed to start teshi-web: ${formatError(err)}`, insecureContextHint());
}
