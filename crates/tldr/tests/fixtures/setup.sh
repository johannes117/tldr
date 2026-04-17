#!/usr/bin/env bash
# Create a small fixture repo under tests/fixtures/repo-small for manual exploration.
# CI and unit tests build their own tempdir repos; this script is for humans.
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)/repo-small"
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"
git init -q -b main
git config user.email test@example.com
git config user.name "Fixture Bot"
cat > a.txt <<'EOF'
one
two
three
EOF
cat > b.txt <<'EOF'
alpha
beta
EOF
git add .
git commit -qm "base"
git checkout -qb feature
cat > a.txt <<'EOF'
one
two-changed
three
four
EOF
echo "new file" > c.txt
rm b.txt
git add -A
git commit -qm "feature change"
echo "Fixture repo ready at $DIR (branches: main, feature)"
