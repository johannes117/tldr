#!/usr/bin/env bash
# tldr installer — downloads a prebuilt binary from GitHub Releases.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/johannes117/tldr/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/johannes117/tldr/main/install.sh | TLDR_VERSION=v0.1.0 bash
#
# Env:
#   TLDR_VERSION  version tag to install (default: latest release)
#   TLDR_PREFIX   install prefix (default: /usr/local if writable, else $HOME/.local)

set -euo pipefail

REPO="johannes117/tldr"
VERSION="${TLDR_VERSION:-}"
PREFIX="${TLDR_PREFIX:-}"

msg()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarn:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"; }
need curl
need tar
need uname

os_arch() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin) os=apple-darwin ;;
    Linux)  os=unknown-linux-gnu ;;
    *) die "unsupported OS: $os" ;;
  esac
  case "$arch" in
    x86_64|amd64) arch=x86_64 ;;
    arm64|aarch64) arch=aarch64 ;;
    *) die "unsupported arch: $arch" ;;
  esac
  echo "${arch}-${os}"
}

resolve_version() {
  if [ -n "$VERSION" ]; then echo "$VERSION"; return; fi
  curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep -m1 '"tag_name":' \
    | sed -E 's/.*"([^"]+)".*/\1/'
}

resolve_prefix() {
  if [ -n "$PREFIX" ]; then echo "$PREFIX"; return; fi
  if [ -w /usr/local/bin ] 2>/dev/null; then echo /usr/local; return; fi
  if command -v sudo >/dev/null 2>&1 && [ -d /usr/local/bin ]; then
    echo /usr/local
    return
  fi
  echo "$HOME/.local"
}

main() {
  local target version prefix tmp url bindir
  target="$(os_arch)"
  version="$(resolve_version)"
  [ -n "$version" ] || die "could not resolve latest version"
  prefix="$(resolve_prefix)"
  bindir="$prefix/bin"

  url="https://github.com/${REPO}/releases/download/${version}/tldr-${version}-${target}.tar.gz"

  msg "installing tldr ${version} (${target}) to ${bindir}"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  msg "downloading ${url}"
  curl -fsSL "$url" -o "$tmp/tldr.tar.gz" || die "download failed"
  tar -xzf "$tmp/tldr.tar.gz" -C "$tmp"

  mkdir -p "$bindir"
  if [ -w "$bindir" ]; then
    install -m 0755 "$tmp/tldr" "$bindir/tldr"
  else
    msg "using sudo to install to $bindir"
    sudo install -m 0755 "$tmp/tldr" "$bindir/tldr"
  fi

  msg "installed: $bindir/tldr"
  case ":$PATH:" in
    *":$bindir:"*) ;;
    *) warn "$bindir is not on your PATH. Add it to your shell profile:"
       printf '    export PATH="%s:$PATH"\n' "$bindir" ;;
  esac

  msg "next: run \`tldr init\` to configure GitHub auth and your Anthropic API key."
}

main "$@"
