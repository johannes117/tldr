# tldr

Local-first PR review tool. Spin up a per-PR worktree, browse the diff in a local
web UI, draft line comments + a verdict, submit to GitHub in one shot.

## Run

```
cd ui && npm install && npm run build && cd ..
cargo run -p tldr -- <pr-number>
```

`cargo run` from inside a git repo's checkout. It will:

1. Resolve the repo slug from `origin`.
2. Read a GitHub token from `GITHUB_TOKEN`, `GH_TOKEN`, or `gh auth token`.
3. Fetch the PR metadata.
4. `git fetch origin pull/<N>/head:refs/tldr/pr-<N>` and add a detached worktree under
   the XDG data dir.
5. Start an axum server on a free port in 47800..47899 and open your browser at `/pr/<N>/files`.

Other commands: `tldr run <n>`, `tldr list`, `tldr status`, `tldr stop`, `tldr auth status`,
`tldr config show`, `tldr doctor`, `tldr open <n>`, `tldr editor <n>`.

## Implemented (MVP)

- CLI skeleton (all subcommands) — `run` is the fully functional one.
- Worktree lifecycle via `git` subprocess.
- Diff compute via `git diff <merge-base>..<head>` + unified-diff parser.
- Drafts: JSON in `<state>/repos/<slug>/drafts/pr-<N>.json`; CRUD via REST.
- GitHub PR fetch (REST) and review submit (GraphQL `addPullRequestReview` + per-file
  viewed state PUT).
- Axum server serving embedded UI + REST API (`/api/*`).
- React 19 + Vite + Tailwind UI: PR overview, file tree + diff viewer, review submit.
- Keyboard shortcuts (j/k/n/p/v/c).
- Per-file viewed state.

## Stubbed / future work

- LSP-backed symbol index and `Language` trait (`crates/tldr/src/indexer.rs`).
- Call graph, blast radius, coverage overlay (stub types in `indexer.rs`).
- AI walkthrough (stub type).
- WebSocket / JSON-RPC transport (MVP uses plain HTTP; query invalidation on mutation).
- VS Code extension (`vscode-extension/`).
- `keyring` keychain storage (env + `gh` only for MVP).
- `list`/`status`/`stop` session registry (prints placeholder).
- Syntax highlighting (shiki is installed but not wired yet).

## Notes for CI

The `ui/dist` folder is embedded into the binary via `rust-embed`. CI should
`npm run build` in `ui/` before `cargo build`. The crate's `build.rs` will also
best-effort attempt the UI build and fall back to a placeholder so the binary
always compiles.
