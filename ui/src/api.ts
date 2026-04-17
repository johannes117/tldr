export type Line = { kind: "add" | "del" | "ctx"; content: string; old_line: number | null; new_line: number | null };
export type Hunk = { header: string; old_start: number; new_start: number; lines: Line[] };
export type FileDiff = { path: string; old_path: string | null; status: string; hunks: Hunk[] };
export type Diff = { files: FileDiff[] };

export type PrMeta = {
  number: number; title: string; body: string | null; state: string;
  head_sha: string; base_sha: string; head_ref: string; base_ref: string;
  node_id: string; author: string | null; html_url: string;
};

export type FileState = { viewed: boolean; collapsed: boolean };
export type DraftComment = { id: string; path: string; line: number; side: string; body: string; created_at: string };
export type Draft = {
  pr: number; body: string; verdict: string | null;
  comments: DraftComment[]; file_state: Record<string, FileState>;
  updated_at: string;
};

async function j<T>(r: Response): Promise<T> {
  if (!r.ok) throw new Error(`${r.status} ${await r.text()}`);
  return r.json();
}

export const api = {
  pr: (n: number) => fetch(`/api/pr/${n}`).then((r) => j<{ pr: PrMeta; worktree: string; slug: string }>(r)),
  diff: (n: number) => fetch(`/api/pr/${n}/diff`).then((r) => j<Diff>(r)),
  getDraft: (n: number) => fetch(`/api/pr/${n}/draft`).then((r) => j<Draft>(r)),
  putDraft: (n: number, d: Draft) =>
    fetch(`/api/pr/${n}/draft`, { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(d) }).then((r) => j<Draft>(r)),
  addComment: (n: number, c: { path: string; line: number; side?: string; body: string }) =>
    fetch(`/api/pr/${n}/comments`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(c) }).then((r) => j<Draft>(r)),
  setFileState: (n: number, path: string, s: Partial<FileState>) =>
    fetch(`/api/pr/${n}/files/${encodeURI(path)}/state`, { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(s) }).then((r) => j<Draft>(r)),
  submit: (n: number) =>
    fetch(`/api/pr/${n}/submit`, { method: "POST" }).then((r) => j<{ ok: boolean; result: unknown }>(r)),
};
