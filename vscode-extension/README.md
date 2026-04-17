# tldr Review (VS Code extension)

Additive companion to the tldr web UI. The browser is the primary review surface; this extension lets you stay in your editor for light-touch actions.

## Features

- Autodiscovers the running tldr server when the workspace root is a tldr worktree (`<state>/tldr/repos/<slug>/worktrees/pr-<n>`) by reading `<state>/tldr/repos/<slug>/session.json` for port + CSRF token.
- Inline gutter decorations for draft + GitHub review comments, with author/body hover. Polls every 5s.
- Minimap (overview ruler) markers for added (green) / removed (red) hunks from `/api/pr/:n/diff`.
- Explorer sidebar "tldr Review" TreeView listing PR files with viewed/unread icons; click to open.
- Commands:
  - `tldr: Mark File Viewed`
  - `tldr: Add Comment on Selection`
  - `tldr: Reply to Comment`
  - `tldr: Open in Browser`
  - `tldr: Refresh`

## Install

```
code --install-extension tldr-vscode-0.1.0.vsix
```

Build the `.vsix` with `npm run package` (requires `@vscode/vsce`).

## Settings

- `tldr.pollInterval` (default `5000`): refresh interval in ms.
- `tldr.serverUrl`: override autodiscovery, e.g. `http://127.0.0.1:8787`.
- `tldr.csrfToken`: override autodiscovery.

## Develop

```
npm install
npm run compile
```

Copy `launch.json.example` to `.vscode/launch.json`, then press F5 in VS Code to launch an Extension Development Host. Open a tldr worktree folder in that host window to activate.

## Caveats

- Autodiscovery assumes the worktree path pattern `.../tldr/repos/<slug>/worktrees/pr-<n>`.
- Diff parser is line-based and approximate; complex renames may render incorrectly.
- No auth beyond CSRF header; assumes the tldr server is bound to loopback.
