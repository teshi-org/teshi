import { useCallback, useEffect, useRef, useState } from "react";

interface Props {
  wsUrl: string | null;
  running: boolean;
  error: string | null;
  hint: string | null;
  onStart: () => void;
  onStop: () => void;
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
  onStart,
  onStop,
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
      <header className="panel-header">
        Browser {running ? "• live" : "• stopped"}
        {running && <span className="fps-label">{fps} fps</span>}
      </header>
      <div className="panel-body browser-body">
        {!running && !error && (
          <div className="browser-placeholder">
            <p>Click Start Browser to launch Playwright Chromium (1920×1080).</p>
            <button type="button" onClick={onStart}>
              Start Browser
            </button>
          </div>
        )}
        {error && (
          <div className="browser-error">
            <p>{error}</p>
            {hint && <code>{hint}</code>}
            <button type="button" onClick={onStart}>
              Retry
            </button>
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
            <button type="button" className="stop-btn" onClick={onStop}>
              Stop Browser
            </button>
          </div>
        )}
      </div>
    </section>
  );
}
