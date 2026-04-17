import { Link, useParams } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { api, type Draft } from "../api";
import { CommentComposer } from "../components/CommentComposer";
import { PrWatchBanner } from "../components/PrWatchBanner";

export function PrReview() {
  const { number } = useParams({ from: "/pr/$number/review" });
  const n = Number(number);
  const qc = useQueryClient();
  const draftQ = useQuery({ queryKey: ["draft", n], queryFn: () => api.getDraft(n) });

  const [body, setBody] = useState("");
  const [verdict, setVerdict] = useState<string>("comment");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => {
    if (draftQ.data) {
      setBody(draftQ.data.body || "");
      setVerdict(draftQ.data.verdict || "comment");
    }
  }, [draftQ.data]);

  useEffect(() => {
    if (!draftQ.data) return;
    const t = setTimeout(() => {
      api.putDraft(n, { ...draftQ.data!, body, verdict }).catch(() => {});
    }, 300);
    return () => clearTimeout(t);
  }, [body, verdict, n, draftQ.data]);

  const save = async (): Promise<Draft | undefined> => {
    if (!draftQ.data) return;
    const d = { ...draftQ.data, body, verdict };
    return api.putDraft(n, d);
  };

  const submit = async () => {
    setBusy(true); setMsg(null);
    try {
      await save();
      await api.submit(n);
      setMsg("submitted");
      qc.invalidateQueries({ queryKey: ["draft", n] });
    } catch (e: any) {
      setMsg(`failed: ${e.message || e}`);
    } finally {
      setBusy(false);
    }
  };

  if (draftQ.isLoading) return <div className="p-6">loading…</div>;
  const comments = draftQ.data?.comments ?? [];

  return (
    <>
    <PrWatchBanner prNumber={n} />
    <div className="p-6 max-w-3xl">
      <div className="flex items-center gap-3 mb-4">
        <Link to="/pr/$number" params={{ number: String(n) }} className="text-sm text-slate-500">&larr; overview</Link>
        <Link to="/pr/$number/files" params={{ number: String(n) }} className="text-sm text-slate-500">files</Link>
      </div>
      <h2 className="text-xl font-semibold mb-4">Submit review</h2>

      <div className="mb-4">
        <div className="text-sm font-medium mb-1">Verdict</div>
        <div className="flex gap-4 text-sm">
          {["comment", "approve", "request_changes"].map((v) => (
            <label key={v} className="flex items-center gap-1">
              <input type="radio" name="verdict" value={v} checked={verdict === v} onChange={() => setVerdict(v)} />
              {v}
            </label>
          ))}
        </div>
      </div>

      <div className="mb-4">
        <div className="text-sm font-medium mb-1">Summary</div>
        <CommentComposer value={body} onChange={setBody} prNumber={n} rows={8} />
      </div>

      <div className="mb-4">
        <div className="text-sm font-medium mb-1">Draft comments ({comments.length})</div>
        <ul className="divide-y border rounded">
          {comments.map((c) => (
            <li key={c.id} className="p-2 text-sm">
              <div className="text-xs text-slate-500 font-mono">{c.path}:{c.line}</div>
              <div className="whitespace-pre-wrap">{c.body}</div>
            </li>
          ))}
          {comments.length === 0 && <li className="p-2 text-sm text-slate-500">none</li>}
        </ul>
      </div>

      <div className="flex gap-2">
        <button onClick={save} disabled={busy} className="px-3 py-1.5 border rounded text-sm">Save draft</button>
        <button onClick={submit} disabled={busy} className="px-3 py-1.5 bg-slate-900 text-white rounded text-sm">
          {busy ? "submitting…" : "Submit to GitHub"}
        </button>
      </div>
      {msg && <div className="mt-3 text-sm">{msg}</div>}
    </div>
    </>
  );
}
