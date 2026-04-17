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

const CSRF = (() => {
  const el = document.querySelector('meta[name="tldr-csrf"]') as HTMLMetaElement | null;
  return el?.content ?? "";
})();

async function j<T>(r: Response): Promise<T> {
  if (!r.ok) throw new Error(`${r.status} ${await r.text()}`);
  return r.json();
}

function mut(url: string, method: string, body?: unknown): Promise<Response> {
  const headers: Record<string, string> = { "x-tldr-csrf": CSRF };
  if (body !== undefined) headers["content-type"] = "application/json";
  return fetch(url, {
    method,
    headers,
    credentials: "omit",
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
}

function getq(url: string): Promise<Response> {
  return fetch(url, { credentials: "omit" });
}

export const api = {
  pr: (n: number) => getq(`/api/pr/${n}`).then((r) => j<{ pr: PrMeta; worktree: string; slug: string }>(r)),
  diff: (n: number) => getq(`/api/pr/${n}/diff`).then((r) => j<Diff>(r)),
  getDraft: (n: number) => getq(`/api/pr/${n}/draft`).then((r) => j<Draft>(r)),
  putDraft: (n: number, d: Draft) => mut(`/api/pr/${n}/draft`, "PUT", d).then((r) => j<Draft>(r)),
  addComment: (n: number, c: { path: string; line: number; side?: string; body: string }) =>
    mut(`/api/pr/${n}/comments`, "POST", c).then((r) => j<Draft>(r)),
  setFileState: (n: number, path: string, s: Partial<FileState>) =>
    mut(`/api/pr/${n}/files/${encodeURI(path)}/state`, "PUT", s).then((r) => j<Draft>(r)),
  submit: (n: number) =>
    mut(`/api/pr/${n}/submit`, "POST").then((r) => j<{ ok: boolean; result: unknown }>(r)),
  openInEditor: (body: { path: string; line?: number; col?: number }) =>
    mut(`/api/editor/open`, "POST", body).then((r) => j<{ ok: boolean }>(r)),
};
