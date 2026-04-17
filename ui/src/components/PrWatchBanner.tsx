import { useEffect, useState } from "react";

type WatchEvent =
  | { type: "NewCommits"; old_head: string; new_head: string; count: number }
  | { type: "NewComments"; count: number }
  | { type: "CiStatusChange"; state: string }
  | { type: "StateChange"; state: string };

export function PrWatchBanner({ prNumber }: { prNumber: number }) {
  const [evt, setEvt] = useState<WatchEvent | null>(null);
  const [author, setAuthor] = useState<string>("Author");

  useEffect(() => {
    fetch(`/api/pr/${prNumber}`, { credentials: "omit" })
      .then((r) => r.json())
      .then((d) => { if (d?.pr?.author) setAuthor(d.pr.author); })
      .catch(() => {});
  }, [prNumber]);

  useEffect(() => {
    const es = new EventSource(`/api/pr/${prNumber}/events`);
    es.onmessage = (m) => {
      try { setEvt(JSON.parse(m.data)); } catch {}
    };
    es.onerror = () => { es.close(); };
    return () => es.close();
  }, [prNumber]);

  if (!evt) return null;

  if (evt.type === "NewCommits") {
    return (
      <div className="bg-amber-100 border-b border-amber-300 px-4 py-2 text-sm flex items-center gap-3">
        <span>{author} pushed {evt.count} commit{evt.count === 1 ? "" : "s"}.</span>
        <button
          className="px-2 py-0.5 bg-slate-900 text-white rounded text-xs"
          onClick={() => window.location.reload()}
        >
          Update
        </button>
        <button className="ml-auto text-slate-500" onClick={() => setEvt(null)}>dismiss</button>
      </div>
    );
  }
  if (evt.type === "NewComments") {
    return (
      <div className="bg-sky-100 border-b border-sky-300 px-4 py-2 text-sm flex items-center gap-3">
        <span>{evt.count} new comment{evt.count === 1 ? "" : "s"} on GitHub.</span>
        <button className="ml-auto text-slate-500" onClick={() => setEvt(null)}>dismiss</button>
      </div>
    );
  }
  if (evt.type === "CiStatusChange") {
    return (
      <div className="bg-slate-100 border-b px-4 py-2 text-sm flex items-center gap-3">
        <span>CI status: {evt.state}</span>
        <button className="ml-auto text-slate-500" onClick={() => setEvt(null)}>dismiss</button>
      </div>
    );
  }
  if (evt.type === "StateChange") {
    return (
      <div className="bg-violet-100 border-b border-violet-300 px-4 py-2 text-sm flex items-center gap-3">
        <span>PR state changed to {evt.state}.</span>
        <button className="ml-auto text-slate-500" onClick={() => setEvt(null)}>dismiss</button>
      </div>
    );
  }
  return null;
}
