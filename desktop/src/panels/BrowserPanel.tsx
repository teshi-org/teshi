import { useEffect, useRef, useState } from "react";

interface Props {
  wsUrl: string | null;
  running: boolean;
  error: string | null;
  hint: string | null;
  onStart: () => void;
  onStop: () => void;
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
  const [fps, setFps] = useState(0);
  const framesRef = useRef(0);

  useEffect(() => {
    if (!wsUrl || !running) return;
    const socket = new WebSocket(wsUrl);
    socket.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data as string);
        if (msg.type === "frame" && imgRef.current) {
          imgRef.current.src = `data:image/jpeg;base64,${msg.data}`;
          framesRef.current += 1;
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
      clearInterval(timer);
    };
  }, [wsUrl, running]);

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
            <img ref={imgRef} alt="Browser stream" className="browser-frame" />
            <button type="button" className="stop-btn" onClick={onStop}>
              Stop Browser
            </button>
          </div>
        )}
      </div>
    </section>
  );
}
