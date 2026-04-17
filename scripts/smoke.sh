#!/usr/bin/env bash
# Smoke test: build binary + UI, run a couple of no-network commands, assert success.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

fail=0
log() { printf "[smoke] %s\n" "$*"; }
pass() { log "PASS: $*"; }
fail() { log "FAIL: $*"; fail=1; }

# 1) Build UI (produces dist used by the binary via rust-embed)
log "building UI"
if (cd ui && npm run build >/tmp/smoke-ui-build.log 2>&1); then
  pass "ui build"
else
  fail "ui build (see /tmp/smoke-ui-build.log)"
fi

# 2) Build binary
log "building tldr binary"
if cargo build --bin tldr >/tmp/smoke-cargo-build.log 2>&1; then
  pass "cargo build"
else
  fail "cargo build (see /tmp/smoke-cargo-build.log)"
fi

BIN="$ROOT/target/debug/tldr"
if [ ! -x "$BIN" ]; then
  fail "binary not found at $BIN"
fi

# 3) --help must exit 0 and print usage
if "$BIN" --help >/tmp/smoke-help.log 2>&1; then
  if grep -qi "tldr" /tmp/smoke-help.log; then
    pass "tldr --help"
  else
    fail "tldr --help output didn't mention 'tldr'"
  fi
else
  fail "tldr --help non-zero"
fi

# 4) doctor should run and exit (may warn; we only require it to not crash)
if "$BIN" doctor >/tmp/smoke-doctor.log 2>&1; then
  pass "tldr doctor"
else
  # Doctor may exit non-zero on a fresh env; still want it to not segfault
  if [ -s /tmp/smoke-doctor.log ]; then
    pass "tldr doctor (ran, produced output)"
  else
    fail "tldr doctor crashed with no output"
  fi
fi

if [ "$fail" -eq 0 ]; then
  log "ALL PASS"
  exit 0
else
  log "FAILURES DETECTED"
  exit 1
fi
