import { useMemo } from "react";
import type { FileDiff, FileFrame, Draft } from "../api";

export function FileTree({
  files,
  framing,
  draft,
  idx,
  onSelect,
}: {
  files: FileDiff[];
  framing: FileFrame[] | undefined;
  draft: Draft | undefined;
  idx: number;
  onSelect: (i: number) => void;
}) {
  const byPath = useMemo(() => {
    const m = new Map<string, FileFrame>();
    for (const f of framing ?? []) m.set(f.path, f);
    return m;
  }, [framing]);

  const ordered = useMemo(() => {
    const indexed = files.map((f, i) => ({ f, i, fr: byPath.get(f.path) }));
    indexed.sort((a, b) => {
      const ao = a.fr?.owned ? 1 : 0;
      const bo = b.fr?.owned ? 1 : 0;
      if (ao !== bo) return bo - ao;
      const at = a.fr?.touched_before ? 1 : 0;
      const bt = b.fr?.touched_before ? 1 : 0;
      if (at !== bt) return bt - at;
      return a.f.path.localeCompare(b.f.path);
    });
    return indexed;
  }, [files, byPath]);

  return (
    <ul>
      {ordered.map(({ f, i, fr }) => {
        const viewed = draft?.file_state?.[f.path]?.viewed;
        return (
          <li key={f.path}>
            <button
              onClick={() => onSelect(i)}
              className={`w-full text-left px-3 py-1.5 text-sm truncate ${i === idx ? "bg-slate-200" : "hover:bg-slate-100"}`}
              title={f.path}
            >
              {fr?.owned && <span className="mr-1" title="You own this file (CODEOWNERS)">👤</span>}
              {fr?.touched_before && <span className="mr-1" title="You've touched this file before">📝</span>}
              <span className={viewed ? "line-through text-slate-400" : ""}>{f.path}</span>
              {f.is_generated && <span className="ml-1 text-[10px] text-purple-600">gen</span>}
              {f.is_large && <span className="ml-1 text-[10px] text-orange-600">large</span>}
              {f.status === "renamed" && <span className="ml-1 text-[10px] text-blue-600">ren</span>}
              {f.is_image && <span className="ml-1 text-[10px] text-teal-600">img</span>}
            </button>
          </li>
        );
      })}
    </ul>
  );
}
