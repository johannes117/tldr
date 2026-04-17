import { Link, useParams } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api";

export function PrOverview() {
  const { number } = useParams({ from: "/pr/$number" });
  const n = Number(number);
  const q = useQuery({ queryKey: ["pr", n], queryFn: () => api.pr(n) });

  if (q.isLoading) return <div className="p-6">loading...</div>;
  if (q.isError) return <div className="p-6 text-red-600">error: {String(q.error)}</div>;
  const { pr, slug, worktree } = q.data!;

  return (
    <div className="p-6 max-w-3xl">
      <div className="text-sm text-slate-500">{slug} #{pr.number}</div>
      <h1 className="text-2xl font-semibold mt-1">{pr.title}</h1>
      <div className="text-sm text-slate-600 mt-1">by {pr.author} · {pr.base_ref} ← {pr.head_ref}</div>
      <div className="mt-4 prose whitespace-pre-wrap">{pr.body || "(no description)"}</div>
      <div className="mt-6 flex gap-3">
        <Link to="/pr/$number/files" params={{ number: String(n) }} className="px-3 py-1.5 bg-slate-900 text-white rounded">Files</Link>
        <Link to="/pr/$number/review" params={{ number: String(n) }} className="px-3 py-1.5 border rounded">Review</Link>
        <a href={pr.html_url} target="_blank" rel="noreferrer" className="px-3 py-1.5 border rounded">Open on GitHub</a>
      </div>
      <div className="mt-6 text-xs text-slate-500">Worktree: <code>{worktree}</code></div>
    </div>
  );
}
