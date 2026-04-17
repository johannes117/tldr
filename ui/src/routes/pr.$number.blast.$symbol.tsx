import { useEffect, useState } from "react";
import { useParams } from "@tanstack/react-router";

type Ref = { path: string; line: number; in_pr_diff: boolean; diagnostic: "ok" | "warning" | "error"; diag_msg: string };
type Blast = {
  symbol: { qualified_name: string; path: string; line: number; kind: string };
  signature_before: string;
  signature_after: string;
  references: Ref[];
};

export function PrBlast() {
  const { number, symbol } = useParams({ strict: false }) as { number: string; symbol: string };
  const [data, setData] = useState<Blast | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [changed, setChanged] = useState<{ qualified_name: string; path: string; line: number }[]>([]);

  useEffect(() => {
    fetch(`/api/pr/${number}/call-graph`).then(r => r.json()).then((g: { nodes: { changed: boolean; qualified_name: string; path: string; line: number }[] }) => {
      setChanged(g.nodes.filter(n => n.changed));
    }).catch(() => {});
  }, [number]);

  useEffect(() => {
    if (!symbol) return;
    fetch(`/api/pr/${number}/blast/${encodeURIComponent(symbol)}`)
      .then(r => r.ok ? r.json() : Promise.reject(r.statusText))
      .then(setData)
      .catch(e => setErr(String(e)));
  }, [number, symbol]);

  return (
    <div style={{ display: "flex", height: "100vh" }}>
      <aside style={{ width: 280, borderRight: "1px solid #ddd", overflow: "auto", padding: 8 }}>
        <h3 style={{ fontSize: 13 }}>Changed exported symbols</h3>
        <ul style={{ fontSize: 12 }}>
          {changed.map(n => (
            <li key={n.qualified_name}>
              <a href={`/pr/${number}/blast/${encodeURIComponent(n.qualified_name)}`}>{n.qualified_name}</a>
              <div style={{ color: "#888" }}>{n.path}:{n.line}</div>
            </li>
          ))}
        </ul>
      </aside>
      <main style={{ flex: 1, padding: 16, overflow: "auto" }}>
        {err && <div style={{ color: "red" }}>{err}</div>}
        {data && (
          <>
            <h2 style={{ fontSize: 16 }}>{data.symbol.qualified_name}</h2>
            <div style={{ fontSize: 12, color: "#666" }}>{data.symbol.path}:{data.symbol.line} ({data.symbol.kind})</div>
            <section style={{ marginTop: 12 }}>
              <h3 style={{ fontSize: 13 }}>Signature diff</h3>
              <pre style={{ background: "#fee", padding: 8 }}>- {data.signature_before}</pre>
              <pre style={{ background: "#efe", padding: 8 }}>+ {data.signature_after}</pre>
            </section>
            <section style={{ marginTop: 12 }}>
              <h3 style={{ fontSize: 13 }}>References ({data.references.length})</h3>
              <table style={{ fontSize: 12, width: "100%", borderCollapse: "collapse" }}>
                <thead><tr><th>Location</th><th>In PR?</th><th>Diag</th><th>Msg</th></tr></thead>
                <tbody>
                  {data.references.map((r, i) => {
                    const critical = !r.in_pr_diff && r.diagnostic === "error";
                    return (
                      <tr key={i} style={{ background: critical ? "#fee" : undefined }}>
                        <td><a href={`/pr/${number}/files?file=${encodeURIComponent(r.path)}#L${r.line}`}>{r.path}:{r.line}</a></td>
                        <td>{r.in_pr_diff ? "yes" : "no"}</td>
                        <td style={{ color: r.diagnostic === "error" ? "red" : r.diagnostic === "warning" ? "orange" : "green" }}>{r.diagnostic}</td>
                        <td>{r.diag_msg}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </section>
          </>
        )}
      </main>
    </div>
  );
}
