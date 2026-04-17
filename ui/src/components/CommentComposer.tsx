import { useEffect, useMemo, useRef, useState } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { api } from "../api";

// v1 limitation: GitHub has no public image upload API. We insert data-URI
// base64 images into the markdown on drop; GitHub will accept them when the
// review is submitted, but users may prefer uploading via the GitHub UI for
// large images. Documented here per SPEC §7.5.10.

let highlighterP: Promise<any> | null = null;
async function getHighlighter() {
  if (!highlighterP) {
    highlighterP = import("shiki").then((s) =>
      s.createHighlighter({
        themes: ["github-light"],
        langs: ["ts", "tsx", "js", "jsx", "rust", "python", "go", "json", "bash", "md"],
      }),
    );
  }
  return highlighterP;
}

export function CommentComposer({
  value,
  onChange,
  prNumber,
  rows = 4,
  onSubmit,
  onCancel,
  autoFocus,
}: {
  value: string;
  onChange: (v: string) => void;
  prNumber: number;
  rows?: number;
  onSubmit?: () => void;
  onCancel?: () => void;
  autoFocus?: boolean;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const [previewHtml, setPreviewHtml] = useState("");
  const [mentionQ, setMentionQ] = useState<string | null>(null);
  const [mentionPos, setMentionPos] = useState<number>(0);
  const [collabs, setCollabs] = useState<string[]>([]);

  useEffect(() => {
    if (autoFocus) ref.current?.focus();
  }, [autoFocus]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const raw = marked.parse(value || "", { async: false }) as string;
      const clean = DOMPurify.sanitize(raw);
      // shiki pass over <pre><code class="language-*">
      try {
        const hl = await getHighlighter();
        const doc = new DOMParser().parseFromString(`<div>${clean}</div>`, "text/html");
        const codes = doc.querySelectorAll("pre > code");
        codes.forEach((el) => {
          const cls = el.getAttribute("class") || "";
          const m = cls.match(/language-([\w-]+)/);
          const lang = m?.[1] ?? "text";
          const code = el.textContent ?? "";
          if (lang === "suggestion") {
            el.parentElement!.outerHTML = `<pre class="bg-yellow-50 border border-yellow-300 rounded p-2"><code>${escapeHtml(
              code,
            )}</code><div class="text-xs text-yellow-700">suggestion</div></pre>`;
            return;
          }
          try {
            const html = hl.codeToHtml(code, { lang, theme: "github-light" });
            el.parentElement!.outerHTML = html;
          } catch {
            /* unsupported lang */
          }
        });
        if (!cancelled) setPreviewHtml(doc.body.firstChild ? (doc.body.firstChild as HTMLElement).innerHTML : clean);
      } catch {
        if (!cancelled) setPreviewHtml(clean);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [value]);

  useEffect(() => {
    if (mentionQ !== null && collabs.length === 0) {
      api.collaborators(prNumber).then((r) => setCollabs(r.logins)).catch(() => setCollabs([]));
    }
  }, [mentionQ, prNumber, collabs.length]);

  const onInput = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const v = e.target.value;
    onChange(v);
    const pos = e.target.selectionStart ?? v.length;
    // detect @word just typed
    const upto = v.slice(0, pos);
    const m = upto.match(/(^|\s)@(\w*)$/);
    if (m) {
      setMentionQ(m[2]);
      setMentionPos(pos - m[2].length);
    } else {
      setMentionQ(null);
    }
  };

  const pickMention = (login: string) => {
    const v = value;
    const before = v.slice(0, mentionPos);
    const after = v.slice(mentionPos + (mentionQ?.length ?? 0));
    const next = `${before}${login} ${after}`;
    onChange(next);
    setMentionQ(null);
    setTimeout(() => ref.current?.focus(), 0);
  };

  const mentions = useMemo(() => {
    if (mentionQ === null) return [];
    const q = mentionQ.toLowerCase();
    return collabs.filter((c) => c.toLowerCase().startsWith(q)).slice(0, 8);
  }, [mentionQ, collabs]);

  const onDrop = async (e: React.DragEvent<HTMLTextAreaElement>) => {
    const files = Array.from(e.dataTransfer.files).filter((f) => f.type.startsWith("image/"));
    if (files.length === 0) return;
    e.preventDefault();
    for (const f of files) {
      const dataUrl: string = await new Promise((res, rej) => {
        const r = new FileReader();
        r.onload = () => res(String(r.result));
        r.onerror = rej;
        r.readAsDataURL(f);
      });
      const ins = `\n![${f.name}](${dataUrl})\n`;
      onChange(value + ins);
    }
  };

  return (
    <div className="grid grid-cols-2 gap-2">
      <div className="relative">
        <textarea
          ref={ref}
          rows={rows}
          className="w-full border rounded p-2 text-sm font-mono"
          value={value}
          onChange={onInput}
          onDrop={onDrop}
          onDragOver={(e) => e.preventDefault()}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && onSubmit) {
              e.preventDefault();
              onSubmit();
            } else if (e.key === "Escape" && onCancel) {
              onCancel();
            }
          }}
          placeholder="Markdown supported. Drop images to insert as data URI. @mention collaborators."
        />
        {mentionQ !== null && mentions.length > 0 && (
          <ul className="absolute z-10 left-2 bottom-2 bg-white border rounded shadow text-xs">
            {mentions.map((m) => (
              <li key={m}>
                <button type="button" className="px-2 py-1 hover:bg-slate-100 block w-full text-left" onClick={() => pickMention(m)}>
                  @{m}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div
        className="border rounded p-2 text-sm prose prose-sm max-w-none overflow-auto"
        // preview is DOMPurify-sanitized
        dangerouslySetInnerHTML={{ __html: previewHtml }}
      />
    </div>
  );
}

function escapeHtml(s: string) {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]!));
}
