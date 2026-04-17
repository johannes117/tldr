# tldr — Product Requirements Document

**Product:** tldr (too long; didn't review)
**Document status:** Engineering spec, v1.0
**Deliverable:** Complete end-state specification for a local-first PR review companion tool

---

## 1. Summary

tldr is a local-first developer tool that replaces the GitHub.com and IDE-extension PR review workflow. It exists for two reasons: (1) reviewers routinely approve PRs they do not fully understand, because the tools available to them make genuine comprehension expensive, and (2) reviewing any non-trivial PR with full code navigation currently requires checking out the PR branch locally, which is disruptive enough that most reviewers skip it.

tldr solves both problems in a single product. It runs as a CLI that spawns a local web server and a browser UI, backed by transparent `git worktree` management so reviewers get a fully-navigable, editor-connected review environment for any PR without disturbing their working tree. Layered on top of this review environment are comprehension aids — call graph diffs, blast radius analysis, test coverage mapping, symbol-anchored comments, and AI-generated walkthroughs — that turn a diff into an explorable, navigable artifact.

Reviewers read, comment, and submit reviews from inside tldr. tldr posts the review to GitHub via the GitHub API on submission. At no point does tldr act as an automated reviewer; its job is to make the human reviewer faster, more accurate, and less likely to rubber-stamp code they don't understand.

---

## 2. Goals and non-goals

### Goals

- Eliminate the "checkout the branch to review" workflow for non-trivial PRs.
- Give reviewers structural context (call graphs, blast radius, test coverage delta) that neither GitHub nor IDE extensions provide.
- Capture structured review intent — per-file sign-off state, symbol-anchored comments, draft review continuity across sessions.
- Make submitting a review a one-click handoff to GitHub's review API, preserving all metadata.
- Work offline for everything that doesn't require GitHub write access.
- Install and start reviewing a PR in under 60 seconds from a fresh machine.

### Non-goals

- tldr is **not** an AI PR reviewer. It does not generate approve/request-changes decisions. It does not leave review comments on behalf of the user.
- tldr is **not** a Graphite/Sapling-style stacked-diff workflow tool. It reviews PRs; it does not create or manage them.
- tldr is **not** a GitHub App or cloud service in v1. It is a local CLI.
- tldr is **not** a full IDE. It integrates with external editors rather than replacing them.
- tldr does **not** support non-GitHub forges (GitLab, Bitbucket, Gerrit) in v1. The architecture must not preclude them, but v1 ships GitHub-only.

---

## 3. Users and core use cases

### Primary user

Professional software engineer whose employer uses GitHub, who reviews 3–20 PRs per week, and who currently uses some combination of github.com, the VS Code PR extension, and occasional local checkouts. Works in a repository between 10K and 10M lines of code, likely a monorepo or a mid-sized service repo. Uses VS Code, Cursor, Zed, or a JetBrains IDE as their primary editor.

### Core use cases

1. **Quick review of a small PR.** Reviewer runs `tldr 1234`, the browser opens to the PR diff, they read 3 files, leave 2 comments, approve, and submit. Total time: under 2 minutes. Never touched their working tree.

2. **Deep review of a large PR.** Reviewer runs `tldr 1234` on a 40-file PR. They use the file tree, per-file sign-off checkboxes, and the call graph diff to work through it over 45 minutes across two sittings. Draft state persists between sessions. They open the worktree in their editor to trace a function call into unchanged code, then return to the browser to leave a comment anchored to that symbol. On submit, the full review with all comments posts to GitHub.

3. **Cross-referencing against related work.** While reviewing, the reviewer asks "why did this change?" on a specific hunk. tldr surfaces the linked issue, the PR description section that motivates this file, any referenced Slack threads or design docs (if integrated), and prior PRs that touched the same symbols.

4. **Catching missing tests.** Reviewer opens a PR and immediately sees which new branches have no test coverage, highlighted inline in the diff.

5. **Understanding blast radius.** Reviewer changes a public function's signature. tldr shows all call sites across the repository, whether each still type-checks, and whether any are in files the PR didn't touch.

---

## 4. Product principles

These are the tiebreakers when engineering decisions are ambiguous.

- **Local-first.** All indexing, analysis, and review drafting happens on the user's machine. Network calls are only to GitHub's API and to the optional AI provider.
- **The human is the reviewer.** tldr surfaces information and captures intent. It never generates review verdicts.
- **Don't disturb the working tree.** The reviewer's current branch, dev server, and editor state are sacred. Everything tldr does happens in a worktree it manages itself.
- **Respect the primary artifact.** Code is the primary artifact. Diagrams, summaries, and call graphs are scaffolding. Every comprehension feature must link back to specific lines of code.
- **Fast install, fast startup, fast everything.** Cold-start to interactive UI under 3 seconds for a cached repo. Indexing happens in the background and degrades gracefully.
- **Degrade gracefully when offline or unauthed.** Read-only review works without a network. Comprehension features that require AI work without AI, they just show less.

---

## 5. System architecture

### 5.1 High-level components

tldr ships as a single binary (`tldr`) that, when invoked, runs three things in one process:

1. **CLI front-end** — argument parsing, auth, worktree management, editor launching.
2. **Local HTTP+WebSocket server** — serves the UI and exposes a JSON-RPC API the UI consumes.
3. **Indexer** — background worker pool that analyzes the PR and the surrounding repository to produce the data the UI needs (symbols, call graphs, coverage, etc.).

### 5.2 Implementation language

The `tldr` binary is implemented in **Rust**. The UI is **TypeScript + React + Vite + TanStack**, built into static assets and embedded in the Rust binary via `rust-embed`.

### 5.3 Process model

- CLI invocation forks the server process, waits for port bind, opens browser, exits (unless `--foreground`).
- Server self-terminates after 30 minutes of UI inactivity, or on `tldr stop`.
- One server per repository; additional `tldr NNN` invocations route to the existing server.
- Indexer is a thread pool within the server process.

### 5.4 Storage

Local state under `$XDG_STATE_HOME/tldr/`:

```
~/.local/state/tldr/
├── config.toml
├── auth.json
├── repos/
│   └── <repo-slug>/
│       ├── index.db
│       ├── blobs/
│       ├── worktrees/
│       │   └── pr-1234/
│       ├── drafts/
│       │   └── pr-1234.json
│       └── lsp-workspace/
```

### 5.5 Authentication

1. Check for `gh` CLI. Read token via `gh auth token` if authed.
2. Otherwise run GitHub device-flow OAuth. Store token in OS keychain (fallback to 0600 file).
3. Scopes: `repo`, `read:user`, `read:org`.

---

## 6. User-facing surfaces

### 6.1 CLI

| Command | Behavior |
|---|---|
| `tldr <number>` | Start review of PR |
| `tldr <url>` | Same, by GitHub URL |
| `tldr list` | Open PRs awaiting review |
| `tldr status` | Server status, active reviews |
| `tldr stop` / `tldr stop --all` | Stop server(s) |
| `tldr auth login` / `logout` | Auth |
| `tldr config` | Open config |
| `tldr doctor` | Diagnostics |
| `tldr open` | Open active review |
| `tldr editor <name>` | Set preferred editor |

Flags: `--no-browser`, `--no-index`, `--editor`, `--foreground`, `--port`, `--worktree-dir`.

### 6.2 Web UI

Stack: React 19 + React Compiler, TanStack Router (file-based), TanStack Query, TanStack Table, Vite, Tailwind, `@pierre/diffs`, Shiki, `@xyflow/react`, Monaco.

Routes:
```
/
/pr/$number
/pr/$number/files
/pr/$number/files/$path
/pr/$number/graph
/pr/$number/blast/$symbol
/pr/$number/coverage
/pr/$number/walkthrough
/pr/$number/review
/settings
```

All routes deep-linkable. WebSocket for real-time (JSON-RPC 2.0), HTTP for reads.

### 6.3 Editor integrations

**A — launch editor pointed at worktree:** VS Code, Cursor, Zed, JetBrains, Neovim.

**B — VS Code extension (ships v1):** inline comment decorations, reply from editor, "Mark as viewed" command, minimap hunk summary. Browser is primary; extension is additive.

---

## 7. The review environment (Product B)

### 7.1 Worktree management

On `tldr <number>`:

1. Locate repo (walk up for `.git`).
2. `git fetch origin pull/<number>/head:refs/tldr/pr-<number>`.
3. Determine base: use GitHub API for base SHA, compute merge-base with base branch. Diff against merge-base.
4. `git worktree add --detach <state>/worktrees/pr-<number> refs/tldr/pr-<number>`.
5. On re-run: fetch + `reset --hard` to handle force-pushes.
6. Cleanup on stop or PR merge.

Invariants: never touch user's branch/HEAD; deterministic worktree path; multiple concurrent PRs OK; uncommitted changes in main worktree untouched.

### 7.2 Fork and permission handling

Fork PRs via `pull/<n>/head` (works with reviewer access). No push permission on branch = grey out "Suggest edit".

### 7.3 Diff computation

Local via libgit2. Reasons: GitHub truncates >3000 lines, need full tree for nav, offline support.

Render via `@pierre/diffs` plus:
- Per-file viewed state (3-way: unread/viewed/approved-for-file)
- Collapsible unchanged context (`j/k`, `]/[`)
- Word-level highlighting
- Image diffs (side-by-side or onion-skin)
- Binary file handling (size delta)
- Moved-code detection (Myers-style)
- Rename detection (50% similarity, configurable)
- Generated-file collapse (lockfiles, `*.pb.go`, linguist-generated)
- Large-file summary (>5000 lines)

### 7.4 Keyboard shortcuts

| Key | Action |
|---|---|
| `j` / `k` | Next/prev hunk |
| `n` / `p` | Next/prev file |
| `o` | Toggle tree sidebar |
| `v` | Mark viewed |
| `c` | Comment on hunk |
| `r` | Reply |
| `g g` | Top |
| `g e` | Open in editor |
| `g h` | Call graph |
| `g b` | Blast radius |
| `/` | Command palette |
| `?` | Help |
| `⌘/Ctrl + Enter` | Submit comment |
| `⌘/Ctrl + S` | Save draft |

### 7.5 Commenting

Types: line, range, symbol (survives force-push), file, PR-level.

Composer: Markdown + live preview, Shiki code blocks, `@` mentions, suggested edits (translated to GitHub `suggestion`), drag-drop images (upload to user-content CDN on submit), auto-save on keystroke.

Per-file sign-off: `unread` / `viewed` / `approved-for-file`. Latter two → GitHub "Viewed".

### 7.6 Review submission

`/pr/$number/review` page. Summary, warnings (unopened files, orphaned comments), verdict selector (Comment/Approve/Request changes), body, Submit.

Flow: serialize → GraphQL `addPullRequestReview` with `threads` → REST viewed PUT per file → archive draft (30d retention) → navigate to overview. On failure: preserve draft, show retry; reconcile partial submits by checking existing comments.

### 7.7 Real-time updates

Watches for: new commits (60s poll, webhooks if helper running), new comments, CI changes, PR state changes. New commit → banner "Author pushed 2 new commits. Update?" → refetch + rebase worktree. Orphaned comments flagged, not deleted.

---

## 8. Comprehension features (Product A)

### 8.1 Indexer

Rust worker pool produces: symbol table, call graph, reference graph, import graph, test-to-code map, blame cache.

Languages v1: TS/JS, Python, Go, Rust. Language-agnostic `Language` trait for extension.

Implementation: tree-sitter parsing + LSP semantic analysis (tsserver, pyright, gopls, rust-analyzer). Shared LSP workspace per repo across PRs. Call graph via `textDocument/callHierarchy` or tree-sitter queries fallback.

Fallback: no LSP → syntactic-only features. LSP crash → 3 restarts, then tree-sitter only.

Perf: 1M LOC first index <5min; incremental <15s; <500MB/1M LOC.

### 8.2 Call graph diff

React Flow. Nodes: changed functions + 1-hop callers/callees (dimmed). Edges: solid (unchanged), green (added), red (removed), orange (call-site args changed). Click node → jump; click edge → call-site diff panel. Force-directed left-to-right. Filters: hide unchanged, 2/3-hop, by file/author/subsystem.

### 8.3 Blast radius

For each changed exported symbol: signature diff (semantic, e.g., "parameter `timeout` added with default `30`"); reference table (location, PR-modified?, LSP diagnostic status, one-click jump). Critical signal: **unmodified reference that now type-errors**.

### 8.4 Test coverage delta

Reads: `lcov.info`, Cobertura `coverage.xml`, Go `coverage.out`, pluggable. Sources: CI artifact via GitHub API, or local. Inline gutter (green/red/grey), summary ("12 new uncovered lines in 3 files"), per-file %, per-symbol ("`validateToken` was fully covered, now has 3 uncovered branches").

### 8.5 "Why did this change?" tracing

Surfaces: PR description (parsed by heading matching paths), linked issues, external links (Linear/Notion/Slack — one click away, not fetched), prior PRs touching same symbols, blame-derived history, release notes. UI composition, not AI.

### 8.6 Reviewer-specific framing

From GitHub identity: files you own (CODEOWNERS), files you've touched (`git log --author`), files matching expertise (heuristic). Default file-tree sort: owned → touched → rest.

### 8.7 AI-generated walkthrough (optional)

Off by default. Provider: Anthropic / OpenAI / Ollama / LiteLLM-compat.

Generates: 3–8 step walkthrough, each with prose (2–4 sentences), hunk pointers, optional Mermaid diagram. Sidebar step-through mode scrolls diff.

**Never generates:** verdicts, line comments, suggestions, anything mistakeable for human review.

Flow: on PR open if enabled, send diff + description + N KB file context → stream to UI → cache by head-SHA.

Privacy: clear notice before first use. Repo opt-out via `.tldr/config.toml` `ai.enabled = false`.

### 8.8 Walkthrough-as-diagram

Alternative presentation of §8.7 as React Flow diagram (nodes = steps, edges = sequence, click → hunks). Same underlying data structure.

---

## 9. Data model

### 9.1 Local index schema (SQLite)

Tables: `repos`, `symbols` (SHA-scoped, with qualified_name, kind, position, signature), `refs` (from→to, kind, site), `blame` (changed files only, per-line), `prs` (per head_sha: title, body, author, base/head/merge-base SHAs).

### 9.2 Draft review format (JSON)

```json
{
  "version": 1,
  "repo": "owner/name",
  "pr_number": 1234,
  "base_sha": "...", "head_sha": "...",
  "started_at": "...", "last_updated_at": "...",
  "reviewer": "octocat",
  "body": "...", "verdict": "approve",
  "comments": [
    {"id": "c-01JXXX", "type": "line", "path": "...", "line": 47, "side": "RIGHT", "body": "...", "created_at": "...", "resolved_in_draft": false},
    {"id": "c-01JYYY", "type": "symbol", "path": "...", "symbol": "validateToken", "body": "...", "created_at": "..."}
  ],
  "file_states": {"path": "viewed" | "approved-for-file" | "unread"}
}
```

### 9.3 Server ↔ UI API

JSON-RPC 2.0 over WebSocket for state changes + subscriptions. HTTP GET for idempotent reads.

Methods:
- `review.open(pr_number)` → `{session_id, pr_data}`
- `review.get_draft(pr_number)` → `DraftReview`
- `review.add_comment(pr_number, comment)` → `{comment_id}`
- `review.update_comment(pr_number, comment_id, patch)` → `ok`
- `review.set_file_state(pr_number, path, state)` → `ok`
- `review.submit(pr_number, verdict, body)` → `{github_review_id, errors}`
- `index.status / call_graph / blast_radius / coverage`
- `walkthrough.get / generate` (streaming)

Subscriptions: `review.subscribe`, `index.subscribe`, `github.subscribe`.

### 9.4 GitHub API usage

Prefer GraphQL. Endpoints: PR metadata, files (fallback), reviews/threads/commits (GraphQL), `addPullRequestReview` mutation, file viewed PUT, Actions runs + artifacts. Rate-limit: exponential backoff + jitter, UI surfacing, pre-submit warning when nearly out.

---

## 10. Security and privacy

### 10.1 Threat model

1. **Credential theft** → keychain; no logs; no URL params.
2. **Malicious PR content** → browser-sandboxed rendering; no server-side execution; LSP no network.
3. **Prompt injection** → explicit system prompt framing; labeled "AI-generated"; no tool access.
4. **Local server exposure** → CORS locked; cryptographic token via `SameSite=Strict` cookie set at server start; random port.
5. **Worktree escape** → never write from PR content; indexer read-only; path validation.

### 10.2 Privacy

No telemetry v1. Only outbound: GitHub API + optional AI provider. `tldr reset --all` wipes. Repo opt-out via `.tldr/config.toml`.

### 10.3 Distribution

Signed + notarized binaries: macOS universal, Windows x64/arm64, Linux x64/arm64. Install via Homebrew, winget, `curl | sh`. Opt-in auto-updates via signed manifest. Source-available license.

---

## 11. Configuration

### 11.1 Global (`~/.config/tldr/config.toml`)

```toml
[editor]
command = "code"
args_template = "--goto {path}:{line}:{col}"

[ui]
port_range = [47800, 47899]
theme = "auto"
open_browser_on_start = true

[indexing]
enabled = true
languages = ["typescript", "javascript", "python", "go", "rust"]
max_memory_mb = 4096
lsp_timeout_seconds = 30

[ai]
enabled = false
provider = "anthropic"
model = "claude-opus-4-7"
api_key_env = "ANTHROPIC_API_KEY"
endpoint = ""

[github]
token_source = "keychain"
retry_attempts = 3
```

### 11.2 Per-repo (`.tldr/config.toml`)

```toml
[ai]
enabled = true

[coverage]
lcov_path = "coverage/lcov.info"
exclude_patterns = ["**/*.test.ts", "**/__mocks__/**"]

[review]
required_viewed_before_approve = true

[codeowners]
path = ".github/CODEOWNERS"
```

---

## 12. Observability

Logs: `$XDG_STATE_HOME/tldr/logs/<date>.log`, daily rotation, 7d retention, 100MB cap.

Logged: server lifecycle, PR open (redacted), index phases/timings, LSP lifecycle, GitHub API calls (redacted URLs, status, timing), AI calls (provider, model, tokens — not content), errors + stacks.

**Never logged:** tokens, PR content, user comments, AI content.

`tldr doctor` produces redacted diagnostic bundle.

---

## 13. Performance requirements

| Metric | Target | Hard ceiling |
|---|---|---|
| Cold-start, cached repo | 2s | 5s |
| Cold-start, first time | 10s | 30s |
| First index, 100K LOC | 20s | 60s |
| First index, 1M LOC | 3min | 8min |
| Incremental re-index | 5s | 20s |
| Diff render, 20 files | 100ms | 500ms |
| Diff render, 200 files | 500ms | 2s |
| Hunk nav (j/k) | 16ms | 50ms |
| Call graph, 100 nodes | 200ms | 1s |
| Draft save | 10ms | 100ms |
| Submit, 20 comments | 2s | 10s |
| Memory idle | 200MB | 500MB |
| Memory, indexing 1M LOC | 2GB | 4GB |

Hard ceiling exceeded = perf bug.

---

## 14. Testing

- Unit (80% line coverage non-UI).
- Integration (real server against fixture repo, JSON-RPC assertions).
- E2E (Playwright, real binary, mock GitHub).
- Fixture repos: 1K, 10K, 100K, 500K, 2M LOC.
- Perf regression in CI (>10% fails build).
- Editor launch compat tests.
- GitHub API contract tests.
- Malicious input: oversize, binary, path traversal, UTF-8, merge conflicts, mid-review force-push.

---

## 15. Launch surface

- Single binary: macOS universal, Linux x64/arm64, Windows x64/arm64
- `brew install tldr-tool/tap/tldr`, `winget install tldr`, `curl -sSL install.tldr.dev | sh`
- VS Code extension in marketplace
- `tldr.dev/docs`: getting started, CLI ref, shortcuts, config, AI setup, editor, troubleshooting
- 90-second demo video: install → review → submit

---

## 16. Open questions for implementation

1. Rust HTTP: `axum` vs `poem` vs `actix` (recommend axum).
2. Tree-sitter vs LSP priority (render first with tree-sitter, upgrade to LSP? likely yes).
3. Binary size budget (UI + grammars push past 50MB — acceptable?).
4. Webhook helper scope (v1 or v2?).
5. JetBrains extension (v2 likely).
6. Symbol-anchored comment force-push behavior spec.
7. Coverage formats first-class (recommend lcov, Cobertura, Go native).
