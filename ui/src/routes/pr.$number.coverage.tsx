import { Link, useParams } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api";

export function PrCoverage() {
  const { number } = useParams({ from: "/pr/$number/coverage" });
  const n = Number(number);
  const q = useQuery({ queryKey: ["coverage", n], queryFn: () => api.coverage(n) });

  if (q.isLoading) return <div className="p-6">loading coverage…</div>;
  if (q.isError) return <div className="p-6 text-red-600">error: {String(q.error)}</div>;
  const data = q.data;
  if (!data) return null;

  const entries = Object.entries(data.files);
  return (
    <div className="p-6 max-w-5xl">
      <div className="flex items-center gap-3 mb-4">
        <Link to="/pr/$number" params={{ number: String(n) }} className="text-sm text-slate-500">&larr; overview</Link>
        <h1 className="text-xl font-semibold">Coverage</h1>
        <span className="text-xs text-slate-500">
          source: {data.source ?? "none"}
        </span>
      </div>
      {!data.source && (
        <div className="mb-4 p-3 border rounded bg-yellow-50 text-sm text-yellow-900">
          No coverage data found. Add <code>coverage/lcov.info</code> locally or ensure CI uploads a coverage artifact.
        </div>
      )}
      <div className="mb-4 text-sm text-slate-700">
        New uncovered lines: <strong>{data.summary.new_uncovered_lines}</strong> across{" "}
        <strong>{data.summary.files_with_uncovered}</strong> file(s).
      </div>
      <table className="w-full text-sm border">
        <thead className="bg-slate-100">
          <tr>
            <th className="text-left p-2">File</th>
            <th className="text-right p-2">Added covered</th>
            <th className="text-right p-2">Added uncovered</th>
            <th className="text-right p-2">% after</th>
          </tr>
        </thead>
        <tbody>
          {entries.map(([path, f]) => (
            <tr key={path} className="border-t">
              <td className="p-2 font-mono truncate max-w-xl">{path}</td>
              <td className="p-2 text-right text-green-700">{f.delta.added_covered}</td>
              <td className="p-2 text-right text-red-700">{f.delta.added_uncovered}</td>
              <td className="p-2 text-right">{f.delta.percent_after != null ? `${f.delta.percent_after.toFixed(1)}%` : "—"}</td>
            </tr>
          ))}
          {entries.length === 0 && (
            <tr><td colSpan={4} className="p-4 text-center text-slate-500">no files</td></tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
