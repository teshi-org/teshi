import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { PanelCollapseButton } from "./PanelCollapseButton";

// Use localhost so Tauri CSP connect-src allows discovery polling from the webview.
const CHROME_DISCOVERY_URL = "http://127.0.0.1:17373/v1/bridge";
const CHROME_ACTIVATE_TAB_URL = "http://127.0.0.1:17373/v1/bridge/activate_tab";
const CHROME_CAPTURE_NOW_URL = "http://127.0.0.1:17373/v1/bridge/capture_now";

interface ChromeWindowTab {
  id: number;
  title: string;
  url: string;
  active: boolean;
  favIconUrl?: string;
  debuggable?: boolean;
}

interface ChromeBridgeInfo {
  page_url?: string;
  title?: string;
  extension_connected?: boolean;
  active_tab_id?: number | null;
  tabs?: ChromeWindowTab[];
  last_frame_error?: string;
  last_frame_age_ms?: number | null;
  project_root?: string;
}

/** Chrome screencast is repaint-driven; idle hint after no new frames. */
const CHROME_PREVIEW_IDLE_MS = 1500;
/** No preview frames at all while extension heartbeats (likely WS/screencast issue). */
const CHROME_STREAM_DISCONNECTED_MS = 5000;
const EMBEDDED_STREAM_STALL_MS = 4000;

interface Props {
  wsUrl: string | null;
  running: boolean;
  mode: "embedded" | "chrome" | "winapp" | null;
  error: string | null;
  hint: string | null;
  fullscreen: boolean;
  gherkinCollapsed?: boolean;
  filesCollapsed?: boolean;
  onToggleGherkin?: () => void;
  onToggleFiles?: () => void;
  onConnectChrome: () => void;
  onConnectWinApp: () => void;
  onStartEmbedded: () => void;
  onStop: () => void;
  onToggleFullscreen: () => void;
}

const EMBEDDED_SOURCE_WIDTH = 1920;
const EMBEDDED_SOURCE_HEIGHT = 1080;
const ZOOM_STEPS = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4];
const DEFAULT_ZOOM = 1;

interface SourceSize {
  width: number;
  height: number;
}

interface FitSize {
  fitW: number;
  fitH: number;
}

function effectiveSourceSize(
  source: SourceSize,
  containerW: number,
  containerH: number,
  chromeMode: boolean,
): SourceSize {
  if (source.width > 0 && source.height > 0) {
    return source;
  }
  if (containerW > 0 && containerH > 0) {
    return { width: containerW, height: containerH };
  }
  if (chromeMode) {
    return { width: 4, height: 3 };
  }
  return { width: EMBEDDED_SOURCE_WIDTH, height: EMBEDDED_SOURCE_HEIGHT };
}

function computeFitSize(
  containerW: number,
  containerH: number,
  source: SourceSize,
  chromeMode: boolean,
): FitSize {
  if (containerW <= 0 || containerH <= 0) {
    return { fitW: 0, fitH: 0 };
  }
  const resolved = effectiveSourceSize(source, containerW, containerH, chromeMode);
  const sourceAspect = resolved.width / resolved.height;
  const containerAspect = containerW / containerH;
  if (containerAspect > sourceAspect) {
    const fitH = containerH;
    return { fitW: fitH * sourceAspect, fitH };
  }
  const fitW = containerW;
  return { fitW, fitH: fitW / sourceAspect };
}

function closestZoomIndex(zoom: number): number {
  let bestIdx = 0;
  let bestDiff = Infinity;
  for (let i = 0; i < ZOOM_STEPS.length; i += 1) {
    const diff = Math.abs(ZOOM_STEPS[i] - zoom);
    if (diff < bestDiff) {
      bestDiff = diff;
      bestIdx = i;
    }
  }
  return bestIdx;
}

function stepZoomLevel(current: number, direction: 1 | -1): number {
  const idx = closestZoomIndex(current);
  const nextIdx = Math.max(0, Math.min(ZOOM_STEPS.length - 1, idx + direction));
  return ZOOM_STEPS[nextIdx];
}

interface ZoomAnchor {
  clientX: number;
  clientY: number;
  relX: number;
  relY: number;
}

function FullscreenIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
      <path
        fill="currentColor"
        d="M1.5 1h4v1.5H3.12L6 5.38 5.38 6 2.5 3.12V5.5H1V1.5zm13 0v4.5h-1.5V3.12L10.62 6 10 5.38 12.88 2.5H10.5V1h4zm0 13h-4v-1.5h2.38L10 10.62 10.62 10 13.5 12.88V10.5H15v3.5zm-13 0v-4.5h1.5v2.38L6 10.62 6.62 11 3.74 13.88V11.5H1v2.5z"
      />
    </svg>
  );
}

function ExitFullscreenIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
      <path
        fill="currentColor"
        d="M3.5 3h2.5v1.5H4.62L7.5 7.38 6.88 8 4 5.12V7.5H2.5V3zm9 0v4.5H11V5.12L8.12 8 7.5 7.38 10.38 4.5H8V3h4.5zm-9 9H2.5V7.5H4v2.38L6.88 7 7.5 7.62 4.62 10.5H7v1.5zm9 0H11v-1.5h2.38L8.5 8.62 9.12 8 12 10.88V8.5h1.5V12z"
      />
    </svg>
  );
}

/** Prefix scheme when the user omits http(s):// */
function normalizeUrl(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return trimmed;
  if (/^[a-z][a-z0-9+.-]*:/i.test(trimmed)) {
    return trimmed;
  }
  return `https://${trimmed}`;
}

export function BrowserPanel({
  wsUrl,
  running,
  mode,
  error,
  hint,
  fullscreen,
  gherkinCollapsed = false,
  filesCollapsed = false,
  onToggleGherkin,
  onToggleFiles,
  onConnectChrome,
  onConnectWinApp,
  onStartEmbedded,
  onStop,
  onToggleFullscreen,
}: Props) {
  const isEmbedded = running && mode === "embedded";
  const isChrome = running && mode === "chrome";
  const isWinApp = running && mode === "winapp";
  const imgRef = useRef<HTMLImageElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const scalerRef = useRef<HTMLDivElement>(null);
  const socketRef = useRef<WebSocket | null>(null);
  const addressFocusedRef = useRef(false);
  const lastMouseRef = useRef<{ x: number; y: number; inside: boolean }>({
    x: 0,
    y: 0,
    inside: false,
  });
  const zoomAnchorRef = useRef<ZoomAnchor | null>(null);
  const dragRef = useRef<{
    active: boolean;
    startX: number;
    startY: number;
    scrollLeft: number;
    scrollTop: number;
  } | null>(null);
  const [fps, setFps] = useState(0);
  const framesRef = useRef(0);
  const [pageUrl, setPageUrl] = useState("about:blank");
  const [addressInput, setAddressInput] = useState("about:blank");
  const [zoom, setZoom] = useState(DEFAULT_ZOOM);
  const [sourceSize, setSourceSize] = useState<SourceSize>({
    width: EMBEDDED_SOURCE_WIDTH,
    height: EMBEDDED_SOURCE_HEIGHT,
  });
  const [fitSize, setFitSize] = useState<FitSize>({ fitW: 0, fitH: 0 });
  const [dragging, setDragging] = useState(false);
  const [chromeInfo, setChromeInfo] = useState<ChromeBridgeInfo | null>(null);
  const [chromePollError, setChromePollError] = useState<string | null>(null);
  const [activatingTabId, setActivatingTabId] = useState<number | null>(null);
  const [frameSrc, setFrameSrc] = useState<string | null>(null);
  const [streamError, setStreamError] = useState<string | null>(null);
  const [lastFrameAt, setLastFrameAt] = useState(0);
  const [streamHealthTick, setStreamHealthTick] = useState(0);
  const lastStreamTabIdRef = useRef<number | null>(null);
  const activatingTabIdRef = useRef<number | null>(null);
  const chromeActiveTabIdRef = useRef<number | null>(null);
  const activateGraceUntilRef = useRef(0);

  const chromeConnected = isChrome && Boolean(chromeInfo?.extension_connected);
  const showChromeWaiting = isChrome && !chromeConnected;
  const showViewport = isEmbedded || isWinApp || chromeConnected;
  const showStreamLoading = isChrome && activatingTabId !== null;
  useEffect(() => {
    if (!isChrome || !chromeConnected) {
      return;
    }
    const timer = setInterval(() => setStreamHealthTick((t) => t + 1), 1000);
    return () => clearInterval(timer);
  }, [isChrome, chromeConnected]);

  const chromeHasPreviewFrame = Boolean(frameSrc) || lastFrameAt > 0;

  const streamStalled = (() => {
    void streamHealthTick;
    if (isChrome && !chromeConnected) {
      return false;
    }
    if (!isChrome && !wsUrl) {
      return false;
    }
    const stallMs = isChrome
      ? chromeHasPreviewFrame
        ? CHROME_PREVIEW_IDLE_MS
        : CHROME_STREAM_DISCONNECTED_MS
      : EMBEDDED_STREAM_STALL_MS;
    if (lastFrameAt > 0) {
      return Date.now() - lastFrameAt > stallMs;
    }
    if (isChrome) {
      const age = chromeInfo?.last_frame_age_ms;
      return age == null || age > stallMs;
    }
    return false;
  })();

  const chromePreviewIdle =
    isChrome &&
    chromeConnected &&
    streamStalled &&
    chromeHasPreviewFrame &&
    !streamError &&
    !showStreamLoading;

  const chromeStreamDisconnected =
    isChrome &&
    chromeConnected &&
    streamStalled &&
    !chromeHasPreviewFrame &&
    !streamError &&
    !showStreamLoading;

  useEffect(() => {
    activatingTabIdRef.current = activatingTabId;
  }, [activatingTabId]);

  useEffect(() => {
    chromeActiveTabIdRef.current = chromeInfo?.active_tab_id ?? null;
  }, [chromeInfo?.active_tab_id]);

  const getZoomAnchorPoint = useCallback((): { x: number; y: number } => {
    const wrap = wrapRef.current;
    if (!wrap) {
      return { x: 0, y: 0 };
    }
    const rect = wrap.getBoundingClientRect();
    if (lastMouseRef.current.inside) {
      return { x: lastMouseRef.current.x, y: lastMouseRef.current.y };
    }
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  }, []);

  const zoomAtPoint = useCallback((direction: 1 | -1, clientX: number, clientY: number) => {
    const scaler = scalerRef.current;
    if (!scaler) return;

    const scalerRect = scaler.getBoundingClientRect();
    if (scalerRect.width <= 0 || scalerRect.height <= 0) return;

    zoomAnchorRef.current = {
      clientX,
      clientY,
      relX: (clientX - scalerRect.left) / scalerRect.width,
      relY: (clientY - scalerRect.top) / scalerRect.height,
    };
    setZoom((current) => stepZoomLevel(current, direction));
  }, []);

  const stepZoom = useCallback(
    (direction: 1 | -1) => {
      const { x, y } = getZoomAnchorPoint();
      zoomAtPoint(direction, x, y);
    },
    [getZoomAnchorPoint, zoomAtPoint],
  );

  const resetZoom = useCallback(() => {
    zoomAnchorRef.current = null;
    setZoom(DEFAULT_ZOOM);
    const wrap = wrapRef.current;
    if (wrap) {
      wrap.scrollLeft = 0;
      wrap.scrollTop = 0;
    }
  }, []);

  const navigateTo = useCallback((raw: string) => {
    const url = normalizeUrl(raw);
    if (!url || !socketRef.current || socketRef.current.readyState !== WebSocket.OPEN) {
      return;
    }
    socketRef.current.send(JSON.stringify({ cmd: "navigate", url }));
    setAddressInput(url);
  }, []);

  const requestChromeCapture = useCallback(async (projectRoot: string) => {
    const res = await fetch(CHROME_CAPTURE_NOW_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ project_root: projectRoot }),
    });
    if (!res.ok) {
      throw new Error(`Capture request failed (HTTP ${res.status})`);
    }
  }, []);

  const activateChromeTab = useCallback(
    async (tabId: number) => {
      const projectRoot = chromeInfo?.project_root;
      if (!projectRoot) {
        return;
      }
      setActivatingTabId(tabId);
      activateGraceUntilRef.current = Date.now() + 8000;
      setStreamError(null);
      try {
        const res = await fetch(CHROME_ACTIVATE_TAB_URL, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ project_root: projectRoot, tab_id: tabId }),
        });
        if (!res.ok) {
          setStreamError(`Could not request tab activation (HTTP ${res.status})`);
          return;
        }
        await requestChromeCapture(projectRoot);
        window.setTimeout(() => {
          void requestChromeCapture(projectRoot).catch(() => {
            /* retry once if the extension heartbeat was busy */
          });
        }, 1600);
      } catch (err) {
        setStreamError(err instanceof Error ? err.message : String(err));
      }
    },
    [chromeInfo?.project_root, requestChromeCapture],
  );

  const refreshChromeStream = useCallback(async () => {
    const projectRoot = chromeInfo?.project_root;
    if (!projectRoot) {
      return;
    }
    setStreamError(null);
    try {
      await requestChromeCapture(projectRoot);
    } catch (err) {
      setStreamError(err instanceof Error ? err.message : String(err));
    }
  }, [chromeInfo?.project_root, requestChromeCapture]);

  useEffect(() => {
    if (!showViewport) {
      setZoom(DEFAULT_ZOOM);
    }
  }, [showViewport]);

  useEffect(() => {
    if (isEmbedded) {
      setSourceSize({
        width: EMBEDDED_SOURCE_WIDTH,
        height: EMBEDDED_SOURCE_HEIGHT,
      });
      return;
    }
    if (isChrome || isWinApp) {
      setSourceSize({ width: 0, height: 0 });
    }
  }, [isEmbedded, isChrome, isWinApp, wsUrl]);

  useEffect(() => {
    if (!isChrome) {
      setChromeInfo(null);
      setChromePollError(null);
      setActivatingTabId(null);
      return;
    }
    const poll = async () => {
      try {
        const res = await fetch(CHROME_DISCOVERY_URL);
        if (!res.ok) {
          setChromePollError(`Bridge discovery failed (${res.status})`);
          return;
        }
        const data = (await res.json()) as ChromeBridgeInfo;
        setChromeInfo(data);
        setChromePollError(null);
        if (data.extension_connected && typeof data.page_url === "string" && data.page_url) {
          setPageUrl(data.page_url);
          if (!addressFocusedRef.current) {
            setAddressInput(data.page_url);
          }
        }
      } catch {
        setChromeInfo(null);
        setChromePollError(
          "Cannot reach local bridge (127.0.0.1:17373). Reinstall the latest MSI and click Connect Chrome again.",
        );
      }
    };
    void poll();
    const timer = setInterval(() => void poll(), 2000);
    return () => clearInterval(timer);
  }, [isChrome]);

  useEffect(() => {
    if (!isChrome) {
      return;
    }
    const active = chromeInfo?.active_tab_id;
    if (active != null && activatingTabId === active) {
      setActivatingTabId(null);
    }
  }, [isChrome, chromeInfo?.active_tab_id, activatingTabId]);

  useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap || !showViewport) return;

    const updateFit = () => {
      setFitSize(
        computeFitSize(wrap.clientWidth, wrap.clientHeight, sourceSize, isChrome),
      );
    };

    updateFit();
    const observer = new ResizeObserver(updateFit);
    observer.observe(wrap);
    return () => observer.disconnect();
  }, [showViewport, fullscreen, sourceSize, isChrome]);

  useLayoutEffect(() => {
    const anchor = zoomAnchorRef.current;
    if (!anchor) return;
    zoomAnchorRef.current = null;

    const wrap = wrapRef.current;
    const scaler = scalerRef.current;
    if (!wrap || !scaler) return;

    const scalerRect = scaler.getBoundingClientRect();
    const currentPointX = scalerRect.left + anchor.relX * scalerRect.width;
    const currentPointY = scalerRect.top + anchor.relY * scalerRect.height;

    wrap.scrollLeft -= anchor.clientX - currentPointX;
    wrap.scrollTop -= anchor.clientY - currentPointY;
  }, [zoom, fitSize]);

  useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap || !showViewport) return;

    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      zoomAtPoint(e.deltaY < 0 ? 1 : -1, e.clientX, e.clientY);
    };

    wrap.addEventListener("wheel", onWheel, { passive: false });
    return () => wrap.removeEventListener("wheel", onWheel);
  }, [showViewport, zoomAtPoint]);

  useEffect(() => {
    if (!dragging) return;

    const onMouseMove = (e: MouseEvent) => {
      const drag = dragRef.current;
      const wrap = wrapRef.current;
      if (!drag?.active || !wrap) return;
      wrap.scrollLeft = drag.scrollLeft - (e.clientX - drag.startX);
      wrap.scrollTop = drag.scrollTop - (e.clientY - drag.startY);
    };

    const endDrag = () => {
      if (dragRef.current) {
        dragRef.current.active = false;
      }
      setDragging(false);
    };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", endDrag);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", endDrag);
    };
  }, [dragging]);

  const applyFrameMessage = useCallback((msg: Record<string, unknown>) => {
    if (msg.type !== "frame" || typeof msg.data !== "string" || !msg.data) {
      return;
    }
    const frameTabId =
      typeof msg.tab_id === "number" ? msg.tab_id : Number(msg.tab_id);
    if (Number.isFinite(frameTabId)) {
      lastStreamTabIdRef.current = frameTabId;
      const pending = activatingTabIdRef.current;
      const active = chromeActiveTabIdRef.current;
      if (pending === frameTabId || active === frameTabId) {
        setActivatingTabId(null);
      }
    }
    const src = `data:image/jpeg;base64,${msg.data}`;
    setLastFrameAt(Date.now());
    setStreamError(null);
    const preload = new Image();
    preload.onload = () => {
      if (preload.naturalWidth > 0 && preload.naturalHeight > 0) {
        setSourceSize({ width: preload.naturalWidth, height: preload.naturalHeight });
      }
      setFrameSrc(src);
    };
    preload.onerror = () => {
      setFrameSrc(src);
    };
    preload.src = src;
    framesRef.current += 1;
    if (typeof msg.url === "string" && msg.url) {
      setPageUrl(msg.url);
      if (!addressFocusedRef.current) {
        setAddressInput(msg.url);
      }
    }
  }, []);

  useEffect(() => {
    if (!wsUrl || !running) {
      socketRef.current = null;
      setPageUrl("about:blank");
      setAddressInput("about:blank");
      setFrameSrc(null);
      setStreamError(null);
      setLastFrameAt(0);
      return;
    }
    const socket = new WebSocket(wsUrl);
    socketRef.current = socket;
    socket.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data as string) as Record<string, unknown>;
        if (msg.type === "frame_error") {
          const raw =
            typeof msg.error === "string" ? msg.error : "Screenshot stream failed";
          const err = raw.replace(/^(Error:\s*)+/i, "").trim();
          setStreamError(err);
          return;
        }
        applyFrameMessage(msg);
      } catch {
        /* ignore malformed frames */
      }
    };
    const timer = setInterval(() => {
      setFps(framesRef.current);
      framesRef.current = 0;
    }, 1000);
    return () => {
      socket.close();
      socketRef.current = null;
      clearInterval(timer);
    };
  }, [wsUrl, running, applyFrameMessage]);

  useEffect(() => {
    if (!isChrome || !chromeInfo?.extension_connected) {
      return;
    }
    if (Date.now() < activateGraceUntilRef.current) {
      return;
    }
    const err = chromeInfo.last_frame_error?.trim();
    if (err) {
      setStreamError(err);
    } else if (!activatingTabId) {
      setStreamError(null);
    }
  }, [isChrome, chromeInfo?.extension_connected, chromeInfo?.last_frame_error, activatingTabId]);

  const onFrameLoad = useCallback(() => {
    const img = imgRef.current;
    if (!img || img.naturalWidth <= 0 || img.naturalHeight <= 0) {
      return;
    }
    setSourceSize({ width: img.naturalWidth, height: img.naturalHeight });
  }, []);

  const onAddressKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      navigateTo(addressInput);
    }
  };

  const onViewportKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!e.ctrlKey) return;
    if (e.key === "=" || e.key === "+") {
      e.preventDefault();
      e.stopPropagation();
      stepZoom(1);
    } else if (e.key === "-") {
      e.preventDefault();
      e.stopPropagation();
      stepZoom(-1);
    } else if (e.key === "0") {
      e.preventDefault();
      e.stopPropagation();
      resetZoom();
    }
  };

  const onViewportMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    lastMouseRef.current = { x: e.clientX, y: e.clientY, inside: true };
  };

  const onViewportMouseLeave = () => {
    lastMouseRef.current.inside = false;
  };

  const onViewportMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.button !== 0 || zoom <= 1) return;
    const wrap = wrapRef.current;
    if (!wrap) return;
    e.preventDefault();
    wrap.focus();
    dragRef.current = {
      active: true,
      startX: e.clientX,
      startY: e.clientY,
      scrollLeft: wrap.scrollLeft,
      scrollTop: wrap.scrollTop,
    };
    setDragging(true);
  };

  const displayW = fitSize.fitW * zoom;
  const displayH = fitSize.fitH * zoom;
  const zoomPercent = Math.round(zoom * 100);

  const chromeTabs = chromeInfo?.tabs ?? [];
  const chromeActiveTabId =
    activatingTabId ?? chromeInfo?.active_tab_id ?? null;

  const statusTitle = running
    ? isChrome
      ? `Browser running (chrome, extension ${chromeConnected ? "connected" : "waiting"})`
      : isWinApp
        ? "WinUI3 app bridge running"
        : `Browser running (${mode ?? "unknown"})`
    : "Browser stopped";

  const zoomControls = (
    <div className="browser-zoom-controls">
      <button
        type="button"
        className="browser-zoom-btn"
        onClick={() => stepZoom(-1)}
        disabled={zoom <= ZOOM_STEPS[0]}
        aria-label="Zoom out"
        title="Zoom out (Ctrl+-)"
      >
        −
      </button>
      <button
        type="button"
        className="browser-zoom-label"
        onClick={resetZoom}
        aria-live="polite"
        title="Reset zoom (Ctrl+0)"
      >
        {zoomPercent}%
      </button>
      <button
        type="button"
        className="browser-zoom-btn"
        onClick={() => stepZoom(1)}
        disabled={zoom >= ZOOM_STEPS[ZOOM_STEPS.length - 1]}
        aria-label="Zoom in"
        title="Zoom in (Ctrl+=)"
      >
        +
      </button>
      <button
        type="button"
        className="browser-zoom-fit-btn"
        onClick={resetZoom}
        title="Fit to panel (Ctrl+0)"
      >
        Fit
      </button>
    </div>
  );

  return (
    <section className="panel browser-panel">
      <header className="panel-header panel-header--browser">
        {!fullscreen && gherkinCollapsed && onToggleGherkin && (
          <PanelCollapseButton
            side="left"
            collapsed
            panelLabel="Gherkin"
            onToggle={onToggleGherkin}
          />
        )}
        <span className="panel-header-title">
          Target {running ? "• live" : "• stopped"}
          {running && mode && (
            <span className="browser-mode-label"> ({mode})</span>
          )}
          {showViewport && <span className="fps-label">{fps} fps</span>}
        </span>
        <div className="panel-header-actions">
          {!running ? (
            <>
              <button
                type="button"
                className="panel-header-btn panel-header-btn--primary"
                onClick={onConnectChrome}
              >
                Connect Chrome
              </button>
              <button type="button" className="panel-header-btn" onClick={onConnectWinApp}>
                Connect WinUI3 App
              </button>
              <button type="button" className="panel-header-btn" onClick={onStartEmbedded}>
                Start Embedded
              </button>
            </>
          ) : (
            <button type="button" className="panel-header-btn" onClick={onStop}>
              Disconnect
            </button>
          )}
          <span
            className={`status-dot ${running ? "on" : "off"}`}
            title={statusTitle}
          />
          <button
            type="button"
            className="panel-header-icon-btn"
            onClick={onToggleFullscreen}
            title={fullscreen ? "Exit fullscreen" : "Fullscreen"}
            aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
          >
            {fullscreen ? <ExitFullscreenIcon /> : <FullscreenIcon />}
          </button>
        </div>
        {!fullscreen && filesCollapsed && onToggleFiles && (
          <PanelCollapseButton
            side="right"
            collapsed
            panelLabel="Files"
            onToggle={onToggleFiles}
          />
        )}
      </header>
      <div className="panel-body browser-body">
        {!running && !error && (
          <div className="browser-placeholder">
            <p>
              <strong>Connect Chrome</strong> — record locators on your logged-in tab (load the
              unpacked extension from <code>C:\Program Files\teshi\share\teshi-bridge</code> first).
            </p>
            <p>
              <strong>Start Embedded</strong> — headless Playwright preview (1920×1080) for
              local/staging URLs.
            </p>
            <p>
              <strong>Connect WinUI3 App</strong> — attach terminal agents to a native Windows app
              through UI Automation.
            </p>
          </div>
        )}
        {error && (
          <div className="browser-error">
            <p>{error}</p>
            {hint && <code>{hint}</code>}
          </div>
        )}
        {showChromeWaiting && (
          <div className="browser-chrome-status browser-placeholder">
            {chromePollError && (
              <p className="browser-chrome-stale">{chromePollError}</p>
            )}
            <p>
              Extension: <strong className="browser-chrome-warn">waiting…</strong>
            </p>
            <p className="browser-chrome-stale">
              No heartbeat from the extension (stale after ~8s). Reload teshi-bridge on{" "}
              <code>chrome://extensions</code>, keep this Chrome tab focused, and ensure
              Connect Chrome is active in teshi.
            </p>
            <ol className="browser-chrome-hints browser-chrome-setup">
              <li>
                In <strong>Google Chrome</strong> (not only inside teshi), open{" "}
                <code>chrome://extensions</code> → enable <strong>Developer mode</strong> →{" "}
                <strong>Load unpacked</strong> → select{" "}
                <code>C:\Program Files\teshi\share\teshi-bridge</code>.
              </li>
              <li>
                Open your <strong>app under test</strong> in a normal Chrome tab (e.g. the
                site you log into).
              </li>
              <li>
                Click the <strong>teshi-bridge</strong> puzzle icon →{" "}
                <strong>Connect to teshi</strong> (wakes the extension after Connect Chrome here).
              </li>
              <li>Extension badge should show <strong>OK</strong> when linked.</li>
            </ol>
            <ul className="browser-chrome-hints">
              <li>
                After connecting, use the tab strip in the Browser panel or switch tabs in Chrome.
              </li>
              <li>Close DevTools on the target tab if attach fails.</li>
            </ul>
          </div>
        )}
        {showViewport && (
          <div className="browser-viewport">
            {isEmbedded ? (
              <form
                className="browser-address-bar"
                onSubmit={(e) => {
                  e.preventDefault();
                  navigateTo(addressInput);
                }}
              >
                <label className="visually-hidden" htmlFor="browser-address">
                  Address
                </label>
                <input
                  id="browser-address"
                  type="text"
                  className="browser-address-input"
                  value={addressInput}
                  onChange={(e) => setAddressInput(e.target.value)}
                  onFocus={() => {
                    addressFocusedRef.current = true;
                  }}
                  onBlur={() => {
                    addressFocusedRef.current = false;
                    setAddressInput(pageUrl);
                  }}
                  onKeyDown={onAddressKeyDown}
                  spellCheck={false}
                  autoComplete="off"
                  placeholder="Enter URL"
                  title={pageUrl}
                />
                <button type="submit" className="browser-go-btn">
                  Go
                </button>
                {zoomControls}
              </form>
            ) : isChrome ? (
              <>
                {chromeTabs.length > 0 && (
                  <div
                    className="browser-chrome-tabstrip"
                    role="tablist"
                    aria-label="Chrome window tabs"
                  >
                    {activatingTabId !== null && (
                      <span className="browser-chrome-tabstrip-status">Switching tab…</span>
                    )}
                    {chromeTabs.map((tab) => {
                      const isActive = chromeActiveTabId === tab.id;
                      const debuggable = tab.debuggable !== false;
                      return (
                        <button
                          key={tab.id}
                          type="button"
                          role="tab"
                          aria-selected={isActive}
                          className={`browser-chrome-tab${isActive ? " browser-chrome-tab--active" : ""}${!debuggable ? " browser-chrome-tab--disabled" : ""}`}
                          disabled={!debuggable || isActive}
                          title={
                            debuggable
                              ? tab.url || tab.title
                              : "This page cannot be debugged (e.g. chrome://)"
                          }
                          onClick={() => {
                            if (debuggable && !isActive) {
                              void activateChromeTab(tab.id);
                            }
                          }}
                        >
                          {tab.favIconUrl ? (
                            <img
                              className="browser-chrome-tab-favicon"
                              src={tab.favIconUrl}
                              alt=""
                              width={14}
                              height={14}
                            />
                          ) : (
                            <span className="browser-chrome-tab-favicon-placeholder" />
                          )}
                          <span className="browser-chrome-tab-title">
                            {tab.title || "Untitled"}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                )}
                <div className="browser-address-bar">
                  <label className="visually-hidden" htmlFor="browser-address-chrome">
                    Active tab URL
                  </label>
                  <input
                    id="browser-address-chrome"
                    type="text"
                    className="browser-address-input"
                    readOnly
                    value={pageUrl}
                    spellCheck={false}
                    autoComplete="off"
                    placeholder="Active Chrome tab URL"
                    title={pageUrl}
                  />
                  <button
                    type="button"
                    className="browser-go-btn"
                    onClick={() => void refreshChromeStream()}
                    title="Request an immediate preview frame"
                  >
                    Refresh
                  </button>
                  {zoomControls}
                </div>
              </>
            ) : (
              <div className="browser-address-bar">
                <label className="visually-hidden" htmlFor="browser-address-winapp">
                  Attached WinUI3 target
                </label>
                <input
                  id="browser-address-winapp"
                  type="text"
                  className="browser-address-input"
                  readOnly
                  value={pageUrl}
                  spellCheck={false}
                  autoComplete="off"
                  placeholder="Run `teshi winapp attach ...` in the terminal"
                  title={pageUrl}
                />
                {zoomControls}
              </div>
            )}
            <div
              ref={wrapRef}
              className={`browser-frame-wrap${isChrome ? " browser-frame-wrap--chrome" : ""}${dragging ? " browser-frame-wrap--dragging" : ""}${zoom > 1 ? " browser-frame-wrap--pannable" : ""}`}
              tabIndex={0}
              onKeyDown={onViewportKeyDown}
              onMouseMove={onViewportMouseMove}
              onMouseLeave={onViewportMouseLeave}
              onMouseDown={onViewportMouseDown}
              onClick={() => wrapRef.current?.focus()}
              aria-label="Browser stream viewport"
            >
              <div
                ref={scalerRef}
                className={`browser-frame-scaler${zoom > 1 ? " browser-frame-scaler--zoomed" : ""}`}
                style={
                  fitSize.fitW > 0 && fitSize.fitH > 0
                    ? { width: displayW, height: displayH }
                    : { width: "100%", height: "100%" }
                }
              >
                {streamError && (
                  <p className="browser-stream-error" role="status">
                    Preview stream: {streamError}
                    {isChrome &&
                      (streamError.includes("Failed to fetch") ||
                        streamError.toLowerCase().includes("stream")) && (
                        <>
                          {" "}
                          — Reload teshi-bridge, Disconnect/Connect Chrome in teshi. Discovery
                          uses port 17373; preview frames use the extension WebSocket from GET
                          /v1/bridge (extension_frame_ws_url).
                        </>
                      )}
                  </p>
                )}
                {chromePreviewIdle && (
                  <p className="browser-stream-idle" role="status">
                    Preview idle — interact in Chrome to refresh.
                  </p>
                )}
                {chromeStreamDisconnected && (
                  <p className="browser-stream-stalled" role="status">
                    Extension stream disconnected — reload teshi-bridge and Connect Chrome
                    again.
                  </p>
                )}
                {showStreamLoading && (
                  <p className="browser-stream-loading" role="status">
                    Updating screenshot…
                  </p>
                )}
                {streamStalled && !showStreamLoading && (streamError || !isChrome) && (
                  <p className="browser-stream-stalled" role="status">
                    Stream stalled —{" "}
                    {isWinApp
                      ? streamError || "attach to a visible WinUI3 window from the terminal."
                      : chromeInfo?.last_frame_error?.trim() ||
                        streamError ||
                        "use an http(s) tab in Chrome (not chrome://)."}
                  </p>
                )}
                <img
                  ref={imgRef}
                  src={frameSrc ?? undefined}
                  alt=""
                  className="browser-frame"
                  style={{ display: frameSrc ? "block" : "none" }}
                  onLoad={onFrameLoad}
                />
              </div>
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
