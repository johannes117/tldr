# tldr

Local-first PR review tool. Spin up a per-PR git worktree, browse the diff in a
local web UI, draft line comments + a verdict, submit to GitHub in one shot.

## Install

### curl

```sh
curl -fsSL https://raw.githubusercontent.com/johannes117/tldr/main/install.sh | bash
```

Pin a version:

```sh
curl -fsSL https://raw.githubusercontent.com/johannes117/tldr/main/install.sh | TLDR_VERSION=v0.1.0 bash
```

### Homebrew

```sh
brew install johannes117/tldr/tldr
```

### From source

Requires Rust and Node:

```sh
cd ui && npm install && npm run build && cd ..
cargo install --path crates/tldr
```

## Setup

```sh
tldr init
```

Walks through:

1. GitHub auth (device flow; stored in your OS keychain).
2. Anthropic API key for AI walkthroughs — uses `$ANTHROPIC_API_KEY` if set,
   otherwise prompts you to paste one. The key is stored in your OS keychain
   (falling back to a `0600` file under `~/.local/state/tldr/` if the keychain
   isn't available).
3. Writes defaults to `~/.config/tldr/config.toml`.

## Use

From inside a git repo checkout:

```sh
tldr <pr-number>
```

What happens:

1. Resolves the repo slug from `origin`.
2. Reads your GitHub token (keychain → `GITHUB_TOKEN`/`GH_TOKEN` → `gh auth token`).
3. Fetches PR metadata.
4. `git fetch origin pull/<N>/head:refs/tldr/pr-<N>` into a detached worktree
   under your XDG data dir.
5. Starts a local server on a free port in `47800..47899` and opens your
   browser at `/pr/<N>/files`.

Other commands: `tldr run <n>`, `tldr list`, `tldr status`, `tldr stop`,
`tldr auth status`, `tldr config show`, `tldr doctor`, `tldr open <n>`,
`tldr editor <n>`.

## Contributing

MIT licensed. Issues and PRs welcome.

## Notes for CI

The `ui/dist` folder is embedded into the binary via `rust-embed`. CI must
`npm run build` inside `ui/` before `cargo build`. The crate's `build.rs`
also best-effort runs the UI build and falls back to a placeholder so the
binary always compiles.
