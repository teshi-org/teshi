import { useState } from "react";
import { toast } from "sonner";
import { getRuntime } from "../platform";
import type { GeneratedData } from "./RequirementsPage";

export interface SegmentsData {
  originalText: string;
  segments: Array<{ id: string; text: string; pos: [number, number] }>;
}

export function RequirementsInput({ onGenerated }: { onGenerated: (data: GeneratedData) => void }) {
  const [text, setText] = useState("");
  const [loading, setLoading] = useState(false);

  const handleGenerate = async () => {
    if (!text.trim()) {
      toast.error("Please enter requirements text first");
      return;
    }
    setLoading(true);
    try {
      const result = await getRuntime().generateRequirements(text);
      onGenerated({
        slug: result.slug,
        segments: { originalText: text, segments: result.segments },
        mindmapXml: result.mindmap_xml,
        mockHtml: result.mock_html,
      });
      toast.success("Test points generated!");
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      toast.error("Generation failed: " + msg);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="requirements-input">
      <textarea
        className="requirements-input__textarea"
        placeholder="Paste your requirements text here..."
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={20}
      />
      <button
        className="requirements-input__button"
        onClick={handleGenerate}
        disabled={loading || !text.trim()}
      >
        {loading ? "Generating..." : "Generate Test Points"}
      </button>
    </div>
  );
}
