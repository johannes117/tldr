import { Link, useParams } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api, type FileDiff, type Line } from "../api";

export function PrFiles() {
  const { number } = useParams({ from: "/pr/$number/files" });
  const n = Number(number);
  const qc = useQueryClient();
  const diffQ = useQuery({ queryKey: ["diff", n], queryFn: () => api.diff(n) });
  const draftQ = useQuery({ queryKey: ["draft", n], queryFn: () => api.getDraft(n) });

  const files = diffQ.data?.files ?? [];
  const [idx, setIdx] = useState(0);
  const current = files[idx];

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === "TEXTAREA" || (e.target as HTMLElement)?.tagName === "INPUT") return;
      if (e.key === "j" || e.key === "n") { e.preventDefault(); setIdx((i) => Math.min(files.length - 1, i + 1)); }
      else if (e.key === "k" || e.key === "p") { e.preventDefault(); setIdx((i) => Math.max(0, i - 1)); }
      else if (e.key === "v" && current) {
        const p = current.path;
        const cur = draftQ.data?.file_state?.[p]?.viewed ?? false;
        api.setFileState(n, p, { viewed: !cur }).then(() => qc.invalidateQueries({ queryKey: ["draft", n] }));
      } else if (e.key === "c") {
        const el = document.getElementById("comment-body") as HTMLTextAreaElement | null;
        el?.focus();
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [files.length, current, draftQ.data, n, qc]);

  if (diffQ.isLoading) return <div className="p-6">loading diff…</div>;
  if (diffQ.isError) return <div className="p-6 text-red-600">error: {String(diffQ.error)}</div>;

  return (
    <div className="h-full flex">
      <aside className="w-72 border-r overflow-y-auto">
        <div className="p-3 text-sm text-slate-500 border-b">Files ({files.length})</div>
        <ul>
          {files.map((f, i) => {
            const viewed = draftQ.data?.file_state?.[f.path]?.viewed;
            return (
              <li key={f.path}>
                <button
                  onClick={() => setIdx(i)}
                  className={`w-full text-left px-3 py-1.5 text-sm truncate ${i === idx ? "bg-slate-200" : "hover:bg-slate-100"}`}
                  title={f.path}
                >
                  <span className={viewed ? "line-through text-slate-400" : ""}>{f.path}</span>
                </button>
              </li>
            );
          })}
        </ul>
      </aside>
      <section className="flex-1 overflow-y-auto">
        <div className="px-4 py-2 border-b flex items-center gap-3">
          <Link to="/pr/$number" params={{ number: String(n) }} className="text-sm text-slate-500">&larr; overview</Link>
          <div className="font-mono text-sm truncate flex-1">{current?.path}</div>
          <Link to="/pr/$number/review" params={{ number: String(n) }} className="px-3 py-1 bg-slate-900 text-white text-sm rounded">Review</Link>
        </div>
        <div className="text-xs text-slate-500 px-4 py-1 border-b">j/k next/prev · v toggle viewed · c focus comment</div>
        {current && <FileView n={n} file={current} />}
      </section>
    </div>
  );
}

function FileView({ n, file }: { n: number; file: FileDiff }) {
  const qc = useQueryClient();
  const [draft, setDraft] = useState<{ line: number; body: string } | null>(null);

  const submit = async () => {
    if (!draft) return;
    await api.addComment(n, { path: file.path, line: draft.line, side: "RIGHT", body: draft.body });
    setDraft(null);
    qc.invalidateQueries({ queryKey: ["draft", n] });
  };

  return (
    <div className="p-4">
      {file.hunks.map((h, i) => (
        <div key={i} className="mb-4 border rounded overflow-hidden">
          <div className="bg-slate-100 font-mono text-xs px-2 py-1 text-slate-600">{h.header}</div>
          <table className="w-full font-mono text-xs">
            <tbody>
              {h.lines.map((l, j) => (
                <LineRow key={j} l={l} onComment={(line) => setDraft({ line, body: "" })} />
              ))}
            </tbody>
          </table>
        </div>
      ))}
      {draft && (
        <div className="border rounded p-3 bg-white shadow-sm max-w-2xl">
          <div className="text-xs text-slate-500 mb-1">comment on line {draft.line}</div>
          <textarea
            id="comment-body"
            className="w-full border rounded p-2 text-sm"
            rows={3}
            value={draft.body}
            onChange={(e) => setDraft({ ...draft, body: e.target.value })}
          />
          <div className="mt-2 flex gap-2">
            <button onClick={submit} className="px-3 py-1 bg-slate-900 text-white rounded text-sm">Add</button>
            <button onClick={() => setDraft(null)} className="px-3 py-1 border rounded text-sm">Cancel</button>
          </div>
        </div>
      )}
    </div>
  );
}

function LineRow({ l, onComment }: { l: Line; onComment: (line: number) => void }) {
  const bg = l.kind === "add" ? "bg-green-50" : l.kind === "del" ? "bg-red-50" : "";
  const marker = l.kind === "add" ? "+" : l.kind === "del" ? "-" : " ";
  const ln = l.new_line ?? l.old_line;
  return (
    <tr className={bg}>
      <td className="text-right pr-2 pl-2 text-slate-400 w-12 select-none">{l.old_line ?? ""}</td>
      <td className="text-right pr-2 text-slate-400 w-12 select-none">{l.new_line ?? ""}</td>
      <td className="w-4 text-slate-400 select-none">{marker}</td>
      <td className="whitespace-pre-wrap break-all pr-2">{l.content}</td>
      <td className="w-8 text-right pr-2">
        {ln != null && l.kind !== "del" && (
          <button className="text-slate-400 hover:text-slate-800" onClick={() => onComment(ln!)} title="Add comment">+</button>
        )}
      </td>
    </tr>
  );
}
