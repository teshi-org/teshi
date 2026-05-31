import { useCallback, useEffect, useRef, useState } from "react";

interface Props {
  wsUrl: string | null;
  running: boolean;
  error: string | null;
  hint: string | null;
  fullscreen: boolean;
  onStart: () => void;
  onStop: () => void;
  onToggleFullscreen: () => void;
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
  onStart,
  onStop,
  onToggleFullscreen,
}: Props) {
  const imgRef = useRef<HTMLImageElement>(null);
  const socketRef = useRef<WebSocket | null>(null);
  const addressFocusedRef = useRef(false);
  const [fps, setFps] = useState(0);
  const framesRef = useRef(0);
  const [pageUrl, setPageUrl] = useState("about:blank");
  const [addressInput, setAddressInput] = useState("about:blank");

  const navigateTo = useCallback((raw: string) => {
    const url = normalizeUrl(raw);
    if (!url || !socketRef.current || socketRef.current.readyState !== WebSocket.OPEN) {
      return;
    }
    socketRef.current.send(JSON.stringify({ cmd: "navigate", url }));
    setAddressInput(url);
  }, []);

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

  return (
    <section className="panel browser-panel">
      <header className="panel-header panel-header--browser">
        <span>
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
            </form>
            <div className="browser-frame-wrap">
              <img ref={imgRef} alt="Browser stream" className="browser-frame" />
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
