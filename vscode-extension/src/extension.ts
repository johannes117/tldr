import * as vscode from "vscode";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

interface SessionInfo {
  port: number;
  csrfToken: string;
  prNumber: number;
}

interface DraftComment {
  id: string;
  author?: string;
  body: string;
  path: string;
  line: number;
  parent_id?: string | null;
}

let session: SessionInfo | undefined;
let pollTimer: NodeJS.Timeout | undefined;
let commentDeco: vscode.TextEditorDecorationType;
let addedDeco: vscode.TextEditorDecorationType;
let removedDeco: vscode.TextEditorDecorationType;
let treeProvider: FilesTreeProvider;
let output: vscode.OutputChannel;
let commentCache: DraftComment[] = [];
let filesCache: { path: string; viewed: boolean }[] = [];

export function activate(context: vscode.ExtensionContext) {
  output = vscode.window.createOutputChannel("tldr");

  commentDeco = vscode.window.createTextEditorDecorationType({
    gutterIconPath: context.asAbsolutePath("media/comment.svg"),
    gutterIconSize: "contain",
    after: { margin: "0 0 0 1em", color: new vscode.ThemeColor("editorCodeLens.foreground") },
  });
  addedDeco = vscode.window.createTextEditorDecorationType({
    overviewRulerColor: "#2ea043",
    overviewRulerLane: vscode.OverviewRulerLane.Right,
  });
  removedDeco = vscode.window.createTextEditorDecorationType({
    overviewRulerColor: "#f85149",
    overviewRulerLane: vscode.OverviewRulerLane.Right,
  });

  treeProvider = new FilesTreeProvider();
  vscode.window.registerTreeDataProvider("tldrReview", treeProvider);

  context.subscriptions.push(
    vscode.commands.registerCommand("tldr.markViewed", markViewed),
    vscode.commands.registerCommand("tldr.addComment", addComment),
    vscode.commands.registerCommand("tldr.replyToComment", replyToComment),
    vscode.commands.registerCommand("tldr.openBrowser", openBrowser),
    vscode.commands.registerCommand("tldr.refresh", refreshAll),
    vscode.window.onDidChangeActiveTextEditor(() => applyDecorations()),
  );

  detectSession().then((s) => {
    session = s;
    if (!session) {
      output.appendLine("No tldr session detected for this workspace.");
      return;
    }
    output.appendLine(`Detected tldr PR #${session.prNumber} on port ${session.port}`);
    startPolling();
    refreshAll();
  });
}

export function deactivate() {
  if (pollTimer) clearInterval(pollTimer);
}

async function detectSession(): Promise<SessionInfo | undefined> {
  const cfg = vscode.workspace.getConfiguration("tldr");
  const overrideUrl = cfg.get<string>("serverUrl") || "";
  const overrideToken = cfg.get<string>("csrfToken") || "";

  const root = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  if (!root) return undefined;

  const m = root.match(/\/tldr\/repos\/([^/]+)\/worktrees\/pr-(\d+)(?:\/|$)/);
  if (!m) return undefined;
  const slug = m[1];
  const prNumber = parseInt(m[2], 10);

  if (overrideUrl) {
    const portMatch = overrideUrl.match(/:(\d+)/);
    return {
      port: portMatch ? parseInt(portMatch[1], 10) : 8787,
      csrfToken: overrideToken,
      prNumber,
    };
  }

  const stateDirs = [
    process.env.XDG_STATE_HOME,
    path.join(os.homedir(), ".local", "state"),
    path.join(os.homedir(), "Library", "Application Support"),
  ].filter(Boolean) as string[];

  for (const base of stateDirs) {
    const p = path.join(base, "tldr", "repos", slug, "session.json");
    if (fs.existsSync(p)) {
      try {
        const data = JSON.parse(fs.readFileSync(p, "utf8"));
        return {
          port: data.port,
          csrfToken: overrideToken || data.csrf_token || data.csrfToken || "",
          prNumber,
        };
      } catch (e) {
        output.appendLine(`Failed to parse ${p}: ${e}`);
      }
    }
  }
  return undefined;
}

function startPolling() {
  const interval = vscode.workspace.getConfiguration("tldr").get<number>("pollInterval", 5000);
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = setInterval(() => refreshAll().catch(() => {}), interval);
}

async function apiFetch(pathname: string, init?: RequestInit): Promise<Response> {
  if (!session) throw new Error("No session");
  const url = `http://127.0.0.1:${session.port}${pathname}`;
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...(init?.headers as Record<string, string> | undefined),
  };
  if (session.csrfToken) headers["X-CSRF-Token"] = session.csrfToken;
  return fetch(url, { ...init, headers });
}

async function refreshAll() {
  if (!session) return;
  try {
    const [draftRes, filesRes] = await Promise.all([
      apiFetch(`/api/pr/${session.prNumber}/draft`),
      apiFetch(`/api/pr/${session.prNumber}/files`),
    ]);
    if (draftRes.ok) {
      const data: any = await draftRes.json();
      commentCache = (data.comments || data.drafts || data || []) as DraftComment[];
    }
    if (filesRes.ok) {
      const data: any = await filesRes.json();
      filesCache = (data.files || data || []).map((f: any) => ({
        path: f.path || f.filename,
        viewed: !!f.viewed,
      }));
      treeProvider.refresh();
    }
    applyDecorations();
  } catch (e) {
    output.appendLine(`refresh failed: ${e}`);
  }
}

function applyDecorations() {
  const editor = vscode.window.activeTextEditor;
  if (!editor || !session) return;
  const relPath = vscode.workspace.asRelativePath(editor.document.uri);
  const relevant = commentCache.filter((c) => c.path === relPath);
  const ranges: vscode.DecorationOptions[] = relevant.map((c) => ({
    range: new vscode.Range(Math.max(0, c.line - 1), 0, Math.max(0, c.line - 1), 0),
    renderOptions: {
      after: { contentText: `  ${c.author || "draft"}: ${c.body.slice(0, 80)}` },
    },
    hoverMessage: new vscode.MarkdownString(`**${c.author || "draft"}**\n\n${c.body}`),
  }));
  editor.setDecorations(commentDeco, ranges);
  loadDiffDecorations(editor, relPath);
}

async function loadDiffDecorations(editor: vscode.TextEditor, relPath: string) {
  if (!session) return;
  try {
    const res = await apiFetch(`/api/pr/${session.prNumber}/diff?path=${encodeURIComponent(relPath)}`);
    if (!res.ok) return;
    const text = await res.text();
    const added: vscode.Range[] = [];
    const removed: vscode.Range[] = [];
    let line = 0;
    for (const raw of text.split("\n")) {
      const hunkMatch = raw.match(/^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      if (hunkMatch) {
        line = parseInt(hunkMatch[1], 10) - 1;
        continue;
      }
      if (raw.startsWith("+") && !raw.startsWith("+++")) {
        added.push(new vscode.Range(line, 0, line, 0));
        line++;
      } else if (raw.startsWith("-") && !raw.startsWith("---")) {
        removed.push(new vscode.Range(line, 0, line, 0));
      } else if (!raw.startsWith("\\")) {
        line++;
      }
    }
    editor.setDecorations(addedDeco, added);
    editor.setDecorations(removedDeco, removed);
  } catch {}
}

async function markViewed() {
  if (!session) return;
  const editor = vscode.window.activeTextEditor;
  if (!editor) return;
  const rel = vscode.workspace.asRelativePath(editor.document.uri);
  const res = await apiFetch(`/api/pr/${session.prNumber}/files/${encodeURIComponent(rel)}/state`, {
    method: "PUT",
    body: JSON.stringify({ state: "viewed" }),
  });
  if (res.ok) {
    vscode.window.showInformationMessage(`Marked viewed: ${rel}`);
    refreshAll();
  } else {
    vscode.window.showErrorMessage(`Failed: ${res.status}`);
  }
}

async function addComment() {
  if (!session) return;
  const editor = vscode.window.activeTextEditor;
  if (!editor) return;
  const body = await vscode.window.showInputBox({ prompt: "Comment body" });
  if (!body) return;
  const rel = vscode.workspace.asRelativePath(editor.document.uri);
  const sel = editor.selection;
  const res = await apiFetch(`/api/pr/${session.prNumber}/comments`, {
    method: "POST",
    body: JSON.stringify({
      path: rel,
      line: sel.end.line + 1,
      start_line: sel.start.line + 1,
      body,
    }),
  });
  if (res.ok) refreshAll();
  else vscode.window.showErrorMessage(`Failed: ${res.status}`);
}

async function replyToComment() {
  if (!session) return;
  const pick = await vscode.window.showQuickPick(
    commentCache.map((c) => ({ label: `${c.author || "draft"}: ${c.body.slice(0, 60)}`, id: c.id })),
    { placeHolder: "Reply to..." },
  );
  if (!pick) return;
  const body = await vscode.window.showInputBox({ prompt: "Reply body" });
  if (!body) return;
  const parent = commentCache.find((c) => c.id === (pick as any).id);
  if (!parent) return;
  const res = await apiFetch(`/api/pr/${session.prNumber}/comments`, {
    method: "POST",
    body: JSON.stringify({ path: parent.path, line: parent.line, body, parent_id: parent.id }),
  });
  if (res.ok) refreshAll();
}

async function openBrowser() {
  if (!session) return;
  vscode.env.openExternal(vscode.Uri.parse(`http://127.0.0.1:${session.port}/pr/${session.prNumber}/files`));
}

class FilesTreeProvider implements vscode.TreeDataProvider<FileItem> {
  private _onDidChange = new vscode.EventEmitter<FileItem | undefined>();
  readonly onDidChangeTreeData = this._onDidChange.event;
  refresh() { this._onDidChange.fire(undefined); }
  getTreeItem(e: FileItem) { return e; }
  getChildren(): FileItem[] {
    return filesCache.map((f) => new FileItem(f.path, f.viewed));
  }
}

class FileItem extends vscode.TreeItem {
  constructor(filePath: string, viewed: boolean) {
    super(filePath, vscode.TreeItemCollapsibleState.None);
    this.iconPath = new vscode.ThemeIcon(viewed ? "check" : "circle-outline");
    this.contextValue = viewed ? "viewed" : "unread";
    const root = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    if (root) {
      const abs = vscode.Uri.file(path.join(root, filePath));
      this.command = { command: "vscode.open", title: "Open", arguments: [abs] };
    }
  }
}
