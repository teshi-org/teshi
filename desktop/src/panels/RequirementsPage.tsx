import { useState, useMemo } from "react";
import { RequirementsInput, type SegmentsData } from "./RequirementsInput";
import { RequirementsText } from "./RequirementsText";
import { MindMapViewer, parseFreeMindXml, type MindMapNode } from "./MindMapViewer";
import { MockHtmlViewer } from "./MockHtmlViewer";

export interface GeneratedData {
  slug: string;
  segments: SegmentsData;
  mindmapXml: string;
  mockHtml: string;
}

export function RequirementsPage() {
  const [data, setData] = useState<GeneratedData | null>(null);
  const [highlightedSegmentIds, setHighlightedSegmentIds] = useState<Set<string>>(new Set());
  const [highlightedNodeIds, setHighlightedNodeIds] = useState<Set<string>>(new Set());

  // Parse mindmap XML once — same node IDs used for the map and the viewer
  const mindmapRoot: MindMapNode | null = useMemo(
    () => (data?.mindmapXml ? parseFreeMindXml(data.mindmapXml) : null),
    [data],
  );

  // Build reverse map: segmentId → Set of nodeIds
  const segmentToNodeMap = useMemo(() => {
    const map = new Map<string, Set<string>>();
    if (!mindmapRoot) return map;

    const walk = (node: MindMapNode) => {
      for (const segId of node.link) {
        if (!map.has(segId)) map.set(segId, new Set());
        map.get(segId)!.add(node.id);
      }
      for (const child of node.children) {
        walk(child);
      }
    };

    walk(mindmapRoot);
    return map;
  }, [mindmapRoot]);

  const handleGenerated = (generated: GeneratedData) => {
    setData(generated);
    setHighlightedSegmentIds(new Set());
    setHighlightedNodeIds(new Set());
  };

  const handleNodeClick = (linkIds: string[]) => {
    setHighlightedSegmentIds(new Set(linkIds));
    setHighlightedNodeIds(new Set());
  };

  const handleSegmentClick = (segmentId: string) => {
    setHighlightedSegmentIds(new Set([segmentId]));
    const nodeIds = segmentToNodeMap.get(segmentId) ?? new Set<string>();
    setHighlightedNodeIds(nodeIds);
  };

  return (
    <div className="requirements-page">
      <div className="requirements-page__panel requirements-page__panel--input">
        {!data ? (
          <RequirementsInput onGenerated={handleGenerated} />
        ) : (
          <RequirementsText
            originalText={data.segments.originalText}
            segments={data.segments.segments}
            highlightedIds={highlightedSegmentIds}
            onSegmentClick={handleSegmentClick}
          />
        )}
      </div>
      <div className="requirements-page__panel requirements-page__panel--mindmap">
        {data ? (
          mindmapRoot ? (
            <MindMapViewer
              root={mindmapRoot}
              highlightedNodeIds={highlightedNodeIds}
              onNodeClick={handleNodeClick}
            />
          ) : (
            <div className="requirements-page__empty">
              <p>No test-point mindmap was generated.</p>
              <p className="requirements-page__empty-hint">
                The LLM did not produce a mindmap. Try adjusting your requirements text or model.
              </p>
            </div>
          )
        ) : null}
      </div>
      <div className="requirements-page__panel requirements-page__panel--mock">
        {data ? (
          data.mockHtml ? (
            <MockHtmlViewer html={data.mockHtml} />
          ) : (
            <div className="requirements-page__empty">
              <p>No mock UI was generated.</p>
              <p className="requirements-page__empty-hint">
                The LLM did not produce a mock HTML preview. Try adjusting your requirements text or model.
              </p>
            </div>
          )
        ) : null}
      </div>
    </div>
  );
}
