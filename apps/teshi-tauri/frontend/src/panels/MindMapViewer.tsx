import { useState } from "react";

export interface MindMapNode {
  id: string;
  text: string;
  link: string[];  // segment IDs from LINK attribute
  children: MindMapNode[];
}

let _nextNodeId = 1;

export function parseFreeMindXml(xml: string): MindMapNode | null {
  const parser = new DOMParser();
  const doc = parser.parseFromString(xml, "text/xml");
  const errorNode = doc.querySelector("parsererror");
  if (errorNode) {
    console.error("Failed to parse FreeMind XML:", errorNode.textContent);
    return null;
  }
  const rootEl = doc.querySelector("map > node");
  if (!rootEl) return null;
  _nextNodeId = 1;
  return parseNode(rootEl);
}

function parseNode(el: Element): MindMapNode {
  const text = el.getAttribute("TEXT") || "";
  const link = (el.getAttribute("LINK") || "").split(",").filter(Boolean);
  const children: MindMapNode[] = [];
  el.querySelectorAll(":scope > node").forEach((child) => {
    children.push(parseNode(child));
  });
  const id = `n${_nextNodeId++}`;
  return { id, text, link, children };
}

export function MindMapViewer({
  root,
  highlightedNodeIds,
  onNodeClick,
}: {
  root: MindMapNode;
  highlightedNodeIds: Set<string>;
  onNodeClick: (linkIds: string[]) => void;
}) {
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());

  const toggleExpand = (id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const renderNode = (node: MindMapNode, depth: number): React.ReactNode => {
    const isExpanded = expandedIds.has(node.id) || depth < 2; // auto-expand first 2 levels
    const isHighlighted = highlightedNodeIds.has(node.id);
    const hasChildren = node.children.length > 0;

    return (
      <div key={node.id} className="mindmap-viewer__node" style={{ paddingLeft: depth * 20 }}>
        <div
          className={`mindmap-viewer__node-content ${isHighlighted ? "mindmap-viewer__node-content--highlighted" : ""}`}
          onClick={() => {
            if (hasChildren) toggleExpand(node.id);
            if (node.link.length > 0) onNodeClick(node.link);
          }}
        >
          {hasChildren && (
            <span className="mindmap-viewer__toggle">{isExpanded ? "▼" : "▶"}</span>
          )}
          <span className="mindmap-viewer__label">{node.text}</span>
          {node.link.length > 0 && (
            <span className="mindmap-viewer__badge">{node.link.length}</span>
          )}
        </div>
        {hasChildren && isExpanded && (
          <div className="mindmap-viewer__children">
            {node.children.map((child) => renderNode(child, depth + 1))}
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="mindmap-viewer">
      <h3 className="mindmap-viewer__title">Test Points</h3>
      <div className="mindmap-viewer__tree">{renderNode(root, 0)}</div>
    </div>
  );
}
