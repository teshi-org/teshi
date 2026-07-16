import React from "react";

interface Segment {
  id: string;
  text: string;
  pos: [number, number];
}

export function RequirementsText({
  originalText,
  segments,
  highlightedIds,
  onSegmentClick,
}: {
  originalText: string;
  segments: Segment[];
  highlightedIds: Set<string>;
  onSegmentClick: (segmentId: string) => void;
}) {
  // Build a map from character position to segment
  const posToSegment = new Map<number, Segment>();
  for (const seg of segments) {
    for (let i = seg.pos[0]; i < seg.pos[1]; i++) {
      posToSegment.set(i, seg);
    }
  }

  // Build display elements: run of chars in same segment become one span
  const elements: React.ReactNode[] = [];
  let i = 0;
  while (i < originalText.length) {
    const seg = posToSegment.get(i);
    if (!seg) {
      elements.push(<span key={`raw-${i}`}>{originalText[i]}</span>);
      i++;
      continue;
    }
    const start = i;
    while (i < originalText.length && posToSegment.get(i)?.id === seg.id) {
      i++;
    }
    const isHighlighted = highlightedIds.has(seg.id);
    elements.push(
      <span
        key={seg.id}
        className={`requirements-text__word ${isHighlighted ? "requirements-text__word--highlighted" : ""}`}
        onClick={() => onSegmentClick(seg.id)}
        title={seg.id}
      >
        {originalText.slice(start, i)}
      </span>
    );
  }

  return (
    <div className="requirements-text">
      <h3 className="requirements-text__title">Requirements</h3>
      <div className="requirements-text__content">{elements}</div>
    </div>
  );
}
