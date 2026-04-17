import { useParams, useSearch, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";

type HunkRef = { path: string; line: number };
type Step = { id: string; prose: string; hunk_refs: HunkRef[]; mermaid?: string | null };
type Cached = { pr_number: number; head_sha: string; steps: Step[]; usage: { input_tokens: number; output_tokens: number; provider: string; model: string }; generated_at: string };
type Status = { enabled?: boolean; reason?: string; provider?: string; model?: string; context_kb?: number; confirmed?: boolean; repo_enabled?: boolean | null };

const CSRF = (() => {
  const el = document.querySelector('meta[name="tldr-csrf"]') as HTMLMetaElement | null;
  return el?.content ?? "";
})();

async function fetchStatus(): Promise<Status> {
  const r = await fetch("/api/ai/status", { credentials: "omit" });
  return r.json();
}
async function fetchWalkthrough(n: number): Promise<{ status: string; walkthrough?: Cached; enabled?: boolean; reason?: string }> {
  const r = await fetch(`/api/pr/${n}/walkthrough`, { credentials: "omit" });
  return r.json();
}

export function PrWalkthrough() {
  const { number } = useParams({ from: "/pr/$number/walkthrough" });
  const search = useSearch({ from: "/pr/$number/walkthrough" }) as { view?: string };
  const view = search.view === "graph" ? "graph" : "list";
  const n = Number(number);
  const statusQ = useQuery({ queryKey: ["ai-status"], queryFn: fetchStatus });
  const wQ = useQuery({ queryKey: ["walkthrough", n], queryFn: () => fetchWalkthrough(n) });

  const [steps, setSteps] = useState<Step[]>([]);
  const [streaming, setStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showPrivacy, setShowPrivacy] = useState(false);
  const [selected, setSelected] = useState<number>(0);
  const iframeRef = useRef<HTMLIFrameElement | null>(null);

  useEffect(() => {
    if (wQ.data?.walkthrough) setSteps(wQ.data.walkthrough.steps);
  }, [wQ.data]);

  const disabled = wQ.data?.enabled === false || statusQ.data?.enabled === false;
  const repoDisabled = wQ.data?.reason === "disabled by repo config";

  async function confirmPrivacy() {
    await fetch("/api/ai/confirm", { method: "POST", headers: { "x-tldr-csrf": CSRF }, credentials: "omit" });
    setShowPrivacy(false);
    statusQ.refetch();
  }

  async function startGenerate() {
    if (!statusQ.data?.confirmed) { setShowPrivacy(true); return; }
    setSteps([]);
    setError(null);
    setStreaming(true);
    try {
      const resp = await fetch(`/api/pr/${n}/walkthrough/generate`, {
        method: "POST",
        headers: { "x-tldr-csrf": CSRF, accept: "text/event-stream" },
        credentials: "omit",
      });
      if (!resp.ok || !resp.body) {
        setError(`HTTP ${resp.status}`);
        setStreaming(false);
        return;
      }
      const ct = resp.headers.get("content-type") ?? "";
      if (!ct.includes("text/event-stream")) {
        const body = await resp.json();
        setError(body.reason ?? "not available");
        setStreaming(false);
        return;
      }
      const reader = resp.body.getReader();
      const dec = new TextDecoder();
      let buf = "";
      let currentEvent = "message";
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += dec.decode(value, { stream: true });
        let idx: number;
        while ((idx = buf.indexOf("\n\n")) !== -1) {
          const block = buf.slice(0, idx);
          buf = buf.slice(idx + 2);
          let ev = "message";
          const dataLines: string[] = [];
          for (const ln of block.split("\n")) {
            if (ln.startsWith("event:")) ev = ln.slice(6).trim();
            else if (ln.startsWith("data:")) dataLines.push(ln.slice(5).trim());
          }
          const dataStr = dataLines.join("\n");
          currentEvent = ev;
          if (ev === "step" && dataStr) {
            try { const s: Step = JSON.parse(dataStr); setSteps((prev) => [...prev, s]); } catch {}
          } else if (ev === "error") {
            setError(dataStr);
          } else if (ev === "done" && dataStr) {
            try { const c: Cached = JSON.parse(dataStr); if (c.steps.length) setSteps(c.steps); } catch {}
          }
        }
      }
      void currentEvent;
    } catch (e) {
      setError(String(e));
    } finally {
      setStreaming(false);
    }
  }

  function jumpTo(step: Step, i: number) {
    setSelected(i);
    const ref = step.hunk_refs[0];
    if (!ref || !iframeRef.current) return;
    const url = `/pr/${n}/files#${encodeURIComponent(ref.path)}:${ref.line}`;
    iframeRef.current.src = url;
  }

  if (repoDisabled) {
    return (
      <div className="p-6 max-w-2xl">
        <Banner />
        <p className="mt-4 text-slate-600">AI walkthrough is disabled by <code>.tldr/config.toml</code> in this repo.</p>
      </div>
    );
  }

  return (
    <div className="flex h-screen">
      <aside className="w-96 border-r overflow-y-auto p-4">
        <Banner />
        <div className="mt-3 flex gap-2">
          <Link to="/pr/$number" params={{ number: String(n) }} className="text-sm text-slate-600 underline">← back</Link>
          <Link to="/pr/$number/walkthrough" params={{ number: String(n) }} search={{ view: view === "graph" ? undefined : "graph" } as any} className="text-sm text-slate-600 underline">
            {view === "graph" ? "list view" : "graph view"}
          </Link>
        </div>
        {disabled && (
          <p className="mt-3 text-sm text-slate-600">AI disabled. Set <code>[ai] enabled = true</code> in config.</p>
        )}
        {!disabled && (
          <div className="mt-4">
            <button
              onClick={startGenerate}
              disabled={streaming}
              className="px-3 py-1.5 bg-slate-900 text-white rounded text-sm disabled:opacity-50"
            >
              {streaming ? "generating…" : steps.length ? "Regenerate" : "Start walkthrough"}
            </button>
            {statusQ.data?.provider && (
              <span className="ml-3 text-xs text-slate-500">
                {statusQ.data.provider} / {statusQ.data.model}
              </span>
            )}
          </div>
        )}
        {error && <p className="mt-3 text-sm text-red-600">{error}</p>}
        {view === "list" && (
          <ol className="mt-4 space-y-3">
            {steps.map((s, i) => (
              <li key={s.id} className={`border rounded p-3 cursor-pointer ${selected === i ? "border-slate-900 bg-slate-50" : "border-slate-200"}`} onClick={() => jumpTo(s, i)}>
                <div className="text-xs text-slate-500">Step {i + 1}</div>
                <p className="text-sm mt-1">{s.prose}</p>
                {s.hunk_refs.length > 0 && (
                  <div className="mt-2 text-xs text-slate-500">
                    {s.hunk_refs.map((h, j) => (
                      <span key={j} className="mr-2">{h.path}:{h.line}</span>
                    ))}
                  </div>
                )}
              </li>
            ))}
            {!steps.length && !streaming && <li className="text-sm text-slate-500">No walkthrough yet.</li>}
          </ol>
        )}
        {view === "graph" && <GraphView steps={steps} onSelect={(i) => jumpTo(steps[i], i)} />}
      </aside>
      <main className="flex-1">
        <iframe ref={iframeRef} title="diff" src={`/pr/${n}/files`} className="w-full h-full border-0" />
      </main>
      {showPrivacy && (
        <PrivacyModal status={statusQ.data} onConfirm={confirmPrivacy} onCancel={() => setShowPrivacy(false)} />
      )}
    </div>
  );
}

function Banner() {
  return (
    <div className="rounded border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-900">
      AI-generated summary. Not a review. Not a verdict.
    </div>
  );
}

function PrivacyModal({ status, onConfirm, onCancel }: { status?: Status; onConfirm: () => void; onCancel: () => void }) {
  return (
    <div className="fixed inset-0 bg-black/30 flex items-center justify-center z-50">
      <div className="bg-white rounded shadow-lg max-w-md p-5">
        <h2 className="text-lg font-semibold">Share PR data with AI provider?</h2>
        <p className="text-sm mt-2 text-slate-700">
          Provider: <b>{status?.provider ?? "unknown"}</b><br />
          Will see: PR diff, description, up to {status?.context_kb ?? 32} KB file context.
        </p>
        <p className="text-xs mt-2 text-slate-500">Stored once per repo in your global config.</p>
        <div className="mt-4 flex justify-end gap-2">
          <button className="px-3 py-1.5 border rounded text-sm" onClick={onCancel}>Cancel</button>
          <button className="px-3 py-1.5 bg-slate-900 text-white rounded text-sm" onClick={onConfirm}>Confirm</button>
        </div>
      </div>
    </div>
  );
}

function GraphView({ steps, onSelect }: { steps: Step[]; onSelect: (i: number) => void }) {
  // Minimal inline SVG "graph": nodes top-to-bottom connected by edges.
  const H = 72;
  return (
    <svg width="100%" height={Math.max(steps.length * H + 20, 100)} className="mt-4">
      {steps.map((s, i) => (
        <g key={s.id} onClick={() => onSelect(i)} style={{ cursor: "pointer" }}>
          <rect x={10} y={i * H + 10} width={320} height={56} rx={6} className="fill-white stroke-slate-400" />
          <text x={22} y={i * H + 30} className="fill-slate-900 text-xs font-semibold">Step {i + 1}</text>
          <text x={22} y={i * H + 52} className="fill-slate-700 text-xs">
            {s.prose.slice(0, 60)}{s.prose.length > 60 ? "…" : ""}
          </text>
          {i > 0 && <line x1={170} y1={i * H + 10} x2={170} y2={i * H} className="stroke-slate-400" />}
        </g>
      ))}
    </svg>
  );
}
