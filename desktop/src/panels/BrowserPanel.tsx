import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { PanelCollapseButton } from "./PanelCollapseButton";

interface Props {
  wsUrl: string | null;
  running: boolean;
  error: string | null;
  hint: string | null;
  fullscreen: boolean;
  gherkinCollapsed?: boolean;
  filesCollapsed?: boolean;
  onToggleGherkin?: () => void;
  onToggleFiles?: () => void;
  onStart: () => void;
  onStop: () => void;
  onToggleFullscreen: () => void;
}

const SOURCE_WIDTH = 1920;
const SOURCE_HEIGHT = 1080;
const SOURCE_ASPECT = SOURCE_WIDTH / SOURCE_HEIGHT;
const ZOOM_STEPS = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4];
const DEFAULT_ZOOM = 1;

interface FitSize {
  fitW: number;
  fitH: number;
}

function computeFitSize(containerW: number, containerH: number): FitSize {
  if (containerW <= 0 || containerH <= 0) {
    return { fitW: 0, fitH: 0 };
  }
  const containerAspect = containerW / containerH;
  if (containerAspect > SOURCE_ASPECT) {
    const fitH = containerH;
    return { fitW: fitH * SOURCE_ASPECT, fitH };
  }
  const fitW = containerW;
  return { fitW, fitH: fitW / SOURCE_ASPECT };
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
  error,
  hint,
  fullscreen,
  gherkinCollapsed = false,
  filesCollapsed = false,
  onToggleGherkin,
  onToggleFiles,
  onStart,
  onStop,
  onToggleFullscreen,
}: Props) {
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
  const [fitSize, setFitSize] = useState<FitSize>({ fitW: 0, fitH: 0 });
  const [dragging, setDragging] = useState(false);

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

  useEffect(() => {
    if (!running) {
      setZoom(DEFAULT_ZOOM);
    }
  }, [running]);

  useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap || !running) return;

    const updateFit = () => {
      setFitSize(computeFitSize(wrap.clientWidth, wrap.clientHeight));
    };

    updateFit();
    const observer = new ResizeObserver(updateFit);
    observer.observe(wrap);
    return () => observer.disconnect();
  }, [running, fullscreen]);

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
    if (!wrap || !running) return;

    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      zoomAtPoint(e.deltaY < 0 ? 1 : -1, e.clientX, e.clientY);
    };

    wrap.addEventListener("wheel", onWheel, { passive: false });
    return () => wrap.removeEventListener("wheel", onWheel);
  }, [running, zoomAtPoint]);

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

  useEffect(() => {
    if (!wsUrl || !running) {
      socketRef.current = null;
      setPageUrl("about:blank");
      setAddressInput("about:blank");
      return;
    }
    const socket = new WebSocket(wsUrl);
    socketRef.current = socket;
    socket.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data as string);
        if (msg.type === "frame" && imgRef.current) {
          imgRef.current.src = `data:image/jpeg;base64,${msg.data}`;
          framesRef.current += 1;
          if (typeof msg.url === "string" && msg.url) {
            setPageUrl(msg.url);
            if (!addressFocusedRef.current) {
              setAddressInput(msg.url);
            }
          }
        }
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
  }, [wsUrl, running]);

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
          Browser {running ? "• live" : "• stopped"}
          {running && <span className="fps-label">{fps} fps</span>}
        </span>
        <div className="panel-header-actions">
          {!running ? (
            <button type="button" className="panel-header-btn" onClick={onStart}>
              Start Browser
            </button>
          ) : (
            <button type="button" className="panel-header-btn" onClick={onStop}>
              Stop Browser
            </button>
          )}
          <span
            className={`status-dot ${running ? "on" : "off"}`}
            title={running ? "Browser running" : "Browser stopped"}
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
            <p>Use Start Browser in the panel header to launch Playwright Chromium (1920×1080).</p>
          </div>
        )}
        {error && (
          <div className="browser-error">
            <p>{error}</p>
            {hint && <code>{hint}</code>}
          </div>
        )}
        {running && (
          <div className="browser-viewport">
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
            </form>
            <div
              ref={wrapRef}
              className={`browser-frame-wrap${dragging ? " browser-frame-wrap--dragging" : ""}${zoom > 1 ? " browser-frame-wrap--pannable" : ""}`}
              tabIndex={0}
              onKeyDown={onViewportKeyDown}
              onMouseMove={onViewportMouseMove}
              onMouseLeave={onViewportMouseLeave}
              onMouseDown={onViewportMouseDown}
              onClick={() => wrapRef.current?.focus()}
              aria-label="Browser stream viewport"
            >
              {fitSize.fitW > 0 && fitSize.fitH > 0 && (
                <div
                  ref={scalerRef}
                  className="browser-frame-scaler"
                  style={{ width: displayW, height: displayH }}
                >
                  <img ref={imgRef} alt="Browser stream" className="browser-frame" />
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
