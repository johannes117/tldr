import { Link, useParams } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { diffWordsWithSpace } from "diff";
import { api, type FileDiff, type Line } from "../api";
import { CommentComposer } from "../components/CommentComposer";
import { FileTree } from "../components/FileTree";
import { CoverageGutter } from "../components/CoverageGutter";

export function PrFiles() {
  const { number } = useParams({ from: "/pr/$number/files" });
  const n = Number(number);
  const qc = useQueryClient();
  const diffQ = useQuery({ queryKey: ["diff", n], queryFn: () => api.diff(n) });
  const draftQ = useQuery({ queryKey: ["draft", n], queryFn: () => api.getDraft(n) });
  const framingQ = useQuery({ queryKey: ["framing", n], queryFn: () => api.framing(n) });
  const coverageQ = useQuery({ queryKey: ["coverage", n], queryFn: () => api.coverage(n) });

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
        <FileTree files={files} framing={framingQ.data?.files} draft={draftQ.data} idx={idx} onSelect={setIdx} />
      </aside>
      <section className="flex-1 overflow-y-auto">
        <div className="px-4 py-2 border-b flex items-center gap-3">
          <Link to="/pr/$number" params={{ number: String(n) }} className="text-sm text-slate-500">&larr; overview</Link>
          <div className="font-mono text-sm truncate flex-1">{current?.path}</div>
          <button
            onClick={() => {
              if (!current) return;
              const line = current.hunks[0]?.new_start;
              api.openInEditor({ path: current.path, line }).catch((e) => alert(`open in editor failed: ${e}`));
            }}
            disabled={!current}
            className="px-3 py-1 border text-sm rounded disabled:opacity-50"
            title="Open current file in your configured editor"
          >
            Open in editor
          </button>
          <Link to="/pr/$number/review" params={{ number: String(n) }} className="px-3 py-1 bg-slate-900 text-white text-sm rounded">Review</Link>
        </div>
        <div className="text-xs text-slate-500 px-4 py-1 border-b">j/k next/prev · v toggle viewed · c focus comment</div>
        {current && <FileView n={n} file={current} coverage={coverageQ.data?.files?.[current.path]?.lines} onJump={(path) => {
          const i = files.findIndex((f) => f.path === path);
          if (i >= 0) setIdx(i);
        }} />}
      </section>
    </div>
  );
}

function FileView({ n, file, onJump, coverage }: { n: number; file: FileDiff; onJump: (p: string) => void; coverage?: Record<string, "covered" | "uncovered" | "none"> }) {
  const qc = useQueryClient();
  const [draft, setDraft] = useState<{ line: number; body: string } | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [fullFile, setFullFile] = useState<FileDiff | null>(null);

  const shouldAutoCollapse = file.is_generated || file.status === "renamed" || file.is_large;
  const isCollapsed = shouldAutoCollapse && !expanded;

  useEffect(() => {
    setExpanded(false);
    setFullFile(null);
  }, [file.path]);

  useEffect(() => {
    if (expanded && file.is_large && !fullFile) {
      api.diffFile(n, file.path, true).then(setFullFile).catch(() => {});
    }
  }, [expanded, file.is_large, file.path, fullFile, n]);

  const effective = fullFile ?? file;

  const submit = async () => {
    if (!draft) return;
    await api.addComment(n, { path: file.path, line: draft.line, side: "RIGHT", body: draft.body });
    setDraft(null);
    qc.invalidateQueries({ queryKey: ["draft", n] });
  };

  if (file.is_image) {
    return <ImageView file={file} />;
  }

  return (
    <div className="p-4">
      {file.status === "renamed" && (
        <div className="mb-2 text-xs text-blue-700">
          Renamed from <code className="font-mono">{file.old_path}</code> to <code className="font-mono">{file.path}</code>
          {file.similarity != null && <span> ({file.similarity}% similar)</span>}
        </div>
      )}
      {isCollapsed && (
        <div className="border rounded p-3 bg-slate-50 flex items-center gap-2">
          <div className="text-sm text-slate-600 flex-1">
            {file.is_generated && "Generated file. "}
            {file.is_large && `Large diff (${file.stats?.added ?? 0} added, ${file.stats?.removed ?? 0} removed). `}
            {file.status === "renamed" && !file.is_large && "Rename. "}
          </div>
          <button className="px-2 py-1 border text-xs rounded" onClick={() => setExpanded(true)}>Expand</button>
        </div>
      )}
      {!isCollapsed && effective.hunks.map((h, i) => {
        const withWord = applyWordDiff(h.lines, file.path);
        return (
          <div key={i} className="mb-4 border rounded overflow-hidden">
            <div className="bg-slate-100 font-mono text-xs px-2 py-1 text-slate-600">{h.header}</div>
            <table className="w-full font-mono text-xs">
              <tbody>
                {withWord.map((l, j) => (
                  <LineRow key={j} l={l} onComment={(line) => setDraft({ line, body: "" })} onJump={onJump} coverage={coverage} />
                ))}
              </tbody>
            </table>
          </div>
        );
      })}
      {draft && (
        <div className="border rounded p-3 bg-white shadow-sm max-w-4xl">
          <div className="text-xs text-slate-500 mb-1">comment on line {draft.line}</div>
          <CommentComposer
            value={draft.body}
            onChange={(v) => setDraft({ ...draft, body: v })}
            prNumber={n}
            onSubmit={submit}
            onCancel={() => setDraft(null)}
            autoFocus
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

function ImageView({ file }: { file: FileDiff }) {
  const [slider, setSlider] = useState(50);
  const img = file.image;
  const isSvg = file.path.toLowerCase().endsWith(".svg");
  if (!img) return <div className="p-4 text-sm text-slate-500">no image data</div>;
  return (
    <div className="p-4">
      <div className="text-sm text-slate-600 mb-2">Image diff: {file.path}</div>
      <div className="flex gap-4 items-start">
        <div className="flex-1">
          <div className="text-xs text-slate-500 mb-1">before</div>
          {img.old_data_url ? (
            isSvg ? <div dangerouslySetInnerHTML={{ __html: atob(img.old_data_url.split(",")[1] ?? "") }} />
                  : <img src={img.old_data_url} className="max-w-full border" />
          ) : <div className="text-slate-400 text-xs">(none)</div>}
        </div>
        <div className="flex-1">
          <div className="text-xs text-slate-500 mb-1">after</div>
          {img.new_data_url ? (
            isSvg ? <div dangerouslySetInnerHTML={{ __html: atob(img.new_data_url.split(",")[1] ?? "") }} />
                  : <img src={img.new_data_url} className="max-w-full border" />
          ) : <div className="text-slate-400 text-xs">(none)</div>}
        </div>
      </div>
      {!isSvg && img.old_data_url && img.new_data_url && (
        <div className="mt-4">
          <div className="text-xs text-slate-500 mb-1">onion-skin ({slider}%)</div>
          <div className="relative inline-block border">
            <img src={img.old_data_url} className="block max-w-full" />
            <img src={img.new_data_url} className="absolute inset-0 max-w-full" style={{ opacity: slider / 100 }} />
          </div>
          <input type="range" min={0} max={100} value={slider} onChange={(e) => setSlider(Number(e.target.value))} className="block w-64 mt-2" />
        </div>
      )}
    </div>
  );
}

type EnrichedLine = Line & { wordParts?: { value: string; added?: boolean; removed?: boolean }[] };

function applyWordDiff(lines: Line[], path: string): EnrichedLine[] {
  const isProseOrCode = /\.(md|markdown|txt|rst)$/i.test(path);
  const out: EnrichedLine[] = lines.map((l) => ({ ...l }));
  for (let i = 0; i < out.length - 1; i++) {
    const a = out[i];
    const b = out[i + 1];
    if (a.kind === "del" && b.kind === "add") {
      const sim = similarity(a.content, b.content);
      if (isProseOrCode || sim > 0.5) {
        const parts = diffWordsWithSpace(a.content, b.content);
        a.wordParts = parts.filter((p) => !p.added).map((p) => ({ value: p.value, removed: p.removed }));
        b.wordParts = parts.filter((p) => !p.removed).map((p) => ({ value: p.value, added: p.added }));
      }
    }
  }
  return out;
}

function similarity(a: string, b: string): number {
  if (!a && !b) return 1;
  const la = a.length, lb = b.length;
  if (!la || !lb) return 0;
  // quick Jaccard over char bigrams
  const bigrams = (s: string) => { const set = new Set<string>(); for (let i = 0; i < s.length - 1; i++) set.add(s.slice(i, i + 2)); return set; };
  const A = bigrams(a); const B = bigrams(b);
  let inter = 0; A.forEach((x) => { if (B.has(x)) inter++; });
  return (2 * inter) / (A.size + B.size || 1);
}

function LineRow({ l, onComment, onJump, coverage }: { l: EnrichedLine; onComment: (line: number) => void; onJump: (p: string) => void; coverage?: Record<string, "covered" | "uncovered" | "none"> }) {
  const covState = l.new_line != null && coverage ? coverage[String(l.new_line)] : undefined;
  const bg = l.kind === "add" ? "bg-green-50" : l.kind === "del" ? "bg-red-50" : "";
  const marker = l.kind === "add" ? "+" : l.kind === "del" ? "-" : " ";
  const ln = l.new_line ?? l.old_line;
  const content = l.wordParts ? (
    <>
      {l.wordParts.map((p, i) => (
        <span key={i} className={p.added ? "bg-green-200" : p.removed ? "bg-red-200 line-through" : ""}>{p.value}</span>
      ))}
    </>
  ) : l.content;
  return (
    <tr className={bg}>
      <td className="text-right pr-2 pl-2 text-slate-400 w-12 select-none">{l.old_line ?? ""}</td>
      <td className="text-right pr-2 text-slate-400 w-12 select-none">{l.new_line ?? ""}</td>
      <td className="w-4 text-slate-400 select-none">{marker}</td>
      <td className="w-2 select-none"><CoverageGutter state={covState} /></td>
      <td className="whitespace-pre-wrap break-all pr-2">
        {content}
        {l.moved && (
          <button
            className="ml-2 text-[10px] px-1 border rounded text-slate-600 hover:bg-slate-100"
            onClick={() => onJump(l.moved!.path)}
            title={`Jump to ${l.moved.path}:${l.moved.line}`}
          >
            {l.moved.kind === "moved-from" ? "Moved from" : "Moved to"} {l.moved.path}:{l.moved.line}
          </button>
        )}
      </td>
      <td className="w-8 text-right pr-2">
        {ln != null && l.kind !== "del" && (
          <button className="text-slate-400 hover:text-slate-800" onClick={() => onComment(ln!)} title="Add comment">+</button>
        )}
      </td>
    </tr>
  );
}
