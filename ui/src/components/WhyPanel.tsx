import { useQuery } from "@tanstack/react-query";
import { apiWhy, type WhyTrace } from "../api";

type Props = {
  prNumber: number;
  path: string;
  line: number;
  count: number;
  onClose: () => void;
};

export function WhyPanel({ prNumber, path, line, count, onClose }: Props) {
  const q = useQuery<WhyTrace>({
    queryKey: ["why", prNumber, path, line, count],
    queryFn: () => apiWhy(prNumber, path, line, count),
  });

  return (
    <div className="fixed inset-y-0 right-0 w-[480px] bg-white border-l shadow-xl z-40 flex flex-col">
      <div className="px-4 py-2 border-b flex items-center gap-2">
        <div className="text-sm font-semibold flex-1">Why did this change?</div>
        <div className="text-xs text-slate-500 font-mono truncate max-w-[180px]" title={`${path}:${line}`}>
          {path}:{line}
        </div>
        <button className="px-2 py-1 text-sm border rounded" onClick={onClose}>Close</button>
      </div>
      <div className="flex-1 overflow-y-auto p-4 text-sm space-y-5">
        {q.isLoading && <div className="text-slate-500">Loading…</div>}
        {q.isError && <div className="text-red-600">error: {String(q.error)}</div>}
        {q.data && <WhyBody data={q.data} />}
      </div>
    </div>
  );
}

function WhyBody({ data }: { data: WhyTrace }) {
  const empty =
    !data.pr_description_section &&
    data.linked_issues.length === 0 &&
    data.external_links.length === 0 &&
    data.prior_prs.length === 0 &&
    data.blame.length === 0 &&
    !data.release_notes;
  if (empty) return <div className="text-slate-500">No context found for this hunk.</div>;

  return (
    <>
      {data.pr_description_section && (
        <Section title="From PR description">
          <pre className="whitespace-pre-wrap font-sans text-xs bg-slate-50 p-2 rounded border">
            {data.pr_description_section}
          </pre>
        </Section>
      )}
      {data.linked_issues.length > 0 && (
        <Section title="Linked issues">
          <div className="flex flex-wrap gap-2">
            {data.linked_issues.map((i) => (
              <a
                key={`${i.url}-${i.number}`}
                href={i.url}
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full border bg-slate-50 hover:bg-slate-100 text-xs"
                title={i.title}
              >
                <span className="font-mono">#{i.number}</span>
                <span className="truncate max-w-[200px]">{i.title}</span>
                {i.state && <span className="text-slate-500">· {i.state.toLowerCase()}</span>}
              </a>
            ))}
          </div>
        </Section>
      )}
      {data.external_links.length > 0 && (
        <Section title="External links">
          <ul className="list-disc pl-5 space-y-1">
            {data.external_links.map((l) => (
              <li key={l.url}>
                <a className="text-blue-700 underline" href={l.url} target="_blank" rel="noreferrer">
                  {l.text || l.url}
                </a>
              </li>
            ))}
          </ul>
        </Section>
      )}
      {data.prior_prs.length > 0 && (
        <Section title="Prior PRs touching this code">
          <ul className="space-y-1">
            {data.prior_prs.map((p) => (
              <li key={p.number}>
                <a className="text-blue-700 underline" href={p.url} target="_blank" rel="noreferrer">
                  #{p.number}
                </a>{" "}
                <span className="text-slate-700">{p.title}</span>
              </li>
            ))}
          </ul>
        </Section>
      )}
      {data.blame.length > 0 && (
        <Section title="Blame">
          <table className="w-full text-xs">
            <tbody>
              {data.blame.map((b, i) => (
                <tr key={i} className="border-b last:border-0">
                  <td className="pr-2 text-right text-slate-400 font-mono w-10">{b.line}</td>
                  <td className="pr-2 font-mono text-slate-600">{b.commit_sha.slice(0, 7)}</td>
                  <td className="pr-2 whitespace-nowrap text-slate-700">{b.author}</td>
                  <td className="pr-2 text-slate-500 whitespace-nowrap">{relative(b.date)}</td>
                  <td className="truncate">{b.commit_msg}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Section>
      )}
      {data.release_notes && (
        <Section title="Release notes">
          <pre className="whitespace-pre-wrap font-sans text-xs bg-slate-50 p-2 rounded border">
            {data.release_notes}
          </pre>
        </Section>
      )}
    </>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="text-xs font-semibold uppercase tracking-wide text-slate-500 mb-1">{title}</div>
      {children}
    </div>
  );
}

function relative(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso).getTime();
  if (!isFinite(d)) return iso;
  const diff = Date.now() - d;
  const s = Math.floor(diff / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const days = Math.floor(h / 24);
  if (days < 30) return `${days}d ago`;
  const mo = Math.floor(days / 30);
  if (mo < 12) return `${mo}mo ago`;
  return `${Math.floor(mo / 12)}y ago`;
}
