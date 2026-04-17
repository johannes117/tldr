import { useEffect, useMemo, useState } from "react";
import { useParams, useNavigate, useSearch } from "@tanstack/react-router";
import { ReactFlow, Background, Controls, Node, Edge } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import dagre from "@dagrejs/dagre";

type GraphNode = { id: string; qualified_name: string; path: string; line: number; changed: boolean };
type GraphEdge = {
  from: string; to: string;
  status: "same" | "added" | "removed" | "args-changed";
  call_sites: { path: string; line: number }[];
};
type Graph = { nodes: GraphNode[]; edges: GraphEdge[] };

const EDGE_COLOR: Record<string, string> = {
  same: "#000", added: "#0a0", removed: "#c00", "args-changed": "#e67e22",
};

function layout(nodes: Node[], edges: Edge[]): Node[] {
  const g = new dagre.graphlib.Graph();
  g.setDefaultEdgeLabel(() => ({}));
  g.setGraph({ rankdir: "LR", nodesep: 40, ranksep: 80 });
  nodes.forEach(n => g.setNode(n.id, { width: 180, height: 40 }));
  edges.forEach(e => g.setEdge(e.source, e.target));
  dagre.layout(g);
  return nodes.map(n => {
    const p = g.node(n.id);
    return { ...n, position: { x: p.x - 90, y: p.y - 20 } };
  });
}

export function PrGraph() {
  const { number } = useParams({ strict: false }) as { number: string };
  const navigate = useNavigate();
  const search = useSearch({ strict: false }) as { hide?: string; hop?: string };
  const hideUnchanged = search.hide === "1";
  const hop = Number(search.hop ?? 1);
  const [graph, setGraph] = useState<Graph | null>(null);
  const [selected, setSelected] = useState<GraphEdge | null>(null);

  useEffect(() => {
    fetch(`/api/pr/${number}/call-graph`).then(r => r.json()).then(setGraph).catch(() => {});
  }, [number]);

  const { nodes, edges } = useMemo(() => {
    if (!graph) return { nodes: [] as Node[], edges: [] as Edge[] };
    const keep = new Set(graph.nodes.filter(n => n.changed).map(n => n.id));
    // expand by hop
    for (let i = 0; i < hop; i++) {
      const add = new Set<string>();
      graph.edges.forEach(e => {
        if (keep.has(e.from)) add.add(e.to);
        if (keep.has(e.to)) add.add(e.from);
      });
      add.forEach(a => keep.add(a));
    }
    const visibleNodes = graph.nodes.filter(n => keep.has(n.id));
    let visibleEdges = graph.edges.filter(e => keep.has(e.from) && keep.has(e.to));
    if (hideUnchanged) visibleEdges = visibleEdges.filter(e => e.status !== "same");
    const rfNodes: Node[] = visibleNodes.map(n => ({
      id: n.id,
      data: { label: `${n.qualified_name.split(":")[1] ?? n.qualified_name}` },
      position: { x: 0, y: 0 },
      style: {
        border: n.changed ? "2px solid #000" : "1px solid #ccc",
        padding: 6, borderRadius: 6, background: "#fff", fontSize: 12,
      },
    }));
    const rfEdges: Edge[] = visibleEdges.map((e, i) => ({
      id: `e${i}`,
      source: e.from, target: e.to,
      style: { stroke: EDGE_COLOR[e.status] ?? "#000" },
      data: e as unknown as Record<string, unknown>,
      label: e.status === "same" ? undefined : e.status,
    }));
    return { nodes: layout(rfNodes, rfEdges), edges: rfEdges };
  }, [graph, hideUnchanged, hop]);

  const toggle = (k: "hide" | "hop", v: string) => {
    const p = new URLSearchParams(window.location.search);
    p.set(k, v);
    navigate({ to: `/pr/${number}/graph`, search: Object.fromEntries(p) as never });
  };

  return (
    <div style={{ display: "flex", height: "100vh" }}>
      <div style={{ flex: 1, position: "relative" }}>
        <div style={{ position: "absolute", top: 8, left: 8, zIndex: 10, background: "#fff", padding: 8, border: "1px solid #ddd", borderRadius: 4, fontSize: 12 }}>
          <label><input type="checkbox" checked={hideUnchanged} onChange={e => toggle("hide", e.target.checked ? "1" : "0")} /> hide unchanged</label>
          <span style={{ marginLeft: 12 }}>hops:
            {[1,2,3].map(h => (
              <button key={h} onClick={() => toggle("hop", String(h))} style={{ marginLeft: 4, fontWeight: hop === h ? "bold" : "normal" }}>{h}</button>
            ))}
          </span>
        </div>
        <ReactFlow
          nodes={nodes} edges={edges}
          onNodeClick={(_, n) => {
            const gn = graph?.nodes.find(x => x.id === n.id);
            if (gn) navigate({ to: `/pr/${number}/files`, search: { file: gn.path } as never, hash: `L${gn.line}` });
          }}
          onEdgeClick={(_, e) => {
            const gd = (e.data as unknown) as GraphEdge | undefined;
            if (gd) setSelected(gd);
          }}
          fitView
        >
          <Background />
          <Controls />
        </ReactFlow>
      </div>
      {selected && (
        <div style={{ width: 320, borderLeft: "1px solid #ddd", padding: 12, overflow: "auto" }}>
          <div style={{ fontWeight: "bold" }}>{selected.status}</div>
          <div style={{ fontSize: 12, color: "#666" }}>{selected.from} → {selected.to}</div>
          <ul>
            {selected.call_sites.map((s, i) => (
              <li key={i}>
                <a href={`/pr/${number}/files?file=${encodeURIComponent(s.path)}#L${s.line}`}>{s.path}:{s.line}</a>
              </li>
            ))}
          </ul>
          <button onClick={() => setSelected(null)}>Close</button>
        </div>
      )}
    </div>
  );
}
