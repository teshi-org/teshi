import { useMemo, useEffect } from "react";

export function MockHtmlViewer({ html }: { html: string }) {
  const blobUrl = useMemo(() => {
    const blob = new Blob([html], { type: "text/html" });
    return URL.createObjectURL(blob);
  }, [html]);

  useEffect(() => {
    return () => {
      URL.revokeObjectURL(blobUrl);
    };
  }, [blobUrl]);

  return (
    <div className="mock-html-viewer">
      <h3 className="mock-html-viewer__title">Mock UI</h3>
      <iframe
        className="mock-html-viewer__iframe"
        src={blobUrl}
        sandbox="allow-scripts"
        title="Mock UI Preview"
      />
    </div>
  );
}
