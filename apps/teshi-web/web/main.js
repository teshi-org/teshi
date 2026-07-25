import init, { run } from "./pkg/teshi_web.js";

const loading = document.getElementById("loading");
const loadingLabel = document.getElementById("loading-label");
const loadingDetail = document.getElementById("loading-detail");
const loadingBarTrack = document.getElementById("loading-bar-track");
const loadingBarFill = document.getElementById("loading-bar-fill");

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
    ratio: 1,
    detail: "100%",
  });
  run();
  loading?.setAttribute("hidden", "");
} catch (err) {
  console.error(err);
  if (loading) {
    loading.classList.add("error");
    loading.textContent = `Failed to start teshi-web: ${err}`;
  }
}
