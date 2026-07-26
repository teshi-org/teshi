import { useCallback, useEffect, useState } from "react";
import { getRuntime } from "../platform";

interface ReplayScreenshotEntry {
  line: number;
  keyword: string;
  text: string;
  filename: string;
}

interface ReplayIndex {
  screenshots: ReplayScreenshotEntry[];
}

interface Props {
  projectRoot: string | null;
}

export function ScreenshotsPanel({ projectRoot }: Props) {
  const [entries, setEntries] = useState<ReplayScreenshotEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dataUrls, setDataUrls] = useState<Record<string, string>>({});
  const [fullImage, setFullImage] = useState<string | null>(null);

  useEffect(() => {
    if (!projectRoot) return;

    let cancelled = false;

    const load = async () => {
      const indexPath =
        projectRoot.replace(/\\/g, "/").replace(/\/$/, "") +
        "/.teshi/logs/replay-screenshots/index.json";

      try {
        const text = await getRuntime().readTextFile(indexPath);
        if (cancelled) return;
        const parsed = JSON.parse(text) as ReplayIndex;
        const list = parsed.screenshots ?? [];
        setEntries(list);
        setError(null);

        // Load all images in parallel
        const basePath =
          projectRoot.replace(/\\/g, "/").replace(/\/$/, "") +
          "/.teshi/logs/replay-screenshots/";
        const urls: Record<string, string> = {};
        const results = await Promise.allSettled(
          list.map(async (entry) => {
            const imgPath = basePath + entry.filename;
            const dataUrl = await getRuntime().readFileAsDataUrl(imgPath);
            return { filename: entry.filename, dataUrl };
          }),
        );
        if (cancelled) return;
        for (const result of results) {
          if (result.status === "fulfilled") {
            urls[result.value.filename] = result.value.dataUrl;
          }
        }
        setDataUrls(urls);
      } catch (e) {
        if (cancelled) return;
        setEntries(null);
        setError(e instanceof Error ? e.message : String(e));
      }
    };

    void load();
    return () => {
      cancelled = true;
    };
  }, [projectRoot]);

  const onOverlayClick = useCallback(() => {
    setFullImage(null);
  }, []);

  // No project loaded
  if (!projectRoot) {
    return (
      <p className="placeholder">Open a project to view replay screenshots.</p>
    );
  }

  // index.json not found or parse error
  if (error && !entries) {
    return (
      <div className="screenshots-panel">
        <p className="placeholder">
          No replay screenshots found. Run{" "}
          <code>teshi browser replay</code> to capture screenshots during replay.
        </p>
      </div>
    );
  }

  // Empty list
  if (entries && entries.length === 0) {
    return (
      <div className="screenshots-panel">
        <p className="placeholder">
          No screenshots captured yet. Run{" "}
          <code>teshi browser replay</code> to capture screenshots during replay.
        </p>
      </div>
    );
  }

  return (
    <div className="screenshots-panel">
      <div className="screenshots-header">
        <span>Screenshots</span>
        <span className="screenshots-status">{entries?.length ?? 0} captures</span>
      </div>
      <div className="screenshots-grid">
        {entries?.map((entry) => {
          const label = `L${entry.line}: ${entry.keyword} ${entry.text}`;
          const truncated =
            label.length > 40 ? label.slice(0, 40) + "…" : label;
          return (
            <div
              key={entry.filename}
              className="screenshot-thumb"
              onClick={() => {
                const url = dataUrls[entry.filename];
                if (url) setFullImage(url);
              }}
            >
              {dataUrls[entry.filename] ? (
                <img
                  className="screenshot-thumb-img"
                  src={dataUrls[entry.filename]}
                  alt={truncated}
                />
              ) : (
                <div className="screenshot-thumb-img" />
              )}
              <div className="screenshot-thumb-label" title={label}>
                {truncated}
              </div>
            </div>
          );
        })}
      </div>

      {fullImage && (
        <div className="screenshot-overlay" onClick={onOverlayClick}>
          <img
            className="screenshot-full"
            src={fullImage}
            alt="Screenshot full size"
            onClick={(e) => e.stopPropagation()}
          />
        </div>
      )}
    </div>
  );
}
