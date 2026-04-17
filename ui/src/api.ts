export type MovedLink = { kind: "moved-from" | "moved-to"; path: string; line: number };
export type Line = { kind: "add" | "del" | "ctx"; content: string; old_line: number | null; new_line: number | null; moved?: MovedLink | null };
export type Hunk = { header: string; old_start: number; new_start: number; lines: Line[] };
export type Stats = { added: number; removed: number };
export type ImagePayload = { old_data_url: string | null; new_data_url: string | null };
export type FileDiff = {
  path: string;
  old_path: string | null;
  status: string;
  similarity?: number | null;
  hunks: Hunk[];
  is_generated?: boolean;
  is_large?: boolean;
  is_binary?: boolean;
  is_image?: boolean;
  stats?: Stats;
  image?: ImagePayload | null;
};
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
  diffFile: (n: number, path: string, expand = true) =>
    getq(`/api/pr/${n}/diff/file?path=${encodeURIComponent(path)}&expand=${expand}`).then((r) => j<FileDiff>(r)),
  collaborators: (n: number) =>
    getq(`/api/pr/${n}/collaborators`).then((r) => j<{ logins: string[] }>(r)),
  getDraft: (n: number) => getq(`/api/pr/${n}/draft`).then((r) => j<Draft>(r)),
  putDraft: (n: number, d: Draft) => mut(`/api/pr/${n}/draft`, "PUT", d).then((r) => j<Draft>(r)),
  addComment: (n: number, c: { path: string; line: number; side?: string; body: string }) =>
    mut(`/api/pr/${n}/comments`, "POST", c).then((r) => j<Draft>(r)),
  setFileState: (n: number, path: string, s: Partial<FileState>) =>
    mut(`/api/pr/${n}/file-state`, "PUT", { path, ...s }).then((r) => j<Draft>(r)),
  submit: (n: number) =>
    mut(`/api/pr/${n}/submit`, "POST").then((r) => j<{ ok: boolean; result: unknown }>(r)),
  openInEditor: (body: { path: string; line?: number; col?: number }) =>
    mut(`/api/editor/open`, "POST", body).then((r) => j<{ ok: boolean }>(r)),
  framing: (n: number) =>
    getq(`/api/pr/${n}/framing`).then((r) => j<Framing>(r)),
  coverage: (n: number) =>
    getq(`/api/pr/${n}/coverage`).then((r) => j<CoverageResp>(r)),
};

export type FileFrame = { path: string; owned: boolean; touched_before: boolean; expertise_score: number };
export type Framing = { reviewer: { login: string | null; email: string | null }; files: FileFrame[] };

export type CoverageLineState = "covered" | "uncovered" | "none";
export type CoverageFileEntry = {
  lines: Record<string, CoverageLineState>;
  delta: { added_covered: number; added_uncovered: number; percent_before: number | null; percent_after: number | null };
};
export type CoverageResp = {
  files: Record<string, CoverageFileEntry>;
  summary: { new_uncovered_lines: number; files_with_uncovered: number };
  source: "ci" | "local" | null;
};

export type WhyLinkedIssue = { number: number; title: string; url: string; state: string };
export type WhyExternalLink = { text: string; url: string };
export type WhyPriorPr = { number: number; title: string; url: string };
export type WhyBlameLine = { line: number; author: string; commit_sha: string; commit_msg: string; date: string };
export type WhyTrace = {
  pr_description_section: string | null;
  linked_issues: WhyLinkedIssue[];
  external_links: WhyExternalLink[];
  prior_prs: WhyPriorPr[];
  blame: WhyBlameLine[];
  release_notes: string | null;
};

export async function apiWhy(n: number, path: string, line: number, count: number): Promise<WhyTrace> {
  const q = `path=${encodeURIComponent(path)}&line=${line}&count=${count}`;
  const r = await fetch(`/api/pr/${n}/why?${q}`, { credentials: "omit" });
  return j<WhyTrace>(r);
}
